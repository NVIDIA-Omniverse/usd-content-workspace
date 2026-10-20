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

//! Test fixtures and mock data for thumbnail-common crate - provides reusable test data, mock implementations, and testing utilities for both isolated unit tests and integration tests

/// Scriptable in-process `EventConsumerService` gRPC server. Lives here (not
/// under a `#[cfg(test)]` module) so both `thumbnail-common`'s own tests and
/// downstream crates (e.g. `thumbnail-service`) can stand one up to exercise
/// durable-queue provisioning glue against a live server.
pub mod mock_consumer_server;

/// Stateful in-process `MetadataService` gRPC server standing in for storage.
/// [`init_test_runtime_config`] starts one per test binary and points the storage
/// endpoint at it, so storage-failure paths are testable and no test can reach a real
/// storage service.
pub mod mock_storage_server;

/// Recording in-process `EventPublishingService` gRPC server, so tests can assert on
/// the fire-and-forget association events the service publishes.
pub mod mock_publisher_server;

pub mod trace_probe;

use crate::notifications::EventConsumerConfig;
use crate::notifications::*;
use crate::RuntimeConfigArgs;
use chrono::{DateTime, Utc};
use clap::Parser;
use std::collections::HashMap;
use std::sync::{Arc, Once};
use tokio::sync::Mutex;

/// Install a test `RuntimeConfigArgs` with a fixed, valid set of `THUMBNAIL_*`
/// values, once per process. Shared by `thumbnail-service`'s HTTP and gRPC
/// test suites so the bootstrap env vars and install sequence live in
/// exactly one place.
///
/// `bin_name` only affects clap's binary-name display (e.g. in `--help`
/// output); it has no effect on the parsed values since no CLI args beyond
/// argv[0] are passed.
///
/// Also starts the [`mock_storage_server`] and points `STORAGE_GRPC_ENDPOINT` at it.
/// Because the config installs exactly once per process, this cannot be opt-in without
/// making the endpoint depend on which test ran first; wiring it unconditionally keeps
/// it deterministic and guarantees no test reaches a real storage service. Reach the
/// double with [`mock_storage_server::mock_storage`].
pub fn init_test_runtime_config(bin_name: &str) -> &'static RuntimeConfigArgs {
    #[derive(Parser, Debug)]
    struct TestRuntimeCli {
        #[command(flatten)]
        runtime: RuntimeConfigArgs,
    }

    static INIT: Once = Once::new();
    INIT.call_once(|| {
        std::env::set_var(
            "GENERATED_THUMBNAIL_PRINCIPAL_IDENTITY",
            "thumbnail-task-queue",
        );
        std::env::set_var(
            "THUMBNAIL_PATH_TEMPLATE",
            "{resource_path}/.thumbs/{width}x{height}/{resource_filename}.auto.{output_encoding}",
        );
        std::env::set_var("THUMBNAIL_SIZES", "256x256");
        std::env::set_var("THUMBNAIL_ENCODING_FORMAT", "PNG");
        std::env::set_var("THUMBNAIL_TRANSFORM_TYPE", "SQUARE");
        std::env::set_var("THUMBNAIL_CROP_STRATEGY", "SMART");
        std::env::set_var("THUMBNAIL_QUALITY", "85");
        std::env::set_var("THUMBNAIL_PRESERVE_ASPECT_RATIO", "true");
        std::env::set_var(
            "STORAGE_GRPC_ENDPOINT",
            mock_storage_server::start_mock_storage().to_string(),
        );
        // Development mode replaces real query results with canned ones, which would
        // quietly invalidate every test that reads storage back. The CI job sets
        // DEVELOPMENT_MODE=true, so the suite has to pin it off rather than inherit it.
        std::env::set_var("DEVELOPMENT_MODE", "false");
        // Outbound deadlines short enough to assert on. The 30s production
        // defaults are correct in a deployment and make a test that proves a
        // stalled dependency is bounded cost 30s of suite time to do it.
        std::env::set_var("STORAGE_GRPC_REQUEST_TIMEOUT", "2s");
        std::env::set_var("OUTBOUND_REQUEST_TIMEOUT", "2s");
        crate::storage::bypass_channel_cache_for_tests();
        let runtime = TestRuntimeCli::parse_from([bin_name.to_string()]).runtime;
        let _ = crate::install_runtime_config(runtime);
    });

    crate::current_runtime_config()
}

