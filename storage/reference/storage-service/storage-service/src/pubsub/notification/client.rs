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

use futures::FutureExt;
use serde::{Deserialize, Serialize};
use snafu::IntoError;
use storage_service_metrics::{future::FutureExt as _, utils::record_metric};
use storage_service_protos::{
    google::protobuf::{value::Kind, Struct, Timestamp, Value},
    nvidia::omniverse::notifications::publisher::v1beta::{
        event_publishing_service_client::EventPublishingServiceClient, BatchPublishEventsRequest, Event, EventResource,
        PublishingResult,
    },
};
use time::OffsetDateTime;
use tonic::{async_trait, metadata::AsciiMetadataValue, transport::Channel, Request};
use tracing::{debug, info_span, instrument, Instrument};

use crate::{
    common::credentials::{access_token_file, oauth2, ArcCredentialsProvider, Credentials},
    pubsub::{
        notification::{
            error::{
                ClientCredentialsFetchFailedSnafu, CredentialsProviderInitSnafu, InitError, MetadataSetFailedSnafu,
                PublishError,
            },
            metrics::Metrics,
        },
        types::{
            ConsumeRequest, ConsumeResults, Consumer, DirEventData, ObjectCreatedEventData, ObjectDeletedEventData,
            StorageEvent,
        },
    },
};

const DEFAULT_DNS_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const DEFAULT_TCP_KEEPALIVE: Duration = Duration::from_secs(30);
const DEFAULT_HTTP2_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);
const DEFAULT_CONNECTIONS_PER_HOST: NonZero<usize> = NonZero::new(4).unwrap();
const DEFAULT_BATCH_SIZE: NonZero<usize> = NonZero::new(100).unwrap();

const METADATA_KEY_AUTHORIZATION: &str = "authorization";

#[derive(Deserialize, Debug, Default, Clone)]
#[serde(default)]
pub struct ClientCredentialsConfig {
    pub oauth2_client_credentials: Option<oauth2::Config>,
    pub access_token_file: Option<access_token_file::Config>,
}

/// Notification service client configuration.
#[derive(Deserialize, Debug, Default, Clone)]
#[serde(default)]
pub struct ClientConfig {
    pub enabled: bool,
    pub endpoint: Option<String>,
    pub event_types: EventTypesConfig,
    pub timeouts: TimeoutsConfig,
    pub batch_size: Option<NonZero<usize>>,
    #[serde(default)]
    pub connections_per_host: Option<NonZero<usize>>,
    #[serde(default, with = "humantime_serde::option")]
    pub dns_refresh_interval: Option<Duration>,
    #[serde(default, with = "humantime_serde::option")]
    pub tcp_keepalive: Option<Duration>,
    #[serde(default, with = "humantime_serde::option")]
    pub http2_keep_alive_interval: Option<Duration>,
}

/// Event type configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EventTypesConfig {
    pub object_created: String,
    pub object_deleted: String,
    pub dir_created: String,
    pub dir_deleted: String,
}

impl Default for EventTypesConfig {
    fn default() -> Self {
        Self {
            object_created: "omni.storage.created".into(),
            object_deleted: "omni.storage.deleted".into(),
            dir_created: "omni.storage.dir_created".into(),
            dir_deleted: "omni.storage.dir_deleted".into(),
        }
    }
}

/// Client timeouts configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TimeoutsConfig {
    #[serde(with = "humantime_serde")]
    pub request: Duration,
    #[serde(with = "humantime_serde")]
    pub connect: Duration,
}

impl Default for TimeoutsConfig {
    fn default() -> Self {
        Self {
            request: Duration::from_secs(60),
            connect: Duration::from_secs(60),
        }
    }
}

/// A notification service client.
#[derive(Clone)]
pub struct NotificationServiceClient {
    client: EventPublishingServiceClient<Channel>,
    credentials: Option<ArcCredentialsProvider>,
    event_types: EventTypesConfig,
    batch_size: NonZero<usize>,
    metrics: Arc<Metrics>,
}

