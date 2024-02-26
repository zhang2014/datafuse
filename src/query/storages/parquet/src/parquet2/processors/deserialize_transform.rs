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
use std::vec;

use databend_common_arrow::arrow::bitmap::Bitmap;
use databend_common_arrow::arrow::bitmap::MutableBitmap;
use databend_common_arrow::parquet::indexes::Interval;
use databend_common_base::base::Progress;
use databend_common_base::base::ProgressValues;
use databend_common_base::runtime::profile::Profile;
use databend_common_base::runtime::profile::ProfileStatisticsName;
use databend_common_catalog::plan::PartInfoPtr;
use databend_common_catalog::query_kind::QueryKind;
use databend_common_catalog::table_context::TableContext;
use databend_common_exception::Result;
use databend_common_expression::filter_helper::FilterHelpers;
use databend_common_expression::types::BooleanType;
use databend_common_expression::types::DataType;
use databend_common_expression::BlockEntry;
use databend_common_expression::BlockMetaInfoDowncast;
use databend_common_expression::DataBlock;
use databend_common_expression::DataSchemaRef;
use databend_common_expression::Evaluator;
use databend_common_expression::Scalar;
use databend_common_expression::Value;
use databend_common_functions::BUILTIN_FUNCTIONS;
use databend_common_pipeline_core::processors::Event;
use databend_common_pipeline_core::processors::InputPort;
use databend_common_pipeline_core::processors::OutputPort;
use databend_common_pipeline_core::processors::Processor;
use databend_common_pipeline_core::processors::ProcessorPtr;
use databend_common_storage::CopyStatus;
use databend_common_storage::FileStatus;
use opendal::services::Memory;
use opendal::Operator;

use super::source::Parquet2SourceMeta;
use crate::parquet2::parquet_reader::BlockIterator;
use crate::parquet2::parquet_reader::IndexedChunks;
use crate::parquet2::parquet_reader::Parquet2PartData;
use crate::parquet2::parquet_reader::Parquet2Reader;
use crate::parquet2::parquet_table::Parquet2PrewhereInfo;
use crate::parquet2::pruning::PartitionPruner;
use crate::parquet2::Parquet2RowGroupPart;
use crate::parquet_part::ParquetFilesPart;
use crate::parquet_part::ParquetPart;
use crate::ReadSettings;

pub trait SmallFilePrunner: Send + Sync {
    fn prune_one_file(
        &self,
        path: &str,
        op: &Operator,
        file_size: u64,
    ) -> Result<Vec<Parquet2RowGroupPart>>;
}

pub struct Parquet2DeserializeTransform {
    // Used for pipeline operations
    scan_progress: Arc<Progress>,
    input: Arc<InputPort>,
    output: Arc<OutputPort>,
    output_data: Vec<DataBlock>,

    // data from input
    parts: VecDeque<(PartInfoPtr, Parquet2PartData)>,
    current_row_group: Option<Box<dyn BlockIterator>>,

    src_schema: DataSchemaRef,
    output_schema: DataSchemaRef,

    // Used for prewhere reading and filtering
    prewhere_info: Option<Parquet2PrewhereInfo>,

    // Used for remain reading
    remain_reader: Arc<Parquet2Reader>,

    // Used for reading from small files
    source_reader: Arc<Parquet2Reader>,
    partition_pruner: Arc<PartitionPruner>,

    // used for collect num_rows for small files
    is_copy: bool,
    copy_status: Arc<CopyStatus>,

    merge_read_max_gap_size: u64,
    merge_read_max_range_size: u64,
}

