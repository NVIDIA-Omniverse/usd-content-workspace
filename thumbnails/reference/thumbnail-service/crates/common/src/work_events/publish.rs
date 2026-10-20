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

//! Publishing `omni.thumbnail.generate` — durable work dispatch, not news.
//!
//! Deliberately separate from `notifications::event_publisher::publish_event`,
//! which swallows its errors because an association event nobody receives costs
//! only a missing notification. A generate event nobody receives costs the
//! thumbnail itself, so this returns a `Result` and the CREATE handler's
//! acknowledgement decision is made from it.

use prost_types::Timestamp;
use tonic::Code;
use tracing::{debug, error, info};

use notifications_proto::nvidia::omniverse::notifications::publisher::v1beta::{
    event_publishing_service_client::EventPublishingServiceClient, Event, EventResource,
    PublishEventRequest,
};

use super::retry::is_retryable_tonic_code;
use super::{GenerateWorkMessage, GENERATE_EVENT_TYPE};
use crate::outbound::OutboundClients;

/// Why a generate event was not durably accepted by event aggregation.
#[derive(Debug, thiserror::Error)]
pub enum GeneratePublishError {
    /// No event-aggregation channel was built at startup.
    ///
    /// Treated as retryable so the CREATE is never acknowledged for work that
    /// was never dispatched. A deployment that reaches this is misconfigured and
    /// the redeliveries are the signal; silently skipping the publish would lose
    /// the work with no trace at all.
    #[error(
        "event publishing channel is not configured; NOTIFICATIONS_EVENT_AGGREGATION_ENDPOINT \
         is required to dispatch generate work"
    )]
    ChannelUnconfigured,

    #[error("generate publish failed: {0}")]
    Transport(#[from] tonic::Status),

    /// The call succeeded but aggregation reported the event as not published.
    ///
    /// Retryable: the reason is a free-form string this service cannot classify,
    /// and redelivering costs a duplicate generate the worker absorbs through
    /// destination idempotency, while discarding costs the thumbnail. A reason
    /// that never clears still terminates, because StorageEventQueue's own delivery limit
    /// dead-letters the CREATE.
    #[error("event aggregation rejected the generate event: {reason}")]
    Rejected { reason: String },
}

impl GeneratePublishError {
    /// Whether the CREATE delivery should be left unacknowledged.
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::ChannelUnconfigured | Self::Rejected { .. } => true,
            Self::Transport(status) => is_retryable_tonic_code(status.code()),
        }
    }
}

/// Publish one unit of thumbnail work and report whether it was accepted.
///
/// The envelope's `resource_id` is the source address, matching the
/// associated-event convention so a durable queue or dead-letter listing can be
/// filtered and triaged by source.
///
/// A single `Unauthenticated` refresh-and-reissue is part of one attempt, the
/// same contract every other outbound call site here follows. It is not an
/// application retry: the pipeline never republishes the same size twice within
/// one delivery.
pub async fn publish_generate_event(
    outbound: &OutboundClients,
    message: &GenerateWorkMessage,
) -> Result<(), GeneratePublishError> {
    let Some(service) = outbound.traced_event_publishing() else {
        return Err(GeneratePublishError::ChannelUnconfigured);
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();

    // Captured here rather than at build time, and authoritative over whatever the
    // message carried: the parent the worker joins must be the span that actually
    // published, or queue dwell lands on the wrong request.
    let mut body = message.to_struct();
    super::CapturedTraceContext::capture().replace_in(&mut body.fields);

    let event = Event {
        event_type: GENERATE_EVENT_TYPE.to_string(),
        message: Some(body),
        occurred_at: Some(Timestamp {
            seconds: now.as_secs() as i64,
            nanos: now.subsec_nanos() as i32,
        }),
        resource: Some(EventResource {
            resource_id: message.envelope_resource_id().to_string(),
        }),
    };

    debug!(
        destination = %message.destination_resource_address,
        source = %message.source.resource_address,
        width = message.specification.width,
        height = message.specification.height,
        "Publishing generate work event"
    );

    let mut client = EventPublishingServiceClient::new(service);

    let request = PublishEventRequest {
        event: Some(event.clone()),
    };
    // Bounded per call: an unbounded publish lets a stalled notifications service
    // hold a CreateThumbnails request open indefinitely, and the fan-out is
    // sequential so one stall blocks every specification behind it.
    let mut publish = tonic::Request::new(request);
    publish.set_timeout(
        crate::current_runtime_config()
            .service
            .outbound_request_timeout,
    );
    let response = match client.publish_event(publish).await {
        Ok(response) => response,
        Err(status) if status.code() == Code::Unauthenticated => {
            info!("Generate publish got Unauthenticated; refreshing the token and reissuing once");
            outbound.refresher.refresh().await;
            // The reissue is part of the same attempt, so it carries the same
            // deadline; leaving it unbounded would reintroduce the stall this
            // guards against on the retry path only.
            let mut reissue = tonic::Request::new(PublishEventRequest { event: Some(event) });
            reissue.set_timeout(
                crate::current_runtime_config()
                    .service
                    .outbound_request_timeout,
            );
            client.publish_event(reissue).await?
        }
        Err(status) => return Err(status.into()),
    };

    match response.into_inner().result {
        Some(result) if result.success => {
            info!(
                destination = %message.destination_resource_address,
                "Generate work event published"
            );
            Ok(())
        }
        Some(result) => {
            error!(
                destination = %message.destination_resource_address,
                reason = %result.failure_reason,
                "Event aggregation reported the generate event as not published"
            );
            Err(GeneratePublishError::Rejected {
                reason: result.failure_reason,
            })
        }
        // A response with no result asserts nothing about whether the work was
        // enqueued, and ACKing the CREATE would claim it was.
        None => Err(GeneratePublishError::Rejected {
            reason: "publish response carried no result".to_string(),
        }),
    }
}
