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

// This code is generated by src/query/codegen/src/writes/register.rs. DO NOT EDIT.

#![allow(unused_parens)]
#![allow(unused_variables)]
#![allow(clippy::redundant_closure)]
use crate::property::Domain;
use crate::register_vectorize::*;
use crate::types::nullable::NullableDomain;
use crate::types::*;
use crate::values::Value;
use crate::EvalContext;
use crate::Function;
use crate::FunctionContext;
use crate::FunctionDomain;
use crate::FunctionEval;
use crate::FunctionRegistry;
use crate::FunctionSignature;

impl FunctionRegistry {
    pub fn register_1_arg<I1: ArgType, O: ArgType, F, G>(
        &mut self,
        name: &str,
        calc_domain: F,
        func: G,
    ) where
        F: Fn(&FunctionContext, &I1::Domain) -> FunctionDomain<O>
            + 'static
            + Clone
            + Copy
            + Send
            + Sync,
        G: Fn(I1::ScalarRef<'_>, &mut EvalContext) -> O::Scalar
            + 'static
            + Clone
            + Copy
            + Send
            + Sync,
    {
        self.register_passthrough_nullable_1_arg::<I1, O, _, _>(
            name,
            calc_domain,
            vectorize_1_arg(func),
        )
    }

    pub fn register_2_arg<I1: ArgType, I2: ArgType, O: ArgType, F, G>(
        &mut self,
        name: &str,
        calc_domain: F,
        func: G,
    ) where
        F: Fn(&FunctionContext, &I1::Domain, &I2::Domain) -> FunctionDomain<O>
            + 'static
            + Clone
            + Copy
            + Send
            + Sync,
        G: Fn(I1::ScalarRef<'_>, I2::ScalarRef<'_>, &mut EvalContext) -> O::Scalar
            + 'static
            + Clone
            + Copy
            + Send
            + Sync,
    {
        self.register_passthrough_nullable_2_arg::<I1, I2, O, _, _>(
            name,
            calc_domain,
            vectorize_2_arg(func),
        )
    }

    pub fn register_3_arg<I1: ArgType, I2: ArgType, I3: ArgType, O: ArgType, F, G>(
        &mut self,
        name: &str,
        calc_domain: F,
        func: G,
    ) where
        F: Fn(&FunctionContext, &I1::Domain, &I2::Domain, &I3::Domain) -> FunctionDomain<O>
            + 'static
            + Clone
            + Copy
            + Send
            + Sync,
        G: Fn(
                I1::ScalarRef<'_>,
                I2::ScalarRef<'_>,
                I3::ScalarRef<'_>,
                &mut EvalContext,
            ) -> O::Scalar
            + 'static
            + Clone
            + Copy
            + Send
            + Sync,
    {
        self.register_passthrough_nullable_3_arg::<I1, I2, I3, O, _, _>(
            name,
            calc_domain,
            vectorize_3_arg(func),
        )
    }

    pub fn register_4_arg<I1: ArgType, I2: ArgType, I3: ArgType, I4: ArgType, O: ArgType, F, G>(
        &mut self,
        name: &str,
        calc_domain: F,
        func: G,
    ) where
        F: Fn(
                &FunctionContext,
                &I1::Domain,
                &I2::Domain,
                &I3::Domain,
                &I4::Domain,
            ) -> FunctionDomain<O>
            + 'static
            + Clone
            + Copy
            + Send
            + Sync,
        G: Fn(
                I1::ScalarRef<'_>,
                I2::ScalarRef<'_>,
                I3::ScalarRef<'_>,
                I4::ScalarRef<'_>,
                &mut EvalContext,
            ) -> O::Scalar
            + 'static
            + Clone
            + Copy
            + Send
            + Sync,
    {
        self.register_passthrough_nullable_4_arg::<I1, I2, I3, I4, O, _, _>(
            name,
            calc_domain,
            vectorize_4_arg(func),
        )
    }

    pub fn register_passthrough_nullable_1_arg<I1: ArgType, O: ArgType, F, G>(
        &mut self,
        name: &str,
        calc_domain: F,
        func: G,
    ) where
        F: Fn(&FunctionContext, &I1::Domain) -> FunctionDomain<O>
            + 'static
            + Clone
            + Copy
            + Send
            + Sync,
        G: for<'a> Fn(Value<I1>, &mut EvalContext) -> Value<O>
            + 'static
            + Clone
            + Copy
            + Send
            + Sync,
    {
        let has_nullable = &[I1::data_type(), O::data_type()]
            .iter()
            .any(|ty| ty.as_nullable().is_some() || ty.is_null());

        assert!(
            !has_nullable,
            "Function {} has nullable argument or output, please use register_1_arg_core instead",
            name
        );

        self.register_1_arg_core::<I1, O, _, _>(name, calc_domain, func);

        self.register_1_arg_core::<NullableType<I1>, NullableType<O>, _, _>(
            name,
            move |ctx, arg1| match (&arg1.value) {
                (Some(value1)) => {
                    if let Some(domain) = calc_domain(ctx, value1).normalize() {
                        FunctionDomain::Domain(NullableDomain {
                            has_null: arg1.has_null,
                            value: Some(Box::new(domain)),
                        })
                    } else {
                        FunctionDomain::MayThrow
                    }
                }
                _ => FunctionDomain::Domain(NullableDomain {
                    has_null: true,
                    value: None,
                }),
            },
            passthrough_nullable_1_arg(func),
        );
    }

    pub fn register_passthrough_nullable_2_arg<I1: ArgType, I2: ArgType, O: ArgType, F, G>(
        &mut self,
        name: &str,
        calc_domain: F,
        func: G,
    ) where
        F: Fn(&FunctionContext, &I1::Domain, &I2::Domain) -> FunctionDomain<O>
            + 'static
            + Clone
            + Copy
            + Send
            + Sync,
        G: for<'a> Fn(Value<I1>, Value<I2>, &mut EvalContext) -> Value<O>
            + 'static
            + Clone
            + Copy
            + Send
            + Sync,
    {
        let has_nullable = &[I1::data_type(), I2::data_type(), O::data_type()]
            .iter()
            .any(|ty| ty.as_nullable().is_some() || ty.is_null());

        assert!(
            !has_nullable,
            "Function {} has nullable argument or output, please use register_2_arg_core instead",
            name
        );

        self.register_2_arg_core::<I1, I2, O, _, _>(name, calc_domain, func);

        self.register_2_arg_core::<NullableType<I1>, NullableType<I2>, NullableType<O>, _, _>(
            name,
            move |ctx, arg1, arg2| match (&arg1.value, &arg2.value) {
                (Some(value1), Some(value2)) => {
                    if let Some(domain) = calc_domain(ctx, value1, value2).normalize() {
                        FunctionDomain::Domain(NullableDomain {
                            has_null: arg1.has_null || arg2.has_null,
                            value: Some(Box::new(domain)),
                        })
                    } else {
                        FunctionDomain::MayThrow
                    }
                }
                _ => FunctionDomain::Domain(NullableDomain {
                    has_null: true,
                    value: None,
                }),
            },
            passthrough_nullable_2_arg(func),
        );
    }

    pub fn register_passthrough_nullable_3_arg<
        I1: ArgType,
        I2: ArgType,
        I3: ArgType,
        O: ArgType,
        F,
        G,
    >(
        &mut self,
        name: &str,
        calc_domain: F,
        func: G,
    ) where
        F: Fn(&FunctionContext, &I1::Domain, &I2::Domain, &I3::Domain) -> FunctionDomain<O>
            + 'static
            + Clone
            + Copy
            + Send
            + Sync,
        G: for<'a> Fn(Value<I1>, Value<I2>, Value<I3>, &mut EvalContext) -> Value<O>
            + 'static
            + Clone
            + Copy
            + Send
            + Sync,
    {
        let has_nullable = &[
            I1::data_type(),
            I2::data_type(),
            I3::data_type(),
            O::data_type(),
        ]
        .iter()
        .any(|ty| ty.as_nullable().is_some() || ty.is_null());

        assert!(
            !has_nullable,
            "Function {} has nullable argument or output, please use register_3_arg_core instead",
            name
        );

        self.register_3_arg_core::<I1, I2, I3, O, _, _>(name, calc_domain, func);

        self.register_3_arg_core::<NullableType<I1>, NullableType<I2>, NullableType<I3>,  NullableType<O>, _, _>(
                        name,
                        move |ctx, arg1,arg2,arg3,| {
                            match (&arg1.value,&arg2.value,&arg3.value) {
                                (Some(value1),Some(value2),Some(value3)) => {
                                    if let Some(domain) = calc_domain(ctx, value1,value2,value3,).normalize() {
                                        FunctionDomain::Domain(NullableDomain {
                                            has_null: arg1.has_null||arg2.has_null||arg3.has_null,
                                            value: Some(Box::new(domain)),
                                        })
                                    } else {
                                        FunctionDomain::MayThrow
                                    }
                                },
                                _ => {
                                    FunctionDomain::Domain(NullableDomain {
                                        has_null: true,
                                        value: None,
                                    })
                                },
                            }
                        },
                        passthrough_nullable_3_arg(func),
                    );
    }

    pub fn register_passthrough_nullable_4_arg<
        I1: ArgType,
        I2: ArgType,
        I3: ArgType,
        I4: ArgType,
        O: ArgType,
        F,
        G,
    >(
        &mut self,
        name: &str,
        calc_domain: F,
        func: G,
    ) where
        F: Fn(
                &FunctionContext,
                &I1::Domain,
                &I2::Domain,
                &I3::Domain,
                &I4::Domain,
            ) -> FunctionDomain<O>
            + 'static
            + Clone
            + Copy
            + Send
            + Sync,
        G: for<'a> Fn(Value<I1>, Value<I2>, Value<I3>, Value<I4>, &mut EvalContext) -> Value<O>
            + 'static
            + Clone
            + Copy
            + Send
            + Sync,
    {
        let has_nullable = &[
            I1::data_type(),
            I2::data_type(),
            I3::data_type(),
            I4::data_type(),
            O::data_type(),
        ]
        .iter()
        .any(|ty| ty.as_nullable().is_some() || ty.is_null());

        assert!(
            !has_nullable,
            "Function {} has nullable argument or output, please use register_4_arg_core instead",
            name
        );

        self.register_4_arg_core::<I1, I2, I3, I4, O, _, _>(name, calc_domain, func);

        self.register_4_arg_core::<NullableType<I1>, NullableType<I2>, NullableType<I3>, NullableType<I4>,  NullableType<O>, _, _>(
                        name,
                        move |ctx, arg1,arg2,arg3,arg4,| {
                            match (&arg1.value,&arg2.value,&arg3.value,&arg4.value) {
                                (Some(value1),Some(value2),Some(value3),Some(value4)) => {
                                    if let Some(domain) = calc_domain(ctx, value1,value2,value3,value4,).normalize() {
                                        FunctionDomain::Domain(NullableDomain {
                                            has_null: arg1.has_null||arg2.has_null||arg3.has_null||arg4.has_null,
                                            value: Some(Box::new(domain)),
                                        })
                                    } else {
                                        FunctionDomain::MayThrow
                                    }
                                },
                                _ => {
                                    FunctionDomain::Domain(NullableDomain {
                                        has_null: true,
                                        value: None,
                                    })
                                },
                            }
                        },
                        passthrough_nullable_4_arg(func),
                    );
    }

    pub fn register_combine_nullable_1_arg<I1: ArgType, O: ArgType, F, G>(
        &mut self,
        name: &str,
        calc_domain: F,
        func: G,
    ) where
        F: Fn(&FunctionContext, &I1::Domain) -> FunctionDomain<NullableType<O>>
            + 'static
            + Clone
            + Copy
            + Send
            + Sync,
        G: for<'a> Fn(Value<I1>, &mut EvalContext) -> Value<NullableType<O>>
            + 'static
            + Clone
            + Copy
            + Send
            + Sync,
    {
        let has_nullable = &[I1::data_type(), O::data_type()]
            .iter()
            .any(|ty| ty.as_nullable().is_some() || ty.is_null());

        assert!(
            !has_nullable,
            "Function {} has nullable argument or output, please use register_1_arg_core instead",
            name
        );

        self.register_1_arg_core::<I1, NullableType<O>, _, _>(name, calc_domain, func);

        self.register_1_arg_core::<NullableType<I1>, NullableType<O>, _, _>(
            name,
            move |ctx, arg1| match (&arg1.value) {
                (Some(value1)) => {
                    if let Some(domain) = calc_domain(ctx, value1).normalize() {
                        FunctionDomain::Domain(NullableDomain {
                            has_null: arg1.has_null || domain.has_null,
                            value: domain.value,
                        })
                    } else {
                        FunctionDomain::MayThrow
                    }
                }
                _ => FunctionDomain::Domain(NullableDomain {
                    has_null: true,
                    value: None,
                }),
            },
            combine_nullable_1_arg(func),
        );
    }

    pub fn register_combine_nullable_2_arg<I1: ArgType, I2: ArgType, O: ArgType, F, G>(
        &mut self,
        name: &str,
        calc_domain: F,
        func: G,
    ) where
        F: Fn(&FunctionContext, &I1::Domain, &I2::Domain) -> FunctionDomain<NullableType<O>>
            + 'static
            + Clone
            + Copy
            + Send
            + Sync,
        G: for<'a> Fn(Value<I1>, Value<I2>, &mut EvalContext) -> Value<NullableType<O>>
            + 'static
            + Clone
            + Copy
            + Send
            + Sync,
    {
        let has_nullable = &[I1::data_type(), I2::data_type(), O::data_type()]
            .iter()
            .any(|ty| ty.as_nullable().is_some() || ty.is_null());

        assert!(
            !has_nullable,
            "Function {} has nullable argument or output, please use register_2_arg_core instead",
            name
        );

        self.register_2_arg_core::<I1, I2, NullableType<O>, _, _>(name, calc_domain, func);

        self.register_2_arg_core::<NullableType<I1>, NullableType<I2>, NullableType<O>, _, _>(
            name,
            move |ctx, arg1, arg2| match (&arg1.value, &arg2.value) {
                (Some(value1), Some(value2)) => {
                    if let Some(domain) = calc_domain(ctx, value1, value2).normalize() {
                        FunctionDomain::Domain(NullableDomain {
                            has_null: arg1.has_null || arg2.has_null || domain.has_null,
                            value: domain.value,
                        })
                    } else {
                        FunctionDomain::MayThrow
                    }
                }
                _ => FunctionDomain::Domain(NullableDomain {
                    has_null: true,
                    value: None,
                }),
            },
            combine_nullable_2_arg(func),
        );
    }

    pub fn register_combine_nullable_3_arg<
        I1: ArgType,
        I2: ArgType,
        I3: ArgType,
        O: ArgType,
        F,
        G,
    >(
        &mut self,
        name: &str,
        calc_domain: F,
        func: G,
    ) where
        F: Fn(
                &FunctionContext,
                &I1::Domain,
                &I2::Domain,
                &I3::Domain,
            ) -> FunctionDomain<NullableType<O>>
            + 'static
            + Clone
            + Copy
            + Send
            + Sync,
        G: for<'a> Fn(Value<I1>, Value<I2>, Value<I3>, &mut EvalContext) -> Value<NullableType<O>>
            + 'static
            + Clone
            + Copy
            + Send
            + Sync,
    {
        let has_nullable = &[
            I1::data_type(),
            I2::data_type(),
            I3::data_type(),
            O::data_type(),
        ]
        .iter()
        .any(|ty| ty.as_nullable().is_some() || ty.is_null());

        assert!(
            !has_nullable,
            "Function {} has nullable argument or output, please use register_3_arg_core instead",
            name
        );

        self.register_3_arg_core::<I1, I2, I3, NullableType<O>, _, _>(name, calc_domain, func);

        self.register_3_arg_core::<NullableType<I1>, NullableType<I2>, NullableType<I3>,  NullableType<O>, _, _>(
                        name,
                        move |ctx, arg1,arg2,arg3,| {
                            match (&arg1.value,&arg2.value,&arg3.value) {
                                (Some(value1),Some(value2),Some(value3)) => {
                                    if let Some(domain) = calc_domain(ctx, value1,value2,value3,).normalize() {
                                        FunctionDomain::Domain(NullableDomain {
                                            has_null: arg1.has_null||arg2.has_null||arg3.has_null || domain.has_null,
                                            value: domain.value,
                                        })
                                    } else {
                                        FunctionDomain::MayThrow
                                    }
                                }
                                _ => {
                                    FunctionDomain::Domain(NullableDomain {
                                        has_null: true,
                                        value: None,
                                    })
                                },
                            }
                        },
                        combine_nullable_3_arg(func),
                    );
    }

    pub fn register_combine_nullable_4_arg<
        I1: ArgType,
        I2: ArgType,
        I3: ArgType,
        I4: ArgType,
        O: ArgType,
        F,
        G,
    >(
        &mut self,
        name: &str,
        calc_domain: F,
        func: G,
    ) where
        F: Fn(
                &FunctionContext,
                &I1::Domain,
                &I2::Domain,
                &I3::Domain,
                &I4::Domain,
            ) -> FunctionDomain<NullableType<O>>
            + 'static
            + Clone
            + Copy
            + Send
            + Sync,
        G: for<'a> Fn(
                Value<I1>,
                Value<I2>,
                Value<I3>,
                Value<I4>,
                &mut EvalContext,
            ) -> Value<NullableType<O>>
            + 'static
            + Clone
            + Copy
            + Send
            + Sync,
    {
        let has_nullable = &[
            I1::data_type(),
            I2::data_type(),
            I3::data_type(),
            I4::data_type(),
            O::data_type(),
        ]
        .iter()
        .any(|ty| ty.as_nullable().is_some() || ty.is_null());

        assert!(
            !has_nullable,
            "Function {} has nullable argument or output, please use register_4_arg_core instead",
            name
        );

        self.register_4_arg_core::<I1, I2, I3, I4, NullableType<O>, _, _>(name, calc_domain, func);

        self.register_4_arg_core::<NullableType<I1>, NullableType<I2>, NullableType<I3>, NullableType<I4>,  NullableType<O>, _, _>(
                        name,
                        move |ctx, arg1,arg2,arg3,arg4,| {
                            match (&arg1.value,&arg2.value,&arg3.value,&arg4.value) {
                                (Some(value1),Some(value2),Some(value3),Some(value4)) => {
                                    if let Some(domain) = calc_domain(ctx, value1,value2,value3,value4,).normalize() {
                                        FunctionDomain::Domain(NullableDomain {
                                            has_null: arg1.has_null||arg2.has_null||arg3.has_null||arg4.has_null || domain.has_null,
                                            value: domain.value,
                                        })
                                    } else {
                                        FunctionDomain::MayThrow
                                    }
                                }
                                _ => {
                                    FunctionDomain::Domain(NullableDomain {
                                        has_null: true,
                                        value: None,
                                    })
                                },
                            }
                        },
                        combine_nullable_4_arg(func),
                    );
    }

