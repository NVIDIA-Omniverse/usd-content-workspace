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

//! In-process, stateful `MetadataService` gRPC server standing in for the storage
//! service, so tests can exercise the association paths' storage-failure behavior.
//!
//! The association and disassociate paths are mostly I/O over storage metadata, and
//! their most consequential rules are about what happens when part of that I/O fails:
//! a write that lands and then has to be compensated, a removal that must precede its
//! source key, an already-removed key that makes a retry idempotent, a token the
//! downstream rejects. None of those are reachable without a storage service that can
//! be made to fail on demand, and none are expressible as pure unit tests.
//!
//! This double is stateful rather than a per-call script, because most of those rules
//! are assertions about the metadata left behind: "the write was compensated" means
//! the key is gone afterwards, which a script cannot show.
//!
//! # How tests reach it
//!
//! [`super::init_test_runtime_config`] starts one of these per test binary and points
//! `STORAGE_GRPC_ENDPOINT` at it, so every storage call in a test process lands here
//! and none can reach a real storage service. Get the handle with [`mock_storage`].
//!
//! # Isolation between tests
//!
//! Tests in a binary share the one server and run concurrently, so isolate by resource
//! address rather than by resetting shared state — [`unique_test_address`] mints
//! addresses that cannot collide. Faults are keyed by address too.
//!
//! # Choosing fault codes
//!
//! `Unavailable`, `DeadlineExceeded`, `ResourceExhausted`, and `Aborted` are retryable,
//! so injecting them makes the client retry with backoff before failing. Prefer
//! `Internal` (or the specific code under test, like `Unauthenticated`) to keep tests
//! fast and deterministic; use a retryable code only when retry itself is the subject.

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use storage_proto::nvidia::omniverse::storage::fileobject::v1alpha as fileobject;
use storage_proto::nvidia::omniverse::storage::metadata::v1alpha::metadata_service_server::{
    MetadataService, MetadataServiceServer,
};
use storage_proto::nvidia::omniverse::storage::metadata::v1alpha::{
    DeleteMetadataBatchRequest, DeleteMetadataBatchResponse, DeleteMetadataRequest,
    DeleteMetadataResponse, GetMetadataBatchRequest, GetMetadataBatchResponse, GetMetadataRequest,
    GetMetadataResponse, QueryMetadataRequest, QueryMetadataResponse, UpdateMetadataBatchRequest,
    UpdateMetadataBatchResponse, UpdateMetadataRequest, UpdateMetadataResponse, UserMetadataValue,
};
use tokio_stream::wrappers::TcpListenerStream;
use tonic::transport::Server;
use tonic::{Code, Request, Response, Status};

/// Which metadata operation a fault applies to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MetadataOp {
    Get,
    Update,
    Delete,
}

/// Which file-object operation a fault applies to.
///
/// Separate from [`MetadataOp`] because they are different services with
/// different failure meanings: a metadata delete removes an association, a file
/// delete removes the blob behind it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileOp {
    Stat,
    Read,
    Write,
    Delete,
}

/// One recorded file-object call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedFileCall {
    pub op: FileOp,
    pub address: String,
}

/// The identity the double reports for a blob at `address`.
///
/// Real storage identities are opaque; this one is derived so a test can seed a
/// blob and predict the identity a `Stat` or `Write` will hand back.
pub fn mock_identity_for(address: &str) -> String {
    format!("{address}?etag=mock-etag")
}

/// One recorded call, for asserting what the code under test actually did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedCall {
    pub op: MetadataOp,
    pub address: String,
    /// The metadata key. `None` for `Get`, which enumerates rather than naming a key.
    pub key: Option<String>,
    /// The W3C `traceparent` that arrived, if any.
    ///
    /// Recorded because until it was, no assertion could see one: the double read
    /// the `authorization` header for its own token checks and discarded the rest,
    /// so outbound trace propagation to storage was untestable rather than
    /// untested. These are the storage calls made in the middle of a request,
    /// which makes them the ones where an unattributed stall is hardest to explain.
    pub traceparent: Option<String>,
    /// The raw `authorization` header that arrived, if any.
    ///
    /// [`bearer_token`] strips the scheme for the double's own use; this keeps the
    /// value as sent, so a test can assert the header reached the wire at all.
    pub authorization: Option<String>,
}

/// A fault to raise instead of performing the operation.
#[derive(Clone, Debug)]
struct Fault {
    code: Code,
    /// Faults left to raise. `None` raises the fault on every matching call, which is
    /// what a genuinely broken upstream looks like; `Some(n)` lets a test fail the
    /// first `n` attempts and then let a retry through.
    remaining: Option<usize>,
}

