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

//! `POST/DELETE /v1alpha/associations` and the `GET /v1alpha/thumbnails`
//! list-read path they feed.

use serde::Deserialize;
use serde_json::json;

use super::RestClient;
use crate::contract::AssociationRejection;
pub(crate) const ASSOCIATION_REJECTIONS: &[AssociationRejection] = &[
    AssociationRejection::BlankAddress,
    AssociationRejection::MissingSpecification,
];
use crate::fixtures::unique_thumbnail_address;
use crate::style::{Suite, TestError, TestRunner};

#[derive(Debug, Deserialize)]
struct ErrorBody {
    error_message: String,
}

#[derive(Debug, Deserialize)]
struct DisassociateBody {
    succeeded: Vec<serde_json::Value>,
    failed: Vec<serde_json::Value>,
}

pub async fn run(runner: &mut TestRunner, rest: &RestClient, source_file: &str) {
    let thumbnail_address = unique_thumbnail_address(source_file, "assoc");

    let associated = runner
        .run(Suite::Rest, "POST /v1alpha/associations (success)", || {
            associate(rest, source_file, &thumbnail_address)
        })
        .await;

    // The list/disassociate assertions all depend on the associate call
    // above having actually created something to observe — running them
    // against a fixture that was never created would just report a second,
    // misleading failure on top of the first.
    if associated {
        runner
            .run(
                Suite::Rest,
                "GET /v1alpha/thumbnails (contains association)",
                || list_contains(rest, source_file, &thumbnail_address),
            )
            .await;

        runner
            .run(
                Suite::Rest,
                "DELETE /v1alpha/associations (success)",
                || disassociate(rest, source_file, &thumbnail_address),
            )
            .await;

        runner
            .run(
                Suite::Rest,
                "GET /v1alpha/thumbnails (absent after disassociate)",
                || list_absent(rest, source_file, &thumbnail_address),
            )
            .await;

        runner
            .run(
                Suite::Rest,
                "DELETE /v1alpha/associations (idempotent re-delete)",
                || disassociate_idempotent(rest, source_file, &thumbnail_address),
            )
            .await;
    } else {
        // See the gRPC twin: recorded rather than omitted, so the summary lists
        // every case the contract has.
        for name in [
            "GET /v1alpha/thumbnails (contains association)",
            "DELETE /v1alpha/associations (success)",
            "GET /v1alpha/thumbnails (absent after disassociate)",
            "DELETE /v1alpha/associations (idempotent re-delete)",
        ] {
            runner.record_skip(
                Suite::Rest,
                name,
                "the POST /v1alpha/associations case did not pass, so there is no association to \
                 act on and these would report a second failure for the first one's cause",
            );
        }
    }

    for &case in ASSOCIATION_REJECTIONS {
        runner
            .run(
                Suite::Rest,
                &format!("POST /v1alpha/associations ({})", case.name()),
                || async {
                    match case {
                        AssociationRejection::BlankAddress => {
                            associate_blank_resource_address(rest, source_file).await
                        }
                        AssociationRejection::MissingSpecification => {
                            associate_missing_specification(rest, source_file).await
                        }
                    }
                },
            )
            .await;
    }
}

fn association_payload(source_file: &str, thumbnail_address: &str) -> serde_json::Value {
    json!({
        "target_type": "FILE",
        "thumbnail": {
            "resource_address": thumbnail_address,
            "resource_identity": thumbnail_address
        },
        "association_target": {
            "resource_address": source_file,
            "resource_identity": source_file
        },
        "specification": {
            "type": "IMAGE_TO_IMAGE",
            "width": 128,
            "height": 128,
            "encoding_format": "PNG",
            "transform_type": "SQUARE",
            "crop_strategy": "CENTER",
            "quality": 85,
            "preserve_aspect_ratio": true
        }
    })
}

