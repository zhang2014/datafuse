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

use std::collections::BTreeSet;
use std::fmt::Display;
use std::fmt::Formatter;

use chrono::DateTime;
use chrono::Utc;

use crate::schema::CreateOption;

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default, Eq, PartialEq)]
pub struct DatamaskNameIdent {
    pub tenant: String,
    pub name: String,
}

impl Display for DatamaskNameIdent {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "'{}'/'{}'", self.tenant, self.name)
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default, Eq, PartialEq)]
pub struct DatamaskId {
    pub id: u64,
}

impl Display for DatamaskId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.id)
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct DatamaskMeta {
    // Vec<(arg_name, arg_type)>
    pub args: Vec<(String, String)>,
    pub return_type: String,
    pub body: String,
    pub comment: Option<String>,
    pub create_on: DateTime<Utc>,
    pub update_on: Option<DateTime<Utc>>,
}

impl From<CreateDatamaskReq> for DatamaskMeta {
    fn from(p: CreateDatamaskReq) -> Self {
        DatamaskMeta {
            args: p.args.clone(),
            return_type: p.return_type.clone(),
            body: p.body.clone(),
            comment: p.comment.clone(),
            create_on: p.create_on,
            update_on: None,
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct CreateDatamaskReq {
    pub create_option: CreateOption,
    pub name: DatamaskNameIdent,
    pub args: Vec<(String, String)>,
    pub return_type: String,
    pub body: String,
    pub comment: Option<String>,
    pub create_on: DateTime<Utc>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct CreateDatamaskReply {
    pub id: u64,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct DropDatamaskReq {
    pub if_exists: bool,
    pub name: DatamaskNameIdent,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct DropDatamaskReply {}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct GetDatamaskReq {
    pub name: DatamaskNameIdent,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct GetDatamaskReply {
    pub policy: DatamaskMeta,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default, Eq, PartialEq)]
pub struct MaskpolicyTableIdListKey {
    pub tenant: String,
    pub name: String,
}

impl MaskpolicyTableIdListKey {
    pub fn new(tenant: impl ToString, name: impl ToString) -> Self {
        MaskpolicyTableIdListKey {
            tenant: tenant.to_string(),
            name: name.to_string(),
        }
    }
}

impl Display for MaskpolicyTableIdListKey {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "'{}'/'{}'", self.tenant, self.name)
    }
}

/// A list of table ids
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Eq, Default, PartialEq)]
pub struct MaskpolicyTableIdList {
    pub id_list: BTreeSet<u64>,
}

mod kvapi_key_impl {
    use databend_common_meta_kvapi::kvapi;
    use databend_common_meta_kvapi::kvapi::Key;

    use super::DatamaskId;
    use super::DatamaskNameIdent;
    use super::MaskpolicyTableIdListKey;
    use crate::data_mask::DatamaskMeta;
    use crate::data_mask::MaskpolicyTableIdList;
    use crate::tenant::Tenant;

    /// __fd_database/<tenant>/<name> -> <data_mask_id>
    impl kvapi::Key for DatamaskNameIdent {
        const PREFIX: &'static str = "__fd_datamask";

        type ValueType = DatamaskId;

        /// It belongs to a tenant
        fn parent(&self) -> Option<String> {
            Some(Tenant::new(&self.tenant).to_string_key())
        }

        fn to_string_key(&self) -> String {
            kvapi::KeyBuilder::new_prefixed(Self::PREFIX)
                .push_str(&self.tenant)
                .push_str(&self.name)
                .done()
        }

        fn from_str_key(s: &str) -> Result<Self, kvapi::KeyError> {
            let mut p = kvapi::KeyParser::new_prefixed(s, Self::PREFIX)?;

            let tenant = p.next_str()?;
            let name = p.next_str()?;
            p.done()?;

            Ok(DatamaskNameIdent { tenant, name })
        }
    }

    /// "__fd_datamask_by_id/<id>"
    impl kvapi::Key for DatamaskId {
        const PREFIX: &'static str = "__fd_datamask_by_id";

        type ValueType = DatamaskMeta;

        fn parent(&self) -> Option<String> {
            None
        }

        fn to_string_key(&self) -> String {
            kvapi::KeyBuilder::new_prefixed(Self::PREFIX)
                .push_u64(self.id)
                .done()
        }

        fn from_str_key(s: &str) -> Result<Self, kvapi::KeyError> {
            let mut p = kvapi::KeyParser::new_prefixed(s, Self::PREFIX)?;

            let id = p.next_u64()?;
            p.done()?;

            Ok(DatamaskId { id })
        }
    }

    impl kvapi::Key for MaskpolicyTableIdListKey {
        const PREFIX: &'static str = "__fd_datamask_id_list";

        type ValueType = MaskpolicyTableIdList;

        /// It belongs to a tenant
        fn parent(&self) -> Option<String> {
            Some(Tenant::new(&self.tenant).to_string_key())
        }

        fn to_string_key(&self) -> String {
            kvapi::KeyBuilder::new_prefixed(Self::PREFIX)
                .push_str(&self.tenant)
                .push_str(&self.name)
                .done()
        }

        fn from_str_key(s: &str) -> Result<Self, kvapi::KeyError> {
            let mut p = kvapi::KeyParser::new_prefixed(s, Self::PREFIX)?;

            let tenant = p.next_str()?;
            let name = p.next_str()?;
            p.done()?;

            Ok(MaskpolicyTableIdListKey { tenant, name })
        }
    }

    impl kvapi::Value for DatamaskId {
        fn dependency_keys(&self) -> impl IntoIterator<Item = String> {
            [self.to_string_key()]
        }
    }

    impl kvapi::Value for DatamaskMeta {
        fn dependency_keys(&self) -> impl IntoIterator<Item = String> {
            []
        }
    }

    impl kvapi::Value for MaskpolicyTableIdList {
        /// It contains table ids but it does not own these table in the meta-data hierarchy.
        fn dependency_keys(&self) -> impl IntoIterator<Item = String> {
            []
        }
    }
}
