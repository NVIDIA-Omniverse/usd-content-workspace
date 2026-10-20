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
use std::time::Duration;

use notifications_proto::nvidia::omniverse::notifications::publisher::v1beta::{
    event_publishing_service_client::EventPublishingServiceClient, Event, EventResource,
    PublishEventRequest,
};
use prost_types::Timestamp;
use tonic::Code;
use tracing::{debug, error, info, warn};

use crate::outbound::OutboundClients;
use crate::storage::{AssociationInvariant, AssociationMessageParams};

/// The association lifecycle events this service publishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssociationEventKind {
    Associated,
    Disassociated,
}

impl AssociationEventKind {
    /// The `event_type` this kind is published under.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Associated => "omni.thumbnail.associated",
            Self::Disassociated => "omni.thumbnail.disassociated",
        }
    }

    /// The resource the event is filed under: an association is news about the target
    /// that gained a thumbnail, a disassociation is news about the thumbnail that went away.
    fn resource_id(self, params: &AssociationEventParams<'_>) -> String {
        match self {
            Self::Associated => params.message.association_target.resource_address.clone(),
            Self::Disassociated => params
                .message
                .thumbnail
                .resource_identity
                .clone()
                .unwrap_or_default(),
        }
    }
}

/// Parameters for publishing an association event: which lifecycle event, and the
/// association it is about.
pub struct AssociationEventParams<'a> {
    pub event_kind: AssociationEventKind,
    pub message: AssociationMessageParams<'a>,
}

fn build_association_event(
    params: &AssociationEventParams<'_>,
) -> Result<Event, AssociationInvariant> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let occurred_at = Timestamp {
        seconds: now.as_secs() as i64,
        nanos: now.subsec_nanos() as i32,
    };

    // Single source of truth for the message shape (ADR 0002 §2/§3): the same
    // builder produces the stored association metadata, so events cannot drift.
    let message_json = crate::storage::association::build_association_message(&params.message)?;

    // `build_association_message` always returns a JSON object and `json_to_prost_value`
    // maps every object to a struct, so anything else means the message shape changed
    // out from under this conversion. Publishing the event without its payload would
    // hide that behind a subscriber receiving an empty notification, so fail instead.
    let message = match crate::utils::json_to_prost_value(&message_json) {
        Ok(prost_types::Value {
            kind: Some(prost_types::value::Kind::StructValue(s)),
        }) => s,
        Ok(value) => {
            return Err(AssociationInvariant::EventPayloadNotAStruct(format!(
                "{:?}",
                value.kind
            )))
        }
        Err(error) => return Err(AssociationInvariant::EventPayloadEncoding(error)),
    };

    Ok(Event {
        event_type: params.event_kind.as_str().to_string(),
        message: Some(message),
        occurred_at: Some(occurred_at),
        resource: Some(EventResource {
            resource_id: params.event_kind.resource_id(params),
        }),
    })
}

/// Bound a publish by the caller's `OUTBOUND_REQUEST_TIMEOUT`, as the generate
/// publish already is.
///
/// The deadline arrives as an argument rather than being read from the runtime
/// config here, and that is deliberate. `publish_event` runs inside a detached
/// `tokio::spawn` whose `JoinHandle` is dropped, so a panic in it is not observed
/// by anything — it reaches stderr and the event is silently lost.
/// `current_runtime_config()` panics when the config is not installed, which is a
/// test-only condition today but exactly the wrong kind of thing to put on a path
/// where failure is invisible. The caller reads it before spawning, where a panic
/// would surface.
fn deadlined(
    request: PublishEventRequest,
    request_timeout: Duration,
) -> tonic::Request<PublishEventRequest> {
    let mut request = tonic::Request::new(request);
    request.set_timeout(request_timeout);
    request
}

