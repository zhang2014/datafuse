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

use std::cmp::max;
use std::cmp::Ordering;
use std::collections::HashSet;

use databend_common_exception::Result;
use databend_common_expression::types::DataType;
use databend_common_expression::types::NumberDataType;
use databend_common_expression::types::NumberScalar;
use databend_common_expression::Scalar;
use databend_common_storage::Datum;
use databend_common_storage::F64;

use crate::optimizer::histogram_from_ndv;
use crate::optimizer::ColumnStat;
use crate::optimizer::Statistics;
use crate::optimizer::DEFAULT_HISTOGRAM_BUCKETS;
use crate::plans::ComparisonOp;
use crate::plans::ConstantExpr;
use crate::plans::ScalarExpr;
use crate::IndexType;

/// A default selectivity factor for a predicate
/// that we cannot estimate the selectivity for it.
/// This factor comes from the paper
/// "Access Path Selection in a Relational Database Management System"
pub const DEFAULT_SELECTIVITY: f64 = 1f64 / 5f64;
pub const SMALL_SELECTIVITY: f64 = 1f64 / 2500f64;
pub const MAX_SELECTIVITY: f64 = 1f64;

pub struct SelectivityEstimator<'a> {
    pub input_stat: &'a mut Statistics,
    pub updated_column_indexes: HashSet<IndexType>,
}

impl<'a> SelectivityEstimator<'a> {
    pub fn new(input_stat: &'a mut Statistics, updated_column_indexes: HashSet<IndexType>) -> Self {
        Self {
            input_stat,
            updated_column_indexes,
        }
    }

    /// Compute the selectivity of a predicate.
    pub fn compute_selectivity(&mut self, predicate: &ScalarExpr, update: bool) -> Result<f64> {
        Ok(match predicate {
            ScalarExpr::BoundColumnRef(_) => {
                // If a column ref is on top of a predicate, e.g.
                // `SELECT * FROM t WHERE c1`, the selectivity is 1.
                1.0
            }

            ScalarExpr::ConstantExpr(constant) => {
                if is_true_constant_predicate(constant) {
                    1.0
                } else {
                    0.0
                }
            }

            ScalarExpr::FunctionCall(func) if func.func_name == "and" => {
                let left_selectivity = self.compute_selectivity(&func.arguments[0], update)?;
                let right_selectivity = self.compute_selectivity(&func.arguments[1], update)?;
                left_selectivity.min(right_selectivity)
            }

            ScalarExpr::FunctionCall(func) if func.func_name == "or" => {
                let left_selectivity = self.compute_selectivity(&func.arguments[0], false)?;
                let right_selectivity = self.compute_selectivity(&func.arguments[1], false)?;
                left_selectivity + right_selectivity - left_selectivity * right_selectivity
            }

            ScalarExpr::FunctionCall(func) if func.func_name == "not" => {
                let argument_selectivity = self.compute_selectivity(&func.arguments[0], false)?;
                1.0 - argument_selectivity
            }

            ScalarExpr::FunctionCall(func) => {
                if let Some(op) = ComparisonOp::try_from_func_name(&func.func_name) {
                    return self.compute_selectivity_comparison_expr(
                        op,
                        &func.arguments[0],
                        &func.arguments[1],
                        update,
                    );
                }

                DEFAULT_SELECTIVITY
            }

            _ => DEFAULT_SELECTIVITY,
        })
    }

