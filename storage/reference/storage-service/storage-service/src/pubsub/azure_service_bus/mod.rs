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

use azservicebus::{
    authorization::AzureNamedKeyCredential, core::BasicRetryPolicy, ServiceBusClient, ServiceBusReceiveMode,
    ServiceBusReceivedMessage, ServiceBusReceiver, ServiceBusReceiverOptions,
};
use azure_core::auth::TokenCredential;
use futures::{future::pending, FutureExt};
use percent_encoding::utf8_percent_encode;
use serde::Deserialize;
use snafu::IntoError;
use storage_service_metrics::{future::FutureExt as _, utils::record_metric};
use time::OffsetDateTime;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, instrument, span, warn, Instrument, Level};

use crate::{
    common::{
        azure::SharedTokenCredential, backoff::retry_with_backoff, error::PrettyPrintErrorExt, secret::Secret,
        url::URI_COMPONENT_SET_PATH, vec::VecExt,
    },
    pubsub::{
        azure_service_bus::error::{EventParseFailedSnafu, SubscriberError},
        types::{Consumer, ObjectCreatedEventData, ObjectDeletedEventData, StorageEvent, StorageEventSource},
    },
};

pub mod error;
pub mod types;

mod metrics;

const DEFAULT_WORKERS: NonZero<usize> = NonZero::new(5).unwrap();
const DEFAULT_RECEIVE_BATCH_SIZE: NonZero<u32> = NonZero::new(10).unwrap();

#[derive(Debug)]
pub enum Credentials {
    SASPolicy {
        policy_name: String,
        policy_key: Secret<String>,
    },
    TokenCredential(Arc<dyn TokenCredential>),
}

/// Service bus subscriber configuration.
#[derive(Deserialize, Debug, Clone)]
pub struct AzureServiceBusSubscriberConfig {
    pub queue_namespace: String,
    pub queue_name: String,
    pub workers: Option<NonZero<usize>>,
    pub receive_batch_size: Option<NonZero<u32>>,
    #[serde(default, with = "humantime_serde")]
    pub receive_max_wait_time: Option<Duration>,
}

/// Azure Service Bus event subscriber.
#[derive(Debug)]
pub struct AzureServiceBusSubscriber(AzureServiceBusSubscriberInner);

impl AzureServiceBusSubscriber {
    /// Create a service bus subscriber from a config.
    pub async fn from_config(
        subscriber_id: &str,
        config: AzureServiceBusSubscriberConfig,
        credentials: Credentials,
        identity_resolver: types::ArcIdentityResolver,
    ) -> Self {
        let fully_qualified_namespace = format!("{}.servicebus.windows.net", config.queue_namespace);
        let client_factory = match credentials {
            Credentials::SASPolicy {
                policy_name,
                policy_key,
            } => {
                debug!(?policy_name, "using sas policy credential");

                ClientFactory::WithSASPolicyCredential {
                    fully_qualified_namespace,
                    policy_name,
                    policy_key,
                }
            },
            Credentials::TokenCredential(credential) => {
                debug!(?credential, "using token credential");

                ClientFactory::WithTokenCredential {
                    fully_qualified_namespace,
                    credential: SharedTokenCredential(credential),
                }
            },
        };

        Self(AzureServiceBusSubscriberInner {
            workers: config.workers.unwrap_or(DEFAULT_WORKERS),
            receiver_factory: Arc::new(ReceiverFactory {
                client_factory,
                queue_name: config.queue_name,
            }),
            receive_batch_size: config.receive_batch_size.unwrap_or(DEFAULT_RECEIVE_BATCH_SIZE),
            receive_max_wait_time: config.receive_max_wait_time,
            metrics: Arc::new(metrics::Metrics::new(subscriber_id)),
            identity_resolver,
        })
    }

