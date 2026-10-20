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

//! Immediate create acceptance and boundary rejection; generation is not polled.
use serde::Deserialize;
use serde_json::{json, Value};

use super::RestClient;
use crate::contract::{CreateCase, VerifiedBound};
use crate::style::{Suite, TestError, TestRunner};

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

#[derive(Debug, Deserialize)]
struct CreateThumbnailsBody {
    failed: Vec<Value>,
}

fn image_to_image(width: u32, height: u32) -> Value {
    json!({"image_to_image": {
        "width": width, "height": height, "encoding_format": "PNG",
        "transform_type": "SQUARE", "crop_strategy": "CENTER"
    }})
}

fn payload(case: CreateCase, source_file: &str, bound: VerifiedBound) -> Value {
    let mut specification = image_to_image(64, 64);
    match case {
        CreateCase::Dimensions => specification["image_to_image"]["width"] = json!(0),
        CreateCase::JpegQuality
        | CreateCase::WebpQuality
        | CreateCase::PngQuality
        | CreateCase::GifQuality => {
            specification["image_to_image"]["encoding_format"] = json!(match case {
                CreateCase::JpegQuality => "JPEG",
                CreateCase::WebpQuality => "WEBP",
                CreateCase::PngQuality => "PNG",
                CreateCase::GifQuality => "GIF",
                _ => unreachable!(),
            });
            specification["image_to_image"]["quality"] = json!(200);
        }
        _ => {}
    }
    json!({
        "resource_address": if case == CreateCase::BlankAddress { "" } else { source_file },
        "specifications": vec![specification; if case == CreateCase::OverBound { bound.over_bound() } else { 1 }]
    })
}

pub async fn run(
    runner: &mut TestRunner,
    rest: &RestClient,
    source_file: &str,
    bound: VerifiedBound,
) {
    for &case in CREATE_CASES {
        runner
            .run(
                Suite::Rest,
                &format!("POST /v1alpha/thumbnails ({})", case.name()),
                || run_case(rest, source_file, bound, case),
            )
            .await;
    }
    for path in ["/health", "/health/live"] {
        runner
            .run(Suite::Rest, &format!("GET {path}"), || {
                rest.check_health(path)
            })
            .await;
    }
}

