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

use std::sync::Arc;

use databend_common_exception::Result;
use databend_common_expression::BlockPartitionStream;
use databend_common_expression::DataBlock;
use databend_common_pipeline_core::processors::InputPort;
use databend_common_pipeline_core::processors::OutputPort;
use databend_common_pipeline_core::processors::ProcessorPtr;
use databend_common_pipeline_transforms::processors::Transform;
use databend_common_pipeline_transforms::processors::Transformer;
use databend_common_pipeline_transforms::{AccumulatingTransform, AccumulatingTransformer};

use super::exchange_transform_shuffle::ExchangeShuffleMeta;
use crate::servers::flight::v1::scatter::FlightScatter;

pub struct ScatterTransform {
    scatter: Arc<Box<dyn FlightScatter>>,
}

impl ScatterTransform {
    pub fn create(
        input: Arc<InputPort>,
        output: Arc<OutputPort>,
        scatter: Arc<Box<dyn FlightScatter>>,
    ) -> ProcessorPtr {
        ProcessorPtr::create(Transformer::create(input, output, ScatterTransform {
            scatter,
        }))
    }
}

impl Transform for ScatterTransform {
    const NAME: &'static str = "ScatterTransform";

    fn name(&self) -> String {
        format!("ScatterTransform({})", self.scatter.name())
    }

    fn transform(&mut self, data: DataBlock) -> Result<DataBlock> {
        let blocks = self.scatter.execute(data)?;

        Ok(DataBlock::empty_with_meta(ExchangeShuffleMeta::create(
            blocks,
        )))
    }
}

pub struct StreamScatterTransform {
    num_partitions: usize,
    scatter: Arc<Box<dyn FlightScatter>>,
    stream_partition: BlockPartitionStream,
}

impl StreamScatterTransform {
    pub fn create(
        input: Arc<InputPort>,
        output: Arc<OutputPort>,
        num_partitions: usize,
        scatter: Arc<Box<dyn FlightScatter>>,
        stream_partition: BlockPartitionStream,
    ) -> ProcessorPtr {
        ProcessorPtr::create(AccumulatingTransformer::create(input, output, StreamScatterTransform {
            scatter,
            num_partitions,
            stream_partition,
        }))
    }
}

impl AccumulatingTransform for StreamScatterTransform {
    const NAME: &'static str = "StreamScatterTransform";

    fn transform(&mut self, data: DataBlock) -> Result<Vec<DataBlock>> {
        let partitions = self.scatter.partitions(&data)?;
        let blocks = self.stream_partition.partition(&partitions, data, true);

        if blocks.is_empty() {
            return Ok(vec![]);
        }

        let mut buckets = Vec::with_capacity(self.num_partitions);
        for _index in 0..self.num_partitions {
            buckets.push(DataBlock::empty());
        }

        for (idx, block) in blocks {
            buckets[idx] = block;
        }

        Ok(vec![DataBlock::empty_with_meta(
            ExchangeShuffleMeta::create(buckets),
        )])
    }

    fn on_finish(&mut self, output: bool) -> Result<Vec<DataBlock>> {
        if output {
            let partitions = self.stream_partition.partition_ids();

            if partitions.is_empty() {
                return Ok(vec![]);
            }

            let mut buckets = Vec::with_capacity(self.num_partitions);
            for _index in 0..self.num_partitions {
                buckets.push(DataBlock::empty());
            }

            for pid in partitions {
                if let Some(block) = self.stream_partition.finalize_partition(pid) {
                    buckets[pid] = block;
                }
            }

            return Ok(vec![DataBlock::empty_with_meta(
                ExchangeShuffleMeta::create(buckets),
            )]);
        }

        Ok(vec![])
    }
}