    /// Run the subscriber with the specified consumer;
    pub async fn run<T: Consumer + 'static>(self, consumer: T) {
        let inner = self.0;
        let consumer = Arc::new(consumer);
        let cancel = CancellationToken::new();
        let drop_guard = cancel.clone().drop_guard();
        for worker_id in 0..inner.workers.get() {
            let mut context = SubscriberWorkerContext {
                receiver_factory: inner.receiver_factory.clone(),
                consumer: consumer.clone(),
                metrics: inner.metrics.clone(),
                receive_batch_size: inner.receive_batch_size,
                receive_max_wait_time: inner.receive_max_wait_time,
                identity_resolver: inner.identity_resolver.clone(),
            };

            tokio::spawn(cancel.clone().run_until_cancelled_owned(
                async move { context.process_messages().await }.instrument(span!(
                    Level::INFO,
                    "process_service_bus_messages",
                    ?worker_id
                )),
            ));
        }

        pending::<()>().await;

        drop(drop_guard);
    }
}

#[derive(Debug)]
enum ClientFactory {
    WithSASPolicyCredential {
        fully_qualified_namespace: String,
        policy_name: String,
        policy_key: Secret<String>,
    },
    WithTokenCredential {
        fully_qualified_namespace: String,
        credential: SharedTokenCredential,
    },
}

impl ClientFactory {
    /// Create a client connection.
    async fn create_client(&self) -> Result<ServiceBusClient<BasicRetryPolicy>, SubscriberError> {
        match self {
            Self::WithSASPolicyCredential {
                fully_qualified_namespace,
                policy_name,
                policy_key,
            } => ServiceBusClient::new_from_named_key_credential(
                fully_qualified_namespace,
                AzureNamedKeyCredential::new(policy_name, policy_key.inner()),
                Default::default(),
            )
            .await
            .map_err(SubscriberError::connect_failed),
            Self::WithTokenCredential {
                fully_qualified_namespace,
                credential: credentials,
            } => ServiceBusClient::new_from_token_credential(
                fully_qualified_namespace,
                credentials.clone(),
                Default::default(),
            )
            .await
            .map_err(SubscriberError::connect_failed),
        }
    }
}

/// Service bus receiver factory.
#[derive(Debug)]
struct ReceiverFactory {
    client_factory: ClientFactory,
    queue_name: String,
}

impl ReceiverFactory {
    /// Create a service bus receiver.
    async fn create_receiver(&self) -> Result<ServiceBusReceiver, SubscriberError> {
        debug!(queue_name = ?self.queue_name, "creating a service bus receiver");

        self.client_factory
            .create_client()
            .await?
            .create_receiver_for_queue(
                &self.queue_name,
                ServiceBusReceiverOptions {
                    receive_mode: ServiceBusReceiveMode::PeekLock,
                    ..Default::default()
                },
            )
            .await
            .map_err(SubscriberError::receiver_create_failed)
    }
}

#[derive(Debug)]
struct AzureServiceBusSubscriberInner {
    receiver_factory: Arc<ReceiverFactory>,
    metrics: Arc<metrics::Metrics>,
    workers: NonZero<usize>,
    receive_batch_size: NonZero<u32>,
    receive_max_wait_time: Option<Duration>,
    identity_resolver: types::ArcIdentityResolver,
}

struct SubscriberWorkerContext<T> {
    receiver_factory: Arc<ReceiverFactory>,
    receive_batch_size: NonZero<u32>,
    receive_max_wait_time: Option<Duration>,
    metrics: Arc<metrics::Metrics>,
    consumer: Arc<T>,
    identity_resolver: types::ArcIdentityResolver,
}

impl<T: Consumer> SubscriberWorkerContext<T> {
    /// Create a service bus client and process messages.
    async fn process_messages(&mut self) {
        loop {
            if let Err(error) = self.receive_and_process_messages().await {
                error!(error = %error.pretty(), "failed to process service bus messages");
            }
        }
    }

    /// Receive and process messages in a loop.
    async fn receive_and_process_messages(&mut self) -> Result<(), SubscriberError> {
        let mut receiver = retry_with_backoff(&Default::default(), || self.receiver_factory.create_receiver()).await?;

        loop {
            self.receive_and_process_single_batch(&mut receiver).await?;
        }
    }

