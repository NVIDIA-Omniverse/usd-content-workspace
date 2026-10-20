// SPDX-FileCopyrightText: Copyright (c) 2024-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
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

//! Use [`ResourceBuilder`] to define resource attributes.
//!
//! The SDK's `Resource`/`ResourceBuilder` is a mess. They are not clone-able,
//! and the API is not stable.

use opentelemetry::{Key, KeyValue, StringValue, Value};
/// Re-exported from [`opentelemetry_sdk::Resource`].
///
///
pub use opentelemetry_sdk::resource::Resource;
use opentelemetry_sdk::resource::{
    EnvResourceDetector, SdkProvidedResourceDetector, TelemetryResourceDetector,
};
use opentelemetry_semantic_conventions::resource as res;
use std::collections::HashMap;
use std::fmt;

#[doc(hidden)]
pub use opentelemetry_sdk::resource::ResourceDetector;

#[doc(hidden)]
#[macro_export]
macro_rules! __detectors {
    [$($d:expr),+ $(,)?] => {
        &[
            $(::std::boxed::Box::new($d) as ::std::boxed::Box<dyn $crate::resource::ResourceDetector>,)+
        ]
    };
}

/// Define a <code>Vec<Box<dyn [ResourceDetector]>></code> for use with
/// [`Resource::from_detectors()`].
///
/// All provided values are boxed by the macro.
///
/// ```rust
/// use opentelemetry_sdk::Resource;
/// use opentelemetry_sdk::resource::{
///     EnvResourceDetector, SdkProvidedResourceDetector, TelemetryResourceDetector,
/// };
/// use otel_tools::resource::detectors;
///
/// let detectors = detectors![
///     TelemetryResourceDetector,
///     SdkProvidedResourceDetector,
///     EnvResourceDetector::new(),
/// ];
/// let resource = Resource::builder_empty().with_detectors(detectors).build();
/// ```
#[doc(inline)]
pub use crate::__detectors as detectors;

/// Provide default resource attributes from environment variables and SDK
/// values.
///
/// This is similar to [`Resource::default()`], but may deviate from that
/// implementation in time.
///
/// This method may block for some duration while default resources are
/// gathered, and should not be called in an asynchronous context.
fn default_resource() -> Resource {
    let detectors = detectors![
        // Sets `telemetry.sdk.name`, `telemetry.sdk.language`, and
        // `telemetry.sdk.version`
        TelemetryResourceDetector,
        // Respects `OTEL_SERVICE_NAME`, and `service.name` from
        // `OTEL_RESOURCE_ATTRIBUTES`
        SdkProvidedResourceDetector,
        // Respects `OTEL_RESOURCE_ATTRIBUTES`
        EnvResourceDetector::new(),
    ];
    Resource::builder_empty().with_detectors(detectors).build()
}

fn resource_to_kv(resource: Resource) -> HashMap<Key, Value> {
    resource
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// Mutable resource, as [`Resource`] cannot be changed.
#[derive(Clone, PartialEq)]
#[repr(transparent)]
struct ResourceMut(HashMap<&'static str, StringValue>);

impl fmt::Debug for ResourceMut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.0.iter()).finish()
    }
}

impl ResourceMut {
    fn new() -> Self {
        Self(HashMap::new())
    }

    fn insert(&mut self, k: &'static str, v: StringValue) {
        let _prev = self.0.insert(k, v);
    }
}

/// A convenient builder for [`Resource`].
///
/// This builder respects all environment variables specified in the
/// observability guide.
///
/// Do not use this in an asynchronous context.
#[derive(Debug, Clone, PartialEq)]
pub struct ResourceBuilder {
    resource: ResourceMut,
    default: HashMap<Key, Value>,
}

impl ResourceBuilder {
    /// Create a new resource builder with the service name and default resource
    /// attributes. It is not async-safe!
    ///
    /// This method may block for some duration while default resources are
    /// gathered, and should not be called in an asynchronous context.
    pub fn new(service_name: impl Into<StringValue>) -> Self {
        let mut resource = ResourceMut::new();
        resource.insert(res::SERVICE_NAME, service_name.into());
        let default = resource_to_kv(default_resource());
        Self { resource, default }
    }

    /// Create a new resource builder with the service name and no default
    /// resource attributes.
    pub fn without_default(service_name: impl Into<StringValue>) -> Self {
        let mut resource = ResourceMut::new();
        resource.insert(res::SERVICE_NAME, service_name.into());
        let default = HashMap::new();
        Self { resource, default }
    }

    /// Override the default resource attributes.
    pub fn with_default(mut self, resource: Resource) -> Self {
        self.default = resource_to_kv(resource);
        self
    }

    /// Set the service namespace.
    pub fn with_service_namespace(mut self, service_namespace: impl Into<StringValue>) -> Self {
        self.resource
            .insert(res::SERVICE_NAMESPACE, service_namespace.into());
        self
    }

    /// Set the service version.
    pub fn with_service_version(mut self, service_version: impl Into<StringValue>) -> Self {
        self.resource
            .insert(res::SERVICE_VERSION, service_version.into());
        self
    }

    /// Get default resource attributes, and merge them with the user-provided
    /// attributes.
    pub fn build(self) -> Resource {
        let Self { resource, default } = self;
        // this order matches the Otel spec:
        // > the user provided resource information has higher priority [than OTEL_RESOURCE_ATTRIBUTES]
        // https://github.com/open-telemetry/opentelemetry-specification/blob/v1.41.0/specification/resource/sdk.md#specifying-resource-information-via-an-environment-variable
        let mut merged = default;
        for (k, v) in resource.0.into_iter() {
            let k = Key::from_static_str(k);
            let v = Value::String(v);
            let _prev = merged.insert(k, v);
        }
        let kvs = merged
            .into_iter()
            .map(|(key, value)| KeyValue::new(key, value));
        Resource::builder_empty().with_attributes(kvs).build()
    }
}

#[cfg(test)]
mod tests;
