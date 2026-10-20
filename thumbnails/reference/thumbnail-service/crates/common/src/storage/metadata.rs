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

//! Storage service metadata operations.

use crate::current_runtime_config;
use crate::resource::canonicalize_resource_address;
use crate::utils;
use futures_util::StreamExt as _;
use std::future::Future;
use std::time::Duration;
use storage_proto::nvidia::omniverse::storage::metadata::v1alpha as metadata_proto;
use storage_proto::nvidia::omniverse::storage::metadata::v1alpha::metadata_service_client::MetadataServiceClient;
use thiserror::Error;
use tonic::{Code, Status};
use tracing::{debug, info, warn};

/// Why a storage metadata call failed.
///
/// Authentication and authorization failures are kept apart from `Failed` so a
/// transport can give the caller an actionable status instead of an upstream fault.
#[derive(Debug, Error, Clone)]
pub enum StorageMetadataError {
    #[error("storage metadata key was not found")]
    NotFound,
    #[error("{0}")]
    Unauthenticated(String),
    #[error("{0}")]
    PermissionDenied(String),
    #[error("{0}")]
    Failed(String),
}

impl StorageMetadataError {
    fn from_status(status: &Status) -> Self {
        let message = storage_status_to_message(status);
        match status.code() {
            Code::NotFound => Self::NotFound,
            Code::Unauthenticated => Self::Unauthenticated(message),
            Code::PermissionDenied => Self::PermissionDenied(message),
            _ => Self::Failed(message),
        }
    }

    /// As [`Self::from_status`], but an absent key is its own outcome. Only removal
    /// treats absence as meaningful: an already-removed key makes a retry idempotent.
    fn from_removal_status(status: &Status) -> Self {
        match status.code() {
            Code::NotFound => Self::NotFound,
            _ => Self::from_status(status),
        }
    }
}

/// The resolved state of one request in a metadata delete batch.
#[derive(Debug)]
pub enum StorageMetadataBatchEntryOutcome {
    /// Storage explicitly reported success, including idempotent `NOT_FOUND`.
    Succeeded,
    /// Storage explicitly reported a non-idempotent per-entry failure.
    Failed(StorageMetadataError),
    /// The response stream ended before this request received a result.
    Unresolved,
}

/// Per-entry outcomes collected from a delete-batch stream and any error that ended it.
#[derive(Debug)]
pub struct StorageMetadataBatchOutcome {
    /// One entry for each request, in request order.
    pub entries: Vec<StorageMetadataBatchEntryOutcome>,
    pub terminal_error: Option<StorageMetadataError>,
}

fn is_retryable_storage_status(status: &Status) -> bool {
    matches!(
        status.code(),
        Code::Unavailable | Code::DeadlineExceeded | Code::ResourceExhausted | Code::Aborted
    )
}

fn classify_auth_failure(status: &Status) -> Option<&'static str> {
    match status.code() {
        Code::Unauthenticated => Some("authentication token appears expired or invalid"),
        Code::PermissionDenied => Some("caller is not authorized for this storage operation"),
        _ => None,
    }
}

fn storage_status_to_message(status: &Status) -> String {
    if let Some(auth_reason) = classify_auth_failure(status) {
        format!("Storage service call failed: {} ({})", status, auth_reason)
    } else {
        format!("Storage service call failed: {}", status)
    }
}

fn exponential_backoff_delay(
    initial_delay: Duration,
    max_delay: Duration,
    attempt: u32,
) -> Duration {
    let multiplier = 2u32.saturating_pow(attempt.saturating_sub(1));
    initial_delay.saturating_mul(multiplier).min(max_delay)
}

