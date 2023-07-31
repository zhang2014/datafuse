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

use std::any::Any;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;

use common_base::base::GlobalUniqName;
use common_exception::ErrorCode;
use common_exception::Result;
use common_expression::arrow::serialize_column;
use common_expression::BlockEntry;
use common_expression::BlockMetaInfoDowncast;
use common_expression::DataBlock;
use common_hashtable::HashtableLike;
use common_pipeline_core::processors::port::InputPort;
use common_pipeline_core::processors::port::OutputPort;
use common_pipeline_core::processors::processor::Event;
use common_pipeline_core::processors::Processor;
use futures_util::future::BoxFuture;
use opendal::Operator;
use tracing::info;

use crate::pipelines::processors::transforms::aggregator::aggregate_meta::AggregateMeta;
use crate::pipelines::processors::transforms::aggregator::aggregate_meta::BucketSpilledPayload;
use crate::pipelines::processors::transforms::aggregator::aggregate_meta::HashTablePayload;
use crate::pipelines::processors::transforms::aggregator::serde::transform_aggregate_serializer::serialize_aggregate;
use crate::pipelines::processors::transforms::group_by::HashMethodBounds;
use crate::pipelines::processors::transforms::group_by::PartitionedHashMethod;
use crate::pipelines::processors::transforms::metrics::metrics_inc_aggregate_spill_data_serialize_milliseconds;
use crate::pipelines::processors::transforms::metrics::metrics_inc_aggregate_spill_write_bytes;
use crate::pipelines::processors::transforms::metrics::metrics_inc_aggregate_spill_write_count;
use crate::pipelines::processors::transforms::metrics::metrics_inc_aggregate_spill_write_milliseconds;
use crate::pipelines::processors::AggregatorParams;

pub struct TransformAggregateSpillWriter<Method: HashMethodBounds> {
    method: Method,
    input: Arc<InputPort>,
    output: Arc<OutputPort>,
    params: Arc<AggregatorParams>,

    operator: Operator,
    location_prefix: String,
    spilled_block: Option<DataBlock>,
    spilling_meta: Option<AggregateMeta<Method, usize>>,
    spilling_future: Option<BoxFuture<'static, Result<()>>>,
}

impl<Method: HashMethodBounds> TransformAggregateSpillWriter<Method> {
    pub fn create(
        input: Arc<InputPort>,
        output: Arc<OutputPort>,
        method: Method,
        operator: Operator,
        params: Arc<AggregatorParams>,
        location_prefix: String,
    ) -> Box<dyn Processor> {
        Box::new(TransformAggregateSpillWriter::<Method> {
            method,
            input,
            output,
            params,
            operator,
            location_prefix,
            spilled_block: None,
            spilling_meta: None,
            spilling_future: None,
        })
    }
}

#[async_trait::async_trait]
impl<Method: HashMethodBounds> Processor for TransformAggregateSpillWriter<Method> {
    fn name(&self) -> String {
        String::from("TransformAggregateSpillWriter")
    }

    fn as_any(&mut self) -> &mut dyn Any {
        self
    }

    fn event(&mut self) -> Result<Event> {
        if self.output.is_finished() {
            self.input.finish();
            return Ok(Event::Finished);
        }

        if !self.output.can_push() {
            self.input.set_not_need_data();
            return Ok(Event::NeedConsume);
        }

        if self.spilling_future.is_some() {
            self.input.set_not_need_data();
            return Ok(Event::Async);
        }

        while let Some(spilled_block) = self.spilled_block.take() {
            if !spilled_block.is_empty() || spilled_block.get_meta().is_some() {
                self.output.push_data(Ok(spilled_block));
                return Ok(Event::NeedConsume);
            }
        }

        if self.spilling_meta.is_some() {
            self.input.set_not_need_data();
            return Ok(Event::Sync);
        }

        if self.input.has_data() {
            let mut data_block = self.input.pull_data().unwrap()?;

            if let Some(block_meta) = data_block
                .get_meta()
                .and_then(AggregateMeta::<Method, usize>::downcast_ref_from)
            {
                if matches!(block_meta, AggregateMeta::Spilling(_)) {
                    self.input.set_not_need_data();
                    let block_meta = data_block.take_meta().unwrap();
                    self.spilling_meta = AggregateMeta::<Method, usize>::downcast_from(block_meta);
                    return Ok(Event::Sync);
                }
            }

            self.output.push_data(Ok(data_block));
            return Ok(Event::NeedConsume);
        }

        if self.input.is_finished() {
            self.output.finish();
            return Ok(Event::Finished);
        }

        self.input.set_need_data();
        Ok(Event::NeedData)
    }

