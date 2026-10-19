// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

pub mod v1alpha {
    use std::collections::HashMap;
    use std::fmt;

    use crate::traits::{Request, Result};
    use futures::StreamExt;
    pub use storage_core_protos::google::protobuf::{value, ListValue, NullValue, Struct, Value};
    pub use storage_core_protos::nvidia::omniverse::storage::metadata::v1alpha::*;

    crate::generate_storage_trait! {
        Metadata {
            async fn get_metadata(&self, request: Request<GetMetadataRequest>) -> Result<GetMetadataResponse>;
            async fn update_metadata(&self, request: Request<UpdateMetadataRequest>) -> Result<UpdateMetadataResponse>;
            async fn delete_metadata(&self, request: Request<DeleteMetadataRequest>) -> Result<DeleteMetadataResponse>;
            async fn update_metadata_batch(&self, request: Request<UpdateMetadataBatchRequest>) -> Result<UpdateMetadataBatchResponse>;
        }
    }

    /// Maximum number of entries allowed in a single `UpdateMetadataBatch` request.
    pub const MAX_BATCH_ENTRIES: usize = 256;

    /// Maximum number of concurrent per-URI worker tasks when fanning out a batch
    /// request.
    pub const MAX_BATCH_CONCURRENCY: usize = 32;

    /// Fan an `UpdateMetadataBatchRequest` out into per-entry `update_metadata`
    /// calls on `service`. This is an explicit utility for implementations that
    /// choose per-entry behavior; it is not the trait-level default.
    ///
    /// Entries are grouped by URI and processed serially within each group; groups
    /// themselves run in parallel up to `MAX_BATCH_CONCURRENCY`.
    ///
    /// Per the proto contract, "each entry is processed independently with the same
    /// rules as a single UpdateMetadata call" — and only conflicting writes to the
    /// same `(uri, user_metadata_key)` are undefined, not different keys on the same
    /// URI. Because each backend's `update_metadata` is implemented as
    /// HEAD-merge-PUT, two concurrent writes to the same URI would let one clobber
    /// the other's keys; we serialize same-URI entries to honor the per-entry
    /// independence the contract promises.
    ///
    /// Results are collected in group-completion order, with no global ordering
    /// guarantee per proto contract.
    pub async fn update_metadata_batch_fanout<S>(
        service: &S,
        request: Request<UpdateMetadataBatchRequest>,
    ) -> Result<UpdateMetadataBatchResponse>
    where
        S: Metadata + ?Sized,
    {
        let (metadata, extensions, batch) = request.0.into_parts();
        validate_metadata_batch_size(batch.requests.len())?;

        // Group entries by URI, preserving input order within each group. A Vec
        // ordered by first-seen URI plus a HashMap that owns the entries keeps the
        // lookup O(1) without pulling in an ordered-map dependency.
        let mut order: Vec<String> = Vec::new();
        let mut groups: HashMap<String, Vec<(usize, UpdateMetadataRequest)>> = HashMap::new();
        for (idx, entry) in batch.requests.into_iter().enumerate() {
            let uri = entry.uri.clone();
            if !groups.contains_key(&uri) {
                order.push(uri.clone());
            }
            groups.entry(uri).or_default().push((idx, entry));
        }
        let grouped: Vec<Vec<(usize, UpdateMetadataRequest)>> = order
            .into_iter()
            .map(|uri| groups.remove(&uri).unwrap_or_default())
            .collect();

        let grouped_results: Vec<Vec<UpdateMetadataBatchEntryResult>> = futures::stream::iter(grouped)
            .map(|entries| {
                let metadata = metadata.clone();
                let extensions = extensions.clone();
                async move {
                    let mut results = Vec::with_capacity(entries.len());
                    for (request_index, entry) in entries {
                        let entry_req = tonic::Request::from_parts(metadata.clone(), extensions.clone(), entry);
                        let outcome = match service.update_metadata(Request(entry_req)).await {
                            Ok(success) => update_metadata_batch_entry_result::Outcome::Success(success),
                            Err(error) => {
                                update_metadata_batch_entry_result::Outcome::Error(batch_entry_error(error.as_ref()))
                            },
                        };
                        results.push(UpdateMetadataBatchEntryResult {
                            outcome: Some(outcome),
                            request_index: request_index as u32,
                        });
                    }
                    results
                }
            })
            .buffer_unordered(MAX_BATCH_CONCURRENCY)
            .collect()
            .await;

        Ok(UpdateMetadataBatchResponse {
            results: grouped_results.into_iter().flatten().collect(),
        })
    }

