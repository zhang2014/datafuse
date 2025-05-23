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
use std::time::Instant;

use databend_common_arrow::arrow::datatypes::Schema as ArrowSchema;
use databend_common_arrow::arrow::io::flight::default_ipc_fields;
use databend_common_arrow::arrow::io::flight::WriteOptions;
use databend_common_arrow::arrow::io::ipc::write::Compression;
use databend_common_arrow::arrow::io::ipc::IpcField;
use databend_common_base::base::GlobalUniqName;
use databend_common_base::base::ProgressValues;
use databend_common_catalog::table_context::TableContext;
use databend_common_exception::Result;
use databend_common_expression::arrow::serialize_column;
use databend_common_expression::types::ArgType;
use databend_common_expression::types::ArrayType;
use databend_common_expression::types::Int64Type;
use databend_common_expression::types::UInt64Type;
use databend_common_expression::types::ValueType;
use databend_common_expression::BlockMetaInfoDowncast;
use databend_common_expression::DataBlock;
use databend_common_expression::DataSchemaRef;
use databend_common_expression::FromData;
use databend_common_hashtable::HashtableLike;
use databend_common_metrics::transform::*;
use databend_common_pipeline_core::processors::InputPort;
use databend_common_pipeline_core::processors::OutputPort;
use databend_common_pipeline_core::processors::Processor;
use databend_common_pipeline_transforms::processors::BlockMetaTransform;
use databend_common_pipeline_transforms::processors::BlockMetaTransformer;
use databend_common_settings::FlightCompression;
use futures_util::future::BoxFuture;
use log::info;
use opendal::Operator;

use crate::api::serialize_block;
use crate::api::ExchangeShuffleMeta;
use crate::pipelines::processors::transforms::aggregator::aggregate_meta::AggregateMeta;
use crate::pipelines::processors::transforms::aggregator::aggregate_meta::HashTablePayload;
use crate::pipelines::processors::transforms::aggregator::exchange_defines;
use crate::pipelines::processors::transforms::aggregator::serialize_aggregate;
use crate::pipelines::processors::transforms::aggregator::spilling_aggregate_payload as local_spilling_aggregate_payload;
use crate::pipelines::processors::transforms::aggregator::AggregateSerdeMeta;
use crate::pipelines::processors::transforms::aggregator::AggregatorParams;
use crate::pipelines::processors::transforms::aggregator::FlightSerialized;
use crate::pipelines::processors::transforms::aggregator::FlightSerializedMeta;
use crate::pipelines::processors::transforms::aggregator::SerializeAggregateStream;
use crate::pipelines::processors::transforms::group_by::HashMethodBounds;
use crate::pipelines::processors::transforms::group_by::PartitionedHashMethod;
use crate::sessions::QueryContext;

pub struct TransformExchangeAggregateSerializer<Method: HashMethodBounds> {
    ctx: Arc<QueryContext>,
    method: Method,
    local_pos: usize,
    options: WriteOptions,
    ipc_fields: Vec<IpcField>,

    operator: Operator,
    location_prefix: String,
    params: Arc<AggregatorParams>,
}

impl<Method: HashMethodBounds> TransformExchangeAggregateSerializer<Method> {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        ctx: Arc<QueryContext>,
        input: Arc<InputPort>,
        output: Arc<OutputPort>,
        method: Method,
        operator: Operator,
        location_prefix: String,
        params: Arc<AggregatorParams>,
        compression: Option<FlightCompression>,
        schema: DataSchemaRef,
        local_pos: usize,
    ) -> Box<dyn Processor> {
        let arrow_schema = ArrowSchema::from(schema.as_ref());
        let ipc_fields = default_ipc_fields(&arrow_schema.fields);
        let compression = match compression {
            None => None,
            Some(compression) => match compression {
                FlightCompression::Lz4 => Some(Compression::LZ4),
                FlightCompression::Zstd => Some(Compression::ZSTD),
            },
        };

        BlockMetaTransformer::create(input, output, TransformExchangeAggregateSerializer::<
            Method,
        > {
            ctx,
            method,
            params,
            operator,
            location_prefix,
            local_pos,
            ipc_fields,
            options: WriteOptions { compression },
        })
    }
}

