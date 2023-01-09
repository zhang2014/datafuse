//  Copyright 2022 Datafuse Labs.
//
//  Licensed under the Apache License, Version 2.0 (the "License");
//  you may not use this file except in compliance with the License.
//  You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
//  Unless required by applicable law or agreed to in writing, software
//  distributed under the License is distributed on an "AS IS" BASIS,
//  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//  See the License for the specific language governing permissions and
//  limitations under the License.

use std::io::Cursor;
use std::io::Read;
use std::mem;
use std::sync::Arc;

use common_exception::ErrorCode;
use common_exception::Result;
use common_expression::TableSchemaRef;
use common_expression::TypeDeserializer;
use common_expression::TypeDeserializerImpl;
use common_formats::FieldDecoder;
use common_formats::FieldDecoderCSV;
use common_formats::FieldDecoderRowBased;
use common_formats::FileFormatOptionsExt;
use common_formats::RecordDelimiter;
use common_io::cursor_ext::*;
use common_io::format_diagnostic::verbose_char;
use common_meta_types::OnErrorMode;
use common_meta_types::StageFileFormatType;
use csv_core::ReadRecordResult;

use crate::processors::sources::input_formats::impls::input_format_tsv::format_column_error;
use crate::processors::sources::input_formats::input_format_text::AligningStateCommon;
use crate::processors::sources::input_formats::input_format_text::AligningStateTextBased;
use crate::processors::sources::input_formats::input_format_text::BlockBuilder;
use crate::processors::sources::input_formats::input_format_text::InputFormatTextBase;
use crate::processors::sources::input_formats::input_format_text::RowBatch;
use crate::processors::sources::input_formats::InputContext;
use crate::processors::sources::input_formats::SplitInfo;

pub struct InputFormatCSV {}

impl InputFormatCSV {
    fn read_row(
        field_decoder: &FieldDecoderCSV,
        buf: &[u8],
        deserializers: &mut [TypeDeserializerImpl],
        schema: &TableSchemaRef,
        field_ends: &[usize],
        path: &str,
        row_index: usize,
    ) -> Result<()> {
        let mut field_start = 0;
        for (c, deserializer) in deserializers.iter_mut().enumerate() {
            let field_end = field_ends[c];
            let col_data = &buf[field_start..field_end];
            let mut reader = Cursor::new(col_data);
            if reader.eof() {
                deserializer.de_default();
            } else {
                if let Err(e) = field_decoder.read_field(deserializer, &mut reader, true) {
                    let err_msg = format_column_error(schema, c, col_data, &e.message());
                    return Err(csv_error(&err_msg, path, row_index));
                };
                let mut next = [0u8; 1];
                let readn = reader.read(&mut next[..])?;
                if readn > 0 {
                    let remaining = col_data.len() - reader.position() as usize + 1;
                    let err_msg = format!(
                        "bad field end, remain {} bytes, next char is {}",
                        remaining,
                        verbose_char(next[0])
                    );

                    let err_msg = format_column_error(schema, c, col_data, &err_msg);
                    return Err(csv_error(&err_msg, path, row_index));
                }
            }
            field_start = field_end;
        }
        Ok(())
    }
}

impl InputFormatTextBase for InputFormatCSV {
    type AligningState = CsvReaderState;

    fn format_type() -> StageFileFormatType {
        StageFileFormatType::Csv
    }

    fn create_field_decoder(options: &FileFormatOptionsExt) -> Arc<dyn FieldDecoder> {
        Arc::new(FieldDecoderCSV::create(options))
    }

