// Copyright 2021 Datafuse Labs.
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

use std::io::BufReader;
use std::sync::Arc;
use futures_util::io::Cursor;

use common_arrow::native::read::reader::PaReader;
use common_arrow::native::read::PaReadBuf;
use common_catalog::plan::PartInfoPtr;
use common_exception::{ErrorCode, Result};
use opendal::Object;
use common_base::runtime::{Runtime, ThreadPool, TrySpawn};

use crate::fuse_part::FusePartInfo;
use crate::io::BlockReader;

// Native storage format

pub type Reader = Box<dyn PaReadBuf + Send + Sync>;

impl BlockReader {
    pub async fn async_read_native_columns_data(
        &self,
        part: PartInfoPtr,
        thread_pool: Option<Arc<Runtime>>,
    ) -> Result<Vec<(usize, PaReader<Reader>)>> {
        let part = FusePartInfo::from_part(&part)?;
        let columns = self.projection.project_column_leaves(&self.column_leaves)?;
        let indices = Self::build_projection_indices(&columns);
        let mut join_handlers = Vec::with_capacity(indices.len());

        for (index, field) in indices {
            let column_meta = &part.columns_meta[&index];
            join_handlers.push(Self::read_native_columns_data(
                self.operator.object(&part.location),
                index,
                column_meta.offset,
                column_meta.len,
                column_meta.num_values,
                field.data_type().clone(),
                thread_pool.clone(),
            ));
        }

        futures::future::try_join_all(join_handlers).await
    }

    pub async fn read_native_columns_data(
        o: Object,
        index: usize,
        offset: u64,
        length: u64,
        rows: u64,
        data_type: common_arrow::arrow::datatypes::DataType,
        thread_pool: Option<Arc<Runtime>>,
    ) -> Result<(usize, PaReader<Reader>)> {
        if let Some(runtime) = thread_pool {
            let reader = o.range_reader(offset..offset + length).await?;
            let content_length = reader.content_length();
            let bytes_reader = reader.into_reader();

            let handler = runtime.spawn(async move {
                let buffer = Vec::with_capacity(content_length as usize);
                let mut bs = Cursor::new(buffer);
                futures::io::copy(bytes_reader, &mut bs).await?;
                Result::<Vec<u8>, ErrorCode>::Ok(bs.into_inner())
            });

            let reader = handler.await.unwrap()?;
            let reader: Reader = Box::new(std::io::Cursor::new(reader));
            let fuse_reader = PaReader::new(reader, data_type, rows as usize, vec![]);
            return Ok((index, fuse_reader));
        }

        let reader = o.range_read(offset..offset + length).await?;
        let reader: Reader = Box::new(std::io::Cursor::new(reader));
        let fuse_reader = PaReader::new(reader, data_type, rows as usize, vec![]);
        Ok((index, fuse_reader))
    }

    pub fn sync_read_native_columns_data(
        &self,
        part: PartInfoPtr,
    ) -> Result<Vec<(usize, PaReader<Reader>)>> {
        let part = FusePartInfo::from_part(&part)?;

        let columns = self.projection.project_column_leaves(&self.column_leaves)?;
        let indices = Self::build_projection_indices(&columns);
        let mut results = Vec::with_capacity(indices.len());

        for (index, field) in indices {
            let column_meta = &part.columns_meta[&index];

            let op = self.operator.clone();

            let location = part.location.clone();
            let offset = column_meta.offset;
            let length = column_meta.len;

            let result = Self::sync_read_native_column(
                op.object(&location),
                index,
                offset,
                length,
                column_meta.num_values,
                field.data_type().clone(),
            );

            results.push(result?);
        }

        Ok(results)
    }

    pub fn sync_read_native_column(
        o: Object,
        index: usize,
        offset: u64,
        length: u64,
        rows: u64,
        data_type: common_arrow::arrow::datatypes::DataType,
    ) -> Result<(usize, PaReader<Reader>)> {
        let reader = o.blocking_range_reader(offset..offset + length)?;
        let reader: Reader = Box::new(BufReader::new(reader));
        let fuse_reader = PaReader::new(reader, data_type, rows as usize, vec![]);
        Ok((index, fuse_reader))
    }
}
