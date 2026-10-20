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

//! Notification event consumer service.
//!
//! This module owns the long-lived gRPC consumer used for `omni.storage.*` durable events. The
//! outer service binaries decide whether HTTP or gRPC should host the consumer, but the stream
//! lifecycle, auth, filtering, and event-to-thumbnail-task handoff all live here.

use std::sync::Arc;
use thumbnail_auth::{AuthInterceptor, TokenRefresher};
use tonic::transport::Channel;

use crate::auth::TokenGate;
use notifications_proto::nvidia::omniverse::notifications::consumer::v1beta::event_consumer_service_client::EventConsumerServiceClient;
use serde::{Deserialize, Serialize};

mod connection;
pub mod event_publisher;
// Expose for testing
#[cfg(test)]
pub mod event_processing;
#[cfg(not(test))]
mod event_processing;
mod helpers;
pub mod provisioning;
pub mod redelivery;
pub mod session;
mod streaming;

pub use helpers::is_auth_error;
pub use notifications_proto::nvidia::omniverse::notifications::consumer::v1beta::FilterGroup;
pub use redelivery::{DeferredNacks, RedeliveryTracker};
pub use session::{
    open_ack_stream, reconnect_backoff, send_ack, send_nack, AckStream, AckStreamClient, NackError,
    OpenError,
};

/// Configuration for the long-lived event consumer.
///
/// Constructed by the binary's startup wiring from a single [`crate::OutboundClients`].
/// The `endpoint` is the address of the event-consumer service; the gRPC channel
/// is rebuilt on every reconnect (since the underlying connection may have died),
/// but the same interceptor and refresher are reused across reconnects so the
/// 401-retry contract has a stable target.
#[derive(Clone)]
pub struct EventConsumerConfig {
    /// Endpoint URL (with or without scheme prefix) for the event-consumer service.
    pub endpoint: String,
    /// gRPC auth interceptor selected at startup; cloned onto each new channel.
    pub interceptor: AuthInterceptor,
    /// Token refresher for 401-retry; outlives reconnects.
    pub refresher: Arc<dyn TokenRefresher>,
    /// Shared startup/expiry probe for the same token source as `refresher`.
    pub token_gate: TokenGate,
    /// Filter groups used to provision the durable queue at startup and to
    /// re-provision it if the queue is found to no longer exist during streaming
    /// (NOT_FOUND recovery path). Stored here so the consume loop can call
    /// `ensure_durable_queue` without needing a separate parameter thread.
    pub filter_groups: Vec<FilterGroup>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageEvent {
    // Original event type from the notifications service, for example `omni.storage.created`.
    pub event_type: String,
    pub principal_identity: String,
    pub occurred_at: String,
    pub published_at: String,
    // Normalized payload fields extracted from the protobuf `Struct` body.
    pub message: StorageEventMessage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageEventMessage {
    // Present when the upstream event already includes a stable identity.
    pub resource_identity: Option<String>,
    // Human-readable storage address used by filtering and logging.
    pub resource_address: String,
    // Optional mime type used to avoid extension-only routing when available.
    pub mime_type: Option<String>,
    /// When true, identifies storage events for blobs written by the thumbnail pipeline.
    pub generated_by_thumbnail_pipeline: Option<bool>,
    /// W3C trace context carried by the upstream event, when it supplied one.
    /// Forwarded onto the generate events this CREATE fans out to, so the
    /// conversion work stays on the same trace as the storage write.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub traceparent: Option<String>,
    /// The other half of the W3C context. Vendor state, including sampling
    /// decisions made upstream, lives here — so extracting `traceparent` alone
    /// honors the trace and quietly overrides the decision about whether to
    /// record it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracestate: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageEventType {
    Created,
    Deleted,
    DirCreated,
    DirDeleted,
}

impl StorageEventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            StorageEventType::Created => "omni.storage.created",
            StorageEventType::Deleted => "omni.storage.deleted",
            StorageEventType::DirCreated => "omni.storage.dir_created",
            StorageEventType::DirDeleted => "omni.storage.dir_deleted",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "omni.storage.created" => Some(StorageEventType::Created),
            "omni.storage.deleted" => Some(StorageEventType::Deleted),
            "omni.storage.dir_created" => Some(StorageEventType::DirCreated),
            "omni.storage.dir_deleted" => Some(StorageEventType::DirDeleted),
            _ => None,
        }
    }
}

pub struct EventConsumer {
    pub(crate) config: EventConsumerConfig,
    /// Active streaming client. Recreated on reconnects when auth or transport fails.
    pub(crate) grpc_client: Option<
        EventConsumerServiceClient<
            tonic::service::interceptor::InterceptedService<Channel, AuthInterceptor>,
        >,
    >,
}
