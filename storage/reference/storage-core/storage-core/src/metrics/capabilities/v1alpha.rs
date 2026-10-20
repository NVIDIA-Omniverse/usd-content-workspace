// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;

use crate::traits::{
    capabilities::v1alpha::{
        Capabilities, ListRoutesRequest, ListRoutesResponse, ListTopLevelAddressesRequest,
        ListTopLevelAddressesResponse,
    },
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
        let _timer = self.list_top_level_addresses_metrics_v1alpha.latency.start();

        record_operation!(
            self.list_top_level_addresses_metrics_v1alpha,
            self.inner.list_top_level_addresses(request).await
        )
    }

    async fn list_routes(&self, request: Request<ListRoutesRequest>) -> Result<ListRoutesResponse> {
        let _timer = self.list_routes_metrics_v1alpha.latency.start();

        record_operation!(self.list_routes_metrics_v1alpha, self.inner.list_routes(request).await)
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
        let _timer = self.list_top_level_addresses_metrics_v1alpha.latency.start();

        record_operation!(
            self.list_top_level_addresses_metrics_v1alpha,
            self.inner.list_top_level_addresses(request).await
        )
    }

    async fn list_routes(&self, request: Request<ListRoutesRequest>) -> Result<ListRoutesResponse> {
        let _timer = self.list_routes_metrics_v1alpha.latency.start();

        record_operation!(self.list_routes_metrics_v1alpha, self.inner.list_routes(request).await)
    }
}
