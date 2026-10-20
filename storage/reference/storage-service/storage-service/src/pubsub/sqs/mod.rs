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

use std::{num::NonZero, sync::Arc, time::Duration};

use aws_config::BehaviorVersion;
use aws_sdk_s3::config::ProvideCredentials;
use aws_sdk_sqs::{
    operation::receive_message::ReceiveMessageOutput,
    types::{BatchResultErrorEntry, DeleteMessageBatchRequestEntry},
    Client,
};
use bit_vec::BitVec;
use futures::{future::pending, TryFutureExt};
use percent_encoding::{percent_decode, percent_encode, utf8_percent_encode};
use serde::Deserialize;
use storage_service_metrics::{
    future::FutureExt as _,
    utils::{record_metric, record_metric_value},
};
use thiserror::Error;
use time::OffsetDateTime;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, instrument, span, Instrument, Level};

mod metrics;

use crate::{
    common::url::{URI_COMPONENT_SET, URI_COMPONENT_SET_PATH},
    pubsub::types::ObjectCreatedEventData,
};
use crate::{
    common::{error::PrettyPrintErrorExt, iterator::TryCollectExt},
    error::BoxError,
    pubsub::types::{Consumer, ObjectDeletedEventData, StorageEvent, StorageEventSource},
};

const DEFAULT_MAX_NUMBER_OF_MESSAGES: NonZero<u32> = NonZero::new(10).unwrap();
const DEFAULT_WORKERS: NonZero<usize> = NonZero::new(10).unwrap();
const DEFAULT_VISIBILITY_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_WAIT_TIME: Duration = Duration::from_secs(20);

/// SQS event subscriber configuration.
#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    queue_url: String,
    region: String,
    endpoint_url: Option<String>,
    max_number_of_messages: Option<NonZero<u32>>,
    #[serde(default, with = "humantime_serde")]
    visibility_timeout: Option<Duration>,
    #[serde(default, with = "humantime_serde")]
    wait_time: Option<Duration>,
    workers: Option<NonZero<usize>>,
}

/// SQS event subscriber.
#[derive(Debug)]
pub struct SqsSubscriber(SqsSubscriberInner);

impl SqsSubscriber {
    /// Create an SQS subscriber from configuration.
    pub async fn from_config(
        subscriber_id: &str,
        config: Config,
        credentials_provider: Arc<dyn ProvideCredentials>,
    ) -> Self {
        let sdk_config = aws_config::defaults(BehaviorVersion::latest());
        let sdk_config = if let Some(endpoint_url) = config.endpoint_url {
            sdk_config.endpoint_url(endpoint_url)
        } else {
            sdk_config
        };

        let sdk_config = sdk_config
            .region(aws_config::Region::new(config.region))
            .credentials_provider(credentials_provider)
            .load()
            .await;

        Self(SqsSubscriberInner {
            client: Client::new(&sdk_config),
            queue_url: config.queue_url,
            max_number_of_messages: config
                .max_number_of_messages
                .unwrap_or(DEFAULT_MAX_NUMBER_OF_MESSAGES)
                .get() as i32,
            visibility_timeout_seconds: config
                .visibility_timeout
                .unwrap_or(DEFAULT_VISIBILITY_TIMEOUT)
                .as_secs() as i32,
            wait_time_seconds: config.wait_time.unwrap_or(DEFAULT_WAIT_TIME).as_secs() as i32,
            workers: config.workers.unwrap_or(DEFAULT_WORKERS),
            metrics: metrics::Metrics::new(subscriber_id),
        })
    }

    /// Subscribe for the storage events and forward them to the configured consumers.
    pub async fn run<T: Consumer + 'static>(self, consumer: T) {
        let inner = self.0;
        let context = Arc::new(SubscriberWorkerContext {
            client: inner.client,
            queue_url: inner.queue_url,
            max_number_of_messages: inner.max_number_of_messages,
            visibility_timeout_seconds: inner.visibility_timeout_seconds,
            wait_time_seconds: inner.wait_time_seconds,
            metrics: inner.metrics,
            consumer,
        });

