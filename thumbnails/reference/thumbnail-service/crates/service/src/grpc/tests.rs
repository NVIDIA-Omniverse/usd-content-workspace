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

use super::*;
use std::sync::Arc;
use thumbnail_common::{storage::association::THUMBNAIL_METADATA_KEY, OutboundClients};
use thumbnail_proto::nvidia::omniverse::thumbnail::v1alpha::{
    thumbnail_specification, AssociateThumbnailRequest, CreateThumbnailsRequest,
    DisassociateThumbnailRequest, ImageToImageSpecification, Resource, TargetType,
    ThumbnailSpecification,
};
use tonic::Request;

fn resource(address: &str, identity: Option<&str>) -> Resource {
    Resource {
        resource_address: address.to_string(),
        resource_identity: identity.map(str::to_string),
    }
}

fn image_to_image_specification() -> ThumbnailSpecification {
    ThumbnailSpecification {
        specification: Some(thumbnail_specification::Specification::ImageToImage(
            ImageToImageSpecification {
                processor_identity: 1,
                encoding_format: 2,
                width: 256,
                height: 256,
                transform_type: 1,
                crop_strategy: 1,
                quality: None,
                preserve_aspect_ratio: None,
            },
        )),
    }
}

fn associate_request_with_image_spec(
    image_spec: ImageToImageSpecification,
    target_type: Option<i32>,
) -> AssociateThumbnailRequest {
    AssociateThumbnailRequest {
        thumbnail: Some(resource("https://store.test/thumb.png", Some("thumb-id"))),
        association_target: Some(resource("https://store.test/source.usd", Some("source-id"))),
        target_type,
        specification: Some(ThumbnailSpecification {
            specification: Some(thumbnail_specification::Specification::ImageToImage(
                image_spec,
            )),
        }),
    }
}

fn test_service() -> ThumbnailServiceImpl {
    thumbnail_common::test_fixtures::init_test_runtime_config("thumbnail-service-grpc-tests");
    ThumbnailServiceImpl::new(Arc::new(OutboundClients::for_tests()))
}

/// Wrap a request with a fake bearer token so validation reaches the
/// field-level checks rather than failing on missing auth.
fn with_auth<T>(req: T) -> Request<T> {
    let mut r = Request::new(req);
    r.metadata_mut()
        .insert("authorization", "Bearer test-token".parse().unwrap());
    r
}

// ---------------------------------------------------------------------
// associate_thumbnail / disassociate_thumbnail validation — shares
// thumbnail_common::validate_resource with the HTTP transport, so these
// lock in the same file/folder + identity rules from the gRPC side.
// ---------------------------------------------------------------------

// Full parity with HTTP: a FOLDER association_target must NOT carry resource_identity.
#[tokio::test]
async fn associate_rejects_folder_target_with_identity() {
    let service = test_service();
    let req = AssociateThumbnailRequest {
        thumbnail: Some(resource("https://store.test/thumb.png", Some("thumb-id"))),
        association_target: Some(resource(
            "https://store.test/folder/",
            Some("unexpected-id"),
        )),
        target_type: Some(TargetType::Folder as i32),
        specification: Some(image_to_image_specification()),
    };

    let err = service
        .associate_thumbnail(with_auth(req))
        .await
        .expect_err("folder target carrying an identity should be rejected");
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
    assert!(
        err.message().contains("association_target"),
        "error should point at association_target: {}",
        err.message()
    );
}

#[tokio::test]
async fn associate_rejects_missing_specification() {
    let service = test_service();
    let req = AssociateThumbnailRequest {
        thumbnail: Some(resource("https://store.test/thumb.png", Some("thumb-id"))),
        association_target: Some(resource("https://store.test/source.usd", Some("source-id"))),
        target_type: Some(TargetType::File as i32),
        specification: None,
    };

    let err = service
        .associate_thumbnail(with_auth(req))
        .await
        .expect_err("missing specification should be rejected");
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
    assert_eq!(err.message(), "specification is required");
}

#[tokio::test]
async fn associate_rejects_missing_auth_as_unauthenticated() {
    let service = test_service();
    let req = AssociateThumbnailRequest {
        thumbnail: Some(resource("https://store.test/thumb.png", Some("thumb-id"))),
        association_target: Some(resource("https://store.test/source.usd", Some("source-id"))),
        target_type: Some(TargetType::File as i32),
        specification: Some(image_to_image_specification()),
    };

    let err = service
        .associate_thumbnail(Request::new(req))
        .await
        .expect_err("missing auth should be rejected");
    assert_eq!(err.code(), tonic::Code::Unauthenticated);
    assert_eq!(err.message(), "authentication token is required");
}

// A FILE association_target (the default when target_type is omitted) still requires an identity.
#[tokio::test]
async fn associate_rejects_file_target_without_identity() {
    let service = test_service();
    let req = AssociateThumbnailRequest {
        thumbnail: Some(resource("https://store.test/thumb.png", Some("thumb-id"))),
        association_target: Some(resource("https://store.test/source.usd", None)),
        target_type: None, // defaults to FILE
        specification: Some(image_to_image_specification()),
    };

    let err = service
        .associate_thumbnail(with_auth(req))
        .await
        .expect_err("file target without an identity should be rejected");
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
    assert!(err.message().contains("association_target"));
}

#[test]
fn associate_proto_defaults_omitted_target_type_to_file() {
    thumbnail_common::test_fixtures::init_test_runtime_config("thumbnail-service-grpc-tests");
    let image_spec = match image_to_image_specification().specification {
        Some(thumbnail_specification::Specification::ImageToImage(spec)) => spec,
        _ => panic!("expected image-to-image specification"),
    };
    let request = associate_request_with_image_spec(image_spec, None);

    let canonical = association_request_from_proto(&request).expect("valid proto request");

    assert_eq!(canonical.target_kind, AssociationTargetKind::File);
}

#[test]
fn associate_proto_rejects_out_of_range_dimensions_and_invalid_required_enums() {
    thumbnail_common::test_fixtures::init_test_runtime_config("thumbnail-service-grpc-tests");
    let valid_spec = || ImageToImageSpecification {
        processor_identity: 1,
        encoding_format: 2,
        width: 256,
        height: 256,
        transform_type: 1,
        crop_strategy: 1,
        quality: None,
        preserve_aspect_ratio: None,
    };

    let max = thumbnail_common::AssociationSpec::MAX_DIMENSION;
    for (width, height) in [(0, 256), (256, 0), (max + 1, 256), (256, max + 1)] {
        let mut spec = valid_spec();
        spec.width = width;
        spec.height = height;
        let canonical =
            association_request_from_proto(&associate_request_with_image_spec(spec, None))
                .expect("dimensions are validated by the canonical request");
        assert_eq!(
            canonical.validate(),
            Err(thumbnail_common::AssociationValidationError::InvalidDimensions)
        );
    }

    for field in [
        "processor_identity",
        "encoding_format",
        "transform_type",
        "crop_strategy",
    ] {
        for raw in [0, i32::MAX] {
            let mut spec = valid_spec();
            match field {
                "processor_identity" => spec.processor_identity = raw,
                "encoding_format" => spec.encoding_format = raw,
                "transform_type" => spec.transform_type = raw,
                "crop_strategy" => spec.crop_strategy = raw,
                _ => unreachable!(),
            }

            let error =
                association_request_from_proto(&associate_request_with_image_spec(spec, None))
                    .expect_err("required enum must reject unspecified and unknown values");
            assert!(error.contains(field), "{field} error was: {error}");
            if raw == 0 {
                assert!(error.contains("UNSPECIFIED"), "{field} error was: {error}");
            } else {
                assert!(error.contains("invalid"), "{field} error was: {error}");
            }
        }
    }

    let mut non_image_processor = valid_spec();
    non_image_processor.processor_identity = proto::ProcessorIdentity::VideoToImage as i32;
    let error = association_request_from_proto(&associate_request_with_image_spec(
        non_image_processor,
        None,
    ))
    .expect_err("image_to_image must not accept a different processor identity");
    assert!(
        error.contains("IMAGE_TO_IMAGE"),
        "unexpected error: {error}"
    );
}

/// A rejected specification has to say *which* specification. Nothing pinned this
/// before, and the messages shipped a literal `specifications[{i}]` for a while: a
/// caller sending a batch was told one of them was wrong, but not which one.
#[test]
fn create_thumbnails_proto_names_the_specification_that_failed() {
    let mut unspecified_encoding = image_to_image_specification();
    if let Some(thumbnail_specification::Specification::ImageToImage(spec)) =
        unspecified_encoding.specification.as_mut()
    {
        spec.encoding_format = 0;
    }

    let error =
        create_thumbnails_specs_from_proto(&[image_to_image_specification(), unspecified_encoding])
            .expect_err("an UNSPECIFIED encoding_format must be rejected");

    assert!(
        error.contains("specifications[1]"),
        "the failing index must be interpolated, got: {error}"
    );
    assert!(
        error.contains("encoding_format") && error.contains("UNSPECIFIED"),
        "the failing field and reason must survive, got: {error}"
    );
}

