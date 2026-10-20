// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use bytes::{Bytes, BytesMut};
use futures::StreamExt;
use std::collections::HashSet;
use storage_core_util::cache::CacheControl;
use storage_core_util::{debug_limited, trace_limited, warn_limited};
use tracing::instrument;

use storage_core_builder::traits::{
    async_trait,
    error::Result,
    fileobject::v1alpha::{
        read_from_address_response, read_response, write_response, AbortMultipartUploadRequest,
        AbortMultipartUploadResponse, Chunk, CompleteMultipartUploadRequest, CompleteMultipartUploadResponse,
        CompleteRedirectUploadRequest, CompleteRedirectUploadResponse, CopyRequest, CopyResponse, DeleteRequest,
        DeleteResponse, DownloadPreference, EnumerateRequest, EnumerateResponse, EnumerateResponseWithContinueToken,
        FetchWriteTypeInfoRequest, FetchWriteTypeInfoResponse, FileObject, IsReadOnlyRequest, IsReadOnlyResponse,
        Metadata, MoveRequest, MoveResponse, ReadFromAddressRequest, ReadFromAddressResponse, ReadRequest,
        ReadResponse, ResourceIdentity, ResourceInfo, StatRequest, StatResponse, UploadPartRequest, UploadPartResponse,
        WriteParameters, WriteResponse,
    },
    BoxFuture, BoxStream, Extensions, MetadataMap, Pagination, Request, StreamingOptions,
};

use super::super::{
    error::Error,
    format::{ManifestFile, WorkspaceKey},
    AddressClass, ManifestService, ManifestState,
};
use super::{WorkspaceEnumerateToken, WorkspaceMutationAction};
use crate::traits::fileobject::v1alpha::AddressInfo;

