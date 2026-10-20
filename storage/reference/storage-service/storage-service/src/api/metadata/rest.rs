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

pub mod v1alpha {
    use std::collections::HashMap;

    use axum::{
        extract::{Path, State},
        http::{header::ETAG, HeaderMap, StatusCode},
        response::Response,
        routing::{post, put},
        Json, Router,
    };
    use futures::TryStreamExt;
    use serde::{Deserialize, Serialize};
    use storage_core::{
        ingress::{OpenAPI, ToRestService},
        traits::{Pagination, Request, StreamingOptions},
    };
    use storage_service_protos::{google::protobuf::Value, nvidia::omniverse::storage::metadata::v1alpha::*};
    use tonic::metadata::MetadataMap;
    use tracing::error;
    use utoipa::{OpenApi, ToSchema};

    use crate::{
        api::{
            metadata::{
                adapters::{invoke_delete_metadata, invoke_get_metadata, invoke_update_metadata},
                v1alpha::Metadata,
            },
            rest_utils::ResponseBuilder,
        },
        backend::metadata::Metadata as MetadataTrait,
        common::request::RequestExt,
    };

    pub type GetMetadataRequestBody = Vec<String>;

    pub type GetMetadataResponseBody = HashMap<String, UserMetadataValue>;

    pub type UpdateMetadataRequestBody = Value;

    #[derive(Serialize, Deserialize, ToSchema)]
    pub struct QueryMetadataRequestBody {
        #[serde(flatten)]
        pub request: QueryMetadataRequest,
        #[serde(default)]
        pub max_page_size: Option<u32>,
        #[serde(default)]
        pub continuation_handle: Option<String>,
    }

    #[derive(Serialize, Deserialize, ToSchema, Default)]
    pub struct QueryMetadataResponseBody {
        #[serde(flatten)]
        pub response: QueryMetadataResponse,
        #[serde(default)]
        pub next_continuation_handle: Option<String>,
    }

    impl<T: MetadataTrait> ToRestService for Metadata<T> {
        fn service(&self) -> Router {
            Router::new()
                .route(
                    "/v1alpha/metadata",
                    post(update_metadata_batch).delete(delete_metadata_batch),
                )
                .route("/v1alpha/metadata/get", post(get_metadata_batch))
                .route("/v1alpha/metadata/query", post(query_metadata))
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
        path = "/v1alpha/metadata/{uri}",
        request_body = GetMetadataRequestBody,
        params(
            ("uri" = String, Path, description = "A resource URI."),
        ),
        responses(
            (status = 200, description = "Successful response.", body = GetMetadataResponseBody),
        ),
        tag = "v1alpha",
    )]
    async fn get_metadata<T: MetadataTrait>(
        headers: HeaderMap,
        State(state): State<Metadata<T>>,
        Path(uri): Path<String>,
        Json(user_metadata_keys): Json<GetMetadataRequestBody>,
    ) -> Result<Response, StatusCode> {
        invoke_get_metadata(
            &*state.inner,
            Request::from_parts(
                MetadataMap::from_headers(headers),
                Default::default(),
                GetMetadataRequest {
                    uri,
                    user_metadata_keys,
                },
            ),
        )
        .await
        .map_err(|error| error.http_status())
        .inspect_err(|status| error!(?status, "get metadata request failed"))
        .map(|response| {
            ResponseBuilder::new()
                .status(StatusCode::OK)
                .json(&response.user_metadata)
        })?
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
    ) -> Result<Response, StatusCode> {
        let expected_etag = get_if_match_header_value(&headers)?;

        invoke_update_metadata(
            &*state.inner,
            Request::from_parts(
                MetadataMap::from_headers(headers),
                Default::default(),
                UpdateMetadataRequest {
                    expected_etag,
                    uri,
                    user_metadata_key,
                    user_metadata: Some(value),
                },
            ),
        )
        .await
        .map_err(|error| error.http_status())
        .inspect_err(|status| error!(?status, "update metadata request failed"))
        .map(|response| {
            ResponseBuilder::new()
                .header(ETAG, response.etag)
                .status(StatusCode::CREATED)
                .empty()
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
    ) -> Result<Response, StatusCode> {
        let expected_etag = get_if_match_header_value(&headers)?;

        invoke_delete_metadata(
            &*state.inner,
            Request::from_parts(
                MetadataMap::from_headers(headers),
                Default::default(),
                DeleteMetadataRequest {
                    expected_etag,
                    uri,
                    user_metadata_key,
                },
            ),
        )
        .await
        .map_err(|error| error.http_status())
        .inspect_err(|status| error!(?status, "delete metadata request failed"))
        .map(|_| ResponseBuilder::new().status(StatusCode::NO_CONTENT).empty())?
    }

    #[utoipa::path(
        post,
        path = "/v1alpha/metadata/get",
        responses(
            (status = 200, description = "Successful response.", body = GetMetadataBatchResponse),
        ),
        tag = "v1alpha",
    )]
    async fn get_metadata_batch<T: MetadataTrait>(
        headers: HeaderMap,
        State(state): State<Metadata<T>>,
        Json(body): Json<GetMetadataBatchRequest>,
    ) -> Result<Response, StatusCode> {
        let response = state
            .inner
            .get_metadata_batch(Request::from_parts(
                MetadataMap::from_headers(headers),
                Default::default(),
                body,
            ))
            .await
            .map_err(|error| error.http_status())
            .inspect_err(|status| error!(?status, "get metadata batch failed"))?;

        ResponseBuilder::new().status(StatusCode::OK).json(&response)
    }

