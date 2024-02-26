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

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::Weak;
use std::time::SystemTime;

use dashmap::DashMap;
use databend_common_base::base::Progress;
use databend_common_base::runtime::Runtime;
use databend_common_catalog::catalog::CatalogManager;
use databend_common_catalog::merge_into_join::MergeIntoJoin;
use databend_common_catalog::query_kind::QueryKind;
use databend_common_catalog::runtime_filter_info::RuntimeFilterInfo;
use databend_common_catalog::statistics::data_cache_statistics::DataCacheMetrics;
use databend_common_catalog::table_context::MaterializedCtesBlocks;
use databend_common_catalog::table_context::StageAttachment;
use databend_common_exception::ErrorCode;
use databend_common_exception::Result;
use databend_common_meta_app::principal::OnErrorMode;
use databend_common_meta_app::principal::RoleInfo;
use databend_common_meta_app::principal::UserDefinedConnection;
use databend_common_meta_app::principal::UserInfo;
use databend_common_pipeline_core::processors::PlanProfile;
use databend_common_pipeline_core::InputError;
use databend_common_settings::Settings;
use databend_common_sql::IndexType;
use databend_common_storage::CopyStatus;
use databend_common_storage::DataOperator;
use databend_common_storage::MergeStatus;
use databend_common_storage::StorageMetrics;
use databend_common_users::UserApiProvider;
use parking_lot::Mutex;
use parking_lot::RwLock;
use uuid::Uuid;

use crate::clusters::Cluster;
use crate::pipelines::executor::PipelineExecutor;
use crate::sessions::query_affect::QueryAffect;
use crate::sessions::Session;
use crate::storages::Table;

type DatabaseAndTable = (String, String, String);

/// Data that needs to be shared in a query context.
pub struct QueryContextShared {
    /// total_scan_values for scan stats
    pub(in crate::sessions) total_scan_values: Arc<Progress>,
    /// scan_progress for scan metrics of datablocks (uncompressed)
    pub(in crate::sessions) scan_progress: Arc<Progress>,
    /// write_progress for write/commit metrics of datablocks (uncompressed)
    pub(in crate::sessions) write_progress: Arc<Progress>,
    /// Record how many bytes/rows have been spilled in join.
    pub(in crate::sessions) join_spill_progress: Arc<Progress>,
    /// Record how many bytes/rows have been spilled in agg.
    pub(in crate::sessions) agg_spill_progress: Arc<Progress>,
    /// Record how many bytes/rows have been spilled in group by
    pub(in crate::sessions) group_by_spill_progress: Arc<Progress>,
    /// result_progress for metrics of result datablocks (uncompressed)
    pub(in crate::sessions) result_progress: Arc<Progress>,
    pub(in crate::sessions) error: Arc<Mutex<Option<ErrorCode>>>,
    pub(in crate::sessions) warnings: Arc<Mutex<Vec<String>>>,
    pub(in crate::sessions) session: Arc<Session>,
    pub(in crate::sessions) runtime: Arc<RwLock<Option<Arc<Runtime>>>>,
    pub(in crate::sessions) init_query_id: Arc<RwLock<String>>,
    pub(in crate::sessions) cluster_cache: Arc<Cluster>,
    pub(in crate::sessions) running_query: Arc<RwLock<Option<String>>>,
    pub(in crate::sessions) running_query_kind: Arc<RwLock<Option<QueryKind>>>,
    pub(in crate::sessions) aborting: Arc<AtomicBool>,
    pub(in crate::sessions) tables_refs: Arc<Mutex<HashMap<DatabaseAndTable, Arc<dyn Table>>>>,
    pub(in crate::sessions) affect: Arc<Mutex<Option<QueryAffect>>>,
    pub(in crate::sessions) catalog_manager: Arc<CatalogManager>,
    pub(in crate::sessions) data_operator: DataOperator,
    pub(in crate::sessions) executor: Arc<RwLock<Weak<PipelineExecutor>>>,
    pub(in crate::sessions) stage_attachment: Arc<RwLock<Option<StageAttachment>>>,
    pub(in crate::sessions) created_time: SystemTime,
    // now it is only set in query_log::log_query_finished
    pub(in crate::sessions) finish_time: RwLock<Option<SystemTime>>,
    // DashMap<file_path, HashMap<ErrorCode::code, (ErrorCode, Number of occurrences)>>
    // We use this field to count maximum of one error found per data file.
    #[allow(clippy::type_complexity)]
    pub(in crate::sessions) on_error_map:
        Arc<RwLock<Option<Arc<DashMap<String, HashMap<u16, InputError>>>>>>,
    pub(in crate::sessions) on_error_mode: Arc<RwLock<Option<OnErrorMode>>>,
    pub(in crate::sessions) copy_status: Arc<CopyStatus>,
    pub(in crate::sessions) merge_status: Arc<RwLock<MergeStatus>>,
    /// partitions_sha for each table in the query. Not empty only when enabling query result cache.
    pub(in crate::sessions) partitions_shas: Arc<RwLock<Vec<String>>>,
    pub(in crate::sessions) cacheable: Arc<AtomicBool>,
    pub(in crate::sessions) can_scan_from_agg_index: Arc<AtomicBool>,
    pub(in crate::sessions) auto_compact_after_write: Arc<AtomicBool>,
    // Status info.
    pub(in crate::sessions) status: Arc<RwLock<String>>,

