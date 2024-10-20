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

use std::sync::Arc;

use databend_common_exception::Result;

use crate::optimizer::rule::Rule;
use crate::optimizer::rule::RuleID;
use crate::optimizer::rule::TransformResult;
use crate::optimizer::RelExpr;
use crate::optimizer::SExpr;
use crate::plans::PatternPlan;
use crate::plans::RelOp;
use crate::plans::Sort;

pub struct RuleEliminateSort {
    id: RuleID,
    patterns: Vec<SExpr>,
}

impl RuleEliminateSort {
    pub fn new() -> Self {
        Self {
            id: RuleID::EliminateSort,
            // Sort
            //  \
            //   *
            patterns: vec![SExpr::create_unary(
                Arc::new(
                    PatternPlan {
                        plan_type: RelOp::Sort,
                    }
                    .into(),
                ),
                Arc::new(SExpr::create_leaf(Arc::new(
                    PatternPlan {
                        plan_type: RelOp::Pattern,
                    }
                    .into(),
                ))),
            )],
        }
    }
}

impl Rule for RuleEliminateSort {
    fn id(&self) -> RuleID {
        self.id
    }

    fn apply(&self, s_expr: &SExpr, state: &mut TransformResult) -> Result<()> {
        let sort: Sort = s_expr.plan().clone().try_into()?;
        let input = s_expr.child(0)?;
        let rel_expr = RelExpr::with_s_expr(input);
        let prop = rel_expr.derive_relational_prop()?;

        if prop.orderings == sort.items {
            // If the derived ordering is completely equal to
            // the sort operator, we can eliminate the sort.
            state.add_result(input.clone());
        }

        Ok(())
    }

    fn patterns(&self) -> &Vec<SExpr> {
        &self.patterns
    }
}