    #[utoipa::path(
        post,
        path = "/v1alpha/metadata/query",
        responses(
            (status = 200, description = "Successful response.", body = QueryMetadataResponseBody),
        ),
        tag = "v1alpha",
    )]
    async fn query_metadata<T: MetadataTrait>(
        headers: HeaderMap,
        State(state): State<Metadata<T>>,
        Json(body): Json<QueryMetadataRequestBody>,
    ) -> Result<Response, StatusCode> {
        let mut responses = state
            .inner
            .query_metadata(
                Request::from_parts(MetadataMap::from_headers(headers), Default::default(), body.request),
                StreamingOptions::Paginated(Pagination {
                    continuation_handle: body.continuation_handle,
                    max_page_size: body.max_page_size,
                }),
            )
            .await
            .map_err(|error| error.http_status())
            .inspect_err(|status| error!(?status, "query metadata failed"))?;

        let response = responses
            .try_next()
            .await
            .map_err(|error| error.http_status())
            .inspect_err(|status| error!(?status, "query metadata failed"))?
            .map(|page| QueryMetadataResponseBody {
                next_continuation_handle: page.next_continuation_handle,
                response: page.response,
            })
            .unwrap_or_default();

        ResponseBuilder::new().status(StatusCode::OK).json(&response)
    }

    #[utoipa::path(
        post,
        path = "/v1alpha/metadata",
        request_body = UpdateMetadataBatchRequest,
        responses(
            (status = 200, description = "Successful response.", body = UpdateMetadataBatchResponse),
        ),
        tag = "v1alpha",
    )]
    async fn update_metadata_batch<T: MetadataTrait>(
        headers: HeaderMap,
        State(state): State<Metadata<T>>,
        Json(body): Json<UpdateMetadataBatchRequest>,
    ) -> Result<Response, StatusCode> {
        let request = Request::from_parts(MetadataMap::from_headers(headers), Default::default(), body);

        let response = state
            .inner
            .update_metadata_batch(request)
            .await
            .map_err(|error| error.http_status())
            .inspect_err(|status| error!(?status, "update metadata batch request failed"))?;

        ResponseBuilder::new().status(StatusCode::OK).json(&response)
    }

    #[utoipa::path(
        delete,
        path = "/v1alpha/metadata",
        responses(
            (status = 200, description = "Successful response.", body = DeleteMetadataBatchResponse),
        ),
        tag = "v1alpha",
    )]
    async fn delete_metadata_batch<T: MetadataTrait>(
        headers: HeaderMap,
        State(state): State<Metadata<T>>,
        Json(body): Json<DeleteMetadataBatchRequest>,
    ) -> Result<Response, StatusCode> {
        let response = state
            .inner
            .delete_metadata_batch(Request::from_parts(
                MetadataMap::from_headers(headers),
                Default::default(),
                body,
            ))
            .await
            .map_err(|error| error.http_status())
            .inspect_err(|status| error!(?status, "delete metadata batch failed"))?;

        ResponseBuilder::new().status(StatusCode::OK).json(&response)
    }

    #[derive(OpenApi)]
    #[openapi(
        paths(get_metadata, update_metadata, update_metadata_batch, delete_metadata),
        tags((name = "v1alpha", description = "Metadata API")),
        components(schemas(
            GetMetadataRequestBody,
            GetMetadataResponseBody,
            GetMetadataBatchRequest,
            GetMetadataBatchResponse,
            QueryMetadataRequestBody,
            QueryMetadataResponseBody,
            UpdateMetadataBatchRequest,
            UpdateMetadataBatchResponse,
            DeleteMetadataBatchRequest,
            DeleteMetadataBatchResponse,
            BatchErrorEntry,
        )),
    )]
    pub struct ApiDoc;

    /// Return the value of the etag header, if present.
    #[track_caller]
    fn get_if_match_header_value(headers: &HeaderMap) -> Result<Option<String>, StatusCode> {
        use axum::http::header::IF_MATCH;
        use tracing::error;

        let Some(etag) = headers.get(IF_MATCH) else {
            return Ok(None);
        };

        let etag = etag
            .to_str()
            .inspect_err(|error| error!(?error, "failed to stringify the if-match header value"))
            .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?
            .to_string();

        Ok(Some(etag))
    }
}
