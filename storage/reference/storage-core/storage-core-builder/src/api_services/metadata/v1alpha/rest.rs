// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use crate::traits::metadata::v1alpha::Value;
use axum::{
    body::Body,
    extract::{Path, State},
    http::{header::ETAG, HeaderMap, StatusCode},
    response::Response,
    routing::{post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use storage_core_util::error_limited;
use utoipa::{OpenApi, ToSchema};

use super::Metadata;
use crate::{
    api_services::error::{log_http_error, ProblemDetails},
    api_services::rest_utils::{empty_response, json_response},
    ingress::rest::{OpenAPI, ToRestService},
    traits::{
        metadata::v1alpha::{
            update_metadata_batch_entry_error, update_metadata_batch_entry_result, DeleteMetadataRequest,
            GetMetadataRequest, Metadata as MetadataTrait, UpdateMetadataBatchEntryError,
            UpdateMetadataBatchEntryResult, UpdateMetadataBatchRequest, UpdateMetadataRequest,
        },
        Extensions, MetadataMap,
    },
};

impl<T: MetadataTrait> ToRestService for Metadata<T> {
    fn service(&self) -> Router {
        Router::new()
            .route("/v1alpha/metadata", post(update_metadata_batch))
            .route("/v1alpha/metadata/{uri}", post(get_metadata))
            .route(
                "/v1alpha/metadata/{uri}/{key}",
                put(update_metadata).delete(delete_metadata),
            )
            .with_state(self.clone())
    }

    fn openapi(&self) -> Option<OpenAPI> {
        Some(OpenAPI::new(
            "metadata/v1alpha",
            "/v1alpha/metadata/openapi.json",
            true,
            ApiDoc::openapi(),
        ))
    }
}

#[utoipa::path(
    post,
    path = "/v1alpha/metadata",
    request_body = UpdateMetadataBatchRequestBody,
    responses(
        (status = 200, description = "Successful response.", body = UpdateMetadataBatchResponseBody),
        (status = 400, description = "Request exceeds the maximum batch size."),
    ),
    tag = "v1alpha",
)]
async fn update_metadata_batch<T: MetadataTrait>(
    headers: HeaderMap,
    State(state): State<Metadata<T>>,
    Json(payload): Json<UpdateMetadataBatchRequestBody>,
) -> Result<Response, ProblemDetails> {
    // Batch-size validation lives on the trait (`validate_metadata_batch_size`)
    // so downstream layers and backends see a consistent oversized-batch error
    // shape. The layer chain enforces it before any per-entry work.

    // Partition entries: keep parseable entries for fan-out, synthesize per-entry
    // InvalidArgument results for entries whose `value` failed to decode. The proto
    // contract says each entry is processed independently, so a single malformed
    // entry must not fail the whole batch.
    let mut requests = Vec::with_capacity(payload.requests.len());
    let mut requests_index_map = Vec::with_capacity(payload.requests.len());
    let mut decode_errors: Vec<UpdateMetadataBatchEntryResult> = Vec::new();
    for (request_index, entry) in payload.requests.into_iter().enumerate() {
        match serde_json::from_value(entry.value) {
            Ok(user_metadata) => {
                requests.push(UpdateMetadataRequest {
                    uri: entry.uri,
                    user_metadata_key: entry.key,
                    user_metadata: Some(user_metadata),
                    expected_etag: entry.expected_etag,
                });
                requests_index_map.push(request_index as u32);
            },
            Err(error) => {
                error_limited!(?error, request_index, "failed to decode batch metadata value");
                decode_errors.push(UpdateMetadataBatchEntryResult {
                    request_index: request_index as u32,
                    outcome: Some(update_metadata_batch_entry_result::Outcome::Error(
                        UpdateMetadataBatchEntryError {
                            code: update_metadata_batch_entry_error::Code::InvalidArgument as i32,
                            message: format!("failed to decode entry value: {error}"),
                        },
                    )),
                });
            },
        }
    }

    let request = UpdateMetadataBatchRequest { requests };
    let response = state
        .metadata
        .update_metadata_batch(
            tonic::Request::from_parts(MetadataMap::from_headers(headers), Extensions::new(), request).into(),
        )
        .await
        .map_err(|e| log_http_error(e.as_ref(), "update_metadata_batch"))?;

    let mut responses = Vec::with_capacity(requests_index_map.len() + decode_errors.len());
    responses.extend(decode_errors.into_iter().map(UpdateMetadataBatchResponseEntry::from));
    for mut entry in response.results {
        // The backend sees only parseable entries; remap its request_index values
        // back to the original REST payload positions.
        if let Some(&original) = requests_index_map.get(entry.request_index as usize) {
            entry.request_index = original;
        }
        responses.push(UpdateMetadataBatchResponseEntry::from(entry));
    }

    json_response(StatusCode::OK, &UpdateMetadataBatchResponseBody { responses })
}

