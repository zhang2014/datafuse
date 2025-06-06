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

use std::cmp;
use std::sync::Arc;

use crate::optimizer::rule::Rule;
use crate::optimizer::rule::TransformResult;
use crate::optimizer::RuleID;
use crate::optimizer::SExpr;
use crate::plans::Limit;
use crate::plans::PatternPlan;
use crate::plans::RelOp;
use crate::plans::RelOp::Pattern;
use crate::plans::RelOp::Sort;
use crate::plans::RelOperator;
use crate::plans::Sort as logsort;

/// Input:  Limit
///           \
///          Sort
///             \
///              *
///
/// Output: Limit
///           \
///          Sort(padding limit)
///             \
///               *
pub struct RulePushDownLimitSort {
    id: RuleID,
    patterns: Vec<SExpr>,
}

impl RulePushDownLimitSort {
    pub fn new() -> Self {
        Self {
            id: RuleID::PushDownLimitSort,
            patterns: vec![SExpr::create_unary(
                Arc::new(
                    PatternPlan {
                        plan_type: RelOp::Limit,
                    }
                    .into(),
                ),
                Arc::new(SExpr::create_unary(
                    Arc::new(PatternPlan { plan_type: Sort }.into()),
                    Arc::new(SExpr::create_leaf(Arc::new(
                        PatternPlan { plan_type: Pattern }.into(),
                    ))),
                )),
            )],
        }
    }
}

impl Rule for RulePushDownLimitSort {
    fn id(&self) -> RuleID {
        self.id
    }

    fn apply(
        &self,
        s_expr: &SExpr,
        state: &mut TransformResult,
    ) -> databend_common_exception::Result<()> {
        let limit: Limit = s_expr.plan().clone().try_into()?;
        if let Some(mut count) = limit.limit {
            count += limit.offset;
            let sort = s_expr.child(0)?;
            let mut sort_limit: logsort = sort.plan().clone().try_into()?;
            sort_limit.limit = Some(sort_limit.limit.map_or(count, |c| cmp::max(c, count)));
            let sort = SExpr::create_unary(
                Arc::new(RelOperator::Sort(sort_limit)),
                Arc::new(sort.child(0)?.clone()),
            );

            let mut result = s_expr.replace_children(vec![Arc::new(sort)]);
            result.set_applied_rule(&self.id);
            state.add_result(result);
        }
        Ok(())
    }

    fn patterns(&self) -> &Vec<SExpr> {
        &self.patterns
    }
}
