// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use storage_core::traits::capabilities::{
    v1alpha::{
        ListServicesRequest, ListServicesResponse, ListTopLevelAddressesRequest, ListTopLevelAddressesResponse,
        ServiceEntry, TopLevelAddressEntry,
    },
    v1beta,
};
use tonic::transport::Channel;
use tonic::Status;

use super::super::{connect_with_retries, request_with_headers_config};
use crate::clients::traits::capabilities::Capabilities;
use crate::headers::HeadersConfig;

pub struct CapabilitiesClient {
    client: v1beta::capabilities_service_client::CapabilitiesServiceClient<Channel>,
    headers_config: HeadersConfig,
}

impl CapabilitiesClient {
    pub async fn new(address: impl AsRef<str>, headers_config: HeadersConfig) -> Self {
        let client = connect_with_retries(|| {
            v1beta::capabilities_service_client::CapabilitiesServiceClient::connect(address.as_ref().to_owned())
        })
        .await;
        Self { client, headers_config }
    }
}

#[tonic::async_trait]
impl Capabilities for CapabilitiesClient {
    async fn list_services(&mut self, _request: ListServicesRequest) -> Result<ListServicesResponse, Status> {
        let request = v1beta::ListServicesRequest {};
        let response = self
            .client
            .list_services(request_with_headers_config(request, &self.headers_config))
            .await
            .map(|response| response.into_inner())?;
        Ok(ListServicesResponse {
            services: response
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
        let request = v1beta::ListTopLevelAddressesRequest {};
        let response = self
            .client
            .list_top_level_addresses(request_with_headers_config(request, &self.headers_config))
            .await
            .map(|response| response.into_inner())?;
        Ok(ListTopLevelAddressesResponse {
            items: response
                .items
                .into_iter()
                .map(|i| TopLevelAddressEntry {
                    top_level_address: i.top_level_address,
                })
                .collect(),
        })
    }
}