/// Address plus optional key a fault is scoped to. A `None` key matches any key for
/// that address, so a test can break a whole resource in one line.
type FaultKey = (MetadataOp, String, Option<String>);

#[derive(Default)]
struct MockStorageState {
    /// address -> key -> value.
    metadata: HashMap<String, HashMap<String, serde_json::Value>>,
    faults: HashMap<FaultKey, Fault>,
    /// Address-scoped terminal errors sent after a batch response.
    delete_batch_stream_faults: HashMap<String, Code>,
    /// Bearer tokens the downstream rejects as unauthenticated, whatever the call.
    rejected_tokens: HashSet<String>,
    calls: Vec<RecordedCall>,

    /// File-object calls that should accept and then never answer, so a caller's
    /// own request deadline is what ends them. Distinct from a fault: a fault is a
    /// reply, and the case worth testing is the peer that does not reply at all —
    /// which connect timeouts and keepalive both miss.
    file_stalls: HashSet<(FileOp, String)>,

    /// address -> bytes, for the file-object side of the double.
    blobs: HashMap<String, Vec<u8>>,
    /// encoded_identity -> address, so `Read` can resolve an identity to bytes.
    identities: HashMap<String, String>,
    file_faults: HashMap<(FileOp, String), Fault>,
    /// Destinations whose `Write` completes without returning a `resource_info`,
    /// which the worker must treat as a failed upload rather than associating an
    /// unverified thumbnail.
    writes_without_identity: HashSet<String>,
    file_calls: Vec<RecordedFileCall>,
}

/// A running storage double.
pub struct MockStorageServer {
    addr: SocketAddr,
    state: Arc<Mutex<MockStorageState>>,
}

impl MockStorageServer {
    /// The metadata stored for one address, as a disassociation operation would enumerate it.
    pub fn metadata(&self, address: &str) -> HashMap<String, serde_json::Value> {
        self.lock()
            .metadata
            .get(address)
            .cloned()
            .unwrap_or_default()
    }

    /// Whether a single metadata key is present.
    pub fn has_key(&self, address: &str, key: &str) -> bool {
        self.lock()
            .metadata
            .get(address)
            .is_some_and(|keys| keys.contains_key(key))
    }

    /// Seed metadata as if a previous association had written it.
    pub fn seed(&self, address: &str, key: &str, value: serde_json::Value) {
        self.lock()
            .metadata
            .entry(address.to_string())
            .or_default()
            .insert(key.to_string(), value);
    }

    /// Fail every matching call with `code` until [`Self::clear_faults`].
    ///
    /// A `key` of `None` matches every key on that address.
    pub fn fail(&self, op: MetadataOp, address: &str, key: Option<&str>, code: Code) {
        self.insert_fault(op, address, key, code, None);
    }

    /// Fail the next `times` matching calls with `code`, then behave normally — the
    /// shape of a transient upstream fault that a retry should survive.
    pub fn fail_times(
        &self,
        op: MetadataOp,
        address: &str,
        key: Option<&str>,
        code: Code,
        times: usize,
    ) {
        self.insert_fault(op, address, key, code, Some(times));
    }

    /// Drop every fault scoped to `address`.
    pub fn clear_faults(&self, address: &str) {
        self.lock()
            .faults
            .retain(|(_, fault_address, _), _| fault_address != address);
    }

    /// Reject the next call carrying `token` as unauthenticated.
    ///
    /// The rejection is consumed so this process-wide test double cannot leak the
    /// rejected token into a later test that happens to reuse the same string.
    pub fn reject_token(&self, token: &str) {
        self.lock().rejected_tokens.insert(token.to_string());
    }

    /// End a delete-batch response stream with `code` after sending its results.
    pub fn fail_delete_batch_stream(&self, address: &str, code: Code) {
        self.lock()
            .delete_batch_stream_faults
            .insert(address.to_string(), code);
    }

    /// Every call the double has served, in order, for the given address.
    pub fn calls_for(&self, address: &str) -> Vec<RecordedCall> {
        self.lock()
            .calls
            .iter()
            .filter(|call| call.address == address)
            .cloned()
            .collect()
    }

    // --- File-object side ---------------------------------------------------

    /// Seed a blob as if something had already written it, and register the
    /// identity a `Stat` or `Read` would use to reach it.
    pub fn seed_blob(&self, address: &str, bytes: Vec<u8>) {
        let mut state = self.lock();
        state
            .identities
            .insert(mock_identity_for(address), address.to_string());
        state.blobs.insert(address.to_string(), bytes);
    }

