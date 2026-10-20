// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use storage_core::traits::{
    filefolder::v1alpha::{
        DeleteFolderRequest, DeleteFolderResponse, FolderAddress, ListItem, ListRequest, ListResponse, ListStatRequest,
        ListStatResponse,
    },
    fileobject::v1alpha::ResourceInfo,
};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::Status;
use tracing::debug;
use urlencoding::encode;

use super::FileFolderRestClient;
use crate::clients::traits::filefolder::FileFolder;
use crate::headers::HeadersConfig;

pub struct FileFolderClient {
    client: FileFolderRestClient,
}

impl FileFolderClient {
    pub fn new(port: u16, headers_config: HeadersConfig) -> Self {
        Self {
            client: FileFolderRestClient::new(port, headers_config),
        }
    }
}

#[tonic::async_trait]
impl FileFolder for FileFolderClient {
    async fn list(&mut self, request: ListRequest) -> Result<ReceiverStream<Result<ListResponse, Status>>, Status> {
        let path = format!("v1beta/filefolder/list/{}", encode(&request.folder.unwrap().uri));

        let (tx, rx) = mpsc::channel::<Result<ListResponse, Status>>(1);
        // This is a for error handling
        let (tx1, mut rx1) = mpsc::channel::<Result<(), Status>>(1);
        let mut client = self.client.clone();
        tokio::spawn(async move {
            let mut continuation_handle: Option<String> = None;
            loop {
                let body = match client.list(&path, continuation_handle.as_deref()).await {
                    Ok(body) => body,
                    Err(e) => {
                        let _ = tx1.send(Err(e)).await;
                        return;
                    },
                };

                debug!(?path, ?continuation_handle);

                let _ = tx1.send(Ok(())).await;

                // Parse v1beta response and convert to v1alpha format
                let v1beta_result: storage_core_builder::api_services::filefolder::v1beta::rest::ListResponse =
                    serde_json::from_slice(&body).unwrap();
                let _ = tx
                    .send(Ok(ListResponse {
                        subfolder_addresses: v1beta_result
                            .subfolder_addresses
                            .into_iter()
                            .map(|f| FolderAddress { uri: f })
                            .collect(),
                        sub_resource_addresses: v1beta_result.sub_resource_addresses,
                    }))
                    .await;
                if let Some(next_continuation_handle) = v1beta_result.next_continuation_handle {
                    continuation_handle = Some(next_continuation_handle);
                } else {
                    break;
                }
            }
        });
        rx1.recv().await.unwrap()?;

        Ok(ReceiverStream::new(rx))
    }

    async fn list_stat(
        &mut self,
        request: ListStatRequest,
    ) -> Result<ReceiverStream<Result<ListStatResponse, Status>>, Status> {
        let path = format!("v1beta/filefolder/liststat/{}", encode(&request.folder.unwrap().uri));

        let (tx, rx) = mpsc::channel(1);
        let (tx1, mut rx1) = mpsc::channel(1);
        let mut client = self.client.clone();
        tokio::spawn(async move {
            let mut continuation_handle: Option<String> = None;
            loop {
                let body = match client.list_stat(&path, continuation_handle.as_deref()).await {
                    Ok(body) => body,
                    Err(e) => {
                        let _ = tx1.send(Err(e)).await;
                        return;
                    },
                };

                debug!(?path, ?continuation_handle);

                let _ = tx1.send(Ok(())).await;
                // Parse v1beta response and convert to v1alpha format
                let v1beta_result: storage_core_builder::api_services::filefolder::v1beta::rest::ListStatResponse =
                    serde_json::from_slice(&body).unwrap();
                let _ = tx
                    .send(Ok(ListStatResponse {
                        subfolder_addresses: v1beta_result
                            .subfolder_addresses
                            .into_iter()
                            .map(|f| FolderAddress { uri: f })
                            .collect(),
                        entries: v1beta_result
                            .entries
                            .into_iter()
                            .map(|e| ListItem {
                                resource_address: e.resource_address,
                                resource_info: Some(ResourceInfo {
                                    resource_identity: e.resource_identity.map(|ri| {
                                        storage_core::traits::fileobject::v1alpha::ResourceIdentity {
                                            encoded_identity: ri.encoded_identity,
                                        }
                                    }),
                                    metadata: e.metadata.map(|m| storage_core::traits::fileobject::v1alpha::Metadata {
                                        native_metadata: Default::default(),
                                        data_object_size: m.data_object_size,
                                        last_modified_timestamp: m.last_modified_timestamp,
                                    }),
                                }),
                            })
                            .collect(),
                    }))
                    .await;
                if let Some(next_continuation_handle) = v1beta_result.next_continuation_handle {
                    continuation_handle = Some(next_continuation_handle);
                } else {
                    break;
                }
            }
        });
        rx1.recv().await.unwrap()?;
        Ok(ReceiverStream::new(rx))
    }

    async fn delete_folder(&mut self, request: DeleteFolderRequest) -> Result<DeleteFolderResponse, Status> {
        let path = format!("v1beta/filefolder/list/{}", encode(&request.folder.unwrap().uri));
        let _body = self.client.delete_folder(&path).await?;
        Ok(DeleteFolderResponse {})
    }
}
