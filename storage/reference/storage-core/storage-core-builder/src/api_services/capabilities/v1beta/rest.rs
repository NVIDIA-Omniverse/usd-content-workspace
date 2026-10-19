// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::atomic::Ordering;

use axum::{extract::State, response::Response as AxumResponse, routing::*};
use http::{HeaderMap, StatusCode};
use tracing::instrument;
use utoipa::OpenApi;

use crate::{
    api_services::error::{log_http_error, ProblemDetails},
    api_services::rest_utils::json_response,
    ingress::rest::{OpenAPI, ToRestService},
    traits::{
        capabilities::v1beta::{
            Capabilities as CapabilitiesTrait, ListServicesResponse, ListTopLevelAddressesRequest,
            ListTopLevelAddressesResponse,
        },
        Extensions, MetadataMap,
    },
};

use super::{list_service_response, Capabilities};
use crate::api_services::REQUEST_ID;

// v1alpha REST service implementation
impl<L> ToRestService for Capabilities<L>
where
    L: CapabilitiesTrait,
{
    fn service(&self) -> axum::Router {
        axum::Router::new()
            .route("/v1beta/capabilities/services", get(list_services))
            .route(
                "/v1beta/capabilities/top-level-addresses",
                get(list_top_level_addresses),
            )
            .with_state(self.clone())
    }

    fn openapi(&self) -> Option<crate::ingress::rest::OpenAPI> {
        Some(OpenAPI::new(
            "capabilities/v1beta",
            "/v1beta/capabilities/openapi.json",
            false,
            ApiDoc::openapi(),
        ))
    }
}

#[utoipa::path(
    get,
    path = "/v1beta/capabilities/services",
    responses(
        (status = 200, description = "List available services", body = ListServicesResponse)
    ),
    tag = "capabilities"
)]
#[instrument(name = "rest::list_services", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
async fn list_services() -> Result<AxumResponse, ProblemDetails> {
    json_response(StatusCode::OK, &list_service_response())
}

#[utoipa::path(
    get,
    path = "/v1beta/capabilities/top-level-addresses",
    responses(
        (status = 200, description = "List top level addresses", body = ListTopLevelAddressesResponse)
    ),
    tag = "capabilities"
)]
#[instrument(name = "rest::list_top_level_addresses", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
async fn list_top_level_addresses<S: CapabilitiesTrait>(
    headers: HeaderMap,
    State(state): State<Capabilities<S>>,
) -> Result<AxumResponse, ProblemDetails> {
    let request = ListTopLevelAddressesRequest {};
    let result = state
        .capabilities
        .list_top_level_addresses(
            tonic::Request::from_parts(MetadataMap::from_headers(headers), Extensions::new(), request).into(),
        )
        .await
        .map_err(|e| log_http_error(e.as_ref(), "list_top_level_addresses"))?;

    json_response(StatusCode::OK, &result)
}

#[derive(OpenApi)]
#[openapi(
    paths(
        list_services,
        list_top_level_addresses,
    ),
    components(
        schemas(
            ListServicesResponse, ListTopLevelAddressesResponse,
        )
    ),
    tags(
        (name = "capabilities", description = "Storage Service Capabilities API")
    )
)]
pub struct ApiDoc;