async fn call_storage_with_retry_typed<T, F, Fut, RetryFn>(
    operation: &str,
    max_attempts: u32,
    initial_delay: Duration,
    max_delay: Duration,
    mut should_retry: RetryFn,
    mut call: F,
) -> Result<T, Status>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, Status>>,
    RetryFn: FnMut(&Status) -> bool,
{
    // max_attempts >= 1 and initial_delay/max_delay > 0 are enforced by clap
    // at parse time (StorageRetryArgs), so the values here are already
    // valid — no runtime clamping needed. A max_delay below initial_delay is
    // still possible (clap can't express that cross-field relationship) but
    // is harmless: exponential_backoff_delay's `.min(max_delay)` just caps
    // every retry at that (smaller) value instead of growing.
    for attempt in 1..=max_attempts {
        match call().await {
            Ok(value) => return Ok(value),
            Err(status) => {
                if attempt >= max_attempts || !should_retry(&status) {
                    return Err(status);
                }

                let delay = exponential_backoff_delay(initial_delay, max_delay, attempt);
                // DEBUG, not WARN, and the level is the point. Since MR 211 the
                // outbound client trace layer emits its own unconditional WARN
                // (`rpc.client.call.exception`, with the error chain) for every
                // failed attempt on this path, and this builder re-creates the
                // channel per attempt, so a storage outage was producing two WARN
                // lines per attempt where it used to produce one — roughly double
                // the volume at exactly the moment someone is reading the logs.
                //
                // What this line adds over the layer's is the attempt number and
                // the backoff, which is detail worth having while the retry may
                // still succeed and not worth a WARN until it has not. The
                // exhaustion case above returns the status to a caller that logs
                // it, so nothing about a genuinely failed operation goes quiet.
                debug!(
                    "[RETRY] {} attempt {}/{} failed: {}. Retrying in {:?}",
                    operation, attempt, max_attempts, status, delay
                );
                tokio::time::sleep(delay).await;
            }
        }
    }

    unreachable!("retry loop should return on success or terminal failure")
}

async fn call_storage_with_retry<T, F, Fut>(
    operation: &str,
    call: F,
) -> Result<T, StorageMetadataError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, Status>>,
{
    let retry = &current_runtime_config().storage_retry;
    call_storage_with_retry_typed(
        operation,
        retry.storage_metadata_max_retries,
        retry.storage_metadata_initial_delay,
        retry.storage_metadata_max_delay,
        is_retryable_storage_status,
        call,
    )
    .await
    .map_err(|status| StorageMetadataError::from_status(&status))
}

/// Get all user metadata keys+values for a storage resource.
pub async fn get_all_storage_metadata(
    resource_address: &str,
    auth_token: &str,
) -> Result<Vec<(String, serde_json::Value)>, StorageMetadataError> {
    let user_metadata = get_all_storage_metadata_values(resource_address, auth_token).await?;

    let mut out = Vec::with_capacity(user_metadata.len());
    for (key, meta) in user_metadata {
        let json_value = meta
            .value
            .as_ref()
            .map(crate::utils::prost_to_json_value)
            .unwrap_or(serde_json::Value::Null);
        out.push((key, json_value));
    }
    Ok(out)
}

/// Get all metadata in its protobuf wire representation for internal readers that
/// already parse `prost_types::Value` and should share the same channel/retry policy.
pub(crate) async fn get_all_storage_metadata_values(
    resource_address: &str,
    auth_token: &str,
) -> Result<
    std::collections::HashMap<String, metadata_proto::UserMetadataValue>,
    StorageMetadataError,
> {
    let resource_address = canonicalize_resource_address(resource_address);
    let response = call_storage_with_retry("GetMetadata (enumerate)", || {
        let resource_address = resource_address.clone();
        async move {
            let channel = super::create_traced_storage_channel()
                .await
                .map_err(Status::unavailable)?;
            let mut client = MetadataServiceClient::new(channel);

            let mut request = tonic::Request::new(metadata_proto::GetMetadataRequest {
                uri: resource_address,
                user_metadata_keys: vec![],
            });
            super::add_auth_headers(&mut request, auth_token).map_err(Status::internal)?;
            let response = client.get_metadata(request).await?;
            Ok(response.into_inner())
        }
    })
    .await?;
    Ok(response.user_metadata)
}