    fn deserialize(builder: &mut BlockBuilder<Self>, batch: RowBatch) -> Result<()> {
        let columns = &mut builder.mutable_columns;
        let n_column = columns.len();
        let mut start = 0usize;
        let mut num_rows = 0usize;
        let start_row = batch.start_row.expect("must success");
        let mut field_end_idx = 0;
        let field_decoder = builder
            .field_decoder
            .as_any()
            .downcast_ref::<FieldDecoderCSV>()
            .expect("must success");
        for (i, end) in batch.row_ends.iter().enumerate() {
            let buf = &batch.data[start..*end];
            if let Err(e) = Self::read_row(
                field_decoder,
                buf,
                columns,
                &builder.ctx.schema,
                &batch.field_ends[field_end_idx..field_end_idx + n_column],
                &batch.path,
                start_row + i,
            ) {
                if builder.ctx.on_error_mode == OnErrorMode::Continue {
                    columns.iter_mut().for_each(|c| {
                        // check if parts of columns inserted data, if so, pop it.
                        if c.len() > num_rows {
                            c.pop_data_value().expect("must success");
                        }
                    });
                    start = *end;
                    field_end_idx += n_column;
                    continue;
                } else {
                    return Err(e);
                }
            }
            start = *end;
            field_end_idx += n_column;
            num_rows += 1;
        }
        Ok(())
    }
}

pub struct CsvReaderState {
    common: AligningStateCommon,
    #[allow(unused)]
    ctx: Arc<InputContext>,
    split_info: Arc<SplitInfo>,
    pub reader: csv_core::Reader,

    // remain from last read batch
    pub out: Vec<u8>,
    pub field_ends: Vec<usize>,
    pub n_end: usize,

    num_fields: usize,
    path: String,
}

impl CsvReaderState {
    fn check_num_field(
        expect: usize,
        actual: usize,
        field_ends: &[usize],
        path: &str,
        rows: usize,
    ) -> Result<()> {
        if actual < expect {
            Err(csv_error(
                &format!("expect {} fields, only found {} ", expect, actual),
                path,
                rows,
            ))
        } else if actual > expect + 1
            || (actual == expect + 1 && field_ends[expect] != field_ends[expect - 1])
        {
            Err(csv_error(
                &format!("too many fields, expect {}, got {}", expect, actual),
                path,
                rows,
            ))
        } else {
            Ok(())
        }
    }
}

impl AligningStateTextBased for CsvReaderState {
    fn try_create(ctx: &Arc<InputContext>, split_info: &Arc<SplitInfo>) -> Result<Self> {
        let escape = if ctx.format_options.stage.escape.is_empty() {
            None
        } else {
            Some(ctx.format_options.stage.escape.as_bytes()[0])
        };
        let reader = csv_core::ReaderBuilder::new()
            .delimiter(ctx.format_options.get_field_delimiter())
            .quote(ctx.format_options.stage.quote.as_bytes()[0])
            .escape(escape)
            .terminator(match ctx.format_options.get_record_delimiter()? {
                RecordDelimiter::Crlf => csv_core::Terminator::CRLF,
                RecordDelimiter::Any(v) => csv_core::Terminator::Any(v),
            })
            .build();
        Ok(Self {
            common: AligningStateCommon::create(ctx, split_info, false),
            ctx: ctx.clone(),
            split_info: split_info.clone(),
            reader,
            out: vec![],
            field_ends: vec![0; ctx.schema.num_fields() + 6],
            n_end: 0,
            num_fields: ctx.schema.num_fields(),
            path: split_info.file.path.clone(),
        })
    }

