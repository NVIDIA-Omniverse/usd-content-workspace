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

//! Delegating association to thumbnail-service over gRPC.
//!
//! The worker never writes association metadata itself. That rule is not about
//! convenience: association semantics change, and if both binaries linked the
//! write logic every such change would need a lock-step deploy of the service
//! and the worker together. An RPC keeps their release cadences independent, and
//! only the wire contract has to stay compatible.
//!
//! Concretely, this module calls `AssociateThumbnail` and nothing else. It does
//! not touch `MetadataService`, and it does not publish
//! `omni.thumbnail.created` — the service emits association events after its own
//! write succeeds, which is what keeps one association from producing two.

use std::time::Duration;

use thumbnail_common::work_events::{GenerateWorkMessage, ResolvedGenerateSpec};
use thumbnail_common::AssociationTargetKind;
use thumbnail_proto::nvidia::omniverse::thumbnail::v1alpha as proto;
use thumbnail_proto::nvidia::omniverse::thumbnail::v1alpha::{
    thumbnail_service_client::ThumbnailServiceClient, AssociateThumbnailRequest,
    ImageToImageSpecification, ProcessorIdentity as ProtoProcessorIdentity, Resource, TargetType,
    ThumbnailSpecification,
};
use tonic::transport::Channel;
use tonic::Code;
use tracing::debug;

use thumbnail_common::work_events::is_retryable_tonic_code;

use proto::AssociationFailureKind;

/// Why an association was not recorded.
#[derive(Debug, thiserror::Error)]
pub enum AssociateError {
    #[error("associate call failed: {0}")]
    Transport(tonic::Status),

    /// The RPC succeeded but the service reported the association as not made.
    /// Reported in-band by design — see the proto's failure model.
    #[error("thumbnail-service rejected the association: {reason}")]
    Rejected {
        reason: String,
        /// What the service said about retrying, if it said anything.
        ///
        /// `None` covers both an older service that does not send the field and
        /// one that sends `UNSPECIFIED`, because they mean the same thing here:
        /// unclassified.
        kind: Option<AssociationFailureKind>,
    },
}

impl AssociateError {
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Transport(status) => is_retryable_tonic_code(status.code()),
            // Classified by the service where it can be. `error_message` is prose,
            // so this side used to have no way to tell "storage blinked" from
            // "this will never succeed" and withheld the acknowledgement for both.
            // That is safe for the transient case and expensive for the permanent
            // one: the delivery comes back and re-downloads, re-converts and
            // re-uploads the same source, repeating the whole pipeline until the
            // platform's delivery limit dead-letters it.
            //
            // An unclassified rejection stays retryable. That is the previous
            // behavior, and it is the right default in the direction that matters:
            // withholding costs a duplicate association, which is idempotent,
            // while acknowledging costs the association entirely.
            Self::Rejected { kind, .. } => match kind {
                Some(AssociationFailureKind::Permanent) => false,
                Some(AssociationFailureKind::Retryable)
                | Some(AssociationFailureKind::Unspecified)
                | None => true,
            },
        }
    }
}

/// Associate an uploaded thumbnail with its source.
///
/// `thumbnail_identity` is the identity storage returned from the write, never
/// one this worker constructed: associating an identity nobody confirmed would
/// record a thumbnail that may not exist.
pub async fn associate_thumbnail(
    channel: Channel,
    interceptor: thumbnail_auth::AuthInterceptor,
    refresher: &std::sync::Arc<dyn thumbnail_auth::TokenRefresher>,
    message: &GenerateWorkMessage,
    thumbnail_identity: &str,
    timeout: Duration,
) -> Result<(), AssociateError> {
    let request = AssociateThumbnailRequest {
        thumbnail: Some(Resource {
            resource_address: message.destination_resource_address.clone(),
            resource_identity: Some(thumbnail_identity.to_string()),
        }),
        association_target: Some(Resource {
            resource_address: message.source.resource_address.clone(),
            resource_identity: message.source.resource_identity.clone(),
        }),
        target_type: Some(target_type(message.target_type) as i32),
        specification: Some(ThumbnailSpecification {
            specification: Some(proto::thumbnail_specification::Specification::ImageToImage(
                specification(&message.specification),
            )),
        }),
    };

    // Traced so the association lands in the trace of the delivery that produced
    // it: this is the hop back into `thumbnail-service`, and untraced it appears
    // there as an RPC from nowhere.
    let mut client = ThumbnailServiceClient::new(thumbnail_common::outbound::trace_authed(
        channel,
        interceptor,
    ));

    let response = match issue(&mut client, &request, timeout).await {
        Ok(response) => response,
        // One refresh and one reissue are part of the same attempt, matching
        // every other outbound call site. This is not an application retry: the
        // convert/upload work is never repeated.
        Err(status) if status.code() == Code::Unauthenticated => {
            refresher.refresh().await;
            issue(&mut client, &request, timeout)
                .await
                .map_err(AssociateError::Transport)?
        }
        Err(status) => return Err(AssociateError::Transport(status)),
    };

    if response.success {
        debug!(
            destination = %message.destination_resource_address,
            "Association recorded by thumbnail-service"
        );
        return Ok(());
    }

    // `failure_kind` is an open enum on the wire: an unrecognized number from a
    // newer service must not silently become `Unspecified`-and-therefore-retryable
    // by accident, so it is mapped explicitly and lands on retryable by the same
    // conservative default.
    let kind = response
        .failure_kind
        .map(AssociationFailureKind::try_from)
        .transpose()
        .unwrap_or(None);

    Err(AssociateError::Rejected {
        reason: response
            .error_message
            .unwrap_or_else(|| "no error message supplied".to_string()),
        kind,
    })
}

