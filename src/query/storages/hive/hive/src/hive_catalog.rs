// Copyright 2022 Datafuse Labs.
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
use std::sync::Arc;

use common_base::base::tokio;
use common_catalog::catalog::Catalog;
use common_catalog::catalog::StorageDescription;
use common_catalog::database::Database;
use common_catalog::table::Table;
use common_catalog::table_args::TableArgs;
use common_catalog::table_function::TableFunction;
use common_exception::ErrorCode;
use common_exception::Result;
use common_hive_meta_store::Partition;
use common_hive_meta_store::TThriftHiveMetastoreSyncClient;
use common_hive_meta_store::ThriftHiveMetastoreSyncClient;
use common_meta_app::schema::CountTablesReply;
use common_meta_app::schema::CountTablesReq;
use common_meta_app::schema::CreateDatabaseReply;
use common_meta_app::schema::CreateDatabaseReq;
use common_meta_app::schema::CreateTableReq;
use common_meta_app::schema::DropDatabaseReq;
use common_meta_app::schema::DropTableReply;
use common_meta_app::schema::DropTableReq;
use common_meta_app::schema::GetTableCopiedFileReply;
use common_meta_app::schema::GetTableCopiedFileReq;
use common_meta_app::schema::RenameDatabaseReply;
use common_meta_app::schema::RenameDatabaseReq;
use common_meta_app::schema::RenameTableReply;
use common_meta_app::schema::RenameTableReq;
use common_meta_app::schema::TableIdent;
use common_meta_app::schema::TableInfo;
use common_meta_app::schema::TableMeta;
use common_meta_app::schema::TruncateTableReply;
use common_meta_app::schema::TruncateTableReq;
use common_meta_app::schema::UndropDatabaseReply;
use common_meta_app::schema::UndropDatabaseReq;
use common_meta_app::schema::UndropTableReply;
use common_meta_app::schema::UndropTableReq;
use common_meta_app::schema::UpdateTableMetaReply;
use common_meta_app::schema::UpdateTableMetaReq;
use common_meta_app::schema::UpsertTableCopiedFileReply;
use common_meta_app::schema::UpsertTableCopiedFileReq;
use common_meta_app::schema::UpsertTableOptionReply;
use common_meta_app::schema::UpsertTableOptionReq;
use common_meta_types::*;
use thrift::protocol::*;
use thrift::transport::*;

use super::hive_database::HiveDatabase;
use crate::hive_table::HiveTable;

pub const HIVE_CATALOG: &str = "hive";

#[derive(Clone)]
pub struct HiveCatalog {
    /// address of hive meta store service
    client_address: String,
}

impl HiveCatalog {
    pub fn try_create(hms_address: impl Into<String>) -> Result<HiveCatalog> {
        Ok(HiveCatalog {
            client_address: hms_address.into(),
        })
    }

    pub fn get_client(&self) -> Result<impl TThriftHiveMetastoreSyncClient> {
        let mut c = TTcpChannel::new();
        c.open(self.client_address.as_str())
            .map_err(from_thrift_error)?;
        let (i_chan, o_chan) = c.split().map_err(from_thrift_error)?;
        let i_tran = TBufferedReadTransport::new(i_chan);
        let o_tran = TBufferedWriteTransport::new(o_chan);
        let i_prot = TBinaryInputProtocol::new(i_tran, true);
        let o_prot = TBinaryOutputProtocol::new(o_tran, true);
        Ok(ThriftHiveMetastoreSyncClient::new(i_prot, o_prot))
    }

    pub async fn get_partitions(
        &self,
        db: String,
        table: String,
        partition_names: Vec<String>,
    ) -> Result<Vec<Partition>> {
        let client = self.get_client()?;
        tokio::task::spawn_blocking(move || {
            Self::do_get_partitions(client, db, table, partition_names)
        })
        .await
        .unwrap()
    }

    pub fn do_get_partitions(
        client: impl TThriftHiveMetastoreSyncClient,
        db_name: String,
        tbl_name: String,
        partition_names: Vec<String>,
    ) -> Result<Vec<Partition>> {
        let mut client = client;
        let max_partitions = 10_000;

        let partitions = partition_names
            .chunks(max_partitions)
            .into_iter()
            .flat_map(|names| {
                client
                    .get_partitions_by_names(db_name.clone(), tbl_name.clone(), names.to_vec())
                    .map_err(from_thrift_error)
                    .unwrap()
            })
            .collect::<Vec<_>>();
        Ok(partitions)
    }

