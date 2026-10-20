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

pub mod grpc;
pub mod rest;

mod adapters;

pub mod v1alpha {
    use storage_core::{
        api_services::ServiceBuilder,
        layer::Layer,
        service_registry::{ServiceInfo, ServiceInfoProvider},
    };

    use crate::backend::metadata::Metadata as MetadataTrait;
    use std::sync::Arc;

    pub type MetadataBuilder<L, B> = ServiceBuilder<L, Metadata<<L as Layer<B>>::Service>>;

    pub struct Metadata<T: MetadataTrait> {
        pub(super) inner: Arc<T>,
    }

    impl<T: MetadataTrait> Metadata<T> {
        pub fn new(inner: T) -> Self {
            Self { inner: Arc::new(inner) }
        }
    }

    impl<T: MetadataTrait> ServiceInfoProvider for Metadata<T> {
        fn service_info(&self) -> Option<ServiceInfo> {
            Some(ServiceInfo {
                name: "metadata".into(),
                version: "v1alpha".into(),
            })
        }
    }

    impl<T: MetadataTrait> From<T> for Metadata<T> {
        fn from(value: T) -> Self {
            Self::new(value)
        }
    }

    impl<T: MetadataTrait> Clone for Metadata<T> {
        fn clone(&self) -> Self {
            Self {
                inner: self.inner.clone(),
            }
        }
    }
}
