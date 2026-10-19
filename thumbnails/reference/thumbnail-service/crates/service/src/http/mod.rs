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

use axum::http::{header, HeaderValue};
use axum::{
    extract::{
        rejection::{JsonRejection, QueryRejection},
        Query, State,
    },
    http::{HeaderMap, Method, StatusCode},
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Router,
};
use axum_observability::server::metric::TowerMetricLayer;
use axum_observability::server::trace::TowerTraceLayer;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tower_http::cors::CorsLayer;
use tower_http::timeout::TimeoutLayer;

use thumbnail_common::{
    create_thumbnails, disassociate_thumbnail as run_disassociate,
    get_thumbnails_for_resource_with_details, store_association, AssociationError,
    AssociationFailure, AssociationRequest, AssociationResource, AssociationSpec,
    AssociationTargetKind, CreateThumbnailsError, DisassociateAbort, DisassociateOutcome,
    DisassociateRequest, OutboundClients, RuntimeConfigArgs,
};
use tracing::{error, info, warn};

/// Most entries a `GetThumbnails` response carries, on either transport.
///
/// `pub(crate)` because the gRPC handler answers the same query and must truncate
/// at the same point — a response of exactly this many entries may be incomplete,
/// and a caller cannot page past it, so the two transports disagreeing about where
/// the list stops would be a silent difference in what data exists.
pub(crate) const MAX_GET_THUMBNAILS_RESULTS: usize = 1000;

/// Server-side ceiling on how long any request may occupy a task.
///
/// A backstop, not the primary bound: the outbound `Stat` and publish calls each
/// carry their own configured deadline (`OUTBOUND_REQUEST_TIMEOUT`,
/// `STORAGE_GRPC_REQUEST_TIMEOUT`), and those should fire first so the response
/// names the dependency that stalled. This catches what they cannot — a handler
/// making several bounded calls in sequence, or a future call site that forgets
/// its own deadline.
///
/// A literal rather than a config read: `app()` is constructed directly by tests
/// that do not install a runtime config, and coupling router construction to
/// global state to express a coarse safety net is a poor trade. It is set well
/// above any per-call deadline so it never pre-empts a more specific error.
const REQUEST_TIMEOUT_BACKSTOP: Duration = Duration::from_secs(120);

/// Single source of truth for the public REST API version, reused in the
/// `#[utoipa::path]` route declarations, the axum router, and advertised by the
/// discovery capabilities API. A macro rather than a `const` because
/// `#[utoipa::path]` needs the version at attribute-expansion time, where only a
/// literal will do.
macro_rules! rest_api_version {
    () => {
        "v1alpha"
    };
}
pub(crate) use rest_api_version;

/// Axum state injected into endpoints that need outbound-gRPC plumbing.
#[derive(Clone)]
pub struct AppState {
    pub outbound: Arc<OutboundClients>,
    pub runtime: &'static RuntimeConfigArgs,
    /// Shared readiness signal set by the gRPC server task: `true` once it has
    /// successfully bound and is serving, `false` before that and after it
    /// exits for any reason. Gates `/health` (readiness) so traffic is not
    /// routed until both HTTP and gRPC sides of the unified process are up.
    /// Kubernetes liveness uses dependency-free `/health/live` instead.
    pub grpc_healthy: Arc<AtomicBool>,
}

use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

mod types;
pub use types::*;

#[cfg(test)]
pub(crate) mod tests;

// HTTP routes accept stringly typed query/body data, so normalize target kinds once and reuse the
// same shared enum as the storage-association helpers in `thumbnail_common`.
impl From<TargetType> for AssociationTargetKind {
    fn from(t: TargetType) -> Self {
        match t {
            TargetType::File => AssociationTargetKind::File,
            TargetType::Folder => AssociationTargetKind::Folder,
        }
    }
}

impl From<AssociationTargetKind> for TargetType {
    fn from(kind: AssociationTargetKind) -> Self {
        match kind {
            AssociationTargetKind::File => TargetType::File,
            AssociationTargetKind::Folder => TargetType::Folder,
        }
    }
}

