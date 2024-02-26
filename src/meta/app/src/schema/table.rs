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

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::ops::Deref;
use std::sync::Arc;

use chrono::DateTime;
use chrono::Utc;
use databend_common_exception::Result;
use databend_common_expression::FieldIndex;
use databend_common_expression::TableField;
use databend_common_expression::TableSchema;
use databend_common_meta_types::MatchSeq;
use databend_common_meta_types::MetaId;
use maplit::hashmap;

use super::CreateOption;
use crate::schema::database::DatabaseNameIdent;
use crate::share::ShareNameIdent;
use crate::share::ShareSpec;
use crate::share::ShareTableInfoMap;
use crate::storage::StorageParams;

/// Globally unique identifier of a version of TableMeta.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, Eq, PartialEq, Default)]
pub struct TableIdent {
    /// Globally unique id to identify a table.
    pub table_id: u64,

    /// seq AKA version of this table snapshot.
    ///
    /// Any change to a table causes the seq to increment, e.g. insert or delete rows, update schema etc.
    /// But renaming a table should not affect the seq, since the table itself does not change.
    /// The tuple (table_id, seq) identifies a unique and consistent table snapshot.
    ///
    /// A seq is not guaranteed to be consecutive.
    pub seq: u64,
}

impl TableIdent {
    pub fn new(table_id: u64, seq: u64) -> Self {
        TableIdent { table_id, seq }
    }
}

impl Display for TableIdent {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "table_id:{}, ver:{}", self.table_id, self.seq)
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Eq, PartialEq, Default)]
pub struct TableNameIdent {
    pub tenant: String,
    pub db_name: String,
    pub table_name: String,
}

impl TableNameIdent {
    pub fn new(
        tenant: impl Into<String>,
        db_name: impl Into<String>,
        table_name: impl Into<String>,
    ) -> TableNameIdent {
        TableNameIdent {
            tenant: tenant.into(),
            db_name: db_name.into(),
            table_name: table_name.into(),
        }
    }

    pub fn tenant(&self) -> &str {
        &self.tenant
    }

    pub fn table_name(&self) -> String {
        self.table_name.clone()
    }

    pub fn db_name_ident(&self) -> DatabaseNameIdent {
        DatabaseNameIdent {
            tenant: self.tenant.clone(),
            db_name: self.db_name.clone(),
        }
    }
}

impl Display for TableNameIdent {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "'{}'.'{}'.'{}'",
            self.tenant, self.db_name, self.table_name
        )
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Eq, PartialEq, Default)]
pub struct DBIdTableName {
    pub db_id: u64,
    pub table_name: String,
}

impl Display for DBIdTableName {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}.'{}'", self.db_id, self.table_name)
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Eq, PartialEq, Default)]
pub struct TableId {
    pub table_id: u64,
}

impl TableId {
    pub fn new(table_id: u64) -> Self {
        TableId { table_id }
    }
}

impl Display for TableId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "TableId{{{}}}", self.table_id)
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Eq, PartialEq, Default)]
pub struct TableIdListKey {
    pub db_id: u64,
    pub table_name: String,
}

impl Display for TableIdListKey {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}.'{}'", self.db_id, self.table_name)
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Eq, PartialEq, Default)]
pub enum DatabaseType {
    #[default]
    NormalDB,
    ShareDB(ShareNameIdent),
}

impl Display for DatabaseType {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            DatabaseType::NormalDB => {
                write!(f, "normal database")
            }
            DatabaseType::ShareDB(share_ident) => {
                write!(
                    f,
                    "share database: {}-{}",
                    share_ident.tenant, share_ident.share_name
                )
            }
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Eq, PartialEq, Default)]
pub struct TableInfo {
    pub ident: TableIdent,

    /// For a table it is `db_name.table_name`.
    /// For a table function, it is `table_name(args)`
    pub desc: String,

    /// `name` is meant to be used with table-function.
    /// Table-function is identified by `name`.
    /// A table in the contrast, can only be identified by table-id.
    pub name: String,

    /// The essential information about a table definition.
    ///
    /// It is about what a table actually is.
    /// `name`, `id` or `version` is not included in the table structure definition.
    pub meta: TableMeta,

    pub tenant: String,