    // Client User-Agent
    pub(in crate::sessions) user_agent: Arc<RwLock<String>>,
    /// Key is (cte index, used_count), value contains cte's materialized blocks
    pub(in crate::sessions) materialized_cte_tables: MaterializedCtesBlocks,

    pub(in crate::sessions) query_profiles: Arc<RwLock<HashMap<Option<u32>, PlanProfile>>>,

    pub(in crate::sessions) runtime_filters: Arc<RwLock<HashMap<IndexType, RuntimeFilterInfo>>>,

    pub(in crate::sessions) merge_into_join: Arc<RwLock<MergeIntoJoin>>,

    // Records query level data cache metrics
    pub(in crate::sessions) query_cache_metrics: DataCacheMetrics,
}

impl QueryContextShared {
    pub fn try_create(
        session: Arc<Session>,
        cluster_cache: Arc<Cluster>,
    ) -> Result<Arc<QueryContextShared>> {
        Ok(Arc::new(QueryContextShared {
            session,
            cluster_cache,
            catalog_manager: CatalogManager::instance(),
            data_operator: DataOperator::instance(),
            init_query_id: Arc::new(RwLock::new(Uuid::new_v4().to_string())),
            total_scan_values: Arc::new(Progress::create()),
            scan_progress: Arc::new(Progress::create()),
            result_progress: Arc::new(Progress::create()),
            write_progress: Arc::new(Progress::create()),
            error: Arc::new(Mutex::new(None)),
            warnings: Arc::new(Mutex::new(vec![])),
            runtime: Arc::new(RwLock::new(None)),
            running_query: Arc::new(RwLock::new(None)),
            running_query_kind: Arc::new(RwLock::new(None)),
            aborting: Arc::new(AtomicBool::new(false)),
            tables_refs: Arc::new(Mutex::new(HashMap::new())),
            affect: Arc::new(Mutex::new(None)),
            executor: Arc::new(RwLock::new(Weak::new())),
            stage_attachment: Arc::new(RwLock::new(None)),
            created_time: SystemTime::now(),
            finish_time: Default::default(),
            on_error_map: Arc::new(RwLock::new(None)),
            on_error_mode: Arc::new(RwLock::new(None)),
            copy_status: Arc::new(Default::default()),
            merge_status: Arc::new(Default::default()),
            partitions_shas: Arc::new(RwLock::new(vec![])),
            cacheable: Arc::new(AtomicBool::new(true)),
            can_scan_from_agg_index: Arc::new(AtomicBool::new(true)),
            auto_compact_after_write: Arc::new(AtomicBool::new(true)),
            status: Arc::new(RwLock::new("null".to_string())),
            user_agent: Arc::new(RwLock::new("null".to_string())),
            materialized_cte_tables: Arc::new(Default::default()),
            join_spill_progress: Arc::new(Progress::create()),
            agg_spill_progress: Arc::new(Progress::create()),
            group_by_spill_progress: Arc::new(Progress::create()),
            query_cache_metrics: DataCacheMetrics::new(),
            query_profiles: Arc::new(RwLock::new(HashMap::new())),
            runtime_filters: Default::default(),
            merge_into_join: Default::default(),
        }))
    }

    pub fn set_error(&self, err: ErrorCode) {
        let mut guard = self.error.lock();
        *guard = Some(err);
    }

    pub fn get_error(&self) -> Option<ErrorCode> {
        let guard = self.error.lock();
        (*guard).clone()
    }

    pub fn push_warning(&self, warn: String) {
        let mut guard = self.warnings.lock();
        (*guard).push(warn);
    }

    pub fn pop_warnings(&self) -> Vec<String> {
        let mut guard = self.warnings.lock();
        let warnings = (*guard).clone();
        (*guard).clear();
        warnings
    }