        let cancel = CancellationToken::new();
        let drop_guard = cancel.clone().drop_guard();
        for worker_id in 0..inner.workers.get() {
            tokio::spawn(
                cancel
                    .clone()
                    .run_until_cancelled_owned({
                        let context = context.clone();
                        async move {
                            context.process_messages().await;
                        }
                    })
                    .instrument(span!(Level::INFO, "process_sqs_messages", ?worker_id)),
            );
        }

        pending::<()>().await;

        drop(drop_guard)
    }
}

#[derive(Debug)]
struct SqsSubscriberInner {
    client: Client,
    queue_url: String,
    max_number_of_messages: i32,
    visibility_timeout_seconds: i32,
    wait_time_seconds: i32,
    workers: NonZero<usize>,
    metrics: metrics::Metrics,
}

struct SubscriberWorkerContext<T> {
    client: Client,
    consumer: T,
    queue_url: String,
    max_number_of_messages: i32,
    visibility_timeout_seconds: i32,
    wait_time_seconds: i32,
    metrics: metrics::Metrics,
}

impl<T: Consumer> SubscriberWorkerContext<T> {
    async fn process_messages(&self) {
        loop {
            self.process_messages_once().await;
        }
    }

    async fn process_messages_once(&self) {
        let response_or_error = self.receive_message().await;
        let response = match response_or_error {
            Ok(response) => response,
            Err(error) => {
                error!(error = %error.pretty(), "failed to receive a message");
                return;
            },
        };

        let messages = response.messages.unwrap_or_default();
        self.metrics.receive_message_batch_size.record(messages.len() as u64);

        if !messages.is_empty() {
            debug!(length = ?messages.len(), "message batch received");

            let mut message_succeeded = BitVec::from_elem(messages.len(), true);
            let mut events = Vec::new();
            let mut event_message_indices = Vec::new();

            for (idx, message) in messages.iter().enumerate() {
                let message_id = message.message_id.as_deref();
                debug!(?message_id, "processing a message");

                let Some(body) = message.body.as_ref() else {
                    continue;
                };

                let mut parsed_events = match parse_storage_events(body) {
                    Ok(events) => events,
                    Err(error) => {
                        message_succeeded.set(idx, false);
                        error!(error = %error.pretty(), ?message_id, "failed to parse a message");
                        continue;
                    },
                };

                debug!(count = parsed_events.len(), ?message_id, "message events extracted");
                self.metrics.events_per_message.record(parsed_events.len() as u64);

                event_message_indices.extend(std::iter::repeat_n(idx, parsed_events.len()));
                events.append(&mut parsed_events);
            }

            if !events.is_empty() {
                debug!(length = events.len(), "notifying the consumer with an event batch");

                let consume_results = self.consumer.consume(events).await;
                debug_assert_eq!(event_message_indices.len(), consume_results.len());

                for (message_index, result) in event_message_indices.into_iter().zip(consume_results) {
                    if let Err(error) = result {
                        error!(
                            error = %error.pretty(),
                            message_id = ?messages[message_index].message_id.as_deref(),
                            "failed to consume an event from a message"
                        );
                        message_succeeded.set(message_index, false);
                    }
                }
            }

            let ids_and_receipt_handles: Vec<_> = messages
                .into_iter()
                .enumerate()
                .filter_map(|(idx, message)| {
                    if message_succeeded[idx] {
                        Some((
                            message.message_id.unwrap_or_default(),
                            message.receipt_handle.unwrap_or_default(),
                        ))
                    } else {
                        None
                    }
                })
                .collect();

            if !ids_and_receipt_handles.is_empty() {
                if let Err(error) = self.delete_messages(ids_and_receipt_handles).await {
                    error!(error = %error.pretty(), "failed to delete messages");
                }
            }
        }
    }

