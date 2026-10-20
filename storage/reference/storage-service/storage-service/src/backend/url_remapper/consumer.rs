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

use tonic::async_trait;
use tracing::debug;

use crate::{
    backend::{metadata::Metadata, url_remapper::service::UrlRemapperService},
    common::url_remapper::RewriteMode,
    pubsub::types::{
        ConsumeRequest, ConsumeResults, Consumer, DirEventData, ObjectCreatedEventData, ObjectDeletedEventData,
        StorageEvent,
    },
};

impl<T: Metadata, S> UrlRemapperService<T, S> {
    fn rewrite_object_deleted_event_data(&self, data: &ObjectDeletedEventData) -> Option<ObjectDeletedEventData> {
        let mapped_address = self
            .mapper
            .rev_rewrite_unversioned_url(&data.resource_address, RewriteMode::Precise)?;

        Some(ObjectDeletedEventData {
            source: data.source,
            resource_address: mapped_address,
            sequencer: data.sequencer.clone(),
            time: data.time,
        })
    }

    fn rewrite_object_created_event_data(&self, data: &ObjectCreatedEventData) -> Option<ObjectCreatedEventData> {
        let mapped_address = self
            .mapper
            .rev_rewrite_unversioned_url(&data.resource_address, RewriteMode::Precise)?;
        let mapped_identity = self
            .mapper
            .rev_rewrite_unversioned_url(&data.resource_identity, RewriteMode::Precise)
            .unwrap_or_else(|| data.resource_identity.clone());

        Some(ObjectCreatedEventData {
            source: data.source,
            resource_address: mapped_address,
            resource_identity: mapped_identity,
            sequencer: data.sequencer.clone(),
            time: data.time,
        })
    }

    fn rewrite_dir_event_data(&self, data: &DirEventData) -> Option<DirEventData> {
        let mapped_address = self
            .mapper
            .rev_rewrite_unversioned_url(&data.resource_address, RewriteMode::Precise)?;
        Some(DirEventData {
            resource_address: mapped_address,
            time: data.time,
        })
    }
}

#[async_trait]
impl<T: Metadata, S: Consumer> Consumer for UrlRemapperService<T, S> {
    async fn consume(&self, mut events: ConsumeRequest) -> ConsumeResults {
        let request_count = events.len();
        let mut mapped_indices = None;
        for request_index in 0..request_count {
            let mapped = match &events[request_index] {
                StorageEvent::ObjectCreated(original) => {
                    self.rewrite_object_created_event_data(original).map(|mapped| {
                        debug!(?original, ?mapped, "object created event remapped");
                        StorageEvent::ObjectCreated(mapped)
                    })
                },
                StorageEvent::ObjectDeleted(original) => {
                    self.rewrite_object_deleted_event_data(original).map(|mapped| {
                        debug!(?original, ?mapped, "object deleted event remapped");
                        StorageEvent::ObjectDeleted(mapped)
                    })
                },
                StorageEvent::DirCreated(original) => self.rewrite_dir_event_data(original).map(|mapped| {
                    debug!(?original, ?mapped, "dir created event remapped");
                    StorageEvent::DirCreated(mapped)
                }),
                StorageEvent::DirDeleted(original) => self.rewrite_dir_event_data(original).map(|mapped| {
                    debug!(?original, ?mapped, "dir deleted event remapped");
                    StorageEvent::DirDeleted(mapped)
                }),
            };

            if let Some(mapped) = mapped {
                let mapped_indices = mapped_indices.get_or_insert_with(|| {
                    events.reserve(request_count);
                    vec![None; request_count]
                });
                mapped_indices[request_index] = Some(events.len());
                events.push(mapped);
            }
        }

        let Some(mapped_indices) = mapped_indices else {
            return self.inner.consume(events).await;
        };

        let event_count = events.len();
        let mut results = self.inner.consume(events).await;
        debug_assert_eq!(event_count, results.len());

        for (request_index, mapped_index) in mapped_indices.into_iter().enumerate() {
            let Some(mapped_index) = mapped_index else {
                continue;
            };
            let mapped_result = std::mem::replace(&mut results[mapped_index], Ok(()));
            if results[request_index].is_ok() && mapped_result.is_err() {
                results[request_index] = mapped_result;
            }
        }

        results.truncate(request_count);
        results
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use time::OffsetDateTime;

    use super::*;
    use crate::{
        common::url_remapper::test_helpers::{make_remapper, UnreachableMetadata},
        pubsub::types::{ConsumeResult, StorageEventSource},
    };

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

    fn object_deleted(resource_address: &str) -> StorageEvent {
        StorageEvent::ObjectDeleted(ObjectDeletedEventData {
            source: StorageEventSource::S3,
            resource_address: resource_address.into(),
            sequencer: String::new(),
            time: OffsetDateTime::UNIX_EPOCH,
        })
    }

    fn failure(message: &'static str) -> ConsumeResult {
        Err(Arc::new(StubConsumerError(message)))
    }

    fn consumer(
        results: ConsumeResults,
    ) -> (
        UrlRemapperService<UnreachableMetadata, StubConsumer>,
        Arc<Mutex<Vec<ConsumeRequest>>>,
    ) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        (
            UrlRemapperService {
                mapper: make_remapper(
                    &[("omniverse://host/mapped/", "https://backend.example/bucket/")],
                    false,
                ),
                address_filter: None,
                inner: StubConsumer {
                    calls: calls.clone(),
                    results,
                },
            },
            calls,
        )
    }