// The task queue expects the raw bearer token, while Axum exposes the whole header value.
fn extract_auth_token(headers: &axum::http::HeaderMap) -> Option<String> {
    headers
        .get("Authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(crate::bearer_token_from_header)
}

/// The HTTP status for a targeted disassociate result.
///
/// `200` when the thumbnail was removed or was not associated (idempotent). `502` when
/// the removal failed or the association target could not be enumerated. An abort the
/// caller can act on keeps its own code: a rejected token is `401` and a bad request
/// is `400`, because `502` would send them looking for an outage that is not there.
fn disassociate_http_status(outcome: &DisassociateOutcome) -> StatusCode {
    if let Some(abort) = &outcome.aborted {
        return match abort {
            // The handler validates first, so this is the function's own backstop firing.
            DisassociateAbort::Validation(_) => StatusCode::BAD_REQUEST,
            _ if abort.is_unauthenticated() => StatusCode::UNAUTHORIZED,
            _ => StatusCode::BAD_GATEWAY,
        };
    }

    let removal_failed = outcome.succeeded.is_empty() && !outcome.failed.is_empty();
    if removal_failed {
        StatusCode::BAD_GATEWAY
    } else {
        StatusCode::OK
    }
}

/// Dependency-free liveness probe: returns 200 whenever the HTTP server is
/// accepting requests. Does not check gRPC or any outbound dependency so
/// Kubernetes liveness cannot restart the pod during transient gRPC hiccups
/// or graceful drain.
#[utoipa::path(
    get,
    path = "/health/live",
    operation_id = "liveHealthCheck",
    responses(
        (status = 200, description = "Process is alive", body = HealthCheckResponse),
    ),
    tag = "status"
)]
pub async fn live_health_check() -> impl IntoResponse {
    Json(HealthCheckResponse {
        status: "OK".to_string(),
    })
}

/// Readiness probe: the unified process is ready to serve traffic only when
/// both HTTP and gRPC are bound and serving.
#[utoipa::path(
    get,
    path = "/health",
    operation_id = "healthCheck",
    responses(
        (status = 200, description = "Service is ready (HTTP and gRPC both serving)", body = HealthCheckResponse),
        (status = 503, description = "gRPC side of the unified process is not currently serving", body = HealthCheckResponse)
    ),
    tag = "status"
)]
pub async fn health_check(State(state): State<AppState>) -> Response {
    if !state.grpc_healthy.load(Ordering::Relaxed) {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(HealthCheckResponse {
                status: "GRPC_UNAVAILABLE".to_string(),
            }),
        )
            .into_response();
    }

    Json(HealthCheckResponse {
        status: "OK".to_string(),
    })
    .into_response()
}

