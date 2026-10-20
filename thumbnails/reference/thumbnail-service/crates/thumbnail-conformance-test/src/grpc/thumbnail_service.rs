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

//! Test cases for `ThumbnailService`, mirroring `rest::associations` and
//! `rest::thumbnails` one-for-one so the two suites report the same
//! contract-level coverage through different transports.

use tonic::Code;

use thumbnail_proto::nvidia::omniverse::thumbnail::v1alpha::thumbnail_specification::Specification;
use thumbnail_proto::nvidia::omniverse::thumbnail::v1alpha::{
    AssociateThumbnailRequest, CreateThumbnailsRequest, CropStrategy, DisassociateThumbnailRequest,
    ImageEncodingFormat, ImageToImageSpecification, ProcessorIdentity, Resource, TargetType,
    ThumbnailSpecification, TransformType,
};

use super::Client;
use crate::contract::{AssociationRejection, CreateCase, VerifiedBound};
pub(crate) const CREATE_CASES: &[CreateCase] = &[
    CreateCase::Valid,
    CreateCase::Dimensions,
    CreateCase::OverBound,
    CreateCase::BlankAddress,
    CreateCase::JpegQuality,
    CreateCase::WebpQuality,
    CreateCase::PngQuality,
    CreateCase::GifQuality,
];
pub(crate) const ASSOCIATION_REJECTIONS: &[AssociationRejection] = &[
    AssociationRejection::BlankAddress,
    AssociationRejection::MissingSpecification,
];
use crate::fixtures::unique_thumbnail_address;
use crate::style::{Suite, TestError, TestRunner};

pub async fn run(
    runner: &mut TestRunner,
    client: &mut Client,
    source_file: &str,
    bound: VerifiedBound,
) {
    let thumbnail_address = unique_thumbnail_address(source_file, "grpc");

    let associated = runner
        .run(Suite::Grpc, "AssociateThumbnail (success)", || {
            associate(client, source_file, &thumbnail_address)
        })
        .await;

    if associated {
        runner
            .run(Suite::Grpc, "DisassociateThumbnail (success)", || {
                disassociate(client, source_file, &thumbnail_address)
            })
            .await;

        runner
            .run(
                Suite::Grpc,
                "DisassociateThumbnail (idempotent re-delete)",
                || disassociate_idempotent(client, source_file, &thumbnail_address),
            )
            .await;
    } else {
        // Recorded, not omitted. Dropping them left the summary a shorter list
        // than the contract, which reads as a shorter contract -- and the README
        // claims a dependent case is reported as skipped rather than omitted.
        for name in [
            "DisassociateThumbnail (success)",
            "DisassociateThumbnail (idempotent re-delete)",
        ] {
            runner.record_skip(
                Suite::Grpc,
                name,
                "the AssociateThumbnail case did not pass, so there is no association to act on \
                 and these would report a second failure for the first one's cause",
            );
        }
    }

    for &case in ASSOCIATION_REJECTIONS {
        runner
            .run(
                Suite::Grpc,
                &format!("AssociateThumbnail ({})", case.name()),
                || async {
                    match case {
                        AssociationRejection::BlankAddress => {
                            associate_blank_resource_address(client, source_file).await
                        }
                        AssociationRejection::MissingSpecification => {
                            associate_missing_specification(client, source_file).await
                        }
                    }
                },
            )
            .await;
    }
    for &case in CREATE_CASES {
        runner
            .run(
                Suite::Grpc,
                &format!("CreateThumbnails ({})", case.name()),
                || create_case(client, source_file, bound, case),
            )
            .await;
    }
}

