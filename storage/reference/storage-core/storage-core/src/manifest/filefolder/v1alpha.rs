// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;
use futures::StreamExt;
use get_size2::GetSize;
use std::collections::HashSet;
use tracing::instrument;

use crate::traits::{
    error::Result,
    filefolder::v1alpha::{
        CreateFolderRequest, CreateFolderResponse, DeleteFolderRequest, DeleteFolderResponse, FileFolder,
        FolderAddress, GetFolderModeRequest, GetFolderModeResponse, ListItem, ListRequest, ListResponse,
        ListResponseWithContinueToken, ListStatRequest, ListStatResponse, ListStatResponseWithContinueToken,
    },
    fileobject::v1alpha::FileObject,
    BoxStream, Pagination, Request, StreamingOptions,
};

use super::super::{error::Error, AddressClass, ManifestService, ManifestState};
use super::WorkspaceListToken;
use crate::traits::fileobject::v1alpha::ResourceInfo;

#[async_trait]
impl<S, R> FileFolder for ManifestService<S, R>
where
    S: FileFolder + Send + Sync,
    R: FileObject,
{
    fn inner(&self) -> Option<&dyn FileFolder> {
        Some(&self.inner)
    }

    #[instrument(name = "manifest::list", skip_all)]
    async fn list(
        &self,
        request: Request<ListRequest>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<ListResponseWithContinueToken>> {
        let folder_address = request
            .get_ref()
            .folder
            .as_ref()
            .ok_or_else(|| Error::internal("Folder address is required"))?;

        let uri = folder_address.uri.clone();
        let key = match self.classify_address(&uri)? {
            AddressClass::NotWorkspace | AddressClass::ManifestFile { .. } => {
                return self.inner.list(request, streaming).await
            },
            AddressClass::WorkspacePath { key } => key,
        };

        let state = self
            .get_manifest_state(&key, |key| {
                self.read_manifest_v1alpha(key, request.metadata().clone(), request.extensions().clone())
            })
            .await?;
        let manifest = match state {
            ManifestState::Loaded(manifest) => manifest,
            ManifestState::Absent => return self.inner.list(request, streaming).await,
            ManifestState::Invalid => return Err(ManifestService::<S>::invalid_manifest_error().into()),
        };

        let (list_data, shadow_names) = self.list_projection(&key, manifest);
        self.list_merged_v1alpha(
            key.workspace_path.clone(),
            key.object_path.clone(),
            request,
            streaming,
            list_data,
            shadow_names,
        )
        .await
    }

    #[instrument(name = "manifest::list_stat", skip_all)]
    async fn list_stat(
        &self,
        request: Request<ListStatRequest>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<ListStatResponseWithContinueToken>> {
        let folder_address = request
            .get_ref()
            .folder
            .as_ref()
            .ok_or_else(|| Error::internal("Folder address is required"))?;

        let uri = folder_address.uri.clone();
        let key = match self.classify_address(&uri)? {
            AddressClass::NotWorkspace | AddressClass::ManifestFile { .. } => {
                return self.inner.list_stat(request, streaming).await
            },
            AddressClass::WorkspacePath { key } => key,
        };

        let state = self
            .get_manifest_state(&key, |key| {
                self.read_manifest_v1alpha(key, request.metadata().clone(), request.extensions().clone())
            })
            .await?;
        let manifest = match state {
            ManifestState::Loaded(manifest) => manifest,
            ManifestState::Absent => return self.inner.list_stat(request, streaming).await,
            ManifestState::Invalid => return Err(ManifestService::<S>::invalid_manifest_error().into()),
        };

        let (list_data, shadow_names) = self.list_projection(&key, manifest);
        self.list_stat_merged_v1alpha(
            key.workspace_path.clone(),
            key.object_path.clone(),
            request,
            streaming,
            list_data,
            shadow_names,
        )
        .await
    }

    #[instrument(name = "manifest::delete_folder", skip_all)]
    async fn delete_folder(&self, request: Request<DeleteFolderRequest>) -> Result<DeleteFolderResponse> {
        let folder_address = request
            .get_ref()
            .folder
            .as_ref()
            .ok_or_else(|| Error::invalid_argument("Folder address is required"))?;

        let uri = folder_address.uri.clone();
        let invalidate_workspace_path = match self.classify_address(&uri)? {
            AddressClass::ManifestFile { key } => Some(key.workspace_path.clone()),
            AddressClass::WorkspacePath { key } if key.object_path.is_none() => Some(key.workspace_path.clone()),
            AddressClass::WorkspacePath { .. } | AddressClass::NotWorkspace => None,
        };

        self.mutation_action_for_path(
            &uri,
            request.metadata().clone(),
            request.extensions().clone(),
            |key, metadata, extensions| self.read_manifest_v1alpha(key, metadata, extensions),
        )
        .await?;

        let response = self.inner.delete_folder(request).await?;
        if let Some(workspace_path) = invalidate_workspace_path {
            self.invalidate_manifest_state(&workspace_path).await;
        }
        Ok(response)
    }

    #[instrument(name = "manifest::create_folder", skip_all)]
    async fn create_folder(&self, request: Request<CreateFolderRequest>) -> Result<CreateFolderResponse> {
        let folder_address = request
            .get_ref()
            .folder
            .as_ref()
            .ok_or_else(|| Error::invalid_argument("Folder address is required"))?;

        let uri = folder_address.uri.clone();
        self.mutation_action_for_path(
            &uri,
            request.metadata().clone(),
            request.extensions().clone(),
            |key, metadata, extensions| self.read_manifest_v1alpha(key, metadata, extensions),
        )
        .await?;

        self.inner.create_folder(request).await
    }

    #[instrument(name = "manifest::get_folder_mode", skip_all)]
    async fn get_folder_mode(&self, request: Request<GetFolderModeRequest>) -> Result<GetFolderModeResponse> {
        self.inner.get_folder_mode(request).await
    }
}

impl<S, R> ManifestService<S, R>
where
    S: FileFolder + Send + Sync,
    R: FileObject,
{
    async fn list_merged_v1alpha(
        &self,
        workspace_path: String,
        parent_object_path: Option<String>,
        request: Request<ListRequest>,
        streaming: StreamingOptions,
        list_data: super::ListData,
        shadow_names: HashSet<String>,
    ) -> Result<BoxStream<ListResponseWithContinueToken>> {
        let manifest_items = list_data.len();
        match streaming {
            StreamingOptions::Streamed => {
                let mut backend_stream = self.inner.list(request, StreamingOptions::Streamed).await?;
                let mut manifest_stream = manifest_list_stream(self, list_data, StreamingOptions::Streamed);

                Ok(Box::pin(async_stream::stream! {
                    while let Some(response) = backend_stream.next().await {
                        match response {
                            Ok(response) => {
                                yield Ok(filter_list_response(&workspace_path, parent_object_path.as_deref(), &shadow_names, response));
                            },
                            Err(e) if manifest_items > 0 && super::is_not_found_error(&e) => break,
                            Err(e) => {
                                yield Err(e);
                                return;
                            },
                        }
                    }
                    while let Some(response) = manifest_stream.next().await {
                        yield response;
                    }
                }))
            },
            StreamingOptions::Paginated(pagination) => {
                match WorkspaceListToken::from_continuation_handle(pagination.continuation_handle.clone())? {
                    WorkspaceListToken::Backend { cursor } => {
                        let backend_stream = self
                            .inner
                            .list(
                                request,
                                StreamingOptions::Paginated(Pagination {
                                    continuation_handle: cursor,
                                    max_page_size: pagination.max_page_size,
                                }),
                            )
                            .await?;
                        let mut manifest_stream = wrap_manifest_list_tokens(manifest_list_stream(
                            self,
                            list_data,
                            StreamingOptions::Paginated(Pagination {
                                continuation_handle: Some("0".to_string()),
                                max_page_size: pagination.max_page_size,
                            }),
                        ));
                        Ok(Box::pin(async_stream::stream! {
                            let mut yielded_backend_page = false;
                            let mut backend_stream = backend_stream;
                            while let Some(response) = backend_stream.next().await {
                                match response {
                                    Ok(response) => {
                                        yielded_backend_page = true;
                                        let mut response = filter_list_response(&workspace_path, parent_object_path.as_deref(), &shadow_names, response);
                                        response.continue_token = match response.continue_token.take() {
                                            Some(cursor) => WorkspaceListToken::backend_continue(Some(cursor)),
                                            None if manifest_items > 0 => Some(WorkspaceListToken::manifest_continue(0)),
                                            None => None,
                                        };
                                        yield Ok(response);
                                    },
                                    Err(e) if manifest_items > 0 && super::is_not_found_error(&e) => {
                                        while let Some(response) = manifest_stream.next().await {
                                            yield response;
                                        }
                                        return;
                                    },
                                    Err(e) => {
                                        yield Err(e);
                                        return;
                                    },
                                }
                            }

                            if !yielded_backend_page && manifest_items > 0 {
                                while let Some(response) = manifest_stream.next().await {
                                    yield response;
                                }
                            }
                        }))
                    },
                    WorkspaceListToken::Manifest { index } => Ok(wrap_manifest_list_tokens(manifest_list_stream(
                        self,
                        list_data,
                        StreamingOptions::Paginated(Pagination {
                            continuation_handle: Some(index.to_string()),
                            max_page_size: pagination.max_page_size,
                        }),
                    ))),
                }
            },
        }
    }

    async fn list_stat_merged_v1alpha(
        &self,
        workspace_path: String,
        parent_object_path: Option<String>,
        request: Request<ListStatRequest>,
        streaming: StreamingOptions,
        list_data: super::ListData,
        shadow_names: HashSet<String>,
    ) -> Result<BoxStream<ListStatResponseWithContinueToken>> {
        let manifest_items = list_data.len();
        match streaming {
            StreamingOptions::Streamed => {
                let mut backend_stream = self.inner.list_stat(request, StreamingOptions::Streamed).await?;
                let mut manifest_stream = manifest_list_stat_stream(self, list_data, StreamingOptions::Streamed);

                Ok(Box::pin(async_stream::stream! {
                    while let Some(response) = backend_stream.next().await {
                        match response {
                            Ok(response) => {
                                yield Ok(filter_list_stat_response(&workspace_path, parent_object_path.as_deref(), &shadow_names, response));
                            },
                            Err(e) if manifest_items > 0 && super::is_not_found_error(&e) => break,
                            Err(e) => {
                                yield Err(e);
                                return;
                            },
                        }
                    }
                    while let Some(response) = manifest_stream.next().await {
                        yield response;
                    }
                }))
            },
            StreamingOptions::Paginated(pagination) => {
                match WorkspaceListToken::from_continuation_handle(pagination.continuation_handle.clone())? {
                    WorkspaceListToken::Backend { cursor } => {
                        let backend_stream = self
                            .inner
                            .list_stat(
                                request,
                                StreamingOptions::Paginated(Pagination {
                                    continuation_handle: cursor,
                                    max_page_size: pagination.max_page_size,
                                }),
                            )
                            .await?;
                        let mut manifest_stream = wrap_manifest_list_stat_tokens(manifest_list_stat_stream(
                            self,
                            list_data,
                            StreamingOptions::Paginated(Pagination {
                                continuation_handle: Some("0".to_string()),
                                max_page_size: pagination.max_page_size,
                            }),
                        ));
                        Ok(Box::pin(async_stream::stream! {
                            let mut yielded_backend_page = false;
                            let mut backend_stream = backend_stream;
                            while let Some(response) = backend_stream.next().await {
                                match response {
                                    Ok(response) => {
                                        yielded_backend_page = true;
                                        let mut response = filter_list_stat_response(&workspace_path, parent_object_path.as_deref(), &shadow_names, response);
                                        response.continue_token = match response.continue_token.take() {
                                            Some(cursor) => WorkspaceListToken::backend_continue(Some(cursor)),
                                            None if manifest_items > 0 => Some(WorkspaceListToken::manifest_continue(0)),
                                            None => None,
                                        };
                                        yield Ok(response);
                                    },
                                    Err(e) if manifest_items > 0 && super::is_not_found_error(&e) => {
                                        while let Some(response) = manifest_stream.next().await {
                                            yield response;
                                        }
                                        return;
                                    },
                                    Err(e) => {
                                        yield Err(e);
                                        return;
                                    },
                                }
                            }

                            if !yielded_backend_page && manifest_items > 0 {
                                while let Some(response) = manifest_stream.next().await {
                                    yield response;
                                }
                            }
                        }))
                    },
                    WorkspaceListToken::Manifest { index } => {
                        Ok(wrap_manifest_list_stat_tokens(manifest_list_stat_stream(
                            self,
                            list_data,
                            StreamingOptions::Paginated(Pagination {
                                continuation_handle: Some(index.to_string()),
                                max_page_size: pagination.max_page_size,
                            }),
                        )))
                    },
                }
            },
        }
    }
}

fn manifest_list_stream<S, R>(
    service: &ManifestService<S, R>,
    list_data: super::ListData,
    streaming: StreamingOptions,
) -> BoxStream<ListResponseWithContinueToken> {
    service.stream_list_response(
        list_data,
        streaming,
        |item| {
            let address = item.resource_address.clone();
            let size = address.get_size() as u64;
            (address, size)
        },
        |subfolder| {
            let folder = FolderAddress {
                uri: subfolder.to_string(),
            };
            let size = folder.get_size() as u64;
            (folder, size)
        },
        |subfolder_addresses, sub_resource_addresses, continue_token| ListResponseWithContinueToken {
            response: ListResponse {
                subfolder_addresses,
                sub_resource_addresses,
            },
            continue_token,
        },
    )
}

fn manifest_list_stat_stream<S, R>(
    service: &ManifestService<S, R>,
    list_data: super::ListData,
    streaming: StreamingOptions,
) -> BoxStream<ListStatResponseWithContinueToken> {
    service.stream_list_response(
        list_data,
        streaming,
        |item| {
            let list_item = ListItem {
                resource_address: item.resource_address.clone(),
                resource_info: ResourceInfo::try_from(&item.entry_value).ok(),
            };
            let size = list_item.get_size() as u64;
            (list_item, size)
        },
        |subfolder| {
            let folder = FolderAddress {
                uri: subfolder.to_string(),
            };
            let size = folder.get_size() as u64;
            (folder, size)
        },
        |subfolder_addresses, entries, continue_token| ListStatResponseWithContinueToken {
            response: ListStatResponse {
                subfolder_addresses,
                entries,
            },
            continue_token,
        },
    )
}

fn filter_list_response(
    workspace_path: &str,
    parent_object_path: Option<&str>,
    shadow_names: &HashSet<String>,
    mut response: ListResponseWithContinueToken,
) -> ListResponseWithContinueToken {
    response.response.sub_resource_addresses.retain(|address| {
        ManifestService::<()>::direct_child_name_from_workspace_address(workspace_path, parent_object_path, address)
            .is_none_or(|child| !shadow_names.contains(&child))
    });
    response.response.subfolder_addresses.retain(|folder| {
        ManifestService::<()>::direct_child_name_from_workspace_address(workspace_path, parent_object_path, &folder.uri)
            .is_none_or(|child| !shadow_names.contains(&child))
    });
    response
}

fn filter_list_stat_response(
    workspace_path: &str,
    parent_object_path: Option<&str>,
    shadow_names: &HashSet<String>,
    mut response: ListStatResponseWithContinueToken,
) -> ListStatResponseWithContinueToken {
    response.response.entries.retain(|item| {
        ManifestService::<()>::direct_child_name_from_workspace_address(
            workspace_path,
            parent_object_path,
            &item.resource_address,
        )
        .is_none_or(|child| !shadow_names.contains(&child))
    });
    response.response.subfolder_addresses.retain(|folder| {
        ManifestService::<()>::direct_child_name_from_workspace_address(workspace_path, parent_object_path, &folder.uri)
            .is_none_or(|child| !shadow_names.contains(&child))
    });
    response
}

fn wrap_manifest_list_tokens(
    stream: BoxStream<ListResponseWithContinueToken>,
) -> BoxStream<ListResponseWithContinueToken> {
    Box::pin(stream.map(|response| {
        response.map(|mut response| {
            response.continue_token = response
                .continue_token
                .and_then(|token| token.parse::<usize>().ok())
                .map(WorkspaceListToken::manifest_continue);
            response
        })
    }))
}

fn wrap_manifest_list_stat_tokens(
    stream: BoxStream<ListStatResponseWithContinueToken>,
) -> BoxStream<ListStatResponseWithContinueToken> {
    Box::pin(stream.map(|response| {
        response.map(|mut response| {
            response.continue_token = response
                .continue_token
                .and_then(|token| token.parse::<usize>().ok())
                .map(WorkspaceListToken::manifest_continue);
            response
        })
    }))
}