    fn compute_selectivity_comparison_expr(
        &mut self,
        op: ComparisonOp,
        left: &ScalarExpr,
        right: &ScalarExpr,
        update: bool,
    ) -> Result<f64> {
        if let (ScalarExpr::BoundColumnRef(column_ref), ScalarExpr::ConstantExpr(constant)) =
            (left, &right)
        {
            // Check if there is available histogram for the column.
            let column_stat = if let Some(stat) = self
                .input_stat
                .column_stats
                .get_mut(&column_ref.column.index)
            {
                stat
            } else {
                // The column is derived column, give a small selectivity currently.
                // Need to improve it later.
                // Another case: column is from system table, such as numbers. We shouldn't use numbers() table to test cardinality estimation.
                return Ok(SMALL_SELECTIVITY);
            };
            let const_datum = if let Some(datum) = Datum::from_scalar(constant.value.clone()) {
                datum
            } else {
                return Ok(DEFAULT_SELECTIVITY);
            };

            return match op {
                ComparisonOp::Equal => {
                    // For equal predicate, we just use cardinality of a single
                    // value to estimate the selectivity. This assumes that
                    // the column is in a uniform distribution.
                    let selectivity = evaluate_equal(column_stat, constant);
                    if update {
                        update_statistic(
                            column_stat,
                            const_datum.clone(),
                            const_datum,
                            selectivity,
                        )?;
                        self.updated_column_indexes.insert(column_ref.column.index);
                    }
                    Ok(selectivity)
                }
                ComparisonOp::NotEqual => {
                    // For not equal predicate, we treat it as opposite of equal predicate.
                    let selectivity = 1.0 - evaluate_equal(column_stat, constant);
                    if update {
                        update_statistic(
                            column_stat,
                            column_stat.min.clone(),
                            column_stat.max.clone(),
                            selectivity,
                        )?;
                        self.updated_column_indexes.insert(column_ref.column.index);
                    }
                    Ok(selectivity)
                }
                ComparisonOp::GT => {
                    let col_hist = if let Some(hist) = column_stat.histogram.as_ref() {
                        hist
                    } else {
                        // Todo(xudong): use ndv to estimate the selectivity, not directly return `DEFAULT_SELECTIVITY`.
                        return Ok(DEFAULT_SELECTIVITY);
                    };
                    // For greater than predicate, we use the number of values
                    // that are greater than the constant value to estimate the
                    // selectivity.
                    let mut num_greater = 0.0;
                    let new_min = const_datum.clone();
                    let new_max = column_stat.max.clone();
                    for bucket in col_hist.buckets_iter() {
                        if let Ok(ord) = bucket.upper_bound().compare(&const_datum) {
                            if ord == Ordering::Less || ord == Ordering::Equal {
                                num_greater += bucket.num_values();
                            } else {
                                break;
                            }
                        } else {
                            return Ok(DEFAULT_SELECTIVITY);
                        }
                    }
                    let selectivity = 1.0 - num_greater / col_hist.num_values();
                    if update {
                        update_statistic(column_stat, new_min, new_max, selectivity)?;
                        self.updated_column_indexes.insert(column_ref.column.index);
                    }
                    Ok(selectivity)
                }
                ComparisonOp::LT => {
                    let col_hist = if let Some(hist) = column_stat.histogram.as_ref() {
                        hist
                    } else {
                        return Ok(DEFAULT_SELECTIVITY);
                    };
                    // For less than predicate, we treat it as opposite of
                    // greater than predicate.
                    let mut num_greater = 0.0;
                    let new_max = const_datum.clone();
                    let new_min = column_stat.min.clone();
                    for bucket in col_hist.buckets_iter() {
                        if let Ok(ord) = bucket.upper_bound().compare(&const_datum) {
                            if ord == Ordering::Less {
                                num_greater += bucket.num_values();
                            } else {
                                break;
                            }
                        } else {
                            return Ok(DEFAULT_SELECTIVITY);
                        }
                    }
                    let selectivity = num_greater / col_hist.num_values();
                    if update {
                        update_statistic(column_stat, new_min, new_max, selectivity)?;
                        self.updated_column_indexes.insert(column_ref.column.index);
                    }
                    Ok(selectivity)
                }
                ComparisonOp::GTE => {
                    let col_hist = if let Some(hist) = column_stat.histogram.as_ref() {
                        hist
                    } else {
                        return Ok(DEFAULT_SELECTIVITY);
                    };
                    // Greater than or equal to predicate is similar to greater than predicate.
                    let mut num_greater = 0.0;
                    let new_min = const_datum.clone();
                    let new_max = column_stat.max.clone();
                    for bucket in col_hist.buckets_iter() {
                        if let Ok(ord) = bucket.upper_bound().compare(&const_datum) {
                            if ord == Ordering::Less {
                                num_greater += bucket.num_values();
                            } else {
                                break;
                            }
                        } else {
                            return Ok(DEFAULT_SELECTIVITY);
                        }
                    }
                    let selectivity = 1.0 - num_greater / col_hist.num_values();
                    if update {
                        update_statistic(column_stat, new_min, new_max, selectivity)?;
                        self.updated_column_indexes.insert(column_ref.column.index);
                    }
                    Ok(selectivity)
                }
                ComparisonOp::LTE => {
                    let col_hist = if let Some(hist) = column_stat.histogram.as_ref() {
                        hist
                    } else {
                        return Ok(DEFAULT_SELECTIVITY);
                    };
                    // Less than or equal to predicate is similar to less than predicate.
                    let mut num_greater = 0.0;
                    let new_max = const_datum.clone();
                    let new_min = column_stat.min.clone();
                    for bucket in col_hist.buckets_iter() {
                        if let Ok(ord) = bucket.upper_bound().compare(&const_datum) {
                            if ord == Ordering::Less || ord == Ordering::Equal {
                                num_greater += bucket.num_values();
                            } else {
                                break;
                            }
                        } else {
                            return Ok(DEFAULT_SELECTIVITY);
                        }
                    }
                    let selectivity = num_greater / col_hist.num_values();
                    if update {
                        update_statistic(column_stat, new_min, new_max, selectivity)?;
                        self.updated_column_indexes.insert(column_ref.column.index);
                    }
                    Ok(selectivity)
                }
            };
        }

        Ok(DEFAULT_SELECTIVITY)
    }

