// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{
    collections::HashMap,
    fmt,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

use bytes::{Bytes, BytesMut};
use bytesize::ByteSize;
use futures::{stream, StreamExt};
use serde_json::json;
use storage_core_util::additional_headers::{AdditionalHeaders, RequestType};
use storage_core_util::cache::{CacheControl, LayerCache, LayerCacheArgs};

use super::{
    format::{MANIFEST_FILENAME, WORKSPACE_SUFFIX},
    AddressClass, Config, ManifestService, ManifestState,
};
use crate::traits::{
    async_trait,
    error::{status_with_default_error_info, Result},
    filefolder::v1alpha as ffa,
    filefolder::v1beta as ff,
    fileobject::v1alpha as foa,
    fileobject::v1beta as fo,
    versioning::v1alpha as vera,
    versioning::v1beta as ver,
    BoxFuture, BoxStream, Pagination, Request, StorageError, StorageErrorExt, StreamingOptions,
};

const WORKSPACE: &str = "s3://bucket/workspace.wkspc";
const MANIFEST: &str = "s3://bucket/workspace.wkspc/.manifest";

static CACHE_COUNTER: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MockFailure {
    NotFound,
    Internal,
    Unavailable,
    AccessDenied,
}

impl MockFailure {
    fn code(self) -> tonic::Code {
        match self {
            Self::NotFound => tonic::Code::NotFound,
            Self::Internal => tonic::Code::Internal,
            Self::Unavailable => tonic::Code::Unavailable,
            Self::AccessDenied => tonic::Code::PermissionDenied,
        }
    }

    fn reason(self) -> &'static str {
        match self {
            Self::NotFound => "NotFound",
            Self::Internal => "Internal",
            Self::Unavailable => "Unavailable",
            Self::AccessDenied => "AccessDenied",
        }
    }

    fn into_storage_error(self) -> StorageError {
        Box::new(MockError(self))
    }
}

#[derive(Debug)]
struct MockError(MockFailure);

impl fmt::Display for MockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "mock {}", self.0.reason())
    }
}

impl std::error::Error for MockError {}

impl StorageErrorExt for MockError {
    fn tonic_status(&self) -> tonic::Status {
        status_with_default_error_info(self.0.code(), self.to_string(), self.0.reason())
    }

    fn http_status(&self) -> http::StatusCode {
        match self.0 {
            MockFailure::NotFound => http::StatusCode::NOT_FOUND,
            MockFailure::Internal => http::StatusCode::INTERNAL_SERVER_ERROR,
            MockFailure::Unavailable => http::StatusCode::SERVICE_UNAVAILABLE,
            MockFailure::AccessDenied => http::StatusCode::FORBIDDEN,
        }
    }

    fn get_size(&self) -> usize {
        std::mem::size_of::<Self>()
    }
}

#[derive(Debug, Clone)]
struct Call {
    method: &'static str,
    target: String,
}

/// Stands in for anything an interceptor or tower layer inserts into the request extensions
/// (an authenticated principal, connection info, ...).
#[derive(Debug, Clone, PartialEq)]
struct CallerMarker(String);

/// The caller context a mock entry point received: the request metadata and the
/// `AdditionalHeaders` / `CacheControl` / [`CallerMarker`] extensions.
#[derive(Debug, Clone)]
struct SeenContext {
    method: &'static str,
    target: String,
    metadata: tonic::metadata::MetadataMap,
    additional_headers: Option<Vec<(String, String)>>,
    nocache: bool,
    marker: Option<CallerMarker>,
}

impl SeenContext {
    fn authorization(&self) -> Option<&str> {
        self.metadata.get("authorization").and_then(|value| value.to_str().ok())
    }
}

#[derive(Clone)]
enum MockResult<T> {
    Ok(T),
    Err(MockFailure),
}

impl<T> MockResult<T> {
    fn into_result(self) -> Result<T> {
        match self {
            Self::Ok(value) => Ok(value),
            Self::Err(error) => Err(error.into_storage_error()),
        }
    }
}

#[derive(Clone)]
enum MockStreamItem<T> {
    Ok(T),
    Err(MockFailure),
}

#[derive(Clone, Default)]
struct ListPage {
    folders: Vec<String>,
    files: Vec<String>,
    continue_token: Option<String>,
}

#[derive(Clone, Default)]
struct ListStatPage {
    folders: Vec<String>,
    files: Vec<String>,
    continue_token: Option<String>,
}

#[derive(Clone, Default)]
struct EnumeratePage {
    files: Vec<String>,
    continue_token: Option<String>,
}

#[derive(Clone, Default)]
struct VersionPage {
    identities: Vec<String>,
    continue_token: Option<String>,
    version_id_token: Option<String>,
}

#[derive(Default)]
struct MockState {
    calls: Vec<Call>,
    manifest_read: Option<MockResult<Bytes>>,
    /// Per-call `.manifest` responses consumed front-to-back before falling back to
    /// `manifest_read`/`manifest_read_by_address`. Lets a test drive successive reads of the same
    /// `.manifest` (e.g. a pre-activation `Absent` then a post-activation `Loaded`) deterministically.
    manifest_read_sequence: std::collections::VecDeque<MockResult<Bytes>>,
    manifest_read_by_address: HashMap<String, MockResult<Bytes>>,
    read_from_address: HashMap<String, MockResult<Bytes>>,
    read_by_identity: HashMap<String, MockResult<Bytes>>,
    stat: HashMap<String, MockResult<fo::StatResponse>>,
    list_pages: HashMap<Option<String>, Vec<MockStreamItem<ListPage>>>,
    list_stat_pages: HashMap<Option<String>, Vec<MockStreamItem<ListStatPage>>>,
    enumerate_pages: HashMap<Option<String>, Vec<MockStreamItem<EnumeratePage>>>,
    version_pages: HashMap<Option<String>, Vec<MockStreamItem<VersionPage>>>,
    /// Addresses for which a `read_from_address` / `stat` arrived carrying the `NoCache` extension.
    read_from_address_nocache: Vec<String>,
    stat_nocache: Vec<String>,
    /// Caller context seen by the entry points that record it (`read_from_address` and v1alpha
    /// `is_versioned`), in call order.
    contexts: Vec<SeenContext>,
}

#[derive(Clone, Default)]
struct MockInner {
    state: Arc<Mutex<MockState>>,
}

impl MockInner {
    fn with_state<R>(&self, f: impl FnOnce(&mut MockState) -> R) -> R {
        let mut state = self.state.lock().expect("mock state");
        f(&mut state)
    }

    fn record(&self, method: &'static str, target: impl Into<String>) {
        self.with_state(|state| {
            state.calls.push(Call {
                method,
                target: target.into(),
            });
        });
    }

    fn record_context<T>(&self, method: &'static str, target: &str, request: &Request<T>) {
        let additional_headers = request
            .extensions()
            .get::<AdditionalHeaders>()
            .map(|headers| headers.clone().request_headers(RequestType::Api).collect());
        let nocache = request
            .extensions()
            .get::<CacheControl>()
            .is_some_and(CacheControl::bypasses_cache);
        let marker = request.extensions().get::<CallerMarker>().cloned();
        let context = SeenContext {
            method,
            target: target.to_string(),
            metadata: request.metadata().clone(),
            additional_headers,
            nocache,
            marker,
        };
        self.with_state(|state| state.contexts.push(context));
    }

    fn contexts(&self, method: &'static str) -> Vec<SeenContext> {
        self.with_state(|state| {
            state
                .contexts
                .iter()
                .filter(|context| context.method == method)
                .cloned()
                .collect()
        })
    }

    fn set_manifest(&self, result: MockResult<Bytes>) {
        self.with_state(|state| state.manifest_read = Some(result));
    }

    /// Queue per-call `.manifest` responses, consumed front-to-back before the single-value fallback.
    fn set_manifest_sequence(&self, results: Vec<MockResult<Bytes>>) {
        self.with_state(|state| state.manifest_read_sequence = results.into());
    }

    fn set_manifest_for(&self, address: &str, result: MockResult<Bytes>) {
        self.with_state(|state| {
            state.manifest_read_by_address.insert(address.to_string(), result);
        });
    }

    fn set_valid_manifest(&self, entries: &[(&str, &str)]) {
        self.set_manifest(MockResult::Ok(manifest_json(entries)));
    }

    fn set_read_identity(&self, identity: &str, result: MockResult<Bytes>) {
        self.with_state(|state| {
            state.read_by_identity.insert(identity.to_string(), result);
        });
    }

    fn set_stat(&self, address: &str, result: MockResult<fo::StatResponse>) {
        self.with_state(|state| {
            state.stat.insert(address.to_string(), result);
        });
    }

    fn set_list_pages(&self, cursor: Option<&str>, pages: Vec<MockStreamItem<ListPage>>) {
        self.with_state(|state| {
            state.list_pages.insert(cursor.map(ToString::to_string), pages);
        });
    }

    fn set_list_stat_pages(&self, cursor: Option<&str>, pages: Vec<MockStreamItem<ListStatPage>>) {
        self.with_state(|state| {
            state.list_stat_pages.insert(cursor.map(ToString::to_string), pages);
        });
    }

    fn set_enumerate_pages(&self, cursor: Option<&str>, pages: Vec<MockStreamItem<EnumeratePage>>) {
        self.with_state(|state| {
            state.enumerate_pages.insert(cursor.map(ToString::to_string), pages);
        });
    }

    fn set_version_pages(&self, cursor: Option<&str>, pages: Vec<MockStreamItem<VersionPage>>) {
        self.with_state(|state| {
            state.version_pages.insert(cursor.map(ToString::to_string), pages);
        });
    }

    fn calls(&self, method: &'static str) -> Vec<String> {
        self.with_state(|state| {
            state
                .calls
                .iter()
                .filter(|call| call.method == method)
                .map(|call| call.target.clone())
                .collect()
        })
    }

    fn call_count(&self, method: &'static str) -> usize {
        self.calls(method).len()
    }

    fn manifest_read_count(&self) -> usize {
        self.calls("read_from_address")
            .into_iter()
            .filter(|target| target.ends_with("/.manifest"))
            .count()
    }

    fn nocache_reads(&self) -> Vec<String> {
        self.with_state(|state| state.read_from_address_nocache.clone())
    }

    fn nocache_stats(&self) -> Vec<String> {
        self.with_state(|state| state.stat_nocache.clone())
    }

    fn manifest_result_for(&self, address: &str) -> MockResult<Bytes> {
        self.with_state(|state| {
            if let Some(next) = state.manifest_read_sequence.pop_front() {
                return next;
            }
            state
                .manifest_read_by_address
                .get(address)
                .cloned()
                .or_else(|| state.manifest_read.clone())
                .unwrap_or(MockResult::Err(MockFailure::NotFound))
        })
    }

    fn replace_manifest_for(&self, address: &str, bytes: Bytes) {
        self.set_manifest_for(address, MockResult::Ok(bytes.clone()));
        self.set_manifest(MockResult::Ok(bytes));
    }

    fn move_manifest(&self, source: &str, destination: &str) {
        let source_result = self.manifest_result_for(source);
        self.with_state(|state| {
            state
                .manifest_read_by_address
                .insert(destination.to_string(), source_result);
            state
                .manifest_read_by_address
                .insert(source.to_string(), MockResult::Err(MockFailure::NotFound));
        });
    }
}

#[async_trait]
impl fo::FileObject for MockInner {
    fn inner(&self) -> Option<&dyn fo::FileObject> {
        None
    }

    async fn stat(&self, request: Request<fo::StatRequest>) -> Result<fo::StatResponse> {
        let address = request.get_ref().resource_address.clone();
        self.record("stat", address.clone());
        if request
            .extensions()
            .get::<CacheControl>()
            .is_some_and(CacheControl::bypasses_cache)
        {
            self.with_state(|state| state.stat_nocache.push(address.clone()));
        }
        let result = self.with_state(|state| state.stat.get(&address).cloned());
        result
            .unwrap_or_else(|| MockResult::Ok(stat_response(&format!("identity:{address}"), 1)))
            .into_result()
    }

    async fn read(&self, request: Request<fo::ReadRequest>) -> Result<BoxStream<fo::ReadResponse>> {
        let identity = request
            .get_ref()
            .resource_identity
            .as_ref()
            .map(|identity| identity.encoded_identity.clone())
            .unwrap_or_default();
        self.record("read", identity.clone());
        let result = self.with_state(|state| state.read_by_identity.get(&identity).cloned());
        let bytes = result
            .unwrap_or_else(|| MockResult::Ok(Bytes::from(format!("read:{identity}"))))
            .into_result()?;
        Ok(read_stream(identity, bytes))
    }

    async fn read_from_address(
        &self,
        request: Request<fo::ReadFromAddressRequest>,
    ) -> Result<BoxStream<fo::ReadFromAddressResponse>> {
        let address = request.get_ref().resource_address.clone();
        self.record("read_from_address", address.clone());
        if request
            .extensions()
            .get::<CacheControl>()
            .is_some_and(CacheControl::bypasses_cache)
        {
            self.with_state(|state| state.read_from_address_nocache.push(address.clone()));
        }

        let result = if address.ends_with(&format!("/{MANIFEST_FILENAME}")) {
            self.manifest_result_for(&address)
        } else {
            self.with_state(|state| state.read_from_address.get(&address).cloned())
                .unwrap_or_else(|| MockResult::Ok(Bytes::from(format!("backend:{address}"))))
        };

        let bytes = result.into_result()?;
        Ok(read_from_address_stream(address, bytes))
    }

    async fn write(
        &self,
        request: Request<fo::WriteParameters>,
        data_future: BoxFuture<Bytes>,
    ) -> Result<BoxStream<fo::WriteResponse>> {
        let destination = request.get_ref().destination_resource_address.clone();
        self.record("write", destination.clone());
        let body = data_future.await?;
        if destination.ends_with(&format!("/{MANIFEST_FILENAME}")) {
            self.replace_manifest_for(&destination, body);
        }
        Ok(Box::pin(stream::iter([Ok(fo::WriteResponse {
            write_response_type: Some(fo::write_response::WriteResponseType::ResourceInfo(resource_info(
                &format!("identity:{destination}"),
                1,
            ))),
        })])))
    }

    async fn delete(&self, request: Request<fo::DeleteRequest>) -> Result<fo::DeleteResponse> {
        self.record("delete", request.get_ref().resource_address.clone());
        Ok(fo::DeleteResponse {})
    }

    async fn abort_multipart_upload(
        &self,
        request: Request<fo::AbortMultipartUploadRequest>,
    ) -> Result<fo::AbortMultipartUploadResponse> {
        self.record(
            "abort_multipart_upload",
            request.get_ref().destination_resource_address.clone(),
        );
        Ok(fo::AbortMultipartUploadResponse {})
    }

    async fn complete_multipart_upload(
        &self,
        request: Request<fo::CompleteMultipartUploadRequest>,
    ) -> Result<fo::CompleteMultipartUploadResponse> {
        let destination = request.get_ref().destination_resource_address.clone();
        self.record("complete_multipart_upload", destination.clone());
        Ok(fo::CompleteMultipartUploadResponse {
            resource_info: Some(resource_info(&format!("identity:{destination}"), 1)),
        })
    }

    async fn upload_part(&self, request: Request<fo::UploadPartRequest>) -> Result<fo::UploadPartResponse> {
        self.record("upload_part", request.get_ref().destination_resource_address.clone());
        Ok(fo::UploadPartResponse {
            part_write_redirects: vec![],
        })
    }

    async fn complete_redirect_upload(
        &self,
        request: Request<fo::CompleteRedirectUploadRequest>,
    ) -> Result<fo::CompleteRedirectUploadResponse> {
        let destination = request.get_ref().destination_resource_address.clone();
        self.record("complete_redirect_upload", destination.clone());
        Ok(fo::CompleteRedirectUploadResponse {
            resource_info: Some(resource_info(&format!("identity:{destination}"), 1)),
        })
    }

    async fn fetch_write_type_info(
        &self,
        request: Request<fo::FetchWriteTypeInfoRequest>,
    ) -> Result<fo::FetchWriteTypeInfoResponse> {
        self.record(
            "fetch_write_type_info",
            request.get_ref().destination_resource_address.clone(),
        );
        Ok(fo::FetchWriteTypeInfoResponse {
            write_type_intervals: vec![],
        })
    }