impl NotificationServiceClient {
    /// Create a notification service client from the configuration.
    pub async fn try_from_config(
        config: ClientConfig,
        credentials_config: Option<ClientCredentialsConfig>,
    ) -> Result<Option<Self>, InitError> {
        if !config.enabled {
            return Ok(None);
        };

        let credentials_config = credentials_config.unwrap_or_default();
        let credentials = if let Some(config) = credentials_config.oauth2_client_credentials {
            debug!("using oauth2 client credentials provider");

            let credentials = oauth2::CredentialsProvider::try_from_config("notification_client", config)
                .await
                .map_err(|source| CredentialsProviderInitSnafu.into_error(source))?;
            Some(Arc::new(credentials) as ArcCredentialsProvider)
        } else if let Some(config) = credentials_config.access_token_file {
            debug!("using access token file credentials provider");

            let credentials = access_token_file::AccessTokenFileCredentialsProvider::from_config(config);
            Some(Arc::new(credentials) as ArcCredentialsProvider)
        } else {
            None
        };

        Self::try_new(config, credentials).await.map(Some)
    }

    /// Create a notification service client.
    async fn try_new(config: ClientConfig, credentials: Option<ArcCredentialsProvider>) -> Result<Self, InitError> {
        use storage_service_grpc::balancer::Builder;
        use tonic::transport::Endpoint;

        let http2_keepalive_interval = config
            .http2_keep_alive_interval
            .unwrap_or(DEFAULT_HTTP2_KEEPALIVE_INTERVAL);
        let tcp_keepalive = config.tcp_keepalive.unwrap_or(DEFAULT_TCP_KEEPALIVE);
        let request_timeout = config.timeouts.request;
        let connect_timeout = config.timeouts.connect;

        let channel = Builder::new(
            config
                .endpoint
                .ok_or_else(InitError::endpoint_missing)?
                .as_str()
                .parse()
                .map_err(InitError::invalid_endpoint_uri)?,
        )
        .map_err(InitError::channel_create)?
        .dns_refresh_interval(config.dns_refresh_interval.unwrap_or(DEFAULT_DNS_REFRESH_INTERVAL))
        .connections_per_host(config.connections_per_host.unwrap_or(DEFAULT_CONNECTIONS_PER_HOST))
        .connect(move |endpoint: Endpoint| -> Result<Endpoint, InitError> {
            let endpoint = endpoint
                .http2_keep_alive_interval(http2_keepalive_interval)
                .tcp_keepalive(Some(tcp_keepalive))
                .connect_timeout(connect_timeout)
                .timeout(request_timeout);
            Ok(endpoint)
        })
        .await
        .map_err(InitError::channel_create)?;

        Ok(Self {
            client: EventPublishingServiceClient::new(channel),
            metrics: Arc::new(Metrics::new()),
            event_types: config.event_types,
            batch_size: config.batch_size.unwrap_or(DEFAULT_BATCH_SIZE),
            credentials,
        })
    }

    /// Attach the configured bearer token to the request, if any.
    async fn attach_bearer_credentials<T>(&self, request: &mut Request<T>) -> Result<(), PublishError> {
        let Some(provider) = self.credentials.as_ref() else {
            return Ok(());
        };

        let credentials = provider
            .get_credentials()
            .await
            .map_err(|source| ClientCredentialsFetchFailedSnafu.into_error(source))?;

        let Credentials::Bearer(token) = credentials;
        request.metadata_mut().append(
            METADATA_KEY_AUTHORIZATION,
            AsciiMetadataValue::try_from(format!("Bearer {token}"))
                .map_err(|source| MetadataSetFailedSnafu.into_error(source))?,
        );

        Ok(())
    }

    fn build_event(&self, event: &StorageEvent) -> Event {
        match event {
            StorageEvent::ObjectCreated(data) => {
                create_object_created_event(self.event_types.object_created.clone(), data)
            },
            StorageEvent::ObjectDeleted(data) => {
                create_object_deleted_event(self.event_types.object_deleted.clone(), data)
            },
            StorageEvent::DirCreated(data) => create_dir_event(self.event_types.dir_created.clone(), data),
            StorageEvent::DirDeleted(data) => create_dir_event(self.event_types.dir_deleted.clone(), data),
        }
    }

