// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use storage_core::traits::versioning::v1alpha::{
    versioning_service_client::VersioningServiceClient, EnumerateVersionsRequest, EnumerateVersionsResponse,
    IsVersionedRequest, IsVersionedResponse,
};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::Channel;
use tonic::Status;

use super::super::{connect_with_retries, request_with_headers_config};
use crate::clients::traits::versioning::Versioning;
use crate::headers::HeadersConfig;

pub struct VersioningClient {
    client: VersioningServiceClient<Channel>,
    headers_config: HeadersConfig,
}
impl VersioningClient {
    pub async fn new(address: impl AsRef<str>, headers_config: HeadersConfig) -> Self {
        let client = connect_with_retries(|| VersioningServiceClient::connect(address.as_ref().to_owned())).await;
        Self { client, headers_config }
    }
}

#[tonic::async_trait]
impl Versioning for VersioningClient {
    async fn enumerate_versions(
        &mut self,
        request: EnumerateVersionsRequest,
    ) -> Result<ReceiverStream<Result<EnumerateVersionsResponse, Status>>, Status> {
        let (tx, rx) = mpsc::channel(1);
        let mut stream = self
            .client
            .enumerate_versions(request_with_headers_config(request, &self.headers_config))
            .await?
            .into_inner();

        tokio::spawn(async move {
            loop {
                let response = stream.message().await;
                match response {
                    Ok(Some(response)) => {
                        let _ = tx.send(Ok(response)).await;
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

    async fn is_versioned(&mut self, request: IsVersionedRequest) -> Result<IsVersionedResponse, Status> {
        self.client
            .is_versioned(request_with_headers_config(request, &self.headers_config))
            .await
            .map(|response| response.into_inner())
    }
}
