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

use bit_vec::BitVec;
use storage_core::traits::Result;
use storage_service_permissions::types::ResourceType;
use tonic::{async_trait, metadata::MetadataMap};

use crate::backend::{
    permissions::{action::Action, error::Error, request::RequestExt, PermissionLayerInner, PermissionsLayer},
    url_remapper::visibility::AddressFilter,
};

impl PermissionsLayer {
    /// Create the address filter delegate for the URL remapper.
    pub fn address_filter(&self) -> PermissionAddressFilter {
        PermissionAddressFilter {
            inner: self.inner.clone(),
        }
    }
}

/// Evaluates remapper-produced addresses against the caller's permissions: an address
/// is retained when the caller is explicitly allowed on it, so an alias is visible in
/// both listings or in neither. An alias whose target endpoint bypasses permission
/// enforcement is always retained.
#[derive(Debug)]
pub struct PermissionAddressFilter {
    inner: Arc<PermissionLayerInner>,
}

#[async_trait]
impl AddressFilter for PermissionAddressFilter {
    async fn retain_top_level_addresses(
        &self,
        metadata: &MetadataMap,
        addresses: &mut (dyn Iterator<Item = &str> + Send),
    ) -> Result<BitVec> {
        self.retain_addresses(metadata, addresses, Action::ListTopLevelAddresses)
            .await
    }

    async fn retain_routes(
        &self,
        metadata: &MetadataMap,
        addresses: &mut (dyn Iterator<Item = &str> + Send),
    ) -> Result<BitVec> {
        self.retain_addresses(metadata, addresses, Action::ListRoutes).await
    }
}

impl PermissionAddressFilter {
    async fn retain_addresses(
        &self,
        metadata: &MetadataMap,
        addresses: &mut (dyn Iterator<Item = &str> + Send),
        action: Action,
    ) -> Result<BitVec> {
        let permission_metadata = metadata.get_permission_request_metadata(&self.inner.forward_headers)?;

        let mut batch = self.inner.batch();
        let bypassed: BitVec = addresses
            .map(|address| {
                let bypassed = if action == Action::ListRoutes {
                    self.inner.filter.is_route_excluded(address)?
                } else {
                    self.inner.filter.is_excluded(address, ResourceType::Folder)?
                };

                if !bypassed {
                    batch
                        .add_resource_hierarchically(address, Some(ResourceType::Folder), action)
                        .map_err(Error::response_resource_url_invalid)?;
                }

                Ok(bypassed)
            })
            .collect::<Result<_>>()?;

        let mut retain_iter = batch
            .execute(&permission_metadata)
            .await?
            .into_retain_iter_with_deny_on_missing();
        Ok(bypassed
            .into_iter()
            .map(|bypassed| bypassed || retain_iter.next() == Some(true))
            .collect())
    }
}
