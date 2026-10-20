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

use std::{collections::HashSet, sync::Arc};

use storage_service_metrics::{
    status::result::{ResultMetrics, Status},
    types::Observe,
};
use time::OffsetDateTime;
use tokio::time::{interval, MissedTickBehavior};
use tracing::{debug, warn};

use crate::{
    common::error::PrettyPrintErrorExt,
    pubsub::{
        synthetic_dir_events::{
            accumulator::{Accumulator, DrainedPaths},
            metrics::Metrics,
        },
        types::{AnyConsumer, ConsumeResult, Consumer, DirEventData, StorageEvent},
    },
};

/// Owns the per-flush state (consumer + metrics) so the helpers below can share
/// `&self` rather than free functions that re-pass the same arguments at every call site.
pub(crate) struct Flusher {
    consumer: AnyConsumer,
    metrics: Arc<Metrics>,
}

impl Flusher {
    pub(crate) fn new(consumer: AnyConsumer, metrics: Arc<Metrics>) -> Self {
        Self { consumer, metrics }
    }

    pub(crate) async fn run(self, accumulator: Arc<Accumulator>, flush_interval: std::time::Duration) {
        let mut ticker = interval(flush_interval);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        // `tokio::time::interval` fires the first tick immediately; consume it so the first real
        // flush happens one interval after startup. Not strictly necessary (an empty accumulator
        // is a no-op) but avoids a redundant zero-sized sample at t=0 in the histograms.
        ticker.tick().await;

        loop {
            ticker.tick().await;
            self.flush_once(&accumulator).await;
        }
    }

    pub(crate) async fn flush_once(&self, accumulator: &Accumulator) {
        let started_at = self.metrics.flush_duration.start();
        let DrainedPaths { created, deleted } = accumulator.drain();

        // Record batch sizes every flush (including zero-size), so downstream dashboards can see
        // the shape of the workload including idle intervals.
        self.metrics.flush_batch_size_created.record(created.len() as u64);
        self.metrics.flush_batch_size_deleted.record(deleted.len() as u64);

        if created.is_empty() && deleted.is_empty() {
            started_at.stop_and_record();
            return;
        }

        let now = OffsetDateTime::now_utc();
        let created_count = created.len();
        let events: Vec<_> = build_events(created, now, StorageEvent::DirCreated)
            .chain(build_events(deleted, now, StorageEvent::DirDeleted))
            .collect();

        let event_count = events.len();
        let results = self.consumer.consume(events).await;
        debug_assert_eq!(event_count, results.len());

        let (created_results, deleted_results) = results.split_at(created_count);
        let created_outcome =
            Self::record_outcomes(created_results, &self.metrics.publish_results_created, "dir_created");
        let deleted_outcome =
            Self::record_outcomes(deleted_results, &self.metrics.publish_results_deleted, "dir_deleted");

        debug!(
            ?created_outcome,
            ?deleted_outcome,
            "synthetic dir events flush completed"
        );
        started_at.stop_and_record();
    }

    fn record_outcomes(results: &[ConsumeResult], results_metric: &ResultMetrics, kind: &'static str) -> FlushOutcome {
        let attempted = results.len();
        let mut total_succeeded = 0usize;
        let mut total_failed = 0usize;

        for result in results {
            match result {
                Ok(()) => {
                    total_succeeded += 1;
                    results_metric.observe(&Status::Ok);
                },
                Err(error) => {
                    total_failed += 1;
                    results_metric.observe(&Status::Err);
                    warn!(
                        kind,
                        error = %error.pretty(),
                        "batch publish entry failed while flushing synthetic dir events",
                    );
                },
            }
        }

        FlushOutcome {
            attempted,
            succeeded: total_succeeded,
            failed: total_failed,
        }
    }
}

fn build_events(
    paths: HashSet<String>,
    time: OffsetDateTime,
    wrap: impl Fn(DirEventData) -> StorageEvent,
) -> impl Iterator<Item = StorageEvent> {
    paths
        .into_iter()
        .map(move |resource_address| wrap(DirEventData { resource_address, time }))
}

#[derive(Debug)]
#[allow(dead_code, reason = "fields are read via the Debug impl when logging flush outcomes")]
struct FlushOutcome {
    attempted: usize,
    succeeded: usize,
    failed: usize,
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use storage_service_protos::nvidia::omniverse::notifications::publisher::v1beta::{
        PublishEventResponse, PublishingResult,
    };
    use storage_service_test::mocks::event_aggregation::RecordingService as MockService;
    use tonic::{async_trait, Response};

    use super::*;
    use crate::pubsub::{
        notification::client::{ClientConfig, NotificationServiceClient},
        types::{AnyConsumer, ConsumeRequest, ConsumeResults},
    };

    #[derive(Clone, Default)]
    struct RecordingBatchConsumer {
        batches: Arc<Mutex<Vec<ConsumeRequest>>>,
    }

