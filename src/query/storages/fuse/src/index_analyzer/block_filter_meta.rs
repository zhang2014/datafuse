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
use std::fmt::Debug;
use std::fmt::Formatter;
use std::sync::Arc;

use common_expression::BlockMetaInfo;
use common_expression::BlockMetaInfoPtr;
use serde::Deserializer;
use serde::Serializer;
use storages_common_pruner::BlockMetaIndex;
use storages_common_table_meta::meta::BlockMeta;
use storages_common_table_meta::meta::CompactSegmentInfo;

use crate::pruning::SegmentLocation;

pub struct BlockFilterMeta {
    pub meta_index: BlockMetaIndex,
    pub block_meta: Arc<BlockMeta>,
    pub segment_location: Arc<SegmentLocation>,
}

impl BlockFilterMeta {
    pub fn create(
        location: Arc<SegmentLocation>,
        block_meta: Arc<BlockMeta>,
        meta_index: BlockMetaIndex,
    ) -> BlockMetaInfoPtr {
        Box::new(BlockFilterMeta {
            meta_index,
            block_meta,
            segment_location: location,
        })
    }
}

impl Debug for BlockFilterMeta {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlockFilterMeta")
            .field("location", &self.segment_location)
            .finish()
    }
}

impl serde::Serialize for BlockFilterMeta {
    fn serialize<S>(&self, _: S) -> Result<S::Ok, S::Error>
    where S: Serializer {
        unimplemented!("Unimplemented serialize BlockFilterMeta")
    }
}

impl<'de> serde::Deserialize<'de> for BlockFilterMeta {
    fn deserialize<D>(_: D) -> Result<Self, D::Error>
    where D: Deserializer<'de> {
        unimplemented!("Unimplemented deserialize BlockFilterMeta")
    }
}

#[typetag::serde(name = "block_filter_meta")]
impl BlockMetaInfo for BlockFilterMeta {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn equals(&self, _: &Box<dyn BlockMetaInfo>) -> bool {
        unimplemented!("Unimplemented equals BlockFilterMeta")
    }

    fn clone_self(&self) -> Box<dyn BlockMetaInfo> {
        unimplemented!("Unimplemented clone BlockFilterMeta")
    }
}