#[utoipa::path(
    post,
    path = "/v1alpha/metadata/{uri}",
    request_body = GetMetadataRequestBody,
    params(
        ("uri" = String, Path, description = "A resource URI."),
    ),
    responses(
        (status = 200, description = "Successful response."),
    ),
    tag = "v1alpha",
)]
async fn get_metadata<T: MetadataTrait>(
    headers: HeaderMap,
    State(state): State<Metadata<T>>,
    Path(uri): Path<String>,
    Json(user_metadata_keys): Json<GetMetadataRequestBody>,
) -> Result<Response, ProblemDetails> {
    let request = GetMetadataRequest {
        uri,
        user_metadata_keys,
    };
    let result = state
        .metadata
        .get_metadata(tonic::Request::from_parts(MetadataMap::from_headers(headers), Extensions::new(), request).into())
        .await
        .map_err(|e| log_http_error(e.as_ref(), "get_metadata"))?;
    json_response(StatusCode::OK, &result)
}

#[utoipa::path(
    put,
    path = "/v1alpha/metadata/{uri}/{user_metadata_key}",
    params(
        ("uri" = String, Path, description = "A resource URI."),
        ("user_metadata_key" = String, Path, description = "A metadata entry key."),
    ),
    responses(
        (status = 201, description = "Successful response."),
    ),
    tag = "v1alpha",
)]
async fn update_metadata<T: MetadataTrait>(
    headers: HeaderMap,
    State(state): State<Metadata<T>>,
    Path((uri, user_metadata_key)): Path<(String, String)>,
    Json(value): Json<UpdateMetadataRequestBody>,
) -> Result<Response, ProblemDetails> {
    let expected_etag = get_if_match_header_value(&headers)?;

    let request = UpdateMetadataRequest {
        expected_etag,
        uri,
        user_metadata_key,
        user_metadata: Some(value),
    };
    state
        .metadata
        .update_metadata(
            tonic::Request::from_parts(MetadataMap::from_headers(headers), Extensions::new(), request).into(),
        )
        .await
        .map_err(|e| log_http_error(e.as_ref(), "update_metadata"))
        .map(|response| {
            Response::builder()
                .header(ETAG, response.etag)
                .status(StatusCode::CREATED)
                .body(Body::empty())
                .map_err(|e| {
                    error_limited!(?e, "Failed to build response");
                    ProblemDetails::from(StatusCode::INTERNAL_SERVER_ERROR)
                })
        })?
}