/// Update metadata for a storage resource.
pub async fn update_storage_metadata(
    resource_address: &str,
    key: &str,
    value: &serde_json::Value,
    auth_token: &str,
) -> Result<(), StorageMetadataError> {
    let resource_address = canonicalize_resource_address(resource_address);
    let prost_value = utils::json_to_prost_value(value)
        .map_err(|e| StorageMetadataError::Failed(format!("Failed to convert metadata: {e}")))?;

    call_storage_with_retry("UpdateMetadata", || {
        let prost_value = prost_value.clone();
        let resource_address = resource_address.clone();
        async move {
            let channel = super::create_traced_storage_channel()
                .await
                .map_err(Status::unavailable)?;
            let mut client = MetadataServiceClient::new(channel);

            let mut request = tonic::Request::new(metadata_proto::UpdateMetadataRequest {
                uri: resource_address,
                user_metadata_key: key.to_string(),
                user_metadata: Some(prost_value),
                expected_etag: None,
            });
            super::add_auth_headers(&mut request, auth_token).map_err(Status::internal)?;
            client.update_metadata(request).await?;
            Ok(())
        }
    })
    .await
}

/// Remove multiple metadata keys in a single batch RPC.
///
/// Each entry is a `(resource_address, metadata_key)` pair. `NOT_FOUND` per-entry is
/// treated as success — the key is already gone, so retrying the full batch is safe.
///
/// Returns `Ok(outcome)` once the response stream is established. The outcome preserves
/// explicit successes, non-`NOT_FOUND` per-entry failures, unresolved request indices,
/// and any error that later terminates the stream. Returns `Err` when the request fails
/// before a response stream is established.
pub async fn remove_storage_metadata_batch(
    deletions: &[(&str, &str)],
    auth_token: &str,
) -> Result<StorageMetadataBatchOutcome, StorageMetadataError> {
    use metadata_proto::{
        batch_error_entry::Code as BatchCode, delete_metadata_batch_response_entry::Outcome,
        resource_location::Resource, DeleteMetadataBatchRequest, DeleteMetadataBatchRequestEntry,
        ResourceLocation,
    };

    let requests = deletions
        .iter()
        .map(|(address, key)| DeleteMetadataBatchRequestEntry {
            resource_location: Some(ResourceLocation {
                resource: Some(Resource::ResourceAddress(canonicalize_resource_address(
                    address,
                ))),
            }),
            user_metadata_key: key.to_string(),
            expected_etag: None,
            namespace: None,
        })
        .collect();

    let channel = super::create_traced_storage_channel()
        .await
        .map_err(|e| StorageMetadataError::Failed(format!("Failed to connect to storage: {e}")))?;

    let mut client = MetadataServiceClient::new(channel);

    let mut request = tonic::Request::new(DeleteMetadataBatchRequest { requests });
    super::add_auth_headers(&mut request, auth_token).map_err(StorageMetadataError::Failed)?;

    let mut stream = client
        .delete_metadata_batch(request)
        .await
        .map_err(|status| StorageMetadataError::from_status(&status))?
        .into_inner();

    let mut entries: Vec<StorageMetadataBatchEntryOutcome> = (0..deletions.len())
        .map(|_| StorageMetadataBatchEntryOutcome::Unresolved)
        .collect();
    let terminal_error = loop {
        let Some(msg) = stream.next().await else {
            break None;
        };
        let msg = match msg {
            Ok(msg) => msg,
            Err(status) => break Some(StorageMetadataError::from_status(&status)),
        };

        for response_entry in msg.results {
            let request_index = response_entry.request_index as usize;
            let Some(outcome) = entries.get_mut(request_index) else {
                warn!(
                    "DeleteMetadataBatch returned out-of-range request index {} for {} requests",
                    request_index,
                    entries.len()
                );
                continue;
            };

            *outcome = match response_entry.outcome {
                Some(Outcome::Success(_)) => StorageMetadataBatchEntryOutcome::Succeeded,
                Some(Outcome::Error(error)) => {
                    let code = BatchCode::try_from(error.code).unwrap_or(BatchCode::Unspecified);
                    if code == BatchCode::NotFound {
                        StorageMetadataBatchEntryOutcome::Succeeded
                    } else {
                        StorageMetadataBatchEntryOutcome::Failed(StorageMetadataError::Failed(
                            error.message,
                        ))
                    }
                }
                None => {
                    StorageMetadataBatchEntryOutcome::Failed(StorageMetadataError::Failed(format!(
                        "DeleteMetadataBatch returned no outcome for request index {request_index}"
                    )))
                }
            };
        }
    };

    Ok(StorageMetadataBatchOutcome {
        entries,
        terminal_error,
    })
}

