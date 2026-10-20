// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use storage_core::traits::versioning::v1alpha::{
    EnumerateVersionsRequest, EnumerateVersionsResponse, IsVersionedRequest, IsVersionedResponse, VersionsOrder,
};
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
        let path = format!("v1alpha/versioning/{}/versions", encode(&request.resource_address));

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

                let result: storage_core_builder::api_services::versioning::v1alpha::rest::EnumerateVersionsResponse =
                    serde_json::from_slice(&body).unwrap();

                // because rest defines the version order strings to be different from grpc
                let versions_order = match result.versions_order.as_str() {
                    "newest_first" => VersionsOrder::NewestFirst,
                    "oldest_first" => VersionsOrder::OldestFirst,
                    "by_key" => VersionsOrder::ByKey,
                    _ => VersionsOrder::Unspecified,
                };

                let _ = tx
                    .send(Ok(EnumerateVersionsResponse {
                        items: result.items,
                        versions_order: versions_order.into(),
                    }))
                    .await;

                if let Some(next_continuation_handle) = result.next_continuation_handle {
                    continuation_handle = Some(next_continuation_handle);
                } else {
                    break;
                }
            }
        });
        Ok(ReceiverStream::new(rx))
    }

    async fn is_versioned(&mut self, request: IsVersionedRequest) -> Result<IsVersionedResponse, Status> {
        let path = format!("v1alpha/versioning/is-versioned/{}", encode(&request.resource_address));
        let body = self.client.is_versioned(&path).await?;
        Ok(serde_json::from_slice(&body).unwrap())
    }
}
