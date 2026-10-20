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

use storage_core::traits::Result;
use tonic::metadata::MetadataMap;

use crate::{
    backend::{metadata::Metadata, url_remapper::service::UrlRemapperService},
    common::{iterator::RetainEachExt, url_remapper::RewriteRule},
};

pub mod v1alpha;
pub mod v1beta;

impl<T: Metadata, S> UrlRemapperService<T, S> {
    /// Return the rewrite rules advertised through `ListTopLevelAddresses` whose
    /// target addresses the caller may see.
    async fn visible_top_level_rules(&self, metadata: &MetadataMap) -> Result<Vec<&RewriteRule>> {
        let mut rules: Vec<_> = self.mapper.top_level_rewrite_rules().collect();
        if let Some(filter) = &self.address_filter {
            if !rules.is_empty() {
                let mut retain = filter
                    .retain_top_level_addresses(
                        metadata,
                        &mut rules.iter().map(|rule| rule.get_target_folder_address()),
                    )
                    .await?
                    .into_iter();
                rules.retain_each(&mut retain);
            }
        }

        Ok(rules)
    }

    /// Return the rewrite rules advertised through `ListRoutes` whose target
    /// addresses the caller may see.
    async fn visible_route_rules(&self, metadata: &MetadataMap) -> Result<Vec<&RewriteRule>> {
        let mut rules: Vec<_> = self.mapper.rewrite_rules().collect();
        if let Some(filter) = &self.address_filter {
            if !rules.is_empty() {
                let mut retain = filter
                    .retain_routes(metadata, &mut rules.iter().map(|rule| rule.get_target_folder_address()))
                    .await?
                    .into_iter();
                rules.retain_each(&mut retain);
            }
        }

        Ok(rules)
    }
}