    /// Publish a logical storage event batch, slicing only at the outbound RPC boundary.
    pub async fn publish_storage_events(&self, events: ConsumeRequest) -> ConsumeResults {
        if events.is_empty() {
            return Vec::new();
        }

        let mut results = ConsumeResults::with_capacity(events.len());

        for request_events in events.chunks(self.batch_size.get()) {
            let events: Vec<_> = request_events.iter().map(|event| self.build_event(event)).collect();
            let request_len = events.len();
            match self.publish_events(events).await {
                Ok(publishing_results) => {
                    debug_assert_eq!(request_len, publishing_results.len());
                    for result in publishing_results {
                        if result.success {
                            results.push(Ok(()));
                        } else {
                            results.push(Err(Arc::new(PublishError::rejected(result.failure_reason))));
                        }
                    }
                },
                Err(error) => {
                    let error = Arc::new(error);
                    for _ in 0..request_len {
                        results.push(Err(error.clone()));
                    }
                },
            }
        }

        results
    }

    #[instrument(skip_all, fields(batch_size = events.len()))]
    async fn publish_events(&self, events: Vec<Event>) -> Result<Vec<PublishingResult>, PublishError> {
        let expected_results = events.len();
        let mut request = Request::new(BatchPublishEventsRequest { events });
        self.attach_bearer_credentials(&mut request).await?;

        let results = self.publish(request).await?;

        if results.len() != expected_results {
            use super::error::InvalidResponseCardinalitySnafu;

            return InvalidResponseCardinalitySnafu {
                expected: expected_results,
                actual: results.len(),
            }
            .fail();
        }

        Ok(results)
    }

    /// Publish a notification batch.
    async fn publish(
        &self,
        request: Request<BatchPublishEventsRequest>,
    ) -> Result<Vec<PublishingResult>, PublishError> {
        let event_count = request.get_ref().events.len() as u64;
        self.metrics.batch_publish_events_batch_size.record(event_count);
        self.metrics.batch_publish_events_event_count.add(event_count);
        debug!(request = ?request.get_ref(), "sending a notification batch publish request");

        let response = self
            .client
            .clone()
            .batch_publish_events(request)
            .with_active_call_count_metric(&self.metrics.batch_publish_events_active_requests)
            .with_latency_metric(&self.metrics.batch_publish_events_latency)
            .inspect(record_metric(&self.metrics.batch_publish_events_count))
            .instrument(info_span!("notification_client::batch_publish"))
            .await
            .map_err(PublishError::unexpected_status)?
            .into_inner();

        debug!("notification batch publish request completed");

        Ok(response.results)
    }
}

#[async_trait]
impl Consumer for NotificationServiceClient {
    async fn consume(&self, events: ConsumeRequest) -> ConsumeResults {
        self.publish_storage_events(events).await
    }
}

fn create_object_created_event(event_type: String, event: &ObjectCreatedEventData) -> Event {
    create_event(
        event_type,
        event.resource_address.clone(),
        event.time,
        Struct {
            fields: [
                create_string_field("resource_address", event.resource_address.clone()),
                create_string_field("resource_identity", event.resource_identity.clone()),
                create_string_field("sequencer", event.sequencer.clone()),
            ]
            .into(),
        },
    )
}

fn create_object_deleted_event(event_type: String, event: &ObjectDeletedEventData) -> Event {
    create_event(
        event_type,
        event.resource_address.clone(),
        event.time,
        Struct {
            fields: [
                create_string_field("resource_address", event.resource_address.clone()),
                create_string_field("sequencer", event.sequencer.clone()),
            ]
            .into(),
        },
    )
}

fn create_dir_event(event_type: String, event: &DirEventData) -> Event {
    create_event(
        event_type,
        event.resource_address.clone(),
        event.time,
        Struct {
            fields: [create_string_field("resource_address", event.resource_address.clone())].into(),
        },
    )
}

fn create_string_field(key: impl Into<String>, value: impl Into<String>) -> (String, Value) {
    (
        key.into(),
        Value {
            kind: Some(Kind::StringValue(value.into())),
        },
    )
}