#[utoipa::path(
    delete,
    path = "/v1alpha/metadata/{uri}/{user_metadata_key}",
    params(
        ("uri" = String, Path, description = "A resource URI."),
        ("user_metadata_key" = String, Path, description = "A metadata entry key."),
    ),
    responses(
        (status = 204, description = "Successful response."),
    ),
    tag = "v1alpha",
)]
async fn delete_metadata<T: MetadataTrait>(
    headers: HeaderMap,
    State(state): State<Metadata<T>>,
    Path((uri, user_metadata_key)): Path<(String, String)>,
) -> Result<Response, ProblemDetails> {
    let expected_etag = get_if_match_header_value(&headers)?;

    let request = DeleteMetadataRequest {
        expected_etag,
        uri,
        user_metadata_key,
    };
    state
        .metadata
        .delete_metadata(
            tonic::Request::from_parts(MetadataMap::from_headers(headers), Extensions::new(), request).into(),
        )
        .await
        .map_err(|e| log_http_error(e.as_ref(), "delete_metadata"))?;
    empty_response(StatusCode::NO_CONTENT)
}

#[derive(OpenApi)]
#[openapi(
    paths(update_metadata_batch, get_metadata, update_metadata, delete_metadata),
    tags((name = "v1alpha", description = "Metadata API")),
    components(schemas(
        GetMetadataRequestBody,
        UpdateMetadataBatchRequestBody,
        UpdateMetadataBatchEntry,
        UpdateMetadataBatchResponseBody,
        UpdateMetadataBatchResponseEntry,
        UpdateMetadataBatchEntrySuccess,
        UpdateMetadataBatchEntryErrorBody,
        UpdateMetadataBatchEntryErrorCode,
    )),
)]
pub struct ApiDoc;

/// Return the value of the etag header, if present.
#[track_caller]
fn get_if_match_header_value(headers: &HeaderMap) -> Result<Option<String>, ProblemDetails> {
    use axum::http::header::IF_MATCH;

    let Some(etag) = headers.get(IF_MATCH) else {
        return Ok(None);
    };

    let etag = etag
        .to_str()
        .inspect_err(|error| error_limited!(?error, "failed to stringify the if-match header value"))
        .map_err(|_| ProblemDetails::from(StatusCode::UNPROCESSABLE_ENTITY))?
        .to_string();

    Ok(Some(etag))
}

