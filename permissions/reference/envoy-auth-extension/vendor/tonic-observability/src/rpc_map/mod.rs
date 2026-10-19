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

//! An RPC map tells layers what RPCs to expect. Layer use this for
//! optimization, or for security reasons to limit cardinality.
//!
//! See [`RpcMap::builder()`] to get started.
mod builder;

pub use builder::{RpcMapBuildError, RpcMapBuilder, RpcMapOptimize};
use opentelemetry::StringValue;

/// A map of RPC services and methods.
///
/// See [`RpcMap::builder()`] for how to construct a new RPC map and why.
#[derive(Debug, Clone, PartialEq)]
pub struct RpcMap(Vec<StringValue>);

impl Default for RpcMap {
    #[inline]
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl RpcMap {
    /// Returns a builder for a new RPC map.
    ///
    /// Knowing which RPCs to expect allows metric layers to optimize memory
    /// usage for frequently called RPCs and - in future - emit streaming
    /// metrics for only streaming RPCs.
    ///
    /// The builder allows for multiple services to be added to the RPC map via
    /// file descriptor sets.
    ///
    /// It's strongly encouraged to explicitly opt into a strategy instead of
    /// using the default value of the builder. See [`RpcMapOptimize`] for the
    /// different optimization strategies and trade-offs.
    ///
    /// ```rust
    /// use tonic_observability::rpc_map::{RpcMap, RpcMapOptimize};
    /// let rpc_map = RpcMap::builder()
    ///     // choose an optimization strategy (default is `OWNED`)
    ///     .optimize(RpcMapOptimize::STATIC)
    ///     // add one or more service file descriptor sets
    ///     .add_encoded_file_descriptor_set(protos::FILE_DESCRIPTOR_SET)
    ///     .unwrap()
    ///     .build();
    /// ```
    pub fn builder() -> RpcMapBuilder {
        RpcMapBuilder::new()
    }

    /// Returns `true` if the RPC map is empty.
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Find an RPC in the RPC map.
    ///
    pub(crate) fn find_ref<'a>(&'a self, rpc: &str) -> Option<&'a StringValue> {
        self.0.iter().find(|s| s.as_str() == rpc)
    }

    /// Find an RPC in the RPC map.
    ///
    /// This is mainly used to convert a string with a lifetime to a static
    /// string.
    pub(crate) fn find(&self, rpc: &str) -> Option<StringValue> {
        self.find_ref(rpc).cloned()
    }
}
