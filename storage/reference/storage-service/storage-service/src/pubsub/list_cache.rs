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

use storage_core::list_cache::ListCache;
use tonic::async_trait;
use tracing::{debug, warn};

use crate::{
    common::error::PrettyPrintErrorExt,
    pubsub::types::{ConsumeRequest, ConsumeResults, Consumer, StorageEvent},
};

#[async_trait]
impl<S: Consumer> Consumer for ListCache<S> {
    async fn consume(&self, events: ConsumeRequest) -> ConsumeResults {
        for event in &events {
            let resource_address = match event {
                StorageEvent::ObjectCreated(data) => data.resource_address.as_str(),
                StorageEvent::ObjectDeleted(data) => data.resource_address.as_str(),
                StorageEvent::DirCreated(data) => data.resource_address.as_str(),
                StorageEvent::DirDeleted(data) => data.resource_address.as_str(),
            };

            debug!(?resource_address, "invalidating a list cache entry");

            if let Err(error) = self.invalidate_cache_entry_by_object_url(resource_address).await {
                warn!(error = %error.pretty(), ?resource_address, "failed to invalidate a list cache entry");
            }
        }

        self.inner.consume(events).await
    }
}
