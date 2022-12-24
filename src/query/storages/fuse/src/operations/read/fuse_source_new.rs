use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use serde::{Deserializer, Serializer};
use tracing::info;
use common_catalog::plan::PartInfoPtr;
use common_catalog::table_context::TableContext;
use common_datablocks::{BlockMetaInfo, BlockMetaInfoPtr, DataBlock};
use common_pipeline_core::processors::Processor;
use common_pipeline_core::processors::processor::{Event, ProcessorPtr};
use crate::io::BlockReader;
use common_exception::{ErrorCode, Result};
use common_pipeline_core::Pipeline;
use common_pipeline_core::processors::port::OutputPort;
use common_pipeline_transforms::processors::transforms::{Transform, Transformer};
use crate::operations::read::native_data_source_deserializer::NativeDeserializeDataTransform;
use crate::operations::read::native_data_source_reader::ReadNativeDataSource;
use crate::operations::read::parquet_data_source::DataSourceMeta;
use crate::operations::read::parquet_data_source_deserializer::DeserializeDataTransform;
use crate::operations::read::parquet_data_source_reader::ReadParquetDataSource;

pub fn build_fuse_native_source_pipeline(
    ctx: Arc<dyn TableContext>,
    pipeline: &mut Pipeline,
    block_reader: Arc<BlockReader>,
    max_threads: usize,
    max_io_requests: usize,
) -> Result<()> {
    match block_reader.support_blocking_api() {
        true => {
            pipeline.add_source(|output| ReadNativeDataSource::<true>::create(
                ctx.clone(),
                output,
                block_reader.clone(),
            ), max_threads)?;
        }
        false => {
            info!("read block data adjust max io requests:{}", max_io_requests);
            pipeline.add_source(|output| ReadNativeDataSource::<false>::create(
                ctx.clone(),
                output,
                block_reader.clone(),
            ), max_io_requests)?;

            pipeline.resize(std::cmp::min(max_threads, max_io_requests))?;

            info!(
                "read block pipeline resize from:{} to:{}",
                max_io_requests, pipeline.output_len()
            );
        }
    };

    pipeline.add_transform(|transform_input, transform_output| {
        NativeDeserializeDataTransform::create(
            ctx.clone(),
            block_reader.clone(),
            transform_input,
            transform_output,
        )
    })
}


pub fn build_fuse_parquet_source_pipeline(
    ctx: Arc<dyn TableContext>,
    pipeline: &mut Pipeline,
    block_reader: Arc<BlockReader>,
    max_threads: usize,
    max_io_requests: usize,
) -> Result<()> {
    let fetch_count = Arc::new(AtomicUsize::new(0));
    match block_reader.support_blocking_api() {
        true => {
            pipeline.add_source(|output| ReadParquetDataSource::<true>::create(
                ctx.clone(),
                output,
                block_reader.clone(),
                fetch_count.clone(),
            ), max_threads)?;
        }
        false => {
            info!("read block data adjust max io requests:{}", max_io_requests);
            pipeline.add_source(|output| ReadParquetDataSource::<false>::create(
                ctx.clone(),
                output,
                block_reader.clone(),
                fetch_count.clone(),
            ), max_io_requests)?;

            pipeline.resize(std::cmp::min(max_threads, max_io_requests))?;

            info!(
                "read block pipeline resize from:{} to:{}",
                max_io_requests, pipeline.output_len()
            );
        }
    };

    pipeline.set_on_finished(move |_s| {
        info!("Request s3 counter: {}", fetch_count.load(Ordering::Relaxed));
        Ok(())
    });

    pipeline.add_transform(|transform_input, transform_output| {
        DeserializeDataTransform::create(
            ctx.clone(),
            block_reader.clone(),
            transform_input,
            transform_output,
        )
    })
}