#[utoipa::path(
    post,
    path = concat!("/", rest_api_version!(), "/associations"),
    operation_id = "associateThumbnail",
    request_body(
        content = AssociationRequest,
        description = "Thumbnail and association target resources to link",
        content_type = "application/json"
    ),
    responses(
        (status = 204, description = "Thumbnail successfully associated with the source resource. No body: the association is readable from GET /v1alpha/thumbnails"),
        (status = 400, description = "Invalid request - malformed resources or specification", body = ErrorResponse),
        (status = 401, description = "No Bearer token was provided, or the downstream storage service rejected the one that was (for example, an expired token)", body = ErrorResponse),
        (status = 500, description = "Internal server error occurred while creating association", body = ErrorResponse)
    ),
    security(
        ("bearer_auth" = [])
    ),
    tag = "thumbnails"
)]
pub async fn associate_thumbnail(
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: Result<Json<AssociationRequest>, JsonRejection>,
) -> Response {
    // Authentication before the body. An unauthenticated caller sending a
    // malformed payload otherwise gets a 400 describing the schema the service
    // expected — handing structural detail to someone who has not authenticated,
    // and reporting the less relevant of two faults. `get_thumbnails` and
    // `disassociate_thumbnail` already ordered it this way.
    let Some(auth_token) = extract_auth_token(&headers) else {
        return unauthorized_from_headers(&headers);
    };

    let Json(req) = match payload {
        Ok(request) => request,
        Err(rejection @ JsonRejection::JsonDataError(_))
        | Err(rejection @ JsonRejection::JsonSyntaxError(_)) => {
            return bad_request(format!(
                "invalid JSON body: {}",
                json_rejection_message(&rejection)
            ));
        }
        Err(rejection) => return rejection.into_response(),
    };

    if let Err(e) = req.validate() {
        return bad_request(e.to_string());
    }

    info!(
        "associate_thumbnail: association_target={} thumbnail={} target_kind={:?}",
        req.association_target.resource_address, req.thumbnail.resource_address, req.target_kind
    );

    match store_association(&req, &auth_token, state.outbound.clone()).await {
        Ok(()) => {
            info!("Successfully associated thumbnail");
            // The old body echoed the request back. Nothing read it, and a caller that
            // wants the association can read it back from GET /v1alpha/thumbnails.
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => {
            error!("Failed to associate thumbnail: {}", e);
            (
                associate_error_status(&e),
                Json(ErrorResponse {
                    source: "thumbnail-service".to_string(),
                    error_message: format!("Failed to associate thumbnail: {}", e),
                    details: None,
                }),
            )
                .into_response()
        }
    }
}

/// The single place an association failure becomes an HTTP status.
///
/// Every variant is a `500` today, matching the behavior before association
/// errors were typed; section 9 of `thumbnail-association-runtime-hardening`
/// splits authentication failures out to `401`.
fn associate_error_status(error: &AssociationError) -> StatusCode {
    match error.failure() {
        // The caller's token was rejected downstream, typically because it expired.
        // Reporting 500 would invite them to retry a request that cannot succeed.
        AssociationFailure::Unauthenticated => StatusCode::UNAUTHORIZED,
        AssociationFailure::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn json_rejection_message(rejection: &JsonRejection) -> String {
    let mut message = rejection.to_string();
    let mut source = std::error::Error::source(rejection);
    while let Some(error) = source {
        let detail = error.to_string();
        if !detail.is_empty() && !message.contains(&detail) {
            message.push_str(": ");
            message.push_str(&detail);
        }
        source = error.source();
    }
    message
}

/// Helper for 400 responses with a thumbnail-service source tag.
fn bad_request(message: String) -> Response {
    const MAX_ERROR_MESSAGE_CHARS: usize = 4096;
    let message = if message.chars().count() > MAX_ERROR_MESSAGE_CHARS {
        let mut truncated = message
            .chars()
            .take(MAX_ERROR_MESSAGE_CHARS - 3)
            .collect::<String>();
        truncated.push_str("...");
        truncated
    } else {
        message
    };

    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse {
            source: "thumbnail-service".to_string(),
            error_message: message,
            details: None,
        }),
    )
        .into_response()
}

fn unauthorized_from_headers(headers: &HeaderMap) -> Response {
    let error_msg = if headers.get("Authorization").is_some() {
        "Invalid Authorization header format. Expected 'Bearer <token>'"
    } else {
        "Missing Authorization header. Include 'Authorization: Bearer <your-token>' in request headers"
    };

    (
        StatusCode::UNAUTHORIZED,
        Json(ErrorResponse {
            source: "thumbnail-service".to_string(),
            error_message: error_msg.to_string(),
            details: None,
        }),
    )
        .into_response()
}

#[utoipa::path(
    delete,
    path = concat!("/", rest_api_version!(), "/associations"),
    operation_id = "disassociateThumbnail",
    params(DisassociateRequest),
    responses(
        (status = 200, description = "Thumbnail removed, or was not associated with the target (idempotent). `succeeded` carries the removed thumbnail; both arrays are empty when the thumbnail was not found.", body = DisassociateThumbnailResponse),
        (status = 400, description = "Invalid request - missing or unparseable query params", body = ErrorResponse),
        (status = 401, description = "No Bearer token was provided, or the downstream storage service rejected the one that was (for example, an expired token)", body = ErrorResponse),
        (status = 502, description = "Removal failed, or the association target's metadata could not be enumerated. When the removal was attempted, `failed` carries the error detail; when enumeration itself failed, both arrays are empty.", body = DisassociateThumbnailResponse)
    ),
    security(
        ("bearer_auth" = [])
    ),
    tag = "thumbnails"
)]
pub async fn disassociate_thumbnail(
    State(state): State<AppState>,
    headers: HeaderMap,
    params: Result<Query<DisassociateRequest>, QueryRejection>,
) -> Response {
    let Some(auth_token) = extract_auth_token(&headers) else {
        return unauthorized_from_headers(&headers);
    };

    // An unparseable query never reaches the handler body, so without this the
    // extractor answers in plain text and the documented ErrorResponse shape only
    // holds for the requests that happen to deserialize.
    let Query(req) = match params {
        Ok(query) => query,
        Err(rejection) => return bad_request(format!("invalid query: {}", rejection.body_text())),
    };

    if let Err(e) = req.validate() {
        return bad_request(e.to_string());
    }

    let outcome = run_disassociate(&req, &auth_token, state.outbound.clone()).await;
    let status = disassociate_http_status(&outcome);

    if matches!(status, StatusCode::BAD_REQUEST | StatusCode::UNAUTHORIZED) {
        let error_message = outcome
            .aborted
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| "Disassociate was rejected".to_string());
        return (
            status,
            Json(ErrorResponse {
                source: "thumbnail-service".to_string(),
                error_message,
                details: None,
            }),
        )
            .into_response();
    }

    let body = DisassociateThumbnailResponse {
        succeeded: outcome
            .succeeded
            .iter()
            .map(|t| Resource {
                resource_address: t.resource_address.clone(),
                resource_identity: t.resource_identity.clone(),
            })
            .collect(),
        failed: outcome
            .failed
            .iter()
            .map(|failure| DisassociateFailure {
                resource_address: failure.thumbnail.resource_address.clone(),
                resource_identity: failure.thumbnail.resource_identity.clone(),
                error: failure.error.to_string(),
            })
            .collect(),
    };

    (status, Json(body)).into_response()
}