    pub fn register_0_arg_core<O: ArgType, F, G>(&mut self, name: &str, calc_domain: F, func: G)
    where
        F: Fn(&FunctionContext) -> FunctionDomain<O> + 'static + Clone + Copy + Send + Sync,
        G: for<'a> Fn(&mut EvalContext) -> Value<O> + 'static + Clone + Copy + Send + Sync,
    {
        let func = Function {
            signature: FunctionSignature {
                name: name.to_string(),
                args_type: vec![],
                return_type: O::data_type(),
            },
            eval: FunctionEval::Scalar {
                calc_domain: Box::new(erase_calc_domain_generic_0_arg::<O>(calc_domain)),
                eval: Box::new(erase_function_generic_0_arg(func)),
            },
        };
        self.register_function(func);
    }

    pub fn register_1_arg_core<I1: ArgType, O: ArgType, F, G>(
        &mut self,
        name: &str,
        calc_domain: F,
        func: G,
    ) where
        F: Fn(&FunctionContext, &I1::Domain) -> FunctionDomain<O>
            + 'static
            + Clone
            + Copy
            + Send
            + Sync,
        G: for<'a> Fn(Value<I1>, &mut EvalContext) -> Value<O>
            + 'static
            + Clone
            + Copy
            + Send
            + Sync,
    {
        let func = Function {
            signature: FunctionSignature {
                name: name.to_string(),
                args_type: vec![I1::data_type()],
                return_type: O::data_type(),
            },
            eval: FunctionEval::Scalar {
                calc_domain: Box::new(erase_calc_domain_generic_1_arg::<I1, O>(calc_domain)),
                eval: Box::new(erase_function_generic_1_arg(func)),
            },
        };
        self.register_function(func);
    }