async fn run_case(
    rest: &RestClient,
    source_file: &str,
    bound: VerifiedBound,
    case: CreateCase,
) -> Result<(), TestError> {
    let context = format!("POST /v1alpha/thumbnails ({})", case.name());
    let response = rest
        .with_auth(
            rest.client
                .post(format!("{}/v1alpha/thumbnails", rest.base_url)),
        )
        .json(&payload(case, source_file, bound))
        .timeout(rest.timeout)
        .send()
        .await
        .map_err(|error| super::classify_request_error(&error, &context))?;
    let status = response.status();
    // Inspect status before decoding the body: even a malformed 2xx can start work.
    if case == CreateCase::OverBound && status.is_success() {
        return Err(TestError::new(format!(
            "{context} returned {status}, expected 400. {}",
            crate::contract::over_bound_acceptance_consequence(bound.over_bound())
        )));
    }
    let body = response
        .text()
        .await
        .map_err(|error| super::classify_request_error(&error, &context))?;
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(super::classify_not_found(&context, &body));
    }
    let expected = if case.accepts() {
        reqwest::StatusCode::ACCEPTED
    } else {
        reqwest::StatusCode::BAD_REQUEST
    };
    if status != expected {
        return Err(super::classify_unexpected_status(
            status,
            &format!("{context}, expected {expected}"),
            &body,
        ));
    }
    if case.accepts() {
        let response: CreateThumbnailsBody = serde_json::from_str(&body).map_err(|error| {
            TestError::new(format!("{context}: invalid acceptance envelope: {error}"))
        })?;
        if !response.failed.is_empty() {
            return Err(TestError::new(format!(
                "{context}: expected empty failed, got {:?}",
                response.failed
            )));
        }
    } else {
        let response: Value = serde_json::from_str(&body).map_err(|error| {
            TestError::new(format!("{context}: invalid ErrorResponse: {error}"))
        })?;
        let message = response["error_message"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| {
                TestError::new(format!(
                    "{context}: ErrorResponse needs a nonempty error_message"
                ))
            })?;
        let field = match case {
            CreateCase::Dimensions => Some("width"),
            CreateCase::JpegQuality | CreateCase::WebpQuality => Some("quality"),
            _ => None,
        };
        if let Some(field) = field {
            if !crate::contract::identifies_first_specification_field(message, field) {
                return Err(TestError::new(format!(
                    "{context}: rejection must identify specification 0 and {field}: {message}"
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_carry_the_values_and_shape_under_test() {
        let bound = crate::contract::verify_advertised_bound(
            &crate::contract::document_with_bound(1000.into()),
        )
        .unwrap();
        assert_eq!(
            payload(CreateCase::OverBound, "source", bound)["specifications"]
                .as_array()
                .unwrap()
                .len(),
            1001
        );
        for (case, format) in [
            (CreateCase::JpegQuality, "JPEG"),
            (CreateCase::WebpQuality, "WEBP"),
            (CreateCase::PngQuality, "PNG"),
            (CreateCase::GifQuality, "GIF"),
        ] {
            let request = payload(case, "source", bound);
            let specification = &request["specifications"][0];
            assert!(specification.get("type").is_none());
            assert_eq!(specification["image_to_image"]["encoding_format"], format);
            assert_eq!(specification["image_to_image"]["quality"], 200);
            assert_eq!(specification["image_to_image"]["width"], 64);
        }
        assert_eq!(
            payload(CreateCase::Dimensions, "source", bound)["specifications"][0]["image_to_image"]
                ["width"],
            0
        );
        assert_eq!(
            payload(CreateCase::BlankAddress, "source", bound)["resource_address"],
            ""
        );
        assert!(
            payload(CreateCase::Valid, "source", bound)["specifications"][0]["image_to_image"]
                .get("quality")
                .is_none()
        );
    }
}

#[cfg(test)]
mod response_tests {
    use super::*;
    use crate::style::ConfigurationFault;
    use std::time::Duration;
    use wiremock::{
        matchers::{method, path},
        Mock, MockServer, ResponseTemplate,
    };

    async fn exercise(
        case: CreateCase,
        status: u16,
        body: &str,
        delay: Duration,
    ) -> Result<(), TestError> {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1alpha/thumbnails"))
            .respond_with(
                ResponseTemplate::new(status)
                    .set_body_string(body)
                    .set_delay(delay),
            )
            .mount(&server)
            .await;
        let client = RestClient::new(&server.uri(), None, Duration::from_millis(50));
        let bound = crate::contract::verify_advertised_bound(
            &crate::contract::document_with_bound(1000.into()),
        )
        .unwrap();
        run_case(&client, "source", bound, case).await
    }

    #[tokio::test]
    async fn all_accepting_over_bound_outcomes_disclose_possible_writes() {
        for status in [200, 201, 202, 204, 206, 299] {
            let error = exercise(
                CreateCase::OverBound,
                status,
                "not a response envelope",
                Duration::ZERO,
            )
            .await
            .unwrap_err();
            assert!(
                error.message.contains("1001")
                    && error.message.contains("associations")
                    && error.message.contains("reads"),
                "{error}"
            );
        }
        assert!(exercise(
            CreateCase::OverBound,
            400,
            r#"{"error_message":"too many"}"#,
            Duration::ZERO
        )
        .await
        .is_ok());
        for (status, body, expected) in [
            (401, "denied", Some(ConfigurationFault::RejectedCredentials)),
            (403, "denied", Some(ConfigurationFault::AccessDenied)),
            (404, "no route", Some(ConfigurationFault::EndpointMismatch)),
            (
                404,
                r#"{"error_message":"missing source"}"#,
                Some(ConfigurationFault::UnresolvableSource),
            ),
            (502, "storage outage", None),
        ] {
            let error = exercise(CreateCase::OverBound, status, body, Duration::ZERO)
                .await
                .unwrap_err();
            assert_eq!(error.configuration_fault, expected);
            assert!(!error.message.contains("associations"), "{error}");
        }
        let error = exercise(CreateCase::OverBound, 202, "{}", Duration::from_secs(1))
            .await
            .unwrap_err();
        assert_eq!(
            error.configuration_fault,
            Some(ConfigurationFault::ClientTimeout)
        );
        assert!(!error.message.contains("associations"));
    }

    #[tokio::test]
    async fn every_create_case_preserves_status_and_timeout_classification() {
        for &case in CREATE_CASES {
            for (status, body, expected) in [
                (401, "denied", Some(ConfigurationFault::RejectedCredentials)),
                (403, "denied", Some(ConfigurationFault::AccessDenied)),
                (404, "no route", Some(ConfigurationFault::EndpointMismatch)),
                (
                    404,
                    r#"{"error_message":"missing source"}"#,
                    Some(ConfigurationFault::UnresolvableSource),
                ),
                (500, "server error", None),
            ] {
                let error = exercise(case, status, body, Duration::ZERO)
                    .await
                    .unwrap_err();
                assert_eq!(
                    error.configuration_fault,
                    expected,
                    "{}: {error}",
                    case.name()
                );
            }
            let error = exercise(case, 202, "{}", Duration::from_secs(1))
                .await
                .unwrap_err();
            assert_eq!(
                error.configuration_fault,
                Some(ConfigurationFault::ClientTimeout)
            );
        }
    }

    #[tokio::test]
    async fn quality_acceptance_fails_if_old_validator_returns_failures() {
        for case in [CreateCase::PngQuality, CreateCase::GifQuality] {
            assert!(exercise(case, 202, r#"{"failed":[]}"#, Duration::ZERO)
                .await
                .is_ok());
            assert!(exercise(
                case,
                202,
                r#"{"failed":[{"error_message":"invalid quality"}]}"#,
                Duration::ZERO
            )
            .await
            .is_err());
            assert!(exercise(
                case,
                502,
                r#"{"error_message":"invalid quality"}"#,
                Duration::ZERO
            )
            .await
            .is_err());
        }
        for case in [CreateCase::JpegQuality, CreateCase::WebpQuality] {
            assert!(exercise(
                case,
                400,
                r#"{"error_message":"specification 0: quality invalid"}"#,
                Duration::ZERO
            )
            .await
            .is_ok());
            assert!(exercise(case, 202, r#"{"failed":[]}"#, Duration::ZERO)
                .await
                .is_err());
        }
    }
}
