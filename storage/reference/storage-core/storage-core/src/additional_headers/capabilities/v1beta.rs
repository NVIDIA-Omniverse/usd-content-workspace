// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;

use crate::traits::{
    capabilities::v1beta::{Capabilities, ListTopLevelAddressesRequest, ListTopLevelAddressesResponse},
    error::Result,
    Request,
};

use super::super::AdditionalHeaderManager;

#[async_trait]
impl<S> Capabilities for AdditionalHeaderManager<S>
where
    S: Capabilities + Send + Sync,
{
    fn inner(&self) -> Option<&dyn Capabilities> {
        Some(&self.inner)
    }

    async fn list_top_level_addresses(
        &self,
        mut request: Request<ListTopLevelAddressesRequest>,
    ) -> Result<ListTopLevelAddressesResponse> {
        let additional_headers = self.parse(&request)?;
        request.extensions_mut().insert(additional_headers);

        self.inner.list_top_level_addresses(request).await
    }
}
