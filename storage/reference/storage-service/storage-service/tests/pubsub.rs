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

use std::time::Duration;

use aws_config::BehaviorVersion;
use aws_sdk_sqs::config::Credentials;
use aws_sdk_sqs::Client as SqsClient;
use azure_core::auth::Secret;
use azure_messaging_servicebus::{prelude::QueueClient, service_bus::SendMessageOptions};
use azure_storage_blobs::prelude::BlobServiceClient;
use more_asserts::assert_gt;
use percent_encoding::{utf8_percent_encode, AsciiSet};
use rstest::{fixture, rstest};
use storage_service_protos::nvidia::omniverse::notifications::publisher::v1beta::Event;
use time::macros::datetime;
use url_escape::NON_ALPHANUMERIC;

use crate::common::{
    env::get_env,
    random::{get_random_alphanumeric_string, get_random_path},
    service::{ClientCredentials, EmptyCredentials, Service},
    types::Interface,
};
use serial_test::serial;
use storage_service_test::{
    event::{
        create_azure_blob_created_event, create_azure_blob_deleted_event, create_azure_blob_event, create_azure_event,
        create_dir_event, create_object_created_event, create_object_deleted_event, create_s3_event,
        create_s3_event_record,
    },
    mocks::{
        credentials_provider::Service as CredentialsProviderService,
        event_aggregation::{EventAggregationMock, RecordingService, Service as EventAggregationService},
    },
};
use tokio::{sync::mpsc::unbounded_channel, time::timeout};

mod common;

const IDENTITY_PROVIDER_CLIENT_ID: &str = "some-client-id";
const IDNETITY_PROVIDER_CLIENT_SECRET: &str = "some-client-secret";
const IDENTITY_PROVIDER_SCOPE: &str = "some-scope";
const IDENTITY_PROVIDER_TOKEN: &str = "access-token";
const NOTIFICATION_TIMEOUT: Duration = Duration::from_secs(10);

const REMAPPER_ORIGINAL_BUCKET: &str = "mappedbucket";
const REMAPPER_ORIGINAL_REGION: &str = "us-west-2";
const REMAPPER_ORIGINAL_PATH: &str = "original/path/";
const REMAPPER_MAPPED_RESOURCE_URL_PREFIX: &str = "omniverse://myhost.mydomain/mapped/path/";

const URI_COMPONENT_SET: AsciiSet = NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'!')
    .remove(b'~')
    .remove(b'*')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')');

const URI_COMPONENT_SET_PATH: AsciiSet = URI_COMPONENT_SET.remove(b'/');

/// storage-core's canonical path spelling, mirroring `storage_core_util::url_path::encode_url_path`
/// (`NON_ALPHANUMERIC` minus `-` `_` `.` `~` `/`). The backend canonicalizes the path of a resource
/// identity when it parses one, so an expected identity has to use this set rather than
/// [`URI_COMPONENT_SET`], which leaves `!*'()` unescaped. Identities are URIs, so both spellings
/// denote the same object; matching the backend's keeps the comparison verbatim.
const STORAGE_CORE_PATH_SET: AsciiSet = NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~')
    .remove(b'/');

#[allow(unused)]
pub struct AzTestServices<M = EventAggregationService> {
    event_aggregation: M,
    credentials_provider: CredentialsProviderService,
    storage_service: Service,
    service_bus_client: QueueClient,
    storage_account: String,
    storage_container: String,
    blob_client: BlobServiceClient,
}

impl<M: EventAggregationMock> AzTestServices<M> {
    async fn new(enable_list_cache: bool, enable_stat_cache: bool) -> Self {
        Self::new_with_options(enable_list_cache, enable_stat_cache, None).await
    }

    /// Like `new`, but optionally enables `synthetic_dir_events` with the given flush interval.
    /// Tests should pass a short interval so they do not wait for the production default on every
    /// flush tick.
    async fn new_with_options(
        enable_list_cache: bool,
        enable_stat_cache: bool,
        synthetic_dir_flush: Option<&str>,
    ) -> Self {
        use azure_storage::StorageCredentials;

        let event_aggregation = M::start();
        let credentials_provider = CredentialsProviderService::new();
        credentials_provider
            .mock_token_request()
            .client_credentials(
                IDENTITY_PROVIDER_CLIENT_ID,
                IDNETITY_PROVIDER_CLIENT_SECRET,
                IDENTITY_PROVIDER_SCOPE,
            )
            .reply_with_bearer_token(IDENTITY_PROVIDER_TOKEN);

        let env = get_env();
        let az_env = env.unwrap_azure();
        let builder = Service::builder()
            .enable_az(true)
            .enable_postgres_metadata()
            .enable_az_url_remapper("mapped/prefix/", REMAPPER_ORIGINAL_PATH)
            .enable_service_bus_subscriber()
            .enable_notification_client(
                event_aggregation.endpoint(),
                ClientCredentials::new(
                    credentials_provider.config_endpoint(),
                    IDENTITY_PROVIDER_CLIENT_ID,
                    IDNETITY_PROVIDER_CLIENT_SECRET,
                    IDENTITY_PROVIDER_SCOPE,
                ),
            );

        let builder = if enable_list_cache {
            builder.enable_list_cache()
        } else {
            builder
        };

        let builder = if enable_stat_cache {
            builder.enable_stat_cache()
        } else {
            builder
        };

        let builder = if let Some(interval) = synthetic_dir_flush {
            builder.enable_synthetic_dir_events(interval)
        } else {
            builder
        };

        let service_bus_env = env.unwrap_azure_service_bus();
        let service_bus_client = QueueClient::new(
            azure_core::new_http_client(),
            &service_bus_env.queue_namespace,
            &service_bus_env.queue_name,
            &service_bus_env.policy_name,
            Secret::new(&service_bus_env.policy_key),
        )
        .expect("failed to create service bus client");

        let blob_client = BlobServiceClient::new(
            &az_env.storage_account,
            StorageCredentials::access_key(&az_env.storage_account, az_env.storage_key.clone()),
        );

        Self {
            event_aggregation,
            credentials_provider,
            service_bus_client,
            storage_service: builder.start(),
            storage_account: az_env.storage_account.clone(),
            storage_container: az_env.storage_container.clone(),
            blob_client,
        }
    }

    async fn create_random_blob(&self) -> (String, String, String) {
        use bytes::Bytes;

        let blob_name = get_random_path();
        let blob_properties = self
            .blob_client
            .container_client(&self.storage_container)
            .blob_client(&blob_name)
            .put_block_blob(Bytes::from_static(b"bytes"))
            .await
            .expect("failed to create a blob");

        let version_id = blob_properties.version_id.expect("version id is missing");
        let etag = strip_quotes(&blob_properties.etag).into();

        (blob_name, version_id, etag)
    }

