// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use storage_core::traits::{
    fileobject::v1alpha::{Metadata, ResourceIdentity, ResourceInfo},
    versioning::{
        v1alpha::{EnumerateVersionsRequest, EnumerateVersionsResponse, VersionInfo},
        v1beta,
    },
};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::Channel;
use tonic::Status;

use super::super::{connect_with_retries, request_with_headers_config};
use crate::clients::traits::versioning::Versioning;
use crate::headers::HeadersConfig;

pub struct VersioningClient {
    client: v1beta::versioning_service_client::VersioningServiceClient<Channel>,
    headers_config: HeadersConfig,
}

impl VersioningClient {
    pub async fn new(address: impl AsRef<str>, headers_config: HeadersConfig) -> Self {
        let client = connect_with_retries(|| {
            v1beta::versioning_service_client::VersioningServiceClient::connect(address.as_ref().to_owned())
        })
        .await;
        Self { client, headers_config }
    }
}

#[tonic::async_trait]
impl Versioning for VersioningClient {
    async fn enumerate_versions(
        &mut self,
        request: EnumerateVersionsRequest,
    ) -> Result<ReceiverStream<Result<EnumerateVersionsResponse, Status>>, Status> {
        // Convert v1alpha request to v1beta request
        let v1beta_request = v1beta::EnumerateVersionsRequest {
            resource_address: request.resource_address,
        };

        let (tx, rx) = mpsc::channel(1);
        let mut stream = self
            .client
            .enumerate_versions(request_with_headers_config(v1beta_request, &self.headers_config))
            .await?
            .into_inner();

        tokio::spawn(async move {
            loop {
                let response = stream.message().await;
                match response {
                    Ok(Some(response)) => {
                        // Convert v1beta response back to v1alpha response
                        let v1alpha_response = EnumerateVersionsResponse {
                            items: response
                                .items
                                .into_iter()
                                .map(|v| VersionInfo {
                                    resource_info: v.resource_info.map(|v| ResourceInfo {
                                        resource_identity: v.resource_identity.map(|v| ResourceIdentity {
                                            encoded_identity: v.encoded_identity,
                                        }),
                                        metadata: v.metadata.map(|v| Metadata {
                                            native_metadata: Default::default(),
                                            data_object_size: v.data_object_size,
                                            last_modified_timestamp: v.last_modified_timestamp,
                                        }),
                                    }),
                                    sorting_key: v.sorting_key,
                                    resource_address: None,
                                })
                                .collect(),
                            versions_order: response.versions_order,
                        };
                        let _ = tx.send(Ok(v1alpha_response)).await;
                    },
                    Ok(None) => break,
                    Err(e) => {
                        let _ = tx.send(Err(e)).await;
                        break;
                    },
                }
            }
        });
        Ok(ReceiverStream::new(rx))
    }
}