async fn associate(
    rest: &RestClient,
    source_file: &str,
    thumbnail_address: &str,
) -> Result<(), TestError> {
    let url = format!("{}/v1alpha/associations", rest.base_url);
    let payload = association_payload(source_file, thumbnail_address);

    let response = rest
        .with_auth(rest.client.post(&url))
        .json(&payload)
        .timeout(rest.timeout)
        .send()
        .await
        .map_err(|e| super::classify_request_error(&e, "POST /v1alpha/associations failed"))?;

    let status = response.status();
    if status != reqwest::StatusCode::NO_CONTENT {
        let body = response
            .text()
            .await
            .map_err(|error| super::classify_request_error(&error, "read response body"))?;
        return Err(super::classify_unexpected_status(
            status,
            "POST /v1alpha/associations (expected 204 No Content)",
            &body,
        ));
    }
    Ok(())
}

/// A blank `resource_address` on the association target is documented as
/// invalid — see AGENTS.md's request-validation table.
async fn associate_blank_resource_address(
    rest: &RestClient,
    source_file: &str,
) -> Result<(), TestError> {
    let mut payload = association_payload(
        source_file,
        &unique_thumbnail_address(source_file, "blank-target"),
    );
    payload["association_target"]["resource_address"] = json!("");

    assert_rejected(rest, &payload, "a blank resource_address").await
}

/// An omitted `specification` is documented as invalid.
///
/// The gRPC suite has asserted its `INVALID_ARGUMENT` counterpart since the
/// first version and REST never has, which the parity audit found alongside
/// `CreateThumbnails`'s blank address. `specification` is `#[serde(default)]` on
/// the request type, so omitting it deserializes cleanly to `None` and is
/// refused by `validate` rather than by the JSON layer — the same rejection the
/// proto reaches through an unset oneof, not a parse error dressed up as one.
async fn associate_missing_specification(
    rest: &RestClient,
    source_file: &str,
) -> Result<(), TestError> {
    let mut payload = association_payload(
        source_file,
        &unique_thumbnail_address(source_file, "no-spec"),
    );
    payload
        .as_object_mut()
        .expect("the association payload is a JSON object")
        .remove("specification");

    assert_rejected(rest, &payload, "no specification").await
}

/// Assert `400` plus the documented `ErrorResponse` shape.
///
/// Shared by both associate rejection cases so they cannot drift into asserting
/// different amounts about the same contract — which is the failure the parity
/// audit found between the two transports, one level up.
async fn assert_rejected(
    rest: &RestClient,
    payload: &serde_json::Value,
    sent: &str,
) -> Result<(), TestError> {
    let url = format!("{}/v1alpha/associations", rest.base_url);

    let response = rest
        .with_auth(rest.client.post(&url))
        .json(payload)
        .timeout(rest.timeout)
        .send()
        .await
        .map_err(|e| super::classify_request_error(&e, "POST /v1alpha/associations failed"))?;

    let status = response.status();
    if status != reqwest::StatusCode::BAD_REQUEST {
        let body = response
            .text()
            .await
            .map_err(|error| super::classify_request_error(&error, "read response body"))?;
        return Err(super::classify_unexpected_status(
            status,
            &format!("POST /v1alpha/associations with {sent} (expected 400 Bad Request)"),
            &body,
        ));
    }

    let body: ErrorBody = response
        .json()
        .await
        .map_err(|e| super::classify_request_error(&e, "failed to parse 400 error body"))?;
    if body.error_message.trim().is_empty() {
        return Err(TestError::new(
            "400 response's error_message was empty; the documented ErrorResponse shape requires it",
        ));
    }
    Ok(())
}

