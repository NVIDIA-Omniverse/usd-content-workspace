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

use super::RpcMap;
use opentelemetry::StringValue;
use prost_types::FileDescriptorSet;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

/// A [`RpcMapBuilder`] error.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum RpcMapBuildError {
    /// The encoded file descriptor set is invalid.
    Decode(prost::DecodeError),
    /// A method with the same name has already been added (in the scope of a
    /// fully-qualified service).
    DuplicateMethodName,
    /// A file descriptor in a file descriptor set has no name.
    MissingFileName,
    /// A method descriptor in a service descriptor has no name.
    MissingMethodName,
    /// A service descriptor in a file descriptor has no name.
    MissingServiceName,
}

impl fmt::Display for RpcMapBuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode(e) => fmt::Display::fmt(e, f),
            Self::DuplicateMethodName => f.write_str("duplicate method name"),
            Self::MissingFileName => f.write_str("file descriptor has no name"),
            Self::MissingMethodName => f.write_str("service method has no name"),
            Self::MissingServiceName => f.write_str("service has no name"),
        }
    }
}

impl std::error::Error for RpcMapBuildError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Decode(e) => Some(e),
            Self::DuplicateMethodName
            | Self::MissingFileName
            | Self::MissingMethodName
            | Self::MissingServiceName => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Optimize {
    Static,
    RefCounted,
    Owned,
}

/// Optimization strategy for values in an [`RpcMap`] returned by the
/// [`RpcMapBuilder`].
///
/// Because of the design of the Rust OpenTelemetry library, some operations
/// such as emitting metrics with dynamic information can end up fairly
/// expensive. `StringValue`s must be cloned often, and the code relies on the
/// compiler to optimize them away.
///
/// By choosing trade-offs described in the individual optimization strategies,
/// the RPC map can provide significant optimizations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RpcMapOptimize(Optimize);

impl RpcMapOptimize {
    /// Values are static.
    ///
    /// This makes values very cheap to clone, but is basically a one-time
    /// memory leak when the `RpcMap` is constructed. For the common service
    /// construction pattern where the server, metric layers, and the RPC map
    /// are constructed once and live for the lifetime of the program, this is a
    /// great trade-off.
    pub const STATIC: Self = Self(Optimize::Static);

    /// Values are owned.
    ///
    /// This means values must be cloned as required. This is the most
    /// conservative and default option. For the common service construction
    /// pattern, this is usually not a good trade-off; see
    /// [`STATIC`](RpcMapOptimize::STATIC).
    pub const OWNED: Self = Self(Optimize::Owned);

    /// Values are atomically reference counted.
    ///
    /// This means fewer allocations and deallocations, at the expense of CPU
    /// time. Without careful profiling on a specific CPU model (!), this may
    /// lead to worse performance than other strategies.
    #[doc(hidden)]
    pub const REF_COUNTED: Self = Self(Optimize::RefCounted);
}

/// Builder for [`RpcMap::builder()`].
#[derive(Debug)]
pub struct RpcMapBuilder {
    optimize: Optimize,
    services: HashMap<String, HashMap<String, ()>>,
    processed_files: HashSet<String>,
}

impl RpcMapBuilder {
    pub(super) fn new() -> Self {
        Self {
            optimize: Optimize::Owned,
            services: HashMap::new(),
            processed_files: HashSet::new(),
        }
    }

    /// Set the optimization strategy for values in the RPC map.
    ///
    /// It's strongly encouraged to explicitly opt into a strategy instead of
    /// using the default value of the builder. See [`RpcMapOptimize`] for more
    /// information.
    pub fn optimize(mut self, optimize: RpcMapOptimize) -> Self {
        self.optimize = optimize.0;
        self
    }

    /// Add a byte slice containing an encoded
    /// [`prost_types::FileDescriptorSet`] to the RPC map.
    ///
    /// This is usually produced from `.proto` files using `tonic_prost_build`.
    pub fn add_encoded_file_descriptor_set(
        self,
        encoded_file_descriptor_set: &[u8],
    ) -> Result<Self, RpcMapBuildError> {
        use prost::Message as _;
        let file_descriptor_set = FileDescriptorSet::decode(encoded_file_descriptor_set)
            .map_err(RpcMapBuildError::Decode)?;
        self.add_file_descriptor_set(file_descriptor_set)
    }

    /// Add a [`prost_types::FileDescriptorSet`] to the RPC map.
    ///
    /// See [`add_encoded_file_descriptor_set()`](Self::add_encoded_file_descriptor_set).
    pub fn add_file_descriptor_set(
        mut self,
        file_descriptor_set: FileDescriptorSet,
    ) -> Result<Self, RpcMapBuildError> {
        // a `FileDescriptorSet` is usually a service definition, and often
        // includes imported files of e.g. common types.
        //
        // a slightly modified `helloworld.proto` example:
        // ```
        // syntax = "proto3";
        // import "common/types.proto";
        // package helloworld;
        //
        // service Greeter {
        //   rpc SayHello(CommonRequest) returns (CommonResponse) {}
        // }
        // ```
        // we mainly want to extract RPC names in the form
        // `helloworld.Greeter/SayHello`.
        for fd in file_descriptor_set.file {
            // extract the package name. although highly undesirable, it's
            // possible for this to be empty.
            let package_name = fd.package.filter(|s| !s.is_empty());

            // in theory, imported files shouldn't include service definitions.
            // but if they do, only processing the file once avoids having
            // spurious "duplicate" method names.
            let file_name = fd.name.ok_or(RpcMapBuildError::MissingFileName)?;
            let new_file = self.processed_files.insert(file_name);
            if !new_file {
                continue;
            }

            for sd in fd.service {
                let service_name = sd.name.ok_or(RpcMapBuildError::MissingServiceName)?;
                // build the fully-qualified service name.
                let key = match package_name.as_deref() {
                    Some(pkg) => format!("{pkg}.{service_name}"),
                    None => service_name,
                };

                // i don't know if anybody is brave enough to split service
                // definitions over multiple files, but this code does support
                // that.
                let service = self.services.entry(key).or_default();
                for md in sd.method {
                    let method_name = md.name.ok_or(RpcMapBuildError::MissingMethodName)?;
                    match service.entry(method_name) {
                        Entry::Occupied(_e) => return Err(RpcMapBuildError::DuplicateMethodName),
                        Entry::Vacant(e) => {
                            // ideally, we could also capture whether the method
                            // uses streaming and automatically adjust whether
                            // requests and responses need streaming metrics.
                            e.insert(());
                        }
                    }
                }
            }
        }

        Ok(self)
    }

    /// Consumes the builder and returns the built RPC map.
    pub fn build(self) -> RpcMap {
        let Self {
            optimize,
            services,
            processed_files: _,
        } = self;

        let mut services: Vec<_> = services.into_iter().collect();
        services.sort_by(|a, b| a.0.cmp(&b.0));

        let mut routes = Vec::new();
        for (service_name, methods) in services {
            let mut methods: Vec<_> = methods.into_iter().collect();
            methods.sort_by(|a, b| a.0.cmp(&b.0));

            for (method_name, ()) in methods {
                let route_name = format!("{service_name}/{method_name}");

                let value = match optimize {
                    Optimize::Static => {
                        let static_ref: &'static str = Box::leak(route_name.into_boxed_str());
                        StringValue::from(static_ref)
                    }
                    Optimize::RefCounted => {
                        let counted_ref: Arc<str> = route_name.into();
                        StringValue::from(counted_ref)
                    }
                    Optimize::Owned => StringValue::from(route_name),
                };
                routes.push(value);
            }
        }

        RpcMap(routes)
    }
}