    pub fn set_on_error_map(&self, map: Arc<DashMap<String, HashMap<u16, InputError>>>) {
        let mut guard = self.on_error_map.write();
        *guard = Some(map);
    }

    pub fn get_on_error_map(&self) -> Option<Arc<DashMap<String, HashMap<u16, InputError>>>> {
        self.on_error_map.read().as_ref().cloned()
    }

    pub fn get_on_error_mode(&self) -> Option<OnErrorMode> {
        self.on_error_mode.read().clone()
    }

    pub fn set_on_error_mode(&self, mode: OnErrorMode) {
        let mut guard = self.on_error_mode.write();
        *guard = Some(mode);
    }

    pub fn kill(&self, cause: ErrorCode) {
        self.set_error(cause.clone());
        self.aborting.store(true, Ordering::Release);

        if let Some(executor) = self.executor.read().upgrade() {
            executor.finish(Some(cause));
        }

        // TODO: Wait for the query to be processed (write out the last error)
    }

    pub fn get_cluster(&self) -> Arc<Cluster> {
        self.cluster_cache.clone()
    }

    pub fn get_current_catalog(&self) -> String {
        self.session.get_current_catalog()
    }

    pub fn get_aborting(&self) -> Arc<AtomicBool> {
        self.aborting.clone()
    }

    pub fn check_aborting(&self) -> Result<()> {
        if self.aborting.load(Ordering::Acquire) {
            Err(self.get_error().unwrap_or_else(|| {
                ErrorCode::AbortedQuery(
                    "Aborted query, because the server is shutting down or the query was killed.",
                )
            }))
        } else {
            Ok(())
        }
    }

    pub fn get_current_database(&self) -> String {
        self.session.get_current_database()
    }

    pub fn set_current_database(&self, new_database_name: String) {
        self.session.set_current_database(new_database_name);
    }

    pub fn get_current_user(&self) -> Result<UserInfo> {
        self.session.get_current_user()
    }

    pub fn get_current_role(&self) -> Option<RoleInfo> {
        self.session.get_current_role()
    }

    pub fn set_current_tenant(&self, tenant: String) {
        self.session.set_current_tenant(tenant);
    }

    /// Get all tables that already attached in this query.
    pub fn get_tables_refs(&self) -> Vec<Arc<dyn Table>> {
        let tables = self.tables_refs.lock();
        tables.values().cloned().collect()
    }

    pub fn get_data_metrics(&self) -> StorageMetrics {
        let tables = self.get_tables_refs();
        let metrics: Vec<Arc<StorageMetrics>> =
            tables.iter().filter_map(|v| v.get_data_metrics()).collect();
        StorageMetrics::merge(&metrics)
    }

    pub fn get_tenant(&self) -> String {
        self.session.get_current_tenant()
    }

    pub fn get_settings(&self) -> Arc<Settings> {
        self.session.get_settings()
    }

    pub fn attach_table(&self, catalog: &str, database: &str, name: &str, table: Arc<dyn Table>) {
        let mut tables_refs = self.tables_refs.lock();
        let table_meta_key = (catalog.to_string(), database.to_string(), name.to_string());

        if let Entry::Vacant(v) = tables_refs.entry(table_meta_key) {
            v.insert(table);
        };
    }

    #[async_backtrace::framed]
    pub async fn get_table(
        &self,
        catalog: &str,
        database: &str,
        table: &str,
    ) -> Result<Arc<dyn Table>> {
        // Always get same table metadata in the same query

        let table_meta_key = (catalog.to_string(), database.to_string(), table.to_string());

        let already_in_cache = { self.tables_refs.lock().contains_key(&table_meta_key) };
        match already_in_cache {
            false => self.get_table_to_cache(catalog, database, table).await,
            true => Ok(self
                .tables_refs
                .lock()
                .get(&table_meta_key)
                .ok_or_else(|| ErrorCode::Internal("Logical error, it's a bug."))?
                .clone()),
        }
    }

    #[async_backtrace::framed]
    async fn get_table_to_cache(
        &self,
        catalog: &str,
        database: &str,
        table: &str,
    ) -> Result<Arc<dyn Table>> {
        let tenant = self.get_tenant();
        let table_meta_key = (catalog.to_string(), database.to_string(), table.to_string());
        let catalog = self.catalog_manager.get_catalog(&tenant, catalog).await?;
        let cache_table = catalog.get_table(tenant.as_str(), database, table).await?;

        let mut tables_refs = self.tables_refs.lock();

        match tables_refs.entry(table_meta_key) {
            Entry::Occupied(v) => Ok(v.get().clone()),
            Entry::Vacant(v) => Ok(v.insert(cache_table).clone()),
        }
    }

