//  Copyright 2021 Datafuse Labs.
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

use std::sync::Arc;

use common_catalog::table_context::TableContext;
use common_exception::Result;
use common_meta_app::schema::TableStatistics;
use common_meta_app::schema::TruncateTableReq;
use common_meta_app::schema::UpdateTableMetaReq;
use common_meta_types::MatchSeq;
use common_storages_table_meta::meta::TableSnapshot;
use common_storages_table_meta::meta::Versioned;
use common_storages_table_meta::table::OPT_KEY_SNAPSHOT_LOCATION;
use uuid::Uuid;

use crate::FuseTable;

impl FuseTable {
    #[inline]
    pub async fn do_truncate(&self, ctx: Arc<dyn TableContext>, purge: bool) -> Result<()> {
        if let Some(prev_snapshot) = self.read_table_snapshot().await? {
            let prev_id = prev_snapshot.snapshot_id;

            let new_snapshot = TableSnapshot::new(
                Uuid::new_v4(),
                &prev_snapshot.timestamp,
                Some((prev_id, prev_snapshot.format_version())),
                prev_snapshot.schema.clone(),
                Default::default(),
                vec![],
                self.cluster_key_meta.clone(),
                // truncate MUST reset ts location
                None,
            );
            let loc = self.meta_location_generator();
            let new_snapshot_loc =
                loc.snapshot_location_from_uuid(&new_snapshot.snapshot_id, TableSnapshot::VERSION)?;
            let bytes = serde_json::to_vec(&new_snapshot)?;
            self.operator.object(&new_snapshot_loc).write(bytes).await?;

            if purge {
                let keep_last_snapshot = false;
                self.do_purge(&ctx, keep_last_snapshot).await?
            }

            let mut new_table_meta = self.table_info.meta.clone();
            // update snapshot location
            new_table_meta.options.insert(
                OPT_KEY_SNAPSHOT_LOCATION.to_owned(),
                new_snapshot_loc.clone(),
            );

            // update table statistics, all zeros
            new_table_meta.statistics = TableStatistics::default();

            let table_id = self.table_info.ident.table_id;
            let table_version = self.table_info.ident.seq;
            let catalog = ctx.get_catalog(self.table_info.catalog())?;

            catalog
                .update_table_meta(&self.table_info, UpdateTableMetaReq {
                    table_id,
                    seq: MatchSeq::Exact(table_version),
                    new_table_meta,
                })
                .await?;

            catalog
                .truncate_table(&self.table_info, TruncateTableReq { table_id })
                .await?;

            // try keep a hit file of last snapshot
            Self::write_last_snapshot_hint(
                &self.operator,
                &self.meta_location_generator,
                new_snapshot_loc,
            )
            .await;
        }

        Ok(())
    }
}
