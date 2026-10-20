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

use std::fmt::Debug;

mod capabilities;
mod filefolder;
mod fileobject;
mod versioning;

pub mod selector;

mod error;

/// Request matcher.
pub trait Match: Send + Sync + 'static {
    /// Whether this backend can process a request against a resource address.
    fn match_resource_address(&self, value: &str) -> bool;

    /// Whether this backend can process a request against a resource identity.
    fn match_resource_identity(&self, value: &str) -> bool;

    /// Whether this backend can process a request against a folder address.
    fn match_folder_address(&self, value: &str) -> bool;

    /// Whether a top-level address produced by this backend lies under an endpoint advertised
    /// in `ListTopLevelAddresses`. Serving is unaffected: a hidden endpoint is handled as
    /// usual, it is just not advertised.
    fn filter_top_level_address(&self, address: &str) -> bool;

    /// The wildcard routes this backend advertises in `ListRoutes`. Serving is unaffected: a
    /// backend hidden from routes stays fully configured and is reachable through a
    /// url-remapper alias.
    fn iter_wildcard_routes(&self) -> impl Iterator<Item = &str>;
}

/// A chain of storage backends.
#[derive(Clone)]
pub struct Service<A, B> {
    pub(self) head: A,
    pub(self) tail: B,
}

impl<A, B> Service<A, B> {
    pub fn backend<S, P>(self, service: S, selector: P) -> Service<ServiceRoute<S, P>, Self> {
        Service {
            head: ServiceRoute { service, selector },
            tail: self,
        }
    }
}

impl<A: Debug, B: Debug> Debug for Service<A, B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Service")
            .field("head", &self.head)
            .field("tail", &self.tail)
            .finish()
    }
}

/// A service builder.
#[derive(Default, Debug, Clone, Copy)]
pub struct Builder;

impl Builder {
    pub fn backend<S, P>(self, service: S, selector: P) -> Service<ServiceRoute<S, P>, Dummy> {
        Service {
            head: ServiceRoute { service, selector },
            tail: Dummy,
        }
    }
}

/// A dummy storage backend that responds with errors.
#[derive(Default, Debug, Copy, Clone)]
pub struct Dummy;

/// A service with an attached route selector.
pub struct ServiceRoute<S, M> {
    pub(self) service: S,
    pub(self) selector: M,
}

impl<S: Debug, M: Debug> Debug for ServiceRoute<S, M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Route")
            .field("service", &self.service)
            .field("match_route", &self.selector)
            .finish()
    }
}
