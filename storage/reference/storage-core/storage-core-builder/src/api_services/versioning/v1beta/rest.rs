// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::atomic::Ordering;

use axum::{
    extract::{Path, Query, State},
    response::Response as AxumResponse,
    routing::*,
};
use futures::StreamExt;
use http::{HeaderMap, StatusCode};
use serde::{Deserialize, Serialize};
use storage_core_util::{error_limited, log_if_slow};
use tracing::instrument;

use crate::api_services::error::{log_http_error, ProblemDetails};
use utoipa::{IntoParams, OpenApi, ToSchema};

use crate::{
    api_services::rest_utils::json_response,
    ingress::rest::ToRestService,
    traits::{
        versioning::v1beta::{
            EnumerateVersionsRequest, EnumerateVersionsResponseWithContinueToken, VersionInfo,
            Versioning as VersioningTrait,
        },
        Extensions, MetadataMap, Pagination, StreamingOptions,
    },
};

use super::super::super::REQUEST_ID;
use super::Versioning;

impl<L> ToRestService for Versioning<L>
where
    L: VersioningTrait,
{
    fn service(&self) -> axum::Router {
        axum::Router::new()
            .route(
                "/v1beta/versioning/{resource_address}/versions",
                get(enumerate_versions),
            )
            .with_state(self.clone())
    }

    fn openapi(&self) -> Option<crate::ingress::rest::OpenAPI> {
        Some(crate::ingress::rest::OpenAPI::new(
            "versioning/v1beta",
            "/v1beta/versioning/openapi.json",
            false,
            ApiDoc::openapi(),
        ))
    }
}

#[utoipa::path(
    get,
    path = "/v1beta/versioning/{folder_address}/versions",
    params(
        ("folder_address" = String, Path, description = "The address of the folder to enumerate versions for"),
        ("query" = EnumerateVersionsQueryParams, Query, description = "Query parameters for the enumerate versions request"),
    ),
    responses(
        (status = 200, description = "Enumerate versions request successful", body = EnumerateVersionsResponse)
    ),
    tag = "v1beta"
)]
#[instrument(name = "rest::enumerate_versions", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
async fn enumerate_versions<S: VersioningTrait>(
    headers: HeaderMap,
    State(state): State<Versioning<S>>,
    Path(folder_address): Path<String>,
    Query(query): Query<EnumerateVersionsQueryParams>,
) -> Result<AxumResponse, ProblemDetails> {
    let _slow = log_if_slow!(guard: "rest::enumerate_versions").with_context(folder_address.as_str());
    let request = EnumerateVersionsRequest {
        resource_address: folder_address.clone(),
    };
    let (continuation_handle, version_id_token) =
        EnumerateVersionsResponse::split_continuation_tokens(query.continuation_handle);
    let mut stream = state
        .versioning
        .enumerate_versions(
            tonic::Request::from_parts(MetadataMap::from_headers(headers), Extensions::new(), request).into(),
            StreamingOptions::Paginated(Pagination {
                continuation_handle,
                max_page_size: query.max_page_size,
            }),
            version_id_token,
        )
        .await
        .map_err(|e| log_http_error(e.as_ref(), &folder_address))?;

    let Some(response) = stream.next().await else {
        error_limited!(code = %StatusCode::INTERNAL_SERVER_ERROR, context = folder_address, "Failed to receive data from Enumerate stream");
        return Err(ProblemDetails::from(StatusCode::INTERNAL_SERVER_ERROR));
    };

    match response {
        Ok(response) => json_response(StatusCode::OK, &EnumerateVersionsResponse::from(response)),
        Err(e) => Err(log_http_error(e.as_ref(), &folder_address)),
    }
}

#[derive(Debug, Serialize, Deserialize, IntoParams, ToSchema)]
pub struct EnumerateVersionsQueryParams {
    continuation_handle: Option<String>,
    max_page_size: Option<u32>,
}

// The ListItems definition is different between Rest and GRPC, so we need to manually create the Rest version
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct EnumerateVersionsResponse {
    pub items: Vec<VersionInfo>,
    pub next_continuation_handle: Option<String>,
    pub versions_order: String,
}

impl EnumerateVersionsResponse {
    const COMBINED_TOKEN_SEPARATOR: &str = "-~SEPARATOR~-";

    pub fn combine_continuation_tokens(
        continue_token: Option<String>,
        version_id_token: Option<String>,
    ) -> Option<String> {
        if let Some(continue_token) = continue_token {
            if let Some(version_id_token) = version_id_token {
                Some(format!(
                    "{}{}{}",
                    continue_token,
                    Self::COMBINED_TOKEN_SEPARATOR,
                    version_id_token
                ))
            } else {
                Some(continue_token)
            }
        } else {
            None
        }
    }

    pub fn split_continuation_tokens(combined_token: Option<String>) -> (Option<String>, Option<String>) {
        if let Some(combined_token) = combined_token {
            if let Some(index) = combined_token.find(Self::COMBINED_TOKEN_SEPARATOR) {
                let continue_token = &combined_token[..index];
                let version_id_token = &combined_token[index + Self::COMBINED_TOKEN_SEPARATOR.len()..];
                (Some(continue_token.to_string()), Some(version_id_token.to_string()))
            } else {
                (Some(combined_token), None)
            }
        } else {
            (None, None)
        }
    }
}

impl From<EnumerateVersionsResponseWithContinueToken> for EnumerateVersionsResponse {
    fn from(response: EnumerateVersionsResponseWithContinueToken) -> Self {
        let continuation_handle =
            EnumerateVersionsResponse::combine_continuation_tokens(response.continue_token, response.version_id_token);
        Self {
            items: response.response.items,
            next_continuation_handle: continuation_handle,
            versions_order: match response.response.versions_order {
                1 => "newest_first".to_string(),
                2 => "oldest_first".to_string(),
                3 => "by_key".to_string(),
                _ => "unspecified".to_string(),
            },
        }
    }
}

#[derive(OpenApi)]
#[openapi(
    paths(enumerate_versions),
    components(schemas(EnumerateVersionsQueryParams, EnumerateVersionsResponse, VersionInfo)),
    tags(
        (name = "versioning", description = "Storage Service Versioning API")
    )
)]
pub struct ApiDoc;