#[async_trait]
impl<S, R> FileObject for ManifestService<S, R>
where
    S: FileObject,
    R: FileObject,
{
    fn inner(&self) -> Option<&dyn FileObject> {
        Some(&self.inner)
    }

    #[instrument(name = "manifest::stat", skip_all)]
    async fn stat(&self, request: Request<StatRequest>) -> Result<StatResponse> {
        self.stat_impl(
            &request.get_ref().resource_address.clone(),
            request,
            |key, metadata, extensions| self.read_manifest_v1alpha(key, metadata, extensions),
            |request| async move { self.inner.stat(request).await },
        )
        .await
    }

    #[instrument(name = "manifest::read", skip_all)]
    async fn read(&self, request: Request<ReadRequest>) -> Result<BoxStream<ReadResponse>> {
        self.inner.read(request).await
    }

    #[instrument(name = "manifest::is_read_only", skip_all)]
    async fn is_read_only(&self, request: Request<IsReadOnlyRequest>) -> Result<IsReadOnlyResponse> {
        // A manifest entry is served from the immutable loaded manifest and the
        // mutation path rejects write/delete/move of such a path with
        // MethodNotAllowed (see `mutation_action_for_path`). Report it read-only
        // so a client learns that before attempting a write. For any other
        // address the manifest layer imposes no read-only status; defer to the
        // physical backend. An invalid manifest fails closed, matching the
        // mutation path.
        let resource_address = request.get_ref().resource_address.clone();
        let key = match self.classify_address(&resource_address)? {
            AddressClass::NotWorkspace | AddressClass::ManifestFile { .. } => {
                return self.inner.is_read_only(request).await
            },
            AddressClass::WorkspacePath { key } => key,
        };

        let metadata = request.metadata().clone();
        let extensions = request.extensions().clone();
        let state = self
            .get_manifest_state(&key, |key| {
                self.read_manifest_v1alpha(key, metadata.clone(), extensions.clone())
            })
            .await?;

        let manifest = match state {
            ManifestState::Loaded(manifest) => manifest,
            ManifestState::Absent => return self.inner.is_read_only(request).await,
            ManifestState::Invalid => return Err(ManifestService::<S>::invalid_manifest_error().into()),
        };

        match key.object_path.as_ref() {
            Some(object_path) if manifest.contains_key(object_path) => Ok(IsReadOnlyResponse { is_read_only: true }),
            _ => self.inner.is_read_only(request).await,
        }
    }

    #[instrument(name = "manifest::read_from_address", skip_all)]
    async fn read_from_address(
        &self,
        request: Request<ReadFromAddressRequest>,
    ) -> Result<BoxStream<ReadFromAddressResponse>> {
        let request_metadata = request.metadata().clone();
        let request_extensions = request.extensions().clone();
        let download_preference = request.get_ref().download_preference;
        let resource_address = request.get_ref().resource_address.clone();

        let key = match self.classify_address(&resource_address)? {
            AddressClass::NotWorkspace => return self.inner.read_from_address(request).await,
            AddressClass::ManifestFile { .. } => {
                // Manifest layer owns the manifest's cache — don't let the caches below double-cache
                // the reserved object (§11.1).
                let mut request = request;
                request.extensions_mut().insert(CacheControl::NoCache);
                return self.inner.read_from_address(request).await;
            },
            AddressClass::WorkspacePath { key } => key,
        };

        let object_path = key.object_path.clone();
        let state = self
            .get_manifest_state(&key, |key| {
                self.read_manifest_v1alpha(key, request_metadata.clone(), request_extensions.clone())
            })
            .await?;

        let entry_value = match state {
            ManifestState::Loaded(manifest) => {
                let Some(object_path) = object_path else {
                    return self.inner.read_from_address(request).await;
                };
                let Some(entry_value) = manifest.get(&object_path) else {
                    return self.inner.read_from_address(request).await;
                };
                entry_value.clone()
            },
            ManifestState::Absent => return self.inner.read_from_address(request).await,
            ManifestState::Invalid => return Err(ManifestService::<S>::invalid_manifest_error().into()),
        };

        let resource_id = entry_value.canonical_resource_identity()?;
        let identity_request = tonic::Request::from_parts(
            request_metadata,
            request_extensions,
            ReadRequest {
                resource_identity: Some(ResourceIdentity {
                    encoded_identity: resource_id.clone(),
                }),
                download_preference,
            },
        )
        .into();
        let stream = self.resolver.read(identity_request).await?;

        Ok(Box::pin(stream.map(move |result| {
            result.map(|response| match response.reply_type {
                Some(read_response::ReplyType::Chunk(chunk)) => ReadFromAddressResponse {
                    reply_type: Some(read_from_address_response::ReplyType::Chunk(Chunk {
                        chunk: chunk.chunk,
                    })),
                },
                Some(read_response::ReplyType::Redirect(redirect)) => ReadFromAddressResponse {
                    reply_type: Some(read_from_address_response::ReplyType::Redirect(redirect)),
                },
                Some(read_response::ReplyType::Metadata(metadata)) => ReadFromAddressResponse {
                    reply_type: Some(read_from_address_response::ReplyType::ResourceInfo(ResourceInfo {
                        resource_identity: Some(ResourceIdentity {
                            encoded_identity: resource_id.clone(),
                        }),
                        metadata: Some(Metadata {
                            native_metadata: Default::default(),
                            data_object_size: metadata.data_object_size,
                            last_modified_timestamp: metadata.last_modified_timestamp,
                        }),
                    })),
                },
                None => ReadFromAddressResponse { reply_type: None },
            })
        })))
    }

    #[instrument(name = "manifest::enumerate", skip_all)]
    async fn enumerate(
        &self,
        request: Request<EnumerateRequest>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<EnumerateResponseWithContinueToken>> {
        let resource_address = request.get_ref().resource_address.clone();
        let key = match self.classify_address(&resource_address)? {
            AddressClass::NotWorkspace | AddressClass::ManifestFile { .. } => {
                return self.inner.enumerate(request, streaming).await
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
            ManifestState::Absent => return self.inner.enumerate(request, streaming).await,
            ManifestState::Invalid => return Err(ManifestService::<S>::invalid_manifest_error().into()),
        };

        let prefix = key.object_path.as_deref().unwrap_or("");
        let mut manifest_items: Vec<_> = manifest
            .iter()
            .filter(|(relative_path, _)| {
                if prefix.is_empty() {
                    true
                } else {
                    relative_path.starts_with(prefix)
                }
            })
            .map(|(relative_path, entry_value)| {
                // Convert manifest entry to v1alpha AddressInfo
                Ok((relative_path.clone(), AddressInfo::try_from(entry_value)?))
            })
            .collect::<Result<Vec<_>>>()?;
        manifest_items.sort_by(|a, b| a.0.cmp(&b.0));
        let shadow_paths: HashSet<String> = manifest_items
            .iter()
            .map(|(relative_path, _)| super::normalize_workspace_path(relative_path))
            .collect();

        self.enumerate_merged_v1alpha(key.workspace_path, request, streaming, manifest_items, shadow_paths)
            .await
    }

    #[instrument(name = "manifest::write", skip_all)]
    async fn write(
        &self,
        request: Request<WriteParameters>,
        data_future: BoxFuture<Bytes>,
    ) -> Result<BoxStream<WriteResponse>> {
        let destination = request.get_ref().destination_resource_address.clone();
        let action = self
            .mutation_action_for_path(
                &destination,
                request.metadata().clone(),
                request.extensions().clone(),
                |key, metadata, extensions| self.read_manifest_v1alpha(key, metadata, extensions),
            )
            .await?;

        match action {
            WorkspaceMutationAction::Delegate => self.inner.write(request, data_future).await,
            WorkspaceMutationAction::ManifestFile { workspace_path } => {
                let stream = self.inner.write(request, data_future).await?;
                Ok(
                    self.invalidate_manifest_on_stream_item(workspace_path, stream, |response: &WriteResponse| {
                        matches!(
                            response.write_response_type,
                            Some(write_response::WriteResponseType::ResourceInfo(_))
                        )
                    }),
                )
            },
        }
    }

    #[instrument(name = "manifest::delete", skip_all)]
    async fn delete(&self, request: Request<DeleteRequest>) -> Result<DeleteResponse> {
        let resource_address = request.get_ref().resource_address.clone();
        let action = self
            .mutation_action_for_path(
                &resource_address,
                request.metadata().clone(),
                request.extensions().clone(),
                |key, metadata, extensions| self.read_manifest_v1alpha(key, metadata, extensions),
            )
            .await?;
        let workspace_path = match action {
            WorkspaceMutationAction::Delegate => None,
            WorkspaceMutationAction::ManifestFile { workspace_path } => Some(workspace_path),
        };

        let response = self.inner.delete(request).await?;
        if let Some(workspace_path) = workspace_path {
            self.invalidate_manifest_state(&workspace_path).await;
        }
        Ok(response)
    }

    #[instrument(name = "manifest::abort_multipart_upload", skip_all)]
    async fn abort_multipart_upload(
        &self,
        request: Request<AbortMultipartUploadRequest>,
    ) -> Result<AbortMultipartUploadResponse> {
        let destination = request.get_ref().destination_resource_address.clone();
        self.mutation_action_for_path(
            &destination,
            request.metadata().clone(),
            request.extensions().clone(),
            |key, metadata, extensions| self.read_manifest_v1alpha(key, metadata, extensions),
        )
        .await?;
        self.inner.abort_multipart_upload(request).await
    }

    #[instrument(name = "manifest::complete_multipart_upload", skip_all)]
    async fn complete_multipart_upload(
        &self,
        request: Request<CompleteMultipartUploadRequest>,
    ) -> Result<CompleteMultipartUploadResponse> {
        let destination = request.get_ref().destination_resource_address.clone();
        let action = self
            .mutation_action_for_path(
                &destination,
                request.metadata().clone(),
                request.extensions().clone(),
                |key, metadata, extensions| self.read_manifest_v1alpha(key, metadata, extensions),
            )
            .await?;
        let workspace_path = match action {
            WorkspaceMutationAction::Delegate => None,
            WorkspaceMutationAction::ManifestFile { workspace_path } => Some(workspace_path),
        };

        let response = self.inner.complete_multipart_upload(request).await?;
        if let Some(workspace_path) = workspace_path {
            self.invalidate_manifest_state(&workspace_path).await;
        }
        Ok(response)
    }

    #[instrument(name = "manifest::upload_part", skip_all)]
    async fn upload_part(&self, request: Request<UploadPartRequest>) -> Result<UploadPartResponse> {
        let destination = request.get_ref().destination_resource_address.clone();
        self.mutation_action_for_path(
            &destination,
            request.metadata().clone(),
            request.extensions().clone(),
            |key, metadata, extensions| self.read_manifest_v1alpha(key, metadata, extensions),
        )
        .await?;
        self.inner.upload_part(request).await
    }

    #[instrument(name = "manifest::complete_redirect_upload", skip_all)]
    async fn complete_redirect_upload(
        &self,
        request: Request<CompleteRedirectUploadRequest>,
    ) -> Result<CompleteRedirectUploadResponse> {
        let destination = request.get_ref().destination_resource_address.clone();
        let action = self
            .mutation_action_for_path(
                &destination,
                request.metadata().clone(),
                request.extensions().clone(),
                |key, metadata, extensions| self.read_manifest_v1alpha(key, metadata, extensions),
            )
            .await?;
        let workspace_path = match action {
            WorkspaceMutationAction::Delegate => None,
            WorkspaceMutationAction::ManifestFile { workspace_path } => Some(workspace_path),
        };

        let response = self.inner.complete_redirect_upload(request).await?;
        if let Some(workspace_path) = workspace_path {
            self.invalidate_manifest_state(&workspace_path).await;
        }
        Ok(response)
    }

    #[instrument(name = "manifest::fetch_write_type_info", skip_all)]
    async fn fetch_write_type_info(
        &self,
        request: Request<FetchWriteTypeInfoRequest>,
    ) -> Result<FetchWriteTypeInfoResponse> {
        let destination = request.get_ref().destination_resource_address.clone();
        self.mutation_action_for_path(
            &destination,
            request.metadata().clone(),
            request.extensions().clone(),
            |key, metadata, extensions| self.read_manifest_v1alpha(key, metadata, extensions),
        )
        .await?;
        self.inner.fetch_write_type_info(request).await
    }

    #[instrument(name = "manifest::copy", skip_all)]
    async fn copy(&self, request: Request<CopyRequest>) -> Result<CopyResponse> {
        let destination = request.get_ref().destination_resource_address.clone();
        let action = self
            .mutation_action_for_path(
                &destination,
                request.metadata().clone(),
                request.extensions().clone(),
                |key, metadata, extensions| self.read_manifest_v1alpha(key, metadata, extensions),
            )
            .await?;
        let workspace_path = match action {
            WorkspaceMutationAction::Delegate => None,
            WorkspaceMutationAction::ManifestFile { workspace_path } => Some(workspace_path),
        };

        let response = self.inner.copy(request).await?;
        if let Some(workspace_path) = workspace_path {
            self.invalidate_manifest_state(&workspace_path).await;
        }
        Ok(response)
    }

    #[instrument(name = "manifest::move_object", skip_all)]
    async fn move_object(&self, request: Request<MoveRequest>) -> Result<MoveResponse> {
        let source = request.get_ref().source_resource_address.clone();
        let destination = request.get_ref().destination_resource_address.clone();
        let source_action = self
            .mutation_action_for_path(
                &source,
                request.metadata().clone(),
                request.extensions().clone(),
                |key, metadata, extensions| self.read_manifest_v1alpha(key, metadata, extensions),
            )
            .await?;
        let destination_action = self
            .mutation_action_for_path(
                &destination,
                request.metadata().clone(),
                request.extensions().clone(),
                |key, metadata, extensions| self.read_manifest_v1alpha(key, metadata, extensions),
            )
            .await?;

        let mut workspace_paths = Vec::new();
        if let WorkspaceMutationAction::ManifestFile { workspace_path } = source_action {
            workspace_paths.push(workspace_path);
        }
        if let WorkspaceMutationAction::ManifestFile { workspace_path } = destination_action {
            workspace_paths.push(workspace_path);
        }

        let response = self.inner.move_object(request).await?;
        workspace_paths.sort();
        workspace_paths.dedup();
        for workspace_path in workspace_paths {
            self.invalidate_manifest_state(&workspace_path).await;
        }
        Ok(response)
    }
}

impl<S, R> ManifestService<S, R>
where
    S: storage_core_builder::traits::fileobject::v1alpha::FileObject,
{
    async fn enumerate_merged_v1alpha(
        &self,
        workspace_path: String,
        request: Request<EnumerateRequest>,
        streaming: StreamingOptions,
        manifest_items: Vec<(String, AddressInfo)>,
        shadow_paths: HashSet<String>,
    ) -> Result<BoxStream<EnumerateResponseWithContinueToken>> {
        let manifest_count = manifest_items.len();
        match streaming {
            StreamingOptions::Streamed => {
                let mut backend_stream = self.inner.enumerate(request, StreamingOptions::Streamed).await?;
                let mut manifest_stream = manifest_enumerate_stream(self, manifest_items, StreamingOptions::Streamed);

                Ok(Box::pin(async_stream::stream! {
                    while let Some(response) = backend_stream.next().await {
                        match response {
                            Ok(response) => {
                                yield Ok(filter_enumerate_response(&workspace_path, &shadow_paths, response));
                            },
                            Err(e) if manifest_count > 0 && super::is_not_found_error(&e) => break,
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
                match WorkspaceEnumerateToken::from_continuation_handle(pagination.continuation_handle.clone())? {
                    WorkspaceEnumerateToken::Backend { cursor } => {
                        let backend_stream = self
                            .inner
                            .enumerate(
                                request,
                                StreamingOptions::Paginated(Pagination {
                                    continuation_handle: cursor,
                                    max_page_size: pagination.max_page_size,
                                }),
                            )
                            .await?;
                        let mut manifest_stream = wrap_manifest_enumerate_tokens(manifest_enumerate_stream(
                            self,
                            manifest_items,
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
                                        let mut response = filter_enumerate_response(&workspace_path, &shadow_paths, response);
                                        response.continue_token = match response.continue_token.take() {
                                            Some(cursor) => WorkspaceEnumerateToken::backend_continue(Some(cursor)),
                                            None if manifest_count > 0 => Some(WorkspaceEnumerateToken::manifest_continue(0)),
                                            None => None,
                                        };
                                        yield Ok(response);
                                    },
                                    Err(e) if manifest_count > 0 && super::is_not_found_error(&e) => {
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

                            if !yielded_backend_page && manifest_count > 0 {
                                while let Some(response) = manifest_stream.next().await {
                                    yield response;
                                }
                            }
                        }))
                    },
                    WorkspaceEnumerateToken::Manifest { index } => {
                        Ok(wrap_manifest_enumerate_tokens(manifest_enumerate_stream(
                            self,
                            manifest_items,
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

impl<S, R> ManifestService<S, R>
where
    R: storage_core_builder::traits::fileobject::v1alpha::FileObject,
{
    pub async fn read_manifest_v1alpha(
        &self,
        manifest_key: WorkspaceKey,
        metadata: MetadataMap,
        extensions: Extensions,
    ) -> Result<ManifestState> {
        debug_limited!("Reading manifest: {}", manifest_key.manifest_path);

        // The manifest layer is the authoritative cache for the parsed manifest; bypass the caches
        // below for this body read so they can't serve a stale `.manifest` that would defeat
        // activation (§11.1).
        let mut extensions = extensions;
        extensions.insert(CacheControl::NoCache);
        let read_request = tonic::Request::from_parts(
            metadata,
            extensions,
            ReadFromAddressRequest {
                resource_address: manifest_key.manifest_path.clone(),
                download_preference: Some(DownloadPreference::Body.into()),
            },
        )
        .into();
        let mut stream = match self.resolver.read_from_address(read_request).await {
            Ok(stream) => stream,
            Err(e) if e.tonic_status().code() == tonic::Code::NotFound => return Ok(ManifestState::Absent),
            Err(e) => return Err(e),
        };

        // The first message should be a ResourceInfo
        let _ = match stream.next().await {
            Some(Ok(response)) => response,
            Some(Err(e)) if e.tonic_status().code() == tonic::Code::NotFound => return Ok(ManifestState::Absent),
            Some(Err(e)) => {
                return Err(e);
            },
            None => {
                return Err(Error::internal("No response from fileobject read_from_address").into());
            },
        };

        // Gather the chunks into a single BytesMut
        let mut manifest = BytesMut::new();
        while let Some(response) = stream.next().await {
            let response = response?;
            match response.reply_type {
                Some(read_from_address_response::ReplyType::Chunk(chunk)) => {
                    manifest.extend_from_slice(&chunk.chunk);
                },

                _ => {
                    return Err(Error::internal("Unexpected response from fileobject read_from_address").into());
                },
            }
        }
        let manifest = manifest.freeze();

        // Parse the JSON blob as a ManifestFile object
        let manifest_file: ManifestFile = match serde_json::from_slice(&manifest) {
            Ok(manifest_file) => manifest_file,
            Err(e) => {
                warn_limited!(
                    workspace_path = %manifest_key.workspace_path,
                    error = %e,
                    "Invalid workspace manifest JSON"
                );
                return Ok(ManifestState::Invalid);
            },
        };

        trace_limited!("Parsed manifest with {} files", manifest_file.files.len());

        let manifest = match manifest_file.into_manifest() {
            Ok(manifest) => manifest,
            Err(e) => {
                warn_limited!(
                    workspace_path = %manifest_key.workspace_path,
                    error = %e,
                    "Invalid workspace manifest"
                );
                return Ok(ManifestState::Invalid);
            },
        };

        Ok(ManifestState::Loaded(manifest))
    }
}

fn manifest_enumerate_stream<S, R>(
    service: &ManifestService<S, R>,
    manifest_items: Vec<(String, AddressInfo)>,
    streaming: StreamingOptions,
) -> BoxStream<EnumerateResponseWithContinueToken> {
    let items = manifest_items.into_iter().map(|(_relative_path, item)| item).collect();

    service.stream_enumerate_response(items, streaming, |items, continue_token| {
        EnumerateResponseWithContinueToken {
            response: EnumerateResponse { items },
            continue_token,
        }
    })
}

fn filter_enumerate_response(
    workspace_path: &str,
    shadow_paths: &HashSet<String>,
    mut response: EnumerateResponseWithContinueToken,
) -> EnumerateResponseWithContinueToken {
    response.response.items.retain(|item| {
        ManifestService::<()>::workspace_relative_path(workspace_path, &item.resource_address)
            .is_none_or(|relative_path| !shadow_paths.contains(&relative_path))
    });
    response
}

fn wrap_manifest_enumerate_tokens(
    stream: BoxStream<EnumerateResponseWithContinueToken>,
) -> BoxStream<EnumerateResponseWithContinueToken> {
    Box::pin(stream.map(|response| {
        response.map(|mut response| {
            response.continue_token = response
                .continue_token
                .and_then(|token| token.parse::<usize>().ok())
                .map(WorkspaceEnumerateToken::manifest_continue);
            response
        })
    }))
}

impl TryFrom<&super::super::format::ManifestEntryValue> for StatResponse {
    type Error = Error;

    fn try_from(value: &super::super::format::ManifestEntryValue) -> std::result::Result<Self, Self::Error> {
        Ok(StatResponse {
            resource_info: Some(ResourceInfo::try_from(value)?),
        })
    }
}
