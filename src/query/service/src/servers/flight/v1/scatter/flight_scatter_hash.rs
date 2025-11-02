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

use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;

use databend_common_column::binary::BinaryColumn;
use databend_common_column::binview::StringColumn;
use databend_common_column::bitmap::Bitmap;
use databend_common_column::types::months_days_micros;
// use databend_common_column::types::timestamp_tz;
use databend_common_exception::ErrorCode;
use databend_common_exception::Result;
use databend_common_expression::type_check::check_function;
use databend_common_expression::types::number::NumberScalar;
use databend_common_expression::types::AccessType;
use databend_common_expression::types::AnyType;
use databend_common_expression::types::Buffer;
use databend_common_expression::types::DataType;
use databend_common_expression::types::DecimalColumn;
use databend_common_expression::types::DecimalScalar;
use databend_common_expression::types::NullableColumn;
use databend_common_expression::types::NullableType;
use databend_common_expression::types::NumberColumn;
use databend_common_expression::types::NumberDataType;
use databend_common_expression::types::NumberType;
// use databend_common_expression::types::OpaqueColumn;
use databend_common_expression::types::VectorColumn;
use databend_common_expression::types::VectorScalar;
use databend_common_expression::AggHash;
use databend_common_expression::Column;
use databend_common_expression::DataBlock;
use databend_common_expression::Evaluator;
use databend_common_expression::Expr;
use databend_common_expression::FunctionContext;
use databend_common_expression::FunctionID;
use databend_common_expression::RemoteExpr;
use databend_common_expression::Scalar;
use databend_common_expression::Value;
use databend_common_functions::BUILTIN_FUNCTIONS;
use databend_common_hashtable::FastHash;
use strength_reduce::StrengthReducedU64;

use crate::servers::flight::v1::scatter::flight_scatter::FlightScatter;

#[derive(Clone)]
pub struct HashFlightScatter {
    func_ctx: FunctionContext,
    hash_key: Vec<Expr>,
    scatter_size: usize,
    default_scatter_index: u64,
}

impl HashFlightScatter {
    pub fn try_create(
        func_ctx: FunctionContext,
        hash_keys: Vec<RemoteExpr>,
        scatter_size: usize,
        local_pos: usize,
    ) -> Result<Box<dyn FlightScatter>> {
        if hash_keys.len() == 1 {
            return OneHashKeyFlightScatter::try_create(
                func_ctx,
                &hash_keys[0],
                scatter_size,
                local_pos,
            );
        }

        let default_scatter_index = match hash_keys.iter().any(shuffle_by_block_id_in_merge_into) {
            true => local_pos as u64,
            false => 0,
        };

        let hash_key = hash_keys
            .iter()
            .map(|key| key.as_expr(&BUILTIN_FUNCTIONS))
            .collect::<Vec<_>>();

        Ok(Box::new(Self {
            func_ctx,
            scatter_size,
            hash_key,
            default_scatter_index,
        }))
    }
}

#[derive(Clone)]
struct OneHashKeyFlightScatter {
    scatter_size: usize,
    func_ctx: FunctionContext,
    key: Expr,
    default_scatter_index: u64,
}

impl OneHashKeyFlightScatter {
    pub fn try_create(
        func_ctx: FunctionContext,
        hash_key: &RemoteExpr,
        scatter_size: usize,
        local_pos: usize,
    ) -> Result<Box<dyn FlightScatter>> {
        let default_scatter_index = if shuffle_by_block_id_in_merge_into(hash_key) {
            local_pos as u64
        } else {
            0
        };

        Ok(Box::new(OneHashKeyFlightScatter {
            scatter_size,
            func_ctx,
            default_scatter_index,
            key: hash_key.as_expr(&BUILTIN_FUNCTIONS),
        }))
    }
}