    async fn consume_partially_remapped(original_result: ConsumeResult, mapped_result: ConsumeResult) -> ConsumeResult {
        let original = object_deleted("https://backend.example/bucket/file.usd");
        let unmapped = object_deleted("https://other.example/file.usd");
        let mapped = object_deleted("omniverse://host/mapped/file.usd");
        let (consumer, calls) = consumer(vec![original_result, Ok(()), mapped_result]);

        let results = consumer.consume(vec![original.clone(), unmapped.clone()]).await;

        assert_eq!(*calls.lock().unwrap(), vec![vec![original, unmapped, mapped]]);
        assert_eq!(results.len(), 2);
        assert!(results[1].is_ok());

        results
            .into_iter()
            .next()
            .expect("response must contain the original event result")
    }

    #[tokio::test]
    async fn test_empty_batch_returns_empty_response() {
        let (consumer, calls) = consumer(Vec::new());

        let results = consumer.consume(Vec::new()).await;

        assert!(results.is_empty());
        assert_eq!(*calls.lock().unwrap(), vec![Vec::new()]);
    }

    #[tokio::test]
    async fn test_unmapped_batch_returns_underlying_response() {
        let event = object_deleted("https://other.example/file.usd");
        let (consumer, calls) = consumer(vec![failure("underlying failure")]);

        let results = consumer.consume(vec![event.clone()]).await;

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].as_ref().unwrap_err().to_string(), "underlying failure");
        assert_eq!(*calls.lock().unwrap(), vec![vec![event]]);
    }

    #[tokio::test]
    async fn test_original_and_remapped_success_is_success() {
        assert!(consume_partially_remapped(Ok(()), Ok(())).await.is_ok());
    }

    #[tokio::test]
    async fn test_original_success_and_remapped_failure_is_failure() {
        let result = consume_partially_remapped(Ok(()), failure("remapped failure")).await;

        assert_eq!(result.unwrap_err().to_string(), "remapped failure");
    }

    #[tokio::test]
    async fn test_original_failure_and_remapped_success_is_failure() {
        let result = consume_partially_remapped(failure("original failure"), Ok(())).await;

        assert_eq!(result.unwrap_err().to_string(), "original failure");
    }

    #[tokio::test]
    async fn test_original_and_remapped_failure_returns_original_failure() {
        let result = consume_partially_remapped(failure("original failure"), failure("remapped failure")).await;

        assert_eq!(result.unwrap_err().to_string(), "original failure");
    }
}