#[derive(OpenApi)]
#[openapi(
    paths(
        get_thumbnails,
        create_thumbnails_handler,
        associate_thumbnail,
        disassociate_thumbnail,
        live_health_check,
        health_check,
    ),
    components(
        schemas(
            Resource,
            GetThumbnailsRequest,
            GetThumbnailsResponse,
            ThumbnailEntry,
            CreateThumbnailsRequest,
            CreateThumbnailsResponse,
            CreateThumbnailsFailure,
            AssociationRequest,
            AssociationResource,
            AssociationSpec,
            AssociationTargetKind,
            thumbnail_common::EncodingFormat,
            DisassociateRequest,
            DisassociateThumbnailResponse,
            DisassociateFailure,
            ThumbnailSpecification,
            ImageToImageSpecification,
            TargetType,
            ThumbnailSpecificationType,
            TransformType,
            CropStrategy,
            ImageEncodingFormat,
            HealthCheckResponse,
            ErrorResponse,
        )
    ),
    modifiers(&SecurityAddon, &ParamSchemaAddon, &AssociationSpecDiscriminatorAddon),
    tags(
        (name = "thumbnails", description = "Core thumbnail operations"),
        (name = "status", description = "Service Status"),
    )
)]
pub struct ApiDoc;

/// utoipa 5 dropped the `discriminator` field from internally-tagged enums
/// (`#[serde(tag = "type")]`). Restore it on `AssociationSpec` so the generated
/// OpenAPI 3.1.0 spec retains the `discriminator: {propertyName: type}` that
/// client generators and the storage sibling repos expect on the `oneOf`.
struct AssociationSpecDiscriminatorAddon;

impl utoipa::Modify for AssociationSpecDiscriminatorAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        use utoipa::openapi::schema::{Discriminator, Schema};
        use utoipa::openapi::RefOr;
        if let Some(RefOr::T(Schema::OneOf(one_of))) = openapi
            .components
            .as_mut()
            .and_then(|c| c.schemas.get_mut("AssociationSpec"))
        {
            one_of.discriminator = Some(Discriminator::new("type"));
        }
    }
}

struct ParamSchemaAddon;

impl utoipa::Modify for ParamSchemaAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        use utoipa::openapi::schema::{ObjectBuilder, Schema, Type};
        use utoipa::openapi::RefOr;

        let string_schema = |max_len: usize| -> Option<RefOr<Schema>> {
            Some(RefOr::T(Schema::Object(
                ObjectBuilder::new()
                    .schema_type(Type::String)
                    .max_length(Some(max_len))
                    .build(),
            )))
        };

        let constraints: &[(&str, usize)] = &[("thumbnail_resource_identity", 2048)];

        for path_item in openapi.paths.paths.values_mut() {
            // utoipa 5.x: operations are individual Option<Operation> fields per HTTP method
            let ops = [
                path_item.get.as_mut(),
                path_item.put.as_mut(),
                path_item.post.as_mut(),
                path_item.delete.as_mut(),
                path_item.options.as_mut(),
                path_item.head.as_mut(),
                path_item.patch.as_mut(),
                path_item.trace.as_mut(),
            ];
            for op in ops.into_iter().flatten() {
                if let Some(params) = op.parameters.as_mut() {
                    for param in params.iter_mut() {
                        for &(name, max_len) in constraints {
                            if param.name == name {
                                param.schema = string_schema(max_len);
                            }
                        }
                    }
                }
            }
        }
    }
}

struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
            components.add_security_scheme(
                "bearer_auth",
                SecurityScheme::Http(
                    HttpBuilder::new()
                        .scheme(HttpAuthScheme::Bearer)
                        .bearer_format("JWT")
                        .description(Some(
                            "Bearer token authentication compliant with RFC8725 JWT best practices. \
                             Token validation is delegated to upstream/downstream services.",
                        ))
                        .build(),
                ),
            );
        }
    }
}