    /// Receive and process a single batch.
    async fn receive_and_process_single_batch(&self, receiver: &mut ServiceBusReceiver) -> Result<(), SubscriberError> {
        let messages = self.receive_messages(receiver).await?;
        if !messages.is_empty() {
            debug!(count = messages.len(), "service bus messages received");

            let mut events = Vec::new();
            let mut event_message_indices = Vec::new();

            for (message_index, message) in messages.iter().enumerate() {
                let message_id = message.message_id();
                debug!(?message_id, "service bus message received");

                let message_body = match message.body() {
                    Ok(body) => body,
                    Err(source) => {
                        error!(error = %source.pretty(), ?message_id, "failed to read a message body");
                        continue;
                    },
                };

                match try_parse_event(message_body, &*self.identity_resolver).await {
                    Ok(Some(event)) => {
                        events.push(event);
                        event_message_indices.push(message_index);
                    },
                    Ok(None) => {},
                    Err(error) => {
                        error!(error = %error.pretty(), ?message_id, "failed to parse a message");
                    },
                }
            }

            if !events.is_empty() {
                let consume_results = self.consumer.consume(events).await;
                debug_assert_eq!(event_message_indices.len(), consume_results.len());

                event_message_indices.retain_indexed(|result_index, &message_index| {
                    match &consume_results[result_index] {
                        Ok(()) => true,
                        Err(error) => {
                            error!(
                                error = %error.pretty(),
                                message_id = ?messages[message_index].message_id(),
                                "failed to consume an event from a message"
                            );
                            false
                        },
                    }
                });
            }

            for message_index in event_message_indices {
                let message = &messages[message_index];
                if let Err(error) = self.complete_message(receiver, message).await {
                    error!(error = %error.pretty(), message_id = ?message.message_id(), "failed to complete a message");
                } else {
                    debug!(message_id = ?message.message_id(), "message has been processed");
                }
            }

            debug!("batch has been processed");
        }

        Ok(())
    }

    /// Receive a message.
    #[instrument(skip_all)]
    async fn receive_messages(
        &self,
        receiver: &mut ServiceBusReceiver,
    ) -> Result<Vec<ServiceBusReceivedMessage>, SubscriberError> {
        let messages = receiver
            .receive_messages_with_max_wait_time(self.receive_batch_size.get(), self.receive_max_wait_time)
            .with_active_call_count_metric(&self.metrics.receive_messages_active_requests)
            .with_latency_metric(&self.metrics.receive_messages_latency)
            .inspect(record_metric(&self.metrics.receive_messages_count))
            .await
            .map_err(SubscriberError::receive_messages_failed)?;

        self.metrics.receive_messages_batch_size.record(messages.len() as u64);

        Ok(messages)
    }

    /// Complete a message.
    #[instrument(skip_all)]
    async fn complete_message(
        &self,
        receiver: &mut ServiceBusReceiver,
        message: &ServiceBusReceivedMessage,
    ) -> Result<(), SubscriberError> {
        receiver
            .complete_message(message)
            .with_active_call_count_metric(&self.metrics.complete_message_active_requests)
            .with_latency_metric(&self.metrics.complete_message_latency)
            .inspect(record_metric(&self.metrics.complete_message_count))
            .await
            .map_err(SubscriberError::complete_message_failed)
    }
}

#[derive(Deserialize)]
struct CloudSchema10Event {
    #[serde(deserialize_with = "time::serde::iso8601::deserialize")]
    time: OffsetDateTime,
    #[serde(flatten)]
    data: CloudSchema10EventData,
}

#[derive(Deserialize)]
#[serde(tag = "type", content = "data")]
enum CloudSchema10EventData {
    #[serde(rename = "Microsoft.Storage.BlobCreated")]
    BlobCreated(CloudSchema10BlobEventData),
    #[serde(rename = "Microsoft.Storage.BlobDeleted")]
    BlobDeleted(CloudSchema10BlobEventData),
}

#[derive(Deserialize)]
struct CloudSchema10BlobEventData {
    url: String,
    #[serde(rename = "eTag")]
    etag: String,
    sequencer: String,
}