impl FlightScatter for OneHashKeyFlightScatter {
    fn name(&self) -> &'static str {
        "OneHashKey"
    }

    fn execute(&self, data_block: DataBlock) -> Result<Vec<DataBlock>> {
        let indices = self.partitions(&data_block)?;
        let data_blocks = DataBlock::scatter(&data_block, &indices, self.scatter_size)?;

        let block_meta = data_block.get_meta();
        let mut res = Vec::with_capacity(data_blocks.len());
        for data_block in data_blocks {
            res.push(data_block.add_meta(block_meta.cloned())?);
        }

        Ok(res)
    }

    fn partitions(&self, data_block: &DataBlock) -> Result<Buffer<u64>> {
        let evaluator = Evaluator::new(&data_block, &self.func_ctx, &BUILTIN_FUNCTIONS);
        let num = data_block.num_rows();

        let mut hashes = vec![0; num];
        hash_key::<false>(evaluator.run(&self.key)?, &mut hashes)?;

        let rem = StrengthReducedU64::new(self.scatter_size as u64);

        if self.default_scatter_index == 0 {
            for hash in &mut hashes {
                *hash = *hash % rem;
            }
        } else {
            for hash in &mut hashes {
                if *hash == 0 {
                    *hash = self.default_scatter_index;
                } else {
                    *hash = *hash % rem;
                }
            }
        }

        Ok(Buffer::from(hashes))
    }
}

impl FlightScatter for HashFlightScatter {
    fn name(&self) -> &'static str {
        "Hash"
    }

    fn execute(&self, data_block: DataBlock) -> Result<Vec<DataBlock>> {
        let indices = self.partitions(&data_block)?;
        let block_meta = data_block.get_meta();
        let data_blocks = DataBlock::scatter(&data_block, &indices, self.scatter_size)?;

        let mut res = Vec::with_capacity(data_blocks.len());
        for data_block in data_blocks {
            res.push(data_block.add_meta(block_meta.cloned())?);
        }

        Ok(res)
    }

    fn partitions(&self, data_block: &DataBlock) -> Result<Buffer<u64>> {
        let evaluator = Evaluator::new(&data_block, &self.func_ctx, &BUILTIN_FUNCTIONS);
        let num = data_block.num_rows();
        let indices = if !self.hash_key.is_empty() {
            let mut hashes = vec![0; num];

            let column = evaluator.run(&self.hash_key[0])?;
            hash_key::<false>(column, &mut hashes)?;

            for expr in self.hash_key.iter().skip(1) {
                let column = evaluator.run(expr)?;
                hash_key::<true>(column, &mut hashes)?;
            }

            let rem = StrengthReducedU64::new(self.scatter_size as u64);

            if self.default_scatter_index == 0 {
                for hash in &mut hashes {
                    *hash = *hash % rem;
                }
            } else {
                for hash in &mut hashes {
                    if *hash == 0 {
                        *hash = self.default_scatter_index;
                    } else {
                        *hash = *hash % rem;
                    }
                }
            }

            Ok::<Vec<u64>, ErrorCode>(hashes)
        } else {
            Ok(vec![0; num])
        }?;

        Ok(Buffer::from(indices))
    }
}

impl HashFlightScatter {
    pub fn combine_hash_keys(
        &self,
        hash_keys: &[Buffer<u64>],
        num_rows: usize,
    ) -> Result<Vec<u64>> {
        if self.hash_key.len() != hash_keys.len() {
            return Err(ErrorCode::Internal(
                "Hash keys and hash functions must be the same length.",
            ));
        }
        let mut hash = vec![DefaultHasher::default(); num_rows];
        for keys in hash_keys.iter() {
            for (i, value) in keys.iter().enumerate() {
                hash[i].write_u64(*value);
            }
        }

        let m = self.scatter_size as u64;
        Ok(hash.into_iter().map(|h| h.finish() % m).collect())
    }
}

fn shuffle_by_block_id_in_merge_into(expr: &RemoteExpr) -> bool {
    if let RemoteExpr::FunctionCall {
        id: box FunctionID::Builtin { name, .. },
        args,
        ..
    } = expr
    {
        if name == "bit_and" {
            if let RemoteExpr::FunctionCall {
                id: box FunctionID::Builtin { name, .. },
                ..
            } = &args[0]
            {
                if name == "bit_shift_right" {
                    return true;
                }
            }
        }
    }
    false
}

fn hash_key<const COMBO: bool>(column: Value<AnyType>, hashes: &mut [u64]) -> Result<()> {
    match column {
        Value::Scalar(value) => hash_scalar::<COMBO>(hashes, &value),
        Value::Column(column) => hash_column::<COMBO>(hashes, &column),
    }
}