/// Mock data builder for creating consistent test fixtures
pub struct MockDataBuilder {
    timestamp: DateTime<Utc>,
    principal_identity: String,
    base_url: String,
}

impl Default for MockDataBuilder {
    fn default() -> Self {
        Self {
            timestamp: DateTime::parse_from_rfc3339("2024-01-01T12:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            principal_identity: "test-service".to_string(),
            base_url: "https://checkpointblobs.blob.core.windows.net/forthumbnails".to_string(),
        }
    }
}

impl MockDataBuilder {
    /// Create a new mock data builder
    pub fn new() -> Self {
        Self::default()
    }

    /// Set custom timestamp for test events
    pub fn with_timestamp(mut self, timestamp: DateTime<Utc>) -> Self {
        self.timestamp = timestamp;
        self
    }

    /// Set custom principal identity
    pub fn with_principal(mut self, principal: &str) -> Self {
        self.principal_identity = principal.to_string();
        self
    }

    /// Set custom base URL
    pub fn with_base_url(mut self, url: &str) -> Self {
        self.base_url = url.to_string();
        self
    }

    /// Create a source file storage event
    pub fn source_file_event(&self, filename: &str) -> StorageEvent {
        StorageEvent {
            event_type: StorageEventType::Created.as_str().to_string(),
            principal_identity: self.principal_identity.clone(),
            occurred_at: self.timestamp.to_rfc3339(),
            published_at: self.timestamp.to_rfc3339(),
            message: StorageEventMessage {
                resource_identity: Some(format!("source_{}", filename)),
                resource_address: format!("{}/source/{}", self.base_url, filename),
                mime_type: None,
                generated_by_thumbnail_pipeline: None,
                traceparent: None,
                tracestate: None,
            },
        }
    }

    /// Create a thumbnail file storage event
    pub fn thumbnail_event(
        &self,
        source_name: &str,
        width: u32,
        height: u32,
        format: &str,
    ) -> StorageEvent {
        let thumbnail_name = format!("thumbnail_{}x{}_{}_{}", width, height, format, source_name);

        StorageEvent {
            event_type: StorageEventType::Created.as_str().to_string(),
            principal_identity: self.principal_identity.clone(),
            occurred_at: self.timestamp.to_rfc3339(),
            published_at: self.timestamp.to_rfc3339(),
            message: StorageEventMessage {
                resource_identity: Some(format!("thumb_{}", thumbnail_name)),
                resource_address: format!("{}/thumbnails/{}", self.base_url, thumbnail_name),
                mime_type: None,
                generated_by_thumbnail_pipeline: None,
                traceparent: None,
                tracestate: None,
            },
        }
    }

    /// Create a delete event
    pub fn delete_event(&self, resource_path: &str) -> StorageEvent {
        StorageEvent {
            event_type: StorageEventType::Deleted.as_str().to_string(),
            principal_identity: self.principal_identity.clone(),
            occurred_at: self.timestamp.to_rfc3339(),
            published_at: self.timestamp.to_rfc3339(),
            message: StorageEventMessage {
                resource_identity: Some(format!("deleted_{}", resource_path)),
                resource_address: format!("{}/{}", self.base_url, resource_path),
                mime_type: None,
                generated_by_thumbnail_pipeline: None,
                traceparent: None,
                tracestate: None,
            },
        }
    }

    /// Create a directory creation event
    pub fn directory_event(&self, dir_path: &str) -> StorageEvent {
        StorageEvent {
            event_type: StorageEventType::DirCreated.as_str().to_string(),
            principal_identity: self.principal_identity.clone(),
            occurred_at: self.timestamp.to_rfc3339(),
            published_at: self.timestamp.to_rfc3339(),
            message: StorageEventMessage {
                resource_identity: Some(format!("dir_{}", dir_path)),
                resource_address: format!("{}/{}/", self.base_url, dir_path),
                mime_type: None,
                generated_by_thumbnail_pipeline: None,
                traceparent: None,
                tracestate: None,
            },
        }
    }
}

/// Mock implementations for testing without external dependencies
pub mod mocks {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Mock event consumer that doesn't require actual gRPC connections
    pub struct MockEventConsumer {
        pub config: EventConsumerConfig,
        pub events_received: Arc<Mutex<Vec<StorageEvent>>>,
        pub queue_id: Option<String>,
        pub queue_path: Option<String>,
        pub connection_attempts: AtomicUsize,
    }

