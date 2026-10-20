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

//! Validate, convert, upload, associate — and return what happened.
//!
//! The pipeline never decides an acknowledgement. It returns a [`WorkOutcome`]
//! and the consumer maps that onto the ack stream, which is what lets the same
//! classification drive a metric label today and an HTTP status code when the
//! synchronous mode lands.
//!
//! It also makes exactly one attempt. There is no in-process retry loop and no
//! Celery-style `max_retries` countdown: a retryable failure returns immediately
//! and the delivery comes back through the redelivery mechanism in `consumer.rs`.
//! Retrying here would hold a conversion slot while the dependency is down,
//! which is the opposite of what a bounded worker should do under stress.

use std::sync::Arc;
use std::time::Instant;

use prost_types::Struct;
use thumbnail_common::spec_enums::{CropStrategy, EncodingFormat, TransformType};
use thumbnail_common::storage::{read_object, write_object, StorageIoError};
use thumbnail_common::work_events::{
    canonicalize_address, is_absolute_address, set_remote_parent, GenerateMessageRejection,
    GenerateWorkMessage, ResolvedGenerateSpec,
};
use thumbnail_common::OutboundClients;
use thumbnail_convert::{
    ConversionError, ConversionFailureClass, CropStrategy as ConvertCropStrategy, ImageSpec,
    OutputFormat, Transform,
};
use tracing::{debug, error, info, Instrument as _};

use crate::associate::associate_thumbnail;
use crate::config::WorkerArgs;
use crate::outcome::{FailureClass, WorkOutcome};

/// What the consumer needs from the pipeline.
///
/// A trait so the consume loop — the acknowledgement matrix, the in-flight
/// bound, the recycle — can be tested against a scripted processor rather than
/// against real storage and a real conversion. The consumer's job is delivery
/// bookkeeping, and that is worth testing without dragging image decoding into
/// it.
#[async_trait::async_trait]
pub trait MessageProcessor: Send + Sync {
    async fn process(&self, message: &Struct) -> WorkOutcome;
}

/// Everything one conversion needs, built once at startup.
pub struct Pipeline {
    pub args: WorkerArgs,
    pub outbound: Arc<OutboundClients>,
    pub thumbnail_service: tonic::transport::Channel,
    /// Shared with the consume loop, which records terminal outcomes.
    ///
    /// The pipeline needs it for one thing the outcome cannot express: whether a
    /// failure left an uploaded blob behind. `WorkOutcome` carries a class, and
    /// the `associate` class is the same either side of the upload.
    pub metrics: Arc<crate::metrics::WorkerMetrics>,
}

#[async_trait::async_trait]
impl MessageProcessor for Pipeline {
    async fn process(&self, message: &Struct) -> WorkOutcome {
        // Spelled as a path call, not `self.process(..)`. Both resolve to the
        // inherent method — inherent wins over trait in method resolution — but
        // the receiver form turns into unbounded recursion the moment anyone
        // renames or removes `Pipeline::process`, and no test would catch it:
        // the unit tests call the inherent method directly and the consumer
        // tests drive this trait through a scripted double, so this body is
        // only ever executed in production.
        Pipeline::process(self, message).await
    }
}