async fn issue(
    client: &mut ThumbnailServiceClient<thumbnail_common::outbound::TracedAuthedChannel>,
    request: &AssociateThumbnailRequest,
    timeout: Duration,
) -> Result<proto::AssociateThumbnailResponse, tonic::Status> {
    let mut call = tonic::Request::new(request.clone());
    call.set_timeout(timeout);
    client
        .associate_thumbnail(call)
        .await
        .map(|r| r.into_inner())
}

fn target_type(kind: AssociationTargetKind) -> TargetType {
    match kind {
        AssociationTargetKind::File => TargetType::File,
        AssociationTargetKind::Folder => TargetType::Folder,
    }
}

/// Carry the resolved specification onto the associate request unchanged.
///
/// The enum conversions come from `thumbnail-common`'s shared types, so the
/// worker cannot invent an alternate spelling: width, height, encoding, transform,
/// crop, quality, and preserve-aspect-ratio round-trip from the generate message
/// into the association exactly as the publisher resolved them.
fn specification(spec: &ResolvedGenerateSpec) -> ImageToImageSpecification {
    ImageToImageSpecification {
        // Task-worker always produces IMAGE_TO_IMAGE; MANUAL specs never enter the queue.
        processor_identity: ProtoProcessorIdentity::ImageToImage as i32,
        encoding_format: proto::ImageEncodingFormat::from(spec.encoding_format) as i32,
        width: spec.width,
        height: spec.height,
        transform_type: proto::TransformType::from(spec.transform_type) as i32,
        crop_strategy: proto::CropStrategy::from(spec.crop_strategy) as i32,
        quality: Some(spec.quality),
        preserve_aspect_ratio: Some(spec.preserve_aspect_ratio),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use thumbnail_common::spec_enums::{CropStrategy, EncodingFormat, TransformType};

    fn resolved() -> ResolvedGenerateSpec {
        ResolvedGenerateSpec {
            width: 256,
            height: 128,
            encoding_format: EncodingFormat::Webp,
            transform_type: TransformType::ScaleAndCrop,
            crop_strategy: CropStrategy::BottomRight,
            quality: 73,
            preserve_aspect_ratio: false,
        }
    }

    /// Every specification field must survive the hop from the generate message
    /// into the association, or the recorded thumbnail describes something other
    /// than what was produced.
    #[test]
    fn the_specification_round_trips_into_the_associate_request() {
        let spec = specification(&resolved());

        assert_eq!(spec.width, 256);
        assert_eq!(spec.height, 128);
        assert_eq!(spec.quality, Some(73));
        assert_eq!(spec.preserve_aspect_ratio, Some(false));
        assert_eq!(
            spec.encoding_format,
            proto::ImageEncodingFormat::Webp as i32
        );
        assert_eq!(
            spec.transform_type,
            proto::TransformType::ScaleAndCrop as i32
        );
        assert_eq!(spec.crop_strategy, proto::CropStrategy::BottomRight as i32);
        assert_eq!(
            spec.processor_identity,
            ProtoProcessorIdentity::ImageToImage as i32,
            "task-worker always associates as IMAGE_TO_IMAGE"
        );
    }

    /// `preserve_aspect_ratio: false` is a value, not an absence. Sending it as
    /// an omitted field would have the service substitute its own default.
    #[test]
    fn an_explicit_false_is_sent_rather_than_omitted() {
        let mut source = resolved();
        source.preserve_aspect_ratio = false;
        assert_eq!(specification(&source).preserve_aspect_ratio, Some(false));

        source.preserve_aspect_ratio = true;
        assert_eq!(specification(&source).preserve_aspect_ratio, Some(true));
    }

    #[test]
    fn target_types_map_to_their_proto_values() {
        assert_eq!(target_type(AssociationTargetKind::File), TargetType::File);
        assert_eq!(
            target_type(AssociationTargetKind::Folder),
            TargetType::Folder
        );
    }

    #[test]
    fn transport_failures_classify_like_every_other_outbound_call() {
        assert!(
            AssociateError::Transport(tonic::Status::unavailable("down")).is_retryable(),
            "an unavailable service should be retried"
        );
        assert!(
            !AssociateError::Transport(tonic::Status::invalid_argument("bad")).is_retryable(),
            "a rejected request reproduces on redelivery"
        );
    }

    fn rejected(kind: Option<AssociationFailureKind>) -> AssociateError {
        AssociateError::Rejected {
            reason: "storage write failed".to_string(),
            kind,
        }
    }

    /// An unclassified rejection stays retryable, which is the pre-classification
    /// behavior. It is the safe direction: withholding costs a duplicate
    /// association, which is idempotent, while acknowledging costs the association
    /// entirely.
    #[test]
    fn an_unclassified_in_band_rejection_is_retried_rather_than_discarded() {
        assert!(
            rejected(None).is_retryable(),
            "an older service sends no kind"
        );
        assert!(
            rejected(Some(AssociationFailureKind::Unspecified)).is_retryable(),
            "an explicit UNSPECIFIED means the same as absent"
        );
    }

    /// The point of the classification: a rejection that cannot succeed on retry
    /// is acknowledged, instead of re-downloading, re-converting and re-uploading
    /// the same source on every redelivery until the platform dead-letters it.
    #[test]
    fn a_permanent_in_band_rejection_is_not_retried() {
        assert!(!rejected(Some(AssociationFailureKind::Permanent)).is_retryable());
    }

    #[test]
    fn a_retryable_in_band_rejection_is_retried() {
        assert!(rejected(Some(AssociationFailureKind::Retryable)).is_retryable());
    }
}
