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

pub mod v1beta {
    use axum::{
        extract::State,
        http::{HeaderMap, StatusCode},
        response::Response,
        routing::post,
        Json, Router,
    };
    use futures::TryStreamExt;
    use serde::{Deserialize, Serialize};
    use storage_core::{
        ingress::{OpenAPI, ToRestService},
        traits::{Pagination, Request, StreamingOptions},
    };
    use storage_service_protos::nvidia::omniverse::metadata::v1beta::*;
    use tonic::metadata::MetadataMap;
    use tracing::error;
    use utoipa::{OpenApi, ToSchema};

    use crate::{
        api::{
            metadata_api::{adapters, v1beta::Metadata},
            rest_utils::ResponseBuilder,
        },
        backend::metadata::Metadata as MetadataTrait,
        common::request::RequestExt,
    };

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
                    "/v1beta/metadata",
                    post(get_metadata_batch)
                        .put(update_metadata_batch)
                        .delete(delete_metadata_batch),
                )
                .route("/v1beta/metadata/query", post(query_metadata))
                .with_state(self.clone())
        }

        fn openapi(&self) -> Option<OpenAPI> {
            Some(OpenAPI::new(
                "metadata-api/v1beta",
                "/v1beta/metadata/openapi.json",
                true,
                ApiDoc::openapi(),
            ))
        }
    }

    #[utoipa::path(
        post,
        path = "/v1beta/metadata",
        responses(
            (status = 200, description = "Successful response.", body = GetMetadataBatchResponse),
        ),
        tag = "v1beta",
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
                adapters::get_metadata_batch_request(body),
            ))
            .await
            .map_err(|error| error.http_status())
            .inspect_err(|status| error!(?status, "get metadata batch failed"))?;

        ResponseBuilder::new()
            .status(StatusCode::OK)
            .json(&adapters::get_metadata_batch_response(response))
    }

    #[utoipa::path(
        post,
        path = "/v1beta/metadata/query",
        responses(
            (status = 200, description = "Successful response.", body = QueryMetadataResponseBody),
        ),
        tag = "v1beta",
    )]
    async fn query_metadata<T: MetadataTrait>(
        headers: HeaderMap,
        State(state): State<Metadata<T>>,
        Json(body): Json<QueryMetadataRequestBody>,
    ) -> Result<Response, StatusCode> {
        let mut responses = state
            .inner
            .query_metadata(
                Request::from_parts(
                    MetadataMap::from_headers(headers),
                    Default::default(),
                    adapters::query_metadata_request(body.request),
                ),
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
                response: adapters::query_metadata_response(page.response),
            })
            .unwrap_or_default();

        ResponseBuilder::new().status(StatusCode::OK).json(&response)
    }

    #[utoipa::path(
        put,
        path = "/v1beta/metadata",
        request_body = UpdateMetadataBatchRequest,
        responses(
            (status = 200, description = "Successful response.", body = UpdateMetadataBatchResponse),
        ),
        tag = "v1beta",
    )]
    async fn update_metadata_batch<T: MetadataTrait>(
        headers: HeaderMap,
        State(state): State<Metadata<T>>,
        Json(body): Json<UpdateMetadataBatchRequest>,
    ) -> Result<Response, StatusCode> {
        let request = Request::from_parts(
            MetadataMap::from_headers(headers),
            Default::default(),
            adapters::update_metadata_batch_request(body),
        );

        let response = state
            .inner
            .update_metadata_batch(request)
            .await
            .map_err(|error| error.http_status())
            .inspect_err(|status| error!(?status, "update metadata batch request failed"))?;

        ResponseBuilder::new()
            .status(StatusCode::OK)
            .json(&adapters::update_metadata_batch_response(response))
    }

    #[utoipa::path(
        delete,
        path = "/v1beta/metadata",
        responses(
            (status = 200, description = "Successful response.", body = DeleteMetadataBatchResponse),
        ),
        tag = "v1beta",
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
                adapters::delete_metadata_batch_request(body),
            ))
            .await
            .map_err(|error| error.http_status())
            .inspect_err(|status| error!(?status, "delete metadata batch failed"))?;

        ResponseBuilder::new()
            .status(StatusCode::OK)
            .json(&adapters::delete_metadata_batch_response(response))
    }

    #[derive(OpenApi)]
    #[openapi(
        paths(update_metadata_batch),
        tags((name = "v1beta", description = "Metadata API")),
        components(schemas(
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
}