impl Pipeline {
    /// Process one generate message.
    ///
    /// Validation runs before anything heavy: a message that cannot produce a
    /// thumbnail should not cost a storage read, let alone a decode.
    ///
    /// Each delivery emits a `thumbnail.generate` span with child spans for
    /// download, convert, upload, and associate. The span is opened after parsing
    /// so destination and source are available as span fields, and it continues
    /// the publisher's trace when the message carries one — so the conversion,
    /// which is where nearly all the elapsed time goes, lands in the trace of the
    /// request or storage event that asked for it. Without a usable context it
    /// roots its own trace, exactly as it did before.
    pub async fn process(&self, message: &Struct) -> WorkOutcome {
        let started = Instant::now();

        let message = match GenerateWorkMessage::from_struct(message) {
            Ok(message) => message,
            Err(rejection) => return rejection_outcome(rejection),
        };
        let destination = message.destination_resource_address.clone();

        let span = tracing::info_span!(
            "thumbnail.generate",
            destination = %destination,
            source = %message.source.resource_address,
            // Filled in at the end so Tempo shows ok/error on the root span.
            "otel.status_code" = tracing::field::Empty,
            "otel.status_message" = tracing::field::Empty,
        );
        set_remote_parent(
            &span,
            message.traceparent.as_deref(),
            message.tracestate.as_deref(),
        );
        self.run(&message, destination, started)
            .instrument(span)
            .await
    }