#[utoipa::path(
    get,
    path = concat!("/", rest_api_version!(), "/thumbnails"),
    operation_id = "getThumbnails",
    params(
        ("association_target_resource_address" = String, Query, description = "The resource_address to retrieve thumbnails for", max_length = 2048),
        ("specification" = Option<ThumbnailSpecificationType>, Query, description = "Optional thumbnail specification type filter; currently only IMAGE_TO_IMAGE is supported"),
        ("width" = Option<u32>, Query, description = "Optional image-to-image filter for an exact width in pixels", minimum = 1, maximum = 8192),
        ("height" = Option<u32>, Query, description = "Optional image-to-image filter for an exact height in pixels", minimum = 1, maximum = 8192),
        ("encoding_format" = Option<ImageEncodingFormat>, Query, description = "Optional image-to-image encoding format filter (PNG, JPEG, GIF, WEBP)"),
        ("transform_type" = Option<TransformType>, Query, description = "Optional image-to-image transform type filter (SQUARE, FIT, CROP, etc.)"),
        ("crop_strategy" = Option<CropStrategy>, Query, description = "Optional image-to-image crop direction filter (CENTER, TOP, BOTTOM, etc.)"),
    ),
    responses(
        (status = 200, description = "Successfully retrieved thumbnails for the specified resource; thumbnails is empty when none exist. Results are ordered by thumbnail resource_address and capped at 1000 entries: a target with more than 1000 thumbnails has the remainder discarded, not paginated. There is no continuation token, so a response of exactly 1000 entries may be incomplete.", body = GetThumbnailsResponse),
        (status = 400, description = "Invalid or unsupported query parameters", body = ErrorResponse),
        (status = 401, description = "Authentication required - valid Bearer token must be provided", body = ErrorResponse),
        (status = 403, description = "The authenticated caller is not authorized to read the requested resource", body = ErrorResponse),
        (status = 502, description = "The storage service could not complete the metadata query", body = ErrorResponse)
    ),
    security(
        ("bearer_auth" = [])
    ),
    tag = "thumbnails"
)]
pub async fn get_thumbnails(
    headers: HeaderMap,
    params: Result<Query<GetThumbnailsRequest>, QueryRejection>,
) -> Response {
    let Some(auth_token) = extract_auth_token(&headers) else {
        return unauthorized_from_headers(&headers);
    };

    let Query(params) = match params {
        Ok(query) => query,
        Err(rejection) => return bad_request(format!("invalid query: {}", rejection.body_text())),
    };
    if let Err(error) = params.validate() {
        return bad_request(format!("invalid query: {error}"));
    }

    info!("Getting thumbnails");

    let filters = thumbnail_common::ThumbnailFilters::from(&params);

    // Associations are written keyed on the target's resource_address (ADR 0002 §4),
    // so reads must use the same key. Both FILE and FOLDER targets read metadata
    // directly on the supplied address — no identity resolution — to keep read and
    // write keys consistent.
    let metadata_uri = &params.association_target_resource_address;

    match get_thumbnails_for_resource_with_details(metadata_uri, &filters, &auth_token, None).await
    {
        Ok(matches) => {
            info!("Found {} thumbnails for resource", matches.len());
            let thumbnails = thumbnail_entries_from_matches(
                &params.association_target_resource_address,
                matches,
            );
            Json(GetThumbnailsResponse { thumbnails }).into_response()
        }
        Err(thumbnail_common::ThumbnailQueryError::NotFound) => {
            Json(GetThumbnailsResponse { thumbnails: vec![] }).into_response()
        }
        Err(e) => {
            let (status, error_message) = match e {
                // Handled by the arm above, so this is dead today. Answering
                // like that arm rather than panicking: the panic is only correct
                // for as long as the outer match keeps its NotFound case, and
                // being wrong about that turns a caller's request into an abort.
                thumbnail_common::ThumbnailQueryError::NotFound => {
                    debug_assert!(false, "NotFound is handled before this match");
                    return Json(GetThumbnailsResponse { thumbnails: vec![] }).into_response();
                }
                thumbnail_common::ThumbnailQueryError::Unauthenticated => (
                    StatusCode::UNAUTHORIZED,
                    "Authentication token was rejected by storage",
                ),
                thumbnail_common::ThumbnailQueryError::PermissionDenied => (
                    StatusCode::FORBIDDEN,
                    "Access to the requested resource was denied",
                ),
                thumbnail_common::ThumbnailQueryError::Storage(_) => {
                    (StatusCode::BAD_GATEWAY, "Storage service is unavailable")
                }
            };
            error!(status = status.as_u16(), "Failed to get thumbnails");
            (
                status,
                Json(ErrorResponse {
                    source: "thumbnail-service".to_string(),
                    error_message: error_message.to_string(),
                    details: None,
                }),
            )
                .into_response()
        }
    }
}