/// The baseline specification every case starts from: valid, PNG, and fully
/// populated so nothing is left to be resolved from the deployment's
/// configuration.
///
fn specification() -> ThumbnailSpecification {
    ThumbnailSpecification {
        specification: Some(Specification::ImageToImage(ImageToImageSpecification {
            processor_identity: ProcessorIdentity::ImageToImage as i32,
            encoding_format: ImageEncodingFormat::Png as i32,
            width: 128,
            height: 128,
            transform_type: TransformType::Square as i32,
            crop_strategy: CropStrategy::Center as i32,
            quality: Some(85),
            preserve_aspect_ratio: Some(true),
        })),
    }
}

async fn associate(
    client: &mut Client,
    source_file: &str,
    thumbnail_address: &str,
) -> Result<(), TestError> {
    let request = AssociateThumbnailRequest {
        thumbnail: Some(Resource {
            resource_address: thumbnail_address.to_string(),
            resource_identity: Some(thumbnail_address.to_string()),
        }),
        association_target: Some(Resource {
            resource_address: source_file.to_string(),
            resource_identity: Some(source_file.to_string()),
        }),
        target_type: Some(TargetType::File as i32),
        specification: Some(specification()),
    };

    let response = client
        .associate_thumbnail(request)
        .await
        .map_err(|status| super::classify_unexpected_status(&status, "AssociateThumbnail"))?
        .into_inner();

    if !response.success {
        return Err(TestError::new(format!(
            "AssociateThumbnail returned success=false: {:?}",
            response.error_message
        )));
    }
    Ok(())
}

/// A blank `resource_address` on the association target is documented as
/// `INVALID_ARGUMENT` — the gRPC counterpart of REST's `400`.
///
/// REST has asserted this since the first version and gRPC never had, which the
/// parity audit found alongside REST's missing `specification`. Reported by
/// failing the RPC rather than in-band as `success=false`: request validation
/// precedes the store call, and only a failure from the store itself is in-band.
async fn associate_blank_resource_address(
    client: &mut Client,
    source_file: &str,
) -> Result<(), TestError> {
    let thumbnail_address = unique_thumbnail_address(source_file, "blank-target");
    let request = AssociateThumbnailRequest {
        thumbnail: Some(Resource {
            resource_address: thumbnail_address.clone(),
            resource_identity: Some(thumbnail_address),
        }),
        association_target: Some(Resource {
            resource_address: String::new(),
            resource_identity: Some(source_file.to_string()),
        }),
        target_type: Some(TargetType::File as i32),
        specification: Some(specification()),
    };

    assert_rejected(client, request, "a blank resource_address").await
}

/// Omitting the required specification is documented as INVALID_ARGUMENT.
async fn associate_missing_specification(
    client: &mut Client,
    source_file: &str,
) -> Result<(), TestError> {
    let malformed_address = unique_thumbnail_address(source_file, "missing-specification");
    let request = AssociateThumbnailRequest {
        thumbnail: Some(Resource {
            resource_address: malformed_address.clone(),
            resource_identity: Some(malformed_address),
        }),
        association_target: Some(Resource {
            resource_address: source_file.to_string(),
            resource_identity: Some(source_file.to_string()),
        }),
        target_type: Some(TargetType::File as i32),
        specification: None,
    };

    assert_rejected(client, request, "no specification").await
}

/// Assert the RPC fails with `INVALID_ARGUMENT`.
///
/// Shared by both associate rejection cases for the reason its REST counterpart
/// is: two cases asserting different amounts about the same contract is the
/// failure the parity audit found one level up, between the transports.
async fn assert_rejected(
    client: &mut Client,
    request: AssociateThumbnailRequest,
    sent: &str,
) -> Result<(), TestError> {
    match client.associate_thumbnail(request).await {
        Ok(response) => {
            let response = response.into_inner();
            Err(TestError::new(format!(
                "AssociateThumbnail with {sent} did not fail the RPC (expected \
                 INVALID_ARGUMENT); it returned success={} error_message={:?}. A request refused \
                 by validation fails the RPC — only a failure from the store itself is reported \
                 in-band.",
                response.success, response.error_message
            )))
        }
        Err(status) if status.code() == Code::InvalidArgument => Ok(()),
        Err(status) => Err(super::classify_unexpected_status(
            &status,
            &format!("AssociateThumbnail with {sent} (expected INVALID_ARGUMENT)"),
        )),
    }
}

