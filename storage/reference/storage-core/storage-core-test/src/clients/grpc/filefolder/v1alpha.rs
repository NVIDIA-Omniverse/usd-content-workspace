// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use storage_core::traits::filefolder::v1alpha::{
    file_folder_service_client::FileFolderServiceClient, CreateFolderRequest, CreateFolderResponse,
    DeleteFolderRequest, DeleteFolderResponse, GetFolderModeRequest, GetFolderModeResponse, ListRequest, ListResponse,
    ListStatRequest, ListStatResponse,
};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::Channel;
use tonic::Status;

use super::super::{connect_with_retries, request_with_headers_config};
use crate::clients::traits::filefolder::FileFolder;
use crate::headers::HeadersConfig;

pub struct FileFolderClient {
    client: FileFolderServiceClient<Channel>,
    headers_config: HeadersConfig,
}

impl FileFolderClient {
    pub async fn new(address: impl AsRef<str>, headers_config: HeadersConfig) -> Self {
        let client = connect_with_retries(|| FileFolderServiceClient::connect(address.as_ref().to_owned())).await;
        Self { client, headers_config }
    }
}

#[tonic::async_trait]
impl FileFolder for FileFolderClient {
    async fn list(&mut self, request: ListRequest) -> Result<ReceiverStream<Result<ListResponse, Status>>, Status> {
        let (tx, rx) = mpsc::channel(1);
        let mut stream = self
            .client
            .list(request_with_headers_config(request, &self.headers_config))
            .await?
            .into_inner();
        let (tx1, mut rx1) = mpsc::channel::<Result<(), Status>>(1);
        tokio::spawn(async move {
            loop {
                let response = stream.message().await;
                match response {
                    Ok(Some(response)) => {
                        let _ = tx.send(Ok(response)).await;
                    },
                    Ok(None) => {
                        let _ = tx1.send(Ok(())).await;
                        break;
                    },
                    Err(e) => {
                        let _ = tx1.send(Err(e)).await;
                        break;
                    },
                }
                let _ = tx1.send(Ok(())).await;
            }
        });
        rx1.recv().await.unwrap()?;
        Ok(ReceiverStream::new(rx))
    }

    async fn list_stat(
        &mut self,
        request: ListStatRequest,
    ) -> Result<ReceiverStream<Result<ListStatResponse, Status>>, Status> {
        let (tx, rx) = mpsc::channel(1);
        let mut stream = self
            .client
            .list_stat(request_with_headers_config(request, &self.headers_config))
            .await?
            .into_inner();
        let (tx1, mut rx1) = mpsc::channel::<Result<(), Status>>(1);
        tokio::spawn(async move {
            loop {
                let response = stream.message().await;
                match response {
                    Ok(Some(response)) => {
                        let _ = tx.send(Ok(response)).await;
                    },
                    Ok(None) => {
                        let _ = tx1.send(Ok(())).await;
                        break;
                    },
                    Err(e) => {
                        let _ = tx1.send(Err(e)).await;
                        break;
                    },
                }
                let _ = tx1.send(Ok(())).await;
            }
        });
        rx1.recv().await.unwrap()?;
        Ok(ReceiverStream::new(rx))
    }

    async fn delete_folder(&mut self, request: DeleteFolderRequest) -> Result<DeleteFolderResponse, Status> {
        self.client
            .delete_folder(request_with_headers_config(request, &self.headers_config))
            .await
            .map(|response| response.into_inner())
    }

    async fn create_folder(&mut self, request: CreateFolderRequest) -> Result<CreateFolderResponse, Status> {
        self.client
            .create_folder(request_with_headers_config(request, &self.headers_config))
            .await
            .map(|response| response.into_inner())
    }

    async fn get_folder_mode(&mut self, request: GetFolderModeRequest) -> Result<GetFolderModeResponse, Status> {
        self.client
            .get_folder_mode(request_with_headers_config(request, &self.headers_config))
            .await
            .map(|response| response.into_inner())
    }
}
