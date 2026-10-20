// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use futures::future::BoxFuture;
use storage_core::traits::{async_trait, filefolder::v1alpha::*, BoxStream, Request, Result, StreamingOptions};

use crate::backend::created_timestamp::{
    delete_created_timestamp, folder_address_location, record_created_timestamp, CreatedTimestampService,
};

#[async_trait]
impl<S: FileFolder> FileFolder for CreatedTimestampService<S> {
    fn inner(&self) -> Option<&dyn FileFolder> {
        None
    }

    fn list<'a: 'b, 'b>(
        &'a self,
        request: Request<ListRequest>,
        streaming: StreamingOptions,
    ) -> BoxFuture<'b, Result<BoxStream<ListResponseWithContinueToken>>> {
        self.inner.list(request, streaming)
    }

    fn list_stat<'a: 'b, 'b>(
        &'a self,
        request: Request<ListStatRequest>,
        streaming: StreamingOptions,
    ) -> BoxFuture<'b, Result<BoxStream<ListStatResponseWithContinueToken>>> {
        self.inner.list_stat(request, streaming)
    }

    // Delete the folder, then best-effort remove its `created_timestamp`. Cleanup runs only after
    // the underlying delete succeeds, so a failed delete leaves the metadata untouched; a metadata
    // failure never fails the delete. Only the directly-deleted folder address is cleaned up; the
    // created_timestamps of recursively removed children are left for the metadata backend to
    // reap, since the layer does not have the child addresses without an extra listing.
    async fn delete_folder(&self, request: Request<DeleteFolderRequest>) -> Result<DeleteFolderResponse> {
        let folder_uri = request.get_ref().folder.as_ref().map(|folder| folder.uri.clone());
        let response = self.inner.delete_folder(request).await?;
        if let Some(folder_uri) = folder_uri {
            delete_created_timestamp(&self.metadata, folder_address_location(folder_uri)).await;
        }
        Ok(response)
    }

    // A folder has no native creation time, so record it here — this is the case the client-library
    // could not populate on its own.
    async fn create_folder(&self, request: Request<CreateFolderRequest>) -> Result<CreateFolderResponse> {
        let folder_uri = request.get_ref().folder.as_ref().map(|folder| folder.uri.clone());
        let response = self.inner.create_folder(request).await?;
        if let Some(folder_uri) = folder_uri {
            record_created_timestamp(&self.metadata, folder_address_location(folder_uri)).await;
        }
        Ok(response)
    }

    fn get_folder_mode<'a: 'b, 'b>(
        &'a self,
        request: Request<GetFolderModeRequest>,
    ) -> BoxFuture<'b, Result<GetFolderModeResponse>> {
        self.inner.get_folder_mode(request)
    }
}

