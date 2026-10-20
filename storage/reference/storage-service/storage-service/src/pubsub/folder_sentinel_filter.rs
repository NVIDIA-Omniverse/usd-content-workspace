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

use fluent_uri::Uri;
use storage_core::layer::Layer;
use tonic::async_trait;
use tracing::{debug, error};

use crate::{
    common::{error::PrettyPrintErrorExt, vec::VecExt},
    pubsub::types::{ConsumeRequest, ConsumeResults, Consumer, StorageEvent, StorageEventSource},
};

pub struct FolderSentinelFilterLayer;

impl<S> Layer<S> for FolderSentinelFilterLayer {
    type Service = FolderSentinelFilter<S>;

    fn layer(&self, inner: S) -> Self::Service {
        FolderSentinelFilter { inner }
    }
}

pub struct FolderSentinelFilter<S> {
    inner: S,
}

#[async_trait]
impl<S: Consumer> Consumer for FolderSentinelFilter<S> {
    async fn consume(&self, mut events: ConsumeRequest) -> ConsumeResults {
        if events.is_empty() {
            return Vec::new();
        }

        let input_len = events.len();
        let mut forwarded_indices = None;

        events.retain_indexed(|index, event| {
            let sentinel = match &event {
                StorageEvent::ObjectCreated(data) => is_folder_sentinel(data.source, &data.resource_address),
                StorageEvent::ObjectDeleted(data) => is_folder_sentinel(data.source, &data.resource_address),
                StorageEvent::DirCreated(_) | StorageEvent::DirDeleted(_) => false,
            };

            if sentinel {
                debug!(?event, "ignoring folder sentinel notification");
                if forwarded_indices.is_none() {
                    forwarded_indices = Some((0..index).collect::<Vec<_>>());
                }
                false
            } else {
                if let Some(forwarded_indices) = forwarded_indices.as_mut() {
                    forwarded_indices.push(index);
                }
                true
            }
        });

        let Some(forwarded_indices) = forwarded_indices else {
            return self.inner.consume(events).await;
        };

        let mut results = vec![Ok(()); input_len];
        if events.is_empty() {
            return results;
        }

        let forwarded_results = self.inner.consume(events).await;
        debug_assert_eq!(forwarded_indices.len(), forwarded_results.len());
        for (index, result) in forwarded_indices.into_iter().zip(forwarded_results) {
            results[index] = result;
        }
        results
    }
}