fn create_event(event_type: String, resource_id: String, occurred_at: OffsetDateTime, message: Struct) -> Event {
    let occurred_at_utc = occurred_at.to_utc();

    Event {
        event_type,
        message: Some(message),
        occurred_at: Some(Timestamp {
            seconds: occurred_at_utc.unix_timestamp(),
            nanos: occurred_at_utc.nanosecond() as i32,
        }),
        resource: Some(EventResource { resource_id }),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use storage_service_protos::{
        google::protobuf::{value::Kind, Struct, Timestamp, Value},
        nvidia::omniverse::notifications::publisher::v1beta::{Event, EventResource},
    };
    use storage_service_test::mocks::event_aggregation::Service;
    use time::{macros::datetime, OffsetDateTime};
    use tonic::async_trait;

    use crate::{
        common::credentials::{self, ArcCredentialsProvider, Credentials},
        pubsub::{
            notification::client::{ClientConfig, EventTypesConfig, NotificationServiceClient},
            types::{ConsumeResult, ObjectCreatedEventData, ObjectDeletedEventData, StorageEvent, StorageEventSource},
        },
    };

    struct BearerTokenCredentials(String);

    #[async_trait]
    impl credentials::CredentialsProvider for BearerTokenCredentials {
        async fn get_credentials(&self) -> credentials::Result<Credentials> {
            Ok(Credentials::Bearer(self.0.clone()))
        }
    }

    async fn publish_one(client: &NotificationServiceClient, event: StorageEvent) -> ConsumeResult {
        let mut results = client.publish_storage_events(vec![event]).await;
        assert_eq!(results.len(), 1);
        results.pop().unwrap()
    }

    #[tokio::test]
    async fn publishes_dir_created_via_storage_event() {
        use crate::pubsub::types::DirEventData;

        let event_time = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        let service_mock = Service::new();
        let client = NotificationServiceClient::try_new(
            ClientConfig {
                enabled: true,
                endpoint: Some(service_mock.endpoint_url()),
                ..Default::default()
            },
            Some(Arc::new(BearerTokenCredentials("some-token".into()))),
        )
        .await
        .expect("client connect failed");

        let expected_event = Event {
            event_type: "omni.storage.dir_created".into(),
            message: Some(Struct {
                fields: [(
                    "resource_address".into(),
                    Value {
                        kind: Some(Kind::StringValue("https://acct.blob.core.windows.net/c/a/b/".into())),
                    },
                )]
                .into(),
            }),
            occurred_at: Some(Timestamp {
                seconds: 1_700_000_000,
                nanos: 0,
            }),
            resource: Some(EventResource {
                resource_id: "https://acct.blob.core.windows.net/c/a/b/".into(),
            }),
        };
        service_mock
            .mock_publish_request()
            .when_bearer_authorization("some-token")
            .when_event(expected_event)
            .then_reply_with_ok();

        publish_one(
            &client,
            StorageEvent::DirCreated(DirEventData {
                resource_address: "https://acct.blob.core.windows.net/c/a/b/".into(),
                time: event_time,
            }),
        )
        .await
        .expect("publish_storage_event failed");
    }

    #[tokio::test]
    async fn test_client_publish_created_event() {
        let event_time = datetime!(2010-01-01 03:03:03 -2);
        let event_time_utc = event_time.to_utc();
        let event = Event {
            event_type: "omni.storage.created".into(),
            message: Some(Struct {
                fields: [
                    (
                        "resource_address".into(),
                        Value {
                            kind: Some(Kind::StringValue("some-address".into())),
                        },
                    ),
                    (
                        "resource_identity".into(),
                        Value {
                            kind: Some(Kind::StringValue("some-identity".into())),
                        },
                    ),
                    (
                        "sequencer".into(),
                        Value {
                            kind: Some(Kind::StringValue("some-sequencer".into())),
                        },
                    ),
                ]
                .into(),
            }),
            occurred_at: Some(Timestamp {
                nanos: event_time_utc.nanosecond() as i32,
                seconds: event_time_utc.unix_timestamp(),
            }),
            resource: Some(EventResource {
                resource_id: "some-address".into(),
            }),
        };
        let service_mock = Service::new();
        let client = create_client(
            service_mock.endpoint_url(),
            Some(Arc::new(BearerTokenCredentials("some-token".into()))),
        )
        .await;

        service_mock
            .mock_publish_request()
            .when_bearer_authorization("some-token")
            .when_event(event.clone())
            .then_reply_with_ok();

        publish_one(
            &client,
            StorageEvent::ObjectCreated(ObjectCreatedEventData {
                source: StorageEventSource::S3,
                time: event_time,
                sequencer: "some-sequencer".into(),
                resource_address: "some-address".into(),
                resource_identity: "some-identity".into(),
            }),
        )
        .await
        .expect("publish failed");
    }

    #[tokio::test]
    async fn test_client_publish_deleted_event() {
        let event_time = datetime!(2010-01-01 03:03:03 -2);
        let event_time_utc = event_time.to_utc();
        let event = Event {
            event_type: "omni.storage.deleted".into(),
            message: Some(Struct {
                fields: [
                    (
                        "resource_address".into(),
                        Value {
                            kind: Some(Kind::StringValue("some-address".into())),
                        },
                    ),
                    (
                        "sequencer".into(),
                        Value {
                            kind: Some(Kind::StringValue("some-sequencer".into())),
                        },
                    ),
                ]
                .into(),
            }),
            occurred_at: Some(Timestamp {
                nanos: event_time_utc.nanosecond() as i32,
                seconds: event_time_utc.unix_timestamp(),
            }),
            resource: Some(EventResource {
                resource_id: "some-address".into(),
            }),
        };
        let service_mock = Service::new();
        let client = create_client(
            service_mock.endpoint_url(),
            Some(Arc::new(BearerTokenCredentials("some-token".into()))),
        )
        .await;

        service_mock
            .mock_publish_request()
            .when_bearer_authorization("some-token")
            .when_event(event.clone())
            .then_reply_with_ok();

        publish_one(
            &client,
            StorageEvent::ObjectDeleted(ObjectDeletedEventData {
                source: StorageEventSource::S3,
                time: event_time,
                sequencer: "some-sequencer".into(),
                resource_address: "some-address".into(),
            }),
        )
        .await
        .expect("publish failed");
    }

    #[tokio::test]
    async fn test_client_errors_on_publish_failure() {
        let service_mock = Service::new();
        let client = create_client(
            service_mock.endpoint_url(),
            Some(Arc::new(BearerTokenCredentials("some-token".into()))),
        )
        .await;

        service_mock
            .mock_publish_request()
            .then_reply_with_publish_failure("error");

        publish_one(
            &client,
            StorageEvent::ObjectDeleted(ObjectDeletedEventData {
                source: StorageEventSource::S3,
                time: OffsetDateTime::now_utc(),
                sequencer: "some-sequencer".into(),
                resource_address: "some-address".into(),
            }),
        )
        .await
        .expect_err("publish is expected to fail");
    }

    #[tokio::test]
    async fn test_client_errors_on_publish_error() {
        let service_mock = Service::new();
        let client = create_client(
            service_mock.endpoint_url(),
            Some(Arc::new(BearerTokenCredentials("some-token".into()))),
        )
        .await;

        publish_one(
            &client,
            StorageEvent::ObjectDeleted(ObjectDeletedEventData {
                source: StorageEventSource::S3,
                time: OffsetDateTime::now_utc(),
                sequencer: "some-sequencer".into(),
                resource_address: "some-address".into(),
            }),
        )
        .await
        .expect_err("publish is expected to fail");
    }

    async fn create_client(uri: String, credentials: Option<ArcCredentialsProvider>) -> NotificationServiceClient {
        NotificationServiceClient::try_new(
            ClientConfig {
                enabled: true,
                endpoint: Some(uri),
                event_types: EventTypesConfig {
                    object_created: "omni.storage.created".into(),
                    object_deleted: "omni.storage.deleted".into(),
                    dir_created: "omni.storage.dir_created".into(),
                    dir_deleted: "omni.storage.dir_deleted".into(),
                },
                ..Default::default()
            },
            credentials,
        )
        .await
        .expect("failed to create a client")
    }
}