    pub fn register_2_arg_core<I1: ArgType, I2: ArgType, O: ArgType, F, G>(
        &mut self,
        name: &str,
        calc_domain: F,
        func: G,
    ) where
        F: Fn(&FunctionContext, &I1::Domain, &I2::Domain) -> FunctionDomain<O>
            + 'static
            + Clone
            + Copy
            + Send
            + Sync,
        G: for<'a> Fn(Value<I1>, Value<I2>, &mut EvalContext) -> Value<O>
            + 'static
            + Clone
            + Copy
            + Send
            + Sync,
    {
        let func = Function {
            signature: FunctionSignature {
                name: name.to_string(),
                args_type: vec![I1::data_type(), I2::data_type()],
                return_type: O::data_type(),
            },
            eval: FunctionEval::Scalar {
                calc_domain: Box::new(erase_calc_domain_generic_2_arg::<I1, I2, O>(calc_domain)),
                eval: Box::new(erase_function_generic_2_arg(func)),
            },
        };
        self.register_function(func);
    }

    pub fn register_3_arg_core<I1: ArgType, I2: ArgType, I3: ArgType, O: ArgType, F, G>(
        &mut self,
        name: &str,
        calc_domain: F,
        func: G,
    ) where
        F: Fn(&FunctionContext, &I1::Domain, &I2::Domain, &I3::Domain) -> FunctionDomain<O>
            + 'static
            + Clone
            + Copy
            + Send
            + Sync,
        G: for<'a> Fn(Value<I1>, Value<I2>, Value<I3>, &mut EvalContext) -> Value<O>
            + 'static
            + Clone
            + Copy
            + Send
            + Sync,
    {
        let func = Function {
            signature: FunctionSignature {
                name: name.to_string(),
                args_type: vec![I1::data_type(), I2::data_type(), I3::data_type()],
                return_type: O::data_type(),
            },
            eval: FunctionEval::Scalar {
                calc_domain: Box::new(erase_calc_domain_generic_3_arg::<I1, I2, I3, O>(
                    calc_domain,
                )),
                eval: Box::new(erase_function_generic_3_arg(func)),
            },
        };
        self.register_function(func);
    }