    /// The bytes currently stored at `address`, if any.
    pub fn blob(&self, address: &str) -> Option<Vec<u8>> {
        self.lock().blobs.get(address).cloned()
    }

    pub fn has_blob(&self, address: &str) -> bool {
        self.lock().blobs.contains_key(address)
    }

    /// Fail every matching file-object call with `code` until cleared.
    pub fn fail_file(&self, op: FileOp, address: &str, code: Code) {
        self.insert_file_fault(op, address, code, None);
    }

    /// Accept a matching file-object call and never answer it.
    ///
    /// The caller's per-request deadline is expected to end it. Nothing clears
    /// this: a stall is a property of the peer for the life of the test.
    pub fn stall_file(&self, op: FileOp, address: &str) {
        self.state
            .lock()
            .expect("mock storage state")
            .file_stalls
            .insert((op, address.to_string()));
    }

    /// Fail the next `times` matching file-object calls, then behave normally.
    pub fn fail_file_times(&self, op: FileOp, address: &str, code: Code, times: usize) {
        self.insert_file_fault(op, address, code, Some(times));
    }

    /// Complete a `Write` to `address` without returning a resource identity.
    pub fn write_without_identity(&self, address: &str) {
        self.lock()
            .writes_without_identity
            .insert(address.to_string());
    }

    /// Every file-object call the double has served for `address`, in order.
    pub fn file_calls_for(&self, address: &str) -> Vec<RecordedFileCall> {
        self.lock()
            .file_calls
            .iter()
            .filter(|call| call.address == address)
            .cloned()
            .collect()
    }

    fn insert_fault(
        &self,
        op: MetadataOp,
        address: &str,
        key: Option<&str>,
        code: Code,
        remaining: Option<usize>,
    ) {
        self.lock().faults.insert(
            (op, address.to_string(), key.map(|k| k.to_string())),
            Fault { code, remaining },
        );
    }

    fn insert_file_fault(&self, op: FileOp, address: &str, code: Code, remaining: Option<usize>) {
        self.lock()
            .file_faults
            .insert((op, address.to_string()), Fault { code, remaining });
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, MockStorageState> {
        self.state
            .lock()
            .expect("mock storage state mutex must not be poisoned")
    }
}

impl MockStorageState {
    /// Consume a fault matching this call, if one is armed. Key-specific faults win
    /// over address-wide ones so a test can break one key of an otherwise fine address.
    fn take_fault(&mut self, op: MetadataOp, address: &str, key: Option<&str>) -> Option<Code> {
        let candidates = [
            key.map(|k| (op, address.to_string(), Some(k.to_string()))),
            Some((op, address.to_string(), None)),
        ];

        for fault_key in candidates.into_iter().flatten() {
            let Some(fault) = self.faults.get_mut(&fault_key) else {
                continue;
            };
            let code = fault.code;
            match &mut fault.remaining {
                Some(0) => continue,
                Some(remaining) => *remaining -= 1,
                None => {}
            }
            return Some(code);
        }
        None
    }