/// Turn the storage query's matches into the entries a `GetThumbnails` response
/// carries: skipping what cannot be read, ordered by thumbnail address, capped.
///
/// Shared by both transports rather than reimplemented per transport. The gRPC
/// handler renders these into its own proto messages, so the skip rule, the sort
/// key and the cap are decided once — the alternative is two copies of this loop
/// quietly disagreeing about which stored entries are returnable, which is the
/// class of divergence this change exists to close.
pub(crate) fn thumbnail_entries_from_matches(
    association_target_resource_address: &str,
    matches: Vec<thumbnail_common::ThumbnailMatchInfo>,
) -> Vec<ThumbnailEntry> {
    let mut thumbnails = Vec::with_capacity(matches.len());
    // Entries too malformed to return. Counted rather than fatal: one
    // legacy or half-written record used to make every other thumbnail on
    // the same target unreadable, with nothing in the response to say
    // which one was at fault or how to route around it. The disassociate
    // sweep already treats such a record as one failed entry rather than
    // as a reason to abandon the run.
    let mut skipped_keys: Vec<String> = Vec::new();

    for m in matches {
        let thumbnail_resource_identity = m.thumbnail_resource_identity.trim().to_string();
        let thumbnail_resource_address = m
            .thumbnail_resource_address
            .as_deref()
            .map(str::trim)
            .filter(|address| !address.is_empty())
            .map(str::to_string);
        let Some(thumbnail_resource_address) = thumbnail_resource_address else {
            warn!(
                metadata_key = %m.metadata_key,
                "Skipping thumbnail metadata entry with no thumbnail_resource_address; \
                 it cannot be returned and the remaining entries are unaffected"
            );
            skipped_keys.push(m.metadata_key);
            continue;
        };

        let association_target_resource_identity = m.association_target_resource_identity;
        let target_type = TargetType::from(m.target_type.as_deref());
        // Return the addresses/identities as stored in the association
        // metadata rather than re-deriving them from request params.
        thumbnails.push(ThumbnailEntry {
            target_type,
            association_target: Resource {
                resource_address: association_target_resource_address.to_string(),
                resource_identity: association_target_resource_identity,
            },
            thumbnail: Resource {
                resource_address: thumbnail_resource_address,
                resource_identity: if thumbnail_resource_identity.is_empty() {
                    None
                } else {
                    Some(thumbnail_resource_identity)
                },
            },
            specification: m.specification,
            create_time: m.create_time,
        });
    }

    thumbnails.sort_by(|left, right| {
        left.thumbnail
            .resource_address
            .cmp(&right.thumbnail.resource_address)
    });
    thumbnails.truncate(MAX_GET_THUMBNAILS_RESULTS);

    if !skipped_keys.is_empty() {
        // Summary as well as the per-entry warnings: a target where every
        // entry is unreadable otherwise looks exactly like a target with
        // no thumbnails, and an empty success is the least investigable
        // response there is.
        warn!(
            returned = thumbnails.len(),
            skipped = skipped_keys.len(),
            skipped_keys = %skipped_keys.join(","),
            "Returned a partial thumbnail list: some stored entries could not be read"
        );
    }

    thumbnails
}

/// Convert one HTTP `ThumbnailSpecification` to the common `ImageProcessingSpec`.
///
/// Only `image_to_image` entries are accepted; anything else returns an error string
/// describing the unsupported type so the caller can surface it as a 400.
fn http_spec_to_processing_spec(
    spec: &ThumbnailSpecification,
) -> Result<thumbnail_common::ImageProcessingSpec, String> {
    use thumbnail_common::{
        CropStrategy as CommonCropStrategy, EncodingFormat as CommonEncodingFormat,
        TransformType as CommonTransformType,
    };

    let i2i = spec.image_to_image.as_ref().ok_or_else(|| {
        "only image_to_image specifications are supported for CreateThumbnails; \
         the image_to_image field must be set"
            .to_string()
    })?;

    let encoding_format = CommonEncodingFormat::from(i2i.encoding_format);
    let transform_type = CommonTransformType::from(i2i.transform_type);
    let crop_strategy = CommonCropStrategy::from(i2i.crop_strategy);

    // Resolve quality from config when not supplied by the caller, so that
    // build_generation_task_request can always render the destination template.
    let quality = i2i.quality.or_else(|| {
        Some(
            thumbnail_common::current_runtime_config()
                .thumbnail_defaults
                .thumbnail_quality as i32,
        )
    });

    Ok(thumbnail_common::ImageProcessingSpec {
        width: Some(i2i.width),
        height: Some(i2i.height),
        encoding_format: Some(encoding_format),
        transform_type: Some(transform_type),
        crop_strategy: Some(crop_strategy),
        quality,
        // Passed through with presence intact rather than resolved here: an
        // omitted value is filled from configuration by the shared dispatch
        // seam, which is what makes an explicit `false` distinguishable from
        // "not supplied". Hardcoding `None` here meant the caller could not
        // express either.
        preserve_aspect_ratio: i2i.preserve_aspect_ratio,
    })
}

/// Map a `CreateThumbnailsError` to the appropriate HTTP status code.
fn create_thumbnails_error_status(e: &CreateThumbnailsError) -> axum::http::StatusCode {
    match e {
        CreateThumbnailsError::InvalidArgument(_) => axum::http::StatusCode::BAD_REQUEST,
        CreateThumbnailsError::NotFound(_) => axum::http::StatusCode::NOT_FOUND,
        CreateThumbnailsError::Unauthenticated(_) => axum::http::StatusCode::UNAUTHORIZED,
        CreateThumbnailsError::PermissionDenied(_) => axum::http::StatusCode::FORBIDDEN,
        CreateThumbnailsError::AllTasksFailed(_) => axum::http::StatusCode::BAD_GATEWAY,
    }
}