    pub fn register_4_arg_core<
        I1: ArgType,
        I2: ArgType,
        I3: ArgType,
        I4: ArgType,
        O: ArgType,
        F,
        G,
    >(
        &mut self,
        name: &str,
        calc_domain: F,
        func: G,
    ) where
        F: Fn(
                &FunctionContext,
                &I1::Domain,
                &I2::Domain,
                &I3::Domain,
                &I4::Domain,
            ) -> FunctionDomain<O>
            + 'static
            + Clone
            + Copy
            + Send
            + Sync,
        G: for<'a> Fn(Value<I1>, Value<I2>, Value<I3>, Value<I4>, &mut EvalContext) -> Value<O>
            + 'static
            + Clone
            + Copy
            + Send
            + Sync,
    {
        let func = Function {
            signature: FunctionSignature {
                name: name.to_string(),
                args_type: vec![
                    I1::data_type(),
                    I2::data_type(),
                    I3::data_type(),
                    I4::data_type(),
                ],
                return_type: O::data_type(),
            },
            eval: FunctionEval::Scalar {
                calc_domain: Box::new(erase_calc_domain_generic_4_arg::<I1, I2, I3, I4, O>(
                    calc_domain,
                )),
                eval: Box::new(erase_function_generic_4_arg(func)),
            },
        };
        self.register_function(func);
    }
}

