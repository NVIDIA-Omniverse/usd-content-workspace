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

use crate::http::middlewares::access_log::AccessLog;
use crate::http::rest_api_version;
use crate::http::ErrorString;
use crate::service::authorization::{
    access_detail_single, access_details_batch, access_params_batch, access_params_single,
    AuthorizationBatchRequest, AuthorizationBatchResponse, AuthorizationPayload,
    AuthorizationResponse,
};
use crate::state::AppState;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::{Extension, Json};
use garde::Validate;
use permission_core::Principal;
use std::sync::Arc;

const AUTHORIZATION_TAG: &str = "authorization";

#[utoipa::path(
    post,
    path = concat!("/", rest_api_version!(), "/authorization/"),
    operation_id = "authorization",
    description = "Determines if the specified principal has access to perform the action on the specified resource.",
    tag = AUTHORIZATION_TAG,
    request_body = AuthorizationPayload,
    responses(
        (
            status = OK,
            body = AuthorizationResponse,
            description = "Returned when the authorization information is returned successfully. \
                The status code does not specify if access is granted, the corresponding payload must be checked instead."
        ),
        (status = BAD_REQUEST, body = ErrorString, description = "The payload was parsed correctly but the operation failed due to invalid information being provided."),
        (status = UNAUTHORIZED, body = ErrorString, description = "Returned if the user lacks valid authentication credentials to call this API."),
        (status = FORBIDDEN, body = ErrorString, description = "Returned when the user does not have access to check authorization for the specified payload."),
        (status = TOO_MANY_REQUESTS, body = ErrorString, description = "Returned when the client sends too many requests and needs to slow down."),
        (status = INTERNAL_SERVER_ERROR, body = ErrorString, description = "Returned when an unexpected server error occurs."),
    )
)]
#[tracing::instrument(level = "info", skip(requester, access, state, payload))]
pub async fn authorize(
    Extension(requester): Extension<Arc<Principal>>,
    Extension(access): Extension<AccessLog>,
    State(state): State<AppState>,
    Json(payload): Json<AuthorizationPayload>,
) -> Result<(HeaderMap, Json<AuthorizationResponse>), (StatusCode, String)> {
    access.params(access_params_single(&payload));

    payload
        .validate()
        .map_err(|e| access.fail(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;

    match crate::service::authorization::authorize(payload, state.clone(), &requester).await {
        Ok(response) => {
            access.detail(access_detail_single(&response));

            let mut headers = HeaderMap::new();

            if state.config.cache.cache_control_max_age > 0 {
                let cache_control_value = format!(
                    "max-age={}, must-revalidate",
                    state.config.cache.cache_control_max_age
                );
                if let Ok(header_value) = HeaderValue::from_str(&cache_control_value) {
                    headers.insert("cache-control", header_value);
                }
            }

            Ok((headers, Json(response)))
        }
        Err(err) => Err(access.fail_err(&err)),
    }
}

#[utoipa::path(
    post,
    path = concat!("/", rest_api_version!(), "/authorization/batch/"),
    operation_id = "authorization_batch",
    description = "Batched variant of POST /authorization/ API that allows checking multiple principals, actions and resources in one single request.",
    tag = AUTHORIZATION_TAG,
    request_body = AuthorizationBatchRequest,
    responses(
        (
            status = OK,
            body = AuthorizationBatchResponse,
            description = "Returned when the authorization information is returned successfully. \
                The status code does not specify if access is granted, the corresponding payload must be checked instead."
        ),
        (status = BAD_REQUEST, body = ErrorString, description = "The payload was parsed correctly but the operation failed due to invalid information being provided."),
        (status = UNAUTHORIZED, body = ErrorString, description = "Returned if the user lacks valid authentication credentials to call this API."),
        (status = FORBIDDEN, body = ErrorString, description = "Returned when the user does not have access to check authorization for the specified payload."),
        (status = TOO_MANY_REQUESTS, body = ErrorString, description = "Returned when the client sends too many requests and needs to slow down."),
        (status = INTERNAL_SERVER_ERROR, body = ErrorString, description = "Returned when an unexpected server error occurs."),
    )
)]
#[tracing::instrument(level = "info", skip(requester, access, state, request))]
pub async fn authorize_batch(
    Extension(requester): Extension<Arc<Principal>>,
    Extension(access): Extension<AccessLog>,
    State(state): State<AppState>,
    Json(request): Json<AuthorizationBatchRequest>,
) -> Result<(HeaderMap, Json<AuthorizationBatchResponse>), (StatusCode, String)> {
    if access.is_enabled() {
        access.items(access_params_batch(request.condition, &request.batches));
    }

    request
        .validate()
        .map_err(|e| access.fail(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;

    let batches = access.is_enabled().then(|| request.batches.clone());
    match crate::service::authorization::authorize_batch(request, state.clone(), &requester).await {
        Ok(response) => {
            if let Some(batches) = &batches {
                access.zip_details(access_details_batch(batches, &response.decisions));
            }

            let mut headers = HeaderMap::new();

            if state.config.cache.cache_control_max_age > 0 {
                let cache_control_value = format!(
                    "max-age={}, must-revalidate",
                    state.config.cache.cache_control_max_age
                );
                if let Ok(header_value) = HeaderValue::from_str(&cache_control_value) {
                    headers.insert("cache-control", header_value);
                }
            }

            Ok((headers, Json(response)))
        }
        Err(err) => Err(access.fail_err(&err)),
    }
}
