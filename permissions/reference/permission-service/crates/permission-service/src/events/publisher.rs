// SPDX-FileCopyrightText: Copyright 2026 NVIDIA CORPORATION & AFFILIATES
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

//! Publishing permission- and metadata-change events to the Notification Service.

use super::{EVENT_TYPE_META_CHANGED, EVENT_TYPE_PERMISSIONS_CHANGED};
use crate::state::AppState;
use clap::Args;
use opentelemetry::KeyValue;
use permission_core::{Action, Principal, Resource};
use permission_service_notification_proto::nvidia::omniverse::notifications::publisher::v1beta::{
    Event, EventResource, PublishEventRequest,
};
use prost_types::{Struct, Timestamp, Value};
use std::collections::BTreeMap;
use std::time::Instant;

#[derive(Args, Clone, Debug)]
pub struct EventPublishingArgs {
    /// Defines if the service needs to publish events about permissions updates.
    #[arg(
        id = "enable-notifications",
        long,
        env = "ENABLE_NOTIFICATIONS",
        requires_ifs = [("true", "event-publishing-grpc-endpoint")],
    )]
    pub enabled: bool,

    /// Defines an endpoint for Event Aggregation API of the Notification Service
    /// to publish events about policy updates.
    #[arg(
        id = "event-publishing-grpc-endpoint",
        long,
        env = "EVENT_PUBLISHING_GRPC_ENDPOINT",
        default_value = ""
    )]
    pub event_publishing_grpc_endpoint: String,

    /// Enables TLS for the outbound gRPC connection to the Notification Service.
    /// When set, the connection is verified against the platform's native trust
    /// store unless `event-publishing-tls-ca-certificate-file` provides an
    /// explicit CA bundle.
    #[arg(
        id = "event-publishing-tls-enabled",
        long,
        env = "EVENT_PUBLISHING_TLS_ENABLED",
        action = clap::ArgAction::Set,
        default_value_t = false,
        default_missing_value = "true",
        num_args = 0..=1,
        require_equals = false,
    )]
    pub tls_enabled: bool,

    /// Path to a PEM-encoded CA certificate bundle used to verify the
    /// Notification Service server certificate. When empty, the platform's
    /// native root certificate store is used.
    #[arg(
        id = "event-publishing-tls-ca-certificate-file",
        long,
        env = "EVENT_PUBLISHING_TLS_CA_CERTIFICATE_FILE",
        default_value = ""
    )]
    pub tls_ca_certificate_file: String,

    /// Optional override for the TLS server name (SNI / peer name) presented
    /// when connecting to the Notification Service. Defaults to the hostname
    /// taken from `event-publishing-grpc-endpoint`.
    #[arg(
        id = "event-publishing-tls-server-name",
        long,
        env = "EVENT_PUBLISHING_TLS_SERVER_NAME",
        default_value = ""
    )]
    pub tls_server_name: String,
}

#[tracing::instrument(level = "info", skip(state))]
pub fn publish_permission_event(
    state: &AppState,
    principal: Option<&Principal>,
    action: Option<&Action>,
    resource: Option<&Resource>,
) {
    if state.event_client.is_none() {
        return;
    }

    let event = Event {
        event_type: EVENT_TYPE_PERMISSIONS_CHANGED.to_string(),
        message: Some(Struct {
            fields: BTreeMap::from([
                (
                    "principal".to_string(),
                    prost_value_string(principal.map(|p| p.id.clone())),
                ),
                (
                    "action".to_string(),
                    prost_value_string(action.map(|a| a.entity_id())),
                ),
                ("resource".to_string(), prost_value_string(resource)),
            ]),
        }),
        occurred_at: Some(now_timestamp()),
        resource: resource.map(|r| EventResource {
            resource_id: r.id.clone(),
        }),
    };

    spawn_publish(state.clone(), event);
}

#[tracing::instrument(level = "info", skip(state))]
pub fn publish_metadata_event(
    state: &AppState,
    service: &str,
    entity: MetadataEntity,
    operation: MetadataOperation,
    name: Option<&str>,
) {
    if state.event_client.is_none() {
        return;
    }

    let event = Event {
        event_type: EVENT_TYPE_META_CHANGED.to_string(),
        message: Some(Struct {
            fields: BTreeMap::from([
                ("service".to_string(), prost_value_string(Some(service))),
                (
                    "entity".to_string(),
                    prost_value_string(Some(entity.as_str())),
                ),
                (
                    "operation".to_string(),
                    prost_value_string(Some(operation.as_str())),
                ),
                ("name".to_string(), prost_value_string(name)),
            ]),
        }),
        occurred_at: Some(now_timestamp()),
        resource: Some(EventResource {
            resource_id: service.to_string(),
        }),
    };

    spawn_publish(state.clone(), event);
}

fn spawn_publish(state: AppState, event: Event) {
    tokio::spawn(async move {
        if let Some(client) = state.event_client {
            let event_type = event.event_type.clone();
            let request = PublishEventRequest { event: Some(event) };
            tracing::debug!("Publishing event: {:#?}", request);

            let start = Instant::now();
            let mut result = client.publish_event(request.clone()).await;
            let mut retried = false;

            if let Err(status) = &result {
                if status.code() == tonic::Code::Unauthenticated {
                    if let Some(refresher) = client.refresher() {
                        tracing::warn!(
                            "Received Unauthenticated from event service, refreshing token and retrying"
                        );
                        refresher.refresh().await;
                        result = client.publish_event(request).await;
                        retried = true;
                    }
                }
            }

            let outcome = match (&result, retried) {
                (Ok(_), false) => "success",
                (Ok(_), true) => "retried_success",
                (Err(_), false) => "failure",
                (Err(_), true) => "retried_failure",
            };
            state.metrics.notification_publish_count.add(
                1,
                &[
                    KeyValue::new("outcome", outcome),
                    KeyValue::new("event_type", event_type.clone()),
                ],
            );
            state
                .metrics
                .notification_publish_duration
                .record(start.elapsed().as_millis() as u64, &[]);

            match result {
                Ok(_) => tracing::debug!("Successfully published {} event", event_type),
                Err(err) => tracing::warn!("Failed to publish {} event: {}", event_type, err),
            }
        }
    });
}

fn now_timestamp() -> Timestamp {
    Timestamp {
        seconds: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64,
        nanos: 0,
    }
}

fn prost_value_string(val: Option<impl ToString>) -> Value {
    Value {
        kind: Some(prost_types::value::Kind::StringValue(
            val.map(|v| v.to_string()).unwrap_or_default(),
        )),
    }
}

#[derive(Clone, Copy, Debug)]
pub enum MetadataEntity {
    Service,
    Action,
    ResourceType,
}

impl MetadataEntity {
    fn as_str(self) -> &'static str {
        match self {
            MetadataEntity::Service => "service",
            MetadataEntity::Action => "action",
            MetadataEntity::ResourceType => "resource_type",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum MetadataOperation {
    Updated,
    Removed,
}

impl MetadataOperation {
    fn as_str(self) -> &'static str {
        match self {
            MetadataOperation::Updated => "updated",
            MetadataOperation::Removed => "removed",
        }
    }
}