/// Publish an event to the event aggregation service.
///
/// Uses the service-account credentials selected by [`OutboundClients`]
/// (in-process client-credentials *or* sidecar access-token-file) to
/// authenticate the gRPC call. The channel and interceptor are built once at
/// startup and reused across publishes.
///
/// Errors are logged but never propagated — event publishing must not
/// block the associate / disassociate response.
///
/// `request_timeout` is passed in rather than read here; see [`deadlined`].
pub async fn publish_event(
    outbound: Arc<OutboundClients>,
    params: &AssociationEventParams<'_>,
    request_timeout: Duration,
) {
    let Some(service) = outbound.traced_event_publishing() else {
        warn!(
            "event_publishing_channel not configured — skipping {} event",
            params.event_kind.as_str()
        );
        return;
    };

    info!(
        "Publishing {} event (association_target={}, thumbnail={})",
        params.event_kind.as_str(),
        params
            .message
            .association_target
            .resource_identity
            .as_deref()
            .unwrap_or(""),
        params
            .message
            .thumbnail
            .resource_identity
            .as_deref()
            .unwrap_or(""),
    );

    let event = match build_association_event(params) {
        Ok(event) => event,
        Err(error) => {
            error!(
                event_type = params.event_kind.as_str(),
                %error,
                "Refusing to publish an association event that could not be built; \
                 an event with a missing or partial payload is worse than none"
            );
            return;
        }
    };

    debug!(
        "[EVENT_BODY] type={}, has_resource={}, has_message={}",
        event.event_type,
        event.resource.is_some(),
        event.message.is_some(),
    );

    let request = PublishEventRequest {
        event: Some(event.clone()),
    };

    let mut client = EventPublishingServiceClient::new(service);

    let response = match client
        .publish_event(deadlined(request, request_timeout))
        .await
    {
        Ok(resp) => Ok(resp),
        Err(status) if status.code() == Code::Unauthenticated => {
            info!(
                "{} event got Unauthenticated; refreshing and retrying once",
                params.event_kind.as_str()
            );
            outbound.refresher.refresh().await;

            let retry = PublishEventRequest { event: Some(event) };
            client
                .publish_event(deadlined(retry, request_timeout))
                .await
        }
        Err(status) => Err(status),
    };

    match response {
        Ok(resp) => {
            if let Some(r) = resp.into_inner().result {
                if r.success {
                    info!(
                        "{} event published successfully",
                        params.event_kind.as_str()
                    );
                } else {
                    error!(
                        "{} event publication failed: {}",
                        params.event_kind.as_str(),
                        r.failure_reason
                    );
                }
            }
        }
        Err(e) => {
            error!(
                "{} event gRPC error: {} ({})",
                params.event_kind.as_str(),
                e.message(),
                e.code()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{AssociationResource, AssociationSpec, AssociationTargetKind};

    fn extract_nested_string(event: &Event, object_key: &str, field_key: &str) -> String {
        let msg = event.message.as_ref().unwrap();
        let obj = msg
            .fields
            .get(object_key)
            .unwrap_or_else(|| panic!("missing object: {}", object_key));
        match &obj.kind {
            Some(prost_types::value::Kind::StructValue(s)) => s
                .fields
                .get(field_key)
                .and_then(|v| match &v.kind {
                    Some(prost_types::value::Kind::StringValue(s)) => Some(s.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("missing field: {}.{}", object_key, field_key)),
            _ => panic!("{} is not a struct", object_key),
        }
    }

    fn extract_string_field(event: &Event, field_key: &str) -> String {
        let msg = event.message.as_ref().unwrap();
        let val = msg
            .fields
            .get(field_key)
            .unwrap_or_else(|| panic!("missing field: {}", field_key));
        match &val.kind {
            Some(prost_types::value::Kind::StringValue(s)) => s.clone(),
            _ => panic!("{} is not a string", field_key),
        }
    }

    #[test]
    fn test_associated_event_contains_all_fields() {
        let assoc_target_identity =
            "https://storage.example.com/container/files/image.png?etag=abc123&last_modified=2026";
        let assoc_target_address = "https://storage.example.com/container/files/image.png";
        let thumb_identity =
            "https://storage.example.com/container/thumbs/image_256x256.png?etag=def456";
        let thumb_address = "https://storage.example.com/container/thumbs/image_256x256.png";

        let params = AssociationEventParams {
            event_kind: AssociationEventKind::Associated,
            message: AssociationMessageParams {
                thumbnail: AssociationResource {
                    resource_address: thumb_address.to_string(),
                    resource_identity: Some(thumb_identity.to_string()),
                },
                association_target: AssociationResource {
                    resource_address: assoc_target_address.to_string(),
                    resource_identity: Some(assoc_target_identity.to_string()),
                },
                target_kind: AssociationTargetKind::File,
                specification: None,
            },
        };
        let event = build_association_event(&params).expect("valid associated event");

        assert_eq!(event.event_type, "omni.thumbnail.associated");
        assert_eq!(
            extract_nested_string(&event, "association_target", "resource_identity"),
            assoc_target_identity
        );
        assert_eq!(
            extract_nested_string(&event, "association_target", "resource_address"),
            assoc_target_address
        );
        assert_eq!(
            extract_nested_string(&event, "thumbnail", "resource_identity"),
            thumb_identity
        );
        assert_eq!(
            extract_nested_string(&event, "thumbnail", "resource_address"),
            thumb_address
        );
        assert_eq!(extract_string_field(&event, "target_type"), "FILE");
        assert_eq!(
            event.resource.as_ref().unwrap().resource_id,
            assoc_target_address
        );
    }

    #[test]
    fn test_disassociated_event_contains_all_fields() {
        let assoc_target_address =
            "https://examplestorage.blob.core.windows.net/example/projects/test.usd";
        let thumb_identity = "https://examplestorage.blob.core.windows.net/example/thumbs/test_256x256.webp?etag=thumb1";
        let thumb_address =
            "https://examplestorage.blob.core.windows.net/example/thumbs/test_256x256.webp";

        let params = AssociationEventParams {
            event_kind: AssociationEventKind::Disassociated,
            message: AssociationMessageParams {
                thumbnail: AssociationResource {
                    resource_address: thumb_address.to_string(),
                    resource_identity: Some(thumb_identity.to_string()),
                },
                // Folder targets carry no resource_identity — None causes the field
                // to be omitted from the event message entirely (ADR 0002 §3).
                association_target: AssociationResource {
                    resource_address: assoc_target_address.to_string(),
                    resource_identity: None,
                },
                target_kind: AssociationTargetKind::Folder,
                specification: None,
            },
        };
        let event = build_association_event(&params).expect("valid disassociated event");

        assert_eq!(event.event_type, "omni.thumbnail.disassociated");
        // Folders have no identity — the field must be omitted entirely (ADR 0002 §3).
        let assoc_target_struct = match &event
            .message
            .as_ref()
            .unwrap()
            .fields
            .get("association_target")
            .unwrap()
            .kind
        {
            Some(prost_types::value::Kind::StructValue(s)) => s,
            _ => panic!("association_target is not a struct"),
        };
        assert!(!assoc_target_struct.fields.contains_key("resource_identity"));
        assert_eq!(
            extract_nested_string(&event, "association_target", "resource_address"),
            assoc_target_address
        );
        assert_eq!(
            extract_nested_string(&event, "thumbnail", "resource_identity"),
            thumb_identity
        );
        assert_eq!(
            extract_nested_string(&event, "thumbnail", "resource_address"),
            thumb_address
        );
        assert_eq!(extract_string_field(&event, "target_type"), "FOLDER");
        assert_eq!(event.resource.as_ref().unwrap().resource_id, thumb_identity);
    }

    #[test]
    fn test_event_with_empty_addresses() {
        let assoc_target_identity =
            "https://storage.example.com/path/file.jpg?etag=e1&last_modified=123";
        let thumb_identity = "https://storage.example.com/thumbs/file_thumb.jpg?etag=t1";

        let params = AssociationEventParams {
            event_kind: AssociationEventKind::Associated,
            message: AssociationMessageParams {
                thumbnail: AssociationResource {
                    resource_address: String::new(),
                    resource_identity: Some(thumb_identity.to_string()),
                },
                association_target: AssociationResource {
                    resource_address: String::new(),
                    resource_identity: Some(assoc_target_identity.to_string()),
                },
                target_kind: AssociationTargetKind::File,
                specification: None,
            },
        };
        let event = build_association_event(&params).expect("valid event");

        assert_eq!(
            extract_nested_string(&event, "association_target", "resource_identity"),
            assoc_target_identity
        );
        assert_eq!(
            extract_nested_string(&event, "association_target", "resource_address"),
            ""
        );
        assert_eq!(
            extract_nested_string(&event, "thumbnail", "resource_identity"),
            thumb_identity
        );
        assert_eq!(
            extract_nested_string(&event, "thumbnail", "resource_address"),
            ""
        );
        assert_eq!(extract_string_field(&event, "target_type"), "FILE");
    }

    #[test]
    fn event_payload_is_unchanged_across_kinds_and_targets() {
        // The three fields the typed boundary now derives instead of round-tripping
        // through strings: the event type, the target type in the message, and which
        // resource the event is filed under.
        let assoc_target_identity = "https://storage.example.com/container/image.png?etag=abc";
        let assoc_target_address = "https://storage.example.com/container/image.png";
        let thumb_identity = "https://storage.example.com/container/thumb.png?etag=def";
        let thumb_address = "https://storage.example.com/container/thumb.png";

        for (event_kind, expected_event_type, expected_resource_id) in [
            (
                AssociationEventKind::Associated,
                "omni.thumbnail.associated",
                assoc_target_address,
            ),
            (
                AssociationEventKind::Disassociated,
                "omni.thumbnail.disassociated",
                thumb_identity,
            ),
        ] {
            for (target_type, expected_target_type) in [
                (AssociationTargetKind::File, "FILE"),
                (AssociationTargetKind::Folder, "FOLDER"),
            ] {
                let params = AssociationEventParams {
                    event_kind,
                    message: AssociationMessageParams {
                        thumbnail: AssociationResource {
                            resource_address: thumb_address.to_string(),
                            resource_identity: Some(thumb_identity.to_string()),
                        },
                        association_target: AssociationResource {
                            resource_address: assoc_target_address.to_string(),
                            // Folder targets have no identity; File targets do.
                            resource_identity: matches!(target_type, AssociationTargetKind::File)
                                .then(|| assoc_target_identity.to_string()),
                        },
                        target_kind: target_type,
                        specification: None,
                    },
                };
                let event = build_association_event(&params).expect("valid event");

                assert_eq!(event.event_type, expected_event_type);
                assert_eq!(
                    extract_string_field(&event, "target_type"),
                    expected_target_type
                );
                assert_eq!(
                    event.resource.as_ref().unwrap().resource_id,
                    expected_resource_id
                );
            }
        }
    }

    #[test]
    fn test_event_with_specification() {
        let assoc_target_identity = "https://storage.example.com/container/image.png?etag=abc";
        let assoc_target_address = "https://storage.example.com/container/image.png";
        let thumb_identity = "https://storage.example.com/container/thumb.webp?etag=def";
        let thumb_address = "https://storage.example.com/container/thumb.webp";

        let spec = AssociationSpec::ImageToImage {
            width: 256,
            height: 256,
            encoding_format: crate::spec_enums::EncodingFormat::Webp,
            transform_type: crate::spec_enums::TransformType::Square,
            crop_strategy: crate::spec_enums::CropStrategy::Center,
            quality: Some(85.0),
            preserve_aspect_ratio: Some(true),
        };

        let params = AssociationEventParams {
            event_kind: AssociationEventKind::Associated,
            message: AssociationMessageParams {
                thumbnail: AssociationResource {
                    resource_address: thumb_address.to_string(),
                    resource_identity: Some(thumb_identity.to_string()),
                },
                association_target: AssociationResource {
                    resource_address: assoc_target_address.to_string(),
                    resource_identity: Some(assoc_target_identity.to_string()),
                },
                target_kind: AssociationTargetKind::File,
                specification: Some(&spec),
            },
        };
        let event = build_association_event(&params).expect("valid event with specification");

        assert_eq!(
            extract_nested_string(&event, "association_target", "resource_identity"),
            assoc_target_identity
        );
        assert_eq!(
            extract_nested_string(&event, "association_target", "resource_address"),
            assoc_target_address
        );
        assert_eq!(
            extract_nested_string(&event, "thumbnail", "resource_identity"),
            thumb_identity
        );
        assert_eq!(
            extract_nested_string(&event, "thumbnail", "resource_address"),
            thumb_address
        );
        assert_eq!(extract_string_field(&event, "target_type"), "FILE");

        let msg = event.message.as_ref().unwrap();
        assert!(
            msg.fields.contains_key("specification"),
            "event should contain specification field"
        );
    }
}
