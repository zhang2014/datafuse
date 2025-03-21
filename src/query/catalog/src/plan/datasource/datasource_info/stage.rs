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

use std::fmt::Debug;
use std::fmt::Formatter;
use std::sync::Arc;

use databend_common_exception::Result;
use databend_common_expression::Scalar;
use databend_common_expression::TableSchema;
use databend_common_expression::TableSchemaRef;
use databend_common_meta_app::principal::StageInfo;
use databend_common_storage::init_stage_operator;
use databend_common_storage::StageFileInfo;
use databend_common_storage::StageFilesInfo;

#[derive(serde::Serialize, serde::Deserialize, Clone, PartialEq, Eq)]
pub struct StageTableInfo {
    pub schema: TableSchemaRef,
    pub default_values: Option<Vec<Scalar>>,
    pub files_info: StageFilesInfo,
    pub stage_info: StageInfo,
    pub files_to_copy: Option<Vec<StageFileInfo>>,
    pub is_select: bool,
}

impl StageTableInfo {
    pub fn schema(&self) -> Arc<TableSchema> {
        self.schema.clone()
    }

    pub fn desc(&self) -> String {
        self.stage_info.stage_name.clone()
    }

    #[async_backtrace::framed]
    pub async fn list_files(&self, max_files: Option<usize>) -> Result<Vec<StageFileInfo>> {
        let op = init_stage_operator(&self.stage_info)?;
        let infos = self
            .files_info
            .list(&op, false, max_files)
            .await?
            .into_iter()
            .collect::<Vec<_>>();
        Ok(infos)
    }
}

impl Debug for StageTableInfo {
    // Ignore the schema.
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.stage_info)
    }
}