    async fn enumerate(
        &self,
        request: Request<fo::EnumerateRequest>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<fo::EnumerateResponseWithContinueToken>> {
        let address = request.get_ref().resource_address.clone();
        self.record("enumerate", address);
        let cursor = continuation(&streaming);
        let pages = self.with_state(|state| state.enumerate_pages.get(&cursor).cloned());
        let pages = pages.unwrap_or_default();
        Ok(Box::pin(stream::iter(pages.into_iter().map(|page| match page {
            MockStreamItem::Ok(page) => Ok(fo::EnumerateResponseWithContinueToken {
                response: fo::EnumerateResponse {
                    items: page.files.into_iter().map(|address| address_info(&address)).collect(),
                },
                continue_token: page.continue_token,
            }),
            MockStreamItem::Err(error) => Err(error.into_storage_error()),
        }))))
    }
}

#[async_trait]
impl foa::FileObject for MockInner {
    fn inner(&self) -> Option<&dyn foa::FileObject> {
        None
    }

    async fn stat(&self, request: Request<foa::StatRequest>) -> Result<foa::StatResponse> {
        let address = request.get_ref().resource_address.clone();
        self.record("v1alpha_stat", address.clone());
        Ok(foa::StatResponse {
            resource_info: Some(alpha_resource_info(&format!("identity:{address}"), 1)),
        })
    }

    async fn read(&self, request: Request<foa::ReadRequest>) -> Result<BoxStream<foa::ReadResponse>> {
        let identity = request
            .get_ref()
            .resource_identity
            .as_ref()
            .map(|identity| identity.encoded_identity.clone())
            .unwrap_or_default();
        self.record("v1alpha_read", identity.clone());
        Ok(alpha_read_stream(identity, Bytes::new()))
    }

    async fn is_read_only(&self, request: Request<foa::IsReadOnlyRequest>) -> Result<foa::IsReadOnlyResponse> {
        let address = request.get_ref().resource_address.clone();
        self.record("is_read_only", address);
        Ok(foa::IsReadOnlyResponse { is_read_only: false })
    }

    async fn read_from_address(
        &self,
        request: Request<foa::ReadFromAddressRequest>,
    ) -> Result<BoxStream<foa::ReadFromAddressResponse>> {
        let address = request.get_ref().resource_address.clone();
        self.record("read_from_address", address.clone());
        self.record_context("read_from_address", &address, &request);
        if request
            .extensions()
            .get::<CacheControl>()
            .is_some_and(CacheControl::bypasses_cache)
        {
            self.with_state(|state| state.read_from_address_nocache.push(address.clone()));
        }
        let result = if address.ends_with(&format!("/{MANIFEST_FILENAME}")) {
            self.manifest_result_for(&address)
        } else {
            MockResult::Ok(Bytes::from(format!("backend:{address}")))
        };
        Ok(alpha_read_from_address_stream(address, result.into_result()?))
    }

    async fn write(
        &self,
        request: Request<foa::WriteParameters>,
        data_future: BoxFuture<Bytes>,
    ) -> Result<BoxStream<foa::WriteResponse>> {
        let destination = request.get_ref().destination_resource_address.clone();
        self.record("v1alpha_write", destination.clone());
        let body = data_future.await?;
        if destination.ends_with(&format!("/{MANIFEST_FILENAME}")) {
            self.replace_manifest_for(&destination, body);
        }
        Ok(Box::pin(stream::iter([Ok(foa::WriteResponse {
            write_response_type: Some(foa::write_response::WriteResponseType::ResourceInfo(
                alpha_resource_info(&format!("identity:{destination}"), 1),
            )),
        })])))
    }

    async fn delete(&self, request: Request<foa::DeleteRequest>) -> Result<foa::DeleteResponse> {
        self.record("v1alpha_delete", request.get_ref().resource_address.clone());
        Ok(foa::DeleteResponse {})
    }

    async fn abort_multipart_upload(
        &self,
        request: Request<foa::AbortMultipartUploadRequest>,
    ) -> Result<foa::AbortMultipartUploadResponse> {
        self.record(
            "v1alpha_abort_multipart_upload",
            request.get_ref().destination_resource_address.clone(),
        );
        Ok(foa::AbortMultipartUploadResponse {})
    }

    async fn complete_multipart_upload(
        &self,
        request: Request<foa::CompleteMultipartUploadRequest>,
    ) -> Result<foa::CompleteMultipartUploadResponse> {
        let destination = request.get_ref().destination_resource_address.clone();
        self.record("v1alpha_complete_multipart_upload", destination.clone());
        Ok(foa::CompleteMultipartUploadResponse {
            resource_info: Some(alpha_resource_info(&format!("identity:{destination}"), 1)),
        })
    }

    async fn upload_part(&self, request: Request<foa::UploadPartRequest>) -> Result<foa::UploadPartResponse> {
        self.record(
            "v1alpha_upload_part",
            request.get_ref().destination_resource_address.clone(),
        );
        Ok(foa::UploadPartResponse {
            part_write_redirects: vec![],
        })
    }

    async fn complete_redirect_upload(
        &self,
        request: Request<foa::CompleteRedirectUploadRequest>,
    ) -> Result<foa::CompleteRedirectUploadResponse> {
        let destination = request.get_ref().destination_resource_address.clone();
        self.record("v1alpha_complete_redirect_upload", destination.clone());
        Ok(foa::CompleteRedirectUploadResponse {
            resource_info: Some(alpha_resource_info(&format!("identity:{destination}"), 1)),
        })
    }

    async fn fetch_write_type_info(
        &self,
        request: Request<foa::FetchWriteTypeInfoRequest>,
    ) -> Result<foa::FetchWriteTypeInfoResponse> {
        self.record(
            "v1alpha_fetch_write_type_info",
            request.get_ref().destination_resource_address.clone(),
        );
        Ok(foa::FetchWriteTypeInfoResponse {
            write_type_intervals: vec![],
        })
    }

    async fn enumerate(
        &self,
        request: Request<foa::EnumerateRequest>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<foa::EnumerateResponseWithContinueToken>> {
        self.record("v1alpha_enumerate", request.get_ref().resource_address.clone());
        let cursor = continuation(&streaming);
        let pages = self.with_state(|state| state.enumerate_pages.get(&cursor).cloned());
        let pages = pages.unwrap_or_default();
        Ok(Box::pin(stream::iter(pages.into_iter().map(|page| {
            match page {
                MockStreamItem::Ok(page) => Ok(foa::EnumerateResponseWithContinueToken {
                    response: foa::EnumerateResponse {
                        items: page
                            .files
                            .into_iter()
                            .map(|address| alpha_address_info(&address))
                            .collect(),
                    },
                    continue_token: page.continue_token,
                }),
                MockStreamItem::Err(error) => Err(error.into_storage_error()),
            }
        }))))
    }

    async fn copy(&self, request: Request<foa::CopyRequest>) -> Result<foa::CopyResponse> {
        let destination = request.get_ref().destination_resource_address.clone();
        self.record("copy", destination.clone());
        Ok(foa::CopyResponse {
            resource_identity: Some(foa::ResourceIdentity {
                encoded_identity: format!("identity:{destination}"),
            }),
        })
    }

    async fn move_object(&self, request: Request<foa::MoveRequest>) -> Result<foa::MoveResponse> {
        let source = request.get_ref().source_resource_address.clone();
        let destination = request.get_ref().destination_resource_address.clone();
        self.record("move_object", format!("{source}->{destination}"));
        if source.ends_with(&format!("/{MANIFEST_FILENAME}")) || destination.ends_with(&format!("/{MANIFEST_FILENAME}"))
        {
            self.move_manifest(&source, &destination);
        }
        Ok(foa::MoveResponse {
            resource_identity: Some(foa::ResourceIdentity {
                encoded_identity: format!("identity:{destination}"),
            }),
        })
    }

    async fn get_optimistic_locking_support(
        &self,
        request: Request<foa::GetOptimisticLockingSupportRequest>,
    ) -> Result<foa::GetOptimisticLockingSupportResponse> {
        self.record(
            "get_optimistic_locking_support",
            request.get_ref().resource_address.clone(),
        );
        Ok(foa::GetOptimisticLockingSupportResponse {
            supports_write: true,
            supports_delete: true,
            supports_copy: true,
            supports_move: true,
            supports_write_fail_if_exists: true,
            supports_copy_fail_if_exists: true,
            supports_move_fail_if_exists: true,
        })
    }
}

#[async_trait]
impl ff::FileFolder for MockInner {
    fn inner(&self) -> Option<&dyn ff::FileFolder> {
        None
    }