    // table belong to which type of database.
    pub db_type: DatabaseType,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Eq, PartialEq, Default)]
pub struct TableStatistics {
    /// Number of rows
    pub number_of_rows: u64,
    // Size of data in bytes
    pub data_bytes: u64,
    /// Size of data compressed in bytes
    pub compressed_data_bytes: u64,
    /// Size of index data in bytes
    pub index_data_bytes: u64,

    /// number of segments
    pub number_of_segments: Option<u64>,

    /// number of blocks
    pub number_of_blocks: Option<u64>,
}

/// The essential state that defines what a table is.
///
/// It is what a meta store just needs to save.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Eq, PartialEq)]
pub struct TableMeta {
    pub schema: Arc<TableSchema>,
    pub catalog: String,
    pub engine: String,
    pub engine_options: BTreeMap<String, String>,
    pub storage_params: Option<StorageParams>,
    pub part_prefix: String,
    pub options: BTreeMap<String, String>,
    // The default cluster key.
    pub default_cluster_key: Option<String>,
    // All cluster keys that have been defined.
    pub cluster_keys: Vec<String>,
    // The sequence number of default_cluster_key in cluster_keys.
    pub default_cluster_key_id: Option<u32>,
    pub created_on: DateTime<Utc>,
    pub updated_on: DateTime<Utc>,
    pub comment: String,
    pub field_comments: Vec<String>,

    // if used in CreateTableReq, this field MUST set to None.
    pub drop_on: Option<DateTime<Utc>>,
    pub statistics: TableStatistics,
    // shared by share_id
    pub shared_by: BTreeSet<u64>,
    pub column_mask_policy: Option<BTreeMap<String, String>>,
}

impl TableMeta {
    pub fn add_column(
        &mut self,
        field: &TableField,
        comment: &str,
        index: FieldIndex,
    ) -> Result<()> {
        self.fill_field_comments();

        let mut new_schema = self.schema.as_ref().to_owned();
        new_schema.add_column(field, index)?;
        self.schema = Arc::new(new_schema);
        self.field_comments.insert(index, comment.to_owned());
        Ok(())
    }

    pub fn drop_column(&mut self, column: &str) -> Result<()> {
        self.fill_field_comments();

        let mut new_schema = self.schema.as_ref().to_owned();
        let index = new_schema.drop_column(column)?;
        self.field_comments.remove(index);
        self.schema = Arc::new(new_schema);
        Ok(())
    }

    /// To fix the field comments panic.
    pub fn fill_field_comments(&mut self) {
        let num_fields = self.schema.num_fields();
        // If the field comments is confused, fill it with empty string.
        if self.field_comments.len() < num_fields {
            self.field_comments = vec!["".to_string(); num_fields];
        }
    }
}