    impl MockEventConsumer {
        pub fn new(config: EventConsumerConfig) -> Self {
            Self {
                config,
                events_received: Arc::new(Mutex::new(Vec::new())),
                queue_id: None,
                queue_path: None,
                connection_attempts: AtomicUsize::new(0),
            }
        }

        /// Simulate receiving an event
        pub async fn simulate_event(&self, event: StorageEvent) {
            let mut events = self.events_received.lock().await;
            events.push(event);
        }

        /// Get count of received events
        pub async fn event_count(&self) -> usize {
            self.events_received.lock().await.len()
        }

        /// Get all received events
        pub async fn get_events(&self) -> Vec<StorageEvent> {
            self.events_received.lock().await.clone()
        }

        /// Simulate connection attempt
        pub fn simulate_connection_attempt(&self) -> usize {
            self.connection_attempts.fetch_add(1, Ordering::SeqCst) + 1
        }
    }

    /// Mock authentication for testing without real tokens
    pub struct MockAuth {
        pub should_succeed: bool,
        pub token: String,
    }

    impl MockAuth {
        pub fn success() -> Self {
            Self {
                should_succeed: true,
                token: "mock_valid_token_12345".to_string(),
            }
        }

        pub fn failure() -> Self {
            Self {
                should_succeed: false,
                token: "".to_string(),
            }
        }

        pub fn get_token(&self) -> Result<String, String> {
            if self.should_succeed {
                Ok(self.token.clone())
            } else {
                Err("Mock authentication failed".to_string())
            }
        }
    }
}

/// Integration test helpers for end-to-end testing
pub mod integration {
    use super::*;

    /// Test environment setup and cleanup
    pub struct TestEnvironment {
        pub original_env: HashMap<String, String>,
    }

    impl TestEnvironment {
        /// Set up test environment with mock values
        pub fn setup() -> Self {
            let original_env = Self::save_current_env();

            // Set test environment variables only if not already set (avoid racing with #[serial] tests)
            if crate::config::raw_env_value("NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT").is_none() {
                std::env::set_var("NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT", "localhost:50002");
            }
            if crate::config::raw_env_value("CLIENT_ID").is_none() {
                std::env::set_var("CLIENT_ID", "test_client_id");
            }
            if crate::config::raw_env_value("CLIENT_SECRET").is_none() {
                std::env::set_var("CLIENT_SECRET", "test_client_secret");
            }
            if crate::config::raw_env_value("CLIENT_CREDENTIALS_OPENID_CONFIGURATION").is_none() {
                std::env::set_var(
                    "CLIENT_CREDENTIALS_OPENID_CONFIGURATION",
                    "https://test.oauth.com/.well-known/openid-configuration",
                );
            }

            Self { original_env }
        }

        /// Save current environment variables
        fn save_current_env() -> HashMap<String, String> {
            let mut env_map = HashMap::new();
            let keys = [
                "NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT",
                "CLIENT_ID",
                "CLIENT_SECRET",
                "CLIENT_CREDENTIALS_OPENID_CONFIGURATION",
            ];

            for key in &keys {
                if let Some(value) = crate::config::raw_env_value(key) {
                    env_map.insert(key.to_string(), value);
                }
            }
            env_map
        }

        /// Restore original environment
        pub fn restore(&self) {
            std::env::remove_var("NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT");
            std::env::remove_var("CLIENT_ID");
            std::env::remove_var("CLIENT_SECRET");
            std::env::remove_var("CLIENT_CREDENTIALS_OPENID_CONFIGURATION");

            for (key, value) in &self.original_env {
                std::env::set_var(key, value);
            }
        }
    }

    impl Drop for TestEnvironment {
        fn drop(&mut self) {
            self.restore();
        }
    }

    /// Test scenario builder for complex integration tests
    pub struct TestScenario {
        events: Vec<StorageEvent>,
        expected_results: Vec<String>,
    }

    impl Default for TestScenario {
        fn default() -> Self {
            Self::new()
        }
    }