async fn list(rest: &RestClient, source_file: &str) -> Result<Vec<serde_json::Value>, TestError> {
    let url = format!("{}/v1alpha/thumbnails", rest.base_url);
    let response = rest
        .with_auth(rest.client.get(&url))
        .query(&[("association_target_resource_address", source_file)])
        .timeout(rest.timeout)
        .send()
        .await
        .map_err(|e| super::classify_request_error(&e, "GET /v1alpha/thumbnails failed"))?;

    let status = response.status();
    if !status.is_success() {
        let body = response
            .text()
            .await
            .map_err(|error| super::classify_request_error(&error, "read response body"))?;
        return Err(super::classify_unexpected_status(
            status,
            "GET /v1alpha/thumbnails",
            &body,
        ));
    }

    let body: serde_json::Value = response.json().await.map_err(|e| {
        super::classify_request_error(&e, "failed to parse GET /v1alpha/thumbnails response")
    })?;

    body.get("thumbnails")
        .and_then(|v| v.as_array())
        .cloned()
        .ok_or_else(|| {
            TestError::new("GET /v1alpha/thumbnails response is missing the 'thumbnails' array")
        })
}

fn contains_thumbnail(thumbnails: &[serde_json::Value], thumbnail_address: &str) -> bool {
    thumbnails.iter().any(|t| {
        t.get("thumbnail")
            .and_then(|th| th.get("resource_address"))
            .and_then(|v| v.as_str())
            == Some(thumbnail_address)
    })
}

async fn list_contains(
    rest: &RestClient,
    source_file: &str,
    thumbnail_address: &str,
) -> Result<(), TestError> {
    let thumbnails = list(rest, source_file).await?;
    if !contains_thumbnail(&thumbnails, thumbnail_address) {
        return Err(TestError::new(format!(
            "GET /v1alpha/thumbnails did not return the associated thumbnail {thumbnail_address} \
             ({} entries returned)",
            thumbnails.len()
        )));
    }
    Ok(())
}

async fn list_absent(
    rest: &RestClient,
    source_file: &str,
    thumbnail_address: &str,
) -> Result<(), TestError> {
    let thumbnails = list(rest, source_file).await?;
    if contains_thumbnail(&thumbnails, thumbnail_address) {
        return Err(TestError::new(format!(
            "GET /v1alpha/thumbnails still returned {thumbnail_address} after it was disassociated"
        )));
    }
    Ok(())
}

fn disassociate_request<'a>(
    rest: &'a RestClient,
    source_file: &'a str,
    thumbnail_address: &'a str,
) -> reqwest::RequestBuilder {
    let url = format!("{}/v1alpha/associations", rest.base_url);
    rest.with_auth(rest.client.delete(url)).query(&[
        ("target_type", "FILE"),
        ("association_target_resource_address", source_file),
        ("thumbnail_resource_address", thumbnail_address),
    ])
}

async fn disassociate(
    rest: &RestClient,
    source_file: &str,
    thumbnail_address: &str,
) -> Result<(), TestError> {
    let response = disassociate_request(rest, source_file, thumbnail_address)
        .timeout(rest.timeout)
        .send()
        .await
        .map_err(|e| super::classify_request_error(&e, "DELETE /v1alpha/associations failed"))?;

    let status = response.status();
    if status != reqwest::StatusCode::OK {
        let body = response
            .text()
            .await
            .map_err(|error| super::classify_request_error(&error, "read response body"))?;
        return Err(super::classify_unexpected_status(
            status,
            "DELETE /v1alpha/associations (expected 200 OK)",
            &body,
        ));
    }

    let body: DisassociateBody = response
        .json()
        .await
        .map_err(|e| super::classify_request_error(&e, "failed to parse DELETE response"))?;
    if body.succeeded.is_empty() {
        return Err(TestError::new(
            "DELETE /v1alpha/associations returned 200 but an empty 'succeeded' for a thumbnail \
             known to be associated",
        ));
    }
    Ok(())
}

