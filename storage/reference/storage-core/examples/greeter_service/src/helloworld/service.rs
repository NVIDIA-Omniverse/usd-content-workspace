// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use storage_core::api_services::ServiceBuilder;
use storage_core::layer::Layer;
use storage_core::service_registry::{ServiceInfo, ServiceInfoProvider};

use super::traits::Greeter as GreeterTrait;

/// The GreeterBuilder is a specialization of the BackendBuilder for the Greeter Service.
// TODO(saxonp): Maybe we should have a macro to generate this code?
pub type GreeterBuilder<L, B> = ServiceBuilder<L, Greeter<<L as Layer<B>>::Service>>;

/// The concrete Greeter struct. This will be used to implement the ingress traits.
pub struct Greeter<T: GreeterTrait> {
    pub(super) greeter: Arc<T>,
}

impl<T: GreeterTrait> Greeter<T> {
    pub fn new(greeter: T) -> Self {
        Self {
            greeter: Arc::new(greeter),
        }
    }
}

/// Create a Greeter from a GreeterTrait.
impl<T: GreeterTrait> From<T> for Greeter<T> {
    fn from(greeter: T) -> Self {
        Self::new(greeter)
    }
}

impl<T: GreeterTrait> Clone for Greeter<T> {
    fn clone(&self) -> Self {
        Self {
            greeter: self.greeter.clone(),
        }
    }
}

/// The ServiceInfoProvider trait must be implemented to add the service to the storage service builder.
/// This is how the storage service builder generates the response for the list_services method.
impl<T: GreeterTrait> ServiceInfoProvider for Greeter<T> {
    fn service_info(&self) -> Option<ServiceInfo> {
        Some(ServiceInfo {
            name: "greeter".to_string(),
            version: "v1alpha".to_string(),
        })
    }
}