    async fn run(
        &self,
        message: &GenerateWorkMessage,
        destination: String,
        started: Instant,
    ) -> WorkOutcome {
        if let Err(outcome) = self.validate(message) {
            tracing::Span::current()
                .record("otel.status_code", "ERROR")
                .record("otel.status_message", "validation failed");
            return outcome;
        }

        // Its own class: a credential failure says nothing about this message,
        // and every conversion fails the same way while it lasts. Counted as
        // `decode` it would look like a sudden epidemic of corrupt images.
        let auth_token = match self.outbound.get_auth_token() {
            Ok(token) => token,
            Err(error) => {
                tracing::Span::current()
                    .record("otel.status_code", "ERROR")
                    .record("otel.status_message", "credential error");
                return classified(
                    error.is_retryable(),
                    FailureClass::Credential,
                    destination,
                    error,
                );
            }
        };

        let source_identity = message
            .source
            .resource_identity
            .as_deref()
            .unwrap_or_default();

        let source_bytes = match read_object(
            &self.outbound,
            source_identity,
            &auth_token,
            self.args.max_source_bytes,
            self.args.outbound_request_timeout,
        )
        .instrument(tracing::info_span!(
            "thumbnail.download",
            source = %message.source.resource_address,
            source_identity = %source_identity,
        ))
        .await
        {
            Ok(bytes) => bytes,
            Err(error) => {
                let class = match error {
                    // Usually a deleted object racing its own CREATE event, and
                    // a different problem from storage being unreachable.
                    StorageIoError::NotFound => FailureClass::SourceMissing,
                    // Caught during the transfer only because that is the
                    // cheapest place to notice it. It is the same bound the
                    // conversion library enforces on bytes it is handed, so it
                    // carries the same label wherever it happens to fire —
                    // otherwise one condition reports as two.
                    StorageIoError::SourceTooLarge { .. } => FailureClass::Decode,
                    // Nothing to do with storage: the token could not be turned
                    // into a header at all.
                    StorageIoError::AuthHeaders(_) => FailureClass::Credential,
                    // `MissingWriteIdentity` cannot arise on a read; it is here
                    // to keep the match exhaustive, so a new variant has to be
                    // classified rather than defaulting into this bucket.
                    StorageIoError::UnsupportedRedirect
                    | StorageIoError::MissingWriteIdentity
                    | StorageIoError::Rpc { .. } => FailureClass::Download,
                };
                tracing::Span::current()
                    .record("otel.status_code", "ERROR")
                    .record("otel.status_message", class.as_label());
                return classified(error.is_retryable(), class, destination, error);
            }
        };

        // Conversion is CPU-bound and would otherwise stall this task's whole
        // runtime thread while other conversions wait for their turn on it.
        let spec = convert_spec(&message.specification);
        debug!(
            destination = %destination,
            source = %message.source.resource_address,
            transform = ?spec.transform,
            crop_strategy = ?spec.crop,
            target_width = spec.width,
            target_height = spec.height,
            preserve_aspect_ratio = spec.preserve_aspect_ratio,
            "Resolved thumbnail conversion specification"
        );
        if spec.crop == ConvertCropStrategy::Smart
            && !matches!(spec.transform, Transform::Crop | Transform::ScaleAndCrop)
        {
            debug!(
                destination = %destination,
                transform = ?spec.transform,
                crop_strategy = ?spec.crop,
                "Smart crop strategy does not apply to selected transform; smartcrop2 will not run"
            );
        }
        let limits = self.args.decode_limits();
        let convert_span = tracing::info_span!(
            "thumbnail.convert",
            width = spec.width,
            height = spec.height,
            format = ?spec.format,
            transform = ?spec.transform,
        );
        let converted = match tokio::task::spawn_blocking(move || {
            let _guard = convert_span.entered();
            thumbnail_convert::convert(source_bytes, &spec, &limits)
        })
        .await
        {
            Ok(Ok(bytes)) => bytes,
            Ok(Err(error)) => {
                tracing::Span::current()
                    .record("otel.status_code", "ERROR")
                    .record("otel.status_message", "conversion failed");
                return conversion_outcome(error, destination);
            }
            // A panicked decode is a bug, not a property of the input, so it
            // is worth one more delivery on a healthy replica.
            Err(join_error) => {
                tracing::Span::current()
                    .record("otel.status_code", "ERROR")
                    .record("otel.status_message", "conversion panicked");
                return WorkOutcome::retryable(FailureClass::Decode, destination, join_error);
            }
        };

        debug!(
            destination = %destination,
            bytes = converted.len(),
            "Converted thumbnail"
        );

        let thumbnail_identity = match write_object(
            &self.outbound,
            &destination,
            converted,
            &auth_token,
            self.args.outbound_request_timeout,
        )
        .instrument(tracing::info_span!(
            "thumbnail.upload",
            destination = %destination,
        ))
        .await
        {
            Ok(identity) => identity,
            Err(error) => {
                let class = match error {
                    StorageIoError::AuthHeaders(_) => FailureClass::Credential,
                    _ => FailureClass::Upload,
                };
                tracing::Span::current()
                    .record("otel.status_code", "ERROR")
                    .record("otel.status_message", class.as_label());
                return classified(error.is_retryable(), class, destination, error);
            }
        };

        if let Err(error) = associate_thumbnail(
            self.thumbnail_service.clone(),
            self.outbound.interceptor.clone(),
            &self.outbound.refresher,
            message,
            &thumbnail_identity,
            self.args.outbound_request_timeout,
        )
        .instrument(tracing::info_span!(
            "thumbnail.associate",
            destination = %destination,
            thumbnail_identity = %thumbnail_identity,
        ))
        .await
        {
            tracing::Span::current()
                .record("otel.status_code", "ERROR")
                .record("otel.status_message", "associate failed");

            // The upload has already happened by this point. A retryable failure
            // withholds the acknowledgement, so a redelivery will overwrite the
            // same destination and eventually associate it — nothing is stranded.
            // A permanent one acknowledges, so nothing will ever retry, and the
            // blob sits at the destination with no metadata pointing at it.
            //
            // The storage-deleted sweep cannot reclaim it: that sweep enumerates
            // `thumbnail_*` metadata on the *source*, and this blob has no such
            // entry. So it is unreferenced and unenumerable, which is the
            // combination that makes it accumulate silently.
            //
            // Recorded rather than deleted, deliberately. Deleting would mean
            // acting on the assumption that the association definitely did not
            // happen, and a permanent transport error cannot tell "the service
            // rejected this" from "the service committed it and the response was
            // lost". In the second case a delete would break a live association —
            // strictly worse than an orphan, which the next regeneration of the
            // same destination overwrites anyway.
            if !error.is_retryable() {
                self.metrics.blob_orphaned();
                error!(
                    destination = %destination,
                    thumbnail_identity = %thumbnail_identity,
                    %error,
                    "Uploaded thumbnail is orphaned: associate failed permanently, so the \
                     delivery is acknowledged and nothing will retry. The blob is not \
                     referenced by any association metadata and the storage-deleted sweep \
                     cannot find it; reconcile from this address."
                );
            }

            return classified(
                error.is_retryable(),
                FailureClass::Associate,
                destination,
                error,
            );
        }

        info!(
            destination = %destination,
            source = %message.source.resource_address,
            elapsed_ms = started.elapsed().as_millis(),
            "Thumbnail generated and associated"
        );
        tracing::Span::current().record("otel.status_code", "OK");
        WorkOutcome::Succeeded { destination }
    }

