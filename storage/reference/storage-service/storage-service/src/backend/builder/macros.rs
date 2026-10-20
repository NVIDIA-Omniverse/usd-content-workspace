// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0
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

#[macro_export]
macro_rules! __impl_service_aggregate {
    (
        $type:ident,
        $(
            { $field:ident, $field_type:ty, $field_trait:path }
        ),+ $(,)?
    ) => {
        #[derive(Clone)]
        pub struct $type {
            $(
                pub $field: $field_type,
            )+
        }

        impl $type {
            pub fn decorate(self, decorator: &(impl $(Decorate<$field_type>+)+)) -> Self {
                Self {
                    $(
                        $field: decorator.decorate(self.clone()),
                    )+
                }
            }

            pub fn decorate_each(self, $($field: &impl Decorate<$field_type>,)+) -> Self {
                Self {
                    $(
                        $field: $field.decorate(self.clone()),
                    )+
                }
            }
        }

        pub trait Decorate<T> {
            fn decorate(&self, inner: $type) -> T;
        }

        $(
            impl <L: ::tower::Layer<$type>> Decorate<$field_type> for L where L::Service: $field_trait {
                fn decorate(&self, inner: $type) -> $field_type {
                    <$field_type>::new(::std::sync::Arc::new(self.layer(inner)))
                }
            }

            impl <L: ::tower::Layer<$type>> Decorate<$field_type> for Option<L> where L::Service: $field_trait {
                fn decorate(&self, inner: $type) -> $field_type {
                    if let Some(ref value) = self {
                        value.decorate(inner)
                    } else {
                        inner.$field
                    }
                }
            }

            impl Decorate<$field_type> for $crate::backend::builder::Skip {
                fn decorate(&self, inner: $type) -> $field_type {
                    inner.$field
                }
            }
        )+
    };
}
