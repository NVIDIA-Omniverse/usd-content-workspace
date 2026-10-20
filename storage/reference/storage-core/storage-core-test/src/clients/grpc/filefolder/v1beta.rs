// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use storage_core::traits::{
    filefolder::{
        v1alpha::{
            DeleteFolderRequest, DeleteFolderResponse, FolderAddress, ListItem, ListRequest, ListResponse,
            ListStatRequest, ListStatResponse,
        },
        v1beta,
    },
    fileobject::v1alpha::{Metadata, ResourceIdentity, ResourceInfo},
};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::Channel;
use tonic::Status;

use super::super::{connect_with_retries, request_with_headers_config};
use crate::clients::traits::filefolder::FileFolder;
use crate::headers::HeadersConfig;

pub struct FileFolderClient {
    client: v1beta::file_folder_service_client::FileFolderServiceClient<Channel>,
    headers_config: HeadersConfig,
}

impl FileFolderClient {
    pub async fn new(address: impl AsRef<str>, headers_config: HeadersConfig) -> Self {
        let client = connect_with_retries(|| {
            v1beta::file_folder_service_client::FileFolderServiceClient::connect(address.as_ref().to_owned())
        })
        .await;
        Self { client, headers_config }
    }
}

#[tonic::async_trait]
impl FileFolder for FileFolderClient {
    async fn list(&mut self, request: ListRequest) -> Result<ReceiverStream<Result<ListResponse, Status>>, Status> {
        // Convert v1alpha request to v1beta request
        let v1beta_request = v1beta::ListRequest {
            folder: request.folder.map(|f| v1beta::FolderAddress { uri: f.uri }),
        };

        let (tx, rx) = mpsc::channel(1);
        let mut stream = self
            .client
            .list(request_with_headers_config(v1beta_request, &self.headers_config))
            .await?
            .into_inner();
        let (tx1, mut rx1) = mpsc::channel::<Result<(), Status>>(1);
        tokio::spawn(async move {
            loop {
                let response = stream.message().await;
                match response {
                    Ok(Some(response)) => {
                        // Convert v1beta response back to v1alpha response
                        let v1alpha_response = ListResponse {
                            subfolder_addresses: response
                                .subfolder_addresses
                                .into_iter()
                                .map(|f| FolderAddress { uri: f.uri })
                                .collect(),
                            sub_resource_addresses: response.sub_resource_addresses,
                        };
                        let _ = tx.send(Ok(v1alpha_response)).await;
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
        // Convert v1alpha request to v1beta request
        let v1beta_request = v1beta::ListStatRequest {
            folder: request.folder.map(|f| v1beta::FolderAddress { uri: f.uri }),
        };

        let (tx, rx) = mpsc::channel(1);
        let mut stream = self
            .client
            .list_stat(request_with_headers_config(v1beta_request, &self.headers_config))
            .await?
            .into_inner();
        let (tx1, mut rx1) = mpsc::channel::<Result<(), Status>>(1);
        tokio::spawn(async move {
            loop {
                let response = stream.message().await;
                match response {
                    Ok(Some(response)) => {
                        // Convert v1beta response back to v1alpha response
                        let v1alpha_response = ListStatResponse {
                            subfolder_addresses: response
                                .subfolder_addresses
                                .into_iter()
                                .map(|f| FolderAddress { uri: f.uri })
                                .collect(),
                            entries: response
                                .entries
                                .into_iter()
                                .map(|f| ListItem {
                                    resource_address: f.resource_address,
                                    resource_info: f.resource_info.map(|f| ResourceInfo {
                                        resource_identity: f.resource_identity.map(|f| ResourceIdentity {
                                            encoded_identity: f.encoded_identity,
                                        }),
                                        metadata: f.metadata.map(|m| Metadata {
                                            native_metadata: Default::default(),
                                            data_object_size: m.data_object_size,
                                            last_modified_timestamp: m.last_modified_timestamp,
                                        }),
                                    }),
                                })
                                .collect(),
                        };
                        let _ = tx.send(Ok(v1alpha_response)).await;
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
        // Convert v1alpha request to v1beta request
        let v1beta_request = v1beta::DeleteFolderRequest {
            folder: request.folder.map(|f| v1beta::FolderAddress { uri: f.uri }),
        };

        self.client
            .delete_folder(request_with_headers_config(v1beta_request, &self.headers_config))
            .await
            .map(|response| response.into_inner())?;

        Ok(DeleteFolderResponse {})
    }
}