type GetMetadataRequestBody = Vec<String>;
type UpdateMetadataRequestBody = Value;

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct UpdateMetadataBatchRequestBody {
    pub requests: Vec<UpdateMetadataBatchEntry>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct UpdateMetadataBatchEntry {
    pub uri: String,
    pub key: String,
    pub value: serde_json::Value,
    pub expected_etag: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct UpdateMetadataBatchResponseBody {
    pub responses: Vec<UpdateMetadataBatchResponseEntry>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct UpdateMetadataBatchResponseEntry {
    pub request_index: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success: Option<UpdateMetadataBatchEntrySuccess>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<UpdateMetadataBatchEntryErrorBody>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct UpdateMetadataBatchEntrySuccess {
    pub etag: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct UpdateMetadataBatchEntryErrorBody {
    pub code: UpdateMetadataBatchEntryErrorCode,
    pub message: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum UpdateMetadataBatchEntryErrorCode {
    InvalidArgument,
    NotFound,
    PermissionDenied,
    FailedPrecondition,
    Internal,
    Unavailable,
}

impl From<UpdateMetadataBatchEntryResult> for UpdateMetadataBatchResponseEntry {
    fn from(result: UpdateMetadataBatchEntryResult) -> Self {
        match result.outcome {
            Some(update_metadata_batch_entry_result::Outcome::Success(success)) => Self {
                request_index: result.request_index,
                success: Some(UpdateMetadataBatchEntrySuccess { etag: success.etag }),
                error: None,
            },
            Some(update_metadata_batch_entry_result::Outcome::Error(error)) => Self {
                request_index: result.request_index,
                success: None,
                error: Some(UpdateMetadataBatchEntryErrorBody {
                    code: batch_error_code(error.code),
                    message: error.message,
                }),
            },
            None => Self {
                request_index: result.request_index,
                success: None,
                error: Some(UpdateMetadataBatchEntryErrorBody {
                    code: UpdateMetadataBatchEntryErrorCode::Internal,
                    message: "missing batch entry outcome".to_string(),
                }),
            },
        }
    }
}

fn batch_error_code(code: i32) -> UpdateMetadataBatchEntryErrorCode {
    use update_metadata_batch_entry_error::Code;

    match Code::try_from(code).unwrap_or(Code::Internal) {
        Code::InvalidArgument => UpdateMetadataBatchEntryErrorCode::InvalidArgument,
        Code::NotFound => UpdateMetadataBatchEntryErrorCode::NotFound,
        Code::PermissionDenied => UpdateMetadataBatchEntryErrorCode::PermissionDenied,
        Code::FailedPrecondition => UpdateMetadataBatchEntryErrorCode::FailedPrecondition,
        Code::Unavailable => UpdateMetadataBatchEntryErrorCode::Unavailable,
        Code::Unspecified | Code::Internal => UpdateMetadataBatchEntryErrorCode::Internal,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use super::*;
    use crate::traits::error::Result as StorageResult;
    use crate::traits::metadata::v1alpha::{DeleteMetadataResponse, GetMetadataResponse};

    #[derive(Clone, Default)]
    struct BatchAwareMetadata {
        update_calls: Arc<AtomicUsize>,
        batch_calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl MetadataTrait for BatchAwareMetadata {
        fn inner(&self) -> Option<&dyn MetadataTrait> {
            None
        }

        async fn get_metadata(
            &self,
            _request: crate::traits::Request<GetMetadataRequest>,
        ) -> StorageResult<GetMetadataResponse> {
            Ok(GetMetadataResponse::default())
        }

        async fn update_metadata(
            &self,
            _request: crate::traits::Request<UpdateMetadataRequest>,
        ) -> StorageResult<crate::traits::metadata::v1alpha::UpdateMetadataResponse> {
            self.update_calls.fetch_add(1, Ordering::SeqCst);
            Ok(crate::traits::metadata::v1alpha::UpdateMetadataResponse { etag: "single".into() })
        }

        async fn update_metadata_batch(
            &self,
            request: crate::traits::Request<UpdateMetadataBatchRequest>,
        ) -> StorageResult<crate::traits::metadata::v1alpha::UpdateMetadataBatchResponse> {
            self.batch_calls.fetch_add(1, Ordering::SeqCst);
            Ok(crate::traits::metadata::v1alpha::UpdateMetadataBatchResponse {
                results: request
                    .get_ref()
                    .requests
                    .iter()
                    .enumerate()
                    .map(|(request_index, _)| UpdateMetadataBatchEntryResult {
                        request_index: request_index as u32,
                        outcome: Some(update_metadata_batch_entry_result::Outcome::Success(
                            crate::traits::metadata::v1alpha::UpdateMetadataResponse { etag: "batch".into() },
                        )),
                    })
                    .collect(),
            })
        }

        async fn delete_metadata(
            &self,
            _request: crate::traits::Request<DeleteMetadataRequest>,
        ) -> StorageResult<DeleteMetadataResponse> {
            Ok(DeleteMetadataResponse {})
        }
    }

    #[tokio::test]
    async fn rest_update_metadata_batch_forwards_to_batch_trait_once() {
        let backend = BatchAwareMetadata::default();
        let update_calls = backend.update_calls.clone();
        let batch_calls = backend.batch_calls.clone();
        let state = Metadata::new(backend);

        let response = update_metadata_batch(
            HeaderMap::new(),
            State(state),
            Json(UpdateMetadataBatchRequestBody {
                requests: vec![UpdateMetadataBatchEntry {
                    uri: "s3://bucket/object".into(),
                    key: "key".into(),
                    value: serde_json::json!("value"),
                    expected_etag: None,
                }],
            }),
        )
        .await
        .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(batch_calls.load(Ordering::SeqCst), 1);
        assert_eq!(update_calls.load(Ordering::SeqCst), 0);
    }
}