fn erase_calc_domain_generic_0_arg<O: ArgType>(
    func: impl Fn(&FunctionContext) -> FunctionDomain<O>,
) -> impl Fn(&FunctionContext, &[Domain]) -> FunctionDomain<AnyType> {
    move |ctx, args| func(ctx).map(O::upcast_domain)
}

fn erase_calc_domain_generic_1_arg<I1: ArgType, O: ArgType>(
    func: impl Fn(&FunctionContext, &I1::Domain) -> FunctionDomain<O>,
) -> impl Fn(&FunctionContext, &[Domain]) -> FunctionDomain<AnyType> {
    move |ctx, args| {
        let arg1 = I1::try_downcast_domain(&args[0]).unwrap();
        func(ctx, &arg1).map(O::upcast_domain)
    }
}

fn erase_calc_domain_generic_2_arg<I1: ArgType, I2: ArgType, O: ArgType>(
    func: impl Fn(&FunctionContext, &I1::Domain, &I2::Domain) -> FunctionDomain<O>,
) -> impl Fn(&FunctionContext, &[Domain]) -> FunctionDomain<AnyType> {
    move |ctx, args| {
        let arg1 = I1::try_downcast_domain(&args[0]).unwrap();
        let arg2 = I2::try_downcast_domain(&args[1]).unwrap();
        func(ctx, &arg1, &arg2).map(O::upcast_domain)
    }
}

