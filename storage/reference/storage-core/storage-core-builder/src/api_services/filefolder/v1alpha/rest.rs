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
        filefolder::v1alpha::{
            CreateFolderRequest, DeleteFolderRequest, FileFolder as FileFolderTrait, FolderAddress,
            GetFolderModeRequest, GetFolderModeResponse, ListRequest, ListResponseWithContinueToken, ListStatRequest,
            ListStatResponseWithContinueToken,
        },
        fileobject::v1alpha::{Metadata, ResourceIdentity},
        Extensions, MetadataMap, Pagination, StreamingOptions,
    },
};

use super::super::super::REQUEST_ID;
use super::FileFolder;

impl<L> ToRestService for FileFolder<L>
where
    L: traits::filefolder::v1alpha::FileFolder,
{
    fn service(&self) -> axum::Router {
        Router::new()
            .route("/v1alpha/filefolder/{folder_address}", put(create_folder))
            .route(
                "/v1alpha/filefolder/list/{folder_address}",
                get(list).delete(delete_folder),
            )
            .route("/v1alpha/filefolder/liststat/{folder_address}", get(list_stat))
            .route(
                "/v1alpha/filefolder/get-folder-mode/{folder_address}",
                get(get_folder_mode),
            )
            .with_state(self.clone())
    }

    fn openapi(&self) -> Option<crate::ingress::rest::OpenAPI> {
        Some(crate::ingress::rest::OpenAPI::new(
            "filefolder/v1alpha",
            "/v1alpha/filefolder/openapi.json",
            false,
            ApiDoc::openapi(),
        ))
    }
}

#[utoipa::path(
    put,
    path = "/v1alpha/filefolder/{folder_address}",
    params(("folder_address" = String, Path, description = "The address of the folder to create")),
    responses(
        (status = 204, description = "Create folder request successful"),
        (status = 400, description = "Invalid folder address"),
        (status = 409, description = "Folder already exists")
    ),
    tag = "v1alpha"
)]
#[instrument(name = "rest::create_folder", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
async fn create_folder<S: FileFolderTrait>(
    headers: HeaderMap,
    State(state): State<FileFolder<S>>,
    Path(folder_address): Path<String>,
) -> Result<AxumResponse, ProblemDetails> {
    let _slow = log_if_slow!(guard: "rest::create_folder").with_context(folder_address.as_str());
    let request = CreateFolderRequest {
        folder: Some(FolderAddress {
            uri: folder_address.clone(),
        }),
    };
    state
        .file_folder
        .create_folder(
            tonic::Request::from_parts(MetadataMap::from_headers(headers), Extensions::new(), request).into(),
        )
        .await
        .map_err(|e| {
            // The underlying code generates a PRECONDITION_FAILED error if the folder exists as a file,
            // but the api spec says that we should return a CONFLICT error.
            let mut details = ProblemDetails::from_error(e.as_ref());
            if e.http_status() == StatusCode::PRECONDITION_FAILED {
                details.status = StatusCode::CONFLICT.as_u16();
                details.title = StatusCode::CONFLICT.canonical_reason().unwrap_or("Conflict");
            }
            error_limited!(code = details.status, context = folder_address, error = %e);
            details
        })?;

    AxumResponse::builder()
        .status(StatusCode::NO_CONTENT)
        .body(Body::empty())
        .map_err(|e| {
            error_limited!(code = %StatusCode::INTERNAL_SERVER_ERROR, context = folder_address, error = %e);
            ProblemDetails::from(StatusCode::INTERNAL_SERVER_ERROR)
        })
}

#[utoipa::path(
    get,
    path = "/v1alpha/filefolder/get-folder-mode/{folder_address}",
    params(("folder_address" = String, Path, description = "The address of the folder to get the mode of")),
    responses(
        (status = 200, description = "Get folder mode request successful", body = GetFolderModeResponse),
        (status = 400, description = "Invalid folder address")
    ),
    tag = "v1alpha"
)]
#[instrument(name = "rest::get_folder_mode", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
async fn get_folder_mode<S: FileFolderTrait>(
    headers: HeaderMap,
    State(state): State<FileFolder<S>>,
    Path(folder_address): Path<String>,
) -> Result<AxumResponse, ProblemDetails> {
    let _slow = log_if_slow!(guard: "rest::get_folder_mode").with_context(folder_address.as_str());
    let request = GetFolderModeRequest {
        folder: Some(FolderAddress {
            uri: folder_address.clone(),
        }),
    };
    let result = state
        .file_folder
        .get_folder_mode(
            tonic::Request::from_parts(MetadataMap::from_headers(headers), Extensions::new(), request).into(),
        )
        .await
        .map_err(|e| log_http_error(e.as_ref(), &folder_address))?;

    json_response(StatusCode::OK, &result)
}

#[utoipa::path(
    get,
    path = "/v1alpha/filefolder/list/{folder_address}",
    params(
        ("folder_address" = String, Path, description = "The address of the folder to list"),
        ("query" = ListQueryParams, Query, description = "Query parameters for the list request"),
    ),
    responses(
        (status = 200, description = "List request successful", body = ListResponse)
    ),
    tag = "v1alpha"
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
    path = "/v1alpha/filefolder/liststat/{folder_address}",
    params(
        ("folder_address" = String, Path, description = "The address of the folder to list"),
        ("query" = ListStatQueryParams, Query, description = "Query parameters for the list stat request"),
    ),
    responses(
        (status = 200, description = "List stat request successful", body = ListStatResponse)
    ),
    tag = "v1alpha"
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
        include_native_metadata: query.include_native_metadata.unwrap_or(false),
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
    path = "/v1alpha/filefolder/list/{folder_address}",
    params(("folder_address" = String, Path, description = "The address of the folder to delete")),
    responses(
        (status = 204, description = "Delete folder request successful")
    ),
    tag = "v1alpha"
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
    /// Include FileObject-native metadata for returned file entries. May cost extra
    /// backend calls on stores whose list does not return object metadata; honored
    /// only when the backend has native metadata enabled.
    include_native_metadata: Option<bool>,
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