#[test]
fn associate_proto_preserves_canonical_thumbnail_and_resize_transforms() {
    for transform in [
        proto::TransformType::Thumbnail,
        proto::TransformType::Resize,
    ] {
        let mut spec = i2i_spec(Some(85), Some(true));
        spec.transform_type = transform as i32;
        let canonical =
            association_request_from_proto(&associate_request_with_image_spec(spec, None))
                .expect("canonical transform should convert");

        if let Some(AssociationSpec::ImageToImage { transform_type, .. }) = canonical.specification
        {
            assert_eq!(
                transform_type.to_string(),
                transform
                    .as_str_name()
                    .trim_start_matches("TRANSFORM_TYPE_")
            );
        } else {
            panic!("expected IMAGE_TO_IMAGE spec");
        }
    }
}

#[test]
fn associate_proto_applies_quality_range_only_to_lossy_formats() {
    for encoding_format in [
        proto::ImageEncodingFormat::Png,
        proto::ImageEncodingFormat::Gif,
    ] {
        let mut spec = i2i_spec(Some(101), Some(true));
        spec.encoding_format = encoding_format as i32;
        let canonical =
            association_request_from_proto(&associate_request_with_image_spec(spec, None))
                .expect("lossless specification should convert");
        assert_eq!(canonical.validate(), Ok(()));
    }

    for encoding_format in [
        proto::ImageEncodingFormat::Jpeg,
        proto::ImageEncodingFormat::Webp,
    ] {
        let mut spec = i2i_spec(Some(101), Some(true));
        spec.encoding_format = encoding_format as i32;
        let canonical =
            association_request_from_proto(&associate_request_with_image_spec(spec, None))
                .expect("lossy specification should convert");
        assert_eq!(
            canonical.validate(),
            Err(thumbnail_common::AssociationValidationError::InvalidQuality)
        );
    }
}

#[tokio::test]
async fn production_task_queue_grpc_payload_reaches_association_execution() {
    let service = test_service();
    let mut spec = i2i_spec(None, None);
    spec.transform_type = proto::TransformType::Thumbnail as i32;

    let response = service
        .associate_thumbnail(with_auth(associate_request_with_image_spec(spec, None)))
        .await
        .expect("valid queue payload should not fail transport validation")
        .into_inner();

    assert!(
        response.success,
        "a production task-queue payload should pass validation and execute: {:?}",
        response.error_message
    );
    assert!(
        response.error_message.is_none(),
        "a successful association reports no in-band error"
    );
}

// The two resource fields only ever echoed the request back and nothing read them. They
// stay in the proto so existing clients keep parsing, but neither path populates them.
// Reading the deprecated fields is the whole point: it is how the test proves the
// handler leaves them unset.
#[allow(deprecated)]
#[tokio::test]
async fn associate_response_carries_only_its_outcome() {
    use thumbnail_common::test_fixtures::mock_storage_server::{
        mock_storage, unique_test_address, MetadataOp,
    };

    let service = test_service();

    let success = service
        .associate_thumbnail(with_auth(associate_request_with_image_spec(
            i2i_spec(None, None),
            None,
        )))
        .await
        .expect("valid request")
        .into_inner();

    assert!(success.success);
    assert_eq!(success.error_message, None);
    assert_eq!(success.thumbnail, None);
    assert_eq!(success.association_target, None);

    // The in-band failure path builds its own response, so it needs checking too. It
    // runs against its own address: the fault below would otherwise reach any test
    // associating to the shared fixture address at the same time.
    let target_address = unique_test_address("associate-response-shape.usd");
    let mut failing_request = associate_request_with_image_spec(i2i_spec(None, None), None);
    failing_request.association_target = Some(resource(&target_address, Some("source-id")));
    mock_storage().fail(
        MetadataOp::Update,
        &target_address,
        None,
        tonic::Code::Internal,
    );

    let failure = service
        .associate_thumbnail(with_auth(failing_request))
        .await
        .expect("a storage failure is reported in-band, not as a status")
        .into_inner();

    assert!(!failure.success);
    assert!(failure.error_message.is_some());
    assert_eq!(failure.thumbnail, None);
    assert_eq!(failure.association_target, None);
}

// A well-formed token this service accepts can still be rejected downstream, most often
// because it expired. Associate reports its failures in-band, but this one gets a status
// so a client handling auth once for every RPC sees it where it already looks.
#[tokio::test]
async fn associate_reports_a_downstream_token_rejection_as_unauthenticated() {
    use thumbnail_common::test_fixtures::mock_storage_server::mock_storage;

    let service = test_service();
    mock_storage().reject_token("grpc-associate-expired-token");

    let mut request = Request::new(associate_request_with_image_spec(
        i2i_spec(None, None),
        None,
    ));
    request.metadata_mut().insert(
        "authorization",
        "Bearer grpc-associate-expired-token".parse().unwrap(),
    );

    let error = service
        .associate_thumbnail(request)
        .await
        .expect_err("a rejected token should fail the association");

    assert_eq!(error.code(), tonic::Code::Unauthenticated);
}

// ---------------------------------------------------------------------
// MR3 cross-transport specification parity: an association built over gRPC and
// the same association built over the HTTP JSON contract must normalize to a
// byte-identical canonical specification, so the deterministic key, stored
// metadata, and event payload (all pure functions of the request) match.
// ---------------------------------------------------------------------

/// A proto image-to-image spec matching the HTTP fixture below, with the
/// additive optional fields set explicitly by the caller.
fn i2i_spec(
    quality: Option<u32>,
    preserve_aspect_ratio: Option<bool>,
) -> ImageToImageSpecification {
    ImageToImageSpecification {
        processor_identity: 1, // IMAGE_TO_IMAGE
        encoding_format: 1,    // JPEG
        width: 256,
        height: 256,
        transform_type: 1, // SQUARE
        crop_strategy: 1,  // CENTER
        quality,
        preserve_aspect_ratio,
    }
}

/// HTTP JSON body whose resources mirror `associate_request_with_image_spec`,
/// with a caller-supplied `specification` object.
fn http_association_request(specification: serde_json::Value) -> AssociationRequest {
    serde_json::from_value(serde_json::json!({
        "target_type": "FILE",
        "thumbnail": {
            "resource_address": "https://store.test/thumb.png",
            "resource_identity": "thumb-id"
        },
        "association_target": {
            "resource_address": "https://store.test/source.usd",
            "resource_identity": "source-id"
        },
        "specification": specification
    }))
    .expect("http association request should deserialize")
}

fn association_output_snapshot(
    request: &AssociationRequest,
) -> (String, serde_json::Value, serde_json::Value) {
    let specification = request.specification.as_ref().expect("specification");
    let writes = thumbnail_common::storage::association::build_association_metadata_writes(
        request,
        specification,
        "2026-07-21T00:00:00Z",
    )
    .expect("metadata output");
    let stored_message = writes.association_target_side.value.clone();
    let event_message = thumbnail_common::storage::association::build_association_message(
        &request
            .message_params(Some(specification))
            .expect("validated request resolves its identities"),
    )
    .expect("event message");

    (
        writes.association_target_side.key.clone(),
        stored_message,
        event_message,
    )
}

#[test]
fn associate_http_and_grpc_normalize_identically_when_optionals_omitted() {
    thumbnail_common::test_fixtures::init_test_runtime_config("thumbnail-service-grpc-tests");

    let mut from_grpc = association_request_from_proto(&associate_request_with_image_spec(
        i2i_spec(None, None),
        Some(TargetType::File as i32),
    ))
    .expect("valid proto request");
    from_grpc.normalize_specification();

    let mut from_http = http_association_request(serde_json::json!({
        "type": "IMAGE_TO_IMAGE",
        "width": 256,
        "height": 256,
        "encoding_format": "JPEG",
        "transform_type": "SQUARE",
        "crop_strategy": "CENTER"
    }));
    from_http.normalize_specification();

    assert_eq!(from_grpc, from_http);
    assert_eq!(
        association_output_snapshot(&from_grpc),
        association_output_snapshot(&from_http)
    );
    if let Some(AssociationSpec::ImageToImage {
        quality,
        preserve_aspect_ratio,
        ..
    }) = from_grpc.specification
    {
        assert_eq!(quality, Some(85.0));
        assert_eq!(preserve_aspect_ratio, Some(true));
    } else {
        panic!("expected IMAGE_TO_IMAGE spec");
    }
}

#[test]
fn associate_http_and_grpc_normalize_identically_with_explicit_optionals() {
    thumbnail_common::test_fixtures::init_test_runtime_config("thumbnail-service-grpc-tests");

    let mut from_grpc = association_request_from_proto(&associate_request_with_image_spec(
        i2i_spec(Some(60), Some(false)),
        Some(TargetType::File as i32),
    ))
    .expect("valid proto request");
    from_grpc.normalize_specification();

    let mut from_http = http_association_request(serde_json::json!({
        "type": "IMAGE_TO_IMAGE",
        "width": 256,
        "height": 256,
        "encoding_format": "JPEG",
        "transform_type": "SQUARE",
        "crop_strategy": "CENTER",
        "quality": 60,
        "preserve_aspect_ratio": false
    }));
    from_http.normalize_specification();

    assert_eq!(from_grpc, from_http);
    assert_eq!(
        association_output_snapshot(&from_grpc),
        association_output_snapshot(&from_http)
    );
    if let Some(AssociationSpec::ImageToImage {
        quality,
        preserve_aspect_ratio,
        ..
    }) = from_grpc.specification
    {
        assert_eq!(quality, Some(60.0));
        // Explicit `false` is honored, distinct from the runtime default of `true`.
        assert_eq!(preserve_aspect_ratio, Some(false));
    } else {
        panic!("expected IMAGE_TO_IMAGE spec");
    }
}

