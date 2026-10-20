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
use axum::{
    body::{to_bytes, Body},
    extract::{Query, State},
    http::Request,
    response::{IntoResponse, Json},
    routing::{get, post},
    Router,
};
use std::sync::{Arc, Mutex};
use thumbnail_common::{
    storage::association::THUMBNAIL_METADATA_KEY, test_fixtures::mock_storage_server::mock_storage,
    AssociationResource,
};
use tower::ServiceExt;

#[derive(Clone, Default)]
struct TestLogWriter(Arc<Mutex<Vec<u8>>>);

/// Log capture for the whole test binary, installed once.
///
/// Global rather than thread-local. `tracing::subscriber::set_default` installs
/// on the calling thread only, so an event emitted from a tonic worker or a
/// spawned task reaches no subscriber at all and the assertion sees nothing —
/// intermittently, depending on where the handler's future happened to be polled.
/// CI caught exactly that on this branch (`rust:test`, pipeline 65551596), and
/// `CLAUDE.md` names this shape as the known flake source in this repository.
///
/// A global default can only be installed once per process, so the buffer is
/// shared and every test that reads it must be `#[serial_test::serial]` and must
/// clear it first. That is the cost of capturing across threads; the alternative
/// is a test that passes when the scheduler is kind.
static CAPTURED_LOGS: std::sync::OnceLock<Arc<Mutex<Vec<u8>>>> = std::sync::OnceLock::new();

/// Clear the shared buffer and hand back a handle for reading it afterwards.
/// `pub(crate)` so the gRPC tests can assert on redaction too. Both test modules
/// compile into one binary and share this single global subscriber, which is why
/// every reader must be `#[serial_test::serial]` regardless of which module it
/// lives in.
pub(crate) fn start_capturing_logs() -> Arc<Mutex<Vec<u8>>> {
    let buffer = CAPTURED_LOGS
        .get_or_init(|| {
            let buffer = Arc::new(Mutex::new(Vec::new()));
            let subscriber = tracing_subscriber::fmt()
                .without_time()
                .with_ansi(false)
                .with_max_level(tracing::Level::INFO)
                .with_writer(TestLogWriter(buffer.clone()))
                .finish();
            // A failure here means something else already installed a global
            // default. Capture would then be impossible rather than flaky, so the
            // reading tests below would fail loudly instead of intermittently.
            let _ = tracing::subscriber::set_global_default(subscriber);
            buffer
        })
        .clone();

    buffer.lock().expect("log buffer").clear();
    buffer
}

pub(crate) fn captured_logs(buffer: &Arc<Mutex<Vec<u8>>>) -> String {
    String::from_utf8(buffer.lock().expect("log buffer").clone()).expect("UTF-8 logs")
}

