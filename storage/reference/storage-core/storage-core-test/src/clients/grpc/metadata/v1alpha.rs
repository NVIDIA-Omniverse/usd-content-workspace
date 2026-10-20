// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use storage_core::traits::metadata::v1alpha::{
    self, DeleteMetadataRequest, DeleteMetadataResponse, GetMetadataRequest, GetMetadataResponse,
    UpdateMetadataBatchRequest, UpdateMetadataBatchResponse, UpdateMetadataRequest, UpdateMetadataResponse,
};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::Channel;
use tonic::Status;

use super::super::{connect_with_retries, request_with_headers_config};
use crate::clients::traits::metadata::Metadata;
use crate::headers::HeadersConfig;

pub struct MetadataClient {
    client: v1alpha::metadata_service_client::MetadataServiceClient<Channel>,
    headers_config: HeadersConfig,
}

impl MetadataClient {
    pub async fn new(address: impl AsRef<str>, headers_config: HeadersConfig) -> Self {
        let client = connect_with_retries(|| {
            v1alpha::metadata_service_client::MetadataServiceClient::connect(address.as_ref().to_owned())
        })
        .await;
        Self { client, headers_config }
    }
}

#[tonic::async_trait]
impl Metadata for MetadataClient {
    async fn get_metadata(&mut self, request: GetMetadataRequest) -> Result<GetMetadataResponse, Status> {
        self.client
            .get_metadata(request_with_headers_config(request, &self.headers_config))
            .await
            .map(|response| response.into_inner())
    }

    async fn update_metadata(&mut self, request: UpdateMetadataRequest) -> Result<UpdateMetadataResponse, Status> {
        self.client
            .update_metadata(request_with_headers_config(request, &self.headers_config))
            .await
            .map(|response| response.into_inner())
    }

    async fn update_metadata_batch(
        &mut self,
        request: UpdateMetadataBatchRequest,
    ) -> Result<ReceiverStream<Result<UpdateMetadataBatchResponse, Status>>, Status> {
        let (tx, rx) = mpsc::channel(1);
        let mut stream = self
            .client
            .update_metadata_batch(request_with_headers_config(request, &self.headers_config))
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

    async fn delete_metadata(&mut self, request: DeleteMetadataRequest) -> Result<DeleteMetadataResponse, Status> {
        self.client
            .delete_metadata(request_with_headers_config(request, &self.headers_config))
            .await
            .map(|response| response.into_inner())
    }
}
