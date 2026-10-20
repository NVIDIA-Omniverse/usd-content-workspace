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

use std::sync::Arc;

use futures::future::BoxFuture;
use storage_core::traits::{capabilities::v1alpha::*, Request, Result};

#[derive(Clone)]
pub struct AnyCapabilities {
    inner: Arc<dyn Capabilities>,
}

impl AnyCapabilities {
    pub fn new(inner: Arc<dyn Capabilities>) -> Self {
        Self { inner }
    }
}

impl Capabilities for AnyCapabilities {
    fn inner(&self) -> Option<&dyn Capabilities> {
        None
    }

    fn list_routes<'a: 'b, 'b>(
        &'a self,
        request: Request<ListRoutesRequest>,
    ) -> BoxFuture<'b, Result<ListRoutesResponse>> {
        self.inner.list_routes(request)
    }

    fn list_top_level_addresses<'a: 'b, 'b>(
        &'a self,
        request: Request<ListTopLevelAddressesRequest>,
    ) -> BoxFuture<'b, Result<ListTopLevelAddressesResponse>> {
        self.inner.list_top_level_addresses(request)
    }
}