fn is_folder_sentinel(source: StorageEventSource, resource_address: &str) -> bool {
    let url = match Uri::parse(resource_address) {
        Ok(url) => url,
        Err(error) => {
            error!(error = %error.pretty(), ?resource_address, "failed to parse storage event resource address");
            return false;
        },
    };
    let path = url.path().as_str();

    match source {
        StorageEventSource::S3 => path.ends_with('/'),
        StorageEventSource::AzureBlob => path.ends_with("/.folder"),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use rstest::rstest;
    use time::OffsetDateTime;

    use super::*;
    use crate::pubsub::types::{ConsumeResult, ObjectCreatedEventData};

    #[derive(Debug, thiserror::Error)]
    #[error("{0}")]
    struct StubConsumerError(&'static str);

    struct StubConsumer {
        calls: Arc<Mutex<Vec<ConsumeRequest>>>,
        results: ConsumeResults,
    }

    #[async_trait]
    impl Consumer for StubConsumer {
        async fn consume(&self, events: ConsumeRequest) -> ConsumeResults {
            self.calls.lock().unwrap().push(events);
            self.results.clone()
        }
    }

    fn object_created(resource_address: &str) -> StorageEvent {
        StorageEvent::ObjectCreated(ObjectCreatedEventData {
            source: StorageEventSource::S3,
            resource_address: resource_address.into(),
            resource_identity: String::new(),
            sequencer: String::new(),
            time: OffsetDateTime::UNIX_EPOCH,
        })
    }

    fn failure(message: &'static str) -> ConsumeResult {
        Err(Arc::new(StubConsumerError(message)))
    }

    fn filter(results: ConsumeResults) -> (FolderSentinelFilter<StubConsumer>, Arc<Mutex<Vec<ConsumeRequest>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        (
            FolderSentinelFilter {
                inner: StubConsumer {
                    calls: calls.clone(),
                    results,
                },
            },
            calls,
        )
    }

    #[tokio::test]
    async fn test_all_filtered_events_succeed_without_calling_inner() {
        let (filter, calls) = filter(Vec::new());

        let results = filter
            .consume(vec![
                object_created("https://bucket.s3.us-west-1.amazonaws.com/a/"),
                object_created("https://bucket.s3.us-west-1.amazonaws.com/b/"),
            ])
            .await;

        assert_eq!(results.len(), 2);
        assert!(results.into_iter().all(|result| result.is_ok()));
        assert!(calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_filtered_events_succeed_in_partially_retained_batch() {
        let retained = object_created("https://bucket.s3.us-west-1.amazonaws.com/file.usd");
        let (filter, calls) = filter(vec![failure("retained event failed")]);

        let results = filter
            .consume(vec![
                object_created("https://bucket.s3.us-west-1.amazonaws.com/folder/"),
                retained.clone(),
            ])
            .await;

        assert!(results[0].is_ok());
        assert_eq!(results[1].as_ref().unwrap_err().to_string(), "retained event failed");
        assert_eq!(*calls.lock().unwrap(), vec![vec![retained]]);
    }

    #[tokio::test]
    async fn test_partially_retained_results_are_merged_in_input_order() {
        let retained_a = object_created("https://bucket.s3.us-west-1.amazonaws.com/a.usd");
        let retained_b = object_created("https://bucket.s3.us-west-1.amazonaws.com/b.usd");
        let retained_c = object_created("https://bucket.s3.us-west-1.amazonaws.com/c.usd");
        let (filter, calls) = filter(vec![failure("first failure"), Ok(()), failure("last failure")]);

        let results = filter
            .consume(vec![
                retained_a.clone(),
                object_created("https://bucket.s3.us-west-1.amazonaws.com/a/"),
                retained_b.clone(),
                object_created("https://bucket.s3.us-west-1.amazonaws.com/b/"),
                retained_c.clone(),
            ])
            .await;

        let errors: Vec<_> = results
            .iter()
            .map(|result| result.as_ref().err().map(ToString::to_string))
            .collect();
        assert_eq!(
            errors,
            vec![
                Some("first failure".into()),
                None,
                None,
                None,
                Some("last failure".into()),
            ]
        );
        assert_eq!(*calls.lock().unwrap(), vec![vec![retained_a, retained_b, retained_c]]);
    }

    #[rstest]
    #[case(StorageEventSource::S3, "https://bucket.s3.us-west-1.amazonaws.com/folder/", true)]
    #[case(StorageEventSource::S3, "omniverse://host/mapped/folder/", true)]
    #[case(StorageEventSource::S3, "https://bucket.s3.us-west-1.amazonaws.com/.folder", false)]
    #[case(StorageEventSource::S3, "https://bucket.s3.us-west-1.amazonaws.com/", true)]
    #[case(
        StorageEventSource::AzureBlob,
        "https://account.blob.core.windows.net/container/folder/.folder",
        true
    )]
    #[case(StorageEventSource::AzureBlob, "omniverse://host/mapped/folder/.folder", true)]
    #[case(
        StorageEventSource::AzureBlob,
        "https://account.blob.core.windows.net/container/",
        false
    )]
    #[case(
        StorageEventSource::AzureBlob,
        "https://account.blob.core.windows.net/container/folder/",
        false
    )]
    #[case(StorageEventSource::AzureBlob, "not a url/.folder", false)]
    fn test_identifies_provider_specific_folder_sentinels(
        #[case] source: StorageEventSource,
        #[case] address: &str,
        #[case] expected: bool,
    ) {
        assert_eq!(is_folder_sentinel(source, address), expected);
    }
}
