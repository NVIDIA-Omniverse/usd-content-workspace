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

use storage_core::traits::{capabilities::v1alpha::*, Request, Result};
use tonic::async_trait;

use crate::{
    backend::{metadata::Metadata, url_remapper::service::UrlRemapperService},
    common::url_remapper::RewriteRule,
};

#[async_trait]
impl<T: Metadata, S: Capabilities> Capabilities for UrlRemapperService<T, S> {
    fn inner(&self) -> Option<&dyn Capabilities> {
        Some(&self.inner)
    }

    async fn list_top_level_addresses(
        &self,
        request: Request<ListTopLevelAddressesRequest>,
    ) -> Result<ListTopLevelAddressesResponse> {
        let rules = self.visible_top_level_rules(request.metadata()).await?;

        let mut response = self.inner.list_top_level_addresses(request).await?;
        // A rewrite rule takes precedence over any backend address it shadows: browsing a rule
        // source routes to the rule's target, so the backend's own entry for that address must not
        // be advertised on its own -- its visibility would otherwise follow the shadowed resource's
        // permissions rather than the target's. Drop shadowed backend entries; the visible rules,
        // already filtered by target permission above, are added back below.
        response
            .items
            .retain(|item| !self.mapper.is_mount_point(&item.top_level_address));
        response
            .items
            .extend(rules.into_iter().map(RewriteRule::get_top_level_address_entry_v1alpha));

        Ok(response)
    }

    async fn list_routes(&self, request: Request<ListRoutesRequest>) -> Result<ListRoutesResponse> {
        let rules = self.visible_route_rules(request.metadata()).await?;

        let mut response = self.inner.list_routes(request).await?;
        response
            .items
            .extend(rules.into_iter().map(RewriteRule::get_route_v1alpha));

        Ok(response)
    }
}
