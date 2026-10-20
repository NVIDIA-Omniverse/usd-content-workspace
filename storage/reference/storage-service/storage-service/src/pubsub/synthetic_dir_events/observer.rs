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

use std::sync::Arc;

use tonic::async_trait;

use crate::{
    common::dir_ancestors::ancestor_dirs,
    pubsub::{
        synthetic_dir_events::{accumulator::Accumulator, metrics::Metrics},
        types::{ConsumeRequest, ConsumeResults, Consumer, StorageEvent},
    },
};

/// Observer middleware that tees file events into the synthetic directory events accumulator.
///
/// The accumulator holds possible `dir_created` / `dir_deleted` paths derived from the
/// ancestor directories of the file's resource URL. A separate flush task drains the
/// accumulator periodically and publishes events to the notification service.
pub struct SyntheticDirEventsObserver<S> {
    accumulator: Arc<Accumulator>,
    metrics: Arc<Metrics>,
    inner: S,
}

impl<S> SyntheticDirEventsObserver<S> {
    pub fn new(accumulator: Arc<Accumulator>, metrics: Arc<Metrics>, inner: S) -> Self {
        Self {
            accumulator,
            metrics,
            inner,
        }
    }
}

#[async_trait]
impl<S: Consumer> Consumer for SyntheticDirEventsObserver<S> {
    async fn consume(&self, events: ConsumeRequest) -> ConsumeResults {
        for event in &events {
            match event {
                StorageEvent::ObjectCreated(data) => {
                    let ancestors = ancestor_dirs(&data.resource_address);
                    if !ancestors.is_empty() {
                        self.metrics.accumulated_created.add(ancestors.len() as u64);
                        self.accumulator.insert_created(ancestors);
                    }
                },
                StorageEvent::ObjectDeleted(data) => {
                    let ancestors = ancestor_dirs(&data.resource_address);
                    if !ancestors.is_empty() {
                        self.metrics.accumulated_deleted.add(ancestors.len() as u64);
                        self.accumulator.insert_deleted(ancestors);
                    }
                },
                // Dir events do not carry object activity to accumulate.
                StorageEvent::DirCreated(_) | StorageEvent::DirDeleted(_) => {},
            }
        }

        self.inner.consume(events).await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use time::OffsetDateTime;
    use tonic::async_trait;

    use super::*;
    use crate::pubsub::types::{
        ConsumeResult, ConsumerError, ObjectCreatedEventData, ObjectDeletedEventData, StorageEventSource,
    };

    // ---------- Test helpers (kept together at the top of the module) ----------

    #[derive(Debug, thiserror::Error)]
    #[error("stub publish failure")]
    struct StubPublishError;

    /// Inner consumer that records every event it receives, for verifying forwarding.
    #[derive(Default)]
    struct RecordingInner {
        received: std::sync::Mutex<Vec<StorageEvent>>,
    }

    impl RecordingInner {
        fn new() -> Arc<Self> {
            Arc::new(Self::default())
        }

        fn events(&self) -> Vec<StorageEvent> {
            self.received.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl Consumer for Arc<RecordingInner> {
        async fn consume(&self, events: ConsumeRequest) -> ConsumeResults {
            let len = events.len();
            self.received.lock().unwrap().extend(events);
            (0..len).map(|_| Ok(())).collect()
        }
    }

    /// Inner consumer whose success can be controlled per-call. Records every event regardless,
    /// but optionally returns errors for the first entries.
    struct ControllableInner {
        received: std::sync::Mutex<Vec<StorageEvent>>,
        fail_first_n: std::sync::atomic::AtomicUsize,
    }

    impl ControllableInner {
        fn new(fail_first_n: usize) -> Arc<Self> {
            Arc::new(Self {
                received: std::sync::Mutex::new(Vec::new()),
                fail_first_n: std::sync::atomic::AtomicUsize::new(fail_first_n),
            })
        }

        fn events(&self) -> Vec<StorageEvent> {
            self.received.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl Consumer for Arc<ControllableInner> {
        async fn consume(&self, events: ConsumeRequest) -> ConsumeResults {
            use std::sync::atomic::Ordering::SeqCst;

            let len = events.len();
            self.received.lock().unwrap().extend(events);
            (0..len)
                .map(|_| {
                    let previous = self
                        .fail_first_n
                        .fetch_update(SeqCst, SeqCst, |value| value.checked_sub(1))
                        .unwrap_or_default();
                    if previous > 0 {
                        let error: ConsumerError = Arc::new(StubPublishError);
                        Err(error)
                    } else {
                        Ok(())
                    }
                })
                .collect()
        }
    }

    /// Stub remapper that mimics the production batch expansion and reduction.
    struct StubUrlRemapper<S, F> {
        inner: S,
        rewrite: F,
    }

    #[async_trait]
    impl<S, F> Consumer for StubUrlRemapper<S, F>
    where
        S: Consumer,
        F: Fn(&str) -> Option<String> + Send + Sync + 'static,
    {
        async fn consume(&self, events: ConsumeRequest) -> ConsumeResults {
            let mut expanded = Vec::new();
            let mut result_lengths = Vec::new();
            for event in events {
                let mapped = match &event {
                    StorageEvent::ObjectCreated(data) => (self.rewrite)(&data.resource_address).map(|addr| {
                        StorageEvent::ObjectCreated(ObjectCreatedEventData {
                            source: data.source,
                            resource_address: addr,
                            resource_identity: data.resource_identity.clone(),
                            sequencer: data.sequencer.clone(),
                            time: data.time,
                        })
                    }),
                    StorageEvent::ObjectDeleted(data) => (self.rewrite)(&data.resource_address).map(|addr| {
                        StorageEvent::ObjectDeleted(ObjectDeletedEventData {
                            source: data.source,
                            resource_address: addr,
                            sequencer: data.sequencer.clone(),
                            time: data.time,
                        })
                    }),
                    StorageEvent::DirCreated(_) | StorageEvent::DirDeleted(_) => None,
                };

                let result_length = if let Some(mapped) = mapped {
                    expanded.push(mapped);
                    2
                } else {
                    1
                };
                expanded.push(event);
                result_lengths.push(result_length);
            }

            let expanded_len = expanded.len();
            let results = self.inner.consume(expanded).await;
            debug_assert_eq!(expanded_len, results.len());
            let mut results = results.into_iter();
            result_lengths
                .into_iter()
                .map(|length| {
                    let mut combined = Ok(());
                    for _ in 0..length {
                        if let Some(Err(error)) = results.next() {
                            if combined.is_ok() {
                                combined = Err(error);
                            }
                        }
                    }
                    combined
                })
                .collect()
        }
    }

    async fn consume_one(consumer: &impl Consumer, event: StorageEvent) -> ConsumeResult {
        let mut results = consumer.consume(vec![event]).await;
        assert_eq!(results.len(), 1);
        results.pop().unwrap()
    }

    fn make_observer() -> (
        SyntheticDirEventsObserver<Arc<RecordingInner>>,
        Arc<Accumulator>,
        Arc<RecordingInner>,
    ) {
        let accumulator = Accumulator::new();
        let metrics = Arc::new(Metrics::new());
        let inner = RecordingInner::new();
        let observer = SyntheticDirEventsObserver::new(accumulator.clone(), metrics, inner.clone());
        (observer, accumulator, inner)
    }

    // ---------- Tests ----------

    #[tokio::test]
    async fn object_created_accumulates_ancestors() {
        let (observer, accumulator, _inner) = make_observer();
        consume_one(
            &observer,
            StorageEvent::ObjectCreated(ObjectCreatedEventData {
                source: StorageEventSource::S3,
                resource_address: "https://b.s3.us-west-2.amazonaws.com/a/b/c/file.usd".into(),
                resource_identity: String::new(),
                sequencer: "seq".into(),
                time: OffsetDateTime::now_utc(),
            }),
        )
        .await
        .unwrap();

        let drained = accumulator.drain();
        let created: HashSet<_> = drained.created.into_iter().collect();
        assert_eq!(
            created,
            [
                "https://b.s3.us-west-2.amazonaws.com/a/b/c/",
                "https://b.s3.us-west-2.amazonaws.com/a/b/",
                "https://b.s3.us-west-2.amazonaws.com/a/"
            ]
            .into_iter()
            .map(String::from)
            .collect::<HashSet<_>>()
        );
        assert!(drained.deleted.is_empty());
    }

    #[tokio::test]
    async fn object_deleted_accumulates_into_deleted_set() {
        let (observer, accumulator, _inner) = make_observer();
        consume_one(
            &observer,
            StorageEvent::ObjectDeleted(ObjectDeletedEventData {
                source: StorageEventSource::S3,
                resource_address: "https://b.s3.us-west-2.amazonaws.com/x/y/file".into(),
                sequencer: String::new(),
                time: OffsetDateTime::now_utc(),
            }),
        )
        .await
        .unwrap();

        let drained = accumulator.drain();
        assert!(drained.created.is_empty());
        let deleted: HashSet<_> = drained.deleted.into_iter().collect();
        assert_eq!(
            deleted,
            [
                "https://b.s3.us-west-2.amazonaws.com/x/y/",
                "https://b.s3.us-west-2.amazonaws.com/x/"
            ]
            .into_iter()
            .map(String::from)
            .collect::<HashSet<_>>()
        );
    }

    #[tokio::test]
    async fn root_level_file_produces_no_ancestors() {
        let (observer, accumulator, _inner) = make_observer();
        consume_one(
            &observer,
            StorageEvent::ObjectCreated(ObjectCreatedEventData {
                source: StorageEventSource::S3,
                resource_address: "https://b.s3.us-west-2.amazonaws.com/file.usd".into(),
                resource_identity: String::new(),
                sequencer: String::new(),
                time: OffsetDateTime::now_utc(),
            }),
        )
        .await
        .unwrap();

        let drained = accumulator.drain();
        assert!(drained.created.is_empty());
        assert!(drained.deleted.is_empty());
    }

    #[tokio::test]
    async fn forwards_event_to_inner_consumer() {
        let (observer, _accumulator, inner) = make_observer();
        let event = StorageEvent::ObjectCreated(ObjectCreatedEventData {
            source: StorageEventSource::S3,
            resource_address: "https://b.s3.us-west-2.amazonaws.com/a/file".into(),
            resource_identity: String::new(),
            sequencer: "s".into(),
            time: OffsetDateTime::now_utc(),
        });
        consume_one(&observer, event.clone()).await.unwrap();

        let seen = inner.events();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0], event);
    }

    #[tokio::test]
    async fn forwards_event_even_when_no_ancestors_are_accumulated() {
        let (observer, _accumulator, inner) = make_observer();
        // Root-level file: no ancestors derived, but the event must still be forwarded.
        let event = StorageEvent::ObjectCreated(ObjectCreatedEventData {
            source: StorageEventSource::S3,
            resource_address: "https://b.s3.us-west-2.amazonaws.com/file.usd".into(),
            resource_identity: String::new(),
            sequencer: String::new(),
            time: OffsetDateTime::now_utc(),
        });
        consume_one(&observer, event.clone()).await.unwrap();

        let seen = inner.events();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0], event);
    }

    /// Happy path: when the remapper's first inner call succeeds, the observer accumulates
    /// ancestors for both the rewritten and the original addresses. This is the dual-publish
    /// guarantee the feature documents.
    #[tokio::test]
    async fn url_remapper_accumulates_both_paths_on_success() {
        let accumulator = Accumulator::new();
        let metrics = Arc::new(Metrics::new());
        let inner = ControllableInner::new(0);
        let observer = SyntheticDirEventsObserver::new(accumulator.clone(), metrics, inner.clone());
        let remapper = StubUrlRemapper {
            inner: observer,
            rewrite: |addr: &str| {
                addr.replace("https://logical.example.com", "https://b2.s3.us-west-2.amazonaws.com")
                    .into()
            },
        };

        consume_one(
            &remapper,
            StorageEvent::ObjectCreated(ObjectCreatedEventData {
                source: StorageEventSource::S3,
                resource_address: "https://logical.example.com/a/b/file.usd".into(),
                resource_identity: String::new(),
                sequencer: "s".into(),
                time: OffsetDateTime::now_utc(),
            }),
        )
        .await
        .expect("happy path must not fail");

        let drained = accumulator.drain();
        let created: HashSet<_> = drained.created.into_iter().collect();
        // Both rewritten and original ancestors are present.
        assert!(created.contains("https://b2.s3.us-west-2.amazonaws.com/a/b/"));
        assert!(created.contains("https://b2.s3.us-west-2.amazonaws.com/a/"));
        assert!(created.contains("https://logical.example.com/a/b/"));
        assert!(created.contains("https://logical.example.com/a/"));
        // Inner saw both events forwarded through.
        assert_eq!(inner.events().len(), 2);
    }

    /// A mapped entry still attempts and accumulates both notifications when one fails.
    #[tokio::test]
    async fn url_remapper_attempts_both_paths_when_inner_fails() {
        let accumulator = Accumulator::new();
        let metrics = Arc::new(Metrics::new());
        // Fail only the rewritten entry; the original entry still succeeds.
        let inner = ControllableInner::new(1);
        let observer = SyntheticDirEventsObserver::new(accumulator.clone(), metrics, inner.clone());
        let remapper = StubUrlRemapper {
            inner: observer,
            rewrite: |addr: &str| {
                addr.replace("https://logical.example.com", "https://b2.s3.us-west-2.amazonaws.com")
                    .into()
            },
        };

        let result = consume_one(
            &remapper,
            StorageEvent::ObjectCreated(ObjectCreatedEventData {
                source: StorageEventSource::S3,
                resource_address: "https://logical.example.com/a/b/file.usd".into(),
                resource_identity: String::new(),
                sequencer: "s".into(),
                time: OffsetDateTime::now_utc(),
            }),
        )
        .await;
        assert!(result.is_err(), "remapper must propagate the inner error");

        let drained = accumulator.drain();
        let created: HashSet<_> = drained.created.into_iter().collect();
        assert!(created.contains("https://b2.s3.us-west-2.amazonaws.com/a/b/"));
        assert!(created.contains("https://b2.s3.us-west-2.amazonaws.com/a/"));
        assert!(created.contains("https://logical.example.com/a/b/"));
        assert!(created.contains("https://logical.example.com/a/"));
        assert_eq!(inner.events().len(), 2);
    }
}
