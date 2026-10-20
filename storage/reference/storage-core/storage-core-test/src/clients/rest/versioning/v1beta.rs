// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use storage_core::traits::versioning::v1alpha::{EnumerateVersionsRequest, EnumerateVersionsResponse, VersionsOrder};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::Status;
use urlencoding::encode;

use super::VersioningRestClient;
use crate::clients::traits::versioning::Versioning;
use crate::headers::HeadersConfig;

pub struct VersioningClient {
    client: VersioningRestClient,
}

impl VersioningClient {
    pub fn new(port: u16, headers_config: HeadersConfig) -> Self {
        Self {
            client: VersioningRestClient::new(port, headers_config),
        }
    }
}

#[tonic::async_trait]
impl Versioning for VersioningClient {
    async fn enumerate_versions(
        &mut self,
        request: EnumerateVersionsRequest,
    ) -> Result<ReceiverStream<Result<EnumerateVersionsResponse, Status>>, Status> {
        let path = format!("v1beta/versioning/{}/versions", encode(&request.resource_address));

        let (tx, rx) = mpsc::channel(1);
        let mut client = self.client.clone();
        tokio::spawn(async move {
            let mut continuation_handle: Option<String> = None;
            loop {
                let body = match client.enumerate_versions(&path, continuation_handle.as_deref()).await {
                    Ok(body) => body,
                    Err(e) => {
                        let _ = tx.send(Err(e)).await;
                        return;
                    },
                };

                let v1beta_result: storage_core_builder::api_services::versioning::v1beta::rest::EnumerateVersionsResponse =
                    serde_json::from_slice(&body).unwrap();

                // Convert v1beta response to v1alpha format
                let converted_items: Vec<storage_core::traits::versioning::v1alpha::VersionInfo> = v1beta_result
                    .items
                    .into_iter()
                    .map(|item| {
                        let resource_info = item.resource_info.unwrap_or_default();
                        storage_core::traits::versioning::v1alpha::VersionInfo {
                            resource_info: Some(storage_core::traits::fileobject::v1alpha::ResourceInfo {
                                resource_identity: Some(storage_core::traits::fileobject::v1alpha::ResourceIdentity {
                                    encoded_identity: resource_info
                                        .resource_identity
                                        .unwrap_or_default()
                                        .encoded_identity,
                                }),
                                metadata: resource_info.metadata.map(|meta| {
                                    storage_core::traits::fileobject::v1alpha::Metadata {
                                        native_metadata: Default::default(),
                                        data_object_size: meta.data_object_size,
                                        last_modified_timestamp: meta.last_modified_timestamp,
                                    }
                                }),
                            }),
                            sorting_key: item.sorting_key,
                            resource_address: None, // v1beta doesn't have this field
                        }
                    })
                    .collect();

                let _ = tx
                    .send(Ok(EnumerateVersionsResponse {
                        items: converted_items,
                        versions_order: VersionsOrder::from_str_name(&v1beta_result.versions_order)
                            .unwrap_or(VersionsOrder::Unspecified)
                            .into(),
                    }))
                    .await;

                if let Some(next_continuation_handle) = v1beta_result.next_continuation_handle {
                    continuation_handle = Some(next_continuation_handle);
                } else {
                    break;
                }
            }
        });
        Ok(ReceiverStream::new(rx))
    }
}