// The delete/cleanup wiring is version-agnostic (fileobject.delete and filefolder.delete_folder
// share the same best-effort helper), so the semantics are exercised once here through the v1alpha
// `delete_folder` path. The end-to-end "delete removes the recorded created_timestamp" behavior is
// covered by the `tests/created_timestamp.rs` integration test.
#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use assert_matches::assert_matches;
    use storage_core::traits::UnimplementedError;

    use super::*;
    use crate::backend::created_timestamp::{CreatedTimestampService, CREATED_TIMESTAMP_KEY};
    use crate::backend::metadata::any::AnyMetadata;
    use crate::backend::metadata::resource_location::Resource as MetaResource;
    use crate::backend::metadata::{
        DeleteMetadataBatchRequest, DeleteMetadataBatchResponse, GetMetadataBatchRequest, GetMetadataBatchResponse,
        Metadata, QueryMetadataRequest, QueryMetadataResponsePage, UpdateMetadataBatchRequest,
        UpdateMetadataBatchResponse,
    };

    const FOLDER_URI: &str = "omniverse://host/folder/";

    /// A metadata backend that only supports `delete_metadata_batch`, records how many delete calls
    /// it saw and the `(key, folder uri)` of the last one, and can be told to fail the delete.
    #[derive(Clone)]
    struct RecordingMetadata {
        delete_calls: Arc<AtomicUsize>,
        last_delete: Arc<Mutex<Option<(String, String)>>>,
        fail_delete: bool,
    }

    impl RecordingMetadata {
        fn new(fail_delete: bool) -> Self {
            Self {
                delete_calls: Arc::new(AtomicUsize::new(0)),
                last_delete: Arc::new(Mutex::new(None)),
                fail_delete,
            }
        }
    }

    #[async_trait]
    impl Metadata for RecordingMetadata {
        fn inner(&self) -> Option<&dyn Metadata> {
            None
        }

        async fn get_metadata_batch(
            &self,
            _request: Request<GetMetadataBatchRequest>,
        ) -> Result<GetMetadataBatchResponse> {
            unreachable!("delete cleanup never reads metadata")
        }

        async fn query_metadata(
            &self,
            _request: Request<QueryMetadataRequest>,
            _options: StreamingOptions,
        ) -> Result<BoxStream<QueryMetadataResponsePage>> {
            unreachable!("delete cleanup never queries metadata")
        }

        async fn update_metadata_batch(
            &self,
            _request: Request<UpdateMetadataBatchRequest>,
        ) -> Result<UpdateMetadataBatchResponse> {
            unreachable!("delete cleanup never writes metadata")
        }

        async fn delete_metadata_batch(
            &self,
            request: Request<DeleteMetadataBatchRequest>,
        ) -> Result<DeleteMetadataBatchResponse> {
            self.delete_calls.fetch_add(1, Ordering::SeqCst);
            let entry = request
                .into_inner()
                .requests
                .into_iter()
                .next()
                .expect("one delete entry");
            let uri = match entry.resource_location.and_then(|location| location.resource) {
                Some(MetaResource::FolderAddress(folder)) => folder.uri,
                other => panic!("expected a folder address, got {other:?}"),
            };
            *self.last_delete.lock().unwrap() = Some((entry.user_metadata_key, uri));
            if self.fail_delete {
                return Err(UnimplementedError::new().into());
            }
            // An empty results set is treated as success by the cleanup helper.
            Ok(DeleteMetadataBatchResponse { results: vec![] })
        }
    }

    /// An inner `FileFolder` whose `delete_folder` returns success or a fixed error on demand; every
    /// other method is unreachable in these tests.
    struct DeleteFolderInner {
        fail: bool,
    }

    #[async_trait]
    impl FileFolder for DeleteFolderInner {
        fn inner(&self) -> Option<&dyn FileFolder> {
            None
        }

        async fn list(
            &self,
            _request: Request<ListRequest>,
            _streaming: StreamingOptions,
        ) -> Result<BoxStream<ListResponseWithContinueToken>> {
            unreachable!()
        }

        async fn list_stat(
            &self,
            _request: Request<ListStatRequest>,
            _streaming: StreamingOptions,
        ) -> Result<BoxStream<ListStatResponseWithContinueToken>> {
            unreachable!()
        }

        async fn delete_folder(&self, _request: Request<DeleteFolderRequest>) -> Result<DeleteFolderResponse> {
            if self.fail {
                Err(UnimplementedError::new().into())
            } else {
                Ok(DeleteFolderResponse::default())
            }
        }

        async fn create_folder(&self, _request: Request<CreateFolderRequest>) -> Result<CreateFolderResponse> {
            unreachable!()
        }

        async fn get_folder_mode(&self, _request: Request<GetFolderModeRequest>) -> Result<GetFolderModeResponse> {
            unreachable!()
        }
    }

    fn service(inner_fails: bool, metadata: RecordingMetadata) -> CreatedTimestampService<DeleteFolderInner> {
        CreatedTimestampService {
            inner: DeleteFolderInner { fail: inner_fails },
            metadata: AnyMetadata::from_metadata(metadata),
        }
    }

    fn delete_request() -> Request<DeleteFolderRequest> {
        Request::new(DeleteFolderRequest {
            folder: Some(FolderAddress {
                uri: FOLDER_URI.to_owned(),
            }),
        })
    }

    // A successful folder delete removes the folder's `created_timestamp` entry.
    #[tokio::test]
    async fn delete_folder_removes_created_timestamp() {
        let metadata = RecordingMetadata::new(false);
        let service = service(false, metadata.clone());

        let response = service.delete_folder(delete_request()).await;

        assert_matches!(response, Ok(_));
        assert_eq!(metadata.delete_calls.load(Ordering::SeqCst), 1);
        assert_matches!(
            metadata.last_delete.lock().unwrap().as_ref(),
            Some((key, uri)) if key == CREATED_TIMESTAMP_KEY && uri == FOLDER_URI
        );
    }

    // Best-effort: a metadata-delete failure must not fail the folder delete the client just made.
    #[tokio::test]
    async fn metadata_delete_failure_does_not_fail_folder_delete() {
        let metadata = RecordingMetadata::new(true);
        let service = service(false, metadata.clone());

        let response = service.delete_folder(delete_request()).await;

        assert_matches!(response, Ok(_));
        // The cleanup was still attempted (and its failure swallowed via a warn-and-continue).
        assert_eq!(metadata.delete_calls.load(Ordering::SeqCst), 1);
    }

    // A failed underlying delete propagates unchanged and leaves the metadata untouched.
    #[tokio::test]
    async fn failed_folder_delete_skips_metadata_cleanup() {
        let metadata = RecordingMetadata::new(false);
        let service = service(true, metadata.clone());

        let response = service.delete_folder(delete_request()).await;

        assert_matches!(response, Err(_));
        assert_eq!(metadata.delete_calls.load(Ordering::SeqCst), 0);
    }
}