impl<Method: HashMethodBounds> BlockMetaTransform<ExchangeShuffleMeta>
    for TransformExchangeAggregateSerializer<Method>
{
    const NAME: &'static str = "TransformExchangeAggregateSerializer";

    fn transform(&mut self, meta: ExchangeShuffleMeta) -> Result<DataBlock> {
        let mut serialized_blocks = Vec::with_capacity(meta.blocks.len());
        for (index, mut block) in meta.blocks.into_iter().enumerate() {
            if block.is_empty() && block.get_meta().is_none() {
                serialized_blocks.push(FlightSerialized::DataBlock(block));
                continue;
            }

            match AggregateMeta::<Method, usize>::downcast_from(block.take_meta().unwrap()) {
                None => unreachable!(),
                Some(AggregateMeta::Spilled(_)) => unreachable!(),
                Some(AggregateMeta::Serialized(_)) => unreachable!(),
                Some(AggregateMeta::BucketSpilled(_)) => unreachable!(),
                Some(AggregateMeta::Partitioned { .. }) => unreachable!(),
                Some(AggregateMeta::Spilling(payload)) => {
                    serialized_blocks.push(FlightSerialized::Future(
                        match index == self.local_pos {
                            true => local_spilling_aggregate_payload(
                                self.ctx.clone(),
                                self.operator.clone(),
                                &self.method,
                                &self.location_prefix,
                                &self.params,
                                payload,
                            )?,
                            false => spilling_aggregate_payload(
                                self.ctx.clone(),
                                self.operator.clone(),
                                &self.method,
                                &self.location_prefix,
                                &self.params,
                                payload,
                            )?,
                        },
                    ));
                }

                Some(AggregateMeta::HashTable(payload)) => {
                    if index == self.local_pos {
                        serialized_blocks.push(FlightSerialized::DataBlock(block.add_meta(
                            Some(Box::new(AggregateMeta::<Method, usize>::HashTable(payload))),
                        )?));
                        continue;
                    }

                    let mut stream =
                        SerializeAggregateStream::create(&self.method, &self.params, payload);
                    let bucket = stream.payload.bucket;
                    serialized_blocks.push(FlightSerialized::DataBlock(match stream.next() {
                        None => DataBlock::empty(),
                        Some(data_block) => {
                            serialize_block(bucket, data_block?, &self.ipc_fields, &self.options)?
                        }
                    }));
                }
            };
        }

        Ok(DataBlock::empty_with_meta(FlightSerializedMeta::create(
            serialized_blocks,
        )))
    }
}

fn spilling_aggregate_payload<Method: HashMethodBounds>(
    ctx: Arc<QueryContext>,
    operator: Operator,
    method: &Method,
    location_prefix: &str,
    params: &Arc<AggregatorParams>,
    mut payload: HashTablePayload<PartitionedHashMethod<Method>, usize>,
) -> Result<BoxFuture<'static, Result<DataBlock>>> {
    let unique_name = GlobalUniqName::unique();
    let location = format!("{}/{}", location_prefix, unique_name);

    let mut write_size = 0;
    let mut write_data = Vec::with_capacity(256);
    let mut buckets_column_data = Vec::with_capacity(256);
    let mut data_range_start_column_data = Vec::with_capacity(256);
    let mut data_range_end_column_data = Vec::with_capacity(256);
    let mut columns_layout_column_data = Vec::with_capacity(256);
    // Record how many rows are spilled.
    let mut rows = 0;

    for (bucket, inner_table) in payload.cell.hashtable.iter_tables_mut().enumerate() {
        if inner_table.len() == 0 {
            continue;
        }

        let now = Instant::now();
        let data_block = serialize_aggregate(method, params, inner_table)?;
        rows += data_block.num_rows();

        let old_write_size = write_size;
        let columns = data_block.columns().to_vec();
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
        buckets_column_data.push(bucket as i64);
        data_range_end_column_data.push(write_size);
        columns_layout_column_data.push(columns_layout);
        data_range_start_column_data.push(old_write_size);
    }

    Ok(Box::pin(async move {
        if !write_data.is_empty() {
            let instant = Instant::now();

            let mut write_bytes = 0;
            let mut writer = operator
                .writer_with(&location)
                .buffer(8 * 1024 * 1024)
                .await?;
            for write_bucket_data in write_data.into_iter() {
                for data in write_bucket_data.into_iter() {
                    write_bytes += data.len();
                    writer.write(data).await?;
                }
            }

            writer.close().await?;

            // perf
            {
                metrics_inc_aggregate_spill_write_count();
                metrics_inc_aggregate_spill_write_bytes(write_bytes as u64);
                metrics_inc_aggregate_spill_write_milliseconds(instant.elapsed().as_millis() as u64);
            }

            {
                {
                    let progress_val = ProgressValues {
                        rows,
                        bytes: write_bytes,
                    };
                    ctx.get_aggregate_spill_progress().incr(&progress_val);
                }
            }

            info!(
                "Write aggregate spill {} successfully, elapsed: {:?}",
                location,
                instant.elapsed()
            );

            let data_block = DataBlock::new_from_columns(vec![
                Int64Type::from_data(buckets_column_data),
                UInt64Type::from_data(data_range_start_column_data),
                UInt64Type::from_data(data_range_end_column_data),
                ArrayType::upcast_column(ArrayType::<UInt64Type>::column_from_iter(
                    columns_layout_column_data
                        .into_iter()
                        .map(|x| UInt64Type::column_from_iter(x.into_iter(), &[])),
                    &[],
                )),
            ]);

            let data_block = data_block.add_meta(Some(AggregateSerdeMeta::create_spilled(
                -1,
                location.clone(),
                0..0,
                vec![],
            )))?;

            let ipc_fields = exchange_defines::spilled_ipc_fields();
            let write_options = exchange_defines::spilled_write_options();
            return serialize_block(-1, data_block, ipc_fields, write_options);
        }

        Ok(DataBlock::empty())
    }))
}