async fn disassociate(
    client: &mut Client,
    source_file: &str,
    thumbnail_address: &str,
) -> Result<(), TestError> {
    let request = DisassociateThumbnailRequest {
        association_target: Some(Resource {
            resource_address: source_file.to_string(),
            resource_identity: None,
        }),
        target_type: Some(TargetType::File as i32),
        thumbnail: Some(Resource {
            resource_address: thumbnail_address.to_string(),
            resource_identity: None,
        }),
    };

    let response = client
        .disassociate_thumbnail(request)
        .await
        .map_err(|status| super::classify_unexpected_status(&status, "DisassociateThumbnail"))?
        .into_inner();

    if response.succeeded.is_empty() {
        return Err(TestError::new(
            "DisassociateThumbnail returned an empty 'succeeded' for a thumbnail known to be associated",
        ));
    }
    Ok(())
}

/// DisassociateThumbnail is documented as idempotent: removing an
/// already-removed thumbnail returns `OK` with empty arrays, not
/// `NOT_FOUND`.
async fn disassociate_idempotent(
    client: &mut Client,
    source_file: &str,
    thumbnail_address: &str,
) -> Result<(), TestError> {
    let request = DisassociateThumbnailRequest {
        association_target: Some(Resource {
            resource_address: source_file.to_string(),
            resource_identity: None,
        }),
        target_type: Some(TargetType::File as i32),
        thumbnail: Some(Resource {
            resource_address: thumbnail_address.to_string(),
            resource_identity: None,
        }),
    };

    let response = client
        .disassociate_thumbnail(request)
        .await
        .map_err(|status| {
            super::classify_unexpected_status(
                &status,
                "DisassociateThumbnail for an already-removed thumbnail (expected idempotent OK)",
            )
        })?
        .into_inner();

    if !response.succeeded.is_empty() || !response.failed.is_empty() {
        return Err(TestError::new(format!(
            "DisassociateThumbnail for an already-removed thumbnail was not idempotent: \
             succeeded={}, failed={}",
            response.succeeded.len(),
            response.failed.len()
        )));
    }
    Ok(())
}

fn create_request(
    case: CreateCase,
    source_file: &str,
    bound: VerifiedBound,
) -> CreateThumbnailsRequest {
    let mut spec = specification();
    let Some(Specification::ImageToImage(image)) = spec.specification.as_mut() else {
        unreachable!()
    };
    match case {
        CreateCase::Valid => image.quality = None,
        CreateCase::Dimensions => image.width = 0,
        CreateCase::JpegQuality
        | CreateCase::WebpQuality
        | CreateCase::PngQuality
        | CreateCase::GifQuality => {
            image.encoding_format = match case {
                CreateCase::JpegQuality => ImageEncodingFormat::Jpeg,
                CreateCase::WebpQuality => ImageEncodingFormat::Webp,
                CreateCase::PngQuality => ImageEncodingFormat::Png,
                CreateCase::GifQuality => ImageEncodingFormat::Gif,
                _ => unreachable!(),
            } as i32;
            image.quality = Some(200);
        }
        _ => {}
    }
    CreateThumbnailsRequest {
        resource_address: if case == CreateCase::BlankAddress {
            String::new()
        } else {
            source_file.to_string()
        },
        specifications: vec![
            spec;
            if case == CreateCase::OverBound {
                bound.over_bound()
            } else {
                1
            }
        ],
    }
}

