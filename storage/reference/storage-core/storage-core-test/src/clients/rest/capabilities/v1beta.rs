// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use storage_core::traits::capabilities::v1alpha::{
    ListServicesRequest, ListServicesResponse, ListTopLevelAddressesRequest, ListTopLevelAddressesResponse,
    ServiceEntry, TopLevelAddressEntry,
};
use storage_core::traits::capabilities::v1beta;
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
        let v1beta_response: v1beta::ListServicesResponse = serde_json::from_slice(&body).unwrap();
        Ok(ListServicesResponse {
            services: v1beta_response
                .services
                .into_iter()
                .map(|s| ServiceEntry {
                    service_name: s.service_name,
                    service_versions: s.service_versions,
                })
                .collect(),
        })
    }

    async fn list_top_level_addresses(
        &mut self,
        _request: ListTopLevelAddressesRequest,
    ) -> Result<ListTopLevelAddressesResponse, Status> {
        let body = self
            .client
            .list_top_level_addresses("v1beta/capabilities/top-level-addresses")
            .await?;
        let v1beta_response: v1beta::ListTopLevelAddressesResponse = serde_json::from_slice(&body).unwrap();
        Ok(ListTopLevelAddressesResponse {
            items: v1beta_response
                .items
                .into_iter()
                .map(|i| TopLevelAddressEntry {
                    top_level_address: i.top_level_address,
                })
                .collect(),
        })
    }
}