impl TableInfo {
    /// Create a TableInfo with only db, table, schema
    pub fn simple(db: &str, table: &str, schema: Arc<TableSchema>) -> TableInfo {
        TableInfo {
            desc: format!("'{}'.'{}'", db, table),
            name: table.to_string(),
            meta: TableMeta {
                schema,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    pub fn new(db_name: &str, table_name: &str, ident: TableIdent, meta: TableMeta) -> TableInfo {
        TableInfo {
            ident,
            desc: format!("'{}'.'{}'", db_name, table_name),
            name: table_name.to_string(),
            meta,
            ..Default::default()
        }
    }

    pub fn schema(&self) -> Arc<TableSchema> {
        self.meta.schema.clone()
    }

    pub fn options(&self) -> &BTreeMap<String, String> {
        &self.meta.options
    }

    pub fn catalog(&self) -> &str {
        &self.meta.catalog
    }

    pub fn engine(&self) -> &str {
        &self.meta.engine
    }

    pub fn engine_options(&self) -> &BTreeMap<String, String> {
        &self.meta.engine_options
    }

    pub fn field_comments(&self) -> &Vec<String> {
        &self.meta.field_comments
    }

    #[must_use]
    pub fn set_schema(mut self, schema: Arc<TableSchema>) -> TableInfo {
        self.meta.schema = schema;
        self
    }
}

impl Default for TableMeta {
    fn default() -> Self {
        TableMeta {
            schema: Arc::new(TableSchema::empty()),
            catalog: "default".to_string(),
            engine: "".to_string(),
            engine_options: BTreeMap::new(),
            storage_params: None,
            part_prefix: "".to_string(),
            options: BTreeMap::new(),
            default_cluster_key: None,
            cluster_keys: vec![],
            default_cluster_key_id: None,
            created_on: Utc::now(),
            updated_on: Utc::now(),
            comment: "".to_string(),
            field_comments: vec![],
            drop_on: None,
            statistics: Default::default(),
            shared_by: BTreeSet::new(),
            column_mask_policy: None,
        }
    }
}

impl TableMeta {
    pub fn push_cluster_key(mut self, cluster_key: String) -> Self {
        self.cluster_keys.push(cluster_key.clone());
        self.default_cluster_key = Some(cluster_key);
        self.default_cluster_key_id = Some(self.cluster_keys.len() as u32 - 1);
        self
    }

    pub fn cluster_key(&self) -> Option<(u32, String)> {
        self.default_cluster_key_id
            .zip(self.default_cluster_key.clone())
    }
}

impl Display for TableMeta {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Engine: {}={:?}, Schema: {:?}, Options: {:?}, FieldComments: {:?} CreatedOn: {:?} DropOn: {:?}",
            self.engine,
            self.engine_options,
            self.schema,
            self.options,
            self.field_comments,
            self.created_on,
            self.drop_on,
        )
    }
}

impl Display for TableInfo {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "DB.Table: {}, Table: {}-{}, Engine: {}",
            self.desc, self.name, self.ident, self.meta.engine
        )
    }
}

/// Save table name id list history.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default, Eq, PartialEq)]
pub struct TableIdList {
    pub id_list: Vec<u64>,
}

impl TableIdList {
    pub fn new() -> TableIdList {
        TableIdList::default()
    }

    pub fn len(&self) -> usize {
        self.id_list.len()
    }

    pub fn id_list(&self) -> &Vec<u64> {
        &self.id_list
    }

    pub fn append(&mut self, table_id: u64) {
        self.id_list.push(table_id);
    }

    pub fn is_empty(&self) -> bool {
        self.id_list.is_empty()
    }

    pub fn pop(&mut self) -> Option<u64> {
        self.id_list.pop()
    }

    pub fn last(&mut self) -> Option<&u64> {
        self.id_list.last()
    }
}

impl Display for TableIdList {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "DB.Table id list: {:?}", self.id_list)
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct CreateTableReq {
    pub create_option: CreateOption,
    pub name_ident: TableNameIdent,
    pub table_meta: TableMeta,
}

impl CreateTableReq {
    pub fn tenant(&self) -> &str {
        &self.name_ident.tenant
    }
    pub fn db_name(&self) -> &str {
        &self.name_ident.db_name
    }
    pub fn table_name(&self) -> &str {
        &self.name_ident.table_name
    }
}

impl Display for CreateTableReq {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        if let CreateOption::CreateIfNotExists(if_not_exists) = self.create_option {
            write!(
                f,
                "create_table(if_not_exists={}):{}/{}-{}={}",
                if_not_exists,
                self.tenant(),
                self.db_name(),
                self.table_name(),
                self.table_meta
            )
        } else {
            write!(
                f,
                "create_or_replace_table:{}/{}-{}={}",
                self.tenant(),
                self.db_name(),
                self.table_name(),
                self.table_meta
            )
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Eq, PartialEq)]
pub struct CreateTableReply {
    pub table_id: u64,
    pub new_table: bool,
    pub spec_vec: Option<(Vec<ShareSpec>, Vec<ShareTableInfoMap>)>,
}

/// Drop table by id.
///
/// Dropping a table requires just `table_id`, but when dropping a table, it also needs to update
/// the count of tables belonging to a tenant, which require tenant information.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct DropTableByIdReq {
    pub if_exists: bool,

    pub tenant: String,

    pub tb_id: MetaId,

    pub table_name: String,

    pub db_id: MetaId,
}

impl DropTableByIdReq {
    pub fn tb_id(&self) -> MetaId {
        self.tb_id
    }
}