fn hash_scalar<const COMBO: bool>(hashes: &mut [u64], value: &Scalar) -> Result<()> {
    match value {
        Scalar::Null => default_hash(hashes),
        Scalar::EmptyArray => default_hash(hashes),
        Scalar::EmptyMap => default_hash(hashes),
        Scalar::Number(number) => match number {
            NumberScalar::UInt8(v) => may_combo::<COMBO>(hashes, v.fast_hash()),
            NumberScalar::UInt16(v) => may_combo::<COMBO>(hashes, v.fast_hash()),
            NumberScalar::UInt32(v) => may_combo::<COMBO>(hashes, v.fast_hash()),
            NumberScalar::UInt64(v) => may_combo::<COMBO>(hashes, v.fast_hash()),
            NumberScalar::Int8(v) => may_combo::<COMBO>(hashes, v.fast_hash()),
            NumberScalar::Int16(v) => may_combo::<COMBO>(hashes, v.fast_hash()),
            NumberScalar::Int32(v) => may_combo::<COMBO>(hashes, v.fast_hash()),
            NumberScalar::Int64(v) => may_combo::<COMBO>(hashes, v.fast_hash()),
            NumberScalar::Float32(v) => may_combo::<COMBO>(hashes, v.fast_hash()),
            NumberScalar::Float64(v) => may_combo::<COMBO>(hashes, v.fast_hash()),
        },
        Scalar::Decimal(v) => match v {
            DecimalScalar::Decimal64(v, _) => may_combo::<COMBO>(hashes, v.fast_hash()),
            DecimalScalar::Decimal128(v, _) => may_combo::<COMBO>(hashes, v.fast_hash()),
            DecimalScalar::Decimal256(v, _) => may_combo::<COMBO>(hashes, v.0.fast_hash()),
        },
        Scalar::Timestamp(v) => may_combo::<COMBO>(hashes, v.fast_hash()),
        Scalar::Date(v) => may_combo::<COMBO>(hashes, v.fast_hash()),
        Scalar::Interval(v) => may_combo::<COMBO>(hashes, v.0.fast_hash()),
        Scalar::Boolean(v) => may_combo::<COMBO>(hashes, v.fast_hash()),
        Scalar::Binary(v) => may_combo::<COMBO>(hashes, v.fast_hash()),
        Scalar::String(v) => may_combo::<COMBO>(hashes, v.fast_hash()),
        Scalar::Array(_) => unreachable!(),
        Scalar::Map(_) => unreachable!(),
        Scalar::Bitmap(v) => may_combo::<COMBO>(hashes, v.fast_hash()),
        Scalar::Tuple(v) => {
            let mut first = true;
            for v in v {
                if first {
                    first = false;
                    hash_scalar::<COMBO>(hashes, v)?;
                } else {
                    hash_scalar::<true>(hashes, v)?;
                }
            }

            Ok(())
        }
        Scalar::Variant(v) => may_combo::<COMBO>(hashes, v.fast_hash()),
        Scalar::Geometry(v) => may_combo::<COMBO>(hashes, v.fast_hash()),
        Scalar::Geography(v) => may_combo::<COMBO>(hashes, v.0.fast_hash()),
        Scalar::Vector(_) => unreachable!(),
    }
}

fn may_combo<const COMBO: bool>(hashes: &mut [u64], hash: u64) -> Result<()> {
    if COMBO {
        for index in 0..hashes.len() {
            hashes[index] = combo_hash(hashes[index], hash);
        }
    } else {
        for index in 0..hashes.len() {
            hashes[index] = hash;
        }
    }

    Ok(())
}