    fn process(&mut self) -> Result<()> {
        if let Some(spilling_meta) = self.spilling_meta.take() {
            if let AggregateMeta::Spilling(payload) = spilling_meta {
                let (spilled_block, spilling_future) = spilling_aggregate_payload(
                    self.operator.clone(),
                    &self.method,
                    &self.location_prefix,
                    &self.params,
                    payload,
                )?;

                self.spilled_block = Some(spilled_block);
                self.spilling_future = Some(spilling_future);

                return Ok(());
            }

            return Err(ErrorCode::Internal(""));
        }

        Ok(())
    }

    #[async_backtrace::framed]
    async fn async_process(&mut self) -> Result<()> {
        if let Some(spilling_future) = self.spilling_future.take() {
            return spilling_future.await;
        }

        Ok(())
    }
}

fn get_columns(data_block: DataBlock) -> Vec<BlockEntry> {
    data_block.columns().to_vec()
}

pub fn spilling_aggregate_payload<Method: HashMethodBounds>(
    operator: Operator,
    method: &Method,
    location_prefix: &str,
    params: &Arc<AggregatorParams>,
    mut payload: HashTablePayload<PartitionedHashMethod<Method>, usize>,
) -> Result<(DataBlock, BoxFuture<'static, Result<()>>)> {
    let unique_name = GlobalUniqName::unique();
    let location = format!("{}/{}", location_prefix, unique_name);

    let mut write_size = 0;
    let mut write_data = Vec::with_capacity(256);
    let mut spilled_buckets_payloads = Vec::with_capacity(256);
    for (bucket, inner_table) in payload.cell.hashtable.iter_tables_mut().enumerate() {
        if inner_table.len() == 0 {
            continue;
        }

        let now = Instant::now();
        let data_block = serialize_aggregate(method, params, inner_table)?;

        let begin = write_size;
        let columns = get_columns(data_block);
        let mut columns_data = Vec::with_capacity(columns.len());
        let mut columns_layout = Vec::with_capacity(columns.len());

        for column in columns.into_iter() {
            let column = column.value.as_column().unwrap();
            let column_data = serialize_column(column);
            write_size += column_data.len() as u64;
            columns_layout.push(column_data.len() as u64);
            columns_data.push(column_data);
        }

        // perf
        {
            metrics_inc_aggregate_spill_data_serialize_milliseconds(
                now.elapsed().as_millis() as u64
            );
        }

        write_data.push(columns_data);
        spilled_buckets_payloads.push(BucketSpilledPayload {
            bucket: bucket as isize,
            location: location.clone(),
            data_range: begin..write_size,
            columns_layout,
        });
    }

    Ok((
        DataBlock::empty_with_meta(AggregateMeta::<Method, usize>::create_spilled(
            spilled_buckets_payloads,
        )),
        Box::pin(async move {
            let instant = Instant::now();

            let mut write_bytes = 0;

            if !write_data.is_empty() {
                let mut writer = operator.writer(&location).await?;
                for write_bucket_data in write_data.into_iter() {
                    for data in write_bucket_data.into_iter() {
                        write_bytes += data.len();
                        writer.write(data).await?;
                    }
                }

                writer.close().await?;
            }

            // perf
            {
                metrics_inc_aggregate_spill_write_count();
                metrics_inc_aggregate_spill_write_bytes(write_bytes as u64);
                metrics_inc_aggregate_spill_write_milliseconds(instant.elapsed().as_millis() as u64);
            }

            info!(
                "Write aggregate spill {} successfully, elapsed: {:?}",
                location,
                instant.elapsed()
            );

            Ok(())
        }),
    ))
}