    /// Build a per-entry batch error from a storage error, mapping the tonic status code
    /// to the closed `UpdateMetadataBatchEntryError.Code` enum.
    pub fn batch_entry_error(error: &dyn crate::traits::StorageErrorExt) -> UpdateMetadataBatchEntryError {
        use update_metadata_batch_entry_error::Code;

        let status = error.tonic_status();
        let code = match status.code() {
            tonic::Code::InvalidArgument => Code::InvalidArgument,
            tonic::Code::NotFound => Code::NotFound,
            tonic::Code::PermissionDenied => Code::PermissionDenied,
            tonic::Code::FailedPrecondition => Code::FailedPrecondition,
            tonic::Code::Unavailable => Code::Unavailable,
            _ => Code::Internal,
        };

        UpdateMetadataBatchEntryError {
            code: code as i32,
            message: status.message().to_string(),
        }
    }

    /// Reject oversized batches at the trait boundary.
    pub fn validate_metadata_batch_size(len: usize) -> Result<()> {
        if len > MAX_BATCH_ENTRIES {
            Err(BatchTooLargeError { len }.into())
        } else {
            Ok(())
        }
    }

    #[derive(Debug)]
    struct BatchTooLargeError {
        len: usize,
    }

    impl fmt::Display for BatchTooLargeError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(
                f,
                "UpdateMetadataBatch.requests has {} entries; max is {MAX_BATCH_ENTRIES}",
                self.len
            )
        }
    }

    impl std::error::Error for BatchTooLargeError {}

    impl crate::traits::StorageErrorExt for BatchTooLargeError {
        fn tonic_status(&self) -> tonic::Status {
            crate::traits::error::status_with_default_error_info(
                tonic::Code::InvalidArgument,
                self.to_string(),
                "InvalidArgument",
            )
        }

        fn http_status(&self) -> http::StatusCode {
            http::StatusCode::BAD_REQUEST
        }

        fn get_size(&self) -> usize {
            std::mem::size_of_val(self)
        }
    }

    #[cfg(test)]
    mod tests {
        use std::collections::HashMap;
        use std::fmt;
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

        use super::*;
        use crate::traits::{async_trait, StorageError, StorageErrorExt};

        #[derive(Debug)]
        struct TestError {
            code: tonic::Code,
            message: String,
        }

        impl fmt::Display for TestError {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.message)
            }
        }

        impl std::error::Error for TestError {}

        impl StorageErrorExt for TestError {
            fn tonic_status(&self) -> tonic::Status {
                tonic::Status::new(self.code, self.message.clone())
            }

            fn http_status(&self) -> http::StatusCode {
                http::StatusCode::INTERNAL_SERVER_ERROR
            }

            fn get_size(&self) -> usize {
                std::mem::size_of_val(self) + self.message.len()
            }
        }

        struct CountingMetadata {
            calls: AtomicUsize,
            outcomes: HashMap<String, std::result::Result<String, tonic::Code>>,
        }

        #[async_trait]
        impl Metadata for CountingMetadata {
            fn inner(&self) -> Option<&dyn Metadata> {
                None
            }

            async fn update_metadata(&self, request: Request<UpdateMetadataRequest>) -> Result<UpdateMetadataResponse> {
                self.calls.fetch_add(1, Ordering::SeqCst);
                let key = request.get_ref().user_metadata_key.clone();
                match self.outcomes.get(&key) {
                    Some(Ok(etag)) => Ok(UpdateMetadataResponse { etag: etag.clone() }),
                    Some(Err(code)) => Err(StorageError::from(TestError {
                        code: *code,
                        message: format!("test failure for key {key}"),
                    })),
                    None => panic!("unexpected key {key}"),
                }
            }
        }

        struct SerialMetadata {
            calls: AtomicUsize,
            in_flight: AtomicBool,
        }

        #[async_trait]
        impl Metadata for SerialMetadata {
            fn inner(&self) -> Option<&dyn Metadata> {
                None
            }

            async fn update_metadata(&self, request: Request<UpdateMetadataRequest>) -> Result<UpdateMetadataResponse> {
                if self.in_flight.swap(true, Ordering::SeqCst) {
                    panic!("same-URI batch entries ran concurrently");
                }
                self.calls.fetch_add(1, Ordering::SeqCst);
                tokio::task::yield_now().await;
                self.in_flight.store(false, Ordering::SeqCst);
                Ok(UpdateMetadataResponse {
                    etag: format!("etag-{}", request.get_ref().user_metadata_key),
                })
            }
        }

        fn make_request(uri: &str, key: &str) -> UpdateMetadataRequest {
            UpdateMetadataRequest {
                uri: uri.into(),
                user_metadata_key: key.into(),
                user_metadata: Some(Value {
                    kind: Some(value::Kind::StringValue("value".into())),
                }),
                expected_etag: None,
            }
        }

        #[tokio::test]
        async fn fanout_invokes_update_metadata_per_entry_and_preserves_indices() {
            let service = CountingMetadata {
                calls: AtomicUsize::new(0),
                outcomes: HashMap::from([
                    ("a".into(), Ok("etag-a".into())),
                    ("b".into(), Ok("etag-b".into())),
                    ("c".into(), Ok("etag-c".into())),
                ]),
            };
            let request = Request::new(UpdateMetadataBatchRequest {
                requests: vec![
                    make_request("s3://bucket/a", "a"),
                    make_request("s3://bucket/b", "b"),
                    make_request("s3://bucket/c", "c"),
                ],
            });

            let response = update_metadata_batch_fanout(&service, request).await.unwrap();

            assert_eq!(service.calls.load(Ordering::SeqCst), 3);
            let etags_by_index: HashMap<u32, String> = response
                .results
                .into_iter()
                .map(|result| {
                    let etag = match result.outcome.unwrap() {
                        update_metadata_batch_entry_result::Outcome::Success(success) => success.etag,
                        update_metadata_batch_entry_result::Outcome::Error(error) => {
                            panic!("unexpected error result: {error:?}")
                        },
                    };
                    (result.request_index, etag)
                })
                .collect();
            assert_eq!(
                etags_by_index,
                HashMap::from([
                    (0, "etag-a".to_string()),
                    (1, "etag-b".to_string()),
                    (2, "etag-c".to_string()),
                ])
            );
        }

        #[tokio::test]
        async fn fanout_reports_per_entry_errors() {
            let service = CountingMetadata {
                calls: AtomicUsize::new(0),
                outcomes: HashMap::from([
                    ("ok".into(), Ok("ok-etag".into())),
                    ("bad".into(), Err(tonic::Code::FailedPrecondition)),
                ]),
            };
            let request = Request::new(UpdateMetadataBatchRequest {
                requests: vec![
                    make_request("s3://bucket/ok", "ok"),
                    make_request("s3://bucket/bad", "bad"),
                ],
            });

            let response = update_metadata_batch_fanout(&service, request).await.unwrap();

            assert_eq!(service.calls.load(Ordering::SeqCst), 2);
            let results_by_index: HashMap<u32, UpdateMetadataBatchEntryResult> = response
                .results
                .into_iter()
                .map(|result| (result.request_index, result))
                .collect();
            assert!(matches!(
                results_by_index[&0].outcome.as_ref().unwrap(),
                update_metadata_batch_entry_result::Outcome::Success(success) if success.etag == "ok-etag"
            ));
            assert!(matches!(
                results_by_index[&1].outcome.as_ref().unwrap(),
                update_metadata_batch_entry_result::Outcome::Error(error)
                    if error.code == update_metadata_batch_entry_error::Code::FailedPrecondition as i32
            ));
        }

        #[tokio::test]
        async fn fanout_handles_empty_batch_without_invoking_service() {
            let service = CountingMetadata {
                calls: AtomicUsize::new(0),
                outcomes: HashMap::new(),
            };
            let request = Request::new(UpdateMetadataBatchRequest { requests: Vec::new() });

            let response = update_metadata_batch_fanout(&service, request).await.unwrap();

            assert!(response.results.is_empty());
            assert_eq!(service.calls.load(Ordering::SeqCst), 0);
        }

        #[tokio::test]
        async fn fanout_rejects_oversized_batches_before_updates() {
            let service = CountingMetadata {
                calls: AtomicUsize::new(0),
                outcomes: HashMap::new(),
            };
            let request = Request::new(UpdateMetadataBatchRequest {
                requests: (0..=MAX_BATCH_ENTRIES)
                    .map(|i| make_request("s3://bucket/object", &format!("key-{i}")))
                    .collect(),
            });

            let error = update_metadata_batch_fanout(&service, request)
                .await
                .expect_err("oversized batch should fail");

            assert_eq!(error.tonic_status().code(), tonic::Code::InvalidArgument);
            assert_eq!(service.calls.load(Ordering::SeqCst), 0);
        }

        #[tokio::test]
        async fn fanout_serializes_entries_for_the_same_uri() {
            let service = SerialMetadata {
                calls: AtomicUsize::new(0),
                in_flight: AtomicBool::new(false),
            };
            let request = Request::new(UpdateMetadataBatchRequest {
                requests: vec![
                    make_request("s3://bucket/shared", "a"),
                    make_request("s3://bucket/shared", "b"),
                    make_request("s3://bucket/shared", "c"),
                ],
            });

            let response = update_metadata_batch_fanout(&service, request).await.unwrap();

            assert_eq!(service.calls.load(Ordering::SeqCst), 3);
            assert_eq!(response.results.len(), 3);
        }
    }
}