/// Parse an event from a message body.
async fn try_parse_event(
    body: &[u8],
    identity_resolver: &dyn types::IdentityResolver,
) -> Result<Option<StorageEvent>, SubscriberError> {
    let message: CloudSchema10Event =
        serde_json::from_slice(body).map_err(|source| EventParseFailedSnafu.into_error(source))?;

    match message.data {
        CloudSchema10EventData::BlobCreated(body) => {
            let resource_address = format_blob_address(&body.url)?;
            let maybe_resource_identity = identity_resolver
                .resolve_identity(&resource_address, &body.etag)
                .await
                .map_err(|source| error::MessageIdentityResolveSnafu.into_error(source))?;

            if maybe_resource_identity.is_none() {
                warn!(
                    ?resource_address,
                    etag = ?body.etag,
                    "resource is unreachable, BlobCreated event will be ignored",
                );
            }

            Ok(maybe_resource_identity.map(|resource_identity| {
                StorageEvent::ObjectCreated(ObjectCreatedEventData {
                    source: StorageEventSource::AzureBlob,
                    time: message.time,
                    sequencer: body.sequencer,
                    resource_address,
                    resource_identity,
                })
            }))
        },
        CloudSchema10EventData::BlobDeleted(body) => Ok(Some(StorageEvent::ObjectDeleted(ObjectDeletedEventData {
            source: StorageEventSource::AzureBlob,
            time: message.time,
            sequencer: body.sequencer,
            resource_address: format_blob_address(&body.url)?,
        }))),
    }
}

/// Format a blob address from an event into a proper URL string.
fn format_blob_address(url: &str) -> Result<String, SubscriberError> {
    let Some((scheme, suffix)) = url.split_once("://") else {
        return Err(SubscriberError::invalid_blob_address(
            "scheme separator is missing from the blob url",
        ));
    };

    let Some((authority, suffix)) = suffix.split_once("/") else {
        return Err(SubscriberError::invalid_blob_address(
            "container name separator is missing from the blob url",
        ));
    };

    let Some((account_name, _)) = authority.split_once(".") else {
        return Err(SubscriberError::invalid_blob_address(
            "account name is missing from the blob url",
        ));
    };

    if account_name.is_empty() {
        return Err(SubscriberError::invalid_blob_address("account name is empty"));
    }

    let Some((container, blob)) = suffix.split_once("/") else {
        return Err(SubscriberError::invalid_blob_address(
            "blob name separator is missing from the blob url",
        ));
    };

    if container.is_empty() {
        return Err(SubscriberError::invalid_blob_address("container name is empty"));
    }

    Ok(format!(
        "{}://{}/{}/{}",
        scheme,
        authority,
        container,
        utf8_percent_encode(blob, &URI_COMPONENT_SET_PATH)
    ))
}

#[cfg(test)]
mod tests {
    use time::{format_description::well_known::Iso8601, OffsetDateTime};

    use crate::pubsub::{
        azure_service_bus::{
            format_blob_address, try_parse_event,
            types::{BoxError, IdentityResolver},
        },
        types::{ObjectCreatedEventData, ObjectDeletedEventData, StorageEvent, StorageEventSource},
    };

    use rstest::rstest;

    #[derive(Debug)]
    struct MockIdentityResolver;

    #[async_trait::async_trait]
    impl IdentityResolver for MockIdentityResolver {
        async fn resolve_identity(&self, address: &str, etag: &str) -> Result<Option<String>, BoxError> {
            Ok(Some(format!("{address}?etag={etag}")))
        }
    }

    #[test]
    fn test_parse_blob_address() {
        let address = format_blob_address("https://someaccount.blob.core.windows.net/somecontainer/path to blob %%")
            .expect("blob address parse failed");

        assert_eq!(
            address,
            "https://someaccount.blob.core.windows.net/somecontainer/path%20to%20blob%20%25%25"
        );
    }

    #[rstest]
    #[case("path/to/file")]
    #[case("https://.blob.core.windows.net/container/blob")]
    #[case("https://myaccount.blob.core.windows.net")]
    #[case("https://myaccount.blob.core.windows.net/")]
    #[case("https://myaccount.blob.core.windows.net//")]
    #[case("https://myaccount.blob.core.windows.net/container")]
    fn test_parse_invalid_blob_address(#[case] address: &str) {
        format_blob_address(address).expect_err("blob parse is expected to fail");
    }