    pub fn evict_table_from_cache(&self, catalog: &str, database: &str, table: &str) -> Result<()> {
        let table_meta_key = (catalog.to_string(), database.to_string(), table.to_string());
        let mut tables_refs = self.tables_refs.lock();
        tables_refs.remove(&table_meta_key);
        Ok(())
    }

    /// Init runtime when first get
    pub fn try_get_runtime(&self) -> Result<Arc<Runtime>> {
        let mut query_runtime = self.runtime.write();

        match &*query_runtime {
            Some(query_runtime) => Ok(query_runtime.clone()),
            None => {
                // To avoid possible deadlock, we should keep at least two threads.
                let runtime = Arc::new(Runtime::with_worker_threads(
                    2,
                    Some("query-ctx".to_string()),
                )?);
                *query_runtime = Some(runtime.clone());
                Ok(runtime)
            }
        }
    }

    pub fn get_runtime(&self) -> Option<Arc<Runtime>> {
        let query_runtime = self.runtime.read();
        (*query_runtime).clone()
    }

    pub fn attach_query_str(&self, kind: QueryKind, query: String) {
        {
            let mut running_query = self.running_query.write();
            *running_query = Some(short_sql(query));
        }

        {
            let mut running_query_kind = self.running_query_kind.write();
            *running_query_kind = Some(kind);
        }
    }

    pub fn get_query_str(&self) -> String {
        let running_query = self.running_query.read();
        running_query.as_ref().unwrap_or(&"".to_string()).clone()
    }

    pub fn get_query_kind(&self) -> QueryKind {
        let running_query_kind = self.running_query_kind.read();
        running_query_kind
            .as_ref()
            .cloned()
            .unwrap_or(QueryKind::Unknown)
    }

    pub fn get_connection_id(&self) -> String {
        self.session.get_id()
    }

    pub fn get_affect(&self) -> Option<QueryAffect> {
        let guard = self.affect.lock();
        (*guard).clone()
    }

    pub fn set_affect(&self, affect: QueryAffect) {
        let mut guard = self.affect.lock();
        *guard = Some(affect);
    }

    pub fn set_executor(&self, executor: Arc<PipelineExecutor>) -> Result<()> {
        let mut guard = self.executor.write();
        match self.check_aborting() {
            Ok(_) => {
                *guard = Arc::downgrade(&executor);
                Ok(())
            }
            Err(err) => {
                executor.finish(Some(err.clone()));
                Err(err)
            }
        }
    }

    pub fn get_stage_attachment(&self) -> Option<StageAttachment> {
        self.stage_attachment.read().clone()
    }

    pub fn attach_stage(&self, attachment: StageAttachment) {
        let mut stage_attachment = self.stage_attachment.write();
        *stage_attachment = Some(attachment);
    }

    pub fn get_created_time(&self) -> SystemTime {
        self.created_time
    }

    pub fn get_status_info(&self) -> String {
        let status = self.status.read();
        status.clone()
    }

    pub async fn get_connection(&self, name: &str) -> Result<UserDefinedConnection> {
        let user_mgr = UserApiProvider::instance();
        let tenant = self.get_tenant();
        user_mgr.get_connection(&tenant, name).await
    }

    pub fn get_query_cache_metrics(&self) -> &DataCacheMetrics {
        &self.query_cache_metrics
    }
}

impl Drop for QueryContextShared {
    fn drop(&mut self) {
        // last_query_id() should return the query_id of the last executed statement,
        // so we set it when the current context drops
        // to avoid returning the query_id of the current statement.
        self.session
            .session_ctx
            .update_query_ids_results(self.init_query_id.read().clone(), None)
    }
}

pub fn short_sql(sql: String) -> String {
    use unicode_segmentation::UnicodeSegmentation;
    let query = sql.trim_start();
    if query.as_bytes().len() >= 64 && query.as_bytes()[..6].eq_ignore_ascii_case(b"INSERT") {
        // keep first 64 graphemes
        String::from_utf8(
            query
                .graphemes(true)
                .take(64)
                .flat_map(|g| g.as_bytes().iter())
                .copied() // copied converts &u8 into u8
                .chain(b"...".iter().copied())
                .collect::<Vec<u8>>(),
        )
        .unwrap() // by construction, this cannot panic as we extracted unicode grapheme
    } else {
        sql
    }
}