fn erase_calc_domain_generic_3_arg<I1: ArgType, I2: ArgType, I3: ArgType, O: ArgType>(
    func: impl Fn(&FunctionContext, &I1::Domain, &I2::Domain, &I3::Domain) -> FunctionDomain<O>,
) -> impl Fn(&FunctionContext, &[Domain]) -> FunctionDomain<AnyType> {
    move |ctx, args| {
        let arg1 = I1::try_downcast_domain(&args[0]).unwrap();
        let arg2 = I2::try_downcast_domain(&args[1]).unwrap();
        let arg3 = I3::try_downcast_domain(&args[2]).unwrap();
        func(ctx, &arg1, &arg2, &arg3).map(O::upcast_domain)
    }
}

fn erase_calc_domain_generic_4_arg<
    I1: ArgType,
    I2: ArgType,
    I3: ArgType,
    I4: ArgType,
    O: ArgType,
>(
    func: impl Fn(
        &FunctionContext,
        &I1::Domain,
        &I2::Domain,
        &I3::Domain,
        &I4::Domain,
    ) -> FunctionDomain<O>,
) -> impl Fn(&FunctionContext, &[Domain]) -> FunctionDomain<AnyType> {
    move |ctx, args| {
        let arg1 = I1::try_downcast_domain(&args[0]).unwrap();
        let arg2 = I2::try_downcast_domain(&args[1]).unwrap();
        let arg3 = I3::try_downcast_domain(&args[2]).unwrap();
        let arg4 = I4::try_downcast_domain(&args[3]).unwrap();
        func(ctx, &arg1, &arg2, &arg3, &arg4).map(O::upcast_domain)
    }
}