    /// Record a file-object call and consume any fault armed for it.
    fn admit_file(&mut self, op: FileOp, address: &str) -> Option<Code> {
        self.file_calls.push(RecordedFileCall {
            op,
            address: address.to_string(),
        });

        let fault = self.file_faults.get_mut(&(op, address.to_string()))?;
        let code = fault.code;
        match &mut fault.remaining {
            Some(0) => return None,
            Some(remaining) => *remaining -= 1,
            None => {}
        }
        Some(code)
    }
}

struct MockMetadataService {
    state: Arc<Mutex<MockStorageState>>,
}

impl MockMetadataService {
    /// Record the call and resolve whether it should fail before touching metadata.
    fn admit<T>(
        &self,
        request: &Request<T>,
        op: MetadataOp,
        address: &str,
        key: Option<&str>,
    ) -> Result<(), Status> {
        let token = bearer_token(request);
        let traceparent = raw_header(request, "traceparent");
        let authorization = raw_header(request, "authorization");
        let mut state = self.lock();

        state.calls.push(RecordedCall {
            op,
            address: address.to_string(),
            key: key.map(|k| k.to_string()),
            traceparent,
            authorization,
        });

        if token
            .as_deref()
            .is_some_and(|token| state.rejected_tokens.remove(token))
        {
            return Err(Status::unauthenticated("mock storage: token rejected"));
        }

        match state.take_fault(op, address, key) {
            Some(code) => Err(Status::new(code, "mock storage: injected fault")),
            None => Ok(()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, MockStorageState> {
        self.state
            .lock()
            .expect("mock storage state mutex must not be poisoned")
    }
}

fn bearer_token<T>(request: &Request<T>) -> Option<String> {
    request
        .metadata()
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim_start_matches("Bearer ").to_string())
}

/// A header exactly as it arrived, for recording rather than for acting on.
fn raw_header<T>(request: &Request<T>, name: &str) -> Option<String> {
    request
        .metadata()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

#[tonic::async_trait]
impl MetadataService for MockMetadataService {
    async fn get_metadata(
        &self,
        request: Request<GetMetadataRequest>,
    ) -> Result<Response<GetMetadataResponse>, Status> {
        let address = request.get_ref().uri.clone();
        self.admit(&request, MetadataOp::Get, &address, None)?;

        let requested_keys = &request.get_ref().user_metadata_keys;
        let stored = self
            .lock()
            .metadata
            .get(&address)
            .cloned()
            .unwrap_or_default();

        let user_metadata = stored
            .into_iter()
            .filter(|(key, _)| requested_keys.is_empty() || requested_keys.contains(key))
            .map(|(key, value)| {
                let value = crate::utils::json_to_prost_value(&value)
                    .expect("seeded mock storage metadata must convert to a proto value");
                (
                    key,
                    UserMetadataValue {
                        value: Some(value),
                        etag: "mock-etag".to_string(),
                    },
                )
            })
            .collect();

        Ok(Response::new(GetMetadataResponse { user_metadata }))
    }

    async fn update_metadata(
        &self,
        request: Request<UpdateMetadataRequest>,
    ) -> Result<Response<UpdateMetadataResponse>, Status> {
        let address = request.get_ref().uri.clone();
        let key = request.get_ref().user_metadata_key.clone();
        self.admit(&request, MetadataOp::Update, &address, Some(&key))?;

        let value = request
            .get_ref()
            .user_metadata
            .as_ref()
            .map(crate::utils::prost_to_json_value)
            .unwrap_or(serde_json::Value::Null);

        self.lock()
            .metadata
            .entry(address)
            .or_default()
            .insert(key, value);

        Ok(Response::new(UpdateMetadataResponse {
            etag: "mock-etag".to_string(),
        }))
    }

    async fn delete_metadata(
        &self,
        request: Request<DeleteMetadataRequest>,
    ) -> Result<Response<DeleteMetadataResponse>, Status> {
        let address = request.get_ref().uri.clone();
        let key = request.get_ref().user_metadata_key.clone();
        self.admit(&request, MetadataOp::Delete, &address, Some(&key))?;

        let removed = self
            .lock()
            .metadata
            .get_mut(&address)
            .and_then(|keys| keys.remove(&key))
            .is_some();

        if removed {
            Ok(Response::new(DeleteMetadataResponse {}))
        } else {
            // What real storage reports for an absent key, and what makes a repeated
            // removal idempotent for callers that treat NOT_FOUND as already-done.
            Err(Status::not_found("mock storage: no such metadata key"))
        }
    }

    type GetMetadataBatchStream =
        futures_util::stream::Empty<Result<GetMetadataBatchResponse, Status>>;

    async fn get_metadata_batch(
        &self,
        _request: Request<GetMetadataBatchRequest>,
    ) -> Result<Response<Self::GetMetadataBatchStream>, Status> {
        Err(Status::unimplemented("mock: get_metadata_batch"))
    }

    type QueryMetadataStream = futures_util::stream::Empty<Result<QueryMetadataResponse, Status>>;

    async fn query_metadata(
        &self,
        _request: Request<QueryMetadataRequest>,
    ) -> Result<Response<Self::QueryMetadataStream>, Status> {
        Err(Status::unimplemented("mock: query_metadata"))
    }

    type UpdateMetadataBatchStream =
        futures_util::stream::Empty<Result<UpdateMetadataBatchResponse, Status>>;

    async fn update_metadata_batch(
        &self,
        _request: Request<UpdateMetadataBatchRequest>,
    ) -> Result<Response<Self::UpdateMetadataBatchStream>, Status> {
        Err(Status::unimplemented("mock: update_metadata_batch"))
    }

    type DeleteMetadataBatchStream =
        tokio_stream::wrappers::ReceiverStream<Result<DeleteMetadataBatchResponse, Status>>;

    async fn delete_metadata_batch(
        &self,
        request: Request<DeleteMetadataBatchRequest>,
    ) -> Result<Response<Self::DeleteMetadataBatchStream>, Status> {
        use storage_proto::nvidia::omniverse::storage::metadata::v1alpha::{
            batch_error_entry::Code as BatchCode, delete_metadata_batch_response_entry::Outcome,
            resource_location::Resource, BatchErrorEntry, DeleteMetadataBatchResponseEntry,
            DeleteMetadataBatchResponseEntrySuccess,
        };

        // Auth check at the RPC level — consume the rejected token once for the whole batch.
        {
            let token = bearer_token(&request);
            let mut state = self.lock();
            if token
                .as_deref()
                .is_some_and(|t| state.rejected_tokens.remove(t))
            {
                return Err(Status::unauthenticated("mock storage: token rejected"));
            }
        }

        // Read once for the whole batch: the entries below are recorded inside a
        // loop that no longer holds the request.
        let traceparent = raw_header(&request, "traceparent");
        let authorization = raw_header(&request, "authorization");

        let entries = request.into_inner().requests;
        let mut results = Vec::with_capacity(entries.len());
        let mut stream_fault = None;

        for (index, entry) in entries.into_iter().enumerate() {
            let address = match entry.resource_location.and_then(|rl| rl.resource) {
                Some(Resource::ResourceAddress(addr)) => addr,
                _ => {
                    results.push(DeleteMetadataBatchResponseEntry {
                        outcome: Some(Outcome::Error(BatchErrorEntry {
                            code: BatchCode::InvalidArgument as i32,
                            message: "mock: missing or unsupported resource location".to_string(),
                        })),
                        request_index: index as u32,
                    });
                    continue;
                }
            };
            let key = entry.user_metadata_key;

            // Record the call and check per-entry faults.
            let fault = {
                let mut state = self.lock();
                state.calls.push(RecordedCall {
                    op: MetadataOp::Delete,
                    address: address.clone(),
                    key: Some(key.clone()),
                    traceparent: traceparent.clone(),
                    authorization: authorization.clone(),
                });
                if stream_fault.is_none() {
                    stream_fault = state.delete_batch_stream_faults.remove(&address);
                }
                state.take_fault(MetadataOp::Delete, &address, Some(&key))
            };

            if let Some(code) = fault {
                // Map the tonic Code to the closest batch error code.
                let batch_code = match code {
                    Code::NotFound => BatchCode::NotFound,
                    Code::InvalidArgument => BatchCode::InvalidArgument,
                    Code::PermissionDenied => BatchCode::PermissionDenied,
                    Code::ResourceExhausted => BatchCode::ResourceExhausted,
                    Code::Unavailable => BatchCode::Unavailable,
                    _ => BatchCode::Internal,
                };
                results.push(DeleteMetadataBatchResponseEntry {
                    outcome: Some(Outcome::Error(BatchErrorEntry {
                        code: batch_code as i32,
                        message: "mock storage: injected fault".to_string(),
                    })),
                    request_index: index as u32,
                });
                continue;
            }

            let removed = self
                .lock()
                .metadata
                .get_mut(&address)
                .and_then(|keys| keys.remove(&key))
                .is_some();

            if removed {
                results.push(DeleteMetadataBatchResponseEntry {
                    outcome: Some(Outcome::Success(DeleteMetadataBatchResponseEntrySuccess {})),
                    request_index: index as u32,
                });
            } else {
                results.push(DeleteMetadataBatchResponseEntry {
                    outcome: Some(Outcome::Error(BatchErrorEntry {
                        code: BatchCode::NotFound as i32,
                        message: "mock storage: no such metadata key".to_string(),
                    })),
                    request_index: index as u32,
                });
            }
        }

        // The real service may return entries in arbitrary order. Reverse multi-entry
        // responses so callers are forced to correlate through request_index.
        if results.len() > 1 {
            results.reverse();
        }

        let (tx, rx) = tokio::sync::mpsc::channel(1);
        tokio::spawn(async move {
            let _ = tx.send(Ok(DeleteMetadataBatchResponse { results })).await;
            if let Some(code) = stream_fault {
                let _ = tx
                    .send(Err(Status::new(
                        code,
                        "mock storage: injected delete-batch stream fault",
                    )))
                    .await;
            }
        });
        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
            rx,
        )))
    }
}

/// The file-object half of the double: blob read, write, stat, and delete.
///
/// Only the operations the thumbnail pipeline uses are implemented; the rest
/// answer `unimplemented` so an accidental new dependency on storage shows up as
/// a failing test rather than silently passing.
struct MockFileObjectService {
    state: Arc<Mutex<MockStorageState>>,
}

impl MockFileObjectService {
    async fn admit<T>(
        &self,
        request: &Request<T>,
        op: FileOp,
        address: &str,
    ) -> Result<(), Status> {
        let token = bearer_token(request);

        // The guard lives in this block and nowhere else. An explicit `drop` is
        // not enough: this is a `std::sync::MutexGuard` in an `async fn`, and the
        // generator analysis keeps its type in the state machine across the await
        // below, which makes the handler future non-Send. A block scope is what
        // proves it is gone.
        let (stalled, fault) = {
            let mut state = self.lock();

            if token
                .as_deref()
                .is_some_and(|token| state.rejected_tokens.remove(token))
            {
                state.file_calls.push(RecordedFileCall {
                    op,
                    address: address.to_string(),
                });
                return Err(Status::unauthenticated("mock storage: token rejected"));
            }

            let stalled = state.file_stalls.contains(&(op, address.to_string()));
            let fault = state.admit_file(op, address);
            (stalled, fault)
        };

        if stalled {
            // Far longer than any deadline under test. If a caller's deadline does
            // not fire, its test times out rather than passing.
            tokio::time::sleep(std::time::Duration::from_secs(3_600)).await;
            return Err(Status::internal(
                "mock storage: unreachable, the caller's deadline should have fired",
            ));
        }

        match fault {
            Some(code) => Err(Status::new(code, "mock storage: injected fault")),
            None => Ok(()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, MockStorageState> {
        self.state
            .lock()
            .expect("mock storage state mutex must not be poisoned")
    }
}

type FileObjectStream<T> =
    std::pin::Pin<Box<dyn tokio_stream::Stream<Item = Result<T, Status>> + Send>>;

#[tonic::async_trait]
impl fileobject::file_object_service_server::FileObjectService for MockFileObjectService {
    type EnumerateStream = FileObjectStream<fileobject::EnumerateResponse>;
    type ReadStream = FileObjectStream<fileobject::ReadResponse>;
    type ReadFromAddressStream = FileObjectStream<fileobject::ReadFromAddressResponse>;
    type WriteStream = FileObjectStream<fileobject::WriteResponse>;

    async fn stat(
        &self,
        request: Request<fileobject::StatRequest>,
    ) -> Result<Response<fileobject::StatResponse>, Status> {
        let address = request.get_ref().resource_address.clone();
        self.admit(&request, FileOp::Stat, &address).await?;

        let size = self
            .lock()
            .blobs
            .get(&address)
            .map(|bytes| bytes.len() as u64)
            .ok_or_else(|| Status::not_found("mock storage: no such data object"))?;

        Ok(Response::new(fileobject::StatResponse {
            resource_info: Some(fileobject::ResourceInfo {
                resource_identity: Some(fileobject::ResourceIdentity {
                    encoded_identity: mock_identity_for(&address),
                }),
                metadata: Some(fileobject::Metadata {
                    data_object_size: Some(size),
                    ..Default::default()
                }),
            }),
        }))
    }

    async fn read(
        &self,
        request: Request<fileobject::ReadRequest>,
    ) -> Result<Response<Self::ReadStream>, Status> {
        let encoded_identity = request
            .get_ref()
            .resource_identity
            .as_ref()
            .map(|identity| identity.encoded_identity.clone())
            .unwrap_or_default();

        let address = self
            .lock()
            .identities
            .get(&encoded_identity)
            .cloned()
            // Fall back to the identity itself so a fault can be armed by
            // identity for a blob that was never seeded.
            .unwrap_or_else(|| encoded_identity.clone());

        self.admit(&request, FileOp::Read, &address).await?;

        let bytes = self
            .lock()
            .blobs
            .get(&address)
            .cloned()
            .ok_or_else(|| Status::not_found("mock storage: no such data object"))?;

        // Metadata first, then the bytes in chunks, matching the documented order.
        let mut messages = vec![Ok(fileobject::ReadResponse {
            reply_type: Some(fileobject::read_response::ReplyType::Metadata(
                fileobject::Metadata {
                    data_object_size: Some(bytes.len() as u64),
                    ..Default::default()
                },
            )),
        })];
        for chunk in bytes.chunks(64 * 1024) {
            messages.push(Ok(fileobject::ReadResponse {
                reply_type: Some(fileobject::read_response::ReplyType::Chunk(
                    fileobject::Chunk {
                        chunk: chunk.to_vec(),
                    },
                )),
            }));
        }

        Ok(Response::new(Box::pin(tokio_stream::iter(messages))))
    }

    async fn write(
        &self,
        request: Request<tonic::Streaming<fileobject::WriteRequest>>,
    ) -> Result<Response<Self::WriteStream>, Status> {
        use fileobject::{write_request::WriteRequestType, write_response::WriteResponseType};

        let token = bearer_token(&request);
        let mut stream = request.into_inner();

        let mut destination: Option<String> = None;
        let mut declared_size: u64 = 0;
        let mut body: Vec<u8> = Vec::new();

        while let Some(message) = stream.message().await? {
            match message.write_request_type {
                Some(WriteRequestType::Params(params)) => {
                    let address = params.destination_resource_address.clone();
                    {
                        let mut state = self.state.lock().expect("mock storage state mutex");
                        if token
                            .as_deref()
                            .is_some_and(|token| state.rejected_tokens.remove(token))
                        {
                            state.file_calls.push(RecordedFileCall {
                                op: FileOp::Write,
                                address,
                            });
                            return Err(Status::unauthenticated("mock storage: token rejected"));
                        }
                        if let Some(code) = state.admit_file(FileOp::Write, &address) {
                            return Err(Status::new(code, "mock storage: injected fault"));
                        }
                    }
                    declared_size = params.data_object_size;
                    destination = Some(params.destination_resource_address);
                }
                Some(WriteRequestType::Chunk(chunk)) => body.extend_from_slice(&chunk.chunk),
                None => {}
            }
        }

        let Some(destination) = destination else {
            return Err(Status::invalid_argument(
                "mock storage: write stream carried no WriteParameters",
            ));
        };

        if declared_size != body.len() as u64 {
            return Err(Status::invalid_argument(format!(
                "mock storage: declared data_object_size {declared_size} but received {} bytes",
                body.len()
            )));
        }

        let omit_identity = {
            let mut state = self.lock();
            state
                .identities
                .insert(mock_identity_for(&destination), destination.clone());
            state.blobs.insert(destination.clone(), body);
            state.writes_without_identity.contains(&destination)
        };

        let completion = if omit_identity {
            fileobject::WriteResponse {
                write_response_type: None,
            }
        } else {
            fileobject::WriteResponse {
                write_response_type: Some(WriteResponseType::ResourceInfo(
                    fileobject::ResourceInfo {
                        resource_identity: Some(fileobject::ResourceIdentity {
                            encoded_identity: mock_identity_for(&destination),
                        }),
                        metadata: None,
                    },
                )),
            }
        };

        Ok(Response::new(Box::pin(tokio_stream::iter(vec![
            Ok(fileobject::WriteResponse {
                write_response_type: Some(WriteResponseType::WriteChunksAccepted(
                    fileobject::WriteChunksAccepted {},
                )),
            }),
            Ok(completion),
        ]))))
    }

    async fn delete(
        &self,
        request: Request<fileobject::DeleteRequest>,
    ) -> Result<Response<fileobject::DeleteResponse>, Status> {
        let address = request.get_ref().resource_address.clone();
        self.admit(&request, FileOp::Delete, &address).await?;

        let removed = self.lock().blobs.remove(&address).is_some();
        if removed {
            Ok(Response::new(fileobject::DeleteResponse {}))
        } else {
            Err(Status::not_found("mock storage: no such data object"))
        }
    }

    async fn enumerate(
        &self,
        _: Request<fileobject::EnumerateRequest>,
    ) -> Result<Response<Self::EnumerateStream>, Status> {
        Err(Status::unimplemented("mock: enumerate"))
    }

    async fn read_from_address(
        &self,
        _: Request<fileobject::ReadFromAddressRequest>,
    ) -> Result<Response<Self::ReadFromAddressStream>, Status> {
        Err(Status::unimplemented("mock: read_from_address"))
    }

    async fn fetch_write_type_info(
        &self,
        _: Request<fileobject::FetchWriteTypeInfoRequest>,
    ) -> Result<Response<fileobject::FetchWriteTypeInfoResponse>, Status> {
        Err(Status::unimplemented("mock: fetch_write_type_info"))
    }

    async fn complete_redirect_upload(
        &self,
        _: Request<fileobject::CompleteRedirectUploadRequest>,
    ) -> Result<Response<fileobject::CompleteRedirectUploadResponse>, Status> {
        Err(Status::unimplemented("mock: complete_redirect_upload"))
    }

    async fn upload_part(
        &self,
        _: Request<fileobject::UploadPartRequest>,
    ) -> Result<Response<fileobject::UploadPartResponse>, Status> {
        Err(Status::unimplemented("mock: upload_part"))
    }

    async fn complete_multipart_upload(
        &self,
        _: Request<fileobject::CompleteMultipartUploadRequest>,
    ) -> Result<Response<fileobject::CompleteMultipartUploadResponse>, Status> {
        Err(Status::unimplemented("mock: complete_multipart_upload"))
    }

    async fn abort_multipart_upload(
        &self,
        _: Request<fileobject::AbortMultipartUploadRequest>,
    ) -> Result<Response<fileobject::AbortMultipartUploadResponse>, Status> {
        Err(Status::unimplemented("mock: abort_multipart_upload"))
    }

    async fn copy(
        &self,
        _: Request<fileobject::CopyRequest>,
    ) -> Result<Response<fileobject::CopyResponse>, Status> {
        Err(Status::unimplemented("mock: copy"))
    }

    async fn r#move(
        &self,
        _: Request<fileobject::MoveRequest>,
    ) -> Result<Response<fileobject::MoveResponse>, Status> {
        Err(Status::unimplemented("mock: move"))
    }

    async fn get_native_metadata_support(
        &self,
        _: Request<fileobject::GetNativeMetadataSupportRequest>,
    ) -> Result<Response<fileobject::GetNativeMetadataSupportResponse>, Status> {
        Err(Status::unimplemented("mock: get_native_metadata_support"))
    }

    async fn get_optimistic_locking_support(
        &self,
        _: Request<fileobject::GetOptimisticLockingSupportRequest>,
    ) -> Result<Response<fileobject::GetOptimisticLockingSupportResponse>, Status> {
        Err(Status::unimplemented(
            "mock: get_optimistic_locking_support",
        ))
    }
}

static MOCK_STORAGE: OnceLock<MockStorageServer> = OnceLock::new();

/// The test binary's storage double.
///
/// Panics if the runtime-config fixture has not run, since that is what starts the
/// double and points the storage endpoint at it.
pub fn mock_storage() -> &'static MockStorageServer {
    MOCK_STORAGE.get().expect(
        "mock storage server is not running; call test_fixtures::init_test_runtime_config() first",
    )
}

/// Start the double and return its address, for the fixture to publish as
/// `STORAGE_GRPC_ENDPOINT` before the runtime config is parsed.
///
/// Runs on its own thread with its own runtime so it outlives each test's runtime and
/// is reachable from sync tests. Called once per test binary; the server then lives for
/// the process, which is why there is no shutdown path.
pub(super) fn start_mock_storage() -> SocketAddr {
    if let Some(server) = MOCK_STORAGE.get() {
        return server.addr;
    }

    let state = Arc::new(Mutex::new(MockStorageState::default()));
    let (addr_tx, addr_rx) = std::sync::mpsc::channel::<SocketAddr>();
    let server_state = state.clone();

    std::thread::Builder::new()
        .name("mock-storage-server".to_string())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("mock storage server must build a runtime");

            runtime.block_on(async move {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                    .await
                    .expect("mock storage server must bind an ephemeral loopback port");
                addr_tx
                    .send(
                        listener
                            .local_addr()
                            .expect("bound listener must expose its local addr"),
                    )
                    .expect("fixture must still be waiting for the mock storage address");

                let _ = Server::builder()
                    .add_service(MetadataServiceServer::new(MockMetadataService {
                        state: server_state.clone(),
                    }))
                    .add_service(
                        fileobject::file_object_service_server::FileObjectServiceServer::new(
                            MockFileObjectService {
                                state: server_state,
                            },
                        ),
                    )
                    .serve_with_incoming(TcpListenerStream::new(listener))
                    .await;
            });
        })
        .expect("mock storage server thread must spawn");