#[test]
fn real_http_and_grpc_mappers_match_outputs_across_supported_formats() {
    thumbnail_common::test_fixtures::init_test_runtime_config("thumbnail-service-grpc-tests");

    for (proto_format, http_format) in [
        (proto::ImageEncodingFormat::Jpeg, "JPEG"),
        (proto::ImageEncodingFormat::Png, "PNG"),
        (proto::ImageEncodingFormat::Webp, "WEBP"),
    ] {
        for (quality, preserve_aspect_ratio) in [(None, None), (Some(63), Some(false))] {
            let mut proto_spec = i2i_spec(quality, preserve_aspect_ratio);
            proto_spec.encoding_format = proto_format as i32;
            let mut from_grpc = association_request_from_proto(&associate_request_with_image_spec(
                proto_spec,
                Some(TargetType::File as i32),
            ))
            .expect("valid proto request");
            from_grpc.normalize_specification();

            let mut http_specification = serde_json::json!({
                "type": "IMAGE_TO_IMAGE",
                "width": 256,
                "height": 256,
                "encoding_format": http_format,
                "transform_type": "SQUARE",
                "crop_strategy": "CENTER"
            });
            if let Some(value) = quality {
                http_specification["quality"] = serde_json::json!(value);
            }
            if let Some(value) = preserve_aspect_ratio {
                http_specification["preserve_aspect_ratio"] = serde_json::json!(value);
            }
            let mut from_http = http_association_request(http_specification);
            from_http.normalize_specification();

            assert_eq!(from_grpc, from_http);
            assert_eq!(
                association_output_snapshot(&from_grpc),
                association_output_snapshot(&from_http),
                "{http_format} output parity failed for quality={quality:?}, preserve_aspect_ratio={preserve_aspect_ratio:?}"
            );
        }
    }
}

#[test]
fn associate_proto_legacy_client_omitting_optionals_uses_documented_defaults() {
    thumbnail_common::test_fixtures::init_test_runtime_config("thumbnail-service-grpc-tests");

    // A legacy client that never sets the additive fields.
    let mut canonical = association_request_from_proto(&associate_request_with_image_spec(
        i2i_spec(None, None),
        None,
    ))
    .expect("legacy proto request should convert");

    // Presence is preserved through conversion — the gRPC handler injects nothing.
    if let Some(AssociationSpec::ImageToImage {
        quality,
        preserve_aspect_ratio,
        ..
    }) = canonical.specification.as_ref()
    {
        assert_eq!(*quality, None);
        assert_eq!(*preserve_aspect_ratio, None);
    } else {
        panic!("expected IMAGE_TO_IMAGE spec");
    }

    // It still validates and, after the shared normalization, uses runtime defaults.
    assert_eq!(canonical.validate(), Ok(()));
    canonical.normalize_specification();
    let (key, stored_message, event_message) = association_output_snapshot(&canonical);
    let expected_event = serde_json::json!({
        "thumbnail": {
            "resource_address": "https://store.test/thumb.png",
            "resource_identity": "thumb-id"
        },
        "association_target": {
            "resource_address": "https://store.test/source.usd",
            "resource_identity": "source-id"
        },
        "target_type": "FILE",
        "specification": {
            "type": "IMAGE_TO_IMAGE",
            "width": 256,
            "height": 256,
            "encoding_format": "JPEG",
            "transform_type": "SQUARE",
            "crop_strategy": "CENTER",
            "quality": 85.0,
            "preserve_aspect_ratio": true
        }
    });
    let mut expected_stored = expected_event.clone();
    expected_stored["create_time"] = serde_json::json!("2026-07-21T00:00:00Z");
    // Key changes with the new hash ordering (spec_type hashed before transform/crop).
    // The new key is verified by running the test.
    assert!(key.starts_with("thumbnail_256x256_JPEG_"), "key: {key}");
    assert_eq!(stored_message, expected_stored);
    assert_eq!(event_message, expected_event);

    if let Some(AssociationSpec::ImageToImage {
        quality,
        preserve_aspect_ratio,
        ..
    }) = canonical.specification
    {
        assert_eq!(quality, Some(85.0));
        assert_eq!(preserve_aspect_ratio, Some(true));
    } else {
        panic!("expected IMAGE_TO_IMAGE spec");
    }
}

#[test]
fn associate_proto_passes_through_explicit_optionals() {
    thumbnail_common::test_fixtures::init_test_runtime_config("thumbnail-service-grpc-tests");

    let canonical = association_request_from_proto(&associate_request_with_image_spec(
        i2i_spec(Some(60), Some(false)),
        None,
    ))
    .expect("proto request with explicit optionals should convert");
    if let Some(AssociationSpec::ImageToImage {
        quality,
        preserve_aspect_ratio,
        ..
    }) = canonical.specification
    {
        assert_eq!(quality, Some(60.0));
        assert_eq!(preserve_aspect_ratio, Some(false));
    } else {
        panic!("expected IMAGE_TO_IMAGE spec");
    }
}

// Both association_target and thumbnail are required.
#[tokio::test]
async fn disassociate_requires_association_target() {
    let service = test_service();
    let req = DisassociateThumbnailRequest {
        association_target: None,
        target_type: None,
        thumbnail: Some(resource("https://store.test/thumb.png", None)),
    };

    let err = service
        .disassociate_thumbnail(with_auth(req))
        .await
        .expect_err("missing association_target should be rejected");
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
}

#[tokio::test]
async fn disassociate_requires_thumbnail() {
    let service = test_service();
    let req = DisassociateThumbnailRequest {
        association_target: Some(resource("https://store.test/source.usd", None)),
        target_type: None,
        thumbnail: None,
    };

    let err = service
        .disassociate_thumbnail(with_auth(req))
        .await
        .expect_err("missing thumbnail should be rejected");
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
}

// Resource identities on both resources are dropped at the proto boundary; only
// addresses are used to locate and remove the association entry.
#[tokio::test]
async fn disassociate_ignores_supplied_identities() {
    for target_type in [TargetType::File, TargetType::Folder] {
        let canonical = disassociate_request_from_proto(&DisassociateThumbnailRequest {
            association_target: Some(resource(
                "https://store.test/source.usd",
                Some("ignored-target-identity"),
            )),
            target_type: Some(target_type as i32),
            thumbnail: Some(resource(
                "https://store.test/thumb.png",
                Some("ignored-thumb-identity"),
            )),
        })
        .expect("supplied identities should not reject the request");

        assert_eq!(
            canonical.association_target_resource_address,
            "https://store.test/source.usd"
        );
        assert_eq!(
            canonical.thumbnail_resource_address,
            "https://store.test/thumb.png"
        );
        assert_eq!(canonical.validate(), Ok(()));
    }
}

// Identities are not required; only resource addresses are used for the removal.
#[tokio::test]
async fn disassociate_accepts_targets_and_thumbnails_without_identities() {
    let service = test_service();
    let (target_address, thumbnail_addresses) = seed_associated_thumbnails("grpc-no-ids", 1);

    let response = service
        .disassociate_thumbnail(with_auth(DisassociateThumbnailRequest {
            association_target: Some(resource(&target_address, None)),
            target_type: Some(TargetType::File as i32),
            thumbnail: Some(resource(&thumbnail_addresses[0], None)),
        }))
        .await
        .expect("addresses without identities should be accepted")
        .into_inner();

    assert_eq!(response.succeeded.len(), 1);
    assert_eq!(
        response.succeeded[0]
            .thumbnail
            .as_ref()
            .expect("thumbnail")
            .resource_address,
        thumbnail_addresses[0]
    );
}

// A malformed address is the caller's mistake, so it must be caught before reaching storage.
#[tokio::test]
async fn disassociate_rejects_a_blank_address_before_reaching_storage() {
    use thumbnail_common::test_fixtures::mock_storage_server::mock_storage;

    let service = test_service();

    let err = service
        .disassociate_thumbnail(with_auth(DisassociateThumbnailRequest {
            association_target: Some(resource("   ", None)),
            target_type: None,
            thumbnail: Some(resource("https://store.test/thumb.png", None)),
        }))
        .await
        .expect_err("a blank association_target address should be rejected");

    assert_eq!(err.code(), tonic::Code::InvalidArgument);
    assert!(mock_storage().calls_for("   ").is_empty());
}

// ---------------------------------------------------------------------
// Disassociate cross-transport parity, the counterpart to the associate parity tests
// above: one logical targeted request through both real request mappings, so the two
// transports cannot drift apart in what they accept or in what they remove.
// ---------------------------------------------------------------------

/// The HTTP mapping — the same query-string format axum's `Query` extractor parses.
fn http_disassociate_request(query: &str) -> DisassociateRequest {
    serde_urlencoded::from_str(query).expect("http disassociate query should deserialize")
}

