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

use databend_common_catalog::catalog::CatalogManager;
use databend_common_exception::Result;
use databend_common_storages_stream::stream_table::StreamTable;
use log::debug;
use poem::web::Json;
use poem::web::Path;
use poem::web::Query;
use poem::IntoResponse;
use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Serialize, Deserialize)]
pub struct StreamStatusQuery {
    pub database: Option<String>,
    pub stream_name: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StreamStatusResponse {
    has_data: bool,
    params: StreamStatusQuery,
}

#[async_backtrace::framed]
async fn check_stream_status(
    tenant: &str,
    params: Query<StreamStatusQuery>,
) -> Result<StreamStatusResponse> {
    let catalog = CatalogManager::instance().get_default_catalog()?;
    let db_name = params.database.clone().unwrap_or("default".to_string());
    let tbl = catalog
        .get_table(tenant, &db_name, &params.stream_name)
        .await?;
    let stream = StreamTable::try_from_table(tbl.as_ref())?;
    let (base_table_ident, _) = catalog
        .get_table_meta_by_id(stream.source_table_id())
        .await?;
    Ok(StreamStatusResponse {
        has_data: base_table_ident.seq != stream.offset(),
        params: params.0,
    })
}

// This handler returns the status of a stream. It's only enabled in management mode.
#[poem::handler]
#[async_backtrace::framed]
pub async fn stream_status_handler(
    Path(tenant): Path<String>,
    params: Query<StreamStatusQuery>,
) -> poem::Result<impl IntoResponse> {
    debug!(
        "check_stream_stauts: tenant: {}, params: {:?}",
        tenant, params
    );

    let resp = check_stream_status(&tenant, params)
        .await
        .map_err(poem::error::InternalServerError)?;
    Ok(Json(resp))
}