#[utoipa::path(
    post,
    path = concat!("/", rest_api_version!(), "/thumbnails"),
    operation_id = "createThumbnails",
    request_body(
        content = CreateThumbnailsRequest,
        description = "Resource address and optional thumbnail specifications",
        content_type = "application/json"
    ),
    responses(
        (status = 202, description = "Thumbnail generation accepted. `failed` is empty on full acceptance and lists specifications whose dispatch failed after request validation, including publisher failures. At least one specification was accepted; a call where none was returns 502. Do not compare `failed.length` to the request's `specifications.length`: absent or empty specifications fan out over the configured THUMBNAIL_SIZES. Acceptance does not prove generation completes.", body = CreateThumbnailsResponse),
        (status = 400, description = "Invalid request — a blank resource_address, more than 1000 specifications, a specification carrying no image_to_image, a width/height outside [1, 8192], or JPEG/WebP quality outside [1, 100]. Refused before any storage call.", body = ErrorResponse),
        (status = 401, description = "No Bearer token was provided, or the downstream storage service rejected the one that was", body = ErrorResponse),
        (status = 403, description = "The token was accepted and the caller is not authorized to read resource_address. Distinct from 401: a new token will not help.", body = ErrorResponse),
        (status = 404, description = "resource_address was not found in storage", body = ErrorResponse),
        (status = 502, description = "An upstream source Stat fault occurred, or every thumbnail dispatch failed. Missing source and access/authentication failures retain their separate 404/403/401 responses. A failed Stat submits nothing. A failed publish is not a guarantee that nothing was enqueued: the broker may have accepted it before its deadline.", body = ErrorResponse),
    ),
    security(
        ("bearer_auth" = [])
    ),
    tag = "thumbnails"
)]
pub async fn create_thumbnails_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: Result<Json<CreateThumbnailsRequest>, JsonRejection>,
) -> Response {
    // Authentication before the body, matching every other endpoint. See the
    // note on `associate_thumbnail`.
    let Some(auth_token) = extract_auth_token(&headers) else {
        return unauthorized_from_headers(&headers);
    };

    let Json(req) = match payload {
        Ok(r) => r,
        Err(rejection @ JsonRejection::JsonDataError(_))
        | Err(rejection @ JsonRejection::JsonSyntaxError(_)) => {
            return bad_request(format!(
                "invalid JSON body: {}",
                json_rejection_message(&rejection)
            ));
        }
        Err(rejection) => return rejection.into_response(),
    };

    // Range-check dimensions and JPEG/WebP quality before the Stat, so an out-of-range integer is a
    // caller error rather than an upstream-fault status paid for with a storage
    // round trip.
    if let Err(error) = req.validate() {
        return bad_request(error.to_string());
    }

    // Validate and convert specifications before any outbound call.
    let specs: Vec<thumbnail_common::ImageProcessingSpec> = {
        let raw = req.specifications.unwrap_or_default();
        if raw.len() > thumbnail_common::work_events::MAX_CREATE_THUMBNAIL_SPECIFICATIONS {
            return bad_request(format!(
                "specifications must contain at most {} entries",
                thumbnail_common::work_events::MAX_CREATE_THUMBNAIL_SPECIFICATIONS
            ));
        }
        if raw.is_empty() {
            vec![]
        } else {
            let mut converted = Vec::with_capacity(raw.len());
            for (i, spec) in raw.iter().enumerate() {
                match http_spec_to_processing_spec(spec) {
                    Ok(s) => converted.push(s),
                    Err(msg) => {
                        return bad_request(format!("specifications[{i}]: {msg}"));
                    }
                }
            }
            converted
        }
    };

    info!(
        resource_address = %req.resource_address,
        spec_count = specs.len(),
        "create_thumbnails: initiating thumbnail generation"
    );

    match create_thumbnails(
        &req.resource_address,
        &specs,
        &auth_token,
        state.outbound.clone(),
        state.runtime,
    )
    .await
    {
        Ok(outcome) if outcome.failed.is_empty() => {
            // Full success uses the same documented response shape as partial acceptance.
            (
                StatusCode::ACCEPTED,
                Json(CreateThumbnailsResponse { failed: vec![] }),
            )
                .into_response()
        }
        Ok(outcome) => {
            // Partial failure: 202 with a body listing the failures.
            info!(
                failed_count = outcome.failed.len(),
                "create_thumbnails: partial task queue failure"
            );
            let body = CreateThumbnailsResponse {
                failed: outcome
                    .failed
                    .iter()
                    .map(|f| {
                        // Convert ImageProcessingSpec back to HTTP ThumbnailSpecification
                        // so the response mirrors what the caller sent (or what defaults produced).
                        CreateThumbnailsFailure {
                            specification: processing_spec_to_http_spec(&f.specification),
                            error_message: f.error_message.clone(),
                        }
                    })
                    .collect(),
            };
            (StatusCode::ACCEPTED, Json(body)).into_response()
        }
        Err(e) => {
            error!("create_thumbnails failed: {e}");
            (
                create_thumbnails_error_status(&e),
                Json(ErrorResponse {
                    source: "thumbnail-service".to_string(),
                    error_message: e.to_string(),
                    details: None,
                }),
            )
                .into_response()
        }
    }
}