    #[instrument(skip_all)]
    async fn receive_message(&self) -> Result<ReceiveMessageOutput, SubscriberError> {
        self.client
            .receive_message()
            .queue_url(&self.queue_url)
            .max_number_of_messages(self.max_number_of_messages)
            .visibility_timeout(self.visibility_timeout_seconds)
            .wait_time_seconds(self.wait_time_seconds)
            .send()
            .with_active_call_count_metric(&self.metrics.receive_message_active_requests)
            .with_latency_metric(&self.metrics.receive_message_latency)
            .inspect_ok(record_metric_value(&self.metrics.receive_message_count, 200u16))
            .inspect_err(record_metric(&self.metrics.receive_message_count))
            .await
            .map_err(SubscriberError::receive_failed)
    }

    #[instrument(skip_all)]
    async fn delete_messages(
        &self,
        ids_and_receipt_handles: impl IntoIterator<Item = (String, String)>,
    ) -> Result<(), SubscriberError> {
        let entries = ids_and_receipt_handles
            .into_iter()
            .map(|(id, receipt_handle)| {
                DeleteMessageBatchRequestEntry::builder()
                    .id(id)
                    .receipt_handle(receipt_handle)
                    .build()
            })
            .try_collect()
            .map_err(SubscriberError::delete_batch_build)?;

        debug!(length = entries.len(), "deleting messages in a batch");

        self.metrics
            .delete_message_batch_batch_size
            .record(entries.len() as u64);

        let response = self
            .client
            .delete_message_batch()
            .queue_url(&self.queue_url)
            .set_entries(Some(entries))
            .send()
            .with_active_call_count_metric(&self.metrics.delete_message_active_requests)
            .with_latency_metric(&self.metrics.delete_message_batch_latency)
            .inspect_ok(record_metric_value(&self.metrics.delete_message_batch_count, 200u16))
            .inspect_err(record_metric(&self.metrics.delete_message_batch_count))
            .await
            .map_err(SubscriberError::delete_failed)?;

        // Report a partial falure, if needed.
        if !response.failed.is_empty() {
            return Err(SubscriberError::PartialDeleteBatch(BatchError(response.failed)));
        }

        Ok(())
    }
}

/// Batch operation error.
#[derive(Debug)]
struct BatchError(Vec<BatchResultErrorEntry>);

impl std::fmt::Display for BatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "batch operation returned one or more failures: ")?;

        for (idx, error) in self.0.iter().enumerate() {
            if idx != 0 {
                write!(f, "; ")?;
            }

            write!(
                f,
                "id={:?}, code={:?}, message={:?}",
                error.id, error.code, error.message
            )?;
        }

        Ok(())
    }
}

impl std::error::Error for BatchError {}

