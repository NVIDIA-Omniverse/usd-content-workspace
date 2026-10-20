// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use storage_core::traits::capabilities::v1alpha::{
    ListRoutesRequest, ListRoutesResponse, ListServicesRequest, ListServicesResponse, ListTopLevelAddressesRequest,
    ListTopLevelAddressesResponse,
};
use tonic::Status;

use super::CapabilitiesRestClient;
use crate::clients::traits::capabilities::Capabilities;
use crate::headers::HeadersConfig;

pub struct CapabilitiesClient {
    client: CapabilitiesRestClient,
}

impl CapabilitiesClient {
    pub fn new(port: u16, headers_config: HeadersConfig) -> Self {
        Self {
            client: CapabilitiesRestClient::new(port, headers_config),
        }
    }
}

#[tonic::async_trait]
impl Capabilities for CapabilitiesClient {
    async fn list_services(&mut self, _request: ListServicesRequest) -> Result<ListServicesResponse, Status> {
        let body = self.client.list_services("v1alpha/capabilities/services").await?;
        let response: ListServicesResponse = serde_json::from_slice(&body).unwrap();
        Ok(response)
    }

    async fn list_top_level_addresses(
        &mut self,
        _request: ListTopLevelAddressesRequest,
    ) -> Result<ListTopLevelAddressesResponse, Status> {
        let body = self
            .client
            .list_top_level_addresses("v1alpha/capabilities/top-level-addresses")
            .await?;
        let response: ListTopLevelAddressesResponse = serde_json::from_slice(&body).unwrap();
        Ok(response)
    }

    async fn list_routes(&mut self, _request: ListRoutesRequest) -> Result<ListRoutesResponse, Status> {
        let body = self.client.list_routes("v1alpha/capabilities/routes").await?;
        let response: ListRoutesResponse = serde_json::from_slice(&body).unwrap();
        Ok(response)
    }
}