/// Convert a common `ImageProcessingSpec` back to the HTTP `ThumbnailSpecification`
/// for inclusion in failure responses.
fn processing_spec_to_http_spec(
    spec: &thumbnail_common::ImageProcessingSpec,
) -> ThumbnailSpecification {
    ThumbnailSpecification {
        image_to_image: Some(ImageToImageSpecification {
            encoding_format: spec
                .encoding_format
                .map(ImageEncodingFormat::from)
                .unwrap_or_default(),
            width: spec.width.unwrap_or(0),
            height: spec.height.unwrap_or(0),
            transform_type: spec
                .transform_type
                .map(TransformType::from)
                .unwrap_or_default(),
            crop_strategy: spec
                .crop_strategy
                .map(CropStrategy::from)
                .unwrap_or_default(),
            quality: spec.quality,
            preserve_aspect_ratio: spec.preserve_aspect_ratio,
        }),
    }
}

pub fn app(state: AppState) -> Router {
    let enable_cors = state.runtime.service.enable_cors;
    let state_cors_origins: Vec<String> = state
        .runtime
        .service
        .cors_allowed_origins
        .split(',')
        .map(str::trim)
        .filter(|origin| !origin.is_empty())
        .map(str::to_string)
        .collect();
    let meter = opentelemetry::global::meter("thumbnail-service");
    // Keep route registration centralized here so the binary only manages process lifecycle.
    let router = Router::new()
        .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", ApiDoc::openapi()))
        .route("/health/live", get(live_health_check))
        .route("/health", get(health_check))
        .route(
            concat!("/", rest_api_version!(), "/thumbnails"),
            get(get_thumbnails).post(create_thumbnails_handler),
        )
        .route(
            concat!("/", rest_api_version!(), "/associations"),
            post(associate_thumbnail).delete(disassociate_thumbnail),
        )
        .with_state(state)
        // HTTP trace layer: creates a span per request and extracts W3C traceparent
        // from incoming headers so distributed traces propagate through the REST API.
        .layer(TowerTraceLayer::builder().build())
        // HTTP metric layer: records request count, duration histogram, and error count
        // per matched route, exported via the global OTel meter provider.
        .layer(TowerMetricLayer::builder(meter).build())
        // Backstop for a handler that outlives its own outbound deadlines. The
        // Stat and publish calls are individually bounded, but a request that
        // makes several of them, or a future call site that forgets its deadline,
        // would still be able to hold a server task open indefinitely. Sized
        // above the per-call bound so an outbound deadline reports its own cause
        // first, and this only fires when nothing else did.
        .layer(TimeoutLayer::new(REQUEST_TIMEOUT_BACKSTOP));

    if enable_cors {
        router.layer(cors_layer(&state_cors_origins))
    } else {
        router
    }
}

/// Build the CORS layer from configured origins.
///
/// Previously `allow_origin(Any)` with `allow_headers(Any)`, so turning the
/// feature on meant "permit everything" — and it permitted `PUT`, which no route
/// serves. CORS is off by default so this was a latent hazard rather than a live
/// exposure, but it is the shape an operator inherits by enabling it.
///
/// No configured origins permits none. That makes an unconfigured `ENABLE_CORS`
/// visibly useless rather than quietly permissive, which is the safer way to be
/// wrong: a caller sees its request rejected and asks why, instead of nobody
/// noticing that any origin can read the API.
fn cors_layer(origins: &[String]) -> CorsLayer {
    let parsed: Vec<HeaderValue> = origins
        .iter()
        .filter_map(|origin| match origin.parse::<HeaderValue>() {
            Ok(value) => Some(value),
            Err(_) => {
                warn!(origin = %origin, "Ignoring CORS origin that is not a valid header value");
                None
            }
        })
        .collect();

    if parsed.is_empty() {
        warn!(
            "CORS is enabled with no usable allowed origins; every cross-origin request will be \
             refused. Set CORS_ALLOWED_ORIGINS."
        );
    } else {
        info!(
            origins = %origins.join(","),
            "CORS enabled for the configured origins"
        );
    }

    CorsLayer::new()
        .allow_origin(parsed)
        // Only what the router serves: GET, POST and DELETE, plus the OPTIONS
        // preflight. PUT was permitted and answered by nothing.
        .allow_methods([Method::GET, Method::POST, Method::DELETE, Method::OPTIONS])
        // Named rather than Any: these are what a caller of this API sends.
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE, header::ACCEPT])
}