impl Parquet2DeserializeTransform {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        ctx: Arc<dyn TableContext>,
        input: Arc<InputPort>,
        output: Arc<OutputPort>,
        src_schema: DataSchemaRef,
        output_schema: DataSchemaRef,
        prewhere_info: Option<Parquet2PrewhereInfo>,
        source_reader: Arc<Parquet2Reader>,
        remain_reader: Arc<Parquet2Reader>,
        partition_pruner: Arc<PartitionPruner>,
    ) -> Result<ProcessorPtr> {
        let scan_progress = ctx.get_scan_progress();

        Ok(ProcessorPtr::create(Box::new(
            Parquet2DeserializeTransform {
                scan_progress,
                input,
                output,
                output_data: vec![],

                parts: VecDeque::new(),

                current_row_group: None,
                src_schema,
                output_schema,

                prewhere_info,
                source_reader,
                remain_reader,
                partition_pruner,

                is_copy: matches!(ctx.get_query_kind(), QueryKind::CopyIntoTable),
                copy_status: ctx.get_copy_status(),
                merge_read_max_gap_size: ctx.get_settings().get_storage_io_min_bytes_for_seek()?,
                merge_read_max_range_size: ctx
                    .get_settings()
                    .get_storage_io_max_page_bytes_for_read()?,
            },
        )))
    }

    fn add_block(&mut self, data_block: DataBlock) -> Result<()> {
        let rows = data_block.num_rows();
        if rows == 0 {
            return Ok(());
        }
        let progress_values = ProgressValues {
            rows,
            bytes: data_block.memory_size(),
        };
        self.scan_progress.incr(&progress_values);
        Profile::record_usize_profile(ProfileStatisticsName::ScanBytes, data_block.memory_size());
        self.output_data.push(data_block);
        Ok(())
    }

    fn process_small_files(
        &mut self,
        part: &ParquetFilesPart,
        buffers: Vec<Vec<u8>>,
    ) -> Result<Vec<DataBlock>> {
        assert_eq!(part.files.len(), buffers.len());
        let mut blocks = Vec::new();
        for (path, data) in part.files.iter().zip(buffers.into_iter()) {
            blocks.extend(self.process_small_file(path.0.as_str(), data)?);
        }
        Ok(blocks)
    }

    fn process_small_file(&mut self, path: &str, data: Vec<u8>) -> Result<Vec<DataBlock>> {
        let mut res = Vec::new();
        let builder = Memory::default();
        let op = Operator::new(builder)?.finish();
        let data_size = data.len();
        let blocking_op = op.blocking();
        blocking_op.write(path, data)?;
        let parts = self
            .partition_pruner
            .prune_one_file(path, &op, data_size as u64)?;

        let settings = ReadSettings {
            max_gap_size: self.merge_read_max_gap_size,
            max_range_size: self.merge_read_max_range_size,
        };

        for part in parts {
            let readers = self.source_reader.sync_read_columns_data_by_merge_io(
                &settings,
                &part,
                &blocking_op,
            )?;
            if let Some(block) = self.process_row_group(&part, &mut readers.column_buffers()?)? {
                res.push(block)
            }
        }
        if self.is_copy {
            let num_rows_loaded = res.iter().map(|b| b.num_rows()).sum();
            self.copy_status.add_chunk(path, FileStatus {
                num_rows_loaded,
                error: None,
            })
        }
        Ok(res)
    }

    fn process_row_group(
        &mut self,
        part: &Parquet2RowGroupPart,
        column_chunks: &mut IndexedChunks,
    ) -> Result<Option<DataBlock>> {
        let row_selection = part
            .row_selection
            .as_ref()
            .map(|sel| intervals_to_bitmap(sel, part.num_rows));
        // this means it's empty projection
        if column_chunks.is_empty() {
            let data_block = DataBlock::new(vec![], part.num_rows);
            return Ok(Some(data_block));
        }

        let data_block = match self.prewhere_info.as_mut() {
            Some(Parquet2PrewhereInfo {
                func_ctx,
                reader,
                filter,
            }) => {
                let chunks = reader.read_from_merge_io(column_chunks)?;

                // only if there is not dictionary page, we can push down the row selection
                let can_push_down = chunks
                    .iter()
                    .all(|(id, _)| !part.column_metas[id].has_dictionary);
                let push_down = if can_push_down {
                    row_selection.clone()
                } else {
                    None
                };

                let mut prewhere_block = reader.deserialize(part, chunks, push_down)?;
                // Step 1: Read Prewhere columns and get the filter
                let evaluator = Evaluator::new(&prewhere_block, func_ctx, &BUILTIN_FUNCTIONS);
                let filter = evaluator
                    .run(filter)
                    .map_err(|e| e.add_message("eval prewhere filter failed:"))?
                    .try_downcast::<BooleanType>()
                    .unwrap();

                // Step 2: Apply the filter, if it's all filtered, we can skip the remain columns.
                if FilterHelpers::is_all_unset(&filter) {
                    return Ok(None);
                }

                if FilterHelpers::is_all_unset(&filter) {
                    return Ok(None);
                }

                // Step 4 Remove columns that are not needed for output. Use dummy column to replace them.
                let mut columns = prewhere_block.columns().to_vec();
                for (col, f) in columns.iter_mut().zip(reader.output_schema().fields()) {
                    if !self.output_schema.has_field(f.name()) {
                        *col = BlockEntry::new(DataType::Null, Value::Scalar(Scalar::Null));
                    }
                }

                // Step 5: Read remain columns.
                let chunks = self.remain_reader.read_from_merge_io(column_chunks)?;

                let can_push_down = chunks
                    .iter()
                    .all(|(id, _)| !part.column_metas[id].has_dictionary);
                let push_down = if can_push_down { row_selection } else { None };
                if push_down.is_some() || !can_push_down {
                    let remain_block = self.remain_reader.deserialize(part, chunks, push_down)?;

                    // Combine two blocks.
                    for col in remain_block.columns() {
                        prewhere_block.add_column(col.clone());
                    }

                    let block = prewhere_block.resort(&self.src_schema, &self.output_schema)?;
                    block.filter_boolean_value(&filter)
                } else {
                    // filter prewhere columns first.
                    let mut prewhere_block = prewhere_block.filter_boolean_value(&filter)?;
                    // If row_selection is None, we can push down the prewhere filter to remain data deserialization.
                    let remain_block = match filter {
                        Value::Column(bitmap) => {
                            self.remain_reader.deserialize(part, chunks, Some(bitmap))?
                        }
                        _ => self.remain_reader.deserialize(part, chunks, None)?, // all true
                    };
                    for col in remain_block.columns() {
                        prewhere_block.add_column(col.clone());
                    }

                    prewhere_block.resort(&self.src_schema, &self.output_schema)
                }
            }
            None => {
                // for now only use current_row_group when prewhere_info is None
                // for now only use current_row_group when prewhere_info is None
                let chunks = self.remain_reader.read_from_merge_io(column_chunks)?;
                let mut current_row_group =
                    self.remain_reader
                        .get_deserializer(part, chunks, row_selection)?;
                let block = match current_row_group.next() {
                    None => return Ok(None),
                    Some(block) => block?,
                };
                if current_row_group.has_next() {
                    self.current_row_group = Some(current_row_group)
                }
                Ok(block)
            }
        }?;

        Ok(Some(data_block))
    }
}