fn hash_column<const COMBO: bool>(hashes: &mut [u64], column: &Column) -> Result<()> {
    match column {
        Column::Null { .. } => default_hash(hashes),
        Column::EmptyArray { .. } => default_hash(hashes),
        Column::EmptyMap { .. } => default_hash(hashes),
        Column::Number(number) => match number {
            NumberColumn::UInt8(v) => fast_hash::<COMBO, _>(&v, hashes),
            NumberColumn::UInt16(v) => fast_hash::<COMBO, _>(&v, hashes),
            NumberColumn::UInt32(v) => fast_hash::<COMBO, _>(&v, hashes),
            NumberColumn::UInt64(v) => fast_hash::<COMBO, _>(&v, hashes),
            NumberColumn::Int8(v) => fast_hash::<COMBO, _>(&v, hashes),
            NumberColumn::Int16(v) => fast_hash::<COMBO, _>(&v, hashes),
            NumberColumn::Int32(v) => fast_hash::<COMBO, _>(&v, hashes),
            NumberColumn::Int64(v) => fast_hash::<COMBO, _>(&v, hashes),
            NumberColumn::Float32(v) => fast_hash::<COMBO, _>(&v, hashes),
            NumberColumn::Float64(v) => fast_hash::<COMBO, _>(&v, hashes),
        },
        Column::Decimal(decimal) => match decimal {
            DecimalColumn::Decimal64(v, _) => fast_hash::<COMBO, _>(&v, hashes),
            DecimalColumn::Decimal128(v, _) => fast_hash::<COMBO, _>(&v, hashes),
            DecimalColumn::Decimal256(buffer, _) => {
                for index in 0..buffer.len() {
                    if COMBO {
                        if hashes[index] != 0 {
                            hashes[index] = combo_hash(hashes[index], buffer[index].0.fast_hash());
                        }
                    } else {
                        hashes[index] = buffer[index].0.fast_hash();
                    }
                }
                Ok(())
            }
        },
        Column::Boolean(v) => hash_bool::<COMBO>(hashes, v),
        Column::Binary(v) => binary_hash::<COMBO>(hashes, &v),
        Column::String(v) => string_hash::<COMBO>(hashes, &v),
        Column::Timestamp(v) => fast_hash::<COMBO, _>(&v, hashes),
        Column::Date(v) => fast_hash::<COMBO, _>(&v, hashes),
        Column::Interval(v) => interval_fast_hash::<COMBO>(hashes, v),
        Column::Array(v) => unreachable!(),
        Column::Map(v) => unreachable!(),
        Column::Bitmap(v) => binary_hash::<COMBO>(hashes, &v),
        Column::Nullable(v) => hash_nullable_column::<COMBO>(hashes, &v),
        Column::Tuple(columns) => hash_tuple_column::<COMBO>(hashes, columns),
        Column::Variant(v) => binary_hash::<COMBO>(hashes, &v),
        Column::Geometry(v) => binary_hash::<COMBO>(hashes, &v),
        Column::Geography(v) => binary_hash::<COMBO>(hashes, &v.0),
        Column::Vector(vector_column) => unreachable!(),
    }
}

fn hash_bool<const COMBO: bool>(hashes: &mut [u64], v: &Bitmap) -> Result<()> {
    for (idx, v) in v.iter().enumerate() {
        if COMBO {
            hashes[idx] = combo_hash(hashes[idx], v.fast_hash());
        } else {
            hashes[idx] = v.fast_hash();
        }
    }

    Ok(())
}

fn hash_nullable_column<const COMBO: bool>(
    hashes: &mut [u64],
    v: &NullableColumn<AnyType>,
) -> Result<()> {
    hash_column::<COMBO>(hashes, &v.column)?;
    if v.validity.null_count() != 0 {
        for (idx, valid) in v.validity.iter().enumerate() {
            if !valid {
                hashes[idx] = 0;
            }
        }
    }

    Ok(())
}

fn hash_tuple_column<const COMBO: bool>(hashes: &mut [u64], columns: &Vec<Column>) -> Result<()> {
    let mut first = true;
    for column in columns {
        if first {
            first = false;
            hash_column::<COMBO>(hashes, column)?;
        } else {
            hash_column::<true>(hashes, column)?;
        }
    }

    Ok(())
}

fn interval_fast_hash<const COMBO: bool>(
    hashes: &mut [u64],
    v: &Buffer<months_days_micros>,
) -> Result<()> {
    for index in 0..v.len() {
        if COMBO {
            if hashes[index] != 0 {
                hashes[index] = combo_hash(hashes[index], v[index].0.fast_hash());
            }
        } else {
            hashes[index] = v[index].0.fast_hash();
        }
    }
    Ok(())
}