    async fn list(
        &self,
        request: Request<ff::ListRequest>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<ff::ListResponseWithContinueToken>> {
        let address = request
            .get_ref()
            .folder
            .as_ref()
            .map(|folder| folder.uri.clone())
            .unwrap_or_default();
        self.record("list", address);
        let cursor = continuation(&streaming);
        let pages = self.with_state(|state| state.list_pages.get(&cursor).cloned());
        let pages = pages.unwrap_or_default();
        Ok(Box::pin(stream::iter(pages.into_iter().map(|page| match page {
            MockStreamItem::Ok(page) => Ok(ff::ListResponseWithContinueToken {
                response: ff::ListResponse {
                    subfolder_addresses: page.folders.into_iter().map(|uri| ff::FolderAddress { uri }).collect(),
                    sub_resource_addresses: page.files,
                },
                continue_token: page.continue_token,
            }),
            MockStreamItem::Err(error) => Err(error.into_storage_error()),
        }))))
    }

    async fn list_stat(
        &self,
        request: Request<ff::ListStatRequest>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<ff::ListStatResponseWithContinueToken>> {
        let address = request
            .get_ref()
            .folder
            .as_ref()
            .map(|folder| folder.uri.clone())
            .unwrap_or_default();
        self.record("list_stat", address);
        let cursor = continuation(&streaming);
        let pages = self.with_state(|state| state.list_stat_pages.get(&cursor).cloned());
        let pages = pages.unwrap_or_default();
        Ok(Box::pin(stream::iter(pages.into_iter().map(|page| {
            match page {
                MockStreamItem::Ok(page) => Ok(ff::ListStatResponseWithContinueToken {
                    response: ff::ListStatResponse {
                        subfolder_addresses: page.folders.into_iter().map(|uri| ff::FolderAddress { uri }).collect(),
                        entries: page
                            .files
                            .into_iter()
                            .map(|resource_address| ff::ListItem {
                                resource_info: Some(resource_info(&format!("identity:{resource_address}"), 1)),
                                resource_address,
                            })
                            .collect(),
                    },
                    continue_token: page.continue_token,
                }),
                MockStreamItem::Err(error) => Err(error.into_storage_error()),
            }
        }))))
    }

    async fn delete_folder(&self, request: Request<ff::DeleteFolderRequest>) -> Result<ff::DeleteFolderResponse> {
        let address = request
            .get_ref()
            .folder
            .as_ref()
            .map(|folder| folder.uri.clone())
            .unwrap_or_default();
        self.record("delete_folder", address);
        Ok(ff::DeleteFolderResponse {})
    }
}

#[async_trait]
impl ver::Versioning for MockInner {
    fn inner(&self) -> Option<&dyn ver::Versioning> {
        None
    }

    async fn enumerate_versions(
        &self,
        request: Request<ver::EnumerateVersionsRequest>,
        streaming: StreamingOptions,
        version_id_token: Option<String>,
    ) -> Result<BoxStream<ver::EnumerateVersionsResponseWithContinueToken>> {
        self.record("enumerate_versions", request.get_ref().resource_address.clone());
        let cursor = continuation(&streaming).or(version_id_token);
        let pages = self.with_state(|state| state.version_pages.get(&cursor).cloned());
        let pages = pages.unwrap_or_default();
        Ok(Box::pin(stream::iter(pages.into_iter().map(|page| {
            match page {
                MockStreamItem::Ok(page) => Ok(ver::EnumerateVersionsResponseWithContinueToken {
                    response: ver::EnumerateVersionsResponse {
                        items: page
                            .identities
                            .into_iter()
                            .map(|identity| ver::VersionInfo {
                                resource_info: Some(resource_info(&identity, 1)),
                                sorting_key: None,
                            })
                            .collect(),
                        versions_order: ver::VersionsOrder::NewestFirst as i32,
                    },
                    continue_token: page.continue_token,
                    version_id_token: page.version_id_token,
                }),
                MockStreamItem::Err(error) => Err(error.into_storage_error()),
            }
        }))))
    }
}

#[async_trait]
impl vera::Versioning for MockInner {
    fn inner(&self) -> Option<&dyn vera::Versioning> {
        None
    }

    async fn is_versioned(&self, request: Request<vera::IsVersionedRequest>) -> Result<vera::IsVersionedResponse> {
        let address = request.get_ref().resource_address.clone();
        self.record("is_versioned", address.clone());
        self.record_context("is_versioned", &address, &request);
        // The mock physical backend has versioning disabled; the manifest layer
        // must not delegate to this when it can synthesize versions itself.
        Ok(vera::IsVersionedResponse { is_versioned: false })
    }
}

#[async_trait]
impl ffa::FileFolder for MockInner {
    fn inner(&self) -> Option<&dyn ffa::FileFolder> {
        None
    }

    async fn list(
        &self,
        request: Request<ffa::ListRequest>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<ffa::ListResponseWithContinueToken>> {
        let address = request
            .get_ref()
            .folder
            .as_ref()
            .map(|folder| folder.uri.clone())
            .unwrap_or_default();
        self.record("list", address);
        let cursor = continuation(&streaming);
        let pages = self.with_state(|state| state.list_pages.get(&cursor).cloned());
        let pages = pages.unwrap_or_default();
        Ok(Box::pin(stream::iter(pages.into_iter().map(|page| match page {
            MockStreamItem::Ok(page) => Ok(ffa::ListResponseWithContinueToken {
                response: ffa::ListResponse {
                    subfolder_addresses: page.folders.into_iter().map(|uri| ffa::FolderAddress { uri }).collect(),
                    sub_resource_addresses: page.files,
                },
                continue_token: page.continue_token,
            }),
            MockStreamItem::Err(error) => Err(error.into_storage_error()),
        }))))
    }

    async fn list_stat(
        &self,
        request: Request<ffa::ListStatRequest>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<ffa::ListStatResponseWithContinueToken>> {
        let address = request
            .get_ref()
            .folder
            .as_ref()
            .map(|folder| folder.uri.clone())
            .unwrap_or_default();
        self.record("list_stat", address);
        let cursor = continuation(&streaming);
        let pages = self.with_state(|state| state.list_stat_pages.get(&cursor).cloned());
        let pages = pages.unwrap_or_default();
        Ok(Box::pin(stream::iter(pages.into_iter().map(|page| {
            match page {
                MockStreamItem::Ok(page) => Ok(ffa::ListStatResponseWithContinueToken {
                    response: ffa::ListStatResponse {
                        subfolder_addresses: page.folders.into_iter().map(|uri| ffa::FolderAddress { uri }).collect(),
                        entries: page
                            .files
                            .into_iter()
                            .map(|resource_address| ffa::ListItem {
                                resource_info: Some(alpha_resource_info(&format!("identity:{resource_address}"), 1)),
                                resource_address,
                            })
                            .collect(),
                    },
                    continue_token: page.continue_token,
                }),
                MockStreamItem::Err(error) => Err(error.into_storage_error()),
            }
        }))))
    }

    async fn delete_folder(&self, request: Request<ffa::DeleteFolderRequest>) -> Result<ffa::DeleteFolderResponse> {
        let address = request
            .get_ref()
            .folder
            .as_ref()
            .map(|folder| folder.uri.clone())
            .unwrap_or_default();
        self.record("delete_folder", address);
        Ok(ffa::DeleteFolderResponse {})
    }
}

async fn create_service(inner: MockInner) -> ManifestService<MockInner, MockInner> {
    create_service_with_resolver(inner.clone(), inner).await
}

async fn create_service_with_resolver(inner: MockInner, resolver: MockInner) -> ManifestService<MockInner, MockInner> {
    let page_size_limit = ByteSize::kib(10);
    let config = Config {
        page_size_limit,
        ..Default::default()
    };
    let args: LayerCacheArgs = config.clone().into();
    let index = CACHE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let namespace = format!("manifest_overlay_tests_{index}");
    let cache = LayerCache::new(&args, &namespace, &namespace, None, None)
        .await
        .expect("LayerCache build for manifest overlay tests");
    ManifestService::new(inner, resolver, config, Arc::new(cache), page_size_limit)
}

async fn create_service_with_config(inner: MockInner, config: Config) -> ManifestService<MockInner, MockInner> {
    let page_size_limit = config.page_size_limit;
    let args: LayerCacheArgs = config.clone().into();
    let index = CACHE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let namespace = format!("manifest_overlay_tests_{index}");
    let cache = LayerCache::new(&args, &namespace, &namespace, None, None)
        .await
        .expect("LayerCache build for manifest overlay tests");
    let resolver = inner.clone();
    ManifestService::new(inner, resolver, config, Arc::new(cache), page_size_limit)
}

fn manifest_json(entries: &[(&str, &str)]) -> Bytes {
    let files: Vec<_> = entries
        .iter()
        .map(|(relative_path, resource_identity)| {
            json!({
                "relative_path": relative_path,
                "resource_identity": resource_identity,
            })
        })
        .collect();
    Bytes::from(
        serde_json::to_vec(&json!({
            "format_version": { "version_string": "1.0" },
            "storage_api_version": "v1beta",
            "files": files,
        }))
        .expect("manifest json"),
    )
}

fn manifest_info_json(path: &str, identity: &str, size: u64) -> Bytes {
    Bytes::from(
        serde_json::to_vec(&json!({
            "format_version": { "version_string": "1.0" },
            "storage_api_version": "v1beta",
            "files": [{
                "relative_path": path,
                "resource_info": {
                    "resource_identity": identity,
                    "size": size,
                    "modified_time": "2024-01-01T00:00:00Z",
                },
            }],
        }))
        .expect("manifest json"),
    )
}

fn invalid_path_manifest(path: &str) -> Bytes {
    manifest_json(&[(path, "s3://bucket/object")])
}

fn resource_info(identity: &str, size: u64) -> fo::ResourceInfo {
    fo::ResourceInfo {
        resource_identity: Some(fo::ResourceIdentity {
            encoded_identity: identity.to_string(),
        }),
        metadata: Some(fo::Metadata {
            data_object_size: Some(size),
            last_modified_timestamp: None,
        }),
    }
}

fn alpha_resource_info(identity: &str, size: u64) -> foa::ResourceInfo {
    foa::ResourceInfo {
        resource_identity: Some(foa::ResourceIdentity {
            encoded_identity: identity.to_string(),
        }),
        metadata: Some(foa::Metadata {
            native_metadata: Default::default(),
            data_object_size: Some(size),
            last_modified_timestamp: None,
        }),
    }
}

fn stat_response(identity: &str, size: u64) -> fo::StatResponse {
    fo::StatResponse {
        resource_info: Some(resource_info(identity, size)),
    }
}

fn address_info(address: &str) -> fo::AddressInfo {
    fo::AddressInfo {
        resource_address: address.to_string(),
        metadata: Some(fo::Metadata {
            data_object_size: Some(1),
            last_modified_timestamp: None,
        }),
    }
}

fn alpha_address_info(address: &str) -> foa::AddressInfo {
    foa::AddressInfo {
        resource_address: address.to_string(),
        metadata: Some(foa::Metadata {
            native_metadata: Default::default(),
            data_object_size: Some(1),
            last_modified_timestamp: None,
        }),
    }
}

fn read_stream(identity: String, bytes: Bytes) -> BoxStream<fo::ReadResponse> {
    Box::pin(
        stream::iter([
            Ok(fo::ReadResponse {
                reply_type: Some(fo::read_response::ReplyType::Metadata(fo::Metadata {
                    data_object_size: Some(bytes.len() as u64),
                    last_modified_timestamp: None,
                })),
            }),
            Ok(fo::ReadResponse {
                reply_type: Some(fo::read_response::ReplyType::Chunk(fo::Chunk { chunk: bytes })),
            }),
        ])
        .inspect(move |_| {
            let _ = &identity;
        }),
    )
}

fn alpha_read_stream(_identity: String, bytes: Bytes) -> BoxStream<foa::ReadResponse> {
    Box::pin(stream::iter([
        Ok(foa::ReadResponse {
            reply_type: Some(foa::read_response::ReplyType::Metadata(foa::Metadata {
                native_metadata: Default::default(),
                data_object_size: Some(bytes.len() as u64),
                last_modified_timestamp: None,
            })),
        }),
        Ok(foa::ReadResponse {
            reply_type: Some(foa::read_response::ReplyType::Chunk(foa::Chunk { chunk: bytes })),
        }),
    ]))
}

fn read_from_address_stream(address: String, bytes: Bytes) -> BoxStream<fo::ReadFromAddressResponse> {
    Box::pin(stream::iter([
        Ok(fo::ReadFromAddressResponse {
            reply_type: Some(fo::read_from_address_response::ReplyType::ResourceInfo(resource_info(
                &format!("identity:{address}"),
                bytes.len() as u64,
            ))),
        }),
        Ok(fo::ReadFromAddressResponse {
            reply_type: Some(fo::read_from_address_response::ReplyType::Chunk(fo::Chunk {
                chunk: bytes,
            })),
        }),
    ]))
}

fn alpha_read_from_address_stream(address: String, bytes: Bytes) -> BoxStream<foa::ReadFromAddressResponse> {
    Box::pin(stream::iter([
        Ok(foa::ReadFromAddressResponse {
            reply_type: Some(foa::read_from_address_response::ReplyType::ResourceInfo(
                alpha_resource_info(&format!("identity:{address}"), bytes.len() as u64),
            )),
        }),
        Ok(foa::ReadFromAddressResponse {
            reply_type: Some(foa::read_from_address_response::ReplyType::Chunk(foa::Chunk {
                chunk: bytes,
            })),
        }),
    ]))
}

fn continuation(streaming: &StreamingOptions) -> Option<String> {
    match streaming {
        StreamingOptions::Streamed => None,
        StreamingOptions::Paginated(pagination) => pagination.continuation_handle.clone(),
    }
}

fn read_request(address: &str) -> Request<fo::ReadFromAddressRequest> {
    Request::new(fo::ReadFromAddressRequest {
        resource_address: address.to_string(),
        download_preference: Some(fo::DownloadPreference::Body as i32),
    })
}

fn stat_request(address: &str) -> Request<fo::StatRequest> {
    Request::new(fo::StatRequest {
        resource_address: address.to_string(),
    })
}

fn is_read_only_request(address: &str) -> Request<foa::IsReadOnlyRequest> {
    Request::new(foa::IsReadOnlyRequest {
        resource_address: address.to_string(),
    })
}

fn is_versioned_request(address: &str) -> Request<vera::IsVersionedRequest> {
    Request::new(vera::IsVersionedRequest {
        resource_address: address.to_string(),
    })
}

/// Adds the caller context that ingress and `AdditionalHeaderManager` attach to a request: an
/// `authorization` metadata entry, an `AdditionalHeaders` extension carrying a request id, and an
/// interceptor-style [`CallerMarker`] extension.
fn with_caller_context<T>(mut request: Request<T>, caller: &str) -> Request<T> {
    request.metadata_mut().insert(
        "authorization",
        format!("Bearer {caller}")
            .parse()
            .expect("authorization metadata value"),
    );
    request.extensions_mut().insert(AdditionalHeaders::new(
        vec![("x-request-id".to_string(), caller.to_string())],
        None,
        None,
        vec![],
    ));
    request.extensions_mut().insert(CallerMarker(caller.to_string()));
    request
}

/// The `AdditionalHeaders` that [`with_caller_context`] attaches, as `SeenContext` records them.
fn caller_headers(caller: &str) -> Option<Vec<(String, String)>> {
    Some(vec![("x-request-id".to_string(), caller.to_string())])
}

fn write_request(address: &str) -> Request<fo::WriteParameters> {
    Request::new(fo::WriteParameters {
        destination_resource_address: address.to_string(),
        previous_version: None,
        data_object_size: 0,
        upload_preference: None,
        fail_if_exists: None,
    })
}

fn delete_request(address: &str) -> Request<fo::DeleteRequest> {
    Request::new(fo::DeleteRequest {
        resource_address: address.to_string(),
        previous_version: None,
    })
}

fn list_request(address: &str) -> Request<ff::ListRequest> {
    Request::new(ff::ListRequest {
        folder: Some(ff::FolderAddress {
            uri: address.to_string(),
        }),
    })
}

fn list_stat_request(address: &str) -> Request<ff::ListStatRequest> {
    Request::new(ff::ListStatRequest {
        folder: Some(ff::FolderAddress {
            uri: address.to_string(),
        }),
    })
}

fn enumerate_request(address: &str) -> Request<fo::EnumerateRequest> {
    Request::new(fo::EnumerateRequest {
        resource_address: address.to_string(),
    })
}

fn versions_request(address: &str) -> Request<ver::EnumerateVersionsRequest> {
    Request::new(ver::EnumerateVersionsRequest {
        resource_address: address.to_string(),
    })
}

fn alpha_copy_request(destination: &str) -> Request<foa::CopyRequest> {
    Request::new(foa::CopyRequest {
        source_resource_identity: Some(foa::ResourceIdentity {
            encoded_identity: "s3://objects/source".to_string(),
        }),
        destination_resource_address: destination.to_string(),
        previous_version: None,
        fail_if_exists: None,
    })
}

fn alpha_move_request(source: &str, destination: &str) -> Request<foa::MoveRequest> {
    Request::new(foa::MoveRequest {
        source_resource_address: source.to_string(),
        source_previous_version: None,
        destination_resource_address: destination.to_string(),
        destination_previous_version: None,
        fail_if_exists: None,
    })
}

fn bytes_future(bytes: Bytes) -> BoxFuture<Bytes> {
    Box::pin(async move { Ok(bytes) })
}

fn alpha_read_request(address: &str) -> Request<foa::ReadFromAddressRequest> {
    Request::new(foa::ReadFromAddressRequest {
        resource_address: address.to_string(),
        download_preference: Some(foa::DownloadPreference::Body as i32),
    })
}

fn alpha_stat_request(address: &str) -> Request<foa::StatRequest> {
    Request::new(foa::StatRequest {
        resource_address: address.to_string(),
    })
}

fn alpha_enumerate_request(address: &str) -> Request<foa::EnumerateRequest> {
    Request::new(foa::EnumerateRequest {
        resource_address: address.to_string(),
    })
}

fn alpha_list_request(address: &str) -> Request<ffa::ListRequest> {
    Request::new(ffa::ListRequest {
        folder: Some(ffa::FolderAddress {
            uri: address.to_string(),
        }),
    })
}

fn alpha_list_stat_request(address: &str) -> Request<ffa::ListStatRequest> {
    Request::new(ffa::ListStatRequest {
        include_native_metadata: false,
        folder: Some(ffa::FolderAddress {
            uri: address.to_string(),
        }),
    })
}

async fn collect_alpha_read_from_address(stream: BoxStream<foa::ReadFromAddressResponse>) -> Result<Bytes> {
    let mut stream = stream;
    let mut bytes = BytesMut::new();
    while let Some(response) = stream.next().await {
        let response = response?;
        if let Some(foa::read_from_address_response::ReplyType::Chunk(chunk)) = response.reply_type {
            bytes.extend_from_slice(&chunk.chunk);
        }
    }
    Ok(bytes.freeze())
}

async fn collect_alpha_read_from_address_identity(stream: BoxStream<foa::ReadFromAddressResponse>) -> Result<String> {
    let mut stream = stream;
    while let Some(response) = stream.next().await {
        let response = response?;
        if let Some(foa::read_from_address_response::ReplyType::ResourceInfo(info)) = response.reply_type {
            return Ok(info.resource_identity.expect("resource identity").encoded_identity);
        }
    }
    panic!("read_from_address stream did not contain resource info")
}

async fn collect_alpha_enumerate(stream: BoxStream<foa::EnumerateResponseWithContinueToken>) -> Result<Vec<String>> {
    let mut stream = stream;
    let mut items = Vec::new();
    while let Some(response) = stream.next().await {
        let response = response?;
        items.extend(response.response.items.into_iter().map(|item| item.resource_address));
    }
    Ok(items)
}

async fn collect_alpha_list(stream: BoxStream<ffa::ListResponseWithContinueToken>) -> Result<Vec<String>> {
    let mut stream = stream;
    let mut items = Vec::new();
    while let Some(response) = stream.next().await {
        let response = response?;
        items.extend(
            response
                .response
                .subfolder_addresses
                .into_iter()
                .map(|folder| format!("folder:{}", folder.uri)),
        );
        items.extend(
            response
                .response
                .sub_resource_addresses
                .into_iter()
                .map(|address| format!("file:{address}")),
        );
    }
    Ok(items)
}

async fn load_state(service: &ManifestService<MockInner, MockInner>) -> Result<ManifestState> {
    let key = super::format::WorkspaceKey::parse(WORKSPACE)
        .expect("parse workspace")
        .expect("workspace key");
    service
        .get_manifest_state(&key, |key| {
            service.read_manifest_v1beta(key, Default::default(), Default::default())
        })
        .await
}

async fn load_alpha_state_at(
    service: &ManifestService<MockInner, MockInner>,
    workspace_path: &str,
) -> Result<ManifestState> {
    let key = super::format::WorkspaceKey::parse(workspace_path)
        .expect("parse workspace")
        .expect("workspace key");
    service
        .get_manifest_state(&key, |key| {
            service.read_manifest_v1alpha(key, Default::default(), Default::default())
        })
        .await
}

async fn collect_read_from_address(stream: BoxStream<fo::ReadFromAddressResponse>) -> Result<Bytes> {
    let mut stream = stream;
    let mut bytes = BytesMut::new();
    while let Some(response) = stream.next().await {
        let response = response?;
        if let Some(fo::read_from_address_response::ReplyType::Chunk(chunk)) = response.reply_type {
            bytes.extend_from_slice(&chunk.chunk);
        }
    }
    Ok(bytes.freeze())
}

async fn collect_read_from_address_identity(stream: BoxStream<fo::ReadFromAddressResponse>) -> Result<String> {
    let mut stream = stream;
    while let Some(response) = stream.next().await {
        let response = response?;
        if let Some(fo::read_from_address_response::ReplyType::ResourceInfo(info)) = response.reply_type {
            return Ok(info.resource_identity.expect("resource identity").encoded_identity);
        }
    }
    panic!("read_from_address stream did not contain resource info")
}

async fn collect_list(stream: BoxStream<ff::ListResponseWithContinueToken>) -> Result<Vec<String>> {
    let mut stream = stream;
    let mut items = Vec::new();
    while let Some(response) = stream.next().await {
        let response = response?;
        items.extend(
            response
                .response
                .subfolder_addresses
                .into_iter()
                .map(|folder| format!("folder:{}", folder.uri)),
        );
        items.extend(
            response
                .response
                .sub_resource_addresses
                .into_iter()
                .map(|address| format!("file:{address}")),
        );
    }
    Ok(items)
}

async fn collect_list_stat(stream: BoxStream<ff::ListStatResponseWithContinueToken>) -> Result<Vec<String>> {
    let mut stream = stream;
    let mut items = Vec::new();
    while let Some(response) = stream.next().await {
        let response = response?;
        items.extend(
            response
                .response
                .subfolder_addresses
                .into_iter()
                .map(|folder| format!("folder:{}", folder.uri)),
        );
        items.extend(
            response
                .response
                .entries
                .into_iter()
                .map(|entry| format!("file:{}", entry.resource_address)),
        );
    }
    Ok(items)
}

async fn collect_enumerate(stream: BoxStream<fo::EnumerateResponseWithContinueToken>) -> Result<Vec<String>> {
    let mut stream = stream;
    let mut items = Vec::new();
    while let Some(response) = stream.next().await {
        let response = response?;
        items.extend(response.response.items.into_iter().map(|item| item.resource_address));
    }
    Ok(items)
}

async fn collect_versions(stream: BoxStream<ver::EnumerateVersionsResponseWithContinueToken>) -> Result<Vec<String>> {
    let mut stream = stream;
    let mut items = Vec::new();
    while let Some(response) = stream.next().await {
        let response = response?;
        items.extend(response.response.items.into_iter().filter_map(|item| {
            item.resource_info
                .and_then(|info| info.resource_identity)
                .map(|identity| identity.encoded_identity)
        }));
    }
    Ok(items)
}

fn assert_code<T>(result: Result<T>, code: tonic::Code) {
    match result {
        Ok(_) => panic!("expected {code:?} error"),
        Err(error) => assert_eq!(error.tonic_status().code(), code),
    }
}

#[tokio::test]
async fn classifier_table() {
    let service = create_service(MockInner::default()).await;

    assert!(matches!(
        service
            .classify_address("s3://bucket/plain/file.usd")
            .expect("classify"),
        AddressClass::NotWorkspace
    ));

    match service.classify_address(MANIFEST).expect("classify") {
        AddressClass::ManifestFile { key } => {
            assert_eq!(key.workspace_path, WORKSPACE);
            assert_eq!(key.manifest_path, MANIFEST);
            assert_eq!(key.object_path.as_deref(), Some(MANIFEST_FILENAME));
        },
        other => panic!("unexpected class: {other:?}"),
    }

    match service.classify_address(WORKSPACE).expect("classify") {
        AddressClass::WorkspacePath { key } => {
            assert_eq!(key.workspace_path, WORKSPACE);
            assert_eq!(key.object_path, None);
        },
        other => panic!("unexpected class: {other:?}"),
    }

    match service.classify_address(&format!("{WORKSPACE}/a/b")).expect("classify") {
        AddressClass::WorkspacePath { key } => assert_eq!(key.object_path.as_deref(), Some("a/b")),
        other => panic!("unexpected class: {other:?}"),
    }
}

#[tokio::test]
async fn manifest_state_loads_loaded_absent_and_invalid() {
    let inner = MockInner::default();
    inner.set_valid_manifest(&[("a.usd", "s3://bucket/a")]);
    let service = create_service(inner.clone()).await;
    assert!(matches!(
        load_state(&service).await.expect("loaded"),
        ManifestState::Loaded(_)
    ));

    let inner = MockInner::default();
    inner.set_manifest(MockResult::Err(MockFailure::NotFound));
    let service = create_service(inner.clone()).await;
    assert!(matches!(
        load_state(&service).await.expect("absent"),
        ManifestState::Absent
    ));

    let inner = MockInner::default();
    inner.set_manifest(MockResult::Ok(Bytes::from_static(b"not-json")));
    let service = create_service(inner.clone()).await;
    assert!(matches!(
        load_state(&service).await.expect("invalid"),
        ManifestState::Invalid
    ));
}

#[tokio::test]
async fn invalid_manifest_versions_and_entries_cache_as_invalid() {
    let cases = [
        Bytes::from_static(br#"{"format_version":{"version_string":"9.9"},"storage_api_version":"v1beta","files":[]}"#),
        invalid_path_manifest(MANIFEST_FILENAME),
        invalid_path_manifest("nested.wkspc/file.usd"),
        invalid_path_manifest(""),
        invalid_path_manifest("/absolute"),
        invalid_path_manifest("a//b"),
        invalid_path_manifest("./a"),
        invalid_path_manifest("../a"),
    ];

    for body in cases {
        let inner = MockInner::default();
        inner.set_manifest(MockResult::Ok(body));
        let service = create_service(inner).await;
        assert!(matches!(
            load_state(&service).await.expect("invalid"),
            ManifestState::Invalid
        ));
    }
}

#[tokio::test]
async fn transient_and_access_denied_manifest_errors_propagate_and_are_not_cached() {
    for failure in [
        MockFailure::Internal,
        MockFailure::Unavailable,
        MockFailure::AccessDenied,
    ] {
        let inner = MockInner::default();
        inner.set_manifest(MockResult::Err(failure));
        let service = create_service(inner.clone()).await;

        assert_code(load_state(&service).await, failure.code());
        assert_code(load_state(&service).await, failure.code());
        assert_eq!(inner.manifest_read_count(), 2);
    }
}

#[tokio::test]
async fn loaded_and_invalid_states_are_cached() {
    for result in [
        MockResult::Ok(manifest_json(&[("a.usd", "s3://bucket/a")])),
        MockResult::Ok(Bytes::from_static(b"not-json")),
    ] {
        let inner = MockInner::default();
        inner.set_manifest(result);
        let service = create_service(inner.clone()).await;

        let _ = load_state(&service).await;
        let _ = load_state(&service).await;
        assert_eq!(inner.manifest_read_count(), 1);
    }
}

#[tokio::test]
async fn absent_state_is_not_cached() {
    let inner = MockInner::default();
    inner.set_manifest(MockResult::Err(MockFailure::NotFound));
    let service = create_service(inner.clone()).await;

    // First load sees no manifest.
    assert!(matches!(
        load_state(&service).await.expect("absent"),
        ManifestState::Absent
    ));

    // The manifest appears out-of-band — e.g. written by another service instance, so no local
    // invalidation runs. Because `Absent` is never cached, the next load re-reads the backend and
    // observes the manifest instead of serving a stale `Absent` for the rest of the TTL.
    inner.set_valid_manifest(&[("a.usd", "s3://bucket/a")]);
    assert!(matches!(
        load_state(&service).await.expect("loaded"),
        ManifestState::Loaded(_)
    ));

    // Two backend reads: the `Absent` result was not retained.
    assert_eq!(inner.manifest_read_count(), 2);
}

#[tokio::test]
async fn mutation_preflight_on_manifestless_workspace_rereads_each_call() {
    // A `.wkspc` path with no `.manifest` is a plain folder: mutations delegate to the backend.
    // `Absent` is deliberately not cached (so a manifest written elsewhere is seen at once), and a
    // mutation preflight additionally re-reads `.manifest` directly to confirm `Absent` before
    // delegating. So each mutation RPC into a manifest-less workspace performs two uncoalesced
    // `.manifest` reads. This documents and locks that accepted per-RPC cost (payload bytes never
    // traverse this control-plane service).
    let inner = MockInner::default();
    inner.set_manifest(MockResult::Err(MockFailure::NotFound));
    let service = create_service(inner.clone()).await;

    for _ in 0..3 {
        fo::FileObject::upload_part(
            &service,
            Request::new(fo::UploadPartRequest {
                upload_id: "u".to_string(),
                destination_resource_address: format!("{WORKSPACE}/big.bin"),
                part_number: 0,
                part_count: None,
            }),
        )
        .await
        .expect("upload part delegates");
    }

    // 3 RPCs × (coalesced preflight read + direct `Absent` confirmation) = 6.
    assert_eq!(inner.manifest_read_count(), 6);
}

#[tokio::test]
async fn mutation_preflight_confirms_absent_and_blocks_write_to_activated_entry() {
    // Models the bounded coalescing race: a mutation preflight whose first (coalesced) manifest read
    // predates a concurrent `.manifest` activation observes `Absent`, but the manifest is now active
    // and the target is a read-only entry. The direct confirmation read must catch the activation and
    // reject the write — a stale `Absent` must not permit an irreversible mutation to a manifest entry.
    let inner = MockInner::default();
    inner.set_manifest_sequence(vec![
        // First read (the coalesced preflight fill): pre-activation → Absent.
        MockResult::Err(MockFailure::NotFound),
        // Second read (the direct confirmation): post-activation → the entry is now in the manifest.
        MockResult::Ok(manifest_json(&[("locked.usd", "s3://objects/locked")])),
    ]);
    let service = create_service(inner.clone()).await;

    let result = fo::FileObject::write(
        &service,
        write_request(&format!("{WORKSPACE}/locked.usd")),
        bytes_future(Bytes::from_static(b"payload")),
    )
    .await;

    assert_code(result.map(|_| ()), tonic::Code::PermissionDenied);
    // Two reads: the stale-Absent preflight fill and the authoritative confirmation.
    assert_eq!(inner.manifest_read_count(), 2);
    // The write never reached the backend.
    assert_eq!(inner.call_count("write"), 0);
}

#[tokio::test]
async fn mutation_preflight_confirms_loaded_missing_target_and_blocks_after_rewrite() {
    // Same cohort race, but the preliminary read is a `Loaded` snapshot that predates a *rewrite*:
    // v1 doesn't contain `locked.usd` (so the preliminary decision is Delegate), but v1 was rewritten
    // to v2 which does. A stale `Loaded` missing the target must not permit a write to what is now a
    // read-only entry — the confirmation read must catch the rewrite.
    let inner = MockInner::default();
    inner.set_manifest_sequence(vec![
        MockResult::Ok(manifest_json(&[("other.usd", "s3://objects/other")])),
        MockResult::Ok(manifest_json(&[("locked.usd", "s3://objects/locked")])),
    ]);
    let service = create_service(inner.clone()).await;

    let result = fo::FileObject::write(
        &service,
        write_request(&format!("{WORKSPACE}/locked.usd")),
        bytes_future(Bytes::from_static(b"payload")),
    )
    .await;

    assert_code(result.map(|_| ()), tonic::Code::PermissionDenied);
    assert_eq!(inner.manifest_read_count(), 2);
    assert_eq!(inner.call_count("write"), 0);
}

#[tokio::test]
async fn mutation_preflight_confirms_loaded_missing_target_and_blocks_when_now_invalid() {
    // Would-delegate preliminary (`Loaded` missing the target), but the manifest is now `Invalid`.
    // Fail-closed must win over the stale permit: the confirmation read observes `Invalid` and the
    // write is rejected rather than delegated.
    let inner = MockInner::default();
    inner.set_manifest_sequence(vec![
        MockResult::Ok(manifest_json(&[("other.usd", "s3://objects/other")])),
        MockResult::Ok(Bytes::from_static(b"not-json")),
    ]);
    let service = create_service(inner.clone()).await;

    let result = fo::FileObject::write(
        &service,
        write_request(&format!("{WORKSPACE}/whatever.usd")),
        bytes_future(Bytes::from_static(b"payload")),
    )
    .await;

    assert_code(result.map(|_| ()), tonic::Code::InvalidArgument);
    assert_eq!(inner.manifest_read_count(), 2);
    assert_eq!(inner.call_count("write"), 0);
}

#[tokio::test]
async fn read_from_address_uses_manifest_identity_and_delegates_for_other_paths() {
    let inner = MockInner::default();
    inner.set_valid_manifest(&[("pkg/file.usd", "s3://objects/pkg-file")]);
    inner.set_read_identity(
        "s3://objects/pkg-file",
        MockResult::Ok(Bytes::from_static(b"manifest bytes")),
    );
    let service = create_service(inner.clone()).await;

    let stream = fo::FileObject::read_from_address(&service, read_request(&format!("{WORKSPACE}/pkg/file.usd")))
        .await
        .expect("manifest read");
    assert_eq!(
        collect_read_from_address(stream).await.expect("bytes"),
        Bytes::from_static(b"manifest bytes")
    );
    assert_eq!(inner.calls("read"), vec!["s3://objects/pkg-file"]);

    let stream = fo::FileObject::read_from_address(&service, read_request(&format!("{WORKSPACE}/other.usd")))
        .await
        .expect("delegated read");
    assert_eq!(
        collect_read_from_address(stream).await.expect("bytes"),
        Bytes::from(format!("backend:{WORKSPACE}/other.usd"))
    );
}

#[tokio::test]
async fn read_from_address_canonicalizes_manifest_identity_for_both_api_versions() {
    let loose_identity = "https://objects.example/pkg/file( )+[x].usd?etag=value#fragment";
    let strict_identity = "https://objects.example/pkg/file%28%20%29%2B%5Bx%5D.usd?etag=value#fragment";

    let inner = MockInner::default();
    inner.set_valid_manifest(&[("pkg/file.usd", loose_identity)]);
    inner.set_read_identity(strict_identity, MockResult::Ok(Bytes::from_static(b"manifest bytes")));
    let service = create_service(inner.clone()).await;
    let stream = fo::FileObject::read_from_address(&service, read_request(&format!("{WORKSPACE}/pkg/file.usd")))
        .await
        .expect("v1beta manifest read");
    assert_eq!(
        collect_read_from_address_identity(stream)
            .await
            .expect("v1beta identity"),
        strict_identity
    );
    assert_eq!(inner.calls("read"), vec![strict_identity]);

    let inner = MockInner::default();
    inner.set_valid_manifest(&[("pkg/file.usd", loose_identity)]);
    let service = create_service(inner.clone()).await;
    let stream = foa::FileObject::read_from_address(&service, alpha_read_request(&format!("{WORKSPACE}/pkg/file.usd")))
        .await
        .expect("v1alpha manifest read");
    assert_eq!(
        collect_alpha_read_from_address_identity(stream)
            .await
            .expect("v1alpha identity"),
        strict_identity
    );
    assert_eq!(inner.calls("v1alpha_read"), vec![strict_identity]);
}

#[tokio::test]
async fn absent_workspace_read_delegates_and_manifest_file_read_stays_open_when_invalid() {
    let inner = MockInner::default();
    inner.set_manifest(MockResult::Err(MockFailure::NotFound));
    let service = create_service(inner.clone()).await;

    let stream = fo::FileObject::read_from_address(&service, read_request(&format!("{WORKSPACE}/other.usd")))
        .await
        .expect("absent delegated read");
    assert_eq!(
        collect_read_from_address(stream).await.expect("bytes"),
        Bytes::from(format!("backend:{WORKSPACE}/other.usd"))
    );

    let inner = MockInner::default();
    inner.set_manifest(MockResult::Ok(Bytes::from_static(b"not-json")));
    let service = create_service(inner.clone()).await;
    assert!(matches!(
        load_state(&service).await.expect("invalid"),
        ManifestState::Invalid
    ));

    let stream = fo::FileObject::read_from_address(&service, read_request(MANIFEST))
        .await
        .expect("manifest raw read");
    assert_eq!(
        collect_read_from_address(stream).await.expect("bytes"),
        Bytes::from_static(b"not-json")
    );
}

#[tokio::test]
async fn manifest_identity_not_found_propagates_not_found() {
    let inner = MockInner::default();
    inner.set_valid_manifest(&[("a.usd", "s3://objects/missing")]);
    inner.set_read_identity("s3://objects/missing", MockResult::Err(MockFailure::NotFound));
    let service = create_service(inner).await;

    let result = fo::FileObject::read_from_address(&service, read_request(&format!("{WORKSPACE}/a.usd"))).await;
    assert_code(result, tonic::Code::NotFound);
}

#[tokio::test]
async fn invalid_manifest_rejects_non_manifest_read() {
    let inner = MockInner::default();
    inner.set_manifest(MockResult::Ok(Bytes::from_static(b"not-json")));
    let service = create_service(inner).await;

    let result = fo::FileObject::read_from_address(&service, read_request(&format!("{WORKSPACE}/a.usd"))).await;
    assert_code(result, tonic::Code::InvalidArgument);
}

#[tokio::test]
async fn stat_manifest_entries_are_synthesized_without_backend_stat() {
    let inner = MockInner::default();
    inner.set_valid_manifest(&[(
        "a.usd",
        "s3://objects/a?content-length=42&last-modified=2024-01-01T00:00:00Z",
    )]);
    let service = create_service(inner.clone()).await;

    let response = fo::FileObject::stat(&service, stat_request(&format!("{WORKSPACE}/a.usd")))
        .await
        .expect("manifest stat");
    let info = response.resource_info.expect("resource info");
    assert_eq!(
        info.resource_identity.expect("identity").encoded_identity,
        "s3://objects/a?content-length=42&last-modified=2024-01-01T00:00:00Z"
    );
    assert_eq!(info.metadata.expect("metadata").data_object_size, Some(42));
    assert_eq!(inner.call_count("stat"), 0);
}

#[tokio::test]
async fn stat_manifest_resource_info_variant_is_synthesized_without_backend_stat() {
    let inner = MockInner::default();
    inner.set_manifest(MockResult::Ok(manifest_info_json("a.usd", "s3://objects/a", 77)));
    let service = create_service(inner.clone()).await;

    let response = fo::FileObject::stat(&service, stat_request(&format!("{WORKSPACE}/a.usd")))
        .await
        .expect("manifest stat");
    assert_eq!(
        response
            .resource_info
            .expect("resource info")
            .metadata
            .expect("metadata")
            .data_object_size,
        Some(77)
    );
    assert_eq!(inner.call_count("stat"), 0);
}

#[tokio::test]
async fn manifest_stat_list_stat_and_versions_canonicalize_source_identity() {
    let loose_identity = "https://objects.example/source( )+[x].usd?etag=value";
    let strict_identity = "https://objects.example/source%28%20%29%2B%5Bx%5D.usd?etag=value";
    let inner = MockInner::default();
    inner.set_valid_manifest(&[("entry.usd", loose_identity)]);
    inner.set_list_stat_pages(None, vec![MockStreamItem::Err(MockFailure::NotFound)]);
    let service = create_service(inner.clone()).await;

    let stat = fo::FileObject::stat(&service, stat_request(&format!("{WORKSPACE}/entry.usd")))
        .await
        .expect("stat");
    assert_eq!(
        stat.resource_info
            .expect("stat resource info")
            .resource_identity
            .expect("stat identity")
            .encoded_identity,
        strict_identity
    );

    let mut list_stat = ff::FileFolder::list_stat(&service, list_stat_request(WORKSPACE), StreamingOptions::Streamed)
        .await
        .expect("list stat");
    let mut list_identities = Vec::new();
    while let Some(page) = list_stat.next().await {
        list_identities.extend(page.expect("list stat page").response.entries.into_iter().map(|entry| {
            entry
                .resource_info
                .expect("list resource info")
                .resource_identity
                .expect("list identity")
                .encoded_identity
        }));
    }
    assert_eq!(list_identities, vec![strict_identity]);

    let versions = ver::Versioning::enumerate_versions(
        &service,
        versions_request(&format!("{WORKSPACE}/entry.usd")),
        StreamingOptions::Streamed,
        None,
    )
    .await
    .expect("versions");
    assert_eq!(
        collect_versions(versions).await.expect("version identities"),
        vec![strict_identity]
    );
    assert_eq!(inner.call_count("stat"), 0);
    assert_eq!(inner.call_count("enumerate_versions"), 0);
}

#[tokio::test]
async fn stat_non_manifest_delegates_and_manifest_file_stat_stays_open_when_invalid() {
    let inner = MockInner::default();
    inner.set_valid_manifest(&[("a.usd", "s3://objects/a")]);
    inner.set_stat(
        &format!("{WORKSPACE}/other.usd"),
        MockResult::Ok(stat_response("backend-id", 5)),
    );
    let service = create_service(inner.clone()).await;

    let response = fo::FileObject::stat(&service, stat_request(&format!("{WORKSPACE}/other.usd")))
        .await
        .expect("delegated stat");
    assert_eq!(
        response
            .resource_info
            .expect("resource info")
            .resource_identity
            .expect("identity")
            .encoded_identity,
        "backend-id"
    );

    let inner = MockInner::default();
    inner.set_manifest(MockResult::Ok(Bytes::from_static(b"not-json")));
    inner.set_stat(MANIFEST, MockResult::Ok(stat_response("manifest-id", 9)));
    let service = create_service(inner.clone()).await;
    assert_code(
        fo::FileObject::stat(&service, stat_request(&format!("{WORKSPACE}/a.usd"))).await,
        tonic::Code::InvalidArgument,
    );
    assert!(fo::FileObject::stat(&service, stat_request(MANIFEST)).await.is_ok());
}

#[tokio::test]
async fn write_and_delete_classification_respects_overlay_and_repair_path() {
    let inner = MockInner::default();
    inner.set_valid_manifest(&[("locked.usd", "s3://objects/locked")]);
    let service = create_service(inner.clone()).await;

    assert_code(
        fo::FileObject::write(
            &service,
            write_request(&format!("{WORKSPACE}/locked.usd")),
            bytes_future(Bytes::new()),
        )
        .await,
        tonic::Code::PermissionDenied,
    );
    assert_eq!(inner.call_count("write"), 0);

    let stream = fo::FileObject::write(
        &service,
        write_request(&format!("{WORKSPACE}/free.usd")),
        bytes_future(Bytes::from_static(b"data")),
    )
    .await
    .expect("delegated write");
    let _ = stream.collect::<Vec<_>>().await;
    assert_eq!(inner.calls("write"), vec![format!("{WORKSPACE}/free.usd")]);

    assert_code(
        fo::FileObject::delete(&service, delete_request(&format!("{WORKSPACE}/locked.usd"))).await,
        tonic::Code::PermissionDenied,
    );
    assert!(
        fo::FileObject::delete(&service, delete_request(&format!("{WORKSPACE}/free.usd")))
            .await
            .is_ok()
    );
    assert!(fo::FileObject::delete(&service, delete_request(MANIFEST)).await.is_ok());
    assert!(inner.calls("delete").contains(&MANIFEST.to_string()));
}

#[tokio::test]
async fn is_read_only_reflects_manifest_overlay() {
    let inner = MockInner::default();
    inner.set_valid_manifest(&[("pkg/model.usd", "s3://objects/model")]);
    let service = create_service(inner.clone()).await;

    // An in-manifest object path is served from the immutable manifest and its
    // mutation is rejected, so IsReadOnly reports read-only without consulting
    // the physical backend.
    let response = foa::FileObject::is_read_only(&service, is_read_only_request(&format!("{WORKSPACE}/pkg/model.usd")))
        .await
        .expect("is_read_only for manifest entry");
    assert!(response.is_read_only);
    assert_eq!(inner.call_count("is_read_only"), 0);

    // A workspace path with no manifest entry carries no overlay read-only
    // status: the answer comes from the physical backend.
    let response = foa::FileObject::is_read_only(&service, is_read_only_request(&format!("{WORKSPACE}/free.usd")))
        .await
        .expect("is_read_only delegated");
    assert!(!response.is_read_only);
    assert_eq!(inner.calls("is_read_only"), vec![format!("{WORKSPACE}/free.usd")]);
}

#[tokio::test]
async fn invalid_manifest_rejects_mutations_except_manifest_file() {
    let inner = MockInner::default();
    inner.set_manifest(MockResult::Ok(Bytes::from_static(b"not-json")));
    let service = create_service(inner.clone()).await;

    assert_code(
        fo::FileObject::write(
            &service,
            write_request(&format!("{WORKSPACE}/free.usd")),
            bytes_future(Bytes::new()),
        )
        .await,
        tonic::Code::InvalidArgument,
    );

    let mut stream = fo::FileObject::write(&service, write_request(MANIFEST), bytes_future(manifest_json(&[])))
        .await
        .expect("manifest write");
    assert!(stream.next().await.transpose().expect("write response").is_some());
}

#[tokio::test]
async fn manifest_write_and_create_invalidate_cached_state() {
    let inner = MockInner::default();
    inner.set_valid_manifest(&[("old.usd", "s3://objects/old")]);
    inner.set_read_identity("s3://objects/old", MockResult::Ok(Bytes::from_static(b"old")));
    inner.set_read_identity("s3://objects/new", MockResult::Ok(Bytes::from_static(b"new")));
    let service = create_service(inner.clone()).await;

    let stream = fo::FileObject::read_from_address(&service, read_request(&format!("{WORKSPACE}/old.usd")))
        .await
        .expect("old read");
    assert_eq!(
        collect_read_from_address(stream).await.expect("old bytes"),
        Bytes::from_static(b"old")
    );
    assert_eq!(inner.manifest_read_count(), 1);

    let mut write_stream = fo::FileObject::write(
        &service,
        write_request(MANIFEST),
        bytes_future(manifest_json(&[("new.usd", "s3://objects/new")])),
    )
    .await
    .expect("manifest write");
    assert!(write_stream
        .next()
        .await
        .transpose()
        .expect("terminal response")
        .is_some());

    let stream = fo::FileObject::read_from_address(&service, read_request(&format!("{WORKSPACE}/new.usd")))
        .await
        .expect("new read");
    assert_eq!(
        collect_read_from_address(stream).await.expect("new bytes"),
        Bytes::from_static(b"new")
    );
    assert_eq!(inner.manifest_read_count(), 2);

    let inner = MockInner::default();
    inner.set_manifest(MockResult::Err(MockFailure::NotFound));
    inner.set_read_identity("s3://objects/created", MockResult::Ok(Bytes::from_static(b"created")));
    let service = create_service(inner.clone()).await;
    let _ = fo::FileObject::stat(&service, stat_request(&format!("{WORKSPACE}/created.usd"))).await;
    assert_eq!(inner.manifest_read_count(), 1);
    let mut write_stream = fo::FileObject::write(
        &service,
        write_request(MANIFEST),
        bytes_future(manifest_json(&[("created.usd", "s3://objects/created")])),
    )
    .await
    .expect("manifest create");
    assert!(write_stream
        .next()
        .await
        .transpose()
        .expect("terminal response")
        .is_some());
    assert!(
        fo::FileObject::stat(&service, stat_request(&format!("{WORKSPACE}/created.usd")))
            .await
            .is_ok()
    );
    assert_eq!(inner.manifest_read_count(), 2);
}

#[tokio::test]
async fn manifest_write_invalidates_when_terminal_stream_item_is_seen() {
    let inner = MockInner::default();
    inner.set_valid_manifest(&[("old.usd", "s3://objects/old")]);
    let service = create_service(inner.clone()).await;
    assert!(load_state(&service).await.is_ok());
    assert_eq!(inner.manifest_read_count(), 1);

    let mut write_stream = fo::FileObject::write(
        &service,
        write_request(MANIFEST),
        bytes_future(manifest_json(&[("new.usd", "s3://objects/new")])),
    )
    .await
    .expect("manifest write");
    let _ = write_stream.next().await.transpose().expect("terminal response");
    assert!(
        fo::FileObject::stat(&service, stat_request(&format!("{WORKSPACE}/new.usd")))
            .await
            .is_ok()
    );
    assert_eq!(inner.manifest_read_count(), 2);
}

#[tokio::test]
async fn complete_uploads_invalidate_but_prepare_or_abort_calls_do_not() {
    let inner = MockInner::default();
    inner.set_valid_manifest(&[("a.usd", "s3://objects/a")]);
    let service = create_service(inner.clone()).await;
    assert!(load_state(&service).await.is_ok());
    assert_eq!(inner.manifest_read_count(), 1);

    fo::FileObject::abort_multipart_upload(
        &service,
        Request::new(fo::AbortMultipartUploadRequest {
            upload_id: "u".to_string(),
            destination_resource_address: MANIFEST.to_string(),
        }),
    )
    .await
    .expect("abort");
    fo::FileObject::upload_part(
        &service,
        Request::new(fo::UploadPartRequest {
            upload_id: "u".to_string(),
            destination_resource_address: MANIFEST.to_string(),
            part_number: 0,
            part_count: None,
        }),
    )
    .await
    .expect("upload part");
    fo::FileObject::fetch_write_type_info(
        &service,
        Request::new(fo::FetchWriteTypeInfoRequest {
            destination_resource_address: MANIFEST.to_string(),
        }),
    )
    .await
    .expect("fetch write info");
    assert!(load_state(&service).await.is_ok());
    assert_eq!(inner.manifest_read_count(), 1);

    fo::FileObject::complete_multipart_upload(
        &service,
        Request::new(fo::CompleteMultipartUploadRequest {
            upload_id: "u".to_string(),
            destination_resource_address: MANIFEST.to_string(),
            parts: vec![],
        }),
    )
    .await
    .expect("complete multipart");
    assert!(load_state(&service).await.is_ok());
    assert_eq!(inner.manifest_read_count(), 2);

    fo::FileObject::complete_redirect_upload(
        &service,
        Request::new(fo::CompleteRedirectUploadRequest {
            destination_resource_address: MANIFEST.to_string(),
            additional_headers: vec![],
        }),
    )
    .await
    .expect("complete redirect");
    assert!(load_state(&service).await.is_ok());
    assert_eq!(inner.manifest_read_count(), 3);
}

#[tokio::test]
async fn listing_backend_first_manifest_wins_and_manifest_file_visible() {
    let inner = MockInner::default();
    inner.set_valid_manifest(&[
        ("x.usd", "s3://objects/manifest-x"),
        ("z.usd", "s3://objects/manifest-z"),
    ]);
    inner.set_list_pages(
        None,
        vec![MockStreamItem::Ok(ListPage {
            folders: vec![],
            files: vec![
                MANIFEST.to_string(),
                format!("{WORKSPACE}/backend.usd"),
                format!("{WORKSPACE}/x.usd"),
            ],
            continue_token: None,
        })],
    );
    let service = create_service(inner).await;

    let stream = ff::FileFolder::list(&service, list_request(WORKSPACE), StreamingOptions::Streamed)
        .await
        .expect("list");
    assert_eq!(
        collect_list(stream).await.expect("items"),
        vec![
            format!("file:{MANIFEST}"),
            format!("file:{WORKSPACE}/backend.usd"),
            format!("file:{WORKSPACE}/x.usd"),
            format!("file:{WORKSPACE}/z.usd"),
        ]
    );
}

#[tokio::test]
async fn listing_dedups_file_folder_and_folder_folder_collisions() {
    let inner = MockInner::default();
    inner.set_valid_manifest(&[
        ("docs/readme.usd", "s3://objects/docs-readme"),
        ("file-collides-with-folder", "s3://objects/file"),
    ]);
    inner.set_list_pages(
        None,
        vec![MockStreamItem::Ok(ListPage {
            folders: vec![
                format!("{WORKSPACE}/docs/"),
                format!("{WORKSPACE}/file-collides-with-folder/"),
            ],
            files: vec![],
            continue_token: None,
        })],
    );
    let service = create_service(inner).await;

    let stream = ff::FileFolder::list(&service, list_request(WORKSPACE), StreamingOptions::Streamed)
        .await
        .expect("list");
    assert_eq!(
        collect_list(stream).await.expect("items"),
        vec![
            format!("folder:{WORKSPACE}/docs/"),
            format!("file:{WORKSPACE}/file-collides-with-folder"),
        ]
    );
}

#[tokio::test]
async fn absent_listing_delegates_and_invalid_listing_rejects() {
    let inner = MockInner::default();
    inner.set_manifest(MockResult::Err(MockFailure::NotFound));
    inner.set_list_pages(
        None,
        vec![MockStreamItem::Ok(ListPage {
            folders: vec![format!("{WORKSPACE}/real/")],
            files: vec![format!("{WORKSPACE}/real.usd")],
            continue_token: None,
        })],
    );
    let service = create_service(inner).await;
    let stream = ff::FileFolder::list(&service, list_request(WORKSPACE), StreamingOptions::Streamed)
        .await
        .expect("list");
    assert_eq!(
        collect_list(stream).await.expect("items"),
        vec![
            format!("folder:{WORKSPACE}/real/"),
            format!("file:{WORKSPACE}/real.usd")
        ]
    );

    let inner = MockInner::default();
    inner.set_manifest(MockResult::Ok(Bytes::from_static(b"not-json")));
    let service = create_service(inner).await;
    assert_code(
        ff::FileFolder::list(&service, list_request(WORKSPACE), StreamingOptions::Streamed).await,
        tonic::Code::InvalidArgument,
    );
}

#[tokio::test]
async fn list_and_list_stat_backend_not_found_falls_through_to_manifest() {
    let inner = MockInner::default();
    inner.set_valid_manifest(&[("pkg/file.usd", "s3://objects/pkg-file")]);
    inner.set_list_pages(None, vec![MockStreamItem::Err(MockFailure::NotFound)]);
    inner.set_list_stat_pages(None, vec![MockStreamItem::Err(MockFailure::NotFound)]);
    let service = create_service(inner).await;

    let stream = ff::FileFolder::list(
        &service,
        list_request(&format!("{WORKSPACE}/pkg")),
        StreamingOptions::Streamed,
    )
    .await
    .expect("list");
    assert_eq!(
        collect_list(stream).await.expect("items"),
        vec![format!("file:{WORKSPACE}/pkg/file.usd")]
    );

    let stream = ff::FileFolder::list_stat(
        &service,
        list_stat_request(&format!("{WORKSPACE}/pkg")),
        StreamingOptions::Streamed,
    )
    .await
    .expect("list stat");
    assert_eq!(
        collect_list_stat(stream).await.expect("items"),
        vec![format!("file:{WORKSPACE}/pkg/file.usd")]
    );
}

#[tokio::test]
async fn manifest_list_and_list_stat_use_strict_workspace_and_child_paths() {
    const LOOSE_WORKSPACE: &str = "https://bucket.example/parent()/work +[x].wkspc";
    const STRICT_WORKSPACE: &str = "https://bucket.example/parent%28%29/work%20%2B%5Bx%5D.wkspc";
    const STRICT_FOLDER: &str = "folder%28%20%29%2B%5Bx%5D";
    const STRICT_FILE: &str = "top%28%20%29%2B%5Bx%5D.usd";

    let inner = MockInner::default();
    inner.set_valid_manifest(&[
        ("top( )+[x].usd", "https://objects.example/top"),
        ("folder( )+[x]/child.usd", "https://objects.example/child"),
    ]);
    inner.set_list_pages(None, vec![MockStreamItem::Err(MockFailure::NotFound)]);
    inner.set_list_stat_pages(None, vec![MockStreamItem::Err(MockFailure::NotFound)]);
    let service = create_service(inner).await;

    let list = ff::FileFolder::list(&service, list_request(LOOSE_WORKSPACE), StreamingOptions::Streamed)
        .await
        .expect("list");
    assert_eq!(
        collect_list(list).await.expect("list items"),
        vec![
            format!("folder:{STRICT_WORKSPACE}/{STRICT_FOLDER}/"),
            format!("file:{STRICT_WORKSPACE}/{STRICT_FILE}"),
        ]
    );

    let list_stat = ff::FileFolder::list_stat(&service, list_stat_request(LOOSE_WORKSPACE), StreamingOptions::Streamed)
        .await
        .expect("list stat");
    assert_eq!(
        collect_list_stat(list_stat).await.expect("list stat items"),
        vec![
            format!("folder:{STRICT_WORKSPACE}/{STRICT_FOLDER}/"),
            format!("file:{STRICT_WORKSPACE}/{STRICT_FILE}"),
        ]
    );
}

#[tokio::test]
async fn cross_source_list_pagination_has_no_duplicates_or_skips() {
    let inner = MockInner::default();
    inner.set_valid_manifest(&[
        ("a-manifest.usd", "s3://objects/a"),
        ("b-manifest.usd", "s3://objects/b"),
        ("z-manifest.usd", "s3://objects/z"),
    ]);
    inner.set_list_pages(
        None,
        vec![MockStreamItem::Ok(ListPage {
            folders: vec![],
            files: vec![format!("{WORKSPACE}/backend-1.usd")],
            continue_token: Some("backend-2".to_string()),
        })],
    );
    inner.set_list_pages(
        Some("backend-2"),
        vec![MockStreamItem::Ok(ListPage {
            folders: vec![],
            files: vec![format!("{WORKSPACE}/backend-2.usd")],
            continue_token: None,
        })],
    );
    let service = create_service(inner).await;

    let mut token = None;
    let mut items = Vec::new();
    loop {
        let mut stream = ff::FileFolder::list(
            &service,
            list_request(WORKSPACE),
            StreamingOptions::Paginated(Pagination {
                continuation_handle: token.clone(),
                max_page_size: Some(1),
            }),
        )
        .await
        .expect("list page");
        let page = stream.next().await.expect("page").expect("page ok");
        items.extend(page.response.sub_resource_addresses);
        token = page.continue_token;
        if token.is_none() {
            break;
        }
    }

    assert_eq!(
        items,
        vec![
            format!("{WORKSPACE}/backend-1.usd"),
            format!("{WORKSPACE}/backend-2.usd"),
            format!("{WORKSPACE}/a-manifest.usd"),
            format!("{WORKSPACE}/b-manifest.usd"),
            format!("{WORKSPACE}/z-manifest.usd"),
        ]
    );
}

#[tokio::test]
async fn enumerate_recursively_merges_flat_manifest_entries_and_dedups() {
    let inner = MockInner::default();
    inner.set_valid_manifest(&[
        ("pkg/a.usd", "s3://objects/manifest-a"),
        ("pkg/b.usd", "s3://objects/manifest-b"),
    ]);
    inner.set_enumerate_pages(
        None,
        vec![MockStreamItem::Ok(EnumeratePage {
            files: vec![format!("{WORKSPACE}/pkg/a.usd"), format!("{WORKSPACE}/pkg/backend.usd")],
            continue_token: None,
        })],
    );
    let service = create_service(inner).await;

    let stream = fo::FileObject::enumerate(
        &service,
        enumerate_request(&format!("{WORKSPACE}/pkg")),
        StreamingOptions::Streamed,
    )
    .await
    .expect("enumerate");
    assert_eq!(
        collect_enumerate(stream).await.expect("items"),
        vec![
            format!("{WORKSPACE}/pkg/backend.usd"),
            "s3://objects/manifest-a".to_string(),
            "s3://objects/manifest-b".to_string(),
        ]
    );
}

#[tokio::test]
async fn enumerate_shadows_strict_backend_collision_for_loose_workspace_request() {
    const LOOSE_WORKSPACE: &str = "https://bucket.example/work().wkspc";
    const STRICT_WORKSPACE: &str = "https://bucket.example/work%28%29.wkspc";

    let inner = MockInner::default();
    inner.set_valid_manifest(&[("pkg/a(1).usd", "https://objects.example/manifest().usd?etag=value")]);
    inner.set_enumerate_pages(
        None,
        vec![MockStreamItem::Ok(EnumeratePage {
            files: vec![
                format!("{STRICT_WORKSPACE}/pkg/a%281%29.usd"),
                format!("{STRICT_WORKSPACE}/pkg/backend%281%29.usd"),
            ],
            continue_token: None,
        })],
    );
    let service = create_service(inner).await;

    let stream = fo::FileObject::enumerate(
        &service,
        enumerate_request(&format!("{LOOSE_WORKSPACE}/pkg")),
        StreamingOptions::Streamed,
    )
    .await
    .expect("enumerate");
    assert_eq!(
        collect_enumerate(stream).await.expect("items"),
        vec![
            format!("{STRICT_WORKSPACE}/pkg/backend%281%29.usd"),
            "https://objects.example/manifest%28%29.usd".to_string(),
        ]
    );
}

#[tokio::test]
async fn enumerate_not_found_falls_through_to_manifest_and_absent_or_invalid_behave() {
    let inner = MockInner::default();
    inner.set_valid_manifest(&[("pkg/a.usd", "s3://objects/manifest-a")]);
    inner.set_enumerate_pages(None, vec![MockStreamItem::Err(MockFailure::NotFound)]);
    let service = create_service(inner).await;
    let stream = fo::FileObject::enumerate(
        &service,
        enumerate_request(&format!("{WORKSPACE}/pkg")),
        StreamingOptions::Streamed,
    )
    .await
    .expect("enumerate");
    assert_eq!(
        collect_enumerate(stream).await.expect("items"),
        vec!["s3://objects/manifest-a"]
    );

    let inner = MockInner::default();
    inner.set_manifest(MockResult::Err(MockFailure::NotFound));
    inner.set_enumerate_pages(
        None,
        vec![MockStreamItem::Ok(EnumeratePage {
            files: vec![format!("{WORKSPACE}/backend.usd")],
            continue_token: None,
        })],
    );
    let service = create_service(inner).await;
    let stream = fo::FileObject::enumerate(&service, enumerate_request(WORKSPACE), StreamingOptions::Streamed)
        .await
        .expect("absent enumerate");
    assert_eq!(
        collect_enumerate(stream).await.expect("items"),
        vec![format!("{WORKSPACE}/backend.usd")]
    );

    let inner = MockInner::default();
    inner.set_manifest(MockResult::Ok(Bytes::from_static(b"not-json")));
    let service = create_service(inner).await;
    assert_code(
        fo::FileObject::enumerate(&service, enumerate_request(WORKSPACE), StreamingOptions::Streamed).await,
        tonic::Code::InvalidArgument,
    );
}

#[tokio::test]
async fn enumerate_versions_resolves_manifest_absent_delegates_invalid_rejects() {
    let inner = MockInner::default();
    inner.set_valid_manifest(&[("a.usd", "s3://objects/a")]);
    inner.set_version_pages(
        None,
        vec![MockStreamItem::Ok(VersionPage {
            identities: vec!["s3://objects/backend".to_string()],
            continue_token: None,
            version_id_token: None,
        })],
    );
    let service = create_service(inner.clone()).await;
    let stream = ver::Versioning::enumerate_versions(
        &service,
        versions_request(&format!("{WORKSPACE}/a.usd")),
        StreamingOptions::Streamed,
        None,
    )
    .await
    .expect("manifest versions");
    assert_eq!(
        collect_versions(stream).await.expect("versions"),
        vec!["s3://objects/a"]
    );
    assert_eq!(inner.call_count("enumerate_versions"), 0);

    let inner = MockInner::default();
    inner.set_manifest(MockResult::Err(MockFailure::NotFound));
    inner.set_version_pages(
        None,
        vec![MockStreamItem::Ok(VersionPage {
            identities: vec!["s3://objects/backend".to_string()],
            continue_token: None,
            version_id_token: None,
        })],
    );
    let service = create_service(inner).await;
    let stream = ver::Versioning::enumerate_versions(
        &service,
        versions_request(&format!("{WORKSPACE}/a.usd")),
        StreamingOptions::Streamed,
        None,
    )
    .await
    .expect("absent versions");
    assert_eq!(
        collect_versions(stream).await.expect("versions"),
        vec!["s3://objects/backend"]
    );

    let inner = MockInner::default();
    inner.set_manifest(MockResult::Ok(Bytes::from_static(b"not-json")));
    let service = create_service(inner).await;
    assert_code(
        ver::Versioning::enumerate_versions(
            &service,
            versions_request(&format!("{WORKSPACE}/a.usd")),
            StreamingOptions::Streamed,
            None,
        )
        .await,
        tonic::Code::InvalidArgument,
    );
}

#[tokio::test]
async fn is_versioned_reports_true_for_manifest_backed_entry() {
    // A workspace path whose manifest contains the object: the manifest layer
    // synthesizes a version for it regardless of the (non-versioned) physical
    // backend, so it must report versioning as enabled and NOT delegate to the
    // backend (which returns false).
    let inner = MockInner::default();
    inner.set_valid_manifest(&[("a.usd", "s3://objects/a")]);
    let service = create_service(inner.clone()).await;
    assert!(
        vera::Versioning::is_versioned(&service, is_versioned_request(&format!("{WORKSPACE}/a.usd")))
            .await
            .expect("is_versioned")
            .is_versioned,
        "manifest-backed entry should report versioning enabled"
    );
    assert_eq!(
        inner.call_count("is_versioned"),
        0,
        "must not delegate to the physical backend for a manifest-backed entry"
    );
}

#[tokio::test]
async fn is_versioned_delegates_for_non_workspace_and_missing_entry() {
    // A plain (non-workspace) address: delegate to the physical backend.
    let inner = MockInner::default();
    let service = create_service(inner.clone()).await;
    assert!(
        !vera::Versioning::is_versioned(&service, is_versioned_request("s3://bucket/plain/file.usd"))
            .await
            .expect("is_versioned")
            .is_versioned,
        "non-workspace address should delegate to the backend (false)"
    );
    assert_eq!(inner.call_count("is_versioned"), 1, "should delegate to the backend");

    // A workspace path whose manifest does NOT contain the object: delegate.
    let inner = MockInner::default();
    inner.set_valid_manifest(&[("other.usd", "s3://objects/other")]);
    let service = create_service(inner.clone()).await;
    assert!(
        !vera::Versioning::is_versioned(&service, is_versioned_request(&format!("{WORKSPACE}/a.usd")))
            .await
            .expect("is_versioned")
            .is_versioned,
        "workspace path missing from manifest should delegate to the backend (false)"
    );
    assert_eq!(inner.call_count("is_versioned"), 1, "should delegate to the backend");
}

#[tokio::test]
async fn is_versioned_rejects_invalid_manifest_and_delegates_absent_with_caller_context() {
    // An invalid manifest fails closed, matching enumerate_versions and is_read_only.
    let inner = MockInner::default();
    inner.set_manifest(MockResult::Ok(Bytes::from_static(b"not-json")));
    let service = create_service(inner.clone()).await;
    assert_code(
        vera::Versioning::is_versioned(&service, is_versioned_request(&format!("{WORKSPACE}/a.usd"))).await,
        tonic::Code::InvalidArgument,
    );
    assert_eq!(inner.call_count("is_versioned"), 0);

    // No manifest at the workspace: the physical backend answers, and it receives the caller's
    // original request, metadata and extensions included.
    let inner = MockInner::default();
    let service = create_service(inner.clone()).await;
    let address = format!("{WORKSPACE}/a.usd");
    let response =
        vera::Versioning::is_versioned(&service, with_caller_context(is_versioned_request(&address), "absent"))
            .await
            .expect("is_versioned delegated");
    assert!(!response.is_versioned);
    let delegated = inner.contexts("is_versioned");
    assert_eq!(delegated.len(), 1);
    assert_eq!(delegated[0].target, address);
    assert_eq!(delegated[0].authorization(), Some("Bearer absent"));
    assert_eq!(delegated[0].additional_headers, caller_headers("absent"));
    assert_eq!(delegated[0].marker, Some(CallerMarker("absent".to_string())));
}

#[tokio::test]
async fn is_versioned_cold_read_forwards_caller_metadata_and_extensions() {
    // The cold-cache `.manifest` read goes through the resolver as the caller: the caller's
    // metadata (credentials) and extensions (AdditionalHeaders) ride along, plus NoCache.
    let inner = MockInner::default();
    let resolver = MockInner::default();
    resolver.set_valid_manifest(&[("entry.usd", "s3://objects/entry")]);
    let service = create_service_with_resolver(inner.clone(), resolver.clone()).await;

    let response = vera::Versioning::is_versioned(
        &service,
        with_caller_context(is_versioned_request(&format!("{WORKSPACE}/entry.usd")), "cold"),
    )
    .await
    .expect("is_versioned");
    assert!(response.is_versioned);

    let reads = resolver.contexts("read_from_address");
    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0].target, MANIFEST);
    assert_eq!(reads[0].authorization(), Some("Bearer cold"));
    assert_eq!(reads[0].additional_headers, caller_headers("cold"));
    assert_eq!(reads[0].marker, Some(CallerMarker("cold".to_string())));
    assert!(reads[0].nocache);
    assert!(inner.calls("read_from_address").is_empty());
    assert_eq!(inner.call_count("is_versioned"), 0);
}

#[tokio::test]
async fn is_versioned_warm_cache_skips_resolver() {
    // The manifest cache is keyed by workspace path, not by principal: once one caller has
    // loaded the manifest, a later caller with different (here: no) credentials is answered
    // from the cache without a resolver read. This pins down the documented principal-blind
    // warm answer.
    let inner = MockInner::default();
    let resolver = MockInner::default();
    resolver.set_valid_manifest(&[("entry.usd", "s3://objects/entry")]);
    let service = create_service_with_resolver(inner.clone(), resolver.clone()).await;
    let entry = format!("{WORKSPACE}/entry.usd");

    let cold = vera::Versioning::is_versioned(&service, with_caller_context(is_versioned_request(&entry), "first"))
        .await
        .expect("cold is_versioned");
    assert!(cold.is_versioned);
    assert_eq!(resolver.manifest_read_count(), 1);

    let warm = vera::Versioning::is_versioned(&service, is_versioned_request(&entry))
        .await
        .expect("warm is_versioned");
    assert!(warm.is_versioned);
    assert_eq!(
        resolver.manifest_read_count(),
        1,
        "a warm answer must not re-read the manifest"
    );
    assert_eq!(inner.call_count("is_versioned"), 0);
}

#[tokio::test]
async fn is_versioned_resolver_error_propagates_and_is_not_cached() {
    // A caller the resolver chain refuses gets the refusal, not a boolean; the backend is not
    // consulted as a fallback; and the error is not cached, so the next call reads again.
    let inner = MockInner::default();
    let resolver = MockInner::default();
    resolver.set_manifest(MockResult::Err(MockFailure::AccessDenied));
    let service = create_service_with_resolver(inner.clone(), resolver.clone()).await;
    let entry = format!("{WORKSPACE}/entry.usd");

    for _ in 0..2 {
        assert_code(
            vera::Versioning::is_versioned(&service, is_versioned_request(&entry)).await,
            tonic::Code::PermissionDenied,
        );
    }
    assert_eq!(resolver.manifest_read_count(), 2, "the error must not be cached");
    assert_eq!(inner.call_count("is_versioned"), 0);
}

#[tokio::test]
async fn is_read_only_cold_read_forwards_caller_metadata_and_extensions() {
    // Same contract as is_versioned: the cold `.manifest` read carries the caller's context.
    let inner = MockInner::default();
    let resolver = MockInner::default();
    resolver.set_valid_manifest(&[("entry.usd", "s3://objects/entry")]);
    let service = create_service_with_resolver(inner.clone(), resolver.clone()).await;

    let response = foa::FileObject::is_read_only(
        &service,
        with_caller_context(is_read_only_request(&format!("{WORKSPACE}/entry.usd")), "cold"),
    )
    .await
    .expect("is_read_only");
    assert!(response.is_read_only);

    let reads = resolver.contexts("read_from_address");
    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0].target, MANIFEST);
    assert_eq!(reads[0].authorization(), Some("Bearer cold"));
    assert_eq!(reads[0].additional_headers, caller_headers("cold"));
    assert_eq!(reads[0].marker, Some(CallerMarker("cold".to_string())));
    assert!(reads[0].nocache);
    assert_eq!(inner.call_count("is_read_only"), 0);
}

#[tokio::test]
async fn v1alpha_move_manifest_invalidates_source_and_destination_workspaces() {
    let source_workspace = "s3://bucket/source.wkspc";
    let destination_workspace = "s3://bucket/destination.wkspc";
    let source_manifest = format!("{source_workspace}/.manifest");
    let destination_manifest = format!("{destination_workspace}/.manifest");
    let inner = MockInner::default();
    inner.set_manifest_for(
        &source_manifest,
        MockResult::Ok(manifest_json(&[("source.usd", "s3://objects/source")])),
    );
    inner.set_manifest_for(
        &destination_manifest,
        MockResult::Ok(manifest_json(&[("destination.usd", "s3://objects/destination")])),
    );
    let service = create_service(inner.clone()).await;

    assert!(matches!(
        load_alpha_state_at(&service, source_workspace).await.expect("source"),
        ManifestState::Loaded(_)
    ));
    assert!(matches!(
        load_alpha_state_at(&service, destination_workspace)
            .await
            .expect("destination"),
        ManifestState::Loaded(_)
    ));
    assert_eq!(inner.manifest_read_count(), 2);

    foa::FileObject::move_object(&service, alpha_move_request(&source_manifest, &destination_manifest))
        .await
        .expect("move manifest");

    assert!(matches!(
        load_alpha_state_at(&service, source_workspace)
            .await
            .expect("source reloaded"),
        ManifestState::Absent
    ));
    assert!(matches!(
        load_alpha_state_at(&service, destination_workspace)
            .await
            .expect("destination reloaded"),
        ManifestState::Loaded(_)
    ));
    assert_eq!(inner.manifest_read_count(), 4);
    assert_eq!(
        inner.calls("move_object"),
        vec![format!("{source_manifest}->{destination_manifest}")]
    );
}

#[tokio::test]
async fn v1alpha_copy_and_move_to_manifest_entries_are_rejected() {
    let inner = MockInner::default();
    inner.set_valid_manifest(&[("locked.usd", "s3://objects/locked")]);
    let service = create_service(inner.clone()).await;
    let locked_path = format!("{WORKSPACE}/locked.usd");

    assert_code(
        foa::FileObject::copy(&service, alpha_copy_request(&locked_path)).await,
        tonic::Code::PermissionDenied,
    );
    assert_code(
        foa::FileObject::move_object(
            &service,
            alpha_move_request(&format!("{WORKSPACE}/free.usd"), &locked_path),
        )
        .await,
        tonic::Code::PermissionDenied,
    );
    assert_eq!(inner.call_count("copy"), 0);
    assert_eq!(inner.call_count("move_object"), 0);
}

// ---- v1alpha overlay coverage. The v1alpha read/stat/list/list_stat/enumerate path is a parallel
// fork of v1beta and was previously exercised only via copy/move; these mirror the v1beta behavior
// tests to guard the fork. ----

#[tokio::test]
async fn v1alpha_read_resolves_manifest_identity_and_delegates_for_other_paths() {
    let inner = MockInner::default();
    inner.set_valid_manifest(&[("pkg/file.usd", "s3://objects/pkg-file")]);
    let service = create_service(inner.clone()).await;

    let stream = foa::FileObject::read_from_address(&service, alpha_read_request(&format!("{WORKSPACE}/pkg/file.usd")))
        .await
        .expect("manifest read");
    let _ = collect_alpha_read_from_address(stream).await.expect("bytes");
    // in-manifest read resolves by identity (mock records the identity it was asked to read)
    assert_eq!(inner.calls("v1alpha_read"), vec!["s3://objects/pkg-file"]);

    let stream = foa::FileObject::read_from_address(&service, alpha_read_request(&format!("{WORKSPACE}/other.usd")))
        .await
        .expect("delegated read");
    assert_eq!(
        collect_alpha_read_from_address(stream).await.expect("bytes"),
        Bytes::from(format!("backend:{WORKSPACE}/other.usd"))
    );
}

#[tokio::test]
async fn v1alpha_stat_is_synthesized_without_backend_stat_and_delegates_otherwise() {
    let inner = MockInner::default();
    inner.set_valid_manifest(&[("a.usd", "s3://objects/a?content-length=42")]);
    let service = create_service(inner.clone()).await;

    let response = foa::FileObject::stat(&service, alpha_stat_request(&format!("{WORKSPACE}/a.usd")))
        .await
        .expect("manifest stat");
    assert_eq!(
        response
            .resource_info
            .expect("info")
            .metadata
            .expect("metadata")
            .data_object_size,
        Some(42)
    );
    assert_eq!(inner.call_count("v1alpha_stat"), 0);

    foa::FileObject::stat(&service, alpha_stat_request(&format!("{WORKSPACE}/other.usd")))
        .await
        .expect("delegated stat");
    assert_eq!(inner.calls("v1alpha_stat"), vec![format!("{WORKSPACE}/other.usd")]);
}

#[tokio::test]
async fn v1alpha_list_backend_first_manifest_wins_and_not_found_falls_through() {
    let inner = MockInner::default();
    inner.set_valid_manifest(&[("x.usd", "s3://objects/x"), ("z.usd", "s3://objects/z")]);
    inner.set_list_pages(
        None,
        vec![MockStreamItem::Ok(ListPage {
            folders: vec![],
            files: vec![
                MANIFEST.to_string(),
                format!("{WORKSPACE}/backend.usd"),
                format!("{WORKSPACE}/x.usd"),
            ],
            continue_token: None,
        })],
    );
    let service = create_service(inner).await;
    let stream = ffa::FileFolder::list(&service, alpha_list_request(WORKSPACE), StreamingOptions::Streamed)
        .await
        .expect("list");
    assert_eq!(
        collect_alpha_list(stream).await.expect("items"),
        vec![
            format!("file:{MANIFEST}"),
            format!("file:{WORKSPACE}/backend.usd"),
            format!("file:{WORKSPACE}/x.usd"),
            format!("file:{WORKSPACE}/z.usd"),
        ]
    );

    let inner = MockInner::default();
    inner.set_valid_manifest(&[("pkg/file.usd", "s3://objects/pkg")]);
    inner.set_list_pages(None, vec![MockStreamItem::Err(MockFailure::NotFound)]);
    let service = create_service(inner).await;
    let stream = ffa::FileFolder::list(
        &service,
        alpha_list_request(&format!("{WORKSPACE}/pkg")),
        StreamingOptions::Streamed,
    )
    .await
    .expect("list pkg");
    assert_eq!(
        collect_alpha_list(stream).await.expect("items"),
        vec![format!("file:{WORKSPACE}/pkg/file.usd")]
    );
}

#[tokio::test]
async fn v1alpha_list_stat_falls_through_on_backend_not_found() {
    let inner = MockInner::default();
    inner.set_valid_manifest(&[("pkg/file.usd", "s3://objects/pkg-file")]);
    inner.set_list_stat_pages(None, vec![MockStreamItem::Err(MockFailure::NotFound)]);
    let service = create_service(inner).await;
    let mut stream = ffa::FileFolder::list_stat(
        &service,
        alpha_list_stat_request(&format!("{WORKSPACE}/pkg")),
        StreamingOptions::Streamed,
    )
    .await
    .expect("list_stat");
    let mut items = Vec::new();
    while let Some(resp) = stream.next().await {
        let resp = resp.expect("page");
        items.extend(
            resp.response
                .subfolder_addresses
                .into_iter()
                .map(|f| format!("folder:{}", f.uri)),
        );
        items.extend(
            resp.response
                .entries
                .into_iter()
                .map(|e| format!("file:{}", e.resource_address)),
        );
    }
    assert_eq!(items, vec![format!("file:{WORKSPACE}/pkg/file.usd")]);
}

#[tokio::test]
async fn v1alpha_enumerate_merges_backend_and_manifest_and_dedups() {
    let inner = MockInner::default();
    inner.set_valid_manifest(&[
        ("pkg/a.usd", "s3://objects/manifest-a"),
        ("pkg/b.usd", "s3://objects/manifest-b"),
    ]);
    inner.set_enumerate_pages(
        None,
        vec![MockStreamItem::Ok(EnumeratePage {
            files: vec![format!("{WORKSPACE}/pkg/a.usd"), format!("{WORKSPACE}/pkg/backend.usd")],
            continue_token: None,
        })],
    );
    let service = create_service(inner).await;
    let stream = foa::FileObject::enumerate(
        &service,
        alpha_enumerate_request(&format!("{WORKSPACE}/pkg")),
        StreamingOptions::Streamed,
    )
    .await
    .expect("enumerate");
    assert_eq!(
        collect_alpha_enumerate(stream).await.expect("items"),
        vec![
            format!("{WORKSPACE}/pkg/backend.usd"),
            "s3://objects/manifest-a".to_string(),
            "s3://objects/manifest-b".to_string(),
        ]
    );
}

// ---- additional v1beta gaps ----

#[tokio::test]
async fn unicode_nfd_manifest_key_matches_nfc_request() {
    let inner = MockInner::default();
    // Key stored decomposed (e + U+0301); the request uses the precomposed form (U+00E9).
    // Both are NFC-normalized, so the direct lookup must resolve to the manifest entry.
    inner.set_valid_manifest(&[("cafe\u{301}.usd", "s3://objects/cafe")]);
    let service = create_service(inner.clone()).await;
    let stream = fo::FileObject::read_from_address(&service, read_request(&format!("{WORKSPACE}/caf\u{e9}.usd")))
        .await
        .expect("nfc read");
    let _ = collect_read_from_address(stream).await.expect("bytes");
    // Resolved via the manifest identity (NFC-consistent) — not a backend fall-through.
    assert_eq!(inner.calls("read"), vec!["s3://objects/cafe"]);
}

#[tokio::test]
async fn list_stat_carries_manifest_metadata_for_manifest_entries() {
    let inner = MockInner::default();
    inner.set_manifest(MockResult::Ok(manifest_info_json(
        "entry.usd",
        "s3://objects/entry",
        99,
    )));
    inner.set_list_stat_pages(
        None,
        vec![MockStreamItem::Ok(ListStatPage {
            folders: vec![],
            files: vec![format!("{WORKSPACE}/backend.usd")],
            continue_token: None,
        })],
    );
    let service = create_service(inner).await;
    let mut stream = ff::FileFolder::list_stat(&service, list_stat_request(WORKSPACE), StreamingOptions::Streamed)
        .await
        .expect("list_stat");
    let mut sizes = HashMap::new();
    while let Some(resp) = stream.next().await {
        for entry in resp.expect("page").response.entries {
            let size = entry
                .resource_info
                .and_then(|i| i.metadata)
                .and_then(|m| m.data_object_size);
            sizes.insert(entry.resource_address, size);
        }
    }
    // Manifest entry carries the manifest's metadata (99); backend entry carries the backend's (1).
    assert_eq!(sizes.get(&format!("{WORKSPACE}/entry.usd")), Some(&Some(99)));
    assert_eq!(sizes.get(&format!("{WORKSPACE}/backend.usd")), Some(&Some(1)));
}

#[tokio::test]
async fn cross_source_list_stat_pagination_has_no_duplicates_or_skips() {
    let inner = MockInner::default();
    inner.set_valid_manifest(&[
        ("a-manifest.usd", "s3://objects/a"),
        ("b-manifest.usd", "s3://objects/b"),
    ]);
    inner.set_list_stat_pages(
        None,
        vec![MockStreamItem::Ok(ListStatPage {
            folders: vec![],
            files: vec![format!("{WORKSPACE}/backend-1.usd")],
            continue_token: Some("ls-2".to_string()),
        })],
    );
    inner.set_list_stat_pages(
        Some("ls-2"),
        vec![MockStreamItem::Ok(ListStatPage {
            folders: vec![],
            files: vec![format!("{WORKSPACE}/backend-2.usd")],
            continue_token: None,
        })],
    );
    let service = create_service(inner).await;
    let mut token = None;
    let mut items = Vec::new();
    loop {
        let mut stream = ff::FileFolder::list_stat(
            &service,
            list_stat_request(WORKSPACE),
            StreamingOptions::Paginated(Pagination {
                continuation_handle: token.clone(),
                max_page_size: Some(1),
            }),
        )
        .await
        .expect("page");
        let page = stream.next().await.expect("page").expect("page ok");
        items.extend(page.response.entries.into_iter().map(|e| e.resource_address));
        token = page.continue_token;
        if token.is_none() {
            break;
        }
    }
    assert_eq!(
        items,
        vec![
            format!("{WORKSPACE}/backend-1.usd"),
            format!("{WORKSPACE}/backend-2.usd"),
            format!("{WORKSPACE}/a-manifest.usd"),
            format!("{WORKSPACE}/b-manifest.usd"),
        ]
    );
}

#[tokio::test]
async fn cross_source_enumerate_pagination_has_no_duplicates_or_skips() {
    let inner = MockInner::default();
    inner.set_valid_manifest(&[("a.usd", "s3://objects/a"), ("b.usd", "s3://objects/b")]);
    inner.set_enumerate_pages(
        None,
        vec![MockStreamItem::Ok(EnumeratePage {
            files: vec![format!("{WORKSPACE}/backend-1.usd")],
            continue_token: Some("en-2".to_string()),
        })],
    );
    inner.set_enumerate_pages(
        Some("en-2"),
        vec![MockStreamItem::Ok(EnumeratePage {
            files: vec![format!("{WORKSPACE}/backend-2.usd")],
            continue_token: None,
        })],
    );
    let service = create_service(inner).await;
    let mut token = None;
    let mut items = Vec::new();
    loop {
        let mut stream = fo::FileObject::enumerate(
            &service,
            enumerate_request(WORKSPACE),
            StreamingOptions::Paginated(Pagination {
                continuation_handle: token.clone(),
                max_page_size: Some(1),
            }),
        )
        .await
        .expect("page");
        let page = stream.next().await.expect("page").expect("page ok");
        items.extend(page.response.items.into_iter().map(|i| i.resource_address));
        token = page.continue_token;
        if token.is_none() {
            break;
        }
    }
    assert_eq!(
        items,
        vec![
            format!("{WORKSPACE}/backend-1.usd"),
            format!("{WORKSPACE}/backend-2.usd"),
            "s3://objects/a".to_string(),
            "s3://objects/b".to_string(),
        ]
    );
}

#[tokio::test]
async fn invalid_manifest_repair_reactivates_overlay() {
    let inner = MockInner::default();
    inner.set_manifest(MockResult::Ok(Bytes::from_static(b"not-json")));
    let service = create_service(inner.clone()).await;

    // Invalid manifest → overlay reads are rejected (fail-closed).
    assert_code(
        fo::FileObject::read_from_address(&service, read_request(&format!("{WORKSPACE}/a.usd"))).await,
        tonic::Code::InvalidArgument,
    );

    // Writing a valid `.manifest` is allowed even when Invalid, and invalidates the cached state.
    let mut write_stream = fo::FileObject::write(
        &service,
        write_request(MANIFEST),
        bytes_future(manifest_json(&[("a.usd", "s3://objects/fixed")])),
    )
    .await
    .expect("repair write");
    let _ = write_stream.next().await.transpose().expect("terminal");

    // The overlay is now active: the entry resolves by identity (mock records the "read").
    let stream = fo::FileObject::read_from_address(&service, read_request(&format!("{WORKSPACE}/a.usd")))
        .await
        .expect("repaired read");
    let _ = collect_read_from_address(stream).await.expect("bytes");
    assert_eq!(inner.calls("read"), vec!["s3://objects/fixed"]);
}

#[tokio::test]
async fn error_on_non_manifest_objects_rejects_non_workspace_paths() {
    let config = Config {
        error_on_non_manifest_objects: true,
        page_size_limit: ByteSize::kib(10),
        ..Config::default()
    };
    let service = create_service_with_config(MockInner::default(), config).await;
    assert_code(
        service.classify_address("s3://bucket/plain/file.usd"),
        tonic::Code::InvalidArgument,
    );
    assert!(service.classify_address(WORKSPACE).is_ok());
}

#[tokio::test]
async fn manifest_object_access_is_marked_nocache_while_entries_and_fallthrough_are_not() {
    let inner = MockInner::default();
    inner.set_valid_manifest(&[("entry.usd", "s3://objects/entry")]);
    let service = create_service(inner.clone()).await;

    // Reading an in-manifest entry forces a `.manifest` body read — that read must be NoCache so a
    // lower cache can't shadow the manifest. (The entry itself is read by identity, which is cacheable.)
    let stream = fo::FileObject::read_from_address(&service, read_request(&format!("{WORKSPACE}/entry.usd")))
        .await
        .expect("entry read");
    let _ = collect_read_from_address(stream).await.expect("bytes");

    // A direct read and stat of the `.manifest` file (the ManifestFile forward) are NoCache too.
    let stream = fo::FileObject::read_from_address(&service, read_request(MANIFEST))
        .await
        .expect("manifest read");
    let _ = collect_read_from_address(stream).await.expect("bytes");
    fo::FileObject::stat(&service, stat_request(MANIFEST))
        .await
        .expect("manifest stat");

    // Fall-through (non-manifest) read + stat go through normal caching (no NoCache).
    let stream = fo::FileObject::read_from_address(&service, read_request(&format!("{WORKSPACE}/passthrough.txt")))
        .await
        .expect("fallthrough read");
    let _ = collect_read_from_address(stream).await.expect("bytes");
    fo::FileObject::stat(&service, stat_request(&format!("{WORKSPACE}/passthrough.txt")))
        .await
        .expect("fallthrough stat");

    let nocache_reads = inner.nocache_reads();
    assert!(
        nocache_reads.iter().all(|addr| addr == MANIFEST),
        "only `.manifest` reads should be NoCache, got {nocache_reads:?}"
    );
    assert!(
        nocache_reads.contains(&MANIFEST.to_string()),
        "the manifest body read must be NoCache"
    );
    assert_eq!(
        inner.nocache_stats(),
        vec![MANIFEST.to_string()],
        "only the `.manifest` stat is NoCache; the fall-through stat stays cacheable"
    );
}

#[tokio::test]
async fn v1beta_manifest_body_and_entry_identity_reads_use_resolver() {
    let inner = MockInner::default();
    let resolver = MockInner::default();
    resolver.set_valid_manifest(&[("entry.usd", "s3://objects/entry")]);
    resolver.set_read_identity("s3://objects/entry", MockResult::Ok(Bytes::from_static(b"resolved")));
    let service = create_service_with_resolver(inner.clone(), resolver.clone()).await;

    fo::FileObject::stat(&service, stat_request(&format!("{WORKSPACE}/entry.usd")))
        .await
        .expect("manifest stat");
    let stream = fo::FileObject::read_from_address(&service, read_request(&format!("{WORKSPACE}/entry.usd")))
        .await
        .expect("manifest entry read");
    assert_eq!(
        collect_read_from_address(stream).await.expect("entry bytes"),
        Bytes::from_static(b"resolved")
    );

    assert!(resolver.manifest_read_count() > 0);
    assert_eq!(resolver.calls("read_from_address"), vec![MANIFEST]);
    assert!(resolver.nocache_reads().contains(&MANIFEST.to_string()));
    assert_eq!(resolver.calls("read"), vec!["s3://objects/entry"]);
    assert_eq!(inner.manifest_read_count(), 0);
    assert!(inner.calls("read").is_empty());
}

#[tokio::test]
async fn v1alpha_manifest_body_and_entry_identity_reads_use_resolver() {
    let inner = MockInner::default();
    let resolver = MockInner::default();
    resolver.set_valid_manifest(&[("entry.usd", "s3://objects/entry")]);
    resolver.set_read_identity("s3://objects/entry", MockResult::Ok(Bytes::from_static(b"resolved")));
    let service = create_service_with_resolver(inner.clone(), resolver.clone()).await;

    foa::FileObject::stat(&service, alpha_stat_request(&format!("{WORKSPACE}/entry.usd")))
        .await
        .expect("manifest stat");
    let stream = foa::FileObject::read_from_address(&service, alpha_read_request(&format!("{WORKSPACE}/entry.usd")))
        .await
        .expect("manifest entry read");
    let _ = collect_alpha_read_from_address(stream).await.expect("entry bytes");

    assert!(resolver.manifest_read_count() > 0);
    assert_eq!(resolver.calls("read_from_address"), vec![MANIFEST]);
    assert!(resolver.nocache_reads().contains(&MANIFEST.to_string()));
    assert_eq!(resolver.calls("v1alpha_read"), vec!["s3://objects/entry"]);
    assert_eq!(inner.manifest_read_count(), 0);
    assert!(inner.calls("v1alpha_read").is_empty());
}

#[tokio::test]
async fn non_workspace_and_direct_manifest_requests_use_inner() {
    let plain = "s3://bucket/plain.usd";
    let inner = MockInner::default();
    inner.set_valid_manifest(&[("entry.usd", "s3://objects/entry")]);
    let resolver = MockInner::default();
    let service = create_service_with_resolver(inner.clone(), resolver.clone()).await;

    fo::FileObject::stat(&service, stat_request(plain))
        .await
        .expect("plain stat");
    let stream = fo::FileObject::read_from_address(&service, read_request(plain))
        .await
        .expect("plain read");
    let _ = collect_read_from_address(stream).await.expect("plain bytes");
    fo::FileObject::stat(&service, stat_request(MANIFEST))
        .await
        .expect("direct manifest stat");
    let stream = fo::FileObject::read_from_address(&service, read_request(MANIFEST))
        .await
        .expect("direct manifest read");
    let _ = collect_read_from_address(stream).await.expect("direct manifest bytes");

    assert_eq!(inner.calls("stat"), vec![plain, MANIFEST]);
    assert_eq!(inner.calls("read_from_address"), vec![plain, MANIFEST]);
    assert_eq!(inner.nocache_reads(), vec![MANIFEST.to_string()]);
    assert_eq!(inner.nocache_stats(), vec![MANIFEST.to_string()]);
    assert!(resolver.calls("stat").is_empty());
    assert!(resolver.calls("read_from_address").is_empty());
    assert!(resolver.calls("read").is_empty());
}

#[tokio::test]
async fn absent_manifest_falls_through_to_inner() {
    let entry = format!("{WORKSPACE}/entry.usd");
    let inner = MockInner::default();
    let resolver = MockInner::default();
    resolver.set_manifest(MockResult::Err(MockFailure::NotFound));
    let service = create_service_with_resolver(inner.clone(), resolver.clone()).await;

    fo::FileObject::stat(&service, stat_request(&entry))
        .await
        .expect("absent stat delegates");
    let stream = fo::FileObject::read_from_address(&service, read_request(&entry))
        .await
        .expect("absent read delegates");
    let _ = collect_read_from_address(stream).await.expect("delegated bytes");

    assert_eq!(resolver.manifest_read_count(), 2);
    assert_eq!(resolver.calls("read_from_address"), vec![MANIFEST, MANIFEST]);
    assert_eq!(
        resolver.nocache_reads(),
        vec![MANIFEST.to_string(), MANIFEST.to_string()]
    );
    assert_eq!(inner.calls("stat"), vec![entry.clone()]);
    assert_eq!(inner.calls("read_from_address"), vec![entry]);
    assert_eq!(inner.manifest_read_count(), 0);
    assert!(resolver.calls("read").is_empty());
}

#[tokio::test]
async fn listing_backend_phases_use_inner() {
    let inner = MockInner::default();
    let resolver = MockInner::default();
    resolver.set_valid_manifest(&[("entry.usd", "s3://objects/entry")]);
    let service = create_service_with_resolver(inner.clone(), resolver.clone()).await;

    let stream = ff::FileFolder::list(&service, list_request(WORKSPACE), StreamingOptions::Streamed)
        .await
        .expect("list");
    let _ = collect_list(stream).await.expect("list items");
    let stream = ff::FileFolder::list_stat(&service, list_stat_request(WORKSPACE), StreamingOptions::Streamed)
        .await
        .expect("list stat");
    let _ = collect_list_stat(stream).await.expect("list stat items");
    let stream = fo::FileObject::enumerate(&service, enumerate_request(WORKSPACE), StreamingOptions::Streamed)
        .await
        .expect("enumerate");
    let _ = collect_enumerate(stream).await.expect("enumerate items");

    assert!(resolver.manifest_read_count() > 0);
    assert_eq!(resolver.calls("read_from_address"), vec![MANIFEST]);
    assert_eq!(inner.calls("list"), vec![WORKSPACE]);
    assert_eq!(inner.calls("list_stat"), vec![WORKSPACE]);
    assert_eq!(inner.calls("enumerate"), vec![WORKSPACE]);
    assert!(inner.calls("read_from_address").is_empty());
    assert!(resolver.calls("list").is_empty());
    assert!(resolver.calls("list_stat").is_empty());
    assert!(resolver.calls("enumerate").is_empty());
    assert!(resolver.calls("read").is_empty());
}

#[tokio::test]
async fn versioning_manifest_reads_use_resolver() {
    let inner = MockInner::default();
    let resolver = MockInner::default();
    resolver.set_valid_manifest(&[("entry.usd", "s3://objects/entry")]);
    let enumerate_service = create_service_with_resolver(inner.clone(), resolver.clone()).await;

    let stream = ver::Versioning::enumerate_versions(
        &enumerate_service,
        versions_request(&format!("{WORKSPACE}/entry.usd")),
        StreamingOptions::Streamed,
        None,
    )
    .await
    .expect("enumerate versions");
    assert_eq!(
        collect_versions(stream).await.expect("version identities"),
        vec!["s3://objects/entry"]
    );

    // Use a fresh manifest cache so each Versioning entry point must route its own cold read.
    let enabled_service = create_service_with_resolver(inner.clone(), resolver.clone()).await;
    assert!(
        vera::Versioning::is_versioned(
            &enabled_service,
            is_versioned_request(&format!("{WORKSPACE}/entry.usd"))
        )
        .await
        .expect("is_versioned")
        .is_versioned
    );

    assert_eq!(resolver.calls("read_from_address"), vec![MANIFEST, MANIFEST]);
    assert_eq!(resolver.nocache_reads(), vec![MANIFEST.to_string(); 2]);
    assert_eq!(inner.manifest_read_count(), 0);
    assert!(inner.calls("enumerate_versions").is_empty());
    assert!(inner.calls("is_versioned").is_empty());
    assert!(resolver.calls("enumerate_versions").is_empty());
    assert!(resolver.calls("is_versioned").is_empty());
}

#[tokio::test]
async fn delegated_mutations_and_confirmation_reads_split_inner_and_resolver() {
    let target = format!("{WORKSPACE}/free.usd");
    let inner = MockInner::default();
    let resolver = MockInner::default();
    resolver.set_valid_manifest(&[("other.usd", "s3://objects/other")]);
    let service = create_service_with_resolver(inner.clone(), resolver.clone()).await;

    fo::FileObject::stat(&service, stat_request(&target))
        .await
        .expect("warm manifest cache");
    assert_eq!(resolver.manifest_read_count(), 1);

    let mut stream = fo::FileObject::write(
        &service,
        write_request(&target),
        bytes_future(Bytes::from_static(b"payload")),
    )
    .await
    .expect("delegated write");
    assert!(stream.next().await.transpose().expect("write response").is_some());
    assert_eq!(resolver.manifest_read_count(), 2);

    fo::FileObject::delete(&service, delete_request(&target))
        .await
        .expect("delegated delete");
    assert_eq!(resolver.manifest_read_count(), 3);

    assert_eq!(inner.calls("write"), vec![target.clone()]);
    assert_eq!(inner.calls("delete"), vec![target]);
    assert!(resolver.calls("write").is_empty());
    assert!(resolver.calls("delete").is_empty());
    assert_eq!(resolver.calls("read_from_address"), vec![MANIFEST; 3]);
    assert!(inner.calls("read_from_address").is_empty());
}

#[tokio::test]
async fn resolver_manifest_errors_propagate_and_are_not_cached() {
    for failure in [
        MockFailure::Internal,
        MockFailure::Unavailable,
        MockFailure::AccessDenied,
    ] {
        let inner = MockInner::default();
        let resolver = MockInner::default();
        resolver.set_manifest(MockResult::Err(failure));
        let service = create_service_with_resolver(inner.clone(), resolver.clone()).await;

        for _ in 0..2 {
            assert_code(
                fo::FileObject::stat(&service, stat_request(&format!("{WORKSPACE}/entry.usd"))).await,
                failure.code(),
            );
        }

        assert_eq!(resolver.manifest_read_count(), 2);
        assert_eq!(inner.manifest_read_count(), 0);
        assert!(inner.calls("stat").is_empty());
    }
}

#[tokio::test]
async fn shared_cache_invalidation_is_visible_across_services() {
    let page_size_limit = ByteSize::kib(10);
    let config = Config {
        page_size_limit,
        ..Default::default()
    };
    let args: LayerCacheArgs = config.clone().into();
    let index = CACHE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let namespace = format!("manifest_overlay_tests_{index}");
    let cache = Arc::new(
        LayerCache::new(&args, &namespace, &namespace, None, None)
            .await
            .expect("shared LayerCache build"),
    );
    let backend = MockInner::default();
    backend.set_valid_manifest(&[("old.usd", "s3://objects/old")]);
    backend.set_read_identity("s3://objects/old", MockResult::Ok(Bytes::from_static(b"old")));
    backend.set_read_identity("s3://objects/new", MockResult::Ok(Bytes::from_static(b"new")));
    let writer = ManifestService::new(
        backend.clone(),
        backend.clone(),
        config.clone(),
        cache.clone(),
        page_size_limit,
    );
    let reader = ManifestService::new(backend.clone(), backend.clone(), config, cache.clone(), page_size_limit);

    let stream = fo::FileObject::read_from_address(&reader, read_request(&format!("{WORKSPACE}/old.usd")))
        .await
        .expect("old entry read");
    assert_eq!(
        collect_read_from_address(stream).await.expect("old bytes"),
        Bytes::from_static(b"old")
    );
    assert_eq!(backend.manifest_read_count(), 1);

    let mut stream = fo::FileObject::write(
        &writer,
        write_request(MANIFEST),
        bytes_future(manifest_json(&[("new.usd", "s3://objects/new")])),
    )
    .await
    .expect("manifest write");
    assert!(stream.next().await.transpose().expect("write response").is_some());

    let stream = fo::FileObject::read_from_address(&reader, read_request(&format!("{WORKSPACE}/new.usd")))
        .await
        .expect("new entry read");
    assert_eq!(
        collect_read_from_address(stream).await.expect("new bytes"),
        Bytes::from_static(b"new")
    );
    assert_eq!(backend.manifest_read_count(), 2);
}

#[test]
fn workspace_constants_drive_parsing() {
    // Behavioral: the parser is built from the WORKSPACE_SUFFIX / MANIFEST_FILENAME constants.
    let key = super::format::WorkspaceKey::parse(format!("s3://bucket/name{WORKSPACE_SUFFIX}/sub/file.usd"))
        .expect("parse")
        .expect("workspace key");
    assert_eq!(key.workspace_path, format!("s3://bucket/name{WORKSPACE_SUFFIX}"));
    assert_eq!(
        key.manifest_path,
        format!("s3://bucket/name{WORKSPACE_SUFFIX}/{MANIFEST_FILENAME}")
    );
    assert_eq!(key.object_path.as_deref(), Some("sub/file.usd"));

    let manifest_key =
        super::format::WorkspaceKey::parse(format!("s3://bucket/name{WORKSPACE_SUFFIX}/{MANIFEST_FILENAME}"))
            .expect("parse")
            .expect("workspace key");
    assert!(manifest_key.is_manifest_file());
}
