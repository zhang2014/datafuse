use std::sync::Arc;
use common_catalog::plan::PartInfoPtr;
use common_catalog::table_context::TableContext;
use common_pipeline_core::processors::port::OutputPort;
use common_pipeline_core::processors::Processor;
use common_pipeline_core::processors::processor::{Event, ProcessorPtr};
use std::any::Any;
use std::sync::atomic::AtomicUsize;
use common_datablocks::DataBlock;
use crate::io::BlockReader;
use crate::operations::read::parquet_data_source::DataSourceMeta;
use common_exception::Result;
use common_pipeline_sources::processors::sources::{SyncSource, SyncSourcer};

pub struct ReadParquetDataSource<const BLOCKING_IO: bool> {
    finished: bool,
    counter: Arc<AtomicUsize>,
    ctx: Arc<dyn TableContext>,
    batch_size: usize,
    block_reader: Arc<BlockReader>,

    output: Arc<OutputPort>,
    output_data: Option<(Vec<PartInfoPtr>, Vec<Vec<(usize, Vec<u8>)>>)>,
}

impl ReadParquetDataSource<true> {
    pub fn create(ctx: Arc<dyn TableContext>, output: Arc<OutputPort>, block_reader: Arc<BlockReader>, counter: Arc<AtomicUsize>) -> Result<ProcessorPtr> {
        let batch_size = ctx.get_settings().get_storage_fetch_part_num()? as usize;
        SyncSourcer::create(ctx.clone(), output.clone(), ReadParquetDataSource::<true> {
            ctx,
            output,
            counter,
            batch_size,
            block_reader,
            finished: false,
            output_data: None,
        })
    }
}

impl ReadParquetDataSource<false> {
    pub fn create(ctx: Arc<dyn TableContext>, output: Arc<OutputPort>, block_reader: Arc<BlockReader>, counter: Arc<AtomicUsize>) -> Result<ProcessorPtr> {
        let batch_size = ctx.get_settings().get_storage_fetch_part_num()? as usize;
        Ok(ProcessorPtr::create(Box::new(ReadParquetDataSource::<false> {
            ctx,
            output,
            counter,
            batch_size,
            block_reader,
            finished: false,
            output_data: None,
        })))
    }
}

impl SyncSource for ReadParquetDataSource<true> {
    const NAME: &'static str = "SyncReadParquetDataSource";

    fn generate(&mut self) -> Result<Option<DataBlock>> {
        match self.ctx.try_get_part() {
            None => Ok(None),
            Some(part) => Ok(Some(DataBlock::empty_with_meta(
                DataSourceMeta::create(
                    vec![part.clone()],
                    vec![self.block_reader.sync_read_columns_data(part)?],
                )
            ))),
        }
    }
}

#[async_trait::async_trait]
impl Processor for ReadParquetDataSource<false> {
    fn name(&self) -> String {
        String::from("AsyncReadParquetDataSource")
    }

    fn as_any(&mut self) -> &mut dyn Any {
        self
    }

    fn event(&mut self) -> Result<Event> {
        if self.finished {
            self.output.finish();
            return Ok(Event::Finished);
        }

        if self.output.is_finished() {
            return Ok(Event::Finished);
        }

        if !self.output.can_push() {
            return Ok(Event::NeedConsume);
        }

        if let Some((part, data)) = self.output_data.take() {
            let output = DataBlock::empty_with_meta(DataSourceMeta::create(part, data));
            self.output.push_data(Ok(output));
            // return Ok(Event::NeedConsume);
        }

        Ok(Event::Async)
    }

    async fn async_process(&mut self) -> Result<()> {
        let mut parts = Vec::with_capacity(self.batch_size);
        let mut chunks = Vec::with_capacity(self.batch_size);

        for _index in 0..self.batch_size {
            if let Some(part) = self.ctx.try_get_part() {
                parts.push(part.clone());
                let counter = self.counter.clone();
                let block_reader = self.block_reader.clone();

                chunks.push(async move {
                    let handler = common_base::base::tokio::spawn(async move {
                        block_reader.read_columns_data(part, Some(counter)).await
                    });
                    handler.await.unwrap()
                });
            }
        }

        if !parts.is_empty() {
            self.output_data = Some((parts, futures::future::try_join_all(chunks).await?));
            return Ok(());
        }

        self.finished = true;
        Ok(())
    }
}