    fn align(&mut self, buf_in: &[u8]) -> Result<Vec<RowBatch>> {
        let num_fields = self.num_fields;
        let field_ends = &mut self.field_ends[..];
        let start_row = self.common.rows;
        self.common.offset += buf_in.len();

        // assume n_out <= n_in for read_record
        let mut out_tmp = vec![0u8; buf_in.len()];
        let mut endlen = self.n_end;
        let mut buf = buf_in;

        while self.common.rows_to_skip > 0 {
            let (result, n_in, _, n_end) =
                self.reader
                    .read_record(buf, &mut out_tmp, &mut field_ends[endlen..]);
            buf = &buf[n_in..];
            endlen += n_end;

            match result {
                ReadRecordResult::InputEmpty => {
                    self.n_end = endlen;
                    return Ok(vec![]);
                }
                ReadRecordResult::OutputFull => {
                    return Err(output_full_error(
                        &self.split_info.file.path,
                        self.common.rows,
                    ));
                }
                ReadRecordResult::OutputEndsFull => {
                    return Err(output_ends_full_error(
                        num_fields,
                        field_ends.len(),
                        &self.split_info.file.path,
                        self.common.rows,
                    ));
                }
                ReadRecordResult::Record => {
                    Self::check_num_field(
                        num_fields,
                        endlen,
                        field_ends,
                        &self.split_info.file.path,
                        self.common.rows,
                    )?;

                    self.common.rows_to_skip -= 1;
                    tracing::debug!(
                        "csv aligner: skip a header row, remain {}",
                        self.common.rows_to_skip
                    );
                    self.common.rows += 1;
                    endlen = 0;
                }
                ReadRecordResult::End => {
                    return Err(csv_error(
                        "unexpect eof in header",
                        &self.split_info.file.path,
                        self.common.rows,
                    ));
                }
            }
        }

        let mut out_pos = 0usize;
        let mut row_batch_end: usize = 0;

        let last_batch_remain_len = self.out.len();

        let mut row_batch = RowBatch {
            data: vec![],
            row_ends: vec![],
            field_ends: vec![],
            path: self.split_info.file.path.to_string(),
            batch_id: self.common.batch_id,
            offset: 0,
            start_row: Some(self.common.rows),
        };

        while !buf.is_empty() {
            let (result, n_in, n_out, n_end) =
                self.reader
                    .read_record(buf, &mut out_tmp[out_pos..], &mut field_ends[endlen..]);
            buf = &buf[n_in..];
            endlen += n_end;
            out_pos += n_out;
            match result {
                ReadRecordResult::InputEmpty => break,
                ReadRecordResult::OutputFull => {
                    return Err(output_full_error(
                        &self.split_info.file.path,
                        start_row + row_batch.row_ends.len(),
                    ));
                }
                ReadRecordResult::OutputEndsFull => {
                    return Err(output_ends_full_error(
                        num_fields,
                        field_ends.len(),
                        &self.split_info.file.path,
                        start_row + row_batch.row_ends.len(),
                    ));
                }
                ReadRecordResult::Record => {
                    Self::check_num_field(
                        num_fields,
                        endlen,
                        field_ends,
                        &self.split_info.file.path,
                        start_row + row_batch.row_ends.len(),
                    )?;
                    row_batch
                        .field_ends
                        .extend_from_slice(&field_ends[..num_fields]);
                    row_batch.row_ends.push(last_batch_remain_len + out_pos);
                    endlen = 0;
                    row_batch_end = out_pos;
                }
                ReadRecordResult::End => {
                    return Err(csv_error(
                        "unexpect eof, should not happen",
                        &self.split_info.file.path,
                        start_row + row_batch.row_ends.len(),
                    ));
                }
            }
        }

        self.n_end = endlen;
        out_tmp.truncate(out_pos);
        if row_batch.row_ends.is_empty() {
            tracing::debug!(
                "csv aligner: {} + {} bytes => 0 rows",
                self.out.len(),
                buf_in.len(),
            );
            self.out.extend_from_slice(&out_tmp);
            Ok(vec![])
        } else {
            let last_remain = mem::take(&mut self.out);

            self.common.batch_id += 1;
            self.common.rows += row_batch.row_ends.len();
            self.out.extend_from_slice(&out_tmp[row_batch_end..]);

            tracing::debug!(
                "csv aligner: {} + {} bytes => {} rows + {} bytes remain",
                last_remain.len(),
                buf_in.len(),
                row_batch.row_ends.len(),
                self.out.len()
            );

            out_tmp.truncate(row_batch_end);
            row_batch.data = if last_remain.is_empty() {
                out_tmp
            } else {
                vec![last_remain, out_tmp].concat()
            };
            Ok(vec![row_batch])
        }
    }