    impl TestScenario {
        pub fn new() -> Self {
            Self {
                events: Vec::new(),
                expected_results: Vec::new(),
            }
        }

        /// Add a source file upload event
        pub fn add_source_upload(mut self, filename: &str) -> Self {
            let event = MockDataBuilder::new().source_file_event(filename);
            self.events.push(event);
            self.expected_results
                .push(format!("Source file uploaded: {}", filename));
            self
        }

        /// Add a thumbnail creation event
        pub fn add_thumbnail_creation(mut self, source: &str, width: u32, height: u32) -> Self {
            let event = MockDataBuilder::new().thumbnail_event(source, width, height, "PNG");
            self.events.push(event);
            self.expected_results.push(format!(
                "Thumbnail created: {}x{} for {}",
                width, height, source
            ));
            self
        }

        /// Get the events for this scenario
        pub fn events(&self) -> &[StorageEvent] {
            &self.events
        }

        /// Get expected results
        pub fn expected_results(&self) -> &[String] {
            &self.expected_results
        }
    }
}

/// Common test utilities used across multiple test files
pub mod test_utils {
    use super::*;

    /// Assert that an event is properly formed
    pub fn assert_valid_storage_event(event: &StorageEvent) {
        assert!(
            !event.event_type.is_empty(),
            "Event type should not be empty"
        );
        assert!(
            !event.principal_identity.is_empty(),
            "Principal identity should not be empty"
        );
        assert!(
            !event.occurred_at.is_empty(),
            "Occurred at should not be empty"
        );
        assert!(
            !event.published_at.is_empty(),
            "Published at should not be empty"
        );
        assert!(
            !event.message.resource_address.is_empty(),
            "Resource address should not be empty"
        );
    }

    /// Create a test [`EventConsumerConfig`] backed by an in-process
    /// [`crate::outbound::OutboundClients::for_tests`] (lazy channels +
    /// no-op refresher). Suitable for any test that just needs to
    /// instantiate an `EventConsumer` without actually issuing RPCs.
    pub fn test_config() -> EventConsumerConfig {
        let outbound = crate::outbound::OutboundClients::for_tests();
        EventConsumerConfig {
            endpoint: "localhost:50002".to_string(),
            interceptor: outbound.interceptor.clone(),
            refresher: outbound.refresher.clone(),
            token_gate: outbound.token_gate.clone(),
            filter_groups: vec![],
        }
    }

    /// Wait for async operations in tests
    pub async fn wait_for_events(duration_ms: u64) {
        tokio::time::sleep(tokio::time::Duration::from_millis(duration_ms)).await;
    }

    /// Generate test timestamps in sequence
    pub fn test_timestamps(count: usize) -> Vec<DateTime<Utc>> {
        let base = DateTime::parse_from_rfc3339("2024-01-01T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        (0..count)
            .map(|i| base + chrono::Duration::seconds(i as i64))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    fn test_mock_data_builder() {
        let builder = MockDataBuilder::new();

        let source_event = builder.source_file_event("test.jpg");
        assert_eq!(source_event.event_type, StorageEventType::Created.as_str());
        assert!(source_event.message.resource_address.contains("/source/"));
        assert!(source_event.message.resource_address.contains("test.jpg"));

        let thumb_event = builder.thumbnail_event("test.jpg", 256, 256, "PNG");
        assert!(thumb_event
            .message
            .resource_address
            .contains("/thumbnails/"));
        assert!(thumb_event.message.resource_address.contains("256x256"));
    }

    #[tokio::test]
    #[serial]
    async fn test_mock_event_consumer() {
        let config = test_utils::test_config();
        let mock_consumer = mocks::MockEventConsumer::new(config);

        assert_eq!(mock_consumer.event_count().await, 0);

        let event = MockDataBuilder::new().source_file_event("test.jpg");
        mock_consumer.simulate_event(event).await;

        assert_eq!(mock_consumer.event_count().await, 1);

        let events = mock_consumer.get_events().await;
        assert_eq!(events.len(), 1);
        assert!(events[0].message.resource_address.contains("test.jpg"));
    }

    #[test]
    #[serial]
    fn test_integration_environment() {
        let _env = integration::TestEnvironment::setup();

        assert_eq!(
            crate::config::raw_env_value("NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT")
                .expect("test fixture should set endpoint"),
            "localhost:50002"
        );
    }
}
