// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::atomic::Ordering;

use axum::{
    body::Body,
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
use utoipa::{OpenApi, ToSchema};

use crate::api_services::rest_utils::json_response;
use crate::{
    ingress::rest::ToRestService,
    traits::{
        self,
        filefolder::v1beta::{DeleteFolderRequest, FolderAddress, ListRequest, ListStatRequest},
        filefolder::v1beta::{
            FileFolder as FileFolderTrait, ListResponseWithContinueToken, ListStatResponseWithContinueToken,
        },
        fileobject::v1beta::{Metadata, ResourceIdentity},
        Extensions, MetadataMap, Pagination, StreamingOptions,
    },
};

use super::super::super::REQUEST_ID;
use super::FileFolder;

impl<L> ToRestService for FileFolder<L>
where
    L: traits::filefolder::v1beta::FileFolder,
{
    fn service(&self) -> axum::Router {
        Router::new()
            .route(
                "/v1beta/filefolder/list/{folder_address}",
                get(list).delete(delete_folder),
            )
            .route("/v1beta/filefolder/liststat/{folder_address}", get(list_stat))
            .with_state(self.clone())
    }

    fn openapi(&self) -> Option<crate::ingress::rest::OpenAPI> {
        Some(crate::ingress::rest::OpenAPI::new(
            "filefolder/v1beta",
            "/v1beta/filefolder/openapi.json",
            false,
            ApiDoc::openapi(),
        ))
    }
}

#[utoipa::path(
    get,
    path = "/v1beta/filefolder/list/{folder_address}",
    params(
        ("folder_address" = String, Path, description = "The address of the folder to list"),
        ("query" = ListQueryParams, Query, description = "Query parameters for the list request"),
    ),
    responses(
        (status = 200, description = "List request successful", body = ListResponse)
    ),
    tag = "v1beta"
)]
#[instrument(name = "rest::list", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
async fn list<S: FileFolderTrait>(
    headers: HeaderMap,
    State(state): State<FileFolder<S>>,
    Path(folder_address): Path<String>,
    Query(query): Query<ListQueryParams>,
) -> Result<AxumResponse, ProblemDetails> {
    let _slow = log_if_slow!(guard: "rest::list").with_context(folder_address.as_str());
    let request = ListRequest {
        folder: Some(FolderAddress {
            uri: folder_address.clone(),
        }),
    };

    let mut stream = state
        .file_folder
        .list(
            tonic::Request::from_parts(MetadataMap::from_headers(headers), Extensions::new(), request).into(),
            traits::StreamingOptions::Paginated(Pagination {
                continuation_handle: query.continuation_handle,
                max_page_size: query.max_page_size,
            }),
        )
        .await
        .map_err(|e| log_http_error(e.as_ref(), &folder_address))?;

    let Some(response) = stream.next().await else {
        error_limited!(code = %StatusCode::INTERNAL_SERVER_ERROR, context = folder_address, "Failed to receive data from List stream");
        return Err(ProblemDetails::from(StatusCode::INTERNAL_SERVER_ERROR));
    };
    match response {
        Ok(response) => json_response(StatusCode::OK, &ListResponse::from(response)),
        Err(e) => Err(log_http_error(e.as_ref(), &folder_address)),
    }
}

#[utoipa::path(
    get,
    path = "/v1beta/filefolder/liststat/{folder_address}",
    params(
        ("folder_address" = String, Path, description = "The address of the folder to list"),
        ("query" = ListStatQueryParams, Query, description = "Query parameters for the list stat request"),
    ),
    responses(
        (status = 200, description = "List stat request successful", body = ListStatResponse)
    ),
    tag = "v1beta"
)]
#[instrument(name = "rest::list_stat", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
async fn list_stat<S: FileFolderTrait>(
    headers: HeaderMap,
    State(state): State<FileFolder<S>>,
    Path(folder_address): Path<String>,
    Query(query): Query<ListStatQueryParams>,
) -> Result<AxumResponse, ProblemDetails> {
    let _slow = log_if_slow!(guard: "rest::list_stat").with_context(folder_address.as_str());
    let request = ListStatRequest {
        folder: Some(FolderAddress {
            uri: folder_address.clone(),
        }),
    };

    let mut stream = state
        .file_folder
        .list_stat(
            tonic::Request::from_parts(MetadataMap::from_headers(headers), Extensions::new(), request).into(),
            StreamingOptions::Paginated(Pagination {
                continuation_handle: query.continuation_handle,
                max_page_size: query.max_page_size,
            }),
        )
        .await
        .map_err(|e| log_http_error(e.as_ref(), &folder_address))?;

    let Some(response) = stream.next().await else {
        error_limited!(code = %StatusCode::INTERNAL_SERVER_ERROR, context = folder_address, "Failed to receive data from ListStat stream");
        return Err(ProblemDetails::from(StatusCode::INTERNAL_SERVER_ERROR));
    };
    match response {
        Ok(response) => json_response(StatusCode::OK, &ListStatResponse::from(response)),
        Err(e) => Err(log_http_error(e.as_ref(), &folder_address)),
    }
}