/// Remove metadata from a storage resource and classify a missing key explicitly.
pub async fn remove_storage_metadata(
    resource_address: &str,
    key: &str,
    auth_token: &str,
) -> Result<(), StorageMetadataError> {
    let resource_address = canonicalize_resource_address(resource_address);
    info!(
        "[DELETE] Removing metadata for resource {} with key {}",
        resource_address, key
    );
    let retry = &current_runtime_config().storage_retry;
    match call_storage_with_retry_typed(
        "DeleteMetadata",
        retry.storage_metadata_max_retries,
        retry.storage_metadata_initial_delay,
        retry.storage_metadata_max_delay,
        is_retryable_storage_status,
        || {
            let resource_address = resource_address.clone();
            async move {
                let channel = super::create_traced_storage_channel()
                    .await
                    .map_err(Status::unavailable)?;
                let mut client = MetadataServiceClient::new(channel);

                let mut request = tonic::Request::new(metadata_proto::DeleteMetadataRequest {
                    uri: resource_address,
                    user_metadata_key: key.to_string(),
                    expected_etag: None,
                });
                super::add_auth_headers(&mut request, auth_token).map_err(Status::internal)?;
                client.delete_metadata(request).await?;
                Ok(())
            }
        },
    )
    .await
    {
        Ok(()) => {
            info!(
                "[OK] Successfully removed storage metadata for resource: {}",
                resource_address
            );
            Ok(())
        }
        Err(status) => Err(StorageMetadataError::from_removal_status(&status)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::mock_storage_server::{
        mock_storage, unique_test_address, MetadataOp,
    };

    #[tokio::test]
    async fn storage_calls_canonicalize_raw_resource_addresses() {
        crate::test_fixtures::init_test_runtime_config("thumbnail-common-metadata-tests");

        let base = unique_test_address("raw-resource-address");
        let raw_address = format!("{base}/9728-pekanno\u{0308}t [w1200 x h400]mm bump_noise.png");
        let canonical_address = crate::resource::canonicalize_resource_address(&raw_address);
        let value = serde_json::json!({ "status": "SUCCESS" });

        update_storage_metadata(&raw_address, "single-delete", &value, "test-token")
            .await
            .expect("update should use the canonical resource address");
        update_storage_metadata(&raw_address, "batch-delete", &value, "test-token")
            .await
            .expect("second update should use the canonical resource address");

        let metadata = get_all_storage_metadata(&raw_address, "test-token")
            .await
            .expect("enumeration should use the canonical resource address");
        assert_eq!(metadata.len(), 2);
        assert!(mock_storage().calls_for(&raw_address).is_empty());
        assert!(mock_storage().has_key(&canonical_address, "single-delete"));

        remove_storage_metadata(&raw_address, "single-delete", "test-token")
            .await
            .expect("single delete should use the canonical resource address");
        let batch = remove_storage_metadata_batch(&[(&raw_address, "batch-delete")], "test-token")
            .await
            .expect("batch delete should use the canonical resource address");
        assert!(matches!(
            batch.entries.as_slice(),
            [StorageMetadataBatchEntryOutcome::Succeeded]
        ));
        assert!(mock_storage().metadata(&canonical_address).is_empty());
    }

    #[tokio::test]
    async fn batch_delete_preserves_failures_when_the_stream_ends_with_an_error() {
        crate::test_fixtures::init_test_runtime_config("thumbnail-common-metadata-tests");

        let address = unique_test_address("batch-stream-error.usd");
        mock_storage().fail(
            MetadataOp::Delete,
            &address,
            Some("failed-key"),
            Code::Internal,
        );
        mock_storage().fail_delete_batch_stream(&address, Code::Unavailable);

        let outcome = remove_storage_metadata_batch(&[(&address, "failed-key")], "test-token")
            .await
            .expect("the established stream should return its partial outcome");

        assert!(matches!(
            &outcome.entries[0],
            StorageMetadataBatchEntryOutcome::Failed(StorageMetadataError::Failed(message))
                if message == "mock storage: injected fault"
        ));
        assert!(matches!(
            outcome.terminal_error,
            Some(StorageMetadataError::Failed(message))
                if message.contains("injected delete-batch stream fault")
        ));
    }
}