impl Display for DropTableByIdReq {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "drop_table_by_id(if_exists={}):{}",
            self.if_exists,
            self.tb_id(),
        )
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct DropTableReply {
    pub spec_vec: Option<(Vec<ShareSpec>, Vec<ShareTableInfoMap>)>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct UndropTableReq {
    pub name_ident: TableNameIdent,
}

impl UndropTableReq {
    pub fn tenant(&self) -> &str {
        &self.name_ident.tenant
    }
    pub fn db_name(&self) -> &str {
        &self.name_ident.db_name
    }
    pub fn table_name(&self) -> &str {
        &self.name_ident.table_name
    }
}

impl Display for UndropTableReq {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "undrop_table:{}/{}-{}",
            self.tenant(),
            self.db_name(),
            self.table_name()
        )
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct UndropTableReply {}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct RenameTableReq {
    pub if_exists: bool,
    pub name_ident: TableNameIdent,
    pub new_db_name: String,
    pub new_table_name: String,
}

impl RenameTableReq {
    pub fn tenant(&self) -> &str {
        &self.name_ident.tenant
    }
    pub fn db_name(&self) -> &str {
        &self.name_ident.db_name
    }
    pub fn table_name(&self) -> &str {
        &self.name_ident.table_name
    }
}

impl Display for RenameTableReq {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "rename_table:{}/{}-{}=>{}-{}",
            self.tenant(),
            self.db_name(),
            self.table_name(),
            self.new_db_name,
            self.new_table_name
        )
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct RenameTableReply {
    pub table_id: u64,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct UpsertTableOptionReq {
    pub table_id: u64,
    pub seq: MatchSeq,

    /// Add or remove options
    ///
    /// Some(String): add or update an option.
    /// None: delete an option.
    pub options: HashMap<String, Option<String>>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct UpdateStreamMetaReq {
    pub stream_id: u64,
    pub seq: MatchSeq,
    pub options: BTreeMap<String, String>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct UpdateTableMetaReq {
    pub table_id: u64,
    pub seq: MatchSeq,
    pub new_table_meta: TableMeta,
    pub copied_files: Option<UpsertTableCopiedFileReq>,
    pub update_stream_meta: Vec<UpdateStreamMetaReq>,
    pub deduplicated_label: Option<String>,
}

impl UpsertTableOptionReq {
    pub fn new(
        table_ident: &TableIdent,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> UpsertTableOptionReq {
        UpsertTableOptionReq {
            table_id: table_ident.table_id,
            seq: MatchSeq::Exact(table_ident.seq),
            options: hashmap! {key.into() => Some(value.into())},
        }
    }
}

impl Display for UpsertTableOptionReq {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "upsert-table-options: table-id:{}({:?}) = {:?}",
            self.table_id, self.seq, self.options
        )
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum SetTableColumnMaskPolicyAction {
    // new mask name, old mask name(if any)
    Set(String, Option<String>),
    // prev mask name
    Unset(String),
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SetTableColumnMaskPolicyReq {
    pub tenant: String,
    pub table_id: u64,
    pub seq: MatchSeq,
    pub column: String,
    pub action: SetTableColumnMaskPolicyAction,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SetTableColumnMaskPolicyReply {
    pub share_table_info: Option<Vec<ShareTableInfoMap>>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct UpsertTableOptionReply {
    pub share_table_info: Option<Vec<ShareTableInfoMap>>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct UpdateTableMetaReply {
    pub share_table_info: Option<Vec<ShareTableInfoMap>>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct GetTableReq {
    pub inner: TableNameIdent,
}

impl Deref for GetTableReq {
    type Target = TableNameIdent;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl From<(&str, &str, &str)> for GetTableReq {
    fn from(db_table: (&str, &str, &str)) -> Self {
        Self::new(db_table.0, db_table.1, db_table.2)
    }
}

impl GetTableReq {
    pub fn new(
        tenant: impl Into<String>,
        db_name: impl Into<String>,
        table_name: impl Into<String>,
    ) -> GetTableReq {
        GetTableReq {
            inner: TableNameIdent::new(tenant, db_name, table_name),
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ListTableReq {
    pub inner: DatabaseNameIdent,
}

impl Deref for ListTableReq {
    type Target = DatabaseNameIdent;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl ListTableReq {
    pub fn new(tenant: impl Into<String>, db_name: impl Into<String>) -> ListTableReq {
        ListTableReq {
            inner: DatabaseNameIdent {
                tenant: tenant.into(),
                db_name: db_name.into(),
            },
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum TableInfoFilter {
    // if datatime is some, filter only dropped tables which drop time before that,
    // else filter all dropped tables
    Dropped(Option<DateTime<Utc>>),
    // filter all dropped tables, including all tables in dropped database and dropped tables in exist dbs,
    // in this case, `ListTableReq`.db_name will be ignored
    // return Tables in two cases:
    //  1) if database drop before date time, then all table in this db will be return;
    //  2) else, return all the tables drop before data time.
    AllDroppedTables(Option<DateTime<Utc>>),
    // return all tables, ignore drop on time.
    All,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ListDroppedTableReq {
    pub inner: DatabaseNameIdent,
    pub filter: TableInfoFilter,
    pub limit: Option<usize>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum DroppedId {
    // db id, db name
    Db(u64, String),
    // db id, table id, table name
    Table(u64, u64, String),
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ListDroppedTableResp {
    pub drop_table_infos: Vec<Arc<TableInfo>>,
    pub drop_ids: Vec<DroppedId>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct GcDroppedTableReq {
    pub tenant: String,
    pub drop_ids: Vec<DroppedId>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct GcDroppedTableResp {}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Eq, PartialEq, Default)]
pub struct CountTablesKey {
    pub tenant: String,
}

impl Display for CountTablesKey {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "'{}'", self.tenant)
    }
}

impl CountTablesKey {
    pub fn new(tenant: impl ToString) -> Self {
        Self {
            tenant: tenant.to_string(),
        }
    }
}

/// count tables for a tenant
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct CountTablesReq {
    pub tenant: String,
}

#[derive(Debug)]
pub struct CountTablesReply {
    pub count: u64,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default, Eq, PartialEq)]
pub struct TableIdToName {
    pub table_id: u64,
}

impl Display for TableIdToName {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "TableIdToName{{{}}}", self.table_id)
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Eq, PartialEq, Default)]
pub struct TableCopiedFileNameIdent {
    pub table_id: u64,
    pub file: String,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Eq, PartialEq, Default)]
pub struct TableCopiedFileInfo {
    pub etag: Option<String>,
    pub content_length: u64,
    pub last_modified: Option<DateTime<Utc>>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct GetTableCopiedFileReq {
    pub table_id: u64,
    pub files: Vec<String>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct GetTableCopiedFileReply {
    pub file_info: BTreeMap<String, TableCopiedFileInfo>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct UpsertTableCopiedFileReq {
    pub file_info: BTreeMap<String, TableCopiedFileInfo>,
    pub expire_at: Option<u64>,
    pub fail_if_duplicated: bool,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct UpsertTableCopiedFileReply {}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct TruncateTableReq {
    pub table_id: u64,
    /// Specify the max number copied file to delete in every sub-transaction.
    ///
    /// By default it use `DEFAULT_MGET_SIZE=256`
    pub batch_size: Option<u64>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct TruncateTableReply {}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct EmptyProto {}

mod kvapi_key_impl {
    use databend_common_meta_kvapi::kvapi;
    use databend_common_meta_kvapi::kvapi::Key;

    use crate::primitive::Id;
    use crate::schema::CountTablesKey;
    use crate::schema::DBIdTableName;
    use crate::schema::DatabaseId;
    use crate::schema::LeastVisibleTime;
    use crate::schema::LeastVisibleTimeKey;
    use crate::schema::TableCopiedFileInfo;
    use crate::schema::TableCopiedFileNameIdent;
    use crate::schema::TableId;
    use crate::schema::TableIdList;
    use crate::schema::TableIdListKey;
    use crate::schema::TableIdToName;
    use crate::schema::TableMeta;
    use crate::tenant::Tenant;

    /// "__fd_table/<db_id>/<tb_name>"
    impl kvapi::Key for DBIdTableName {
        const PREFIX: &'static str = "__fd_table";

        type ValueType = TableId;

        fn parent(&self) -> Option<String> {
            Some(DatabaseId::new(self.db_id).to_string_key())
        }

        fn to_string_key(&self) -> String {
            kvapi::KeyBuilder::new_prefixed(Self::PREFIX)
                .push_u64(self.db_id)
                .push_str(&self.table_name)
                .done()
        }

        fn from_str_key(s: &str) -> Result<Self, kvapi::KeyError> {
            let mut p = kvapi::KeyParser::new_prefixed(s, Self::PREFIX)?;

            let db_id = p.next_u64()?;
            let table_name = p.next_str()?;
            p.done()?;

            Ok(DBIdTableName { db_id, table_name })
        }
    }

    /// "__fd_table_id_to_name/<table_id> -> DBIdTableName"
    impl kvapi::Key for TableIdToName {
        const PREFIX: &'static str = "__fd_table_id_to_name";

        type ValueType = DBIdTableName;

        fn parent(&self) -> Option<String> {
            Some(TableId::new(self.table_id).to_string_key())
        }

        fn to_string_key(&self) -> String {
            kvapi::KeyBuilder::new_prefixed(Self::PREFIX)
                .push_u64(self.table_id)
                .done()
        }

        fn from_str_key(s: &str) -> Result<Self, kvapi::KeyError> {
            let mut p = kvapi::KeyParser::new_prefixed(s, Self::PREFIX)?;

            let table_id = p.next_u64()?;
            p.done()?;

            Ok(TableIdToName { table_id })
        }
    }

    /// "__fd_table_by_id/<tb_id> -> TableMeta"
    impl kvapi::Key for TableId {
        const PREFIX: &'static str = "__fd_table_by_id";

        type ValueType = TableMeta;

        fn parent(&self) -> Option<String> {
            None
        }

        fn to_string_key(&self) -> String {
            kvapi::KeyBuilder::new_prefixed(Self::PREFIX)
                .push_u64(self.table_id)
                .done()
        }

        fn from_str_key(s: &str) -> Result<Self, kvapi::KeyError> {
            let mut p = kvapi::KeyParser::new_prefixed(s, Self::PREFIX)?;

            let table_id = p.next_u64()?;
            p.done()?;

            Ok(TableId { table_id })
        }
    }

    /// "_fd_table_id_list/<db_id>/<tb_name> -> id_list"
    impl kvapi::Key for TableIdListKey {
        const PREFIX: &'static str = "__fd_table_id_list";

        type ValueType = TableIdList;

        fn parent(&self) -> Option<String> {
            Some(DatabaseId::new(self.db_id).to_string_key())
        }

        fn to_string_key(&self) -> String {
            kvapi::KeyBuilder::new_prefixed(Self::PREFIX)
                .push_u64(self.db_id)
                .push_str(&self.table_name)
                .done()
        }

        fn from_str_key(s: &str) -> Result<Self, kvapi::KeyError> {
            let mut p = kvapi::KeyParser::new_prefixed(s, Self::PREFIX)?;

            let db_id = p.next_u64()?;
            let table_name = p.next_str()?;
            p.done()?;

            Ok(TableIdListKey { db_id, table_name })
        }
    }

    /// "__fd_table_count/<tenant>" -> <table_count>
    impl kvapi::Key for CountTablesKey {
        const PREFIX: &'static str = "__fd_table_count";

        type ValueType = Id;

        fn parent(&self) -> Option<String> {
            Some(Tenant::new(&self.tenant).to_string_key())
        }

        fn to_string_key(&self) -> String {
            kvapi::KeyBuilder::new_prefixed(Self::PREFIX)
                .push_raw(&self.tenant)
                .done()
        }

        fn from_str_key(s: &str) -> Result<Self, kvapi::KeyError> {
            let mut p = kvapi::KeyParser::new_prefixed(s, Self::PREFIX)?;

            let tenant = p.next_str()?;
            p.done()?;

            Ok(CountTablesKey { tenant })
        }
    }

    // __fd_table_copied_files/table_id/file_name -> TableCopiedFileInfo
    impl kvapi::Key for TableCopiedFileNameIdent {
        const PREFIX: &'static str = "__fd_table_copied_files";

        type ValueType = TableCopiedFileInfo;

        fn parent(&self) -> Option<String> {
            Some(TableId::new(self.table_id).to_string_key())
        }

        fn to_string_key(&self) -> String {
            // TODO: file is not escaped!!!
            //       There already are non escaped data stored on disk.
            //       We can not change it anymore.
            kvapi::KeyBuilder::new_prefixed(Self::PREFIX)
                .push_u64(self.table_id)
                .push_raw(&self.file)
                .done()
        }

        fn from_str_key(s: &str) -> Result<Self, kvapi::KeyError> {
            let mut p = kvapi::KeyParser::new_prefixed(s, Self::PREFIX)?;

            let table_id = p.next_u64()?;
            let file = p.tail_raw()?.to_string();

            Ok(TableCopiedFileNameIdent { table_id, file })
        }
    }

    /// "__fd_table_lvt/table_id"
    impl kvapi::Key for LeastVisibleTimeKey {
        const PREFIX: &'static str = "__fd_table_lvt";

        type ValueType = LeastVisibleTime;

        fn parent(&self) -> Option<String> {
            Some(TableId::new(self.table_id).to_string_key())
        }

        fn to_string_key(&self) -> String {
            kvapi::KeyBuilder::new_prefixed(Self::PREFIX)
                .push_u64(self.table_id)
                .done()
        }

        fn from_str_key(s: &str) -> Result<Self, kvapi::KeyError> {
            let mut p = kvapi::KeyParser::new_prefixed(s, Self::PREFIX)?;

            let table_id = p.next_u64()?;
            p.done()?;

            Ok(LeastVisibleTimeKey { table_id })
        }
    }

    impl kvapi::Value for TableId {
        fn dependency_keys(&self) -> impl IntoIterator<Item = String> {
            [self.to_string_key()]
        }
    }

    impl kvapi::Value for DBIdTableName {
        fn dependency_keys(&self) -> impl IntoIterator<Item = String> {
            []
        }
    }

    impl kvapi::Value for TableMeta {
        fn dependency_keys(&self) -> impl IntoIterator<Item = String> {
            []
        }
    }

    impl kvapi::Value for TableIdList {
        fn dependency_keys(&self) -> impl IntoIterator<Item = String> {
            self.id_list
                .iter()
                .map(|id| TableId::new(*id).to_string_key())
        }
    }

    impl kvapi::Value for TableCopiedFileInfo {
        fn dependency_keys(&self) -> impl IntoIterator<Item = String> {
            []
        }
    }

    impl kvapi::Value for LeastVisibleTime {
        fn dependency_keys(&self) -> impl IntoIterator<Item = String> {
            []
        }
    }
}

#[cfg(test)]
mod tests {
    use databend_common_meta_kvapi::kvapi;
    use databend_common_meta_kvapi::kvapi::Key;

    use crate::schema::TableCopiedFileNameIdent;

    #[test]
    fn test_table_copied_file_name_ident_conversion() -> Result<(), kvapi::KeyError> {
        // test with a key has a file has multi path
        {
            let name = TableCopiedFileNameIdent {
                table_id: 2,
                file: "/path/to/file".to_owned(),
            };

            let key = name.to_string_key();
            assert_eq!(
                key,
                format!(
                    "{}/{}/{}",
                    TableCopiedFileNameIdent::PREFIX,
                    name.table_id,
                    name.file,
                )
            );
            let from = TableCopiedFileNameIdent::from_str_key(&key)?;
            assert_eq!(from, name);
        }

        // test with a key has only 2 sub-path
        {
            let key = format!("{}/{}", TableCopiedFileNameIdent::PREFIX, 2,);
            let res = TableCopiedFileNameIdent::from_str_key(&key);
            assert!(res.is_err());
            let err = res.unwrap_err();
            assert_eq!(err, kvapi::KeyError::WrongNumberOfSegments {
                expect: 3,
                got: key,
            });
        }

        // test with a key has 5 sub-path but an empty file string
        {
            let key = format!("{}/{}/{}", TableCopiedFileNameIdent::PREFIX, 2, "");
            let res = TableCopiedFileNameIdent::from_str_key(&key)?;
            assert_eq!(res, TableCopiedFileNameIdent {
                table_id: 2,
                file: "".to_string(),
            });
        }
        Ok(())
    }
}
