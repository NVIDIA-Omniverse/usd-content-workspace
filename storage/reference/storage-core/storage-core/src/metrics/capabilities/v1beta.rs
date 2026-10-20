// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;

use crate::traits::{
    capabilities::v1beta::{Capabilities, ListTopLevelAddressesRequest, ListTopLevelAddressesResponse},
    error::Result,
    Request,
};

use super::super::{OperationMetrics, SdkMetrics};

#[async_trait]
impl<S> Capabilities for OperationMetrics<S>
where
    S: Capabilities + Send + Sync,
{
    fn inner(&self) -> Option<&dyn Capabilities> {
        Some(&self.inner)
    }

    async fn list_top_level_addresses(
        &self,
        request: Request<ListTopLevelAddressesRequest>,
    ) -> Result<ListTopLevelAddressesResponse> {
        let _timer = self.list_top_level_addresses_metrics_v1beta.latency.start();

        record_operation!(
            self.list_top_level_addresses_metrics_v1beta,
            self.inner.list_top_level_addresses(request).await
        )
    }
}

#[async_trait]
impl<S> Capabilities for SdkMetrics<S>
where
    S: Capabilities + Send + Sync,
{
    fn inner(&self) -> Option<&dyn Capabilities> {
        Some(&self.inner)
    }

    async fn list_top_level_addresses(
        &self,
        request: Request<ListTopLevelAddressesRequest>,
    ) -> Result<ListTopLevelAddressesResponse> {
        let _timer = self.list_top_level_addresses_metrics_v1beta.latency.start();

        record_operation!(
            self.list_top_level_addresses_metrics_v1beta,
            self.inner.list_top_level_addresses(request).await
        )
    }
}