    #[tokio::test]
    async fn test_parse_object_created_event() {
        let event_data = r#"{
            "id":"98e5fb8f-601e-00a5-7aea-efb71706d71b",
            "source":"/subscriptions/mysubscription/resourceGroups/DefaultResourceGroup-EUS/providers/Microsoft.Storage/storageAccounts/mystorageacc",
            "specversion":"1.0",
            "type":"Microsoft.Storage.BlobCreated",
            "subject":"/blobServices/default/containers/stuff/blobs/stuff.zip",
            "time":"2025-07-08T09:23:47.1016888Z",
            "data":{
                "api":"PutBlob",
                "clientRequestId":"e8a62567-64de-4422-a216-f4efcca3a012",
                "requestId":"98e5fb8f-601e-00a5-7aea-efb717000000",
                "eTag":"0x8DDBE01246030B0",
                "contentType":"application/x-zip-compressed",
                "contentLength":22,
                "blobType":"BlockBlob",
                "accessTier":"Default",
                "url":"https://mystorageacc.blob.core.windows.net/stuff/stuff.zip",
                "sequencer":"0000000000000000000000000000A69A00000000004084c7",
                "storageDiagnostics":{"batchId":"f6de2ca3-6006-006e-00ea-efb442000000"}
            }
        }"#;
        let expected_time = OffsetDateTime::parse("2025-07-08T09:23:47.1016888Z", &Iso8601::DEFAULT).unwrap();

        let event = try_parse_event(event_data.as_bytes(), &MockIdentityResolver)
            .await
            .expect("event parse failed");

        assert_eq!(
            event,
            Some(StorageEvent::ObjectCreated(ObjectCreatedEventData {
                source: StorageEventSource::AzureBlob,
                resource_address: "https://mystorageacc.blob.core.windows.net/stuff/stuff.zip".into(),
                resource_identity: "https://mystorageacc.blob.core.windows.net/stuff/stuff.zip?etag=0x8DDBE01246030B0"
                    .into(),
                sequencer: "0000000000000000000000000000A69A00000000004084c7".into(),
                time: expected_time,
            }))
        );
    }

    #[tokio::test]
    async fn test_parse_object_deleted_event() {
        let event_data = r#"{
            "id":"98e6407e-601e-00a5-6cea-efb717062272",
            "source":"/subscriptions/mysubscription/resourceGroups/DefaultResourceGroup-EUS/providers/Microsoft.Storage/storageAccounts/mystorageacc",
            "specversion":"1.0",
            "type":"Microsoft.Storage.BlobDeleted",
            "subject":"/blobServices/default/containers/stuff/blobs/stuff.zip",
            "time":"2025-07-08T09:24:15.5527707Z",
            "data":{
                "api":"DeleteBlob",
                "requestId":"98e6407e-601e-00a5-6cea-efb717000000",
                "eTag":"0x8DDBE01246030B0",
                "contentType":"application/x-zip-compressed",
                "contentLength":0,
                "blobType":"BlockBlob",
                "url":"https://mystorageacc.blob.core.windows.net/stuff/stuff.zip",
                "sequencer":"0000000000000000000000000000A69A0000000000408fe3",
                "storageDiagnostics":{"batchId":"f6de6682-6006-006e-00ea-efb442000000"}
            }
        }"#;
        let expected_time = OffsetDateTime::parse("2025-07-08T09:24:15.5527707Z", &Iso8601::DEFAULT).unwrap();

        let event = try_parse_event(event_data.as_bytes(), &MockIdentityResolver)
            .await
            .expect("event parse failed");

        assert_eq!(
            event,
            Some(StorageEvent::ObjectDeleted(ObjectDeletedEventData {
                source: StorageEventSource::AzureBlob,
                resource_address: "https://mystorageacc.blob.core.windows.net/stuff/stuff.zip".into(),
                sequencer: "0000000000000000000000000000A69A0000000000408fe3".into(),
                time: expected_time,
            }))
        );
    }
}
