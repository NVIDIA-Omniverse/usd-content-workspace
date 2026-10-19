// SPDX-FileCopyrightText: Copyright 2026 NVIDIA CORPORATION & AFFILIATES
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

use crate::http::middlewares::access_log::access_log_middleware;
use crate::http::middlewares::auth::auth_middleware;
use crate::http::middlewares::metrics::metrics_middleware;
use crate::state::AppState;
use axum::extract::{MatchedPath, Request};
use axum::http::StatusCode;
use axum::middleware::from_fn_with_state;
use axum::{http, Router};
use axum_observability::server::metric::TowerMetricLayer;
use opentelemetry_http::HeaderExtractor;
use serde::Serialize;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::info_span;
use tracing_opentelemetry::OpenTelemetrySpanExt;
use utoipa::openapi::extensions::{Extensions, ExtensionsBuilder};
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use utoipa_swagger_ui::SwaggerUi;

pub mod authorization;
pub mod diagnostics;
pub mod idp;
pub mod metadata;
pub mod middlewares;
pub mod policies;

/// Single source of truth for the public REST API version, reused in the
/// `#[utoipa::path]` route declarations and advertised by the capabilities API.
macro_rules! rest_api_version {
    () => {
        "v1beta"
    };
}
pub(crate) use rest_api_version;

pub struct ErrorString;

impl utoipa::PartialSchema for ErrorString {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        use utoipa::openapi::schema::{Schema, SchemaType, Type};
        use utoipa::openapi::{ObjectBuilder, RefOr};

        RefOr::T(Schema::Object(
            ObjectBuilder::new()
                .schema_type(SchemaType::Type(Type::String))
                .max_length(Some(2048))
                .pattern(Some("^[\\x20-\\x7E\\r\\n]+$"))
                .build(),
        ))
    }
}

impl utoipa::ToSchema for ErrorString {
    fn name() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("ErrorString")
    }
}

#[derive(Debug, Serialize)]
struct AuthModifier;

impl Modify for AuthModifier {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(schema) = openapi.components.as_mut() {
            schema.add_security_scheme(
                "token_jwt",
                SecurityScheme::Http(
                    HttpBuilder::new()
                        .scheme(HttpAuthScheme::Bearer)
                        .bearer_format("JWT")
                        .description(Some("Bearer token authentication using JWT. Implements security best practices from RFC8725."))
                        .build(),
                ),
            );
        }
    }
}

#[derive(OpenApi)]
#[openapi(
    info(title = "Permission Service"),
    modifiers(&AuthModifier),
    security(
        (),
        ("token_jwt" = [])
    )
)]
pub struct ApiDoc;

/// Records an nSpect program ID in the `info.x-nspect.id` extension of `openapi`.
///
/// NVIDIA's API governance pipeline reads the ID from there. The ID is not part of the
/// source: `openapi-generator` adds it only when `OPENAPI_NSPECT_ID` is set, which CI does
/// in the job that prepares the spec for governance and publication. The committed
/// `openapi.json` and the document the service serves carry no ID.
pub fn add_nspect_id(openapi: &mut utoipa::openapi::OpenApi, id: &str) {
    let extension = ExtensionsBuilder::new()
        .add("x-nspect", serde_json::json!({ "id": id }))
        .build();
    openapi
        .info
        .extensions
        .get_or_insert_with(Extensions::default)
        .merge(extension);
}

const HEALTH_TAG: &str = "health";

#[utoipa::path(
    get,
    path = "/health",
    operation_id = "health",
    description = "Health check to determine API availability.",
    tag = HEALTH_TAG,
    responses(
        (status = OK),
        (status = BAD_REQUEST, body = ErrorString, description = "The payload was parsed correctly but the operation failed due to invalid information being provided."),
        (status = UNAUTHORIZED, body = ErrorString, description = "Returned if the user lacks valid authentication credentials to call this API."),
        (status = TOO_MANY_REQUESTS, body = ErrorString, description = "Returned when the client sends too many requests and needs to slow down."),
        (status = INTERNAL_SERVER_ERROR, body = ErrorString, description = "Returned when an unexpected server error occurs."),
    )
)]
pub async fn health() -> StatusCode {
    StatusCode::OK
}

pub fn routes() -> (Router<AppState>, utoipa::openapi::OpenApi) {
    let router = OpenApiRouter::with_openapi(ApiDoc::openapi());
    let router = add_public_routes(router);
    let router = add_private_routes(router);
    router.split_for_parts()
}

