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
use crate::http::ErrorString;
use crate::service::authorization::{
    access_params_batch, access_params_single, AuthorizationBatch, AuthorizationBatchRequest,
    AuthorizationPayload, DiagnosticsAuthorizeBatchResponse, DiagnosticsAuthorizeResponse,
    DiagnosticsBatchResult,
};
use crate::state::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::{Extension, Json};
use garde::Validate;
use permission_core::Principal;
use serde_json::{json, Value};
use std::sync::Arc;

const DIAGNOSTICS_TAG: &str = "diagnostics";

#[utoipa::path(
    post,
    path = "/v1beta/diagnostics/authorize/",
    operation_id = "diagnostics_authorize",
    description = "Returns the list of policies that would be evaluated for the given PARC parameters and the evaluation priority, without returning a decision.",
    tag = DIAGNOSTICS_TAG,
    request_body = AuthorizationPayload,
    responses(
        (
            status = OK,
            body = DiagnosticsAuthorizeResponse,
            description = "Returned when the diagnostics information is returned successfully."
        ),
        (status = BAD_REQUEST, body = ErrorString, description = "The payload was parsed correctly but the operation failed due to invalid information being provided."),
        (status = UNAUTHORIZED, body = ErrorString, description = "Returned if the user lacks valid authentication credentials to call this API."),
        (status = FORBIDDEN, body = ErrorString, description = "Returned when the user does not have the permissions:diagnostics permission."),
        (status = UNPROCESSABLE_ENTITY, body = ErrorString, description = "Returned when the request payload fails validation."),
        (status = INTERNAL_SERVER_ERROR, body = ErrorString, description = "Returned when an unexpected server error occurs."),
    )
)]
#[tracing::instrument(level = "info", skip(requester, access, state, payload))]
pub async fn diagnostics_authorize(
    Extension(requester): Extension<Arc<Principal>>,
    Extension(access): Extension<AccessLog>,
    State(state): State<AppState>,
    Json(payload): Json<AuthorizationPayload>,
) -> Result<Json<DiagnosticsAuthorizeResponse>, (StatusCode, String)> {
    access.params(access_params_single(&payload));

    payload
        .validate()
        .map_err(|e| access.fail(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;

    match crate::service::authorization::diagnostics_authorize(payload, state, &requester).await {
        Ok(response) => {
            access.detail(json!({
                "evaluation_priority": response.evaluation_priority,
                "policy_count": response.policies.len(),
            }));
            Ok(Json(response))
        }
        Err(err) => Err(access.fail_err(&err)),
    }
}

#[utoipa::path(
    post,
    path = "/v1beta/diagnostics/authorize/batch/",
    operation_id = "diagnostics_authorize_batch",
    description = "Batched variant of POST /diagnostics/authorize/ that returns, for each batch entry, the list of policies that would be evaluated and the evaluation priority, without returning a decision.",
    tag = DIAGNOSTICS_TAG,
    request_body = AuthorizationBatchRequest,
    responses(
        (
            status = OK,
            body = DiagnosticsAuthorizeBatchResponse,
            description = "Returned when the diagnostics information is returned successfully."
        ),
        (status = BAD_REQUEST, body = ErrorString, description = "The payload was parsed correctly but the operation failed due to invalid information being provided."),
        (status = UNAUTHORIZED, body = ErrorString, description = "Returned if the user lacks valid authentication credentials to call this API."),
        (status = FORBIDDEN, body = ErrorString, description = "Returned when the user does not have the permissions:diagnostics permission."),
        (status = UNPROCESSABLE_ENTITY, body = ErrorString, description = "Returned when the request payload fails validation."),
        (status = INTERNAL_SERVER_ERROR, body = ErrorString, description = "Returned when an unexpected server error occurs."),
    )
)]
#[tracing::instrument(level = "info", skip(requester, access, state, request))]
pub async fn diagnostics_authorize_batch(
    Extension(requester): Extension<Arc<Principal>>,
    Extension(access): Extension<AccessLog>,
    State(state): State<AppState>,
    Json(request): Json<AuthorizationBatchRequest>,
) -> Result<Json<DiagnosticsAuthorizeBatchResponse>, (StatusCode, String)> {
    if access.is_enabled() {
        access.items(access_params_batch(request.condition, &request.batches));
    }

    request
        .validate()
        .map_err(|e| access.fail(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;

    let batches = access.is_enabled().then(|| request.batches.clone());
    match crate::service::authorization::diagnostics_authorize_batch(request, state, &requester)
        .await
    {
        Ok(response) => {
            if let Some(batches) = &batches {
                access.zip_details(diagnostics_batch_details(batches, &response.results));
            }
            Ok(Json(response))
        }
        Err(err) => Err(access.fail_err(&err)),
    }
}

/// Per-line success `detail` for a diagnostics batch response, aligned with the
/// order produced by [`access_params_batch`]: `evaluation_priority` and the
/// number of policies that would be evaluated (never the policy text itself).
fn diagnostics_batch_details(
    batches: &[AuthorizationBatch],
    results: &[DiagnosticsBatchResult],
) -> Vec<Value> {
    let mut details = Vec::new();
    for (index, batch) in batches.iter().enumerate() {
        if batch.actions.is_empty() {
            continue;
        }
        let result = results.get(index);
        for action_index in 0..batch.actions.len() {
            let detail = match result {
                Some(result) => json!({
                    "evaluation_priority": result.evaluation_priority,
                    "policy_count": result
                        .actions
                        .get(action_index)
                        .map(|action| action.policies.len())
                        .unwrap_or(0),
                }),
                None => Value::Object(serde_json::Map::new()),
            };
            details.push(detail);
        }
    }
    details
}
