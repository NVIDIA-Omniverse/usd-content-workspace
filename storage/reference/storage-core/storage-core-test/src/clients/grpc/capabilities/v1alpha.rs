// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use storage_core::traits::capabilities::v1alpha::{
    self, ListRoutesRequest, ListRoutesResponse, ListServicesRequest, ListServicesResponse,
    ListTopLevelAddressesRequest, ListTopLevelAddressesResponse,
};
use tonic::transport::Channel;
use tonic::Status;

use super::super::{connect_with_retries, request_with_headers_config};
use crate::clients::traits::capabilities::Capabilities;
use crate::headers::HeadersConfig;

pub struct CapabilitiesClient {
    client: v1alpha::capabilities_service_client::CapabilitiesServiceClient<Channel>,
    headers_config: HeadersConfig,
}

impl CapabilitiesClient {
    pub async fn new(address: impl AsRef<str>, headers_config: HeadersConfig) -> Self {
        let client = connect_with_retries(|| {
            v1alpha::capabilities_service_client::CapabilitiesServiceClient::connect(address.as_ref().to_owned())
        })
        .await;
        Self { client, headers_config }
    }
}

#[tonic::async_trait]
impl Capabilities for CapabilitiesClient {
    async fn list_services(&mut self, request: ListServicesRequest) -> Result<ListServicesResponse, Status> {
        self.client
            .list_services(request_with_headers_config(request, &self.headers_config))
            .await
            .map(|response| response.into_inner())
    }

    async fn list_top_level_addresses(
        &mut self,
        request: ListTopLevelAddressesRequest,
    ) -> Result<ListTopLevelAddressesResponse, Status> {
        self.client
            .list_top_level_addresses(request_with_headers_config(request, &self.headers_config))
            .await
            .map(|response| response.into_inner())
    }

    async fn list_routes(&mut self, request: ListRoutesRequest) -> Result<ListRoutesResponse, Status> {
        self.client
            .list_routes(request_with_headers_config(request, &self.headers_config))
            .await
            .map(|response| response.into_inner())
    }
}