    async fn create_object_created_event_pair(&self) -> (String, Event) {
        let sequencer = get_random_alphanumeric_string(32);
        let time = datetime!(2028-10-11 11:12:13 UTC);
        let (blob_name, version_id, etag) = self.create_random_blob().await;

        let az_event = create_azure_blob_created_event(
            &self.storage_account,
            &self.storage_container,
            &blob_name,
            &etag,
            &sequencer,
            time,
        );
        let event = create_object_created_event(
            format!(
                "https://{}.blob.core.windows.net/{}/{}",
                self.storage_account, self.storage_container, blob_name,
            ),
            // The address keeps the blob name verbatim: its escaping is part of the API contract.
            // Only the identity is canonicalized, by the backend that parses it.
            format!(
                "https://{}.blob.core.windows.net/{}/{}?etag=%22{}%22&version-id={}",
                self.storage_account,
                self.storage_container,
                utf8_percent_encode(&blob_name, &STORAGE_CORE_PATH_SET),
                utf8_percent_encode(&etag, &URI_COMPONENT_SET),
                utf8_percent_encode(&version_id, &URI_COMPONENT_SET),
            ),
            sequencer,
            time,
        );

        (az_event, event)
    }

    fn create_object_deleted_event_pair(&self) -> (String, Event) {
        self.create_object_deleted_event_pair_for_object(get_random_path())
    }

    fn create_object_deleted_event_pair_for_object(&self, blob_name: impl Into<String>) -> (String, Event) {
        let time = datetime!(2028-09-10 11:12:13 UTC);
        let sequencer = get_random_alphanumeric_string(32);
        let blob_name = blob_name.into();

        let az_event = create_azure_blob_deleted_event(
            &self.storage_account,
            &self.storage_container,
            &blob_name,
            &sequencer,
            time,
        );
        let event = create_object_deleted_event(
            format!(
                "https://{}.blob.core.windows.net/{}/{}",
                self.storage_account, self.storage_container, blob_name
            ),
            sequencer,
            time,
        );

        (az_event, event)
    }

    fn create_mapped_object_deleted_event_pair_for_object(
        &self,
        mapped_path: impl Into<String>,
    ) -> (String, Event, Event) {
        let time = datetime!(2028-09-10 11:12:13 UTC);
        let sequencer = get_random_alphanumeric_string(32);
        let mapped_path = mapped_path.into();

        let az_event = create_azure_blob_deleted_event(
            &self.storage_account,
            &self.storage_container,
            format!("{}{}", REMAPPER_ORIGINAL_PATH, mapped_path),
            &sequencer,
            time,
        );

        let encoded_mapped_path = utf8_percent_encode(&mapped_path, &URI_COMPONENT_SET_PATH).to_string();
        let event = create_object_deleted_event(
            self.storage_service
                .remapper_original_url_builder()
                .raw_path(&encoded_mapped_path)
                .to_string(),
            &sequencer,
            time,
        );
        let mapped_event = create_object_deleted_event(
            self.storage_service
                .remapper_mapped_url_builder()
                .raw_path(encoded_mapped_path)
                .to_string(),
            sequencer,
            time,
        );

        (az_event, event, mapped_event)
    }

    async fn send_message(&self, data: &str) {
        self.service_bus_client
            .send_message(
                data,
                Some(SendMessageOptions {
                    content_type: Some("application/json".into()),
                    ..Default::default()
                }),
            )
            .await
            .expect("failed to send a service bus message");
    }
}

#[allow(unused)]
pub struct AwsTestServices<M = EventAggregationService> {
    event_aggregation: M,
    credentials_provider: CredentialsProviderService,
    storage_service: Service,
    sqs_client: SqsClient,
    sqs_queue_url: String,
}

impl<M: EventAggregationMock> AwsTestServices<M> {
    async fn new(use_anonymous_notification_auth: bool) -> Self {
        Self::new_with_options(use_anonymous_notification_auth, None).await
    }

    /// Like `new`, but optionally enables `synthetic_dir_events` with the given flush interval.
    async fn new_with_options(use_anonymous_notification_auth: bool, synthetic_dir_flush: Option<&str>) -> Self {
        let event_aggregation = M::start();
        let credentials_provider = CredentialsProviderService::new();
        credentials_provider
            .mock_token_request()
            .client_credentials(
                IDENTITY_PROVIDER_CLIENT_ID,
                IDNETITY_PROVIDER_CLIENT_SECRET,
                IDENTITY_PROVIDER_SCOPE,
            )
            .reply_with_bearer_token(IDENTITY_PROVIDER_TOKEN);

        let builder = Service::builder()
            .enable_s3(false)
            .enable_postgres_metadata()
            .enable_url_remapper(
                REMAPPER_MAPPED_RESOURCE_URL_PREFIX,
                format!(
                    "https://{}.s3.{}.amazonaws.com/{}",
                    REMAPPER_ORIGINAL_BUCKET, REMAPPER_ORIGINAL_REGION, REMAPPER_ORIGINAL_PATH
                ),
            )
            .enable_sqs_subscriber();

        let builder = if use_anonymous_notification_auth {
            builder.enable_notification_client(event_aggregation.endpoint(), EmptyCredentials)
        } else {
            builder.enable_notification_client(
                event_aggregation.endpoint(),
                ClientCredentials::new(
                    credentials_provider.config_endpoint(),
                    IDENTITY_PROVIDER_CLIENT_ID,
                    IDNETITY_PROVIDER_CLIENT_SECRET,
                    IDENTITY_PROVIDER_SCOPE,
                ),
            )
        };

        let builder = if let Some(interval) = synthetic_dir_flush {
            builder.enable_synthetic_dir_events(interval)
        } else {
            builder
        };

        let storage_service = builder.start();

        let env = get_env().unwrap_elasticmq();
        let sdk_config = aws_config::defaults(BehaviorVersion::latest())
            .endpoint_url(env.endpoint_url())
            .region(aws_config::Region::new(&env.region_name))
            .credentials_provider(Credentials::new(
                &env.access_key_id,
                &env.secret_access_key,
                None,
                None,
                "storage-service-event-subscriber",
            ))
            .load()
            .await;

        Self {
            event_aggregation,
            credentials_provider,
            storage_service,
            sqs_client: SqsClient::new(&sdk_config),
            sqs_queue_url: env.queue_url.clone(),
        }
    }