    #[tracing::instrument(level = "info", skip(self))]
    pub async fn get_partition_names(
        &self,
        db: String,
        table: String,
        max_parts: i16,
    ) -> Result<Vec<String>> {
        let client = self.get_client()?;
        tokio::task::spawn_blocking(move || {
            Self::do_get_partition_names(client, db, table, max_parts)
        })
        .await
        .unwrap()
    }

    pub fn do_get_partition_names(
        client: impl TThriftHiveMetastoreSyncClient,
        db: String,
        table: String,
        max_parts: i16,
    ) -> Result<Vec<String>> {
        let mut client = client;
        client
            .get_partition_names(db, table, max_parts)
            .map_err(from_thrift_error)
    }

    fn do_get_table(
        client: impl TThriftHiveMetastoreSyncClient,
        db_name: String,
        table_name: String,
    ) -> Result<Arc<dyn Table>> {
        let mut client = client;
        let table = client.get_table(db_name.clone(), table_name.clone());
        let table_meta = match table {
            Ok(table_meta) => table_meta,
            Err(e) => {
                if let thrift::Error::User(err) = &e {
                    if let Some(e) =
                        err.downcast_ref::<common_hive_meta_store::NoSuchObjectException>()
                    {
                        return Err(ErrorCode::TableInfoError(
                            e.message.clone().unwrap_or_default(),
                        ));
                    }
                }
                return Err(from_thrift_error(e));
            }
        };

        if let Some(sd) = table_meta.sd.as_ref() {
            if let Some(input_format) = sd.input_format.as_ref() {
                if input_format != "org.apache.hadoop.hive.ql.io.parquet.MapredParquetInputFormat" {
                    return Err(ErrorCode::Unimplemented(format!(
                        "only support parquet, {} not support",
                        input_format
                    )));
                }
            }
        }

        if let Some(t) = table_meta.table_type.as_ref() {
            if t == "VIRTUAL_VIEW" {
                return Err(ErrorCode::Unimplemented("not support view table"));
            }
        }

        let fields = client
            .get_schema(db_name, table_name)
            .map_err(from_thrift_error)?;
        let table_info: TableInfo = super::converters::try_into_table_info(table_meta, fields)?;
        let res: Arc<dyn Table> = Arc::new(HiveTable::try_create(table_info)?);
        Ok(res)
    }

    fn do_get_database(
        client: impl TThriftHiveMetastoreSyncClient,
        db_name: String,
    ) -> Result<Arc<dyn Database>> {
        let mut client = client;
        let thrift_db_meta = client.get_database(db_name).map_err(from_thrift_error)?;
        let hive_database: HiveDatabase = thrift_db_meta.into();
        let res: Arc<dyn Database> = Arc::new(hive_database);
        Ok(res)
    }
}

fn from_thrift_error(error: thrift::Error) -> ErrorCode {
    ErrorCode::from_std_error(error)
}

#[async_trait::async_trait]
impl Catalog for HiveCatalog {
    fn as_any(&self) -> &dyn Any {
        self
    }

    #[tracing::instrument(level = "info", skip(self))]
    async fn get_database(&self, _tenant: &str, db_name: &str) -> Result<Arc<dyn Database>> {
        let client = self.get_client()?;
        let _tenant = _tenant.to_string();
        let db_name = db_name.to_string();
        tokio::task::spawn_blocking(move || Self::do_get_database(client, db_name))
            .await
            .unwrap()
    }

    // Get all the databases.
    async fn list_databases(&self, _tenant: &str) -> Result<Vec<Arc<dyn Database>>> {
        todo!()
    }

    // Operation with database.
    async fn create_database(&self, _req: CreateDatabaseReq) -> Result<CreateDatabaseReply> {
        Err(ErrorCode::Unimplemented(
            "Cannot create database in HIVE catalog",
        ))
    }

    async fn drop_database(&self, _req: DropDatabaseReq) -> Result<()> {
        Err(ErrorCode::Unimplemented(
            "Cannot drop database in HIVE catalog",
        ))
    }

    async fn undrop_database(&self, _req: UndropDatabaseReq) -> Result<UndropDatabaseReply> {
        Err(ErrorCode::Unimplemented(
            "Cannot undrop database in HIVE catalog",
        ))
    }

    async fn rename_database(&self, _req: RenameDatabaseReq) -> Result<RenameDatabaseReply> {
        Err(ErrorCode::Unimplemented(
            "Cannot rename database in HIVE catalog",
        ))
    }

