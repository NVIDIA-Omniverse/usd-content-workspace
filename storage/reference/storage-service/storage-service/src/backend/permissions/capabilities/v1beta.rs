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

use bit_vec::BitVec;
use storage_core::traits::{capabilities::v1beta::*, Request, Result};
use storage_service_permissions::types::ResourceType;
use tonic::async_trait;

use crate::backend::permissions::{action::Action, error::Error, request::RequestExt, Permissions};

#[async_trait]
impl<S> Capabilities for Permissions<S>
where
    S: Capabilities,
{
    fn inner(&self) -> Option<&dyn Capabilities> {
        None
    }

    async fn list_top_level_addresses(
        &self,
        request: Request<ListTopLevelAddressesRequest>,
    ) -> Result<ListTopLevelAddressesResponse> {
        let permission_metadata = request.get_permission_request_metadata(&self.inner.forward_headers)?;
        let mut response = self.next.list_top_level_addresses(request).await?;

        // A bypassed endpoint's top-level address is always listed, even for a
        // principal with no ACLs, so only the enforced items enter the batch.
        let mut batch = self.batch();
        let bypassed: BitVec = response
            .items
            .iter()
            .map(|item| {
                let bypassed = self.is_permission_check_excluded(&item.top_level_address, ResourceType::Folder)?;
                if !bypassed {
                    batch
                        .add_resource_hierarchically(
                            &item.top_level_address,
                            Some(ResourceType::Folder),
                            Action::ListTopLevelAddresses,
                        )
                        .map_err(Error::response_resource_url_invalid)?;
                }

                Ok(bypassed)
            })
            .collect::<Result<_>>()?;

        let mut retain_iter = batch
            .execute(&permission_metadata)
            .await?
            .into_retain_iter_with_deny_on_missing();
        let mut bypassed = bypassed.into_iter();
        response
            .items
            .retain(|_| bypassed.next() == Some(true) || retain_iter.next() == Some(true));

        Ok(response)
    }
}