    #[async_trait]
    impl Consumer for RecordingBatchConsumer {
        async fn consume(&self, events: ConsumeRequest) -> ConsumeResults {
            let event_count = events.len();
            self.batches.lock().unwrap().push(events);
            (0..event_count).map(|_| Ok(())).collect()
        }
    }

    async fn new_client(mock: &MockService) -> NotificationServiceClient {
        NotificationServiceClient::try_from_config(
            ClientConfig {
                enabled: true,
                endpoint: Some(mock.endpoint_url()),
                ..Default::default()
            },
            None,
        )
        .await
        .expect("client connect failed")
        .expect("client should be enabled")
    }

    fn success_handler(mock: &MockService) {
        mock.set_publish_handler(|request| {
            let event = request.into_inner().event;
            Ok(Response::new(PublishEventResponse {
                result: Some(PublishingResult {
                    event,
                    success: true,
                    failure_reason: String::new(),
                }),
            }))
        });
    }

    async fn flush_once(accumulator: &Accumulator, metrics: &Arc<Metrics>, consumer: AnyConsumer) {
        let flusher = Flusher::new(consumer, metrics.clone());
        flusher.flush_once(accumulator).await;
    }

    #[tokio::test]
    async fn test_created_and_deleted_events_are_consumed_in_one_batch() {
        let accumulator = Accumulator::new();
        accumulator.insert_created([
            "https://acct.blob.core.windows.net/c/a/".to_string(),
            "https://acct.blob.core.windows.net/c/b/".to_string(),
        ]);
        accumulator.insert_deleted(["https://acct.blob.core.windows.net/c/c/".to_string()]);
        let consumer = RecordingBatchConsumer::default();
        let batches = consumer.batches.clone();

        flush_once(
            &accumulator,
            &Arc::new(Metrics::new()),
            AnyConsumer::from_consumer(consumer),
        )
        .await;

        let batches = batches.lock().unwrap();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].len(), 3);
        assert!(batches[0][..2]
            .iter()
            .all(|event| matches!(event, StorageEvent::DirCreated(_))));
        assert!(matches!(batches[0][2], StorageEvent::DirDeleted(_)));
    }

    #[tokio::test]
    async fn flush_publishes_each_event_via_batch_publish_events() {
        let mock = MockService::new();
        let client = new_client(&mock).await;
        let accumulator = Accumulator::new();
        let metrics = Arc::new(Metrics::new());

        accumulator.insert_created([
            "https://acct.blob.core.windows.net/c/a/".to_string(),
            "https://acct.blob.core.windows.net/c/a/b/".to_string(),
        ]);
        accumulator.insert_deleted(["https://acct.blob.core.windows.net/c/x/".to_string()]);

        success_handler(&mock);

        flush_once(&accumulator, &metrics, AnyConsumer::from_consumer(client)).await;

        // Three events are exposed as flattened publish entries by the recording mock.
        assert_eq!(mock.received_publishes().len(), 3);

        let mut seen_created = Vec::new();
        let mut seen_deleted = Vec::new();
        for received in mock.received_publishes() {
            let event = received.event.expect("publish missing event");
            let resource_id = event.resource.as_ref().unwrap().resource_id.clone();
            match event.event_type.as_str() {
                "omni.storage.dir_created" => seen_created.push(resource_id),
                "omni.storage.dir_deleted" => seen_deleted.push(resource_id),
                other => panic!("unexpected event_type {other}"),
            }
        }
        seen_created.sort();
        seen_deleted.sort();
        assert_eq!(
            seen_created,
            vec![
                "https://acct.blob.core.windows.net/c/a/",
                "https://acct.blob.core.windows.net/c/a/b/",
            ]
        );
        assert_eq!(seen_deleted, vec!["https://acct.blob.core.windows.net/c/x/"]);
    }

    #[tokio::test]
    async fn flush_drains_accumulator() {
        let mock = MockService::new();
        let client = new_client(&mock).await;
        let accumulator = Accumulator::new();
        let metrics = Arc::new(Metrics::new());

        accumulator.insert_created(["https://acct.blob.core.windows.net/c/a/".to_string()]);

        success_handler(&mock);
        flush_once(&accumulator, &metrics, AnyConsumer::from_consumer(client)).await;

        assert_eq!(accumulator.created_len(), 0);
        assert_eq!(accumulator.deleted_len(), 0);
    }

    #[tokio::test]
    async fn empty_accumulator_is_noop() {
        let mock = MockService::new();
        let client = new_client(&mock).await;
        let accumulator = Accumulator::new();
        let metrics = Arc::new(Metrics::new());

        flush_once(&accumulator, &metrics, AnyConsumer::from_consumer(client)).await;

        assert!(mock.received_publishes().is_empty());
    }
}