// fn timestamp_tz_fast_hash<const COMBO: bool>(
//     hashes: &mut [u64],
//     v: &Buffer<timestamp_tz>,
// ) -> Result<()> {
//     for index in 0..v.len() {
//         if COMBO {
//             if hashes[index] != 0 {
//                 hashes[index] = combo_hash(hashes[index], v[index].0.fast_hash());
//             }
//         } else {
//             hashes[index] = v[index].0.fast_hash();
//         }
//     }
//
//     Ok(())
// }

fn default_hash(hashes: &mut [u64]) -> Result<()> {
    for index in 0..hashes.len() {
        hashes[index] = 0;
    }

    Ok(())
}

fn binary_hash<const COMBO: bool>(hashes: &mut [u64], binary: &BinaryColumn) -> Result<()> {
    for (idx, data) in binary.iter().enumerate() {
        if COMBO {
            if hashes[idx] != 0 {
                hashes[idx] = combo_hash(hashes[idx], data.fast_hash());
            }
        } else {
            hashes[idx] = data.fast_hash();
        }
    }

    Ok(())
}

fn string_hash<const COMBO: bool>(hashes: &mut [u64], string_column: &StringColumn) -> Result<()> {
    for (idx, data) in string_column.iter().enumerate() {
        if COMBO {
            if hashes[idx] != 0 {
                hashes[idx] = combo_hash(hashes[idx], data.fast_hash());
            }
        } else {
            hashes[idx] = data.fast_hash();
        }
    }

    Ok(())
}

fn fast_hash<const COMBO: bool, T: FastHash>(buffer: &Buffer<T>, hashes: &mut [u64]) -> Result<()> {
    for index in 0..buffer.len() {
        if COMBO {
            if hashes[index] != 0 {
                hashes[index] = combo_hash(hashes[index], buffer[index].fast_hash());
            }
        } else {
            hashes[index] = buffer[index].fast_hash();
        }
    }
    Ok(())
}

fn combo_hash(first: u64, second: u64) -> u64 {
    let mul = 0x9ddfea08eb382d69_u64;
    let mut a = (second ^ first).wrapping_mul(mul);
    a ^= (a >> 47);
    let mut b = (first ^ a).wrapping_mul(mul);
    b ^= (b >> 47);
    b.wrapping_mul(mul)
}

// fn get_hash_values(
//     column: Value<AnyType>,
//     rows: usize,
//     default_scatter_index: u64,
// ) -> Result<Buffer<u64>> {
//     match column {
//         Value::Scalar(c) => match c {
//             databend_common_expression::Scalar::Null => {
//                 Ok(vec![default_scatter_index; rows].into())
//             }
//             databend_common_expression::Scalar::Number(NumberScalar::UInt64(x)) => {
//                 Ok(vec![x; rows].into())
//             }
//             _ => unreachable!(),
//         },
//         Value::Column(c) => {
//             if let Some(column) = NumberType::<u64>::try_downcast_column(&c) {
//                 Ok(column)
//             } else if let Some(mut column) =
//                 NullableType::<NumberType<u64>>::try_downcast_column(&c)
//             {
//                 let null_map = column.validity;
//                 if null_map.null_count() == 0 {
//                     Ok(column.column)
//                 } else if null_map.null_count() == null_map.len() {
//                     Ok(vec![default_scatter_index; rows].into())
//                 } else {
//                     let mut need_new_vec = true;
//                     if let Some(column) = unsafe { column.column.get_mut() } {
//                         column
//                             .iter_mut()
//                             .zip(null_map.iter())
//                             .for_each(|(x, valid)| {
//                                 if valid {
//                                     *x *= valid as u64;
//                                 } else {
//                                     *x = default_scatter_index;
//                                 }
//                             });
//                         need_new_vec = false;
//                     }
//
//                     if !need_new_vec {
//                         Ok(column.column)
//                     } else {
//                         Ok(column
//                             .column
//                             .iter()
//                             .zip(null_map.iter())
//                             .map(|(x, b)| if b { *x } else { default_scatter_index })
//                             .collect())
//                     }
//                 }
//             } else {
//                 unreachable!()
//             }
//         }
//     }
// }