fn erase_function_generic_0_arg<O: ArgType>(
    func: impl for<'a> Fn(&mut EvalContext) -> Value<O>,
) -> impl Fn(&[Value<AnyType>], &mut EvalContext) -> Value<AnyType> {
    move |args, ctx| Value::upcast(func(ctx))
}

fn erase_function_generic_1_arg<I1: ArgType, O: ArgType>(
    func: impl for<'a> Fn(Value<I1>, &mut EvalContext) -> Value<O>,
) -> impl Fn(&[Value<AnyType>], &mut EvalContext) -> Value<AnyType> {
    move |args, ctx| {
        let arg1 = args[0].try_downcast().unwrap();
        Value::upcast(func(arg1, ctx))
    }
}

fn erase_function_generic_2_arg<I1: ArgType, I2: ArgType, O: ArgType>(
    func: impl for<'a> Fn(Value<I1>, Value<I2>, &mut EvalContext) -> Value<O>,
) -> impl Fn(&[Value<AnyType>], &mut EvalContext) -> Value<AnyType> {
    move |args, ctx| {
        let arg1 = args[0].try_downcast().unwrap();
        let arg2 = args[1].try_downcast().unwrap();
        Value::upcast(func(arg1, arg2, ctx))
    }
}

fn erase_function_generic_3_arg<I1: ArgType, I2: ArgType, I3: ArgType, O: ArgType>(
    func: impl for<'a> Fn(Value<I1>, Value<I2>, Value<I3>, &mut EvalContext) -> Value<O>,
) -> impl Fn(&[Value<AnyType>], &mut EvalContext) -> Value<AnyType> {
    move |args, ctx| {
        let arg1 = args[0].try_downcast().unwrap();
        let arg2 = args[1].try_downcast().unwrap();
        let arg3 = args[2].try_downcast().unwrap();
        Value::upcast(func(arg1, arg2, arg3, ctx))
    }
}

fn erase_function_generic_4_arg<I1: ArgType, I2: ArgType, I3: ArgType, I4: ArgType, O: ArgType>(
    func: impl for<'a> Fn(Value<I1>, Value<I2>, Value<I3>, Value<I4>, &mut EvalContext) -> Value<O>,
) -> impl Fn(&[Value<AnyType>], &mut EvalContext) -> Value<AnyType> {
    move |args, ctx| {
        let arg1 = args[0].try_downcast().unwrap();
        let arg2 = args[1].try_downcast().unwrap();
        let arg3 = args[2].try_downcast().unwrap();
        let arg4 = args[3].try_downcast().unwrap();
        Value::upcast(func(arg1, arg2, arg3, arg4, ctx))
    }
}