pub fn add_public_routes(router: OpenApiRouter<AppState>) -> OpenApiRouter<AppState> {
    router
        .routes(routes!(authorization::authorize))
        .routes(routes!(authorization::authorize_batch))
        .routes(routes!(health))
}

pub fn add_private_routes(router: OpenApiRouter<AppState>) -> OpenApiRouter<AppState> {
    router
        .routes(routes!(policies::get_policies_paginated_api))
        .routes(routes!(policies::put_policy_api))
        .routes(routes!(policies::put_policy_batch_api))
        .routes(routes!(policies::get_policy_api))
        .routes(routes!(policies::delete_policy_api))
        .routes(routes!(metadata::get_service))
        .routes(routes!(metadata::get_service_metadata))
        .routes(routes!(metadata::put_service_metadata))
        .routes(routes!(metadata::delete_service_metadata))
        .routes(routes!(metadata::get_service_resource_types))
        .routes(routes!(metadata::replace_service_resource_types))
        .routes(routes!(metadata::get_resource_metadata))
        .routes(routes!(metadata::put_resource_metadata))
        .routes(routes!(metadata::delete_resource_metadata))
        .routes(routes!(metadata::get_service_actions))
        .routes(routes!(metadata::replace_service_actions))
        .routes(routes!(metadata::put_service_action))
        .routes(routes!(metadata::delete_service_action))
        .routes(routes!(diagnostics::diagnostics_authorize))
        .routes(routes!(diagnostics::diagnostics_authorize_batch))
        .routes(routes!(idp::get_jwks))
}

pub fn configure_router(state: AppState) -> Router<()> {
    let (router, api) = routes();

    let meter = state.metrics.meter.clone();

    let mut router = router
        .route_layer(from_fn_with_state(state.clone(), metrics_middleware))
        .route_layer(from_fn_with_state(state.clone(), auth_middleware))
        .layer(TowerMetricLayer::builder(meter).build())
        .layer(
            TraceLayer::new_for_http().make_span_with(|request: &Request<_>| {
                let matched_path = request
                    .extensions()
                    .get::<MatchedPath>()
                    .map(MatchedPath::as_str)
                    .unwrap_or("UNKNOWN");

                let headers = request.headers();
                let context = opentelemetry::global::get_text_map_propagator(|propagator| {
                    propagator.extract(&HeaderExtractor(headers))
                });

                let span = info_span!(
                    "http_request",
                    method = ?request.method(),
                    matched_path,
                    otel.name = format!("{} {}", request.method(), matched_path),
                    otel.kind = "server",
                );

                let _ = span.set_parent(context);
                span
            }),
        )
        .merge(SwaggerUi::new("/").url("/openapi.json", api))
        // Applied as the outermost global layer (after the Swagger merge) so it
        // runs after routing yet wraps every route: the auth layer, the fallback,
        // and the Swagger UI catch-all that would otherwise intercept unmatched
        // paths. This lets it observe rejected (`401`/`403`), unmatched (`404`),
        // and method-not-allowed (`405`) requests. The middleware skips the
        // discovery endpoints and successfully served Swagger assets itself.
        .layer(from_fn_with_state(state.clone(), access_log_middleware))
        .with_state(state);

    if cfg!(debug_assertions) {
        let cors = CorsLayer::new()
            .allow_headers([
                http::header::ACCEPT,
                http::header::AUTHORIZATION,
                http::header::CONTENT_TYPE,
            ])
            .allow_methods([
                http::Method::GET,
                http::Method::HEAD,
                http::Method::POST,
                http::Method::PUT,
                http::Method::PATCH,
                http::Method::OPTIONS,
                http::Method::DELETE,
            ])
            .allow_origin(tower_http::cors::Any);
        router = router.route_layer(cors);
    }
    router
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openapi_carries_no_nspect_id_or_tags_by_default() {
        let doc = serde_json::to_value(ApiDoc::openapi()).unwrap();
        assert!(doc["info"].get("x-nspect").is_none());
        assert!(doc.get("tags").is_none());
    }

    #[test]
    fn add_nspect_id_sets_info_extension() {
        let mut openapi = ApiDoc::openapi();
        add_nspect_id(&mut openapi, "example-program-id");
        let doc = serde_json::to_value(&openapi).unwrap();
        assert_eq!(
            doc["info"]["x-nspect"],
            serde_json::json!({ "id": "example-program-id" })
        );
        assert_eq!(doc["info"]["title"], "Permission Service");
        assert!(doc.get("tags").is_none());
    }
}