    fn get_table_by_info(&self, table_info: &TableInfo) -> Result<Arc<dyn Table>> {
        let res: Arc<dyn Table> = Arc::new(HiveTable::try_create(table_info.clone())?);
        Ok(res)
    }

    async fn get_table_meta_by_id(
        &self,
        _table_id: MetaId,
    ) -> Result<(TableIdent, Arc<TableMeta>)> {
        Err(ErrorCode::Unimplemented(
            "Cannot get table by id in HIVE catalog",
        ))
    }

    // Get one table by db and table name.
    #[tracing::instrument(level = "info", skip(self))]
    async fn get_table(
        &self,
        _tenant: &str,
        db_name: &str,
        table_name: &str,
    ) -> Result<Arc<dyn Table>> {
        let client = self.get_client()?;
        let db_name = db_name.to_string();
        let table_name = table_name.to_string();
        tokio::task::spawn_blocking(move || Self::do_get_table(client, db_name, table_name))
            .await
            .unwrap()
    }

    async fn list_tables(&self, _tenant: &str, _db_name: &str) -> Result<Vec<Arc<dyn Table>>> {
        todo!()
    }

    async fn list_tables_history(
        &self,
        _tenant: &str,
        _db_name: &str,
    ) -> Result<Vec<Arc<dyn Table>>> {
        Err(ErrorCode::Unimplemented(
            "Cannot list table history in HIVE catalog",
        ))
    }

    async fn create_table(&self, _req: CreateTableReq) -> Result<()> {
        Err(ErrorCode::Unimplemented(
            "Cannot create table in HIVE catalog",
        ))
    }

    async fn drop_table(&self, _req: DropTableReq) -> Result<DropTableReply> {
        Err(ErrorCode::Unimplemented(
            "Cannot drop table in HIVE catalog",
        ))
    }

    async fn undrop_table(&self, _req: UndropTableReq) -> Result<UndropTableReply> {
        Err(ErrorCode::Unimplemented(
            "Cannot undrop table in HIVE catalog",
        ))
    }

    async fn rename_table(&self, _req: RenameTableReq) -> Result<RenameTableReply> {
        Err(ErrorCode::Unimplemented(
            "Cannot rename table in HIVE catalog",
        ))
    }

    // Check a db.table is exists or not.
    async fn exists_table(&self, tenant: &str, db_name: &str, table_name: &str) -> Result<bool> {
        // TODO refine this
        match self.get_table(tenant, db_name, table_name).await {
            Ok(_) => Ok(true),
            Err(err) => {
                if err.code() == ErrorCode::UNKNOWN_TABLE {
                    Ok(false)
                } else {
                    Err(err)
                }
            }
        }
    }

    async fn upsert_table_option(
        &self,
        _tenant: &str,
        _db_name: &str,
        _req: UpsertTableOptionReq,
    ) -> Result<UpsertTableOptionReply> {
        Err(ErrorCode::Unimplemented(
            "Cannot upsert table option in HIVE catalog",
        ))
    }

    async fn update_table_meta(
        &self,
        _table_info: &TableInfo,
        _req: UpdateTableMetaReq,
    ) -> Result<UpdateTableMetaReply> {
        Err(ErrorCode::Unimplemented(
            "Cannot update table meta in HIVE catalog",
        ))
    }

    async fn get_table_copied_file_info(
        &self,
        _tenant: &str,
        _db_name: &str,
        _req: GetTableCopiedFileReq,
    ) -> Result<GetTableCopiedFileReply> {
        unimplemented!()
    }

    async fn upsert_table_copied_file_info(
        &self,
        _tenant: &str,
        _db_name: &str,
        _req: UpsertTableCopiedFileReq,
    ) -> Result<UpsertTableCopiedFileReply> {
        unimplemented!()
    }

    async fn truncate_table(
        &self,
        _table_info: &TableInfo,
        _req: TruncateTableReq,
    ) -> Result<TruncateTableReply> {
        unimplemented!()
    }

    async fn count_tables(&self, _req: CountTablesReq) -> Result<CountTablesReply> {
        unimplemented!()
    }

    /// Table function

    // Get function by name.
    fn get_table_function(
        &self,
        _func_name: &str,
        _tbl_args: TableArgs,
    ) -> Result<Arc<dyn TableFunction>> {
        unimplemented!()
    }

    // Get table engines
    fn get_table_engines(&self) -> Vec<StorageDescription> {
        unimplemented!()
    }
}