#[utoipa::path(
    delete,
    path = "/v1beta/filefolder/list/{folder_address}",
    params(("folder_address" = String, Path, description = "The address of the folder to delete")),
    responses(
        (status = 204, description = "Delete folder request successful")
    ),
    tag = "v1beta"
)]
#[instrument(name = "rest::delete_folder", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
async fn delete_folder<S: FileFolderTrait>(
    headers: HeaderMap,
    State(state): State<FileFolder<S>>,
    Path(folder_address): Path<String>,
) -> Result<AxumResponse, ProblemDetails> {
    let _slow = log_if_slow!(guard: "rest::delete_folder").with_context(folder_address.as_str());
    let request = DeleteFolderRequest {
        folder: Some(FolderAddress {
            uri: folder_address.clone(),
        }),
    };
    state
        .file_folder
        .delete_folder(
            tonic::Request::from_parts(MetadataMap::from_headers(headers), Extensions::new(), request).into(),
        )
        .await
        .map_err(|e| log_http_error(e.as_ref(), &folder_address))?;

    AxumResponse::builder()
        .status(StatusCode::NO_CONTENT)
        .body(Body::empty())
        .map_err(|e| {
            error_limited!(code = %StatusCode::INTERNAL_SERVER_ERROR, context = folder_address, error = %e);
            ProblemDetails::from(StatusCode::INTERNAL_SERVER_ERROR)
        })
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ListQueryParams {
    continuation_handle: Option<String>,
    max_page_size: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ListStatQueryParams {
    continuation_handle: Option<String>,
    max_page_size: Option<u32>,
}

// The ListItems definition is different between Rest and GRPC, so we need to manually create the Rest version
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ListResponse {
    pub subfolder_addresses: Vec<String>,
    pub sub_resource_addresses: Vec<String>,
    pub next_continuation_handle: Option<String>,
}

impl From<ListResponseWithContinueToken> for ListResponse {
    fn from(response: ListResponseWithContinueToken) -> Self {
        Self {
            subfolder_addresses: response
                .response
                .subfolder_addresses
                .into_iter()
                .map(|f| f.uri)
                .collect(),
            sub_resource_addresses: response.response.sub_resource_addresses,
            next_continuation_handle: response.continue_token,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ListItem {
    pub resource_address: String,
    pub resource_identity: Option<ResourceIdentity>,
    pub metadata: Option<Metadata>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ListStatResponse {
    pub subfolder_addresses: Vec<String>,
    pub entries: Vec<ListItem>,
    pub next_continuation_handle: Option<String>,
}

impl From<ListStatResponseWithContinueToken> for ListStatResponse {
    fn from(response: ListStatResponseWithContinueToken) -> Self {
        Self {
            subfolder_addresses: response
                .response
                .subfolder_addresses
                .into_iter()
                .map(|f| f.uri)
                .collect(),
            entries: response
                .response
                .entries
                .into_iter()
                .map(|item| ListItem {
                    resource_address: item.resource_address,
                    resource_identity: item
                        .resource_info
                        .as_ref()
                        .map(|info| info.resource_identity.clone())
                        .unwrap_or_default(),
                    metadata: item.resource_info.map(|info| info.metadata).unwrap_or_default(),
                })
                .collect(),
            next_continuation_handle: response.continue_token,
        }
    }
}

#[derive(OpenApi)]
#[openapi(
    paths(list, list_stat, delete_folder),
    components(schemas(ListQueryParams, ListStatQueryParams, ListResponse, ListStatResponse)),
    tags(
        (name = "filefolder", description = "Storage Service File Folder API")
    )
)]
pub struct ApiDoc;