/// DisassociateThumbnail is documented as idempotent: removing an association
/// that no longer exists succeeds with empty `succeeded`/`failed` arrays, not
/// a not-found error.
async fn disassociate_idempotent(
    rest: &RestClient,
    source_file: &str,
    thumbnail_address: &str,
) -> Result<(), TestError> {
    let response = disassociate_request(rest, source_file, thumbnail_address)
        .timeout(rest.timeout)
        .send()
        .await
        .map_err(|e| super::classify_request_error(&e, "DELETE /v1alpha/associations failed"))?;

    let status = response.status();
    if status != reqwest::StatusCode::OK {
        let body = response
            .text()
            .await
            .map_err(|error| super::classify_request_error(&error, "read response body"))?;
        return Err(super::classify_unexpected_status(
            status,
            "DELETE /v1alpha/associations for an already-removed thumbnail (expected idempotent \
             200 OK)",
            &body,
        ));
    }

    let body: DisassociateBody = response
        .json()
        .await
        .map_err(|e| super::classify_request_error(&e, "failed to parse DELETE response"))?;
    if !body.succeeded.is_empty() || !body.failed.is_empty() {
        return Err(TestError::new(format!(
            "DELETE /v1alpha/associations for an already-removed thumbnail was not idempotent: \
             succeeded={}, failed={}",
            body.succeeded.len(),
            body.failed.len()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::ConfigurationFault;
    use std::time::Duration;
    use wiremock::{matchers::any, Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn rejection_addresses_are_unique_across_cases_and_runs() {
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_json(json!({"error_message":"invalid request"})),
            )
            .mount(&server)
            .await;
        let rest = RestClient::new(&server.uri(), None, Duration::from_secs(1));
        for _ in 0..2 {
            associate_blank_resource_address(&rest, "file-storage://fixture")
                .await
                .unwrap();
            associate_missing_specification(&rest, "file-storage://fixture")
                .await
                .unwrap();
        }
        let requests = server.received_requests().await.unwrap();
        let addresses: std::collections::BTreeSet<_> = requests
            .iter()
            .map(|request| {
                request.body_json::<serde_json::Value>().unwrap()["thumbnail"]["resource_address"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert_eq!(addresses.len(), 4);
        assert!(addresses
            .iter()
            .all(|address| address.starts_with("file-storage://fixture.conformance-test-")));
    }

    #[tokio::test]
    async fn all_association_call_sites_preserve_run_fault_classification() {
        for (status, delay, expected) in [
            (
                401,
                Duration::ZERO,
                Some(ConfigurationFault::RejectedCredentials),
            ),
            (403, Duration::ZERO, Some(ConfigurationFault::AccessDenied)),
            (
                404,
                Duration::ZERO,
                Some(ConfigurationFault::EndpointMismatch),
            ),
            (500, Duration::ZERO, None),
            (
                200,
                Duration::from_secs(1),
                Some(ConfigurationFault::ClientTimeout),
            ),
        ] {
            let server = MockServer::start().await;
            Mock::given(any())
                .respond_with(
                    ResponseTemplate::new(status)
                        .set_body_string("injected")
                        .set_delay(delay),
                )
                .mount(&server)
                .await;
            let rest = RestClient::new(&server.uri(), None, Duration::from_millis(50));
            assert_eq!(
                associate(&rest, "source", "thumbnail")
                    .await
                    .unwrap_err()
                    .configuration_fault,
                expected
            );
            assert_eq!(
                associate_blank_resource_address(&rest, "source")
                    .await
                    .unwrap_err()
                    .configuration_fault,
                expected
            );
            assert_eq!(
                associate_missing_specification(&rest, "source")
                    .await
                    .unwrap_err()
                    .configuration_fault,
                expected
            );
            assert_eq!(
                disassociate(&rest, "source", "thumbnail")
                    .await
                    .unwrap_err()
                    .configuration_fault,
                expected
            );
            assert_eq!(
                disassociate_idempotent(&rest, "source", "thumbnail")
                    .await
                    .unwrap_err()
                    .configuration_fault,
                expected
            );
            assert_eq!(
                list_contains(&rest, "source", "thumbnail")
                    .await
                    .unwrap_err()
                    .configuration_fault,
                expected
            );
            assert_eq!(
                list_absent(&rest, "source", "thumbnail")
                    .await
                    .unwrap_err()
                    .configuration_fault,
                expected
            );
        }
    }
}