#[test]
fn disassociate_http_and_grpc_produce_the_same_canonical_request() {
    for (proto_kind, wire_kind) in [(TargetType::File, "FILE"), (TargetType::Folder, "FOLDER")] {
        let from_grpc = disassociate_request_from_proto(&DisassociateThumbnailRequest {
            association_target: Some(resource("https://store.test/source.usd", None)),
            target_type: Some(proto_kind as i32),
            thumbnail: Some(resource("https://store.test/thumb.png", None)),
        })
        .expect("valid proto request");

        let from_http = http_disassociate_request(&format!(
            "target_type={wire_kind}\
             &association_target_resource_address=https://store.test/source.usd\
             &thumbnail_resource_address=https://store.test/thumb.png"
        ));

        assert_eq!(from_grpc, from_http);
        assert_eq!(from_grpc.validate(), from_http.validate());
    }
}

#[test]
fn disassociate_http_and_grpc_reach_the_same_validation_decision() {
    for (address, should_be_accepted) in [
        ("https://store.test/source.usd", true),
        ("   ", false),
        ("", false),
    ] {
        let from_grpc = disassociate_request_from_proto(&DisassociateThumbnailRequest {
            association_target: Some(resource(address, None)),
            target_type: None,
            thumbnail: Some(resource("https://store.test/thumb.png", None)),
        })
        .expect("an address is never rejected at the proto mapping, only at validate()");

        let from_http = http_disassociate_request(&format!(
            "association_target_resource_address={address}\
             &thumbnail_resource_address=https://store.test/thumb.png"
        ));

        assert_eq!(from_grpc, from_http);
        assert_eq!(from_grpc.validate(), from_http.validate());
        assert_eq!(from_grpc.validate().is_ok(), should_be_accepted);
    }
}

// The two transports share one business-logic function, so identical inputs must
// produce identical removals.
#[tokio::test]
async fn disassociate_http_and_grpc_remove_identically() {
    use axum::extract::{Query, State};

    let service = test_service();
    let (grpc_target, grpc_thumbnails) = seed_associated_thumbnails("parity-grpc", 1);
    let (http_target, http_thumbnails) = seed_associated_thumbnails("parity-http", 1);

    let grpc_response = service
        .disassociate_thumbnail(with_auth(DisassociateThumbnailRequest {
            association_target: Some(resource(&grpc_target, None)),
            target_type: Some(TargetType::Folder as i32),
            thumbnail: Some(resource(&grpc_thumbnails[0], None)),
        }))
        .await
        .expect("removal is reported in-band over gRPC")
        .into_inner();

    let mut headers = axum::http::HeaderMap::new();
    headers.insert("Authorization", "Bearer test-token".parse().unwrap());
    let http_response = crate::http::disassociate_thumbnail(
        State(crate::http::tests::test_app_state()),
        headers,
        Ok(Query(http_disassociate_request(&format!(
            "target_type=FOLDER\
             &association_target_resource_address={http_target}\
             &thumbnail_resource_address={}",
            http_thumbnails[0]
        )))),
    )
    .await;

    assert_eq!(http_response.status(), axum::http::StatusCode::OK);
    let http_body: serde_json::Value = {
        let bytes = axum::body::to_bytes(http_response.into_body(), usize::MAX)
            .await
            .expect("response body");
        serde_json::from_slice(&bytes).expect("json body")
    };

    // Both transports succeed and report the same thumbnail.
    assert_eq!(grpc_response.succeeded.len(), 1);
    assert!(grpc_response.failed.is_empty());
    assert_eq!(
        http_body["succeeded"].as_array().expect("succeeded").len(),
        1
    );
    assert!(http_body["failed"].as_array().expect("failed").is_empty());

    assert_eq!(
        grpc_response.succeeded[0]
            .thumbnail
            .as_ref()
            .expect("thumbnail")
            .resource_address,
        grpc_thumbnails[0]
    );
    assert_eq!(
        http_body["succeeded"][0]["resource_address"],
        http_thumbnails[0]
    );
}

// gRPC's target_type is optional and has always defaulted to FILE; HTTP now matches.
#[tokio::test]
async fn disassociate_defaults_an_omitted_target_type_to_file() {
    let canonical = disassociate_request_from_proto(&DisassociateThumbnailRequest {
        association_target: Some(resource("https://store.test/source.usd", None)),
        target_type: None,
        thumbnail: Some(resource("https://store.test/thumb.png", None)),
    })
    .expect("an omitted target_type should be accepted");

    assert_eq!(canonical.target_kind, AssociationTargetKind::File);
}

/// Seed one association-target address holding `count` thumbnail entries, as a prior
/// round of associations would have left it. Returns the target and thumbnail addresses.
fn seed_associated_thumbnails(label: &str, count: usize) -> (String, Vec<String>) {
    use thumbnail_common::test_fixtures::mock_storage_server::{mock_storage, unique_test_address};

    let target_address = unique_test_address(&format!("{label}-source.usd"));
    let thumbnail_addresses: Vec<String> = (0..count)
        .map(|n| unique_test_address(&format!("{label}-thumb-{n}.png")))
        .collect();

    for (n, thumbnail_address) in thumbnail_addresses.iter().enumerate() {
        mock_storage().seed(
            &target_address,
            &format!("thumbnail_256x256_PNG_key{n}"),
            serde_json::json!({
                "thumbnail": { "resource_address": thumbnail_address },
                "association_target": { "resource_address": target_address },
                "target_type": "FILE",
            }),
        );
        mock_storage().seed(
            thumbnail_address,
            THUMBNAIL_METADATA_KEY,
            serde_json::json!({ "status": "SUCCESS" }),
        );
    }

    (target_address, thumbnail_addresses)
}

// An error Status carries no response body, so gRPC reserves one for the single outcome
// with no targeted result data (enumeration failure). A removal failure returns OK with the
// detail in `failed`, so the client can read it. This diverges from HTTP's 502.
#[tokio::test]
async fn disassociate_reports_removal_failure_as_targeted_detail() {
    use thumbnail_common::test_fixtures::mock_storage_server::{mock_storage, MetadataOp};

    let service = test_service();
    let (target_address, thumbnail_addresses) = seed_associated_thumbnails("grpc-failure", 1);
    mock_storage().fail(
        MetadataOp::Delete,
        &target_address,
        Some("thumbnail_256x256_PNG_key0"),
        tonic::Code::Internal,
    );

    let response = service
        .disassociate_thumbnail(with_auth(DisassociateThumbnailRequest {
            association_target: Some(resource(&target_address, None)),
            target_type: Some(TargetType::Folder as i32),
            thumbnail: Some(resource(&thumbnail_addresses[0], None)),
        }))
        .await
        .expect("removal failure carries targeted detail, not an error Status")
        .into_inner();

    assert!(response.succeeded.is_empty());
    assert_eq!(response.failed.len(), 1);
    assert!(
        response.failed[0].error_message.is_some(),
        "the failure should describe why, so the client can decide what to retry"
    );
}

#[tokio::test]
async fn disassociate_surfaces_enumeration_failure_as_an_error_status() {
    use thumbnail_common::test_fixtures::mock_storage_server::{mock_storage, MetadataOp};

    let service = test_service();
    let (target_address, thumbnail_addresses) =
        seed_associated_thumbnails("grpc-enumeration-failure", 1);
    mock_storage().fail(
        MetadataOp::Get,
        &target_address,
        None,
        tonic::Code::Internal,
    );

    let error = service
        .disassociate_thumbnail(with_auth(DisassociateThumbnailRequest {
            association_target: Some(resource(&target_address, None)),
            target_type: Some(TargetType::Folder as i32),
            thumbnail: Some(resource(&thumbnail_addresses[0], None)),
        }))
        .await
        .expect_err("an enumeration failure has no results to report");

    // Distinguishes it from a missing association, which legitimately yields empty arrays.
    assert_eq!(error.code(), tonic::Code::Unavailable);
}

// A rejected token is the caller's to fix. UNAUTHENTICATED rather than UNAVAILABLE so
// they don't retry and wait for an outage that isn't there.
#[tokio::test]
async fn disassociate_reports_a_rejected_token_as_unauthenticated() {
    use thumbnail_common::test_fixtures::mock_storage_server::mock_storage;

    let service = test_service();
    let (target_address, thumbnail_addresses) = seed_associated_thumbnails("grpc-expired", 1);
    mock_storage().reject_token("grpc-expired-token");

    let mut request = Request::new(DisassociateThumbnailRequest {
        association_target: Some(resource(&target_address, None)),
        target_type: Some(TargetType::Folder as i32),
        thumbnail: Some(resource(&thumbnail_addresses[0], None)),
    });
    request.metadata_mut().insert(
        "authorization",
        "Bearer grpc-expired-token".parse().unwrap(),
    );

    let error = service
        .disassociate_thumbnail(request)
        .await
        .expect_err("a rejected token should fail the call");

    assert_eq!(error.code(), tonic::Code::Unauthenticated);
}