struct TestLogGuard(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for TestLogGuard {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("log buffer").write(bytes)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'writer> tracing_subscriber::fmt::MakeWriter<'writer> for TestLogWriter {
    type Writer = TestLogGuard;

    fn make_writer(&'writer self) -> Self::Writer {
        TestLogGuard(self.0.clone())
    }
}

/// A fully-populated valid spec for use in tests that check resource validation
/// rather than spec validation.
fn valid_spec() -> AssociationSpec {
    serde_json::from_value(serde_json::json!({
        "type": "IMAGE_TO_IMAGE",
        "width": 256,
        "height": 256,
        "encoding_format": "PNG",
        "transform_type": "SQUARE",
        "crop_strategy": "CENTER",
        "quality": 85,
        "preserve_aspect_ratio": true
    }))
    .expect("valid association specification")
}

fn association_file_resource(address: &str) -> AssociationResource {
    AssociationResource {
        resource_address: address.to_string(),
        resource_identity: Some(address.to_string()),
    }
}

fn association_folder_resource(address: &str) -> AssociationResource {
    AssociationResource {
        resource_address: address.to_string(),
        resource_identity: None,
    }
}

pub(crate) fn test_app_state() -> AppState {
    AppState {
        outbound: Arc::new(thumbnail_common::OutboundClients::for_tests()),
        runtime: thumbnail_common::test_fixtures::init_test_runtime_config(
            "thumbnail-service-http-tests",
        ),
        grpc_healthy: Arc::new(std::sync::atomic::AtomicBool::new(true)),
    }
}

/// Extracts `error_message` and `source` from an `ErrorResponse` JSON body.
/// Every error path in this module SHALL return both fields (see the
/// `unified-service-ops-parity` capability's response-consistency
/// requirement), so every test below checks both rather than just the
/// message.
async fn error_message_and_source(response: Response) -> (String, String) {
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("failed to read response body");
    let body_json: serde_json::Value =
        serde_json::from_slice(&body).expect("response body should be valid JSON");
    let error_message = body_json
        .get("error_message")
        .and_then(serde_json::Value::as_str)
        .expect("error_message should be present and a string")
        .to_string();
    let source = body_json
        .get("source")
        .and_then(serde_json::Value::as_str)
        .expect("source should be present and a string")
        .to_string();
    (error_message, source)
}

/// Every endpoint documents exactly the statuses it can produce.
///
/// Both directions of drift had happened and neither was visible from the
/// annotations alone. `GET /v1alpha/thumbnails` documented a `500` that no path
/// produced, `POST /v1alpha/associations` documented a `404` that nothing in the
/// file could return, and `POST /v1alpha/thumbnails` omitted the `403` that
/// `create_thumbnails_error_status` maps `PermissionDenied` to — the worst of the
/// three, since a caller handling the documented set met an undocumented status.
///
/// An exact set, not a `contains_key` sweep. The check this replaces could only
/// catch a status that went missing, never one advertised and unreachable, which
/// is how the `500` on the read path survived. The same argument already retired a
/// documented `429`: nothing in the service, the chart, or the Contour HTTPProxy
/// enforces a rate limit, so it was a status no caller could receive. The drift
/// job regenerates the published document from these annotations and fails on a
/// difference, so an inaccurate one is enforced into the contract rather than
/// drifting out of it.
///
/// A golden list rather than something derived: the reachable set comes from a
/// handler's early returns plus its error mapping, which no test can compute. What
/// this does is make an annotation edit deliberate, and it names the mapping each
/// entry answers to so the next reader can check it.
#[test]
fn documented_statuses_match_what_the_handlers_can_return() {
    let document = serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI should serialize");

    // (path, method, reachable statuses, what produces the non-obvious ones)
    let expected: &[(&str, &str, &[&str])] = &[
        // 200 empty covers both "no metadata" and "no match"; NotFound never
        // surfaces as 404. 401/403/502 are the three arms of the error match.
        (
            "/v1alpha/thumbnails",
            "get",
            &["200", "400", "401", "403", "502"],
        ),
        // create_thumbnails_error_status: InvalidArgument/NotFound/Unauthenticated/
        // PermissionDenied/AllTasksFailed -> 400/404/401/403/502.
        (
            "/v1alpha/thumbnails",
            "post",
            &["202", "400", "401", "403", "404", "502"],
        ),
        // associate_error_status has two arms only: Unauthenticated -> 401 and
        // Internal -> 500. Nothing on this route returns 404.
        (
            "/v1alpha/associations",
            "post",
            &["204", "400", "401", "500"],
        ),
        // disassociate_http_status: 400 | 401 | 502 | 200.
        (
            "/v1alpha/associations",
            "delete",
            &["200", "400", "401", "502"],
        ),
        ("/health", "get", &["200", "503"]),
        ("/health/live", "get", &["200"]),
    ];

    for (path, method, statuses) in expected {
        let documented = document["paths"][path][method]["responses"]
            .as_object()
            .unwrap_or_else(|| panic!("{method} {path} should declare responses"));
        let mut documented: Vec<&str> = documented.keys().map(String::as_str).collect();
        documented.sort_unstable();
        let mut want = statuses.to_vec();
        want.sort_unstable();

        assert_eq!(
            documented, want,
            "{method} {path}: documented statuses must match the ones the handler can return"
        );
    }
}

async fn get_thumbnails_query(query: &str) -> Response {
    get_thumbnails_query_as(query, "test-token").await
}

async fn get_thumbnails_query_as(query: &str, token: &str) -> Response {
    let _ = test_app_state();
    Router::new()
        .route("/v1alpha/thumbnails", get(get_thumbnails))
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1alpha/thumbnails?{query}"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("router response")
}

async fn get_thumbnails_query_without_auth(query: &str) -> Response {
    let _ = test_app_state();
    Router::new()
        .route("/v1alpha/thumbnails", get(get_thumbnails))
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!("/v1alpha/thumbnails?{query}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("router response")
}

async fn response_json(response: Response) -> serde_json::Value {
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("failed to read response body");
    serde_json::from_slice(&body).expect("response body should be valid JSON")
}

#[tokio::test]
async fn get_thumbnails_route_checks_auth_before_parsing_query() {
    let response =
        get_thumbnails_query_without_auth("association_target_resource_address=source&width=wide")
            .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let (error_message, source) = error_message_and_source(response).await;
    assert_eq!(
        error_message,
        "Missing Authorization header. Include 'Authorization: Bearer <your-token>' in request headers"
    );
    assert_eq!(source, "thumbnail-service");
}

#[tokio::test]
async fn get_thumbnails_route_returns_json_for_malformed_query_values() {
    let response = get_thumbnails_query(
        "association_target_resource_address=https%3A%2F%2Fstore.test%2Fsource.png&width=wide",
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("application/json")
    );
    let (error_message, source) = error_message_and_source(response).await;
    assert!(error_message.contains("invalid query"));
    assert_eq!(source, "thumbnail-service");
}

#[tokio::test]
async fn get_thumbnails_route_enforces_documented_query_constraints() {
    let oversized_address = "a".repeat(2049);
    let cases = [
        (
            "blank address",
            "association_target_resource_address=%20%20%20".to_string(),
        ),
        (
            "oversized address",
            format!("association_target_resource_address={oversized_address}"),
        ),
        (
            "zero width",
            "association_target_resource_address=source&width=0".to_string(),
        ),
        (
            "oversized height",
            "association_target_resource_address=source&height=8193".to_string(),
        ),
    ];

    for (label, query) in cases {
        let response = get_thumbnails_query(&query).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{label}");
        let (error_message, source) = error_message_and_source(response).await;
        assert!(
            error_message.contains("invalid query"),
            "{label}: {error_message}"
        );
        assert_eq!(source, "thumbnail-service", "{label}");
    }
}

#[tokio::test]
async fn get_thumbnails_route_rejects_unsupported_filter_types() {
    for filter in [
        "specification=VIDEO_TO_IMAGE",
        "frame_select_strategy=AUTO",
        "timecode=00%3A00%3A01",
        "start_timecode=00%3A00%3A01",
        "end_timecode=00%3A00%3A05",
        "playback_rate=1.5",
        "stage_name=World",
        "render_product=Beauty",
        "frame_count=12",
        "video_format=MP4_H264",
    ] {
        let query = format!("association_target_resource_address=source&{filter}");
        let response = get_thumbnails_query(&query).await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{filter}");
        let (error_message, source) = error_message_and_source(response).await;
        assert!(error_message.contains("invalid query"), "{filter}");
        assert_eq!(source, "thumbnail-service", "{filter}");
    }
}

#[tokio::test]
async fn get_thumbnails_route_filters_image_to_image_transform_and_crop() {
    use thumbnail_common::test_fixtures::mock_storage_server::unique_test_address;

    let _ = test_app_state();
    let target = unique_test_address("get-filter-target.png");
    for (suffix, transform_type, crop_strategy) in
        [("match", "CROP", "TOP"), ("other", "FIT", "CENTER")]
    {
        mock_storage().seed(
            &target,
            &format!("thumbnail_256x256_PNG_{suffix}"),
            serde_json::json!({
                "thumbnail": {
                    "resource_address": format!("https://store.test/{suffix}.png"),
                    "resource_identity": format!("https://store.test/{suffix}.png?version=1")
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
                    "transform_type": transform_type,
                    "crop_strategy": crop_strategy
                }
            }),
        );
    }

    let query = serde_urlencoded::to_string([
        ("association_target_resource_address", target.as_str()),
        ("specification", "IMAGE_TO_IMAGE"),
        ("transform_type", "CROP"),
        ("crop_strategy", "TOP"),
    ])
    .expect("query");
    let response = get_thumbnails_query(&query).await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    let thumbnails = body["thumbnails"].as_array().expect("thumbnails array");
    assert_eq!(thumbnails.len(), 1);
    assert_eq!(
        thumbnails[0]["thumbnail"]["resource_address"],
        "https://store.test/match.png"
    );
}

/// Rewrite every number in `value` as a double, recursively.
///
/// `serde_json` distinguishes `Number(256)` from `Number(256.0)`;
/// `google.protobuf.Struct` cannot, so a comparison across the two encodings has
/// to agree on one of them.
fn numbers_as_doubles(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Number(number) => number
            .as_f64()
            .and_then(serde_json::Number::from_f64)
            .map(serde_json::Value::Number)
            .unwrap_or_else(|| value.clone()),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(numbers_as_doubles).collect())
        }
        serde_json::Value::Object(fields) => serde_json::Value::Object(
            fields
                .iter()
                .map(|(key, field)| (key.clone(), numbers_as_doubles(field)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// The property this change exists to hold: the same read, asked over either
/// transport, answers with the same thumbnails.
///
/// Asserted field by field rather than by count. Both transports go through one
/// storage query and one entry-mapping function, so a count-only check would pass
/// even if one of them dropped or re-derived a field on the way out — which is
/// the shape every divergence this change fixes actually took.
#[tokio::test]
async fn get_thumbnails_returns_the_same_entries_over_http_and_grpc() {
    use thumbnail_common::test_fixtures::mock_storage_server::unique_test_address;
    use thumbnail_proto::nvidia::omniverse::thumbnail::v1alpha as proto;

    let _ = test_app_state();
    let target = unique_test_address("parity-get-target.png");
    for suffix in ["a", "b"] {
        mock_storage().seed(
            &target,
            &format!("thumbnail_256x256_PNG_{suffix}"),
            serde_json::json!({
                "thumbnail": {
                    "resource_address": format!("https://store.test/{suffix}.png"),
                    "resource_identity": format!("https://store.test/{suffix}.png?version=1")
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
    }

    let query = serde_urlencoded::to_string([
        ("association_target_resource_address", target.as_str()),
        ("specification", "IMAGE_TO_IMAGE"),
        ("transform_type", "CROP"),
        ("crop_strategy", "TOP"),
    ])
    .expect("query");
    let http_response = get_thumbnails_query(&query).await;
    assert_eq!(http_response.status(), StatusCode::OK);
    let http_body = response_json(http_response).await;
    let http_entries = http_body["thumbnails"]
        .as_array()
        .expect("thumbnails array");

    let service = crate::grpc::ThumbnailServiceImpl::new(Arc::new(
        thumbnail_common::OutboundClients::for_tests(),
    ));
    let mut request = tonic::Request::new(proto::GetThumbnailsRequest {
        association_target_resource_address: target.clone(),
        specification: Some(proto::ThumbnailSpecificationType::ImageToImage as i32),
        transform_type: Some(proto::TransformType::Crop as i32),
        crop_strategy: Some(proto::CropStrategy::Top as i32),
        ..proto::GetThumbnailsRequest::default()
    });
    request
        .metadata_mut()
        .insert("authorization", "Bearer test-token".parse().unwrap());
    let grpc_entries = {
        use thumbnail_proto::nvidia::omniverse::thumbnail::v1alpha::thumbnail_service_server::ThumbnailService;
        service
            .get_thumbnails(request)
            .await
            .expect("the same read must succeed over gRPC")
            .into_inner()
            .thumbnails
    };

    assert_eq!(
        http_entries.len(),
        grpc_entries.len(),
        "both transports must return the same number of entries"
    );
    assert_eq!(http_entries.len(), 2, "both seeded entries should match");

    // Same order too: both apply the one sort by thumbnail resource_address, so
    // entry N means the same association on both sides.
    for (index, (http_entry, grpc_entry)) in http_entries.iter().zip(&grpc_entries).enumerate() {
        let resource = |value: &serde_json::Value| {
            (
                value["resource_address"].as_str().map(str::to_string),
                value["resource_identity"].as_str().map(str::to_string),
            )
        };
        let proto_resource = |value: &Option<proto::Resource>| {
            value
                .as_ref()
                .map(|r| {
                    (
                        Some(r.resource_address.clone()),
                        r.resource_identity.clone(),
                    )
                })
                .unwrap_or((None, None))
        };

        assert_eq!(
            resource(&http_entry["thumbnail"]),
            proto_resource(&grpc_entry.thumbnail),
            "entry {index}: thumbnail resource must match"
        );
        assert_eq!(
            resource(&http_entry["association_target"]),
            proto_resource(&grpc_entry.association_target),
            "entry {index}: association target must match"
        );
        assert_eq!(
            http_entry["target_type"], "FILE",
            "entry {index}: HTTP target type"
        );
        assert_eq!(
            grpc_entry.target_type,
            proto::TargetType::File as i32,
            "entry {index}: gRPC target type must be the same FILE"
        );
        assert_eq!(
            http_entry["create_time"].as_str().map(str::to_string),
            grpc_entry.create_time,
            "entry {index}: create_time must match"
        );

        // The gRPC Struct converted back to JSON must equal the JSON the HTTP
        // route returns, which is what makes `google.protobuf.Struct` an honest
        // representation of the stored specification rather than a lossy one.
        //
        // Compared with every number widened to a double on both sides.
        // `Struct` has one numeric type, so a stored `256` reaches a gRPC caller
        // as `256.0` — the documented cost of the Struct representation, and the
        // one difference between the transports that is a property of the wire
        // format rather than of this service. Comparing serde_json's integer and
        // float variants directly would fail on that and hide everything else.
        let grpc_specification = grpc_entry.specification.as_ref().map(|fields| {
            numbers_as_doubles(&thumbnail_common::utils::prost_to_json_value(
                &prost_types::Value {
                    kind: Some(prost_types::value::Kind::StructValue(fields.clone())),
                },
            ))
        });
        assert_eq!(
            Some(numbers_as_doubles(&http_entry["specification"])),
            grpc_specification,
            "entry {index}: the stored specification must survive both encodings identically"
        );
    }
}

#[tokio::test]
async fn get_thumbnails_route_returns_unauthorized_when_storage_rejects_token() {
    use thumbnail_common::test_fixtures::mock_storage_server::unique_test_address;

    let _ = test_app_state();
    let target = unique_test_address("get-rejected-token.png");
    let token = unique_test_address("get-rejected-token-value");
    mock_storage().reject_token(&token);
    let query =
        serde_urlencoded::to_string([("association_target_resource_address", target.as_str())])
            .expect("query");

    let response = get_thumbnails_query_as(&query, &token).await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let (error_message, source) = error_message_and_source(response).await;
    assert_eq!(
        error_message,
        "Authentication token was rejected by storage"
    );
    assert_eq!(source, "thumbnail-service");
}

#[tokio::test]
async fn get_thumbnails_route_recovers_from_a_transient_storage_failure() {
    use thumbnail_common::test_fixtures::mock_storage_server::{unique_test_address, MetadataOp};

    let _ = test_app_state();
    let target = unique_test_address("get-retry-target.png");
    mock_storage().seed(
        &target,
        "thumbnail_256x256_PNG_retry",
        serde_json::json!({
            "thumbnail": {
                "resource_address": "https://store.test/retried.png",
                "resource_identity": "https://store.test/retried.png?version=1"
            },
            "association_target": {
                "resource_address": target,
                "resource_identity": format!("{target}?version=1")
            },
            "target_type": "FILE"
        }),
    );
    mock_storage().fail_times(MetadataOp::Get, &target, None, tonic::Code::Unavailable, 1);
    let query =
        serde_urlencoded::to_string([("association_target_resource_address", target.as_str())])
            .expect("query");

    let response = get_thumbnails_query(&query).await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["thumbnails"].as_array().map(Vec::len), Some(1));
}

/// One unreadable stored entry must not make every other thumbnail on the same
/// target unreachable.
///
/// The read path used to return 500 for the whole request on the first entry with
/// no `thumbnail_resource_address`, so a single legacy or half-written record hid
/// every good one, with nothing in the response naming the culprit. The
/// disassociate sweep already treats such a record as one failed entry rather
/// than as a reason to abandon the run.
#[tokio::test]
async fn get_thumbnails_route_skips_a_malformed_entry_and_returns_the_rest() {
    use thumbnail_common::test_fixtures::mock_storage_server::unique_test_address;

    let _ = test_app_state();
    let target = unique_test_address("get-malformed-entry-target.png");

    for index in 0..2 {
        let address = format!("https://store.test/good-{index}.png");
        mock_storage().seed(
            &target,
            &format!("thumbnail_256x256_PNG_good{index}"),
            serde_json::json!({
                "thumbnail": {
                    "resource_address": address,
                    "resource_identity": format!("{address}?version=1")
                },
                "association_target": {
                    "resource_address": target,
                    "resource_identity": format!("{target}?version=1")
                },
                "target_type": "FILE"
            }),
        );
    }

    // The malformed one: an identity but no address, which is the shape a legacy
    // entry has and the shape the reader cannot turn into a response.
    mock_storage().seed(
        &target,
        "thumbnail_256x256_PNG_malformed",
        serde_json::json!({
            "thumbnail": {
                "resource_identity": "https://store.test/orphan.png?version=1"
            },
            "association_target": {
                "resource_address": target,
                "resource_identity": format!("{target}?version=1")
            },
            "target_type": "FILE"
        }),
    );

    let query =
        serde_urlencoded::to_string([("association_target_resource_address", target.as_str())])
            .expect("query");

    let response = get_thumbnails_query(&query).await;

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "one bad entry must not fail the request"
    );
    let body = response_json(response).await;
    let thumbnails = body["thumbnails"].as_array().expect("thumbnails array");
    assert_eq!(
        thumbnails.len(),
        2,
        "both readable thumbnails must be returned, got: {thumbnails:?}"
    );
}

/// Every entry unreadable is still a success, not a 500.
///
/// An empty list is a worse answer than an error only if the caller cannot tell
/// the difference — which is why the skip is logged with the offending keys. What
/// it must not be is a server error, because nothing about the request was wrong.
#[tokio::test]
async fn get_thumbnails_route_returns_an_empty_list_when_every_entry_is_malformed() {
    use thumbnail_common::test_fixtures::mock_storage_server::unique_test_address;

    let _ = test_app_state();
    let target = unique_test_address("get-all-malformed-target.png");

    for index in 0..2 {
        mock_storage().seed(
            &target,
            &format!("thumbnail_256x256_PNG_bad{index}"),
            serde_json::json!({
                "thumbnail": {
                    "resource_identity": format!("https://store.test/orphan-{index}.png?version=1")
                },
                "association_target": {
                    "resource_address": target,
                    "resource_identity": format!("{target}?version=1")
                },
                "target_type": "FILE"
            }),
        );
    }

    let query =
        serde_urlencoded::to_string([("association_target_resource_address", target.as_str())])
            .expect("query");

    let response = get_thumbnails_query(&query).await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(
        body["thumbnails"].as_array().map(Vec::len),
        Some(0),
        "an unreadable population is an empty success, not a server error"
    );
}

#[tokio::test]
async fn get_thumbnails_route_caps_and_orders_large_result_sets() {
    use thumbnail_common::test_fixtures::mock_storage_server::unique_test_address;

    let _ = test_app_state();
    let target = unique_test_address("get-large-result-target.png");
    for index in (0..=1000).rev() {
        let address = format!("https://store.test/thumb-{index:04}.png");
        mock_storage().seed(
            &target,
            &format!("thumbnail_256x256_PNG_{index:04}"),
            serde_json::json!({
                "thumbnail": {
                    "resource_address": address,
                    "resource_identity": format!("{address}?version=1")
                },
                "association_target": {
                    "resource_address": target,
                    "resource_identity": format!("{target}?version=1")
                },
                "target_type": "FILE"
            }),
        );
    }
    let query =
        serde_urlencoded::to_string([("association_target_resource_address", target.as_str())])
            .expect("query");

    let response = get_thumbnails_query(&query).await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    let thumbnails = body["thumbnails"].as_array().expect("thumbnails array");
    assert_eq!(thumbnails.len(), 1000);
    assert_eq!(
        thumbnails[0]["thumbnail"]["resource_address"],
        "https://store.test/thumb-0000.png"
    );
    assert_eq!(
        thumbnails[999]["thumbnail"]["resource_address"],
        "https://store.test/thumb-0999.png"
    );
}

/// Pins SAS-token redaction: the most valuable assertion in this file, and the
/// reason the fix here is to re-point the capture rather than delete the test.
#[tokio::test]
#[serial_test::serial]
async fn get_thumbnails_route_does_not_log_signed_resource_addresses() {
    let _ = test_app_state();
    let target = "https://store.test/private/source.png?sig=secret-sas-token";
    mock_storage().seed(
        target,
        "thumbnail_256x256_PNG_safe-log",
        serde_json::json!({
            "thumbnail": {
                "resource_address": "https://store.test/thumb.png",
                "resource_identity": "https://store.test/thumb.png?version=1"
            },
            "association_target": {
                "resource_address": target,
                "resource_identity": format!("{target}&version=1")
            },
            "target_type": "FILE"
        }),
    );
    let query = serde_urlencoded::to_string([("association_target_resource_address", target)])
        .expect("query");
    let buffer = start_capturing_logs();

    let response = get_thumbnails_query(&query).await;

    assert_eq!(response.status(), StatusCode::OK);
    let logs = captured_logs(&buffer);
    assert!(
        !logs.contains(target),
        "signed address leaked in logs: {logs}"
    );
    assert!(
        !logs.contains("secret-sas-token"),
        "signed query credential leaked in logs: {logs}"
    );
}

#[test]
fn get_thumbnails_openapi_documents_every_query_filter() {
    let document =
        serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI document should serialize");
    let parameters = document["paths"]["/v1alpha/thumbnails"]["get"]["parameters"]
        .as_array()
        .expect("GET parameters");
    let names: std::collections::BTreeSet<&str> = parameters
        .iter()
        .filter_map(|parameter| parameter["name"].as_str())
        .collect();
    let expected: std::collections::BTreeSet<&str> = [
        "association_target_resource_address",
        "specification",
        "width",
        "height",
        "encoding_format",
        "transform_type",
        "crop_strategy",
    ]
    .into_iter()
    .collect();
    assert_eq!(names, expected);

    assert_eq!(
        document["components"]["schemas"]["ThumbnailSpecificationType"]["enum"],
        serde_json::json!(["IMAGE_TO_IMAGE"])
    );

    // The status surface is checked exhaustively, for every endpoint, by
    // `documented_statuses_match_what_the_handlers_can_return`. A `contains_key`
    // loop lived here and could only catch a status that went missing, never one
    // that was advertised and unreachable — which is how the `500` below it
    // survived.
}

#[tokio::test]
async fn get_thumbnails_route_returns_empty_list_when_target_has_no_thumbnail_metadata() {
    use thumbnail_common::test_fixtures::mock_storage_server::unique_test_address;

    let _ = test_app_state();
    let target = unique_test_address("get-no-thumbnails-target.png");
    mock_storage().seed(
        &target,
        "unrelated_metadata",
        serde_json::json!({ "value": true }),
    );
    let query =
        serde_urlencoded::to_string([("association_target_resource_address", target.as_str())])
            .expect("query");

    let response = get_thumbnails_query(&query).await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(
        body["thumbnails"],
        serde_json::json!([]),
        "no thumbnails should return an empty list, not an error"
    );
}

#[tokio::test]
async fn get_thumbnails_route_maps_storage_authorization_and_outage_failures() {
    use thumbnail_common::test_fixtures::mock_storage_server::{unique_test_address, MetadataOp};

    let _ = test_app_state();
    let cases = [
        (
            tonic::Code::PermissionDenied,
            StatusCode::FORBIDDEN,
            "Access to the requested resource was denied",
        ),
        (
            tonic::Code::Unavailable,
            StatusCode::BAD_GATEWAY,
            "Storage service is unavailable",
        ),
    ];

    for (code, expected_status, expected_message) in cases {
        let target = unique_test_address(&format!("get-storage-error-{code:?}"));
        mock_storage().fail(MetadataOp::Get, &target, None, code);
        let query =
            serde_urlencoded::to_string([("association_target_resource_address", target.as_str())])
                .expect("query");

        let response = get_thumbnails_query(&query).await;

        assert_eq!(response.status(), expected_status, "{code:?}");
        let (error_message, source) = error_message_and_source(response).await;
        assert_eq!(error_message, expected_message, "{code:?}");
        assert_eq!(source, "thumbnail-service", "{code:?}");
        assert!(
            !error_message.contains("http://"),
            "internal endpoint leaked"
        );
    }
}

#[tokio::test]
#[serial_test::serial]
async fn get_thumbnails_route_logs_only_mapped_status_for_storage_failures() {
    use thumbnail_common::test_fixtures::mock_storage_server::{unique_test_address, MetadataOp};

    let _ = test_app_state();
    let target = unique_test_address("get-storage-log-error");
    mock_storage().fail(MetadataOp::Get, &target, None, tonic::Code::Unavailable);
    let query =
        serde_urlencoded::to_string([("association_target_resource_address", target.as_str())])
            .expect("query");
    let buffer = start_capturing_logs();

    let response = get_thumbnails_query(&query).await;

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let logs = captured_logs(&buffer);
    // Asserted across every captured handler line rather than against the first
    // one. The capture is process-global, so concurrent tests contribute their own
    // "Failed to get thumbnails" lines — and the line deliberately carries no
    // resource address (that is the redaction property the test above pins), so
    // there is nothing in it to attribute it by. Picking the first match is how
    // this test asserted `status=502` against another test's `status=401`.
    let handler_logs: Vec<&str> = logs
        .lines()
        .filter(|line| line.contains("Failed to get thumbnails"))
        .collect();

    assert!(
        handler_logs.iter().any(|line| line.contains("status=502")),
        "no handler error log reported the mapped 502; captured instead:\n{logs}"
    );
    // Scoped to the handler's own lines. The storage client legitimately logs the
    // upstream detail in its retry warnings — that is its job — so asserting
    // against the whole capture would forbid the layer below from describing what
    // it retried. The property under test is narrower: the handler's mapped-status
    // line must not carry the upstream message outward.
    assert!(
        !handler_logs
            .iter()
            .any(|line| line.contains("mock storage")),
        "the handler's mapped-status line leaked the storage detail:\n{}",
        handler_logs.join("\n")
    );
}

// ---------------------------------------------------------------------
// associate_thumbnail validation
// ---------------------------------------------------------------------

fn auth_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("Authorization", "Bearer test-token".parse().unwrap());
    headers
}

async fn post_association_body(body: impl Into<Body>) -> Response {
    post_association_as(body, "test-token").await
}

async fn post_association_as(body: impl Into<Body>, token: &str) -> Response {
    Router::new()
        .route("/v1alpha/associations", post(associate_thumbnail))
        .with_state(test_app_state())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1alpha/associations")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(body.into())
                .expect("request"),
        )
        .await
        .expect("router response")
}

async fn post_association_without_content_type(body: impl Into<Body>) -> Response {
    Router::new()
        .route("/v1alpha/associations", post(associate_thumbnail))
        .with_state(test_app_state())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1alpha/associations")
                .header("authorization", "Bearer test-token")
                .body(body.into())
                .expect("request"),
        )
        .await
        .expect("router response")
}

fn association_json_body() -> serde_json::Value {
    serde_json::json!({
        "target_type": "FILE",
        "thumbnail": {
            "resource_address": "https://store.test/thumb.png",
            "resource_identity": "thumb-id"
        },
        "association_target": {
            "resource_address": "https://store.test/source.png",
            "resource_identity": "source-id"
        },
        "specification": {
            "type": "IMAGE_TO_IMAGE",
            "width": 256,
            "height": 256,
            "encoding_format": "JPEG",
            "transform_type": "SQUARE",
            "crop_strategy": "CENTER",
            "quality": 85,
            "preserve_aspect_ratio": true
        }
    })
}

#[test]
fn canonical_association_request_accepts_the_http_request_shape() {
    let request = AssociationRequest {
        target_kind: AssociationTargetKind::File,
        thumbnail: association_file_resource("thumb://resource"),
        association_target: association_file_resource("source://resource"),
        specification: Some(valid_spec()),
    };

    assert_eq!(request.thumbnail.resource_address, "thumb://resource");
    assert_eq!(
        request.association_target.resource_address,
        "source://resource"
    );
}

#[test]
fn association_openapi_uses_canonical_typed_request_schema() {
    let document =
        serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI document should serialize");

    assert_eq!(
        document["paths"]["/v1alpha/associations"]["post"]["requestBody"]["content"]
            ["application/json"]["schema"]["$ref"],
        "#/components/schemas/AssociationRequest"
    );
    assert_eq!(
        document["components"]["schemas"]["AssociationRequest"]["properties"]["specification"]
            ["allOf"][0]["$ref"],
        "#/components/schemas/AssociationSpec"
    );
    assert!(
        document["components"]["schemas"]["AssociationRequest"]["properties"]["specification"]
            .get("nullable")
            .is_none(),
        "required specification must not accept null"
    );
    for schema_name in [
        "AssociationResource",
        "AssociationSpec",
        "AssociationTargetKind",
        "EncodingFormat",
        "TransformType",
        "CropStrategy",
    ] {
        assert!(
            document["components"]["schemas"].get(schema_name).is_some(),
            "OpenAPI should define referenced schema {schema_name}"
        );
    }
    // ProcessorIdentity was removed; its role is now the "type" discriminant on AssociationSpec.
    assert!(
        document["components"]["schemas"]
            .get("ProcessorIdentity")
            .is_none(),
        "ProcessorIdentity schema should not be present after removal"
    );

    let required = document["components"]["schemas"]["AssociationRequest"]["required"]
        .as_array()
        .expect("AssociationRequest.required should be an array");
    assert!(required.iter().any(|field| field == "specification"));
    assert!(!required.iter().any(|field| field == "target_type"));
    assert_eq!(
        document["components"]["schemas"]["AssociationRequest"]["properties"]["target_type"]
            ["default"],
        "FILE"
    );

    // AssociationSpec is a serde internally-tagged enum. utoipa represents it
    // as a schema with variants — not as a flat object with named properties.
    let spec_schema = &document["components"]["schemas"]["AssociationSpec"];
    assert!(
        spec_schema.is_object(),
        "AssociationSpec schema should be present"
    );
    // processor_identity is removed; type discrimination is now structural.
    assert!(
        spec_schema
            .get("properties")
            .and_then(|p| p.get("processor_identity"))
            .is_none(),
        "processor_identity was removed from AssociationSpec"
    );

    let transforms = document["components"]["schemas"]["TransformType"]["enum"]
        .as_array()
        .expect("TransformType.enum should be an array");
    for transform in ["FIT", "THUMBNAIL", "RESIZE"] {
        assert!(
            transforms.iter().any(|value| value == transform),
            "OpenAPI should expose canonical transform {transform}"
        );
    }

    // OWASP integer-limit-legacy: width/height bounds are documented in each variant schema
    // (AssociationSpec is a tagged enum; the constraints live in the ImageToImage variant).
}

#[test]
fn create_thumbnails_openapi_requires_a_non_null_image_specification() {
    let document =
        serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI document should serialize");
    let schema = &document["components"]["schemas"]["ThumbnailSpecification"];
    let required = schema["required"]
        .as_array()
        .expect("ThumbnailSpecification.required should be an array");

    assert!(required.iter().any(|field| field == "image_to_image"));
    assert!(
        schema["properties"]["image_to_image"]
            .get("nullable")
            .is_none(),
        "the runtime rejects a null image_to_image value"
    );
}

#[test]
fn disassociate_openapi_bounds_optional_identity_strings() {
    let document =
        serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI document should serialize");

    let properties = &document["components"]["schemas"]["DisassociateRequest"]["properties"];
    let parameters = document["paths"]["/v1alpha/associations"]["delete"]["parameters"]
        .as_array()
        .expect("disassociate parameters should be an array");

    for name in [
        "association_target_resource_identity",
        "thumbnail_resource_identity",
    ] {
        assert_eq!(
            properties[name]["maxLength"], 2048,
            "{name} component schema must be bounded"
        );

        let parameter = parameters
            .iter()
            .find(|parameter| parameter["name"] == name)
            .unwrap_or_else(|| panic!("missing disassociate query parameter {name}"));
        assert_eq!(
            parameter["schema"]["maxLength"], 2048,
            "{name} query parameter must be bounded"
        );
    }
}

#[tokio::test]
async fn associate_endpoint_rejects_missing_auth() {
    let request = AssociationRequest {
        target_kind: AssociationTargetKind::File,
        thumbnail: association_file_resource("thumb://resource"),
        association_target: association_file_resource("source://resource"),
        specification: Some(valid_spec()),
    };

    let response =
        associate_thumbnail(State(test_app_state()), HeaderMap::new(), Ok(Json(request))).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let (error_message, source) = error_message_and_source(response).await;
    assert!(
        error_message.contains("Authorization"),
        "error_message should mention Authorization, got: {error_message}"
    );
    assert_eq!(source, "thumbnail-service");
}

#[tokio::test]
async fn associate_endpoint_rejects_missing_specification() {
    let request = AssociationRequest {
        target_kind: AssociationTargetKind::File,
        thumbnail: association_file_resource("thumb://resource"),
        association_target: association_file_resource("source://resource"),
        specification: None,
    };

    let response =
        associate_thumbnail(State(test_app_state()), auth_headers(), Ok(Json(request))).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let (error_message, source) = error_message_and_source(response).await;
    assert_eq!(error_message, "specification is required");
    assert_eq!(source, "thumbnail-service");
}

#[tokio::test]
async fn associate_route_maps_typed_json_rejections_to_error_response() {
    let cases = [("encoding_format", serde_json::json!("TIFF"))];

    for (field, invalid_value) in cases {
        let mut body = association_json_body();
        body["specification"][field] = invalid_value;
        let response = post_association_body(Body::from(body.to_string())).await;

        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "{field} should map to the documented 400 contract"
        );
        let (error_message, source) = error_message_and_source(response).await;
        // Error message must mention either the field name or the invalid value
        // (serde's tagged enum errors name the path but may surface via the value).
        assert!(
            error_message.contains(field) || error_message.contains("TIFF"),
            "{field} error was: {error_message}"
        );
        assert_eq!(source, "thumbnail-service");
    }
}

#[tokio::test]
async fn associate_route_accepts_numeric_quality_with_builtin_deserialization() {
    for quality in [85.0, 85.5] {
        let mut body = association_json_body();
        body["specification"]["quality"] = serde_json::json!(quality);
        let response = post_association_body(Body::from(body.to_string())).await;

        assert_eq!(
            response.status(),
            StatusCode::NO_CONTENT,
            "numeric quality {quality} should pass deserialization and reach test storage"
        );
    }
}

#[tokio::test]
async fn associate_route_bounds_json_rejection_error_messages() {
    let mut body = association_json_body();
    body["specification"]["transform_type"] = serde_json::Value::String("X".repeat(5000));
    let response = post_association_body(Body::from(body.to_string())).await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let (error_message, _) = error_message_and_source(response).await;
    assert!(error_message.chars().count() <= 4096);
}

#[tokio::test]
async fn associate_route_maps_malformed_json_to_error_response() {
    let response = post_association_body(Body::from(r#"{"target_type":"FILE""#)).await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let (error_message, source) = error_message_and_source(response).await;
    assert!(
        error_message.contains("JSON"),
        "unexpected error: {error_message}"
    );
    assert_eq!(source, "thumbnail-service");
}

#[tokio::test]
async fn associate_route_preserves_non_json_rejection_status() {
    let response =
        post_association_without_content_type(Body::from(association_json_body().to_string()))
            .await;

    assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
}

#[tokio::test]
async fn associate_route_rejects_missing_spec_and_out_of_range_dimensions() {
    let mut missing_spec = association_json_body();
    missing_spec
        .as_object_mut()
        .expect("request object")
        .remove("specification");
    let response = post_association_body(Body::from(missing_spec.to_string())).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let (error_message, _) = error_message_and_source(response).await;
    assert_eq!(error_message, "specification is required");

    for (field, value) in [("width", 0), ("height", 8193)] {
        let mut body = association_json_body();
        body["specification"][field] = serde_json::json!(value);
        let response = post_association_body(Body::from(body.to_string())).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let (error_message, _) = error_message_and_source(response).await;
        assert!(
            error_message.contains("width and height"),
            "{field} error was: {error_message}"
        );
    }
}

#[tokio::test]
async fn associate_route_applies_quality_range_only_to_lossy_formats() {
    for encoding_format in ["JPEG", "WEBP"] {
        for quality in [0, 101] {
            let mut body = association_json_body();
            body["specification"]["encoding_format"] = serde_json::json!(encoding_format);
            body["specification"]["quality"] = serde_json::json!(quality);
            let response = post_association_body(Body::from(body.to_string())).await;
            assert_eq!(
                response.status(),
                StatusCode::BAD_REQUEST,
                "{encoding_format} quality {quality} should be rejected"
            );
        }
    }

    for encoding_format in ["PNG", "GIF"] {
        let mut body = association_json_body();
        body["specification"]["encoding_format"] = serde_json::json!(encoding_format);
        body["specification"]["quality"] = serde_json::json!(101);
        let response = post_association_body(Body::from(body.to_string())).await;
        assert_eq!(
            response.status(),
            StatusCode::NO_CONTENT,
            "{encoding_format} quality should pass contract validation and reach test storage"
        );
    }
}

#[tokio::test]
async fn associate_route_defaults_omitted_target_type_to_file_validation() {
    let mut body = association_json_body();
    body.as_object_mut()
        .expect("request object")
        .remove("target_type");
    body["association_target"]
        .as_object_mut()
        .expect("association target")
        .remove("resource_identity");
    let response = post_association_body(Body::from(body.to_string())).await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let (error_message, _) = error_message_and_source(response).await;
    assert!(error_message.contains("association_target"));
}

#[tokio::test]
async fn production_task_queue_http_payload_reaches_association_execution() {
    let mut body = association_json_body();
    body["specification"]["transform_type"] = serde_json::json!("THUMBNAIL");
    body["specification"]
        .as_object_mut()
        .expect("specification object")
        .remove("quality");
    body["specification"]
        .as_object_mut()
        .expect("specification object")
        .remove("preserve_aspect_ratio");

    let response = post_association_body(Body::from(body.to_string())).await;

    assert_eq!(
        response.status(),
        StatusCode::NO_CONTENT,
        "a production task-queue payload should pass validation and execute"
    );
    // Execution reached storage, which is the point of the test: the payload got past
    // contract validation and produced the writes an association is made of.
    assert!(
        !mock_storage()
            .metadata("https://store.test/source.png")
            .is_empty(),
        "the association-target side of the write plan should have landed"
    );
    assert!(
        mock_storage().has_key("https://store.test/thumb.png", THUMBNAIL_METADATA_KEY),
        "the thumbnail side of the write plan should have landed"
    );
}

// The response used to echo the request back, which nothing read. A 204 says the same
// thing without asking every caller to parse a body to learn it.
#[tokio::test]
async fn associate_endpoint_returns_no_content_with_an_empty_body() {
    let response = post_association_body(Body::from(association_json_body().to_string())).await;

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(
        response.headers().get("content-type").is_none(),
        "a body-less response should not claim a content type"
    );

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    assert!(body.is_empty(), "204 must carry no body");
}

// A well-formed token this service accepts can still be rejected downstream, most often
// because it expired. That is a 401 the caller can act on, not a 500 inviting them to
// retry the same doomed request.
#[tokio::test]
async fn associate_endpoint_returns_unauthorized_when_storage_rejects_the_token() {
    let _ = test_app_state();
    mock_storage().reject_token("http-associate-expired-token");

    let response = post_association_as(
        Body::from(association_json_body().to_string()),
        "http-associate-expired-token",
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let (error_message, source) = error_message_and_source(response).await;
    assert_eq!(source, "thumbnail-service");
    assert!(
        !error_message.is_empty(),
        "a 401 should still carry the documented ErrorResponse body"
    );
}

#[tokio::test]
async fn associate_endpoint_rejects_file_thumbnail_without_identity() {
    // The thumbnail is always validated as a file (requires identity),
    // regardless of target_type.
    let request = AssociationRequest {
        target_kind: AssociationTargetKind::File,
        thumbnail: association_folder_resource("thumb://resource"),
        association_target: association_file_resource("source://resource"),
        specification: Some(valid_spec()),
    };

    let response =
        associate_thumbnail(State(test_app_state()), auth_headers(), Ok(Json(request))).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let (error_message, source) = error_message_and_source(response).await;
    assert!(
        error_message.contains("invalid thumbnail resource"),
        "error_message should mention the invalid thumbnail resource, got: {error_message}"
    );
    assert_eq!(source, "thumbnail-service");
}

#[tokio::test]
async fn associate_endpoint_rejects_folder_target_carrying_identity() {
    let mut folder_with_identity = association_folder_resource("source://resource");
    folder_with_identity.resource_identity = Some("unexpected-identity".to_string());

    let request = AssociationRequest {
        target_kind: AssociationTargetKind::Folder,
        thumbnail: association_file_resource("thumb://resource"),
        association_target: folder_with_identity,
        specification: Some(valid_spec()),
    };

    let response =
        associate_thumbnail(State(test_app_state()), auth_headers(), Ok(Json(request))).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let (error_message, source) = error_message_and_source(response).await;
    assert!(
        error_message.contains("invalid association_target resource"),
        "error_message should mention the invalid association_target, got: {error_message}"
    );
    assert_eq!(source, "thumbnail-service");
}

// ---------------------------------------------------------------------
// disassociate_thumbnail validation
// ---------------------------------------------------------------------

#[tokio::test]
async fn disassociate_endpoint_rejects_blank_association_target_address() {
    let query = DisassociateRequest {
        target_kind: AssociationTargetKind::File,
        association_target_resource_address: "   ".to_string(),
        association_target_resource_identity: None,
        thumbnail_resource_address: "https://store.test/thumb.png".to_string(),
        thumbnail_resource_identity: None,
    };

    let response =
        disassociate_thumbnail(State(test_app_state()), auth_headers(), Ok(Query(query))).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let (error_message, source) = error_message_and_source(response).await;
    assert!(
        error_message.contains("association_target_resource_address"),
        "error_message should mention the invalid address, got: {error_message}"
    );
    assert_eq!(source, "thumbnail-service");
}

#[tokio::test]
async fn disassociate_endpoint_rejects_blank_thumbnail_address() {
    let query = DisassociateRequest {
        target_kind: AssociationTargetKind::File,
        association_target_resource_address: "https://store.test/source.usd".to_string(),
        association_target_resource_identity: None,
        thumbnail_resource_address: "   ".to_string(),
        thumbnail_resource_identity: None,
    };

    let response =
        disassociate_thumbnail(State(test_app_state()), auth_headers(), Ok(Query(query))).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let (error_message, source) = error_message_and_source(response).await;
    assert!(
        error_message.contains("thumbnail_resource_address"),
        "error_message should mention the invalid thumbnail address, got: {error_message}"
    );
    assert_eq!(source, "thumbnail-service");
}

// ---------------------------------------------------------------------
// live_health_check — dependency-free liveness (always OK)
// health_check — readiness; reflects the shared gRPC serving flag
// ---------------------------------------------------------------------

#[tokio::test]
async fn live_health_check_always_ok() {
    let response = live_health_check().await.into_response();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn health_check_ok_when_grpc_healthy() {
    let response = health_check(State(test_app_state())).await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn health_check_unavailable_when_grpc_unhealthy() {
    let state = test_app_state();
    state
        .grpc_healthy
        .store(false, std::sync::atomic::Ordering::Relaxed);

    let response = health_check(State(state)).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

// ---------------------------------------------------------------------
// disassociate_http_status — pure decision function, no network involved
// ---------------------------------------------------------------------

/// Seed one association-target address holding `count` thumbnail entries, as a prior
/// round of associations would have left it. Returns the target and thumbnail addresses.
fn seed_associated_thumbnails(label: &str, count: usize) -> (String, Vec<String>) {
    use thumbnail_common::test_fixtures::mock_storage_server::unique_test_address;

    // Installs the runtime config, which is what starts the storage double.
    let _ = test_app_state();

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

async fn disassociate(target_address: &str, thumbnail_address: &str) -> axum::response::Response {
    disassociate_with_headers(target_address, thumbnail_address, auth_headers()).await
}

async fn disassociate_with_headers(
    target_address: &str,
    thumbnail_address: &str,
    headers: HeaderMap,
) -> axum::response::Response {
    disassociate_thumbnail(
        State(test_app_state()),
        headers,
        Ok(Query(DisassociateRequest {
            target_kind: AssociationTargetKind::Folder,
            association_target_resource_address: target_address.to_string(),
            association_target_resource_identity: None,
            thumbnail_resource_address: thumbnail_address.to_string(),
            thumbnail_resource_identity: None,
        })),
    )
    .await
}

fn bearer(token: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("Authorization", format!("Bearer {token}").parse().unwrap());
    headers
}

// A successful targeted removal returns 200 with the thumbnail in `succeeded`.
#[tokio::test]
async fn disassociate_endpoint_returns_ok_when_removal_succeeds() {
    let (target_address, thumbnail_addresses) = seed_associated_thumbnails("http-success", 1);

    let response = disassociate(&target_address, &thumbnail_addresses[0]).await;
    assert_eq!(response.status(), StatusCode::OK);
}

// The only removal failing makes the outcome a total failure → 502.
#[tokio::test]
async fn disassociate_endpoint_returns_bad_gateway_when_removal_fails() {
    use thumbnail_common::test_fixtures::mock_storage_server::MetadataOp;

    let (target_address, thumbnail_addresses) = seed_associated_thumbnails("http-failure", 1);
    mock_storage().fail(
        MetadataOp::Delete,
        &target_address,
        Some("thumbnail_256x256_PNG_key0"),
        tonic::Code::Internal,
    );

    assert_eq!(
        disassociate(&target_address, &thumbnail_addresses[0])
            .await
            .status(),
        StatusCode::BAD_GATEWAY
    );
}

// Two empty arrays mean opposite things either side of the status: with 200 the
// thumbnail was not found (idempotent), with 502 the enumeration failed. The body
// cannot tell them apart, so the status must.
#[tokio::test]
async fn disassociate_endpoint_returns_bad_gateway_with_empty_arrays_when_enumeration_fails() {
    use thumbnail_common::test_fixtures::mock_storage_server::MetadataOp;

    let (target_address, thumbnail_addresses) = seed_associated_thumbnails("http-unenumerable", 1);
    mock_storage().fail(
        MetadataOp::Get,
        &target_address,
        None,
        tonic::Code::Unavailable,
    );

    let response = disassociate(&target_address, &thumbnail_addresses[0]).await;

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);

    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body"),
    )
    .expect("a disassociate response body");

    assert_eq!(
        body["succeeded"].as_array().map(Vec::len),
        Some(0),
        "nothing was removed"
    );
    assert_eq!(
        body["failed"].as_array().map(Vec::len),
        Some(0),
        "no thumbnail was inspected, so none can be reported as failed"
    );
}

// A rejected token is the caller's to fix. 502 would send them looking for an outage
// that is not there, so the abort cause decides between the two.
#[tokio::test]
async fn disassociate_endpoint_returns_unauthorized_when_the_token_is_rejected() {
    let (target_address, thumbnail_addresses) = seed_associated_thumbnails("http-expired", 1);
    mock_storage().reject_token("http-expired-token");

    let response = disassociate_with_headers(
        &target_address,
        &thumbnail_addresses[0],
        bearer("http-expired-token"),
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let (error_message, source) = error_message_and_source(response).await;
    assert_eq!(source, "thumbnail-service");
    assert!(error_message.contains("token rejected"));
}

#[tokio::test]
async fn disassociate_endpoint_fails_fast_without_a_token() {
    let (target_address, thumbnail_addresses) = seed_associated_thumbnails("http-no-token", 1);

    let response =
        disassociate_with_headers(&target_address, &thumbnail_addresses[0], HeaderMap::new()).await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(
        mock_storage().calls_for(&target_address).is_empty(),
        "should reject the request before enumerating anything"
    );
}

// A malformed address is the caller's mistake.
#[tokio::test]
async fn disassociate_endpoint_rejects_a_blank_address_before_reaching_storage() {
    let response =
        disassociate_with_headers("   ", "https://store.test/thumb.png", auth_headers()).await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(mock_storage().calls_for("   ").is_empty());
}

/// Drive the real route, so the query string goes through axum's extractor rather than
/// a hand-built struct that would skip the deserialization rules under test.
async fn delete_association(query: &str) -> Response {
    Router::new()
        .route(
            "/v1alpha/associations",
            axum::routing::delete(disassociate_thumbnail),
        )
        .with_state(test_app_state())
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/v1alpha/associations?{query}"))
                .header("authorization", "Bearer test-token")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("router response")
}

// A query that will not deserialize is rejected by the extractor, before the handler
// body runs. Left to itself the extractor answers in plain text, so the ErrorResponse
// the OpenAPI spec promises for 400 would hold only for requests that parse.
#[tokio::test]
async fn disassociate_endpoint_reports_an_unparseable_query_as_an_error_response() {
    for query in [
        "",
        "target_type=NOPE&association_target_resource_address=x&thumbnail_resource_address=y",
    ] {
        let response = delete_association(query).await;

        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "query {query:?}"
        );
        assert_eq!(
            response
                .headers()
                .get("content-type")
                .and_then(|value| value.to_str().ok()),
            Some("application/json"),
            "query {query:?} should answer in the service's error shape"
        );

        let (error_message, source) = error_message_and_source(response).await;
        assert_eq!(source, "thumbnail-service");
        assert!(
            error_message.contains("invalid query"),
            "expected an invalid-query message, got {error_message:?}"
        );
    }
}

// gRPC has always defaulted an omitted target_type to FILE; HTTP matches.
#[tokio::test]
async fn disassociate_endpoint_accepts_an_omitted_target_type_as_file() {
    let (target_address, thumbnail_addresses) = seed_associated_thumbnails("http-default-kind", 1);

    let response = delete_association(&format!(
        "association_target_resource_address={target_address}\
         &thumbnail_resource_address={}",
        thumbnail_addresses[0]
    ))
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        !mock_storage().calls_for(&target_address).is_empty(),
        "the handler should have reached storage rather than rejecting for a missing parameter"
    );
}

#[tokio::test]
async fn disassociate_endpoint_preserves_an_explicit_target_type() {
    let (target_address, thumbnail_addresses) = seed_associated_thumbnails("http-explicit-kind", 1);

    let response = delete_association(&format!(
        "target_type=FOLDER&association_target_resource_address={target_address}\
         &thumbnail_resource_address={}",
        thumbnail_addresses[0]
    ))
    .await;

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn disassociate_endpoint_returns_ok_when_thumbnail_is_not_associated() {
    use thumbnail_common::test_fixtures::mock_storage_server::unique_test_address;
    let (target_address, _) = seed_associated_thumbnails("http-empty", 0);
    // Seed an unrelated key so the target exists but has no matching thumbnail.
    mock_storage().seed(&target_address, "unrelated", serde_json::json!({}));
    let non_associated_thumb = unique_test_address("not-associated.png");

    // Requesting removal of a thumbnail that is not present succeeds with empty arrays.
    assert_eq!(
        disassociate(&target_address, &non_associated_thumb)
            .await
            .status(),
        StatusCode::OK
    );
}

/// A disassociate outcome with `succeeded` and `failed` of the given sizes.
fn disassociate_outcome(succeeded: usize, failed: usize) -> DisassociateOutcome {
    let thumbnail = || thumbnail_common::DisassociatedThumbnail {
        resource_address: "https://store.test/thumb.png".to_string(),
        resource_identity: None,
    };
    DisassociateOutcome {
        succeeded: (0..succeeded).map(|_| thumbnail()).collect(),
        failed: (0..failed)
            .map(|_| thumbnail_common::DisassociateEntryFailure {
                thumbnail: thumbnail(),
                error: thumbnail_common::DisassociateEntryError::MalformedEntry {
                    key: "thumbnail_abc123".to_string(),
                },
            })
            .collect(),
        aborted: None,
    }
}

#[test]
fn disassociate_http_status_ok_when_removal_succeeded() {
    assert_eq!(
        disassociate_http_status(&disassociate_outcome(1, 0)),
        StatusCode::OK
    );
}

#[test]
fn disassociate_http_status_ok_when_thumbnail_is_not_associated() {
    // Nothing to remove is not a failure to remove anything.
    assert_eq!(
        disassociate_http_status(&disassociate_outcome(0, 0)),
        StatusCode::OK
    );
}

#[test]
fn disassociate_http_status_bad_gateway_when_removal_failed() {
    assert_eq!(
        disassociate_http_status(&disassociate_outcome(0, 1)),
        StatusCode::BAD_GATEWAY
    );
}

#[test]
fn disassociate_http_status_bad_gateway_on_enumeration_failure() {
    // The operation never inspected association metadata, so empty lists mean "unknown".
    let mut outcome = disassociate_outcome(0, 0);
    outcome.aborted = Some(thumbnail_common::DisassociateAbort::Enumeration(
        thumbnail_common::StorageMetadataError::Failed("upstream down".to_string()),
    ));
    assert_eq!(disassociate_http_status(&outcome), StatusCode::BAD_GATEWAY);
}

// ─────────────────────────────────────────────────────────────────────────────
// create_thumbnails_handler
// ─────────────────────────────────────────────────────────────────────────────

mod create_thumbnails_tests {
    use super::*;
    use axum::{body::to_bytes, Router};
    use std::net::SocketAddr;
    use std::sync::{Arc, Mutex};

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
    use tower::ServiceExt;

    type BoxStream<T> =
        std::pin::Pin<Box<dyn tokio_stream::Stream<Item = Result<T, Status>> + Send>>;

    #[derive(Clone)]
    enum StatResp {
        Ok(String),
        Err(tonic::Code),
        /// Accept the call and never answer it.
        ///
        /// This is the case connect timeout and keepalive both miss: the
        /// connection is established and the peer is alive, it just does not
        /// reply. Without a per-request deadline the caller waits forever.
        Stall,
    }

    struct MockFos {
        resp: Arc<Mutex<StatResp>>,
        /// How many `Stat` calls arrived.
        ///
        /// A test that claims validation happens "before any outbound call" needs
        /// something to observe, and pointing the state at no double at all only
        /// proves the call failed, not that it was never made.
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
            // Bound before the match so the guard is released: an arm that awaits
            // would otherwise hold a std MutexGuard across it, making the future
            // non-Send.
            let behavior = self.resp.lock().unwrap().clone();
            match behavior {
                StatResp::Ok(id) => Ok(TonicResponse::new(StatResponse {
                    resource_info: Some(ResourceInfo {
                        resource_identity: Some(ResourceIdentity {
                            encoded_identity: id,
                        }),
                        metadata: None,
                    }),
                })),
                StatResp::Err(c) => Err(Status::new(c, "mock")),
                StatResp::Stall => {
                    // Far longer than any deadline under test; the request must
                    // not be waiting on this when the assertion runs.
                    tokio::time::sleep(std::time::Duration::from_secs(3_600)).await;
                    Err(Status::internal(
                        "unreachable: the deadline should have fired",
                    ))
                }
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

    fn state_with_storage(storage_addr: SocketAddr) -> AppState {
        let ch = tonic::transport::Endpoint::from_shared(format!("http://{storage_addr}"))
            .unwrap()
            .connect_lazy();
        let outbound = Arc::new(thumbnail_common::OutboundClients {
            storage_channel: ch,
            ..thumbnail_common::OutboundClients::for_tests()
        });
        AppState {
            outbound,
            runtime: thumbnail_common::test_fixtures::init_test_runtime_config(
                "thumbnail-service-http-tests",
            ),
            grpc_healthy: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        }
    }

    /// Fixed address all task-queue mock servers in this module bind to.
    /// Must match `MOCK_TASK_QUEUE_PORT` in the outer `tests` module.
    /// App state whose storage points at `storage_addr` and whose generate
    /// publishing reaches `publisher`.
    ///
    /// Dispatch is a gRPC publish now, so these tests no longer need a fixed
    /// port and no longer have to run serially or under `--ignored`: each one
    /// gets its own ephemeral publisher.
    fn state_with_storage_and_publisher(
        storage_addr: SocketAddr,
        publisher: &MockPublisherServer,
    ) -> AppState {
        let storage = tonic::transport::Endpoint::from_shared(format!("http://{storage_addr}"))
            .unwrap()
            .connect_lazy();
        let outbound = Arc::new(thumbnail_common::OutboundClients {
            storage_channel: storage,
            ..thumbnail_common::OutboundClients::for_tests_publishing_to(publisher.addr)
        });
        AppState {
            outbound,
            runtime: thumbnail_common::test_fixtures::init_test_runtime_config(
                "thumbnail-service-http-tests",
            ),
            grpc_healthy: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        }
    }

    async fn post_create(
        state: AppState,
        body: serde_json::Value,
        token: Option<&str>,
    ) -> axum::response::Response {
        let mut builder = Request::builder()
            .method("POST")
            .uri("/v1alpha/thumbnails")
            .header("content-type", "application/json");
        if let Some(t) = token {
            builder = builder.header("authorization", format!("Bearer {t}"));
        }
        Router::new()
            .route(
                "/v1alpha/thumbnails",
                axum::routing::post(create_thumbnails_handler),
            )
            .with_state(state)
            .oneshot(
                builder
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    // ── Validation tests (no network needed) ─────────────────────────────────

    #[test]
    fn create_thumbnails_openapi_has_one_consistent_202_contract() {
        let document =
            serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI document should serialize");
        let response = &document["paths"]["/v1alpha/thumbnails"]["post"]["responses"]["202"];

        assert_eq!(
            response["description"],
            "Thumbnail generation accepted. `failed` is empty on full acceptance and lists specifications whose dispatch failed after request validation, including publisher failures. At least one specification was accepted; a call where none was returns 502. Do not compare `failed.length` to the request's `specifications.length`: absent or empty specifications fan out over the configured THUMBNAIL_SIZES. Acceptance does not prove generation completes."
        );
        assert_eq!(
            response["content"]["application/json"]["schema"]["$ref"],
            "#/components/schemas/CreateThumbnailsResponse"
        );
        assert_eq!(
            document["components"]["schemas"]["CreateThumbnailsFailure"]["properties"]
                ["specification"]["$ref"],
            "#/components/schemas/ThumbnailSpecification"
        );
    }

    #[tokio::test]
    async fn missing_token_returns_401() {
        thumbnail_common::test_fixtures::init_test_runtime_config("thumbnail-service-http-tests");
        let body = serde_json::json!({ "resource_address": "https://storage.test/image.png" });
        let resp = post_create(test_app_state(), body, None).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn invalid_json_returns_400() {
        thumbnail_common::test_fixtures::init_test_runtime_config("thumbnail-service-http-tests");
        let state = test_app_state();
        let resp = Router::new()
            .route(
                "/v1alpha/thumbnails",
                axum::routing::post(create_thumbnails_handler),
            )
            .with_state(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1alpha/thumbnails")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer test-token")
                    .body(Body::from("{ not json"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn non_image_to_image_spec_type_returns_400() {
        thumbnail_common::test_fixtures::init_test_runtime_config("thumbnail-service-http-tests");
        let body = serde_json::json!({
            "resource_address": "https://storage.test/image.png",
            "specifications": [{ "video_to_image": { "encoding_format": "JPEG", "width": 256, "height": 256, "transform_type": "SQUARE", "crop_strategy": "CENTER", "frame_select_strategy": "AUTO" } }]
        });
        let resp = post_create(test_app_state(), body, Some("test-token")).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let (msg, src) = error_message_and_source(resp).await;
        assert!(
            msg.contains("image_to_image"),
            "expected image_to_image mention, got: {msg}"
        );
        assert_eq!(src, "thumbnail-service");
    }

    #[tokio::test]
    async fn omitting_image_to_image_field_returns_400() {
        // With the simplified spec type, a ThumbnailSpecification with no
        // image_to_image field set should be rejected before any outbound call.
        thumbnail_common::test_fixtures::init_test_runtime_config("thumbnail-service-http-tests");
        let body = serde_json::json!({
            "resource_address": "https://storage.test/image.png",
            "specifications": [{}]
        });

        let resp = post_create(test_app_state(), body, Some("test-token")).await;

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let (msg, _) = error_message_and_source(resp).await;
        assert!(
            msg.contains("image_to_image"),
            "expected image_to_image mention, got: {msg}"
        );
    }

    /// The bound is documented on the schema and enforced in the handler, and the
    /// two are separate statements of the same number: `#[schema(max_items)]`
    /// takes a literal, so it cannot read the constant. If they drift, generated
    /// clients pre-validate against a limit the service does not have.
    #[test]
    fn create_thumbnails_max_items_matches_the_enforced_bound() {
        let document =
            serde_json::to_value(ApiDoc::openapi()).expect("OpenAPI document should serialize");
        assert_eq!(
            document["components"]["schemas"]["CreateThumbnailsRequest"]["properties"]
                ["specifications"]["maxItems"],
            serde_json::json!(thumbnail_common::work_events::MAX_CREATE_THUMBNAIL_SPECIFICATIONS),
        );
    }

    fn create_body_with_spec_count(count: usize) -> serde_json::Value {
        let spec = serde_json::json!({
            "image_to_image": {
                "encoding_format": "PNG",
                "width": 256,
                "height": 256,
                "transform_type": "SQUARE",
                "crop_strategy": "CENTER"
            }
        });
        serde_json::json!({
            "resource_address": "https://storage.test/image.png",
            "specifications": vec![spec; count]
        })
    }

    #[tokio::test]
    async fn more_than_one_thousand_specs_returns_400_before_outbound_calls() {
        thumbnail_common::test_fixtures::init_test_runtime_config("thumbnail-service-http-tests");
        // A double that would answer, so a passing "no Stat" assertion means the
        // call was never made rather than that it could not have succeeded.
        let (storage_addr, stats) = start_counting_fos(StatResp::Ok("enc-id".to_string())).await;
        let body = create_body_with_spec_count(
            thumbnail_common::work_events::MAX_CREATE_THUMBNAIL_SPECIFICATIONS + 1,
        );

        let resp = post_create(state_with_storage(storage_addr), body, Some("test-token")).await;

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let (msg, _) = error_message_and_source(resp).await;
        assert!(
            msg.contains("specifications") && msg.contains("at most 1000"),
            "expected specifications cap message, got: {msg}"
        );
        assert_eq!(
            stats.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "a refused request must cost no storage Stat"
        );
    }

    /// The bound is inclusive, and identically so on both transports — see the
    /// gRPC twin, `exactly_one_thousand_specs_is_accepted`. An off-by-one here
    /// would refuse a request the documented `maxItems` says is legal.
    #[tokio::test]
    async fn exactly_one_thousand_specs_is_accepted() {
        let publisher = MockPublisherServer::start().await;
        let storage_addr = start_fos(StatResp::Ok("enc-id".to_string())).await;
        let body = create_body_with_spec_count(
            thumbnail_common::work_events::MAX_CREATE_THUMBNAIL_SPECIFICATIONS,
        );

        let resp = post_create(
            state_with_storage_and_publisher(storage_addr, &publisher),
            body,
            Some("test-token"),
        )
        .await;

        assert_eq!(
            resp.status(),
            StatusCode::ACCEPTED,
            "1000 is at the bound, not past it"
        );

        publisher.shutdown().await;
    }

    /// Dispatch one specification and hand back the `preserve_aspect_ratio` the
    /// published generate event resolved to. `supplied` is what the caller puts
    /// in the request body; `None` omits the field entirely.
    async fn dispatched_preserve_aspect_ratio(supplied: Option<bool>) -> serde_json::Value {
        let publisher = MockPublisherServer::start().await;
        let storage_addr = start_fos(StatResp::Ok("enc-id".to_string())).await;

        let mut image_to_image = serde_json::json!({
            "encoding_format": "PNG",
            "width": 256,
            "height": 256,
            "transform_type": "SQUARE",
            "crop_strategy": "CENTER"
        });
        if let Some(value) = supplied {
            image_to_image["preserve_aspect_ratio"] = serde_json::json!(value);
        }
        let body = serde_json::json!({
            "resource_address": "https://storage.test/image.png",
            "specifications": [{ "image_to_image": image_to_image }]
        });

        let resp = post_create(
            state_with_storage_and_publisher(storage_addr, &publisher),
            body,
            Some("test-token"),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::ACCEPTED);

        let published = publisher
            .await_events(1, std::time::Duration::from_secs(2))
            .await;
        let resolved = published[0].message["specification"]["preserve_aspect_ratio"].clone();

        publisher.shutdown().await;
        resolved
    }

    /// The HTTP specification type had no `preserve_aspect_ratio` field and the
    /// handler hardcoded `None`, so every HTTP request published the configured
    /// default no matter what the caller wanted. `false` is the telling case: the
    /// fixture configures the default as `true`, so a `false` reaching the event
    /// can only have come from the request.
    #[tokio::test]
    async fn an_explicit_preserve_aspect_ratio_reaches_the_work_event() {
        thumbnail_common::test_fixtures::init_test_runtime_config("thumbnail-service-http-tests");

        for supplied in [true, false] {
            assert_eq!(
                dispatched_preserve_aspect_ratio(Some(supplied)).await,
                serde_json::json!(supplied),
                "preserve_aspect_ratio: {supplied} must survive to the published event"
            );
        }
    }

    /// Omission is distinct from an explicit `false`, and resolves to the
    /// configured default — the same rule gRPC follows for an absent optional.
    #[tokio::test]
    async fn an_omitted_preserve_aspect_ratio_resolves_to_the_configured_default() {
        let runtime = thumbnail_common::test_fixtures::init_test_runtime_config(
            "thumbnail-service-http-tests",
        );
        let configured = runtime.thumbnail_defaults.thumbnail_preserve_aspect_ratio;

        assert_eq!(
            dispatched_preserve_aspect_ratio(None).await,
            serde_json::json!(configured),
            "an omitted preserve_aspect_ratio must resolve to THUMBNAIL_PRESERVE_ASPECT_RATIO"
        );
    }

    // ── Cross-transport parity ────────────────────────────────────────────────

    /// A gRPC service whose storage and generate publishing reach the same
    /// doubles an `AppState` from this module would.
    fn grpc_service(
        storage_addr: SocketAddr,
        publisher: &MockPublisherServer,
    ) -> crate::grpc::ThumbnailServiceImpl {
        thumbnail_common::test_fixtures::init_test_runtime_config("thumbnail-service-http-tests");
        let storage = tonic::transport::Endpoint::from_shared(format!("http://{storage_addr}"))
            .unwrap()
            .connect_lazy();
        crate::grpc::ThumbnailServiceImpl::new(Arc::new(thumbnail_common::OutboundClients {
            storage_channel: storage,
            ..thumbnail_common::OutboundClients::for_tests_publishing_to(publisher.addr)
        }))
    }

    fn with_grpc_auth<T>(request: T) -> tonic::Request<T> {
        let mut request = tonic::Request::new(request);
        request
            .metadata_mut()
            .insert("authorization", "Bearer test-token".parse().unwrap());
        request
    }

    fn proto_spec_with_preserve_aspect_ratio(
        preserve_aspect_ratio: Option<bool>,
    ) -> thumbnail_proto::nvidia::omniverse::thumbnail::v1alpha::ThumbnailSpecification {
        use thumbnail_proto::nvidia::omniverse::thumbnail::v1alpha as proto;
        proto::ThumbnailSpecification {
            specification: Some(proto::thumbnail_specification::Specification::ImageToImage(
                proto::ImageToImageSpecification {
                    processor_identity: proto::ProcessorIdentity::ImageToImage as i32,
                    encoding_format: proto::ImageEncodingFormat::Png as i32,
                    width: 256,
                    height: 256,
                    transform_type: proto::TransformType::Square as i32,
                    crop_strategy: proto::CropStrategy::Center as i32,
                    quality: None,
                    preserve_aspect_ratio,
                },
            )),
        }
    }

    #[tokio::test]
    async fn lossy_quality_rejected_on_both_transports_before_stat_or_publish() {
        use proto::thumbnail_service_server::ThumbnailService;
        use thumbnail_proto::nvidia::omniverse::thumbnail::v1alpha as proto;
        let publisher = MockPublisherServer::start().await;
        let (storage_addr, stats) = start_counting_fos(StatResp::Ok("enc-id".to_string())).await;
        for (format, proto_format) in [
            ("JPEG", proto::ImageEncodingFormat::Jpeg),
            ("WEBP", proto::ImageEncodingFormat::Webp),
        ] {
            for quality in [0, 101, 200, u32::MAX] {
                let mut body = create_body_with_spec_count(2);
                body["specifications"][1]["image_to_image"]["encoding_format"] =
                    serde_json::json!(format);
                // HTTP's wire value is i32; test its negative extreme as well.
                body["specifications"][1]["image_to_image"]["quality"] =
                    serde_json::json!(quality as i32);
                let response = post_create(
                    state_with_storage_and_publisher(storage_addr, &publisher),
                    body,
                    Some("test-token"),
                )
                .await;
                assert_eq!(
                    response.status(),
                    StatusCode::BAD_REQUEST,
                    "{format} {quality}"
                );
                let (message, _) = error_message_and_source(response).await;
                assert!(
                    message.contains("specification 1")
                        && message.contains("quality")
                        && message.contains("JPEG/WebP"),
                    "{message}"
                );
                let mut invalid = proto_spec_with_preserve_aspect_ratio(None);
                let Some(proto::thumbnail_specification::Specification::ImageToImage(spec)) =
                    &mut invalid.specification
                else {
                    unreachable!()
                };
                spec.encoding_format = proto_format as i32;
                spec.quality = Some(quality);
                let error = grpc_service(storage_addr, &publisher)
                    .create_thumbnails(with_grpc_auth(proto::CreateThumbnailsRequest {
                        resource_address: "https://storage.test/image.png".to_string(),
                        specifications: vec![proto_spec_with_preserve_aspect_ratio(None), invalid],
                    }))
                    .await
                    .unwrap_err();
                assert_eq!(error.code(), tonic::Code::InvalidArgument);
                assert!(
                    error
                        .message()
                        .contains("specifications[1].image_to_image.quality")
                        && error.message().contains("JPEG/WebP")
                );
            }
        }
        assert_eq!(stats.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert!(publisher
            .settle(std::time::Duration::from_millis(100))
            .await
            .is_empty());
        publisher.shutdown().await;
    }

    #[tokio::test]
    async fn accepted_and_omitted_quality_round_trips_on_both_transports() {
        use proto::thumbnail_service_server::ThumbnailService;
        use thumbnail_proto::nvidia::omniverse::thumbnail::v1alpha as proto;
        let runtime = thumbnail_common::test_fixtures::init_test_runtime_config(
            "thumbnail-service-http-tests",
        );
        for (format, proto_format) in [
            ("PNG", proto::ImageEncodingFormat::Png),
            ("GIF", proto::ImageEncodingFormat::Gif),
            ("JPEG", proto::ImageEncodingFormat::Jpeg),
            ("WEBP", proto::ImageEncodingFormat::Webp),
        ] {
            let qualities = if matches!(format, "PNG" | "GIF") {
                vec![Some(200), None]
            } else {
                vec![Some(1), Some(100), None]
            };
            for quality in qualities {
                let publisher = MockPublisherServer::start().await;
                let (storage_addr, stats) =
                    start_counting_fos(StatResp::Ok("enc-id".to_string())).await;
                let mut body = create_body_with_spec_count(1);
                body["specifications"][0]["image_to_image"]["encoding_format"] =
                    serde_json::json!(format);
                if let Some(value) = quality {
                    body["specifications"][0]["image_to_image"]["quality"] =
                        serde_json::json!(value);
                }
                let response = post_create(
                    state_with_storage_and_publisher(storage_addr, &publisher),
                    body,
                    Some("test-token"),
                )
                .await;
                assert_eq!(
                    response.status(),
                    StatusCode::ACCEPTED,
                    "{format} {quality:?}"
                );
                let body: CreateThumbnailsResponse = serde_json::from_slice(
                    &to_bytes(response.into_body(), usize::MAX).await.unwrap(),
                )
                .unwrap();
                assert!(body.failed.is_empty());
                let mut input = proto_spec_with_preserve_aspect_ratio(None);
                let Some(proto::thumbnail_specification::Specification::ImageToImage(spec)) =
                    &mut input.specification
                else {
                    unreachable!()
                };
                spec.encoding_format = proto_format as i32;
                spec.quality = quality;
                let response = grpc_service(storage_addr, &publisher)
                    .create_thumbnails(with_grpc_auth(proto::CreateThumbnailsRequest {
                        resource_address: "https://storage.test/image.png".to_string(),
                        specifications: vec![input],
                    }))
                    .await
                    .unwrap()
                    .into_inner();
                assert!(response.failed.is_empty());
                assert_eq!(stats.load(std::sync::atomic::Ordering::SeqCst), 2);
                let events = publisher
                    .await_events(2, std::time::Duration::from_secs(2))
                    .await;
                assert_eq!(events.len(), 2);
                for event in events {
                    assert_eq!(
                        event.message["specification"]["quality"].as_f64(),
                        Some(f64::from(quality.unwrap_or(
                            runtime.thumbnail_defaults.thumbnail_quality as u32
                        )))
                    );
                    assert_eq!(event.message["specification"]["encoding_format"], format);
                }
                publisher.shutdown().await;
            }
        }
    }

    #[test]
    fn openapi_quality_bounds_match_http_wire_range_for_png_and_gif() {
        let document = serde_json::to_value(ApiDoc::openapi()).unwrap();
        let quality = &document["components"]["schemas"]["ImageToImageSpecification"]["properties"]
            ["quality"];
        let minimum = quality["minimum"]
            .as_i64()
            .expect("explicit integer minimum");
        let maximum = quality["maximum"]
            .as_i64()
            .expect("explicit integer maximum");
        assert_eq!(minimum, i64::from(i32::MIN));
        assert_eq!(maximum, i64::from(i32::MAX));
        assert_eq!(quality["format"], "int32");
        assert_eq!(quality["type"], serde_json::json!(["integer", "null"]));
        let description = quality["description"].as_str().unwrap();
        assert!(description.contains("JPEG/WebP") && description.contains("PNG/GIF"));

        for format in ["PNG", "GIF"] {
            for value in [
                minimum - 1,
                minimum,
                -1,
                0,
                1,
                100,
                200,
                maximum,
                maximum + 1,
            ] {
                let mut body = create_body_with_spec_count(1);
                let spec = &mut body["specifications"][0]["image_to_image"];
                spec["encoding_format"] = serde_json::json!(format);
                spec["quality"] = serde_json::json!(value);
                let request = serde_json::from_value::<CreateThumbnailsRequest>(body);
                assert_eq!(
                    request.is_ok(),
                    (minimum..=maximum).contains(&value),
                    "{format} quality {value}: schema bounds must match deserialization"
                );
                if let Ok(request) = request {
                    assert!(request.validate().is_ok(), "{format} quality {value}");
                }
            }
        }
    }

    #[tokio::test]
    async fn failed_source_stat_preserves_both_statuses_without_publishing() {
        use proto::thumbnail_service_server::ThumbnailService;
        use thumbnail_proto::nvidia::omniverse::thumbnail::v1alpha as proto;
        let publisher = MockPublisherServer::start().await;
        let (storage_addr, stats) =
            start_counting_fos(StatResp::Err(tonic::Code::Unavailable)).await;
        let response = post_create(
            state_with_storage_and_publisher(storage_addr, &publisher),
            create_body_with_spec_count(1),
            Some("test-token"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let error = grpc_service(storage_addr, &publisher)
            .create_thumbnails(with_grpc_auth(proto::CreateThumbnailsRequest {
                resource_address: "https://storage.test/image.png".to_string(),
                specifications: vec![proto_spec_with_preserve_aspect_ratio(None)],
            }))
            .await
            .unwrap_err();
        assert_eq!(error.code(), tonic::Code::Internal);
        assert_eq!(stats.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert!(publisher
            .settle(std::time::Duration::from_millis(100))
            .await
            .is_empty());
        publisher.shutdown().await;
    }

    /// The gRPC twin of `dispatched_preserve_aspect_ratio`, so the two can be
    /// compared on the value each transport actually published.
    async fn grpc_dispatched_preserve_aspect_ratio(supplied: Option<bool>) -> serde_json::Value {
        use thumbnail_proto::nvidia::omniverse::thumbnail::v1alpha::thumbnail_service_server::ThumbnailService;

        let publisher = MockPublisherServer::start().await;
        let storage_addr = start_fos(StatResp::Ok("enc-id".to_string())).await;
        let service = grpc_service(storage_addr, &publisher);

        service
            .create_thumbnails(with_grpc_auth(
                thumbnail_proto::nvidia::omniverse::thumbnail::v1alpha::CreateThumbnailsRequest {
                    resource_address: "https://storage.test/image.png".to_string(),
                    specifications: vec![proto_spec_with_preserve_aspect_ratio(supplied)],
                },
            ))
            .await
            .expect("a valid gRPC create must be accepted");

        let published = publisher
            .await_events(1, std::time::Duration::from_secs(2))
            .await;
        let resolved = published[0].message["specification"]["preserve_aspect_ratio"].clone();

        publisher.shutdown().await;
        resolved
    }

    /// `preserve_aspect_ratio` existed on the proto and not on the HTTP body, so
    /// the same intent was expressible over one transport and not the other. Now
    /// both must resolve it to the same value — including on omission, where the
    /// configured default has to be applied by the same seam rather than by each
    /// transport separately.
    #[tokio::test]
    async fn preserve_aspect_ratio_resolves_identically_over_http_and_grpc() {
        thumbnail_common::test_fixtures::init_test_runtime_config("thumbnail-service-http-tests");

        for supplied in [Some(true), Some(false), None] {
            let http = dispatched_preserve_aspect_ratio(supplied).await;
            let grpc = grpc_dispatched_preserve_aspect_ratio(supplied).await;
            assert_eq!(
                http, grpc,
                "supplied {supplied:?}: both transports must publish the same resolved \
                 preserve_aspect_ratio"
            );
        }
    }

    /// The property group 1 restores: a request one transport refuses is a
    /// request the other refuses too. This is the case that was broken — HTTP
    /// returned 400 and gRPC accepted the same request — and the one a future
    /// divergence would break again.
    #[tokio::test]
    async fn an_over_long_specification_list_is_refused_by_both_transports() {
        use thumbnail_proto::nvidia::omniverse::thumbnail::v1alpha::thumbnail_service_server::ThumbnailService;

        let publisher = MockPublisherServer::start().await;
        // Doubles that would answer, so "no Stat" means neither transport made
        // the call rather than that the call could not have succeeded.
        let (storage_addr, stats) = start_counting_fos(StatResp::Ok("enc-id".to_string())).await;
        let over_bound = thumbnail_common::work_events::MAX_CREATE_THUMBNAIL_SPECIFICATIONS + 1;

        let http_response = post_create(
            state_with_storage_and_publisher(storage_addr, &publisher),
            create_body_with_spec_count(over_bound),
            Some("test-token"),
        )
        .await;
        assert_eq!(
            http_response.status(),
            StatusCode::BAD_REQUEST,
            "HTTP must refuse an over-long specification list"
        );

        let grpc_error = grpc_service(storage_addr, &publisher)
            .create_thumbnails(with_grpc_auth(
                thumbnail_proto::nvidia::omniverse::thumbnail::v1alpha::CreateThumbnailsRequest {
                    resource_address: "https://storage.test/image.png".to_string(),
                    specifications: vec![proto_spec_with_preserve_aspect_ratio(None); over_bound],
                },
            ))
            .await
            .expect_err("gRPC must refuse the same over-long specification list");
        assert_eq!(
            grpc_error.code(),
            tonic::Code::InvalidArgument,
            "the gRPC refusal must be a caller error, matching HTTP's 400"
        );

        let (http_message, _) = error_message_and_source(http_response).await;
        for message in [http_message.as_str(), grpc_error.message()] {
            assert!(
                message.contains("specifications") && message.contains("at most 1000"),
                "both transports must name the specification list and the same bound, got: \
                 {message}"
            );
        }

        assert_eq!(
            stats.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "neither transport may pay for a Stat on a request it refuses"
        );
        assert!(
            publisher
                .settle(std::time::Duration::from_millis(200))
                .await
                .is_empty(),
            "a refused request must publish no generate work on either transport"
        );

        publisher.shutdown().await;
    }

    /// An out-of-range dimension is the caller's error, and must be reported as
    /// one before the source Stat is paid for.
    ///
    /// Previously the `#[schema(minimum, maximum)]` annotations were the only
    /// statement of the range and enforced nothing, so a width of 20000 reached
    /// the work-event contract. Mixed with valid specs that surfaced as a 202
    /// with a per-spec failure; alone it surfaced as 502, the status reserved for
    /// "every publish failed". Both blame the service for a bad integer.
    #[tokio::test]
    #[serial_test::serial]
    async fn out_of_range_dimensions_return_400_before_any_stat() {
        thumbnail_common::test_fixtures::init_test_runtime_config("thumbnail-service-http-tests");

        for (field, width, height) in [
            ("width", 20_000, 256),
            ("height", 256, 20_000),
            ("width", 0, 256),
            ("height", 256, 0),
        ] {
            let body = serde_json::json!({
                "resource_address": "https://storage.test/image.png",
                "specifications": [{
                    "image_to_image": {
                        "encoding_format": "PNG",
                        "width": width,
                        "height": height,
                        "transform_type": "SQUARE",
                        "crop_strategy": "CENTER"
                    }
                }]
            });

            let resp = post_create(test_app_state(), body, Some("test-token")).await;

            assert_eq!(
                resp.status(),
                StatusCode::BAD_REQUEST,
                "{field}={width}x{height} must be a caller error"
            );
            let (msg, _) = error_message_and_source(resp).await;
            assert!(
                msg.contains(field) && msg.contains("between 1 and 8192"),
                "the message must name the offending field and the range, got: {msg}"
            );
        }
    }

    /// One bad specification rejects the whole request rather than returning a
    /// partial success. Validation runs before anything is dispatched, so there is
    /// no partial outcome to report yet -- unlike a publish failure, where some
    /// events have already gone out.
    #[tokio::test]
    #[serial_test::serial]
    async fn one_out_of_range_specification_rejects_the_whole_request() {
        thumbnail_common::test_fixtures::init_test_runtime_config("thumbnail-service-http-tests");

        let good = serde_json::json!({
            "image_to_image": {
                "encoding_format": "PNG",
                "width": 256,
                "height": 256,
                "transform_type": "SQUARE",
                "crop_strategy": "CENTER"
            }
        });
        let bad = serde_json::json!({
            "image_to_image": {
                "encoding_format": "PNG",
                "width": 9_000,
                "height": 256,
                "transform_type": "SQUARE",
                "crop_strategy": "CENTER"
            }
        });
        let body = serde_json::json!({
            "resource_address": "https://storage.test/image.png",
            "specifications": [good, bad]
        });

        let resp = post_create(test_app_state(), body, Some("test-token")).await;

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let (msg, _) = error_message_and_source(resp).await;
        assert!(
            msg.contains("specification 1"),
            "the message must identify which specification was wrong, got: {msg}"
        );
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn a_boundary_dimension_is_accepted() {
        thumbnail_common::test_fixtures::init_test_runtime_config("thumbnail-service-http-tests");

        let body = serde_json::json!({
            "resource_address": "https://storage.test/image.png",
            "specifications": [{
                "image_to_image": {
                    "encoding_format": "PNG",
                    "width": 8192,
                    "height": 8192,
                    "transform_type": "SQUARE",
                    "crop_strategy": "CENTER"
                }
            }]
        });

        let resp = post_create(test_app_state(), body, Some("test-token")).await;

        assert_ne!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "8192 is inside the accepted range and must not be rejected as out of range"
        );
    }

    // ── Storage error tests ───────────────────────────────────────────────────

    #[tokio::test]
    async fn storage_404_returns_404() {
        thumbnail_common::test_fixtures::init_test_runtime_config("thumbnail-service-http-tests");
        let storage_addr = start_fos(StatResp::Err(tonic::Code::NotFound)).await;
        let body = serde_json::json!({ "resource_address": "https://storage.test/missing.png" });
        let resp = post_create(state_with_storage(storage_addr), body, Some("test-token")).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let (_, src) = error_message_and_source(resp).await;
        assert_eq!(src, "thumbnail-service");
    }

    #[tokio::test]
    async fn storage_unauthenticated_returns_401() {
        thumbnail_common::test_fixtures::init_test_runtime_config("thumbnail-service-http-tests");
        let storage_addr = start_fos(StatResp::Err(tonic::Code::Unauthenticated)).await;
        let body = serde_json::json!({ "resource_address": "https://storage.test/image.png" });
        let resp = post_create(state_with_storage(storage_addr), body, Some("bad-token")).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    /// A storage service that accepts the Stat and stops answering must not hold
    /// the request open.
    ///
    /// Neither of the two bounds that existed covers this: connect timeout
    /// bounds establishing the connection, and HTTP/2 keepalive notices a peer
    /// that has gone away — this peer is alive and simply silent. Before the
    /// per-call deadline the handler waited on it indefinitely, occupying a
    /// server task the whole time.
    #[tokio::test]
    async fn a_stalled_storage_stat_does_not_hold_the_request_open() {
        thumbnail_common::test_fixtures::init_test_runtime_config("thumbnail-service-http-tests");
        let storage_addr = start_fos(StatResp::Stall).await;
        let body = serde_json::json!({ "resource_address": "https://storage.test/stalled.png" });

        // Generous relative to the 2s deadline the test fixture configures, so a
        // pass means the deadline fired rather than that the harness gave up.
        let started = std::time::Instant::now();
        let resp = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            post_create(state_with_storage(storage_addr), body, Some("test-token")),
        )
        .await
        .expect("the handler must return on its own deadline, not hang");

        assert!(
            started.elapsed() < std::time::Duration::from_secs(20),
            "returned only after {:?}; the per-call deadline should have fired far sooner",
            started.elapsed()
        );
        assert_eq!(
            resp.status(),
            StatusCode::BAD_GATEWAY,
            "a stalled dependency is an upstream fault, not a caller error"
        );
    }

    /// 403, not 401. The token was accepted and the authorization decision is
    /// what failed, so telling the caller to refresh invites a round trip that
    /// cannot succeed. The read path already made this distinction.
    #[tokio::test]
    async fn storage_permission_denied_returns_403() {
        thumbnail_common::test_fixtures::init_test_runtime_config("thumbnail-service-http-tests");
        let storage_addr = start_fos(StatResp::Err(tonic::Code::PermissionDenied)).await;
        let body = serde_json::json!({ "resource_address": "https://storage.test/forbidden.png" });
        let resp = post_create(state_with_storage(storage_addr), body, Some("test-token")).await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    // ── Generate dispatch tests ───────────────────────────────────────────────

    #[tokio::test]
    async fn full_success_returns_202_with_empty_failed_list() {
        let publisher = MockPublisherServer::start().await;
        let storage_addr = start_fos(StatResp::Ok("enc-id".to_string())).await;
        let body = serde_json::json!({ "resource_address": "https://storage.test/image.png" });

        let resp = post_create(
            state_with_storage_and_publisher(storage_addr, &publisher),
            body,
            Some("token"),
        )
        .await;

        assert_eq!(resp.status(), StatusCode::ACCEPTED);
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let body: CreateThumbnailsResponse = serde_json::from_slice(&bytes)
            .expect("full success must return the documented response body");
        assert!(body.failed.is_empty());

        let published = publisher
            .await_events(1, std::time::Duration::from_secs(2))
            .await;
        assert!(published
            .iter()
            .all(|event| event.event_type == thumbnail_common::GENERATE_EVENT_TYPE));

        publisher.shutdown().await;
    }

    #[tokio::test]
    async fn defaults_fan_out_can_report_more_failures_than_the_callers_empty_list() {
        let publisher = MockPublisherServer::start_with(PublisherBehavior::RejectAfter {
            successes: 1,
            reason: "injected defaults rejection".to_string(),
        })
        .await;
        let storage_addr = start_fos(StatResp::Ok("enc-id".to_string())).await;
        let mut state = state_with_storage_and_publisher(storage_addr, &publisher);
        let mut runtime = state.runtime.clone();
        runtime.thumbnail_defaults.thumbnail_sizes = "64x64,128x128,256x256".parse().unwrap();
        state.runtime = Box::leak(Box::new(runtime));
        let response = post_create(state, serde_json::json!({"resource_address": "https://storage.test/image.png", "specifications": []}), Some("token")).await;
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let body: CreateThumbnailsResponse =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(
            body.failed.len(),
            2,
            "two failures from three defaults despite zero caller specifications"
        );
        assert!(body.failed.iter().all(|failure| failure
            .error_message
            .contains("injected defaults rejection")));
        let events = publisher
            .await_events(3, std::time::Duration::from_secs(2))
            .await;
        assert_eq!(events.len(), 3);
        publisher.shutdown().await;
    }

    #[tokio::test]
    async fn all_dispatches_failed_returns_502() {
        let publisher =
            MockPublisherServer::start_with(PublisherBehavior::FailWith(tonic::Code::Internal))
                .await;
        let storage_addr = start_fos(StatResp::Ok("enc-id".to_string())).await;
        let body = serde_json::json!({ "resource_address": "https://storage.test/image.png" });

        let resp = post_create(
            state_with_storage_and_publisher(storage_addr, &publisher),
            body,
            Some("token"),
        )
        .await;

        assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);

        publisher.shutdown().await;
    }

    /// The API reports per-specification outcomes, unlike the CREATE path which
    /// has an acknowledgement to decide and no caller to tell. Migrating it to
    /// generate publishing must not collapse that into a batch result.
    #[tokio::test]
    async fn partial_failure_returns_202_with_failed_body() {
        // Valid specifications isolate an injected publisher rejection after one
        // successful publish, preserving the partial-dispatch response contract.
        let publisher = MockPublisherServer::start_with(PublisherBehavior::RejectAfter {
            successes: 1,
            reason: "mock publisher: injected partial rejection".to_string(),
        })
        .await;
        let storage_addr = start_fos(StatResp::Ok("enc-id".to_string())).await;

        let first = serde_json::json!({
            "image_to_image": { "encoding_format": "PNG", "width": 256, "height": 256, "transform_type": "SQUARE", "crop_strategy": "CENTER" }
        });
        let second = serde_json::json!({
            "image_to_image": { "encoding_format": "PNG", "width": 512, "height": 512, "transform_type": "SQUARE", "crop_strategy": "CENTER" }
        });
        let body = serde_json::json!({
            "resource_address": "https://storage.test/image.png",
            "specifications": [first, second]
        });

        let resp = post_create(
            state_with_storage_and_publisher(storage_addr, &publisher),
            body,
            Some("token"),
        )
        .await;

        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        assert_eq!(
            status,
            StatusCode::ACCEPTED,
            "body: {}",
            String::from_utf8_lossy(&bytes)
        );
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let failed = json["failed"].as_array().expect("failed array");
        assert_eq!(failed.len(), 1, "one specification should have failed");
        assert!(failed[0]["specification"]["image_to_image"].is_object());
        assert!(!failed[0]["error_message"].as_str().unwrap_or("").is_empty());

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

/// The readiness probe advertises no credential and only the statuses it can
/// return.
///
/// It declared bearer auth plus 401, 429 and 500 while the handler reads one flag
/// and returns 200 or 503. A gateway generated from that document would conclude
/// the readiness probe needs a token — and the deployed probes, which call it
/// without one, prove it does not.
#[tokio::test]
async fn health_check_advertises_no_credential_and_only_reachable_statuses() {
    let document =
        serde_json::to_value(ApiDoc::openapi()).expect("the OpenAPI document must serialize");
    let health = &document["paths"]["/health"]["get"];

    assert!(
        health["security"].is_null(),
        "the readiness probe must not declare a credential it does not require: {}",
        health["security"]
    );

    let responses = health["responses"].as_object().expect("health responses");
    let mut declared: Vec<&String> = responses.keys().collect();
    declared.sort();
    assert_eq!(
        declared,
        vec!["200", "503"],
        "the handler can only answer healthy or unhealthy"
    );
}

// --- CORS ------------------------------------------------------------------

/// Enabling CORS must not mean permitting everything.
///
/// The layer used to allow any origin and any header, and to permit `PUT`, which
/// no route serves. CORS is off by default so this was latent rather than live —
/// but it is the configuration an operator inherits by turning it on.
#[test]
fn cors_permits_only_configured_origins_and_served_methods() {
    use axum::http::{header, HeaderValue, Method};

    let layer = cors_layer(&["https://console.example".to_string()]);
    // `CorsLayer` exposes no getters, so assert through its Debug shape: enough
    // to catch a reintroduced `Any` or a resurrected PUT, which is what matters.
    let shape = format!("{layer:?}");

    assert!(
        shape.contains("console.example"),
        "the configured origin must reach the layer: {shape}"
    );
    assert!(
        !shape.contains("Any"),
        "no dimension of the policy may be Any: {shape}"
    );
    assert!(
        !shape.contains("PUT"),
        "PUT is served by no route and must not be permitted: {shape}"
    );

    // Named so the assertions above cannot pass by the constants being unused.
    let _ = (
        Method::GET,
        Method::POST,
        Method::DELETE,
        header::AUTHORIZATION,
        HeaderValue::from_static("x"),
    );
}

/// No configured origins permits none, rather than defaulting to permissive.
#[test]
fn cors_with_no_configured_origins_permits_nothing() {
    let shape = format!("{:?}", cors_layer(&[]));

    assert!(
        !shape.contains("Any"),
        "an unconfigured CORS setting must not fall back to permitting everything: {shape}"
    );
}

// --- auth precedence over payload parsing -----------------------------------

/// An unauthenticated caller with a malformed body gets 401, not a 400 that
/// describes the schema.
///
/// The existing `associate_endpoint_rejects_missing_auth` calls the handler with
/// a pre-parsed `Ok(Json(..))`, so it never exercised the extractor and could not
/// have caught this ordering either way. These go through the router.
#[tokio::test]
async fn associate_route_rejects_missing_auth_before_parsing_the_body() {
    let response = Router::new()
        .route("/v1alpha/associations", post(associate_thumbnail))
        .with_state(test_app_state())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1alpha/associations")
                .header("content-type", "application/json")
                .body(Body::from("{ this is not json"))
                .expect("request"),
        )
        .await
        .expect("router response");

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "the missing credential is the relevant fault, not the parse error"
    );
    let (message, _) = error_message_and_source(response).await;
    assert!(
        !message.contains("JSON"),
        "an unauthenticated caller must not be told what the body should look like: {message}"
    );
}

#[tokio::test]
async fn create_thumbnails_route_rejects_missing_auth_before_parsing_the_body() {
    let response = Router::new()
        .route("/v1alpha/thumbnails", post(create_thumbnails_handler))
        // Any state works: the request is refused at the credential check before
        // anything reads it.
        .with_state(test_app_state())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1alpha/thumbnails")
                .header("content-type", "application/json")
                .body(Body::from("{ this is not json"))
                .expect("request"),
        )
        .await
        .expect("router response");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let (message, _) = error_message_and_source(response).await;
    assert!(
        !message.contains("JSON"),
        "an unauthenticated caller must not be told what the body should look like: {message}"
    );
}

/// With a credential, a malformed body still reports the parse error — the
/// reordering must not have swallowed it.
#[tokio::test]
async fn a_malformed_body_with_a_credential_still_reports_the_parse_error() {
    let response = post_association_body("{ this is not json").await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let (message, _) = error_message_and_source(response).await;
    assert!(
        message.contains("JSON"),
        "an authenticated caller should still learn what was wrong: {message}"
    );
}