    /// Checks that are cheap and can only fail permanently, run before the
    /// source is fetched.
    /// Checks about *this worker's* ability to do the work, as distinct from the
    /// contract checks `GenerateWorkMessage::from_struct` has already applied —
    /// the processor gate, dimension bounds, and quality range among them.
    fn validate(&self, message: &GenerateWorkMessage) -> Result<(), WorkOutcome> {
        let destination = message.destination_resource_address.as_str();

        if message
            .source
            .resource_identity
            .as_deref()
            .is_none_or(|identity| identity.trim().is_empty())
        {
            return Err(WorkOutcome::permanent(
                FailureClass::Validation,
                destination,
                "source resource_identity is required to read the source object",
            ));
        }

        if destination.trim().is_empty() {
            return Err(WorkOutcome::permanent(
                FailureClass::Validation,
                destination,
                "destination_resource_address is required",
            ));
        }
        if !is_absolute_address(destination) {
            return Err(WorkOutcome::permanent(
                FailureClass::Validation,
                destination,
                "destination_resource_address must be an absolute storage address",
            ));
        }

        // Normalized on both sides: a source spelled with an explicit default
        // port or a mixed-case host names the same object as its canonical form,
        // and a raw string comparison would let the thumbnail overwrite its own
        // source. The publisher checks this too; the worker is the last line
        // before the write.
        if canonicalize_address(destination)
            == canonicalize_address(&message.source.resource_address)
        {
            return Err(WorkOutcome::permanent(
                FailureClass::Validation,
                destination,
                "destination_resource_address resolves to the source address",
            ));
        }

        Ok(())
    }
}

fn classified(
    retryable: bool,
    class: FailureClass,
    destination: String,
    detail: impl std::fmt::Display,
) -> WorkOutcome {
    if retryable {
        WorkOutcome::retryable(class, destination, detail)
    } else {
        WorkOutcome::permanent(class, destination, detail)
    }
}

/// A message that would not parse or would not satisfy the contract. Only a
/// *newer* schema version is preserved; everything else is bad input no
/// redelivery improves.
fn rejection_outcome(rejection: GenerateMessageRejection) -> WorkOutcome {
    let withhold = rejection.withholds_ack();
    // All generate-queue messages are IMAGE_TO_IMAGE; MANUAL specs never enter
    // the queue, so there is no separate UnsupportedProcessor class here.
    let class = FailureClass::Validation;
    // The destination is unknown for a message that did not parse, which is
    // itself worth saying in the log rather than leaving blank.
    let destination = "<unparsed generate message>".to_string();
    classified(withhold, class, destination, rejection)
}

fn conversion_outcome(error: ConversionError, destination: String) -> WorkOutcome {
    // Every conversion failure is a property of the bytes or the specification,
    // so all of them are permanent. Only the label differs — and it maps one to
    // one, because an encode failure means the requested output format could not
    // be written, which is a different thing to investigate than bytes that
    // would not decode.
    let class = match error.failure_class() {
        ConversionFailureClass::Specification => FailureClass::Validation,
        ConversionFailureClass::UnsupportedFormat => FailureClass::UnsupportedFormat,
        ConversionFailureClass::Bounds | ConversionFailureClass::Decode => FailureClass::Decode,
        ConversionFailureClass::Encode => FailureClass::Encode,
    };
    WorkOutcome::permanent(class, destination, error)
}