async fn create_case(
    client: &mut Client,
    source_file: &str,
    bound: VerifiedBound,
    case: CreateCase,
) -> Result<(), TestError> {
    let context = format!("CreateThumbnails ({})", case.name());
    match client.create_thumbnails(create_request(case, source_file, bound)).await {
        Ok(response) if case.accepts() => {
            let response = response.into_inner();
            if response.failed.is_empty() { Ok(()) } else {
                Err(TestError::new(format!("{context}: expected empty failed, got {:?}", response.failed)))
            }
        }
        Ok(_) => Err(TestError::new(format!("{context}: returned OK, expected INVALID_ARGUMENT. {}",
            if case == CreateCase::OverBound { crate::contract::over_bound_acceptance_consequence(bound.over_bound()) } else { String::new() }))),
        Err(status) if !case.accepts() && status.code() == Code::InvalidArgument => {
            let field = match case {
                CreateCase::Dimensions => Some("width"),
                CreateCase::JpegQuality | CreateCase::WebpQuality => Some("quality"), _ => None,
            };
            if let Some(field) = field {
                if !crate::contract::identifies_first_specification_field(status.message(), field) {
                    return Err(TestError::new(format!("{context}: rejection must identify specification 0 and {field}: {status}")));
                }
            }
            Ok(())
        }
        Err(status) if status.code() == Code::NotFound => Err(TestError::unresolvable_source(format!(
            "{context}: NOT_FOUND; --source-file must name an object that exists in storage: {status}"))),
        Err(status) => Err(super::classify_unexpected_status(&status, &context)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requests_carry_the_quality_and_verified_bound_under_test() {
        let bound = crate::contract::verify_advertised_bound(
            &crate::contract::document_with_bound(1000.into()),
        )
        .unwrap();
        let valid_request = create_request(CreateCase::Valid, "source", bound);
        let Some(Specification::ImageToImage(image)) =
            &valid_request.specifications[0].specification
        else {
            panic!("missing valid image specification")
        };
        assert_eq!(
            image.quality, None,
            "valid create must exercise omitted quality"
        );
        assert_eq!(
            create_request(CreateCase::OverBound, "source", bound)
                .specifications
                .len(),
            1001
        );
        for (case, format) in [
            (CreateCase::JpegQuality, ImageEncodingFormat::Jpeg),
            (CreateCase::WebpQuality, ImageEncodingFormat::Webp),
            (CreateCase::PngQuality, ImageEncodingFormat::Png),
            (CreateCase::GifQuality, ImageEncodingFormat::Gif),
        ] {
            let request = create_request(case, "source", bound);
            let Some(Specification::ImageToImage(image)) = &request.specifications[0].specification
            else {
                panic!("missing image specification")
            };
            assert_eq!(image.quality, Some(200));
            assert_eq!(image.encoding_format, format as i32);
            assert_eq!(image.width, 128);
        }
        assert!(create_request(CreateCase::BlankAddress, "source", bound)
            .resource_address
            .is_empty());
    }
}

#[cfg(test)]
mod rpc_tests {
    use super::*;
    use crate::grpc::{test_server, wait_for_ready};
    use crate::style::ConfigurationFault;
    use std::time::Duration;

    #[tokio::test]
    async fn healthy_protocol_double_passes_all_cases_and_old_quality_rule_fails() {
        let bound = crate::contract::verify_advertised_bound(
            &crate::contract::document_with_bound(1000.into()),
        )
        .unwrap();
        let server =
            test_server::start(test_server::FakeService::default(), Some(Duration::ZERO)).await;
        let mut client = wait_for_ready(
            &server.url,
            false,
            None,
            Duration::from_secs(1),
            1,
            Duration::ZERO,
        )
        .await
        .unwrap();
        let mut runner = TestRunner::new();
        run(&mut runner, &mut client, "source", bound).await;
        assert!(runner.all_passed(), "{}", runner.summary_lines().join("\n"));
        let server = test_server::start(
            test_server::FakeService {
                reject_png_quality: true,
                ..Default::default()
            },
            Some(Duration::ZERO),
        )
        .await;
        let mut client = wait_for_ready(
            &server.url,
            false,
            None,
            Duration::from_secs(1),
            1,
            Duration::ZERO,
        )
        .await
        .unwrap();
        for case in [CreateCase::PngQuality, CreateCase::GifQuality] {
            assert!(create_case(&mut client, "source", bound, case)
                .await
                .is_err());
        }
    }

    #[tokio::test]
    async fn every_rpc_call_site_preserves_error_classification() {
        let bound = crate::contract::verify_advertised_bound(
            &crate::contract::document_with_bound(1000.into()),
        )
        .unwrap();
        for (code, delay, expected) in [
            (
                Some(Code::Unauthenticated),
                Duration::ZERO,
                Some(ConfigurationFault::RejectedCredentials),
            ),
            (
                Some(Code::PermissionDenied),
                Duration::ZERO,
                Some(ConfigurationFault::AccessDenied),
            ),
            (
                Some(Code::Unimplemented),
                Duration::ZERO,
                Some(ConfigurationFault::EndpointMismatch),
            ),
            (Some(Code::Internal), Duration::ZERO, None),
            (Some(Code::Cancelled), Duration::ZERO, None),
            (
                None,
                Duration::from_secs(1),
                Some(ConfigurationFault::ClientTimeout),
            ),
        ] {
            let server = test_server::start(
                test_server::FakeService {
                    code,
                    delay,
                    ..Default::default()
                },
                Some(Duration::ZERO),
            )
            .await;
            let mut client = wait_for_ready(
                &server.url,
                false,
                None,
                Duration::from_secs(1),
                1,
                Duration::ZERO,
            )
            .await
            .unwrap();
            client.timeout = Duration::from_millis(50);
            for case in CREATE_CASES {
                assert_eq!(
                    create_case(&mut client, "source", bound, *case)
                        .await
                        .unwrap_err()
                        .configuration_fault,
                    expected,
                    "{}",
                    case.name()
                );
            }
            assert_eq!(
                associate(&mut client, "source", "thumbnail")
                    .await
                    .unwrap_err()
                    .configuration_fault,
                expected
            );
            assert_eq!(
                associate_blank_resource_address(&mut client, "source")
                    .await
                    .unwrap_err()
                    .configuration_fault,
                expected
            );
            assert_eq!(
                associate_missing_specification(&mut client, "source")
                    .await
                    .unwrap_err()
                    .configuration_fault,
                expected
            );
            assert_eq!(
                disassociate(&mut client, "source", "thumbnail")
                    .await
                    .unwrap_err()
                    .configuration_fault,
                expected
            );
            assert_eq!(
                disassociate_idempotent(&mut client, "source", "thumbnail")
                    .await
                    .unwrap_err()
                    .configuration_fault,
                expected
            );
        }
    }
}

#[cfg(test)]
mod fixture_tests {
    use super::*;
    use crate::grpc::{test_server, wait_for_ready};
    use std::time::Duration;
    #[tokio::test]
    async fn rejection_addresses_are_unique_across_cases_and_runs() {
        let service = test_server::FakeService::default();
        let seen = service.thumbnails_seen.clone();
        let server = test_server::start(service, Some(Duration::ZERO)).await;
        let mut client = wait_for_ready(
            &server.url,
            false,
            None,
            Duration::from_secs(1),
            1,
            Duration::ZERO,
        )
        .await
        .unwrap();
        for _ in 0..2 {
            associate_blank_resource_address(&mut client, "source")
                .await
                .unwrap();
            associate_missing_specification(&mut client, "source")
                .await
                .unwrap();
        }
        let seen = seen.lock().unwrap();
        let unique: std::collections::BTreeSet<_> = seen.iter().collect();
        assert_eq!(unique.len(), 4);
        assert!(unique
            .iter()
            .all(|address| address.starts_with("source.conformance-test-")));
    }
}
