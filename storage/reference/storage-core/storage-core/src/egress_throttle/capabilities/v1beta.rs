// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;

use storage_core_builder::traits::{
    capabilities::v1beta::{Capabilities, ListTopLevelAddressesRequest, ListTopLevelAddressesResponse},
    error::Result,
    Request,
};

use crate::egress_throttle::EgressThrottle;

#[async_trait]
impl<S> Capabilities for EgressThrottle<S>
where
    S: Capabilities,
{
    fn inner(&self) -> Option<&dyn Capabilities> {
        Some(&self.inner)
    }

    async fn list_top_level_addresses(
        &self,
        request: Request<ListTopLevelAddressesRequest>,
    ) -> Result<ListTopLevelAddressesResponse> {
        let _permit = self.semaphore.acquire().await;
        self.inner.list_top_level_addresses(request).await
    }
}