/// Map the contract's specification onto the conversion crate's own.
///
/// Exhaustive by construction: a new variant in `thumbnail-common` breaks this
/// build rather than silently falling through to a default, which is the whole
/// reason the conversion crate declares its own enums instead of sharing these.
fn convert_spec(spec: &ResolvedGenerateSpec) -> ImageSpec {
    ImageSpec {
        width: spec.width,
        height: spec.height,
        format: match spec.encoding_format {
            EncodingFormat::Png => OutputFormat::Png,
            EncodingFormat::Jpeg => OutputFormat::Jpeg,
            EncodingFormat::Gif => OutputFormat::Gif,
            EncodingFormat::Webp => OutputFormat::Webp,
        },
        transform: match spec.transform_type {
            TransformType::Square => Transform::Square,
            TransformType::ScaleByWidth => Transform::ScaleByWidth,
            TransformType::ScaleByHeight => Transform::ScaleByHeight,
            TransformType::Crop => Transform::Crop,
            TransformType::Fit => Transform::Fit,
            TransformType::ScaleAndCrop => Transform::ScaleAndCrop,
            TransformType::Thumbnail => Transform::Thumbnail,
            TransformType::Resize => Transform::Resize,
        },
        crop: match spec.crop_strategy {
            CropStrategy::Center => ConvertCropStrategy::Center,
            CropStrategy::Top => ConvertCropStrategy::Top,
            CropStrategy::Bottom => ConvertCropStrategy::Bottom,
            CropStrategy::Left => ConvertCropStrategy::Left,
            CropStrategy::Right => ConvertCropStrategy::Right,
            CropStrategy::TopLeft => ConvertCropStrategy::TopLeft,
            CropStrategy::TopRight => ConvertCropStrategy::TopRight,
            CropStrategy::BottomLeft => ConvertCropStrategy::BottomLeft,
            CropStrategy::BottomRight => ConvertCropStrategy::BottomRight,
            CropStrategy::Smart => ConvertCropStrategy::Smart,
        },
        quality: spec.quality,
        preserve_aspect_ratio: spec.preserve_aspect_ratio,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn spec_with(transform: TransformType, crop: CropStrategy) -> ResolvedGenerateSpec {
        ResolvedGenerateSpec {
            width: 64,
            height: 64,
            encoding_format: EncodingFormat::Png,
            transform_type: transform,
            crop_strategy: crop,
            quality: 85,
            preserve_aspect_ratio: true,
        }
    }

    /// Every contract transform and crop direction has a conversion counterpart.
    /// A gap here would be a silent substitution, which the spec forbids.
    #[test]
    fn every_transform_and_crop_strategy_maps() {
        let transforms = [
            (TransformType::Square, Transform::Square),
            (TransformType::ScaleByWidth, Transform::ScaleByWidth),
            (TransformType::ScaleByHeight, Transform::ScaleByHeight),
            (TransformType::Crop, Transform::Crop),
            (TransformType::Fit, Transform::Fit),
            (TransformType::ScaleAndCrop, Transform::ScaleAndCrop),
            (TransformType::Thumbnail, Transform::Thumbnail),
            (TransformType::Resize, Transform::Resize),
        ];
        for (contract, expected) in transforms {
            assert_eq!(
                convert_spec(&spec_with(contract, CropStrategy::Center)).transform,
                expected,
                "{contract} must map to a conversion transform"
            );
        }

        let crops = [
            (CropStrategy::Center, ConvertCropStrategy::Center),
            (CropStrategy::Top, ConvertCropStrategy::Top),
            (CropStrategy::Bottom, ConvertCropStrategy::Bottom),
            (CropStrategy::Left, ConvertCropStrategy::Left),
            (CropStrategy::Right, ConvertCropStrategy::Right),
            (CropStrategy::TopLeft, ConvertCropStrategy::TopLeft),
            (CropStrategy::TopRight, ConvertCropStrategy::TopRight),
            (CropStrategy::BottomLeft, ConvertCropStrategy::BottomLeft),
            (CropStrategy::BottomRight, ConvertCropStrategy::BottomRight),
            // The shipped chart default, and the one this loop used to skip.
            (CropStrategy::Smart, ConvertCropStrategy::Smart),
        ];
        for (contract, expected) in crops {
            assert_eq!(
                convert_spec(&spec_with(TransformType::Crop, contract)).crop,
                expected,
                "{contract} must map to a crop anchor"
            );
        }
    }

    #[test]
    fn every_output_encoding_maps() {
        for (contract, expected) in [
            (EncodingFormat::Png, OutputFormat::Png),
            (EncodingFormat::Jpeg, OutputFormat::Jpeg),
            (EncodingFormat::Gif, OutputFormat::Gif),
            (EncodingFormat::Webp, OutputFormat::Webp),
        ] {
            let mut spec = spec_with(TransformType::Resize, CropStrategy::Center);
            spec.encoding_format = contract;
            assert_eq!(convert_spec(&spec).format, expected);
        }
    }

    /// Conversion failures never withhold an acknowledgement: none of them
    /// improves on a redelivery, and a decode bomb that kept its delivery would
    /// crash-loop the replacement replica.
    #[test]
    fn every_conversion_failure_is_permanent() {
        let cases = [
            ConversionError::EmptySource,
            ConversionError::UnrecognizedSourceFormat,
            ConversionError::SourceTooLarge {
                actual: 10,
                limit: 5,
            },
            ConversionError::PixelBudgetExceeded {
                width: 10,
                height: 10,
                pixels: 100,
                limit: 10,
            },
            ConversionError::Decode("corrupt".to_string()),
            ConversionError::InvalidSpecification("zero width".to_string()),
        ];

        for error in cases {
            let outcome = conversion_outcome(error, "https://storage.example/t.png".to_string());
            assert!(
                outcome.should_ack(),
                "a conversion failure must acknowledge, got {outcome:?}"
            );
        }
    }

    #[test]
    fn an_unsupported_source_format_is_labelled_as_such() {
        let outcome = conversion_outcome(
            ConversionError::UnrecognizedSourceFormat,
            "https://storage.example/t.png".to_string(),
        );

        assert_eq!(
            outcome.failure_class(),
            Some(FailureClass::UnsupportedFormat)
        );
    }

    /// Bytes that will not decode and an output format that will not encode are
    /// different things to investigate, so they do not share a label. Folding
    /// the second into the first is what made `decode` a bucket.
    #[test]
    fn an_encode_failure_is_not_reported_as_a_decode_failure() {
        let encode = conversion_outcome(
            ConversionError::Encode {
                format: OutputFormat::Gif,
                reason: "too many colours".to_string(),
            },
            "https://storage.example/t.gif".to_string(),
        );
        let decode = conversion_outcome(
            ConversionError::Decode("truncated".to_string()),
            "https://storage.example/t.png".to_string(),
        );

        assert_eq!(encode.failure_class(), Some(FailureClass::Encode));
        assert_eq!(decode.failure_class(), Some(FailureClass::Decode));
    }

    /// A rolling deploy must not have an old replica discard work it simply does
    /// not understand yet.
    #[test]
    fn a_newer_schema_version_withholds_the_ack() {
        let outcome = rejection_outcome(GenerateMessageRejection::NewerSchemaVersion {
            received: 2,
            supported: 1,
        });

        assert!(!outcome.should_ack());
    }

    #[test]
    fn a_malformed_message_is_permanent() {
        let outcome = rejection_outcome(GenerateMessageRejection::MissingSchemaVersion);

        assert!(outcome.should_ack());
        assert_eq!(outcome.failure_class(), Some(FailureClass::Validation));
    }
}