    fn align_flush(&mut self) -> Result<Vec<RowBatch>> {
        let mut res = vec![];
        let num_fields = self.num_fields;
        let field_ends = &mut self.field_ends[..];
        let in_tmp = Vec::new();
        let mut out_tmp = vec![0u8; 1];
        let mut endlen = self.n_end;

        if self.common.rows_to_skip > 0 {
            let (result, _, _, n_end) =
                self.reader
                    .read_record(&in_tmp, &mut out_tmp, &mut field_ends[endlen..]);
            endlen += n_end;

            return match result {
                ReadRecordResult::InputEmpty => {
                    self.n_end = endlen;
                    Ok(vec![])
                }
                ReadRecordResult::OutputFull => {
                    Err(output_full_error(&self.path, self.common.rows))
                }
                ReadRecordResult::OutputEndsFull => Err(output_ends_full_error(
                    num_fields,
                    field_ends.len(),
                    &self.split_info.file.path,
                    self.common.rows,
                )),
                ReadRecordResult::Record => {
                    Self::check_num_field(
                        num_fields,
                        endlen,
                        field_ends,
                        &self.path,
                        self.common.rows,
                    )?;

                    self.common.rows_to_skip -= 1;
                    tracing::debug!(
                        "csv aligner: skip a header row, remain {}",
                        self.common.rows_to_skip
                    );
                    Ok(vec![])
                }
                ReadRecordResult::End => Err(csv_error(
                    "unexpect eof in header",
                    &self.path,
                    self.common.rows,
                )),
            };
        }

        let num_fields = self.num_fields;
        let field_ends = &mut self.field_ends[..];
        let start_row = self.common.rows;

        let in_tmp = Vec::new();
        let mut out_tmp = vec![0u8; 1];
        let mut endlen = self.n_end;

        let last_batch_remain_len = self.out.len();

        let (result, _, n_out, n_end) =
            self.reader
                .read_record(&in_tmp, &mut out_tmp, &mut field_ends[endlen..]);

        endlen += n_end;

        match result {
            ReadRecordResult::InputEmpty => {
                return Err(csv_error(
                    "unexpect eof",
                    &self.split_info.file.path,
                    start_row,
                ));
            }
            ReadRecordResult::OutputFull => {
                return Err(output_full_error(&self.split_info.file.path, start_row));
            }
            ReadRecordResult::OutputEndsFull => {
                return Err(output_ends_full_error(
                    num_fields,
                    field_ends.len(),
                    &self.split_info.file.path,
                    start_row,
                ));
            }
            ReadRecordResult::Record => {
                Self::check_num_field(num_fields, endlen, field_ends, &self.path, start_row)?;
                let data = mem::take(&mut self.out);

                let row_batch = RowBatch {
                    data,
                    row_ends: vec![last_batch_remain_len + n_out],
                    field_ends: field_ends[..num_fields].to_vec(),
                    path: self.split_info.file.path.to_string(),
                    batch_id: self.common.batch_id,
                    offset: 0,
                    start_row: Some(self.common.rows),
                };
                res.push(row_batch);

                self.common.batch_id += 1;
                self.common.rows += 1;

                tracing::debug!(
                    "csv aligner flush last row of {} bytes",
                    last_batch_remain_len,
                );
            }
            ReadRecordResult::End => {}
        }
        Ok(res)
    }
}

fn output_full_error(path: &str, row: usize) -> ErrorCode {
    csv_error("output more than input", path, row)
}

fn output_ends_full_error(
    num_fields_expect: usize,
    num_fields_actual: usize,
    path: &str,
    row: usize,
) -> ErrorCode {
    csv_error(
        &format!(
            "too many fields, expect {}, got more than {}",
            num_fields_expect, num_fields_actual
        ),
        path,
        row,
    )
}

fn csv_error(msg: &str, path: &str, row: usize) -> ErrorCode {
    let row = row + 1;
    let msg = format!("fail to parse CSV {}:{} {} ", path, row, msg);

    ErrorCode::BadBytes(msg)
}