#[derive(Debug, Error)]
#[allow(clippy::enum_variant_names)]
enum SubscriberError {
    #[error("failed to delete messages")]
    DeleteBatchFailed(#[source] BoxError),

    #[error("failed to receive messages")]
    ReceiveFailed(#[source] BoxError),

    #[error("failed to parse an event")]
    EventParseFailed(#[source] serde_json::Error),

    #[error("failed to build a delete batch request")]
    DeleteBatchBuildFailed(#[source] BoxError),

    #[error("failed to delete one or more messages in a batch")]
    PartialDeleteBatch(#[source] BatchError),
}

impl SubscriberError {
    fn delete_failed(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::DeleteBatchFailed(Box::new(error))
    }

    fn receive_failed(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::ReceiveFailed(Box::new(error))
    }

    fn delete_batch_build(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::DeleteBatchBuildFailed(Box::new(error))
    }
}

#[derive(Deserialize, Debug)]
struct S3ObjectMetadata {
    key: String,
    sequencer: String,
    #[serde(rename = "eTag")]
    etag: Option<String>,
    #[serde(default, rename = "versionId")]
    version_id: Option<String>,
}

#[derive(Deserialize, Debug)]
struct S3BucketMetadata {
    name: String,
}

#[derive(Deserialize, Debug)]
struct S3EventMetadata {
    object: S3ObjectMetadata,
    bucket: S3BucketMetadata,
}

#[derive(Deserialize, Debug)]
struct EventRecord {
    #[serde(rename = "eventSource")]
    event_source: String,
    #[serde(rename = "eventName")]
    event_name: String,
    #[serde(rename = "eventTime", deserialize_with = "time::serde::iso8601::deserialize")]
    event_time: OffsetDateTime,
    #[serde(rename = "awsRegion")]
    aws_region: String,
    s3: S3EventMetadata,
}

#[derive(Deserialize, Debug)]
struct EventRecords {
    #[serde(rename = "Records")]
    records: Vec<EventRecord>,
}

/// Replaces plus characters in an S3 object key with the %20 escape sequence and normalizes the rest of the path.
struct TranscodeObjectKey<'a>(&'a [u8]);

impl<'a> std::fmt::Display for TranscodeObjectKey<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut key = self.0;
        while !key.is_empty() {
            if let Some(pos) = key.iter().position(|b| *b == b'+') {
                if pos == 0 {
                    write!(f, "%20")?;
                } else {
                    write!(f, "{}%20", TranscodePath(&key[..pos]))?;
                }

                key = &key[pos + 1..];
            } else {
                write!(f, "{}", TranscodePath(key))?;
                break;
            }
        }

        Ok(())
    }
}

/// Normalizes capitalization and escaping for an S3 key path.
struct TranscodePath<'a>(&'a [u8]);

impl<'a> std::fmt::Display for TranscodePath<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for decoded in percent_decode(self.0) {
            let decoded_buffer = [decoded];
            write!(f, "{}", percent_encode(&decoded_buffer, &URI_COMPONENT_SET_PATH))?;
        }

        Ok(())
    }
}

fn parse_storage_events(data: &str) -> Result<Vec<StorageEvent>, SubscriberError> {
    let records: EventRecords = serde_json::from_str(data).map_err(SubscriberError::EventParseFailed)?;

    Ok(records.records.into_iter().filter_map(create_storage_event).collect())
}

fn create_storage_event(record: EventRecord) -> Option<StorageEvent> {
    if record.event_source != "aws:s3" {
        None
    } else if record.event_name.starts_with("ObjectCreated:") {
        let (resource_address, resource_identity) = get_object_resource_address_identity(
            &record.s3.bucket.name,
            &record.aws_region,
            &record.s3.object.key,
            record.s3.object.etag.as_deref().unwrap_or_default(),
            record.s3.object.version_id.as_deref(),
        );

        Some(StorageEvent::ObjectCreated(ObjectCreatedEventData {
            source: StorageEventSource::S3,
            time: record.event_time,
            sequencer: record.s3.object.sequencer,
            resource_address,
            resource_identity,
        }))
    } else if record.event_name.starts_with("ObjectRemoved:") {
        Some(StorageEvent::ObjectDeleted(ObjectDeletedEventData {
            source: StorageEventSource::S3,
            time: record.event_time,
            sequencer: record.s3.object.sequencer,
            resource_address: get_object_resource_address(
                &record.s3.bucket.name,
                &record.aws_region,
                &record.s3.object.key,
            ),
        }))
    } else {
        None
    }
}

fn get_object_resource_address(bucket: &str, region: &str, key: &str) -> String {
    format!(
        "https://{}.s3.{}.amazonaws.com/{}",
        bucket,
        region,
        TranscodeObjectKey(key.as_bytes()),
    )
}

