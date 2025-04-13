// Copyright 2021 Datafuse Labs
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::collections::VecDeque;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use bumpalo::Bump;
use databend_common_exception::ErrorCode;
use databend_common_exception::Result;
use databend_common_expression::AggregateHashTable;
use databend_common_expression::BlockMetaInfoDowncast;
use databend_common_expression::DataBlock;
use databend_common_expression::HashTableConfig;
use databend_common_expression::InputColumns;
use databend_common_expression::Payload;
use databend_common_expression::PayloadFlushState;
use databend_common_expression::ProbeState;
use databend_common_pipeline_core::processors::Exchange;
use databend_common_pipeline_core::processors::ReadyPartition;

use crate::pipelines::processors::transforms::aggregator::AggregateMeta;
use crate::pipelines::processors::transforms::aggregator::AggregatePayload;
use crate::pipelines::processors::transforms::aggregator::AggregatorParams;
use crate::pipelines::processors::transforms::aggregator::InFlightPayload;

const HASH_SEED: u64 = 9263883436177860930;

pub struct ExchangePartition {
    processed_rows: AtomicUsize,
    processed_block: AtomicUsize,
    params: Arc<AggregatorParams>,
}

impl ExchangePartition {
    pub fn create(params: Arc<AggregatorParams>) -> Arc<Self> {
        Arc::new(ExchangePartition {
            processed_rows: AtomicUsize::new(0),
            processed_block: AtomicUsize::new(0),
            params,
        })
    }
}

impl ExchangePartition {
    fn unpark(block: &mut DataBlock) -> AggregatePayload {
        let meta = block.take_meta().unwrap();
        match AggregateMeta::downcast_from(meta).unwrap() {
            AggregateMeta::AggregatePayload(payload) => payload,
            _ => unreachable!(),
        }
    }

    fn partition_final_payload(to: &mut Vec<VecDeque<DataBlock>>) -> Result<ReadyPartition> {
        for partition in to {
            partition.push_back(DataBlock::empty_with_meta(AggregateMeta::create_final()));
        }

        Ok(ReadyPartition::AllPartitionReady)
    }

    fn partition_aggregate(
        &self,
        mut payload: AggregatePayload,
        to: &mut [VecDeque<DataBlock>],
    ) -> Result<ReadyPartition> {
        if payload.payload.len() == 0 {
            return Ok(ReadyPartition::NoReady);
        }

        let processed_block = self.processed_block.fetch_add(1, Ordering::Acquire) + 1;
        let processed_rows = self
            .processed_rows
            .fetch_add(payload.payload.len(), Ordering::Acquire)
            + payload.payload.len();
        let avg_rows = processed_rows / processed_block;

        for partitioning in to.iter_mut() {
            if partitioning.is_empty() {
                let repartition_payload = Payload::new(
                    payload.payload.arena.clone(),
                    payload.payload.group_types.clone(),
                    payload.payload.aggrs.clone(),
                    payload.payload.states_layout.clone(),
                );

                partitioning.push_back(DataBlock::empty_with_meta(
                    AggregateMeta::create_agg_payload(
                        repartition_payload,
                        payload.partition,
                        payload.max_partition,
                        payload.global_max_partition,
                    ),
                ));
            }
        }

        let mut ready_partition = vec![];
        let mut state = PayloadFlushState::default();

        // scatter each page of the payload.
        while payload
            .payload
            .scatter_with_seed::<HASH_SEED>(&mut state, to.len())
        {
            // copy to the corresponding bucket.
            for (idx, block) in to.iter_mut().enumerate() {
                let back = block.back_mut().unwrap();
                let mut aggregate_payload = Self::unpark(back);

                let count = state.probe_state.partition_count[idx];

                if count > 0 {
                    let sel = &state.probe_state.partition_entries[idx];
                    aggregate_payload
                        .payload
                        .copy_rows(sel, count, &state.addresses);
                    aggregate_payload
                        .payload
                        .arena
                        .extend(payload.payload.arena.clone());
                }

                if aggregate_payload.payload.len() >= avg_rows {
                    ready_partition.push(idx);
                }

                *block.back_mut().unwrap() = DataBlock::empty_with_meta(Box::new(
                    AggregateMeta::AggregatePayload(aggregate_payload),
                ));
            }
        }

        payload.payload.state_move_out = true;
        Ok(ReadyPartition::PartialPartition(ready_partition))
    }

    fn partition_flight_payload(
        &self,
        payload: InFlightPayload,
        block: DataBlock,
        to: &mut [VecDeque<DataBlock>],
    ) -> Result<ReadyPartition> {
        let rows_num = block.num_rows();

        if rows_num == 0 {
            return Ok(ReadyPartition::NoReady);
        }

        let group_len = self.params.group_data_types.len();

        let mut state = ProbeState::default();

        // create single partition hash table for deserialize
        let capacity = AggregateHashTable::get_capacity_for_count(rows_num);
        let config = HashTableConfig::default().with_initial_radix_bits(0);
        let mut hashtable = AggregateHashTable::new_directly(
            self.params.group_data_types.clone(),
            self.params.aggregate_functions.clone(),
            config,
            capacity,
            Arc::new(Bump::new()),
            false,
        );

        let num_states = self.params.num_states();
        let states_index: Vec<usize> = (0..num_states).collect();
        let agg_states = InputColumns::new_block_proxy(&states_index, &block);

        let group_index: Vec<usize> = (num_states..(num_states + group_len)).collect();
        let group_columns = InputColumns::new_block_proxy(&group_index, &block);

        let _ = hashtable.add_groups(
            &mut state,
            group_columns,
            &[(&[]).into()],
            agg_states,
            rows_num,
        )?;

        hashtable.payload.mark_min_cardinality();
        assert_eq!(hashtable.payload.payloads.len(), 1);

        self.partition_aggregate(
            AggregatePayload {
                partition: payload.partition,
                payload: hashtable.payload.payloads.pop().unwrap(),
                max_partition: payload.max_partition,
                global_max_partition: payload.global_max_partition,
            },
            to,
        )
    }
}

impl Exchange for ExchangePartition {
    const NAME: &'static str = "AggregatePartitionExchange";
    const MULTIWAY_SORT: bool = false;

    fn partition(
        &self,
        mut data_block: DataBlock,
        to: &mut Vec<VecDeque<DataBlock>>,
    ) -> Result<ReadyPartition> {
        let Some(meta) = data_block.take_meta() else {
            return Err(ErrorCode::Internal(
                "AggregatePartitionExchange only recv AggregateMeta",
            ));
        };

        let Some(meta) = AggregateMeta::downcast_from(meta) else {
            return Err(ErrorCode::Internal(
                "AggregatePartitionExchange only recv AggregateMeta",
            ));
        };

        match meta {
            // already restore in upstream
            AggregateMeta::SpilledPayload(_) => unreachable!(),
            // broadcast final partition to downstream
            AggregateMeta::FinalPartition => Self::partition_final_payload(to),
            AggregateMeta::AggregatePayload(payload) => self.partition_aggregate(payload, to),
            AggregateMeta::InFlightPayload(payload) => {
                self.partition_flight_payload(payload, data_block, to)
            }
        }
    }
}