    let addr = addr_rx
        .recv()
        .expect("mock storage server must report its bound address");
    let _ = MOCK_STORAGE.set(MockStorageServer { addr, state });
    addr
}

/// A resource address no other test will use.
///
/// Tests in a binary share the one double concurrently, so scope each test's metadata
/// and faults to addresses of its own rather than resetting shared state.
pub fn unique_test_address(label: &str) -> String {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    format!("https://storage.example.test/container/{label}-{n}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request_with_token(token: &str) -> Request<()> {
        let mut request = Request::new(());
        request.metadata_mut().insert(
            "authorization",
            format!("Bearer {token}")
                .parse()
                .expect("test token must be valid metadata"),
        );
        request
    }

    #[test]
    fn rejected_token_is_consumed_after_one_call() {
        let state = Arc::new(Mutex::new(MockStorageState::default()));
        state
            .lock()
            .expect("mock storage state mutex must not be poisoned")
            .rejected_tokens
            .insert("reusable-token".to_string());
        let service = MockMetadataService { state };

        let rejection = service
            .admit(
                &request_with_token("reusable-token"),
                MetadataOp::Get,
                "https://storage.example.test/source",
                None,
            )
            .expect_err("the armed rejection must reject the next call");
        assert_eq!(rejection.code(), Code::Unauthenticated);

        service
            .admit(
                &request_with_token("reusable-token"),
                MetadataOp::Get,
                "https://storage.example.test/source",
                None,
            )
            .expect("the same token must be reusable after its rejection is consumed");
    }
}