impl Processor for Parquet2DeserializeTransform {
    fn name(&self) -> String {
        String::from("ParquetDeserializeTransform")
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

        if let Some(data_block) = self.output_data.pop() {
            self.output.push_data(Ok(data_block));
            return Ok(Event::NeedConsume);
        }

        if self.current_row_group.is_some() || !self.parts.is_empty() {
            if !self.input.has_data() {
                self.input.set_need_data();
            }
            return Ok(Event::Sync);
        }

        if self.input.has_data() {
            let mut data_block = self.input.pull_data().unwrap()?;
            let source_meta = data_block.take_meta().unwrap();
            let source_meta = Parquet2SourceMeta::downcast_from(source_meta).unwrap();

            self.parts = VecDeque::from(source_meta.parts);
            return Ok(Event::Sync);
        }

        if self.input.is_finished() {
            self.output.finish();
            return Ok(Event::Finished);
        }

        self.input.set_need_data();
        Ok(Event::NeedData)
    }

    fn process(&mut self) -> Result<()> {
        if let Some(deserializer) = &mut self.current_row_group {
            let data_block = deserializer.next().unwrap()?;
            self.output_data.push(data_block);
            if !deserializer.has_next() {
                self.current_row_group = None
            }
            return Ok(());
        }

        if let Some((part, data)) = self.parts.pop_front() {
            let part = ParquetPart::from_part(&part)?;
            match (&part, data) {
                (ParquetPart::Parquet2RowGroup(rg), Parquet2PartData::RowGroup(mut reader)) => {
                    if let Some(block) = self.process_row_group(rg, &mut reader)? {
                        self.add_block(block)?;
                    }
                }
                (ParquetPart::ParquetFiles(p), Parquet2PartData::SmallFiles(buffers)) => {
                    let blocks = self.process_small_files(p, buffers)?;
                    if !blocks.is_empty() {
                        self.add_block(DataBlock::concat(&blocks)?)?;
                    }
                }
                _ => {
                    unreachable!("wrong type ParquetPartData for ParquetPart")
                }
            }
        }

        Ok(())
    }
}

/// Convert intervals to a bitmap. The `intervals` represents the row selection across `num_rows`.
fn intervals_to_bitmap(interval: &[Interval], num_rows: usize) -> Bitmap {
    debug_assert!(
        interval.is_empty()
            || interval.last().unwrap().start + interval.last().unwrap().length <= num_rows
    );

    let mut bitmap = MutableBitmap::with_capacity(num_rows);
    let mut offset = 0;

    for intv in interval {
        bitmap.extend_constant(intv.start - offset, false);
        bitmap.extend_constant(intv.length, true);
        offset = intv.start + intv.length;
    }
    bitmap.extend_constant(num_rows - offset, false);

    bitmap.into()
}