    fn create_object_created_event_pair(&self) -> (String, Event) {
        let bucket = get_random_alphanumeric_string(8);
        let region = "us-west-1";
        let key = get_random_path();
        let sequencer = get_random_alphanumeric_string(32);
        let time = datetime!(2029-01-02 03:04:05 UTC);
        let version_id = get_random_alphanumeric_string(16);
        let etag = get_random_alphanumeric_string(16);

        let s3_event = create_s3_event([create_s3_event_record(
            "ObjectCreated:Put",
            &bucket,
            region,
            &key,
            &version_id,
            &etag,
            &sequencer,
            time,
        )]);
        let storage_event = create_object_created_event(
            format!(
                "https://{}.s3.{}.amazonaws.com/{}",
                bucket,
                region,
                utf8_percent_encode(&key, &URI_COMPONENT_SET_PATH)
            ),
            format!(
                "https://{}.s3.{}.amazonaws.com/{}?etag=%22{}%22&version-id={}",
                bucket,
                region,
                utf8_percent_encode(&key, &URI_COMPONENT_SET_PATH),
                utf8_percent_encode(&etag, &URI_COMPONENT_SET),
                utf8_percent_encode(&version_id, &URI_COMPONENT_SET),
            ),
            sequencer,
            time,
        );

        (s3_event, storage_event)
    }

    fn create_object_deleted_event_record_pair(&self) -> (serde_json::Value, Event) {
        let bucket = get_random_alphanumeric_string(8);
        let region = "us-west-1";
        let key = get_random_path();
        let sequencer = get_random_alphanumeric_string(32);
        let time = datetime!(2029-01-02 03:04:05 UTC);
        let version_id = get_random_alphanumeric_string(16);
        let etag = get_random_alphanumeric_string(16);

        let s3_record = create_s3_event_record(
            "ObjectRemoved:Delete",
            &bucket,
            region,
            &key,
            &version_id,
            &etag,
            &sequencer,
            time,
        );
        let storage_event = create_object_deleted_event(
            format!(
                "https://{}.s3.{}.amazonaws.com/{}",
                bucket,
                region,
                utf8_percent_encode(&key, &URI_COMPONENT_SET_PATH)
            ),
            sequencer,
            time,
        );

        (s3_record, storage_event)
    }

    fn create_mapper_object_created_event_pair(&self) -> (String, Event, Event) {
        let sequencer = get_random_alphanumeric_string(32);
        let version_id = get_random_alphanumeric_string(16);
        let etag = get_random_alphanumeric_string(16);
        let time = datetime!(2029-02-28 12:34:56 UTC);
        let key = "path to file";
        let s3_event = create_s3_event([create_s3_event_record(
            "ObjectCreated:Put",
            REMAPPER_ORIGINAL_BUCKET,
            REMAPPER_ORIGINAL_REGION,
            format!("{}{}", REMAPPER_ORIGINAL_PATH, key),
            &version_id,
            &etag,
            &sequencer,
            time,
        )]);

        let original_event = create_object_created_event(
            format!(
                "https://{}.s3.{}.amazonaws.com/{}{}",
                REMAPPER_ORIGINAL_BUCKET,
                REMAPPER_ORIGINAL_REGION,
                REMAPPER_ORIGINAL_PATH,
                utf8_percent_encode(key, &URI_COMPONENT_SET_PATH)
            ),
            format!(
                "https://{}.s3.{}.amazonaws.com/{}{}?etag=%22{}%22&version-id={}",
                REMAPPER_ORIGINAL_BUCKET,
                REMAPPER_ORIGINAL_REGION,
                REMAPPER_ORIGINAL_PATH,
                utf8_percent_encode(key, &URI_COMPONENT_SET_PATH),
                utf8_percent_encode(&etag, &URI_COMPONENT_SET),
                utf8_percent_encode(&version_id, &URI_COMPONENT_SET),
            ),
            &sequencer,
            time,
        );
        let mapped_event = create_object_created_event(
            format!(
                "{}{}",
                REMAPPER_MAPPED_RESOURCE_URL_PREFIX,
                utf8_percent_encode(key, &URI_COMPONENT_SET_PATH)
            ),
            format!(
                "{}{}?etag=%22{}%22&version-id={}",
                REMAPPER_MAPPED_RESOURCE_URL_PREFIX,
                utf8_percent_encode(key, &URI_COMPONENT_SET_PATH),
                utf8_percent_encode(&etag, &URI_COMPONENT_SET),
                utf8_percent_encode(&version_id, &URI_COMPONENT_SET),
            ),
            sequencer,
            time,
        );

        (s3_event, original_event, mapped_event)
    }

    fn create_mapper_object_deleted_event_pair(&self) -> (String, Event, Event) {
        let sequencer = get_random_alphanumeric_string(32);
        let version_id = get_random_alphanumeric_string(16);
        let etag = get_random_alphanumeric_string(16);
        let time = datetime!(2029-02-28 12:34:56 UTC);
        let key = "path to file";
        let s3_event = create_s3_event([create_s3_event_record(
            "ObjectRemoved:Delete",
            REMAPPER_ORIGINAL_BUCKET,
            REMAPPER_ORIGINAL_REGION,
            format!("{}{}", REMAPPER_ORIGINAL_PATH, key),
            &version_id,
            &etag,
            &sequencer,
            time,
        )]);

        let original_event = create_object_deleted_event(
            format!(
                "https://{}.s3.{}.amazonaws.com/{}{}",
                REMAPPER_ORIGINAL_BUCKET,
                REMAPPER_ORIGINAL_REGION,
                REMAPPER_ORIGINAL_PATH,
                utf8_percent_encode(key, &URI_COMPONENT_SET_PATH)
            ),
            &sequencer,
            time,
        );
        let mapped_event = create_object_deleted_event(
            format!(
                "{}{}",
                REMAPPER_MAPPED_RESOURCE_URL_PREFIX,
                utf8_percent_encode(key, &URI_COMPONENT_SET_PATH)
            ),
            sequencer,
            time,
        );

        (s3_event, original_event, mapped_event)
    }

    async fn send_message(&self, data: impl Into<String>) {
        self.sqs_client
            .send_message()
            .queue_url(&self.sqs_queue_url)
            .message_body(data)
            .send()
            .await
            .expect("failed to send an event");
    }
}

#[fixture]
async fn service_bus_test_services() -> AzTestServices {
    AzTestServices::new(false, true).await
}

#[fixture]
async fn sqs_test_services() -> AwsTestServices {
    AwsTestServices::new(false).await
}