fn get_object_resource_address_identity(
    bucket: &str,
    region: &str,
    key: &str,
    etag: &str,
    version_id: Option<&str>,
) -> (String, String) {
    let resource_address = get_object_resource_address(bucket, region, key);
    let resource_identity = if let Some(version_id) = version_id {
        format!(
            "{}?etag=%22{}%22&version-id={}",
            resource_address,
            utf8_percent_encode(etag, &URI_COMPONENT_SET),
            utf8_percent_encode(version_id, &URI_COMPONENT_SET)
        )
    } else {
        format!(
            "{}?etag=%22{}%22",
            resource_address,
            utf8_percent_encode(etag, &URI_COMPONENT_SET)
        )
    };

    (resource_address, resource_identity)
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use time::{format_description::well_known::Iso8601, OffsetDateTime};

    use crate::pubsub::{
        sqs::{get_object_resource_address, parse_storage_events},
        types::{ObjectCreatedEventData, ObjectDeletedEventData, StorageEvent, StorageEventSource},
    };

    #[test]
    fn test_parse_storage_events() {
        let event_data = r#"
        {
            "Records":[
                {
                    "eventVersion":"2.1",
                    "eventSource":"aws:s3",
                    "awsRegion":"eu-north-1",
                    "eventTime":"2025-07-16T06:51:17.171Z",
                    "eventName":"ObjectCreated:Put",
                    "userIdentity":{
                        "principalId":"AWS:OIDWBE8942N:myprincipal"
                    },
                    "requestParameters":{
                        "sourceIPAddress":"1.1.1.1"
                    },
                    "responseElements":{
                        "x-amz-request-id":"DFON3456NQ43Y7U",
                        "x-amz-id-2":"88ZlQDPmJOojAiTg01fOAR1iV5qoMXl+um1LRmNklVGWFvJwNyn5W5E4Q7+yJ7azsa5/HT4QEx8Z4/rSb0I4UkA4WO1A+/mC"
                    },
                    "s3":{
                        "s3SchemaVersion":"1.0",
                        "configurationId":"mytestbucketevents",
                        "bucket":{
                            "name":"mytest",
                            "ownerIdentity":{
                                "principalId":"EDFWWON3011"
                            },
                            "arn":"arn:aws:s3:::mytest"
                        },
                        "object":{
                            "key":"stuff.zip",
                            "size":22,
                            "eTag":"76cdb2bad9582d23c1f6f4d868218d6c",
                            "versionId":"bQ7VTv1JCgAFsmtyy91cncmTBumcsIFz",
                            "sequencer":"3E2GTH234HG39RG"
                        }
                    }
                },
                {
                    "eventVersion":"2.1",
                    "eventSource":"aws:s3",
                    "awsRegion":"eu-north-1",
                    "eventTime":"2025-07-16T06:51:17.171Z",
                    "eventName":"ObjectRemoved:Delete",
                    "userIdentity":{
                        "principalId":"AWS:OIDWBE8942N:myprincipal"
                    },
                    "requestParameters":{
                        "sourceIPAddress":"1.1.1.1"
                    },
                    "responseElements":{
                        "x-amz-request-id":"DFON3456NQ43Y7U",
                        "x-amz-id-2":"88ZlQDPmJOojAiTg01fOAR1iV5qoMXl+um1LRmNklVGWFvJwNyn5W5E4Q7+yJ7azsa5/HT4QEx8Z4/rSb0I4UkA4WO1A+/mC"
                    },
                    "s3":{
                        "s3SchemaVersion":"1.0",
                        "configurationId":"mytestbucketevents",
                        "bucket":{
                            "name":"mytest",
                            "ownerIdentity":{
                                "principalId":"EDFWWON3011"
                            },
                            "arn":"arn:aws:s3:::mytest"
                        },
                        "object":{
                            "key":"stuff.zip",
                            "size":22,
                            "eTag":"76cdb2bad9582d23c1f6f4d868218d6c",
                            "versionId":"bQ7VTv1JCgAFsmtyy91cncmTBumcsIFz",
                            "sequencer":"0068774BE5232290A2"
                        }
                    }
                }
            ]
        }
        "#;

        let events = parse_storage_events(event_data).expect("failed to parse storage events");

        let event_time = OffsetDateTime::parse("2025-07-16T06:51:17.171Z", &Iso8601::DEFAULT).unwrap();
        assert_eq!(
            events,
            vec![
                StorageEvent::ObjectCreated(ObjectCreatedEventData {
                    source: StorageEventSource::S3,
                    time: event_time,
                    sequencer: "3E2GTH234HG39RG".into(),
                    resource_address: "https://mytest.s3.eu-north-1.amazonaws.com/stuff.zip".into(),
                    resource_identity: "https://mytest.s3.eu-north-1.amazonaws.com/stuff.zip?etag=%2276cdb2bad9582d23c1f6f4d868218d6c%22&version-id=bQ7VTv1JCgAFsmtyy91cncmTBumcsIFz".into(),
                }),
                StorageEvent::ObjectDeleted(ObjectDeletedEventData {
                    source: StorageEventSource::S3,
                    time: event_time,
                    sequencer: "0068774BE5232290A2".into(),
                    resource_address: "https://mytest.s3.eu-north-1.amazonaws.com/stuff.zip".into(),
                }),
            ]
        );
    }

    #[test]
    fn test_parse_delete_event_with_missing_etag() {
        let event_data = r#"
        {
            "Records": [
                {
                    "eventVersion": "2.6",
                    "eventSource": "aws:s3",
                    "awsRegion": "eu-north-1",
                    "eventTime": "2026-10-06T07:55:48.969Z",
                    "eventName": "ObjectRemoved:Delete",
                    "userIdentity": {
                        "principalId": "AWS:OIDWBE8942N:myprincipal"
                    },
                    "requestParameters": {
                        "sourceIPAddress": "1.1.1.1"
                    },
                    "responseElements": {
                        "x-amz-request-id": "DFON3456NQ43Y7U",
                        "x-amz-id-2": "88ZlQDPmJOojAiTg01fOAR1iV5qoMXl+um1LRmNklVGWFvJwNyn5W5E4Q7+yJ7azsa5/HT4QEx8Z4/rSb0I4UkA4WO1A+/mC"
                    },
                    "s3": {
                        "s3SchemaVersion": "1.0",
                        "configurationId": "some-configuration-id",
                        "bucket": {
                            "name": "somebucket",
                            "ownerIdentity": {
                                "principalId": "EDFWWON3011"
                            },
                            "arn": "arn:aws:s3:::somebucket"
                        },
                        "object": {
                            "key": "512.txt",
                            "sequencer": "006AC4A984F0FDE30C"
                        }
                    }
                }
            ]
        }
        "#;

        let events = parse_storage_events(event_data).unwrap();

        let event_time = OffsetDateTime::parse("2026-10-06T07:55:48.969Z", &Iso8601::DEFAULT).unwrap();
        assert_eq!(
            events,
            [StorageEvent::ObjectDeleted(ObjectDeletedEventData {
                source: StorageEventSource::S3,
                resource_address: "https://somebucket.s3.eu-north-1.amazonaws.com/512.txt".into(),
                sequencer: "006AC4A984F0FDE30C".into(),
                time: event_time
            })]
        );
    }

    #[rstest]
    #[case("", "https://bucket.s3.us-east-1.amazonaws.com/")]
    #[case("stuff", "https://bucket.s3.us-east-1.amazonaws.com/stuff")]
    #[case("with%20space", "https://bucket.s3.us-east-1.amazonaws.com/with%20space")]
    #[case("with+space", "https://bucket.s3.us-east-1.amazonaws.com/with%20space")]
    #[case("+head", "https://bucket.s3.us-east-1.amazonaws.com/%20head")]
    #[case("tail+", "https://bucket.s3.us-east-1.amazonaws.com/tail%20")]
    #[case("+a+b+c%20d", "https://bucket.s3.us-east-1.amazonaws.com/%20a%20b%20c%20d")]
    #[case("%40d", "https://bucket.s3.us-east-1.amazonaws.com/%40d")]
    #[case("@d", "https://bucket.s3.us-east-1.amazonaws.com/%40d")]
    #[case("%3fd", "https://bucket.s3.us-east-1.amazonaws.com/%3Fd")]
    #[case("%25%25", "https://bucket.s3.us-east-1.amazonaws.com/%25%25")]
    fn test_get_object_resource_address(#[case] key: &str, #[case] expected_resource_address: &str) {
        let resource_address = get_object_resource_address("bucket", "us-east-1", key);
        assert_eq!(resource_address, expected_resource_address);
    }
}
