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

use std::cell::RefCell;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::runtime::profile::ProfileStatisticsName;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ProfileLabel {
    pub name: String,
    pub value: Vec<String>,
}

impl ProfileLabel {
    pub fn create(name: String, value: Vec<String>) -> ProfileLabel {
        ProfileLabel { name, value }
    }
}

pub struct Profile {
    /// The id of processor
    pub pid: usize,
    /// The name of processor
    pub p_name: String,

    pub plan_id: Option<u32>,
    pub plan_name: Option<String>,
    pub plan_parent_id: Option<u32>,
    pub labels: Arc<Vec<ProfileLabel>>,
    pub title: Arc<String>,

    pub statistics: [AtomicUsize; std::mem::variant_count::<ProfileStatisticsName>()],
}

thread_local! {
    static CURRENT_PROFILE: RefCell<Option<Arc<Profile>>> = const { RefCell::new(None) };
}

impl Profile {
    fn create_items() -> [AtomicUsize; std::mem::variant_count::<ProfileStatisticsName>()] {
        std::array::from_fn(|_| AtomicUsize::new(0))
    }
    pub fn create(
        pid: usize,
        p_name: String,
        plan_id: Option<u32>,
        plan_name: Option<String>,
        plan_parent_id: Option<u32>,
        title: Arc<String>,
        labels: Arc<Vec<ProfileLabel>>,
    ) -> Profile {
        Profile {
            pid,
            p_name,
            plan_id,
            plan_name,
            plan_parent_id,
            title,
            labels,
            statistics: Self::create_items(),
        }
    }

    pub fn track_profile(profile: &Arc<Profile>) {
        CURRENT_PROFILE.set(Some(profile.clone()))
    }

    pub fn record_usize_profile(name: ProfileStatisticsName, value: usize) {
        CURRENT_PROFILE.with(|x| match x.borrow().as_ref() {
            None => {}
            Some(profile) => {
                profile.statistics[name as usize].fetch_add(value, Ordering::SeqCst);
            }
        });
    }

    pub fn load_profile(&self, name: ProfileStatisticsName) -> usize {
        self.statistics[name as usize].load(Ordering::SeqCst)
    }
}