// ---------- Synthetic directory events: fixtures & helpers ----------
//
// The tests these support exercise the `synthetic_dir_events` feature end-to-end: a real
// `storage-service` child process consumes file events from a real Azure Service Bus / SQS queue,
// the observer accumulates ancestor directory paths, and the flush loop publishes
// `omni.storage.dir_created` / `omni.storage.dir_deleted` events to the mock event-aggregation
// service. The flush interval is shortened so the tests don't sleep for the production default.

const SYNTHETIC_DIR_FLUSH_INTERVAL: &str = "100ms";

#[fixture]
async fn service_bus_synthetic_dir_test_services() -> AzTestServices<RecordingService> {
    AzTestServices::new_with_options(false, false, Some(SYNTHETIC_DIR_FLUSH_INTERVAL)).await
}

#[fixture]
async fn sqs_synthetic_dir_test_services() -> AwsTestServices<RecordingService> {
    AwsTestServices::new_with_options(false, Some(SYNTHETIC_DIR_FLUSH_INTERVAL)).await
}

/// Install a default "succeed on every publish" handler on the event-aggregation mock so the
/// notification client doesn't bail out of the consumer chain on the (uninteresting-for-this-test)
/// `object_created` / `object_deleted` events. Critical for the URL remapper case, where the
/// `?` short-circuit on the first inner-consume drops the original-path accumulation entirely.
fn install_success_publish_handler(mock: &RecordingService) {
    use storage_service_protos::nvidia::omniverse::notifications::publisher::v1beta::{
        PublishEventResponse, PublishingResult,
    };
    use tonic::Response;

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

/// Poll `received_publishes()` until every expected `(event_type, resource_id)` has been seen,
/// or the timeout elapses. Panics on timeout with what was actually observed.
async fn wait_for_dir_events(mock: &RecordingService, expected: &[(&'static str, String)], timeout_total: Duration) {
    let deadline = tokio::time::Instant::now() + timeout_total;
    loop {
        let received = mock.received_publishes();
        let missing: Vec<&(&'static str, String)> = expected
            .iter()
            .filter(|(event_type, resource_id)| {
                !received.iter().any(|p| {
                    let event = match p.event.as_ref() {
                        Some(event) => event,
                        None => return false,
                    };
                    event.event_type == *event_type
                        && event.resource.as_ref().is_some_and(|r| &r.resource_id == resource_id)
                })
            })
            .collect();
        if missing.is_empty() {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            let seen: Vec<(String, String)> = received
                .iter()
                .filter_map(|p| {
                    let event = p.event.as_ref()?;
                    let resource_id = event.resource.as_ref()?.resource_id.clone();
                    Some((event.event_type.clone(), resource_id))
                })
                .collect();
            panic!(
                "did not see all expected dir events in time. missing: {:?}, saw: {:?}",
                missing, seen,
            );
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn received_event(mock: &RecordingService, event_type: &str, resource_id: &str) -> bool {
    mock.received_publishes().iter().any(|publish| {
        publish.event.as_ref().is_some_and(|event| {
            event.event_type == event_type
                && event
                    .resource
                    .as_ref()
                    .is_some_and(|resource| resource.resource_id == resource_id)
        })
    })
}

#[rstest]
#[serial(service_bus)]
#[test_log::test(tokio::test)]
async fn test_azure_service_bus_processes_object_created_events(#[future] service_bus_test_services: AzTestServices) {
    let services = service_bus_test_services.await;

    // Configure an expectation for the object created event.
    let (azure_event, event) = services.create_object_created_event_pair().await;
    let (callback_tx, mut callback_rx) = unbounded_channel();
    services
        .event_aggregation
        .mock_publish_request()
        .when_bearer_authorization(IDENTITY_PROVIDER_TOKEN)
        .when_event(event)
        .then_call(move || {
            let _ = callback_tx.send(());
        })
        .then_reply_with_ok();

    // Publish an event. We must receive it back.
    services.send_message(&azure_event).await;

    timeout(NOTIFICATION_TIMEOUT, callback_rx.recv())
        .await
        .expect("failed to receive an event in time")
        .expect("receive half has unexpectedly closed");
}

#[rstest]
#[serial(service_bus)]
#[test_log::test(tokio::test)]
async fn test_azure_service_bus_processes_object_deleted_events(#[future] service_bus_test_services: AzTestServices) {
    let services = service_bus_test_services.await;

    // Configure an expectation for the object created event.
    let (azure_event, event) = services.create_object_deleted_event_pair();
    let (callback_tx, mut callback_rx) = unbounded_channel();
    services
        .event_aggregation
        .mock_publish_request()
        .when_bearer_authorization(IDENTITY_PROVIDER_TOKEN)
        .when_event(event)
        .then_call(move || {
            let _ = callback_tx.send(());
        })
        .then_reply_with_ok();

    // Publish the event. The notification service must receive a notification.
    services.send_message(&azure_event).await;

    timeout(NOTIFICATION_TIMEOUT, callback_rx.recv())
        .await
        .expect("failed to receive an event in time")
        .expect("receive half has unexpectedly closed");
}

#[rstest]
#[serial(service_bus)]
#[test_log::test(tokio::test)]
async fn test_azure_service_bus_ignores_unknown_events_mixed_with_expected_event(
    #[future] service_bus_test_services: AzTestServices,
) {
    let services = service_bus_test_services.await;

    // Configure an expectation for the object created event.
    let (azure_event, event) = services.create_object_created_event_pair().await;
    let (callback_tx, mut callback_rx) = unbounded_channel();
    services
        .event_aggregation
        .mock_publish_request()
        .when_bearer_authorization(IDENTITY_PROVIDER_TOKEN)
        .when_event(event)
        .then_call(move || {
            let _ = callback_tx.send(());
        })
        .then_reply_with_ok();

    let unknown_event = create_azure_event(create_azure_blob_event(
        "Microsoft.Storage.BlobRenamed",
        &services.storage_account,
        &services.storage_container,
        get_random_path(),
        get_random_alphanumeric_string(16),
        get_random_alphanumeric_string(32),
        datetime!(2028-10-11 11:12:13 UTC),
    ));

    // Publish the unknown and expected events into the same receive batch. Only the expected event
    // must be forwarded to the notification service.
    services.send_message(&unknown_event).await;
    services.send_message(&azure_event).await;

    timeout(NOTIFICATION_TIMEOUT, callback_rx.recv())
        .await
        .expect("failed to receive an event in time")
        .expect("receive half has unexpectedly closed");
}

#[rstest]
#[serial(sqs)]
#[test_log::test(tokio::test)]
async fn test_sqs_processes_object_created_events(#[future] sqs_test_services: AwsTestServices) {
    let services = sqs_test_services.await;

    // Configure an expectation for the object created event.
    let (s3_event, event) = services.create_object_created_event_pair();
    let (callback_tx, mut callback_rx) = unbounded_channel();
    services
        .event_aggregation
        .mock_publish_request()
        .when_bearer_authorization(IDENTITY_PROVIDER_TOKEN)
        .when_event(event)
        .then_call(move || {
            let _ = callback_tx.send(());
        })
        .then_reply_with_ok();

    // Publish an event. It must be consumed and forwarded to the notification service.
    services.send_message(s3_event).await;

    timeout(NOTIFICATION_TIMEOUT, callback_rx.recv())
        .await
        .expect("failed to receive an event in time")
        .expect("receive half has unexpectedly closed");
}

#[rstest]
#[serial(sqs)]
#[test_log::test(tokio::test)]
async fn test_sqs_processes_object_deleted_events(#[future] sqs_test_services: AwsTestServices) {
    let services = sqs_test_services.await;

    // Configure an expectation for the object deleted event.
    let (s3_record, event) = services.create_object_deleted_event_record_pair();
    let (callback_tx, mut callback_rx) = unbounded_channel();
    services
        .event_aggregation
        .mock_publish_request()
        .when_bearer_authorization(IDENTITY_PROVIDER_TOKEN)
        .when_event(event)
        .then_call(move || {
            let _ = callback_tx.send(());
        })
        .then_reply_with_ok();

    // Publish an event. It must be consumed and forwarded to the notification service.
    services.send_message(create_s3_event([s3_record])).await;

    timeout(NOTIFICATION_TIMEOUT, callback_rx.recv())
        .await
        .expect("failed to receive an event in time")
        .expect("receive half has unexpectedly closed");
}

#[rstest]
#[serial(sqs)]
#[test_log::test(tokio::test)]
async fn test_sqs_ignores_unknown_events_mixed_with_expected_event(#[future] sqs_test_services: AwsTestServices) {
    let services = sqs_test_services.await;

    // Configure an expectation for the object deleted event.
    let (s3_record, event) = services.create_object_deleted_event_record_pair();
    let (callback_tx, mut callback_rx) = unbounded_channel();
    services
        .event_aggregation
        .mock_publish_request()
        .when_bearer_authorization(IDENTITY_PROVIDER_TOKEN)
        .when_event(event)
        .then_call(move || {
            let _ = callback_tx.send(());
        })
        .then_reply_with_ok();

    let unknown_record = create_s3_event_record(
        "ObjectRestore:Completed",
        get_random_alphanumeric_string(8),
        "us-west-1",
        get_random_path(),
        get_random_alphanumeric_string(16),
        get_random_alphanumeric_string(16),
        get_random_alphanumeric_string(32),
        datetime!(2029-01-02 03:04:05 UTC),
    );

    // Publish a single notification containing unknown and expected records. Only the expected
    // record must be forwarded to the notification service.
    services
        .send_message(create_s3_event([unknown_record, s3_record]))
        .await;

    timeout(NOTIFICATION_TIMEOUT, callback_rx.recv())
        .await
        .expect("failed to receive an event in time")
        .expect("receive half has unexpectedly closed");
}

#[rstest]
#[serial(sqs)]
#[test_log::test(tokio::test)]
async fn test_url_remapper_rewrites_addresses_in_object_created_event(#[future] sqs_test_services: AwsTestServices) {
    let services = sqs_test_services.await;

    // Configure expectations for both the original and mapped events.
    let (s3_bucket_event, original_event, mapped_event) = services.create_mapper_object_created_event_pair();
    let (original_tx, mut original_rx) = unbounded_channel();
    let (mapped_tx, mut mapped_rx) = unbounded_channel();
    services
        .event_aggregation
        .mock_publish_request()
        .when_bearer_authorization(IDENTITY_PROVIDER_TOKEN)
        .when_event(original_event)
        .then_call(move || {
            let _ = original_tx.send(());
        })
        .then_reply_with_ok();
    services
        .event_aggregation
        .mock_publish_request()
        .when_bearer_authorization(IDENTITY_PROVIDER_TOKEN)
        .when_event(mapped_event)
        .then_call(move || {
            let _ = mapped_tx.send(());
        })
        .then_reply_with_ok();

    // Publish an event. Two notifications must be produced.
    services.send_message(s3_bucket_event).await;

    timeout(NOTIFICATION_TIMEOUT, original_rx.recv())
        .await
        .expect("failed to receive an original event in time")
        .expect("receive half has unexpectedly closed");
    timeout(NOTIFICATION_TIMEOUT, mapped_rx.recv())
        .await
        .expect("failed to receive an mapped event in time")
        .expect("receive half has unexpectedly closed");
}

#[rstest]
#[serial(sqs)]
#[test_log::test(tokio::test)]
async fn test_url_remapper_rewrites_addresses_in_object_deleted_event(#[future] sqs_test_services: AwsTestServices) {
    let services = sqs_test_services.await;

    // Configure expectations for both the original and mapped events.
    let (s3_bucket_event, original_event, mapped_event) = services.create_mapper_object_deleted_event_pair();
    let (original_tx, mut original_rx) = unbounded_channel();
    let (mapped_tx, mut mapped_rx) = unbounded_channel();
    services
        .event_aggregation
        .mock_publish_request()
        .when_bearer_authorization(IDENTITY_PROVIDER_TOKEN)
        .when_event(original_event)
        .then_call(move || {
            let _ = original_tx.send(());
        })
        .then_reply_with_ok();
    services
        .event_aggregation
        .mock_publish_request()
        .when_bearer_authorization(IDENTITY_PROVIDER_TOKEN)
        .when_event(mapped_event)
        .then_call(move || {
            let _ = mapped_tx.send(());
        })
        .then_reply_with_ok();

    // Publish an event. Two notifications must be produced.
    services.send_message(s3_bucket_event).await;

    timeout(NOTIFICATION_TIMEOUT, original_rx.recv())
        .await
        .expect("failed to receive an original event in time")
        .expect("receive half has unexpectedly closed");
    timeout(NOTIFICATION_TIMEOUT, mapped_rx.recv())
        .await
        .expect("failed to receive an mapped event in time")
        .expect("receive half has unexpectedly closed");
}

#[rstest]
#[serial(service_bus)]
#[test_log::test(tokio::test)]
async fn test_storage_events_invalidate_stat_cache(#[future] service_bus_test_services: AzTestServices) {
    use storage_core::traits::fileobject::v1alpha::StatRequest;

    let services = service_bus_test_services.await;
    let fileobject_client = services
        .storage_service
        .fileobject_v1alpha_client(Interface::Grpc)
        .await;
    let metrics_client = services.storage_service.metrics_client();

    // Create an object.
    let path = get_random_path();
    let address = services.storage_service.az_url_builder().raw_path(&path).to_string();
    fileobject_client
        .write_random_object(&address, 32)
        .await
        .expect("object write failed");

    let metrics_1 = metrics_client.get_stat_cache_metrics().await;
    assert_eq!(metrics_1.cache_access, 0);
    assert_eq!(metrics_1.cache_miss, 0);

    // Populate stat cache and remember the initial metric value.
    fileobject_client
        .stat(StatRequest {
            resource_address: address.clone(),
        })
        .await
        .expect("object stat failed");

    let metrics_2 = metrics_client.get_stat_cache_metrics().await;
    assert_gt!(metrics_2.cache_access, metrics_1.cache_access);
    assert_gt!(metrics_2.cache_miss, metrics_1.cache_miss);

    // Send a storage notification.
    let (azure_event, event) = services.create_object_deleted_event_pair_for_object(&path);
    let (callback_tx, mut callback_rx) = unbounded_channel();
    services
        .event_aggregation
        .mock_publish_request()
        .when_bearer_authorization(IDENTITY_PROVIDER_TOKEN)
        .when_event(event)
        .then_call(move || {
            let _ = callback_tx.send(());
        })
        .then_reply_with_ok();

    services.send_message(&azure_event).await;

    // Wait until the message has been processed.
    timeout(NOTIFICATION_TIMEOUT, callback_rx.recv())
        .await
        .expect("failed to receive an original event in time")
        .expect("receive half has unexpectedly closed");

    // Stat the object. The metric value must increase.
    fileobject_client
        .stat(StatRequest {
            resource_address: address,
        })
        .await
        .expect("object stat failed");

    let metrics_3 = metrics_client.get_stat_cache_metrics().await;
    assert_gt!(metrics_3.cache_access, metrics_2.cache_access);
    assert_gt!(metrics_3.cache_miss, metrics_2.cache_miss);
}

#[rstest]
#[serial(service_bus)]
#[test_log::test(tokio::test)]
async fn test_mapped_storage_events_invalidate_stat_cache(#[future] service_bus_test_services: AzTestServices) {
    use storage_core::traits::fileobject::v1alpha::StatRequest;

    let services = service_bus_test_services.await;
    let fileobject_client = services
        .storage_service
        .fileobject_v1alpha_client(Interface::Grpc)
        .await;
    let metrics_client = services.storage_service.metrics_client();

    // Create an object.
    let path = get_random_path();
    let original_address = services
        .storage_service
        .remapper_original_url_builder()
        .raw_path(&path)
        .to_string();
    fileobject_client
        .write_random_object(&original_address, 32)
        .await
        .expect("object write failed");

    let metrics_1 = metrics_client.get_stat_cache_metrics().await;
    assert_eq!(metrics_1.cache_access, 0);
    assert_eq!(metrics_1.cache_miss, 0);

    // Populate stat cache and remember the initial metric value.
    let mapped_address = services
        .storage_service
        .remapper_mapped_url_builder()
        .raw_path(&path)
        .to_string();
    fileobject_client
        .stat(StatRequest {
            resource_address: mapped_address.clone(),
        })
        .await
        .expect("object stat failed");

    let metrics_2 = metrics_client.get_stat_cache_metrics().await;
    assert_gt!(metrics_2.cache_access, metrics_1.cache_access);
    assert_gt!(metrics_2.cache_miss, metrics_1.cache_miss);

    // Send a storage notification.
    let (azure_event, event, mapped_event) = services.create_mapped_object_deleted_event_pair_for_object(&path);
    let (callback_tx, mut callback_rx) = unbounded_channel();
    services
        .event_aggregation
        .mock_publish_request()
        .when_bearer_authorization(IDENTITY_PROVIDER_TOKEN)
        .when_event(event)
        .then_call(move || {
            let _ = callback_tx.send(());
        })
        .then_reply_with_ok();
    services
        .event_aggregation
        .mock_publish_request()
        .when_bearer_authorization(IDENTITY_PROVIDER_TOKEN)
        .when_event(mapped_event)
        .then_reply_with_ok();

    services.send_message(&azure_event).await;

    // Wait until the message has been processed.
    timeout(NOTIFICATION_TIMEOUT, callback_rx.recv())
        .await
        .expect("failed to receive an original event in time")
        .expect("receive half has unexpectedly closed");

    // Stat the object. The metric value must increase.
    fileobject_client
        .stat(StatRequest {
            resource_address: mapped_address,
        })
        .await
        .expect("object stat failed");

    let metrics_3 = metrics_client.get_stat_cache_metrics().await;
    assert_gt!(metrics_3.cache_access, metrics_2.cache_access);
    assert_gt!(metrics_3.cache_miss, metrics_2.cache_miss);
}

#[serial(sqs)]
#[test_log::test(tokio::test)]
async fn test_notification_client_supports_empty_credentials() {
    let services: AwsTestServices = AwsTestServices::new(true).await;

    // Configure an expectation for the object created event.
    let (s3_event, event) = services.create_object_created_event_pair();
    let (callback_tx, mut callback_rx) = unbounded_channel();
    services
        .event_aggregation
        .mock_publish_request()
        .when_no_authorization()
        .when_event(event)
        .then_call(move || {
            let _ = callback_tx.send(());
        })
        .then_reply_with_ok();

    // Publish an event. We must receive it back.
    services.send_message(&s3_event).await;

    timeout(NOTIFICATION_TIMEOUT, callback_rx.recv())
        .await
        .expect("failed to receive an event in time")
        .expect("receive half has unexpectedly closed");
}

#[rstest]
#[serial(service_bus)]
#[test_log::test(tokio::test)]
async fn test_storage_events_invalidate_list_cache() {
    use storage_core::traits::fileobject::v1alpha::StatRequest;

    let services: AzTestServices = AzTestServices::new(true, false).await;
    let fileobject_client = services
        .storage_service
        .fileobject_v1alpha_client(Interface::Grpc)
        .await;
    let metrics_client = services.storage_service.metrics_client();

    // Create an object.
    let path = get_random_path();
    let address = services.storage_service.az_url_builder().raw_path(&path).to_string();
    fileobject_client
        .write_random_object(&address, 32)
        .await
        .expect("object write failed");

    let metrics_1 = metrics_client.get_list_cache_metrics().await;
    assert_eq!(metrics_1.cache_access, 0);
    assert_eq!(metrics_1.cache_miss, 0);

    // Populate stat cache and remember the initial metric value.
    fileobject_client
        .stat(StatRequest {
            resource_address: address.clone(),
        })
        .await
        .expect("object stat failed");

    let metrics_2 = metrics_client.get_list_cache_metrics().await;
    assert_gt!(metrics_2.cache_access, metrics_1.cache_access);
    assert_gt!(metrics_2.cache_miss, metrics_1.cache_miss);

    // Send a storage notification.
    let (azure_event, event) = services.create_object_deleted_event_pair_for_object(&path);
    let (callback_tx, mut callback_rx) = unbounded_channel();
    services
        .event_aggregation
        .mock_publish_request()
        .when_bearer_authorization(IDENTITY_PROVIDER_TOKEN)
        .when_event(event)
        .then_call(move || {
            let _ = callback_tx.send(());
        })
        .then_reply_with_ok();

    services.send_message(&azure_event).await;

    // Wait until the message has been processed.
    timeout(NOTIFICATION_TIMEOUT, callback_rx.recv())
        .await
        .expect("failed to receive an original event in time")
        .expect("receive half has unexpectedly closed");

    // Stat the object. The metric value must increase.
    fileobject_client
        .stat(StatRequest {
            resource_address: address,
        })
        .await
        .expect("object stat failed");

    let metrics_3 = metrics_client.get_list_cache_metrics().await;
    assert_gt!(metrics_3.cache_access, metrics_2.cache_access);
    assert_gt!(metrics_3.cache_miss, metrics_2.cache_miss);
}

fn strip_quotes(value: &str) -> &str {
    value
        .strip_prefix('"')
        .map(|value| value.strip_suffix('"').unwrap_or(value))
        .unwrap_or(value)
}

// ---------- Synthetic directory events: tests ----------
// (fixtures & helpers are grouped with the other fixtures near the top of this file)

#[rstest]
#[serial(service_bus)]
#[test_log::test(tokio::test)]
async fn test_azure_folder_sentinels_are_not_published(
    #[future] service_bus_synthetic_dir_test_services: AzTestServices<RecordingService>,
) {
    let services = service_bus_synthetic_dir_test_services.await;
    install_success_publish_handler(&services.event_aggregation);

    let parent = get_random_alphanumeric_string(8);
    let sentinel_name = format!("{parent}/folder/.folder");
    let sentinel_address = format!(
        "https://{}.blob.core.windows.net/{}/{}",
        services.storage_account, services.storage_container, sentinel_name,
    );
    let (sentinel_event, _) = services.create_object_deleted_event_pair_for_object(sentinel_name);
    services.send_message(&sentinel_event).await;

    wait_for_dir_events(
        &services.event_aggregation,
        &[(
            "omni.storage.dir_deleted",
            format!(
                "https://{}.blob.core.windows.net/{}/{}/folder/",
                services.storage_account, services.storage_container, parent,
            ),
        )],
        NOTIFICATION_TIMEOUT,
    )
    .await;
    assert!(!received_event(
        &services.event_aggregation,
        "omni.storage.deleted",
        &sentinel_address,
    ));

    let (empty_name_event, empty_name_notification) = services.create_object_deleted_event_pair_for_object("");
    let empty_name_address = empty_name_notification
        .resource
        .expect("empty object name event resource is missing")
        .resource_id;
    services.send_message(&empty_name_event).await;
    wait_for_dir_events(
        &services.event_aggregation,
        &[("omni.storage.deleted", empty_name_address)],
        NOTIFICATION_TIMEOUT,
    )
    .await;
}

#[rstest]
#[serial(sqs)]
#[test_log::test(tokio::test)]
async fn test_s3_folder_sentinels_are_not_published(
    #[future] sqs_synthetic_dir_test_services: AwsTestServices<RecordingService>,
) {
    let services = sqs_synthetic_dir_test_services.await;
    install_success_publish_handler(&services.event_aggregation);

    let bucket = get_random_alphanumeric_string(8);
    let region = "us-west-1";
    let parent = get_random_alphanumeric_string(8);
    let sentinel_key = format!("{parent}/folder/");
    let sentinel_address = format!("https://{bucket}.s3.{region}.amazonaws.com/{sentinel_key}");
    let sentinel_event = create_s3_event([create_s3_event_record(
        "ObjectRemoved:Delete",
        &bucket,
        region,
        sentinel_key,
        get_random_alphanumeric_string(16),
        get_random_alphanumeric_string(16),
        get_random_alphanumeric_string(32),
        datetime!(2029-01-02 03:04:05 UTC),
    )]);
    services.send_message(sentinel_event).await;

    wait_for_dir_events(
        &services.event_aggregation,
        &[(
            "omni.storage.dir_deleted",
            format!("https://{bucket}.s3.{region}.amazonaws.com/{parent}/"),
        )],
        NOTIFICATION_TIMEOUT,
    )
    .await;
    assert!(!received_event(
        &services.event_aggregation,
        "omni.storage.deleted",
        &sentinel_address,
    ));

    let dot_folder_key = format!("{parent}/.folder");
    let dot_folder_address = format!("https://{bucket}.s3.{region}.amazonaws.com/{dot_folder_key}");
    let dot_folder_event = create_s3_event([create_s3_event_record(
        "ObjectRemoved:Delete",
        bucket,
        region,
        dot_folder_key,
        get_random_alphanumeric_string(16),
        get_random_alphanumeric_string(16),
        get_random_alphanumeric_string(32),
        datetime!(2029-01-02 03:04:06 UTC),
    )]);
    services.send_message(dot_folder_event).await;
    wait_for_dir_events(
        &services.event_aggregation,
        &[("omni.storage.deleted", dot_folder_address)],
        NOTIFICATION_TIMEOUT,
    )
    .await;
}

#[rstest]
#[serial(service_bus)]
#[test_log::test(tokio::test)]
async fn test_azure_service_bus_emits_synthetic_dir_created_events(
    #[future] service_bus_synthetic_dir_test_services: AzTestServices<RecordingService>,
) {
    let services = service_bus_synthetic_dir_test_services.await;
    install_success_publish_handler(&services.event_aggregation);

    // 2-segment blob name → 2 ancestor dir URLs to expect.
    let blob_name = format!(
        "{}/{}",
        get_random_alphanumeric_string(8),
        get_random_alphanumeric_string(8),
    );
    let sequencer = get_random_alphanumeric_string(32);
    let time = datetime!(2028-10-11 11:12:13 UTC);

    // Create the blob so its etag is real on the wire (the storage-service consumer fetches it
    // to populate `resource_identity` on the published event).
    use bytes::Bytes;
    let blob_properties = services
        .blob_client
        .container_client(&services.storage_container)
        .blob_client(&blob_name)
        .put_block_blob(Bytes::from_static(b"bytes"))
        .await
        .expect("failed to create a blob");
    let etag = strip_quotes(&blob_properties.etag).to_string();

    let az_event = create_azure_blob_created_event(
        &services.storage_account,
        &services.storage_container,
        &blob_name,
        &etag,
        &sequencer,
        time,
    );

    let (seg1, _seg2) = blob_name.split_once('/').expect("blob name should have 2 segments");
    let expected = vec![
        (
            "omni.storage.dir_created",
            format!(
                "https://{}.blob.core.windows.net/{}/{}/",
                services.storage_account, services.storage_container, seg1,
            ),
        ),
        (
            "omni.storage.dir_created",
            format!(
                "https://{}.blob.core.windows.net/{}/",
                services.storage_account, services.storage_container,
            ),
        ),
    ];

    services.send_message(&az_event).await;
    wait_for_dir_events(&services.event_aggregation, &expected, NOTIFICATION_TIMEOUT).await;
}

#[rstest]
#[serial(service_bus)]
#[test_log::test(tokio::test)]
async fn test_azure_service_bus_emits_synthetic_dir_deleted_events(
    #[future] service_bus_synthetic_dir_test_services: AzTestServices<RecordingService>,
) {
    let services = service_bus_synthetic_dir_test_services.await;
    install_success_publish_handler(&services.event_aggregation);

    let blob_name = format!(
        "{}/{}",
        get_random_alphanumeric_string(8),
        get_random_alphanumeric_string(8),
    );
    let (az_event, _original_event) = services.create_object_deleted_event_pair_for_object(blob_name.clone());

    let (seg1, _seg2) = blob_name.split_once('/').expect("blob name should have 2 segments");
    let expected = vec![
        (
            "omni.storage.dir_deleted",
            format!(
                "https://{}.blob.core.windows.net/{}/{}/",
                services.storage_account, services.storage_container, seg1,
            ),
        ),
        (
            "omni.storage.dir_deleted",
            format!(
                "https://{}.blob.core.windows.net/{}/",
                services.storage_account, services.storage_container,
            ),
        ),
    ];

    services.send_message(&az_event).await;
    wait_for_dir_events(&services.event_aggregation, &expected, NOTIFICATION_TIMEOUT).await;
}

#[rstest]
#[serial(sqs)]
#[test_log::test(tokio::test)]
async fn test_sqs_emits_synthetic_dir_created_events(
    #[future] sqs_synthetic_dir_test_services: AwsTestServices<RecordingService>,
) {
    let services = sqs_synthetic_dir_test_services.await;
    install_success_publish_handler(&services.event_aggregation);

    // 3-segment key so we get 2 emitted ancestor dirs. For S3 URLs the bucket is the authority
    // (not a path segment), so `ancestor_dirs` does NOT emit `https://<bucket>.s3.<region>.../`
    // — see `storage-service/src/common/dir_ancestors.rs`. We use 3 segments to confirm we get
    // two intermediate ancestors instead of relying on the bucket root.
    let bucket = get_random_alphanumeric_string(8);
    let region = "us-west-1";
    let seg1 = get_random_alphanumeric_string(8);
    let seg2 = get_random_alphanumeric_string(8);
    let seg3 = get_random_alphanumeric_string(8);
    let key = format!("{seg1}/{seg2}/{seg3}");
    let sequencer = get_random_alphanumeric_string(32);
    let time = datetime!(2029-01-02 03:04:05 UTC);
    let version_id = get_random_alphanumeric_string(16);
    let etag = get_random_alphanumeric_string(16);

    let s3_event = create_s3_event([create_s3_event_record(
        "ObjectCreated:Put",
        &bucket,
        region,
        &key,
        &version_id,
        &etag,
        &sequencer,
        time,
    )]);

    let expected = vec![
        (
            "omni.storage.dir_created",
            format!("https://{}.s3.{}.amazonaws.com/{}/{}/", bucket, region, seg1, seg2),
        ),
        (
            "omni.storage.dir_created",
            format!("https://{}.s3.{}.amazonaws.com/{}/", bucket, region, seg1),
        ),
    ];

    services.send_message(&s3_event).await;
    wait_for_dir_events(&services.event_aggregation, &expected, NOTIFICATION_TIMEOUT).await;
}

#[rstest]
#[serial(sqs)]
#[test_log::test(tokio::test)]
async fn test_sqs_url_remapper_emits_dir_events_for_both_rewritten_and_original_paths(
    #[future] sqs_synthetic_dir_test_services: AwsTestServices<RecordingService>,
) {
    // The synthetic-dir-events observer sits inside the url_remapper's double-consume loop.
    // On the happy path (no inner failure — guaranteed by `install_success_publish_handler`)
    // BOTH the rewritten and the original-address ancestors must reach the notification service.
    let services = sqs_synthetic_dir_test_services.await;
    install_success_publish_handler(&services.event_aggregation);

    let (s3_event, _original_event, _mapped_event) = services.create_mapper_object_created_event_pair();

    // Probe one ancestor URL from each address space — confirming both proves the dual-publish
    // path. The fixture's mapped prefix is `omniverse://myhost.mydomain/mapped/path/`, original
    // is `https://mappedbucket.s3.us-west-2.amazonaws.com/original/path/`.
    let expected = vec![
        (
            "omni.storage.dir_created",
            "omniverse://myhost.mydomain/mapped/path/".to_string(),
        ),
        (
            "omni.storage.dir_created",
            format!(
                "https://{}.s3.{}.amazonaws.com/{}",
                REMAPPER_ORIGINAL_BUCKET, REMAPPER_ORIGINAL_REGION, REMAPPER_ORIGINAL_PATH,
            ),
        ),
    ];

    services.send_message(&s3_event).await;
    wait_for_dir_events(&services.event_aggregation, &expected, NOTIFICATION_TIMEOUT).await;
}

#[test]
fn create_dir_event_shape() {
    // Lightweight check that the shared `create_dir_event` helper produces the wire shape the
    // synthetic-dir-events flow expects (event_type, resource_id, single resource_address
    // message field). Keeps the integration tests above honest about what they're asserting on.
    let time = datetime!(2030-01-02 03:04:05 UTC);
    let event = create_dir_event("omni.storage.dir_created", "https://x/y/", time);
    assert_eq!(event.event_type, "omni.storage.dir_created");
    assert_eq!(event.resource.as_ref().expect("resource").resource_id, "https://x/y/");
}