    // Update other columns' statistic according to selectivity.
    pub fn update_other_statistic_by_selectivity(&mut self, selectivity: f64) {
        for (index, column_stat) in self.input_stat.column_stats.iter_mut() {
            if !self.updated_column_indexes.contains(index) {
                let new_ndv = (column_stat.ndv * selectivity).ceil();
                column_stat.ndv = new_ndv;
                if let Some(histogram) = &mut column_stat.histogram {
                    if new_ndv as u64 <= 2 {
                        column_stat.histogram = None;
                    } else {
                        for bucket in histogram.buckets.iter_mut() {
                            bucket.update(selectivity);
                        }
                    }
                }
            }
        }
    }
}

// TODO(andylokandy): match on non-null boolean only once we have constant folding in the optimizer.
fn is_true_constant_predicate(constant: &ConstantExpr) -> bool {
    match &constant.value {
        Scalar::Null => false,
        Scalar::Boolean(v) => *v,
        Scalar::Number(NumberScalar::Int64(v)) => *v != 0,
        Scalar::Number(NumberScalar::UInt64(v)) => *v != 0,
        Scalar::Number(NumberScalar::Float64(v)) => *v != 0.0,
        _ => true,
    }
}

fn evaluate_equal(column_stat: &ColumnStat, constant: &ConstantExpr) -> f64 {
    let constant_datum = Datum::from_scalar(constant.value.clone());
    match constant.value.as_ref().infer_data_type() {
        DataType::Null => 0.0,
        DataType::Number(number) => match number {
            NumberDataType::UInt8
            | NumberDataType::UInt16
            | NumberDataType::UInt32
            | NumberDataType::UInt64
            | NumberDataType::Int8
            | NumberDataType::Int16
            | NumberDataType::Int32
            | NumberDataType::Int64
            | NumberDataType::Float32
            | NumberDataType::Float64 => compare_equal(&constant_datum, column_stat),
        },
        DataType::Boolean | DataType::Binary | DataType::String => {
            compare_equal(&constant_datum, column_stat)
        }
        _ => {
            if column_stat.ndv == 0.0 {
                0.0
            } else {
                1.0 / column_stat.ndv
            }
        }
    }
}

fn compare_equal(datum: &Option<Datum>, column_stat: &ColumnStat) -> f64 {
    let col_min = &column_stat.min;
    let col_max = &column_stat.max;
    if let Some(constant_datum) = datum {
        if col_min.type_comparable(constant_datum) {
            // Safe to unwrap, because type is comparable.
            if constant_datum.compare(col_min).unwrap() == Ordering::Less
                || constant_datum.compare(col_max).unwrap() == Ordering::Greater
            {
                return 0.0;
            }
        }
    }

    if column_stat.ndv == 0.0 {
        0.0
    } else {
        1.0 / column_stat.ndv
    }
}

fn update_statistic(
    column_stat: &mut ColumnStat,
    mut new_min: Datum,
    mut new_max: Datum,
    selectivity: f64,
) -> Result<()> {
    let new_ndv = (column_stat.ndv * selectivity).ceil();
    column_stat.ndv = new_ndv;
    if matches!(
        new_min,
        Datum::Bool(_) | Datum::Int(_) | Datum::UInt(_) | Datum::Float(_)
    ) {
        new_min = Datum::Float(F64::from(new_min.to_double()?));
        new_max = Datum::Float(F64::from(new_max.to_double()?));
    }
    column_stat.min = new_min.clone();
    column_stat.max = new_max.clone();
    if let Some(histogram) = &column_stat.histogram {
        let num_values = histogram.num_values();
        let new_num_values = (num_values * selectivity).ceil() as u64;
        let new_ndv = new_ndv as u64;
        if new_ndv <= 2 {
            column_stat.histogram = None;
            return Ok(());
        }
        column_stat.histogram = Some(histogram_from_ndv(
            new_ndv,
            max(new_num_values, new_ndv),
            Some((new_min, new_max)),
            DEFAULT_HISTOGRAM_BUCKETS,
        )?);
    }
    Ok(())
}