#[tokio::test]
async fn disassociate_fails_fast_without_a_token() {
    use thumbnail_common::test_fixtures::mock_storage_server::mock_storage;

    let service = test_service();
    let (target_address, thumbnail_addresses) = seed_associated_thumbnails("grpc-no-token", 1);

    let error = service
        .disassociate_thumbnail(Request::new(DisassociateThumbnailRequest {
            association_target: Some(resource(&target_address, None)),
            target_type: Some(TargetType::Folder as i32),
            thumbnail: Some(resource(&thumbnail_addresses[0], None)),
        }))
        .await
        .expect_err("a call without a token should be rejected");

    assert_eq!(error.code(), tonic::Code::Unauthenticated);
    assert!(
        mock_storage().calls_for(&target_address).is_empty(),
        "should reject the request before enumerating anything"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// create_thumbnails
// ─────────────────────────────────────────────────────────────────────────────

mod create_thumbnails_tests {
    use super::*;
    use std::net::SocketAddr;
    use std::sync::{Arc, Mutex};

    /// Fixed address the task-queue mock server binds to in serial tests.
    /// Must match `MOCK_TASK_QUEUE_PORT_GRPC` set in `test_service()`.
    use storage_proto::nvidia::omniverse::storage::fileobject::v1alpha::file_object_service_server::{
        FileObjectService as FosTrait, FileObjectServiceServer,
    };
    use storage_proto::nvidia::omniverse::storage::fileobject::v1alpha::{
        AbortMultipartUploadRequest, AbortMultipartUploadResponse, CompleteMultipartUploadRequest,
        CompleteMultipartUploadResponse, CompleteRedirectUploadRequest,
        CompleteRedirectUploadResponse, CopyRequest, CopyResponse, DeleteRequest, DeleteResponse,
        EnumerateRequest, EnumerateResponse, FetchWriteTypeInfoRequest, FetchWriteTypeInfoResponse,
        GetNativeMetadataSupportRequest, GetNativeMetadataSupportResponse,
        GetOptimisticLockingSupportRequest, GetOptimisticLockingSupportResponse, MoveRequest,
        MoveResponse, ReadFromAddressRequest, ReadFromAddressResponse, ReadRequest, ReadResponse,
        ResourceIdentity, ResourceInfo, StatRequest, StatResponse, UploadPartRequest,
        UploadPartResponse, WriteRequest, WriteResponse,
    };
    use thumbnail_common::test_fixtures::mock_publisher_server::{
        MockPublisherServer, PublisherBehavior,
    };
    use tokio_stream::wrappers::TcpListenerStream;
    use tonic::transport::Server;
    use tonic::{Request as TonicRequest, Response as TonicResponse, Status};

    type BoxStream<T> =
        std::pin::Pin<Box<dyn tokio_stream::Stream<Item = Result<T, Status>> + Send>>;

    #[derive(Clone)]
    enum StatResp {
        Ok(String),
        Err(tonic::Code),
    }

    struct MockFos {
        resp: Arc<Mutex<StatResp>>,
        /// How many `Stat` calls arrived. A test asserting that validation
        /// precedes the outbound call needs something to observe; wiring up no
        /// double at all only proves the call failed, not that it never happened.
        stats: Arc<std::sync::atomic::AtomicUsize>,
    }

    #[tonic::async_trait]
    impl FosTrait for MockFos {
        type EnumerateStream = BoxStream<EnumerateResponse>;
        type ReadStream = BoxStream<ReadResponse>;
        type ReadFromAddressStream = BoxStream<ReadFromAddressResponse>;
        type WriteStream = BoxStream<WriteResponse>;

        async fn stat(
            &self,
            _: TonicRequest<StatRequest>,
        ) -> Result<TonicResponse<StatResponse>, Status> {
            self.stats.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            match self.resp.lock().unwrap().clone() {
                StatResp::Ok(id) => Ok(TonicResponse::new(StatResponse {
                    resource_info: Some(ResourceInfo {
                        resource_identity: Some(ResourceIdentity {
                            encoded_identity: id,
                        }),
                        metadata: None,
                    }),
                })),
                StatResp::Err(c) => Err(Status::new(c, "mock")),
            }
        }
        async fn enumerate(
            &self,
            _: TonicRequest<EnumerateRequest>,
        ) -> Result<TonicResponse<Self::EnumerateStream>, Status> {
            Err(Status::unimplemented(""))
        }
        async fn read(
            &self,
            _: TonicRequest<ReadRequest>,
        ) -> Result<TonicResponse<Self::ReadStream>, Status> {
            Err(Status::unimplemented(""))
        }
        async fn read_from_address(
            &self,
            _: TonicRequest<ReadFromAddressRequest>,
        ) -> Result<TonicResponse<Self::ReadFromAddressStream>, Status> {
            Err(Status::unimplemented(""))
        }
        async fn fetch_write_type_info(
            &self,
            _: TonicRequest<FetchWriteTypeInfoRequest>,
        ) -> Result<TonicResponse<FetchWriteTypeInfoResponse>, Status> {
            Err(Status::unimplemented(""))
        }
        async fn write(
            &self,
            _: tonic::Request<tonic::Streaming<WriteRequest>>,
        ) -> Result<TonicResponse<Self::WriteStream>, Status> {
            Err(Status::unimplemented(""))
        }
        async fn complete_redirect_upload(
            &self,
            _: TonicRequest<CompleteRedirectUploadRequest>,
        ) -> Result<TonicResponse<CompleteRedirectUploadResponse>, Status> {
            Err(Status::unimplemented(""))
        }
        async fn upload_part(
            &self,
            _: TonicRequest<UploadPartRequest>,
        ) -> Result<TonicResponse<UploadPartResponse>, Status> {
            Err(Status::unimplemented(""))
        }
        async fn complete_multipart_upload(
            &self,
            _: TonicRequest<CompleteMultipartUploadRequest>,
        ) -> Result<TonicResponse<CompleteMultipartUploadResponse>, Status> {
            Err(Status::unimplemented(""))
        }
        async fn abort_multipart_upload(
            &self,
            _: TonicRequest<AbortMultipartUploadRequest>,
        ) -> Result<TonicResponse<AbortMultipartUploadResponse>, Status> {
            Err(Status::unimplemented(""))
        }
        async fn delete(
            &self,
            _: TonicRequest<DeleteRequest>,
        ) -> Result<TonicResponse<DeleteResponse>, Status> {
            Err(Status::unimplemented(""))
        }
        async fn copy(
            &self,
            _: TonicRequest<CopyRequest>,
        ) -> Result<TonicResponse<CopyResponse>, Status> {
            Err(Status::unimplemented(""))
        }

        async fn r#move(
            &self,
            _: TonicRequest<MoveRequest>,
        ) -> Result<TonicResponse<MoveResponse>, Status> {
            Err(Status::unimplemented(""))
        }

        async fn get_native_metadata_support(
            &self,
            _: TonicRequest<GetNativeMetadataSupportRequest>,
        ) -> Result<TonicResponse<GetNativeMetadataSupportResponse>, Status> {
            Err(Status::unimplemented(""))
        }

        async fn get_optimistic_locking_support(
            &self,
            _: TonicRequest<GetOptimisticLockingSupportRequest>,
        ) -> Result<TonicResponse<GetOptimisticLockingSupportResponse>, Status> {
            Err(Status::unimplemented(""))
        }
    }

    async fn start_fos(resp: StatResp) -> SocketAddr {
        start_counting_fos(resp).await.0
    }

    /// `start_fos` plus the counter its double increments on every `Stat`.
    async fn start_counting_fos(
        resp: StatResp,
    ) -> (SocketAddr, Arc<std::sync::atomic::AtomicUsize>) {
        let resp = Arc::new(Mutex::new(resp));
        let stats = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let served_stats = stats.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = Server::builder()
                .add_service(FileObjectServiceServer::new(MockFos {
                    resp,
                    stats: served_stats,
                }))
                .serve_with_incoming(TcpListenerStream::new(listener))
                .await;
        });
        (addr, stats)
    }

    fn service_with_storage(storage_addr: SocketAddr) -> ThumbnailServiceImpl {
        thumbnail_common::test_fixtures::init_test_runtime_config("thumbnail-service-grpc-tests");
        let ch = tonic::transport::Endpoint::from_shared(format!("http://{storage_addr}"))
            .unwrap()
            .connect_lazy();
        let outbound = Arc::new(OutboundClients {
            storage_channel: ch,
            ..OutboundClients::for_tests()
        });
        ThumbnailServiceImpl::new(outbound)
    }

    /// A service whose storage points at `storage_addr` and whose generate
    /// publishing reaches `publisher`. Dispatch is a gRPC publish now, so each
    /// test gets its own ephemeral publisher instead of sharing a fixed port.
    fn service_with_storage_and_publisher(
        storage_addr: SocketAddr,
        publisher: &MockPublisherServer,
    ) -> ThumbnailServiceImpl {
        thumbnail_common::test_fixtures::init_test_runtime_config("thumbnail-service-grpc-tests");
        let storage = tonic::transport::Endpoint::from_shared(format!("http://{storage_addr}"))
            .unwrap()
            .connect_lazy();
        ThumbnailServiceImpl::new(Arc::new(OutboundClients {
            storage_channel: storage,
            ..OutboundClients::for_tests_publishing_to(publisher.addr)
        }))
    }

    fn image_to_image_spec() -> ThumbnailSpecification {
        use thumbnail_proto::nvidia::omniverse::thumbnail::v1alpha::thumbnail_specification::Specification;
        ThumbnailSpecification {
            specification: Some(Specification::ImageToImage(ImageToImageSpecification {
                processor_identity: 1,
                encoding_format: 2, // PNG
                width: 256,
                height: 256,
                transform_type: 1, // SQUARE
                crop_strategy: 1,  // CENTER
                quality: Some(85), // required by resolve() for template rendering
                preserve_aspect_ratio: None,
            })),
        }
    }

    // ── Validation tests (no network) ────────────────────────────────────────

    #[tokio::test]
    async fn missing_token_returns_unauthenticated() {
        thumbnail_common::test_fixtures::init_test_runtime_config("thumbnail-service-grpc-tests");
        let service = ThumbnailServiceImpl::new(Arc::new(OutboundClients::for_tests()));
        let result = service
            .create_thumbnails(Request::new(CreateThumbnailsRequest {
                resource_address: "https://storage.test/image.png".to_string(),
                specifications: vec![],
            }))
            .await;
        assert_eq!(result.unwrap_err().code(), tonic::Code::Unauthenticated);
    }

    #[tokio::test]
    async fn empty_resource_address_returns_invalid_argument() {
        thumbnail_common::test_fixtures::init_test_runtime_config("thumbnail-service-grpc-tests");
        let service = ThumbnailServiceImpl::new(Arc::new(OutboundClients::for_tests()));
        let result = service
            .create_thumbnails(with_auth(CreateThumbnailsRequest {
                resource_address: "  ".to_string(),
                specifications: vec![],
            }))
            .await;
        assert_eq!(result.unwrap_err().code(), tonic::Code::InvalidArgument);
    }

    #[tokio::test]
    async fn manual_spec_is_rejected_by_create_thumbnails() {
        use thumbnail_proto::nvidia::omniverse::thumbnail::v1alpha::{
            thumbnail_specification::Specification, ManualSpecification,
        };
        thumbnail_common::test_fixtures::init_test_runtime_config("thumbnail-service-grpc-tests");
        let service = ThumbnailServiceImpl::new(Arc::new(OutboundClients::for_tests()));
        let manual_spec = ThumbnailSpecification {
            specification: Some(Specification::Manual(ManualSpecification {
                width: 256,
                height: 256,
                encoding_format: 2, // PNG
            })),
        };
        let result = service
            .create_thumbnails(with_auth(CreateThumbnailsRequest {
                resource_address: "https://storage.test/image.png".to_string(),
                specifications: vec![manual_spec],
            }))
            .await;
        let err = result.unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(
            err.message().to_lowercase().contains("manual"),
            "error should identify MANUAL as unsupported for CreateThumbnails: {}",
            err.message()
        );
    }

    /// The gRPC boundary range-checks dimensions too, and does it before the
    /// source Stat. Reaching the work-event contract instead would report a bad
    /// integer as INTERNAL -- the status reserved for "every publish failed".
    #[tokio::test]
    async fn out_of_range_dimensions_are_invalid_argument_before_any_stat() {
        use thumbnail_proto::nvidia::omniverse::thumbnail::v1alpha::{
            thumbnail_specification::Specification, ImageToImageSpecification,
        };
        thumbnail_common::test_fixtures::init_test_runtime_config("thumbnail-service-grpc-tests");

        for (field, width, height) in [
            ("width", 20_000, 256),
            ("height", 256, 20_000),
            ("width", 0, 256),
            ("height", 256, 0),
        ] {
            // No storage double is wired up: if this reached the Stat it would
            // fail as a transport error rather than INVALID_ARGUMENT.
            let service = ThumbnailServiceImpl::new(Arc::new(OutboundClients::for_tests()));
            let spec = ThumbnailSpecification {
                specification: Some(Specification::ImageToImage(ImageToImageSpecification {
                    processor_identity: 1,
                    encoding_format: 2,
                    width,
                    height,
                    transform_type: 1,
                    crop_strategy: 1,
                    quality: None,
                    preserve_aspect_ratio: None,
                })),
            };

            let err = service
                .create_thumbnails(with_auth(CreateThumbnailsRequest {
                    resource_address: "https://storage.test/image.png".to_string(),
                    specifications: vec![spec],
                }))
                .await
                .expect_err("an out-of-range dimension is a caller error");

            assert_eq!(
                err.code(),
                tonic::Code::InvalidArgument,
                "{field}={width}x{height} must be INVALID_ARGUMENT, got {err:?}"
            );
            assert!(
                err.message().contains(field) && err.message().contains("between 1 and 8192"),
                "the message must name the field and the range, got: {}",
                err.message()
            );
        }
    }

    #[tokio::test]
    async fn a_boundary_dimension_passes_grpc_validation() {
        use thumbnail_proto::nvidia::omniverse::thumbnail::v1alpha::{
            thumbnail_specification::Specification, ImageToImageSpecification,
        };
        thumbnail_common::test_fixtures::init_test_runtime_config("thumbnail-service-grpc-tests");
        let service = ThumbnailServiceImpl::new(Arc::new(OutboundClients::for_tests()));
        let spec = ThumbnailSpecification {
            specification: Some(Specification::ImageToImage(ImageToImageSpecification {
                processor_identity: 1,
                encoding_format: 2,
                width: 8192,
                height: 8192,
                transform_type: 1,
                crop_strategy: 1,
                quality: None,
                preserve_aspect_ratio: None,
            })),
        };

        let result = service
            .create_thumbnails(with_auth(CreateThumbnailsRequest {
                resource_address: "https://storage.test/image.png".to_string(),
                specifications: vec![spec],
            }))
            .await;

        if let Err(err) = result {
            assert_ne!(
                err.code(),
                tonic::Code::InvalidArgument,
                "8192 is inside the range and must not be rejected as out of range"
            );
        }
    }

    /// gRPC accepted a specification list of any length while HTTP refused
    /// anything past 1,000, so the same request was legal on one transport and
    /// not the other. Refused before the Stat, for the reason the dimension check
    /// is: a request the service will not serve should cost no outbound call.
    #[tokio::test]
    async fn more_than_one_thousand_specs_returns_invalid_argument_before_any_stat() {
        // A double that would answer, so a passing "no Stat" assertion means the
        // call was never made rather than that it could not have succeeded.
        let (addr, stats) = start_counting_fos(StatResp::Ok("enc-id".to_string())).await;
        let service = service_with_storage(addr);

        let err = service
            .create_thumbnails(with_auth(CreateThumbnailsRequest {
                resource_address: "https://storage.test/image.png".to_string(),
                specifications: vec![
                    image_to_image_specification();
                    thumbnail_common::work_events::MAX_CREATE_THUMBNAIL_SPECIFICATIONS
                        + 1
                ],
            }))
            .await
            .expect_err("an over-long specification list is a caller error");

        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(
            err.message().contains("specifications") && err.message().contains("at most 1000"),
            "the message must name the specification list and the bound, got: {}",
            err.message()
        );
        assert_eq!(
            stats.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "a refused request must cost no storage Stat"
        );
    }

    /// The bound is inclusive, and identically so on both transports — see the
    /// HTTP twin of the same name.
    #[tokio::test]
    async fn exactly_one_thousand_specs_is_accepted() {
        let publisher = MockPublisherServer::start().await;
        let addr = start_fos(StatResp::Ok("enc-id".to_string())).await;
        let service = service_with_storage_and_publisher(addr, &publisher);

        let response = service
            .create_thumbnails(with_auth(CreateThumbnailsRequest {
                resource_address: "https://storage.test/image.png".to_string(),
                specifications: vec![
                    image_to_image_specification();
                    thumbnail_common::work_events::MAX_CREATE_THUMBNAIL_SPECIFICATIONS
                ],
            }))
            .await
            .expect("1000 is at the bound, not past it");

        assert!(
            response.get_ref().failed.is_empty(),
            "every specification at the bound should have dispatched"
        );

        publisher.shutdown().await;
    }

    /// A storage permission denial is PERMISSION_DENIED, not UNAUTHENTICATED.
    /// The token was accepted; refreshing it cannot help.
    #[tokio::test]
    async fn storage_permission_denied_returns_permission_denied() {
        let addr = start_fos(StatResp::Err(tonic::Code::PermissionDenied)).await;
        let service = service_with_storage(addr);

        let err = service
            .create_thumbnails(with_auth(CreateThumbnailsRequest {
                resource_address: "https://storage.test/image.png".to_string(),
                specifications: vec![image_to_image_specification()],
            }))
            .await
            .expect_err("a denied Stat must fail the request");

        assert_eq!(err.code(), tonic::Code::PermissionDenied);
    }

    // ── Storage error tests ───────────────────────────────────────────────────

    #[tokio::test]
    async fn storage_404_returns_not_found() {
        let addr = start_fos(StatResp::Err(tonic::Code::NotFound)).await;
        let service = service_with_storage(addr);
        let result = service
            .create_thumbnails(with_auth(CreateThumbnailsRequest {
                resource_address: "https://storage.test/missing.png".to_string(),
                specifications: vec![],
            }))
            .await;
        assert_eq!(result.unwrap_err().code(), tonic::Code::NotFound);
    }

    #[tokio::test]
    async fn storage_unauthenticated_returns_unauthenticated() {
        let addr = start_fos(StatResp::Err(tonic::Code::Unauthenticated)).await;
        let service = service_with_storage(addr);
        let result = service
            .create_thumbnails(with_auth(CreateThumbnailsRequest {
                resource_address: "https://storage.test/image.png".to_string(),
                specifications: vec![],
            }))
            .await;
        assert_eq!(result.unwrap_err().code(), tonic::Code::Unauthenticated);
    }

    // ── Generate dispatch tests ───────────────────────────────────────────────

    #[tokio::test]
    async fn full_success_returns_ok_with_empty_failed() {
        let publisher = MockPublisherServer::start().await;
        let addr = start_fos(StatResp::Ok("enc-id".to_string())).await;

        let resp = service_with_storage_and_publisher(addr, &publisher)
            .create_thumbnails(with_auth(CreateThumbnailsRequest {
                resource_address: "https://storage.test/image.png".to_string(),
                specifications: vec![image_to_image_spec()],
            }))
            .await
            .expect("full success must return Ok")
            .into_inner();

        assert!(
            resp.failed.is_empty(),
            "full success must have empty failed, got: {:?}",
            resp.failed
        );
        let published = publisher
            .await_events(1, std::time::Duration::from_secs(2))
            .await;
        assert_eq!(
            published[0].event_type,
            thumbnail_common::GENERATE_EVENT_TYPE
        );

        publisher.shutdown().await;
    }

    #[tokio::test]
    async fn all_dispatches_failed_returns_internal() {
        let publisher =
            MockPublisherServer::start_with(PublisherBehavior::FailWith(tonic::Code::Internal))
                .await;
        let addr = start_fos(StatResp::Ok("enc-id".to_string())).await;

        let result = service_with_storage_and_publisher(addr, &publisher)
            .create_thumbnails(with_auth(CreateThumbnailsRequest {
                resource_address: "https://storage.test/image.png".to_string(),
                specifications: vec![image_to_image_spec()],
            }))
            .await;

        assert_eq!(result.unwrap_err().code(), tonic::Code::Internal);

        publisher.shutdown().await;
    }

    /// Per-specification reporting survives the move to generate publishing:
    /// the caller still learns which specification failed.
    #[tokio::test]
    async fn partial_failure_returns_ok_with_failed_populated() {
        use thumbnail_proto::nvidia::omniverse::thumbnail::v1alpha::thumbnail_specification::Specification;

        // Valid specifications isolate an injected publisher rejection after one
        // successful publish, preserving the partial-dispatch response contract.
        let publisher = MockPublisherServer::start_with(PublisherBehavior::RejectAfter {
            successes: 1,
            reason: "mock publisher: injected partial rejection".to_string(),
        })
        .await;
        let addr = start_fos(StatResp::Ok("enc-id".to_string())).await;

        let second = ImageToImageSpecification {
            processor_identity: 1,
            encoding_format: 2,
            width: 512,
            height: 512,
            transform_type: 1,
            crop_strategy: 1,
            quality: Some(85),
            preserve_aspect_ratio: Some(true),
        };

        let resp = service_with_storage_and_publisher(addr, &publisher)
            .create_thumbnails(with_auth(CreateThumbnailsRequest {
                resource_address: "https://storage.test/image.png".to_string(),
                specifications: vec![
                    image_to_image_spec(),
                    ThumbnailSpecification {
                        specification: Some(Specification::ImageToImage(second)),
                    },
                ],
            }))
            .await
            .expect("a partial failure is still an accepted request")
            .into_inner();

        assert_eq!(
            resp.failed.len(),
            1,
            "one specification should be reported as failed, got: {:?}",
            resp.failed
        );
        assert!(!resp.failed[0].error_message.is_empty());
        assert!(resp.failed[0].specification.is_some());

        // The reported failure is only half the contract. A partial failure that
        // quietly dropped the *good* specification too would satisfy every
        // assertion above while generating nothing.
        let published = publisher
            .await_events(2, std::time::Duration::from_secs(2))
            .await;
        assert_eq!(
            published.len(),
            2,
            "both specifications must be attempted; the second is rejected by the \
             publisher rather than skipped, saw {published:?}"
        );

        publisher.shutdown().await;
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// get_thumbnails
// ─────────────────────────────────────────────────────────────────────────────

mod get_thumbnails_tests {
    use super::*;
    use thumbnail_common::test_fixtures::mock_storage_server::{mock_storage, unique_test_address};
    use thumbnail_proto::nvidia::omniverse::thumbnail::v1alpha::GetThumbnailsRequest;

    /// Seed one association on `target` and return the thumbnail address it points at.
    ///
    /// The read path resolves storage from the installed runtime config rather than
    /// from `OutboundClients`, so seeding the shared double is all a caller needs.
    fn seed_association(target: &str, suffix: &str) -> String {
        let thumbnail_address = format!("https://store.test/{suffix}.png");
        mock_storage().seed(
            target,
            &format!("thumbnail_256x256_PNG_{suffix}"),
            serde_json::json!({
                "thumbnail": {
                    "resource_address": thumbnail_address,
                    "resource_identity": format!("{thumbnail_address}?version=1")
                },
                "association_target": {
                    "resource_address": target,
                    "resource_identity": format!("{target}?version=1")
                },
                "target_type": "FILE",
                "specification": {
                    "type": "IMAGE_TO_IMAGE",
                    "width": 256,
                    "height": 256,
                    "encoding_format": "PNG",
                    "transform_type": "CROP",
                    "crop_strategy": "TOP"
                }
            }),
        );
        thumbnail_address
    }

    fn request_for(target: &str) -> GetThumbnailsRequest {
        GetThumbnailsRequest {
            association_target_resource_address: target.to_string(),
            ..GetThumbnailsRequest::default()
        }
    }

    #[tokio::test]
    async fn missing_address_returns_invalid_argument() {
        let service = test_service();

        for (label, address) in [("empty", ""), ("blank", "   ")] {
            let err = service
                .get_thumbnails(with_auth(request_for(address)))
                .await
                .expect_err("a missing association target is a caller error");

            assert_eq!(err.code(), tonic::Code::InvalidArgument, "{label}");
            assert!(
                err.message()
                    .contains("association_target_resource_address"),
                "{label}: the message must name the missing field, got: {}",
                err.message()
            );
        }
    }

    /// The same `[1, 8192]` range the HTTP query enforces, reached through the
    /// same `validate()`.
    #[tokio::test]
    async fn out_of_range_dimension_filter_returns_invalid_argument() {
        let service = test_service();

        for (field, width, height) in [
            ("width", Some(0), None),
            ("width", Some(8193), None),
            ("height", None, Some(0)),
            ("height", None, Some(8193)),
        ] {
            let err = service
                .get_thumbnails(with_auth(GetThumbnailsRequest {
                    association_target_resource_address: "https://store.test/source.png"
                        .to_string(),
                    width,
                    height,
                    ..GetThumbnailsRequest::default()
                }))
                .await
                .expect_err("an out-of-range filter is a caller error");

            assert_eq!(err.code(), tonic::Code::InvalidArgument, "{field}");
            assert!(
                err.message().contains(field),
                "the message must name the offending field, got: {}",
                err.message()
            );
        }
    }

    #[tokio::test]
    async fn missing_token_returns_unauthenticated() {
        let service = test_service();

        // A blank address as well, so a pass means auth was checked first rather
        // than that there was nothing else to reject.
        let err = service
            .get_thumbnails(Request::new(request_for("")))
            .await
            .expect_err("a request with no token cannot be served");

        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }

    #[tokio::test]
    async fn a_valid_request_returns_the_stored_entries() {
        let service = test_service();
        let target = unique_test_address("grpc-get-target.png");
        let thumbnail_address = seed_association(&target, "stored");

        let response = service
            .get_thumbnails(with_auth(request_for(&target)))
            .await
            .expect("a seeded target must read back");

        let thumbnails = &response.get_ref().thumbnails;
        assert_eq!(thumbnails.len(), 1);
        let entry = &thumbnails[0];
        assert_eq!(
            entry
                .thumbnail
                .as_ref()
                .map(|t| t.resource_address.as_str()),
            Some(thumbnail_address.as_str())
        );
        assert_eq!(
            entry
                .association_target
                .as_ref()
                .map(|t| t.resource_address.as_str()),
            Some(target.as_str())
        );
        assert_eq!(entry.target_type, proto::TargetType::File as i32);
        // The stored specification crosses as a Struct rather than a typed
        // message, so the arbitrary JSON storage holds survives the trip.
        let specification = entry
            .specification
            .as_ref()
            .expect("the seeded entry records a specification");
        assert_eq!(
            specification
                .fields
                .get("transform_type")
                .and_then(|value| match &value.kind {
                    Some(prost_types::value::Kind::StringValue(s)) => Some(s.as_str()),
                    _ => None,
                }),
            Some("CROP")
        );
    }

    /// A target with no thumbnail metadata is a successful read of nothing.
    /// NOT_FOUND would send the caller looking for a resource that is there.
    #[tokio::test]
    async fn a_target_with_no_thumbnails_is_an_empty_success() {
        let service = test_service();
        let target = unique_test_address("grpc-get-empty.png");

        let response = service
            .get_thumbnails(with_auth(request_for(&target)))
            .await
            .expect("no matches is a success, not a failure");

        assert!(response.get_ref().thumbnails.is_empty());
    }

    /// A supplied filter narrows by exact match; an omitted one does not narrow.
    #[tokio::test]
    async fn supplied_filters_narrow_by_exact_match() {
        let service = test_service();
        let target = unique_test_address("grpc-get-filtered.png");
        seed_association(&target, "match");

        let matching = service
            .get_thumbnails(with_auth(GetThumbnailsRequest {
                association_target_resource_address: target.clone(),
                transform_type: Some(proto::TransformType::Crop as i32),
                crop_strategy: Some(proto::CropStrategy::Top as i32),
                ..GetThumbnailsRequest::default()
            }))
            .await
            .expect("a matching filter must still read back");
        assert_eq!(matching.get_ref().thumbnails.len(), 1);

        let narrowed_away = service
            .get_thumbnails(with_auth(GetThumbnailsRequest {
                association_target_resource_address: target.clone(),
                transform_type: Some(proto::TransformType::Fit as i32),
                ..GetThumbnailsRequest::default()
            }))
            .await
            .expect("a filter that matches nothing is still a success");
        assert!(narrowed_away.get_ref().thumbnails.is_empty());
    }

    /// An `UNSPECIFIED` enum on a filter field means "do not narrow", which is
    /// the only reading proto3 leaves available: a caller that sets the zero
    /// variant is indistinguishable from one that sets nothing.
    #[tokio::test]
    async fn unspecified_enum_filters_do_not_narrow() {
        let service = test_service();
        let target = unique_test_address("grpc-get-unspecified.png");
        seed_association(&target, "unspecified");

        let response = service
            .get_thumbnails(with_auth(GetThumbnailsRequest {
                association_target_resource_address: target.clone(),
                specification: Some(proto::ThumbnailSpecificationType::Unspecified as i32),
                encoding_format: Some(proto::ImageEncodingFormat::Unspecified as i32),
                transform_type: Some(proto::TransformType::Unspecified as i32),
                crop_strategy: Some(proto::CropStrategy::Unspecified as i32),
                ..GetThumbnailsRequest::default()
            }))
            .await
            .expect("UNSPECIFIED filters are no filters, not invalid ones");

        assert_eq!(response.get_ref().thumbnails.len(), 1);
    }

    /// A stored specification that is not a JSON object costs its own field, not
    /// the whole read.
    ///
    /// `extract_match_info_from_metadata` forwards whatever JSON sits at
    /// `specification` verbatim, so a legacy record can hold a bare string there.
    /// A `Struct` cannot carry one, and this used to answer `INTERNAL` — which
    /// meant one bad record made every good thumbnail on the same target
    /// unreadable over gRPC while HTTP returned them all, so the two transports
    /// disagreed about whether the target had thumbnails at all.
    #[tokio::test]
    async fn a_non_object_specification_is_omitted_rather_than_failing_the_read() {
        let service = test_service();
        let target = unique_test_address("grpc-get-malformed-spec.png");
        let good_thumbnail = seed_association(&target, "good");

        // The shape this cannot represent: a specification stored as a string.
        mock_storage().seed(
            &target,
            "thumbnail_256x256_PNG_malformed",
            serde_json::json!({
                "thumbnail": {
                    "resource_address": "https://store.test/malformed.png",
                    "resource_identity": "https://store.test/malformed.png?version=1"
                },
                "association_target": {
                    "resource_address": target,
                    "resource_identity": format!("{target}?version=1")
                },
                "target_type": "FILE",
                "specification": "IMAGE_TO_IMAGE"
            }),
        );

        let response = service
            .get_thumbnails(with_auth(request_for(&target)))
            .await
            .expect("one malformed record must not fail the whole read");

        let thumbnails = &response.get_ref().thumbnails;
        assert_eq!(
            thumbnails.len(),
            2,
            "both entries must come back; the malformed one loses only its specification"
        );

        let by_address = |address: &str| {
            thumbnails
                .iter()
                .find(|entry| {
                    entry
                        .thumbnail
                        .as_ref()
                        .is_some_and(|t| t.resource_address == address)
                })
                .expect("entry must be present")
        };

        assert!(
            by_address("https://store.test/malformed.png")
                .specification
                .is_none(),
            "a non-object specification cannot be represented and is omitted"
        );
        assert!(
            by_address(&good_thumbnail).specification.is_some(),
            "the well-formed entry keeps its specification"
        );
    }

    /// The `warn!` on the omission path records the JSON kind and never the value.
    ///
    /// A stored specification can hold a signed address — a legacy record may have
    /// put one there as a bare string, which is the shape this path exists for —
    /// and the read path already declines to log those
    /// (`get_thumbnails_route_does_not_log_signed_resource_addresses`). Without
    /// this, the redaction is a claim in a comment: the line could start
    /// interpolating the value and only the omission assertion would still run.
    ///
    /// Serial and clearing first because the capture harness is one global
    /// subscriber shared with the HTTP tests in the same binary.
    #[tokio::test]
    #[serial_test::serial]
    async fn a_non_object_specification_logs_its_kind_and_never_its_value() {
        let service = test_service();
        let target = unique_test_address("grpc-get-spec-redaction.png");
        let credential = "secret-sas-token";
        mock_storage().seed(
            &target,
            "thumbnail_256x256_PNG_signed-spec",
            serde_json::json!({
                "thumbnail": {
                    "resource_address": "https://store.test/redacted.png",
                    "resource_identity": "https://store.test/redacted.png?version=1"
                },
                "association_target": {
                    "resource_address": target,
                    "resource_identity": format!("{target}?version=1")
                },
                "target_type": "FILE",
                // Not an object, so it takes the omission path, and carrying a
                // credential so a leak is visible rather than theoretical.
                "specification": format!("https://store.test/private/spec.png?sig={credential}")
            }),
        );

        let buffer = crate::http::tests::start_capturing_logs();

        let response = service
            .get_thumbnails(with_auth(request_for(&target)))
            .await
            .expect("the read still succeeds");

        assert_eq!(response.get_ref().thumbnails.len(), 1);
        assert!(
            response.get_ref().thumbnails[0].specification.is_none(),
            "the non-object specification must be omitted"
        );

        let logs = crate::http::tests::captured_logs(&buffer);
        assert!(
            logs.contains("Omitting a stored thumbnail specification"),
            "the omission must be reported so an operator can find the record: {logs}"
        );
        assert!(
            logs.contains("stored_kind=\"string\""),
            "the log must name the JSON kind, which is what says what to look for: {logs}"
        );
        assert!(
            !logs.contains(credential),
            "the stored specification's contents must never reach the logs: {logs}"
        );
    }

    /// A rejected token is UNAUTHENTICATED, and a denial is PERMISSION_DENIED —
    /// the same distinction the HTTP route draws between 401 and 403, because
    /// refreshing a token cannot fix an authorization decision.
    #[tokio::test]
    async fn a_token_storage_rejects_is_unauthenticated() {
        let service = test_service();
        let target = unique_test_address("grpc-get-rejected.png");
        let token = unique_test_address("grpc-get-rejected-token");
        mock_storage().reject_token(&token);

        let mut request = Request::new(request_for(&target));
        request
            .metadata_mut()
            .insert("authorization", format!("Bearer {token}").parse().unwrap());

        let err = service
            .get_thumbnails(request)
            .await
            .expect_err("a rejected token cannot be served");

        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }

    /// A denial is PERMISSION_DENIED, not UNAUTHENTICATED.
    ///
    /// The fourth branch of `get_thumbnails_error_to_status`, and the one whose
    /// distinction is easiest to erase: the token was accepted and the
    /// authorization decision is what failed, so UNAUTHENTICATED would send the
    /// caller to refresh a token that was never the problem. HTTP pins the same
    /// split as 403 vs 401 in
    /// `get_thumbnails_route_maps_storage_authorization_and_outage_failures`.
    #[tokio::test]
    async fn a_storage_denial_is_permission_denied_not_unauthenticated() {
        use thumbnail_common::test_fixtures::mock_storage_server::MetadataOp;

        let service = test_service();
        let target = unique_test_address("grpc-get-denied.png");
        mock_storage().fail(
            MetadataOp::Get,
            &target,
            None,
            tonic::Code::PermissionDenied,
        );

        let err = service
            .get_thumbnails(with_auth(request_for(&target)))
            .await
            .expect_err("a denied read cannot be served");

        assert_eq!(err.code(), tonic::Code::PermissionDenied);
        assert_ne!(
            err.code(),
            tonic::Code::Unauthenticated,
            "a denial must not invite a token refresh that cannot help"
        );
    }

    /// A storage outage is UNAVAILABLE: an upstream fault worth retrying, which
    /// INTERNAL would not say. Mirrors the HTTP route's 502.
    #[tokio::test]
    async fn a_storage_outage_is_unavailable() {
        use thumbnail_common::test_fixtures::mock_storage_server::MetadataOp;

        let service = test_service();
        let target = unique_test_address("grpc-get-outage.png");
        mock_storage().fail(MetadataOp::Get, &target, None, tonic::Code::Unavailable);

        let err = service
            .get_thumbnails(with_auth(request_for(&target)))
            .await
            .expect_err("an unreachable storage service cannot be served");

        assert_eq!(err.code(), tonic::Code::Unavailable);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// reflection
// ─────────────────────────────────────────────────────────────────────────────

mod reflection_tests {
    use super::*;

    /// Both reflection services are built during startup, and building fails if a
    /// registered descriptor set is missing a transitive dependency. Constructing
    /// them here makes that a test failure rather than a pod that never serves.
    #[test]
    fn both_reflection_protocol_versions_construct() {
        reflection_builder()
            .build_v1()
            .expect("v1 reflection should build from the registered descriptor sets");
        reflection_builder()
            .build_v1alpha()
            .expect("v1alpha reflection should build from the registered descriptor sets");
    }
}
