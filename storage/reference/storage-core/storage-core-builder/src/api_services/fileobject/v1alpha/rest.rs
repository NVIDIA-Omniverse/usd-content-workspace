// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::atomic::Ordering;

// Third-party imports
use axum::{
    body::Body,
    extract::{self, Path, Query, State},
    response::Response as AxumResponse,
    routing::*,
};
use futures::StreamExt;
use http::HeaderMap;
use http::StatusCode;
use storage_core_util::{error_limited, log_if_slow};
use tracing::instrument;

use crate::api_services::error::{log_http_error, ProblemDetails};
use utoipa::OpenApi;

// Local imports
use super::super::super::REQUEST_ID;
use super::super::utils::collect_request_body_future;
use super::types::*;
use super::utils::*;
use crate::api_services::fileobject::canonicalize_identity;
use crate::api_services::fileobject::v1alpha::FileObject;
use crate::api_services::rest_utils::{empty_response, json_response};
use crate::traits::{
    fileobject::v1alpha::{
        read_from_address_response, read_response, write_response::WriteResponseType, AbortMultipartUploadRequest,
        CompleteMultipartUploadRequest, CompleteRedirectUploadRequest, CompletedUploadPart, CopyRequest, CopyResponse,
        DeleteRequest, EnumerateRequest, FetchWriteTypeInfoRequest, GetNativeMetadataSupportRequest,
        GetNativeMetadataSupportResponse, GetOptimisticLockingSupportRequest, GetOptimisticLockingSupportResponse,
        Header, IsReadOnlyRequest, IsReadOnlyResponse, MoveRequest, MoveResponse, ReadFromAddressRequest, ReadRequest,
        Redirect, ResourceIdentity, ResourceInfo, StatRequest, UploadPartRequest, UploadPreference, WriteParameters,
    },
    Extensions, MetadataMap, Pagination, StreamingOptions,
};
use crate::{ingress::rest::ToRestService, traits::fileobject::v1alpha::FileObject as FileObjectTrait};

impl<L> ToRestService for FileObject<L>
where
    L: FileObjectTrait,
{
    fn service(&self) -> axum::Router {
        axum::Router::new()
            .route(
                "/v1alpha/fileobject/by-address/{resource_address}",
                head(stat_by_address)
                    .get(read_by_address)
                    .put(write_by_address)
                    .delete(delete_by_address),
            )
            .route(
                "/v1alpha/fileobject/by-address/{resource_address}/multipart/abort",
                post(abort_multipart_upload),
            )
            .route(
                "/v1alpha/fileobject/by-address/{resource_address}/multipart/complete",
                post(complete_multipart_upload),
            )
            .route(
                "/v1alpha/fileobject/by-address/{resource_address}/multipart/prepare",
                post(prepare_multipart_upload),
            )
            .route(
                "/v1alpha/fileobject/by-address/{resource_address}/redirect/complete",
                post(complete_redirect_upload),
            )
            .route(
                "/v1alpha/fileobject/by-identity/{resource_identity}",
                get(read_by_identity),
            )
            .route("/v1alpha/fileobject/data-objects/{resource_address}", get(enumerate))
            .route(
                "/v1alpha/fileobject/upload-options/by-address/{resource_address}",
                get(fetch_write_type_info),
            )
            .route("/v1alpha/fileobject/by-identity/{resource_identity}/copy", post(copy))
            .route(
                "/v1alpha/fileobject/by-address/{resource_address}/move",
                post(move_object),
            )
            .route(
                "/v1alpha/fileobject/optimistic-locking-support/{resource_address}",
                get(get_optimistic_locking_support),
            )
            .route(
                "/v1alpha/fileobject/native-metadata-support/{resource_address}",
                get(get_native_metadata_support),
            )
            .route("/v1alpha/fileobject/is-read-only/{resource_address}", get(is_read_only))
            .with_state(self.clone())
    }

    fn openapi(&self) -> Option<crate::ingress::rest::OpenAPI> {
        Some(crate::ingress::rest::OpenAPI::new(
            "fileobject/v1alpha",
            "/v1alpha/fileobject/openapi.json",
            true,
            ApiDoc::openapi(),
        ))
    }
}

#[utoipa::path(
    head,
    path = "/v1alpha/fileobject/by-address/{resource_address}",
    params(
        ("resource_address" = String, Path, description = "The address of the file object to stat"),
    ),
    responses(
        (status = 204, description = "File object stat successful",
            headers(
                ("x-nvidia-omniverse-storage-metadata" = String, description = "Metadata of the file object encoded in JSON"),
                ("x-nvidia-omniverse-storage-resource-identity" = String, description = "Resource identity of the file object"),
            ),
        )
    ),
    tag = "v1alpha",
)]
#[instrument(name = "rest::stat_by_address", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
async fn stat_by_address<S: FileObjectTrait>(
    headers: HeaderMap,
    State(state): State<FileObject<S>>,
    Path(resource_address): Path<String>,
) -> Result<AxumResponse, ProblemDetails> {
    let _slow = log_if_slow!(guard: "rest::stat_by_address").with_context(resource_address.as_str());
    let request = StatRequest {
        resource_address: resource_address.clone(),
    };
    let result = state
        .file_object
        .stat(tonic::Request::from_parts(MetadataMap::from_headers(headers), Extensions::new(), request).into())
        .await
        .map_err(|e| log_http_error(e.as_ref(), &resource_address))?;
    let Some(resource_info) = result.resource_info else {
        error_limited!(
            code = %StatusCode::INTERNAL_SERVER_ERROR,
            context = resource_address,
            "ResourceInfo is not present in the stat response"
        );
        return Err(ProblemDetails::from(StatusCode::INTERNAL_SERVER_ERROR));
    };

    let builder = AxumResponse::builder();
    add_headers_from_resource_info(resource_info, builder)?
        .status(StatusCode::NO_CONTENT)
        .body(Body::empty())
        .map_err(|e| {
            error_limited!(
                code = %StatusCode::INTERNAL_SERVER_ERROR,
                context = resource_address,
                error = %e
            );
            ProblemDetails::from(StatusCode::INTERNAL_SERVER_ERROR)
        })
}

#[utoipa::path(
    get,
    path = "/v1alpha/fileobject/by-address/{resource_address}",
    params(
        ("resource_address" = String, Path, description = "The address of the file object to read"),
        ("query" = ReadByAddressQueryParams, Query, description = "Query parameters for the read request"),
    ),
    responses(
        (status = 200, description = "File object read successful, data in body",
            headers(
                ("x-nvidia-omniverse-storage-metadata" = String, description = "Metadata of the file object encoded in JSON"),
                ("x-nvidia-omniverse-storage-resource-identity" = String, description = "Resource identity of the file object"),
            ),
        ),
        (status = 300, description = "Redirect to an HTTP server via pre-signed URL, returns redirection information as JSON data.",
            headers(
                ("x-nvidia-omniverse-storage-metadata" = String, description = "Metadata of the file object encoded in JSON"),
                ("x-nvidia-omniverse-storage-resource-identity" = String, description = "Resource identity of the file object"),
            ),
            body = Redirect,
        ),
    ),
    tag = "v1alpha",
)]
#[instrument(name = "rest::read_by_address", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
async fn read_by_address<S: FileObjectTrait>(
    headers: HeaderMap,
    State(state): State<FileObject<S>>,
    Path(resource_address): Path<String>,
    Query(query): Query<ReadByAddressQueryParams>,
) -> Result<AxumResponse, ProblemDetails> {
    let _slow = log_if_slow!(guard: "rest::read_by_address").with_context(resource_address.as_str());
    let request = ReadFromAddressRequest {
        resource_address: resource_address.clone(),
        download_preference: query.download_preference.map(|d| d.into()),
    };
    let mut stream = state
        .file_object
        .read_from_address(
            tonic::Request::from_parts(MetadataMap::from_headers(headers), Extensions::new(), request).into(),
        )
        .await
        .map_err(|e| log_http_error(e.as_ref(), &resource_address))?;

    // The first message should be a ResourceInfo
    let response = match stream.next().await {
        Some(Ok(response)) => response,
        Some(Err(e)) => {
            error_limited!(
                code = %e.http_status(),
                context = resource_address,
                error = %e
            );
            return Err(ProblemDetails::from_error(e.as_ref()));
        },
        None => {
            error_limited!(
                code = %StatusCode::INTERNAL_SERVER_ERROR,
                context = resource_address,
                "Failed to receive data from Storage Service stream"
            );
            return Err(ProblemDetails::from(StatusCode::INTERNAL_SERVER_ERROR));
        },
    };
    let Some(read_from_address_response::ReplyType::ResourceInfo(resource_info)) = response.reply_type else {
        error_limited!(
            code = %StatusCode::INTERNAL_SERVER_ERROR,
            context = resource_address,
            "First message from Storage Service stream is not a ResourceInfo"
        );
        return Err(ProblemDetails::from(StatusCode::INTERNAL_SERVER_ERROR));
    };

    // Create a peekable stream to look at the next message without consuming it
    let mut peek = stream.peekable();
    match std::pin::Pin::new(&mut peek).peek().await {
        Some(Ok(response)) => match &response.reply_type {
            Some(read_from_address_response::ReplyType::ResourceInfo(_)) => {
                error_limited!(
                    code = %StatusCode::INTERNAL_SERVER_ERROR,
                    context = resource_address,
                    "Second message from Storage Service stream is a ResourceInfo"
                );
                Err(ProblemDetails::from(StatusCode::INTERNAL_SERVER_ERROR))
            },
            // A chunk indicates the data is being streamed back to the client
            Some(read_from_address_response::ReplyType::Chunk(_)) => {
                let builder = AxumResponse::builder();
                let context_stream = resource_address.clone();
                Ok(add_headers_from_resource_info(resource_info, builder)?
                    .header("Content-Type", "application/octet-stream")
                    // Map the stream to return the bytes from the chunk
                    .body(Body::from_stream(peek.map(move |r| match r {
                        Ok(r) => match r.reply_type {
                            Some(read_from_address_response::ReplyType::Chunk(chunk)) => Ok(chunk.chunk),
                            _ => {
                                error_limited!(
                                    code = %StatusCode::INTERNAL_SERVER_ERROR,
                                    context = context_stream,
                                    "Unexpected Non-Chunk message in read response"
                                );
                                Err(Box::new(InternalError {
                                    message: "Unexpected Non-Chunk message in read response".to_string(),
                                }) as axum::BoxError)
                            },
                        },
                        Err(e) => {
                            log_http_error(e.as_ref(), &context_stream);
                            Err(e as axum::BoxError)
                        },
                    })))
                    .map_err(|e| {
                        error_limited!(code = %StatusCode::INTERNAL_SERVER_ERROR, context = resource_address, error = %e);
                        ProblemDetails::from(StatusCode::INTERNAL_SERVER_ERROR)
                    })?)
            },
            // Otherwise it's a redirect
            Some(read_from_address_response::ReplyType::Redirect(redirect)) => {
                let builder = AxumResponse::builder();
                Ok(add_headers_from_resource_info(resource_info, builder)?
                    .header("Content-Type", "application/json")
                    .status(StatusCode::MULTIPLE_CHOICES)
                    .body(Body::from(serde_json::to_string(&redirect).map_err(|e| {
                        error_limited!(code = %StatusCode::INTERNAL_SERVER_ERROR, context = resource_address, error = %e);
                        ProblemDetails::from(StatusCode::INTERNAL_SERVER_ERROR)
                    })?))
                    .map_err(|e| {
                        error_limited!(code = %StatusCode::INTERNAL_SERVER_ERROR, context = resource_address, error = %e);
                        ProblemDetails::from(StatusCode::INTERNAL_SERVER_ERROR)
                    })?)
            },
            None => {
                error_limited!(code = %StatusCode::INTERNAL_SERVER_ERROR, context = resource_address, "Storage Service stream ended unexpectedly");
                Err(ProblemDetails::from(StatusCode::INTERNAL_SERVER_ERROR))
            },
        },
        Some(Err(e)) => Err(ProblemDetails::from_error(e.as_ref())),
        None => {
            // If the object is empty, no chunks will be returned, but it's still a valid response
            if let Some(metadata) = &resource_info.metadata {
                if let Some(data_object_size) = metadata.data_object_size {
                    if data_object_size == 0 {
                        let builder = AxumResponse::builder();
                        return add_headers_from_resource_info(resource_info, builder)?
                            .status(StatusCode::OK)
                            .body(Body::empty())
                            .map_err(|e| {
                                error_limited!(code = %StatusCode::INTERNAL_SERVER_ERROR, context = resource_address, error = %e);
                                ProblemDetails::from(StatusCode::INTERNAL_SERVER_ERROR)
                            });
                    }
                }
            }
            error_limited!(code = %StatusCode::INTERNAL_SERVER_ERROR, context = resource_address, "Storage Service stream ended unexpectedly");
            Err(ProblemDetails::from(StatusCode::INTERNAL_SERVER_ERROR))
        },
    }
}

#[utoipa::path(
    put,
    path = "/v1alpha/fileobject/by-address/{resource_address}",
    params(
        ("resource_address" = String, Path, description = "The address of the file object to write"),
        ("query" = WriteByAddressQueryParams, Query, description = "Query parameters for the write request"),
    ),
    responses(
        (status = 201, description = "File object write successful", body = ResourceInfo),
        (status = 300, description = "Either a redirect to a different upload location or multipart request properties.", body = WriteResponse),
        (status = 400, description = "Invalid request: fail_if_exists combined with previous_version, or other validation error."),
        (status = 409, description = "Resource already exists at the destination (fail_if_exists=true)."),
        (status = 501, description = "The backend does not support fail_if_exists for write."),
    ),
    tag = "v1alpha",
)]
#[instrument(name = "rest::write_by_address", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
async fn write_by_address<S: FileObjectTrait>(
    headers: HeaderMap,
    State(state): State<FileObject<S>>,
    Path(resource_address): Path<String>,
    Query(query): Query<WriteByAddressQueryParams>,
    request: extract::Request,
) -> Result<AxumResponse, ProblemDetails> {
    let _slow = log_if_slow!(guard: "rest::write_by_address").with_context(resource_address.as_str());
    let params = WriteParameters {
        destination_resource_address: resource_address.clone(),
        // REST write does not yet accept native metadata; gRPC clients set it. Follow-up.
        native_metadata: Default::default(),
        upload_preference: query.upload_preference.map(|u| match u.as_str() {
            "body" => UploadPreference::Body as i32,
            "redirect" => UploadPreference::Redirect as i32,
            "multipart" => UploadPreference::Multipart as i32,
            _ => UploadPreference::Unspecified as i32,
        }),
        previous_version: query.previous_version.map(|v| ResourceIdentity { encoded_identity: v }),
        fail_if_exists: query.fail_if_exists,
        data_object_size: query.data_object_size,
    };
    let mut stream = state
        .file_object
        .write(
            tonic::Request::from_parts(MetadataMap::from_headers(headers), Extensions::new(), params).into(),
            collect_request_body_future(request, query.data_object_size as usize),
        )
        .await
        .map_err(|e| log_http_error(e.as_ref(), &resource_address))?;

    match stream.next().await {
        Some(Ok(response)) => match response.write_response_type {
            Some(response) => match response {
                WriteResponseType::WriteChunksAccepted(_) => {
                    // Next message should be a ResourceInfo
                    let response = match stream.next().await {
                        Some(Ok(response)) => response,
                        Some(Err(e)) => {
                            return Err(log_http_error(e.as_ref(), &resource_address));
                        },
                        None => {
                            error_limited!(code = %StatusCode::INTERNAL_SERVER_ERROR, context = resource_address, "Unexpected end of stream");
                            return Err(ProblemDetails::from(StatusCode::INTERNAL_SERVER_ERROR));
                        },
                    };

                    let Some(WriteResponseType::ResourceInfo(resource_info)) = response.write_response_type else {
                        error_limited!(code = %StatusCode::INTERNAL_SERVER_ERROR, context = resource_address, "Unexpected ResourceInfo in write response");
                        return Err(ProblemDetails::from(StatusCode::INTERNAL_SERVER_ERROR));
                    };

                    json_response(StatusCode::CREATED, &resource_info)
                },
                WriteResponseType::WriteRedirect(redirect) => json_response(
                    StatusCode::MULTIPLE_CHOICES,
                    &WriteRedirectResponse {
                        redirect: WriteRedirectProperties::from(redirect),
                    },
                ),
                WriteResponseType::MultipartUpload(multipart_upload) => json_response(
                    StatusCode::MULTIPLE_CHOICES,
                    &MultipartUploadResponse {
                        multipart: CreateMultipartUploadResponse::try_from(multipart_upload)?,
                    },
                ),
                WriteResponseType::ResourceInfo(_) => {
                    error_limited!(code = %StatusCode::INTERNAL_SERVER_ERROR, context = resource_address, "Unexpected ResourceInfo in write response");
                    Err(ProblemDetails::from(StatusCode::INTERNAL_SERVER_ERROR))
                },
            },
            None => {
                error_limited!(
                    code = %StatusCode::INTERNAL_SERVER_ERROR,
                    context = resource_address,
                    "Write response type is not present in the response"
                );
                Err(ProblemDetails::from(StatusCode::INTERNAL_SERVER_ERROR))
            },
        },
        Some(Err(e)) => Err(log_http_error(e.as_ref(), &resource_address)),
        None => {
            error_limited!(code = %StatusCode::INTERNAL_SERVER_ERROR, context = resource_address, "Write stream ended unexpectedly");
            Err(ProblemDetails::from(StatusCode::INTERNAL_SERVER_ERROR))
        },
    }
}

#[utoipa::path(
    delete,
    path = "/v1alpha/fileobject/by-address/{resource_address}",
    params(
        ("resource_address" = String, Path, description = "The address of the file object to delete"),
        ("query" = DeleteByAddressQueryParams, Query, description = "Query parameters for the delete request"),
    ),
    responses(
        (status = 204, description = "File object deleted successfully"),
    ),
    tag = "v1alpha",
)]
#[instrument(name = "rest::delete_by_address", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
async fn delete_by_address<S: FileObjectTrait>(
    headers: HeaderMap,
    State(state): State<FileObject<S>>,
    Path(resource_address): Path<String>,
    Query(query): Query<DeleteByAddressQueryParams>,
) -> Result<AxumResponse, ProblemDetails> {
    let _slow = log_if_slow!(guard: "rest::delete_by_address").with_context(resource_address.as_str());
    let request = DeleteRequest {
        resource_address: resource_address.clone(),
        previous_version: query.previous_version.map(|v| ResourceIdentity { encoded_identity: v }),
    };
    state
        .file_object
        .delete(tonic::Request::from_parts(MetadataMap::from_headers(headers), Extensions::new(), request).into())
        .await
        .map_err(|e| log_http_error(e.as_ref(), &resource_address))?;
    empty_response(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/v1alpha/fileobject/by-address/{resource_address}/multipart/abort",
    params(
        ("resource_address" = String, Path, description = "The address of the file object to abort multipart upload"),
        ("payload" = AbortMultipartUploadRequestBody, description = "The request body"),
    ),
    responses(
        (status = 204, description = "Multipart upload aborted successfully")
    ),
    tag = "v1alpha",
)]
#[instrument(name = "rest::abort_multipart_upload", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
async fn abort_multipart_upload<S: FileObjectTrait>(
    headers: HeaderMap,
    State(state): State<FileObject<S>>,
    Path(resource_address): Path<String>,
    extract::Json(payload): extract::Json<AbortMultipartUploadRequestBody>,
) -> Result<AxumResponse, ProblemDetails> {
    let _slow = log_if_slow!(guard: "rest::abort_multipart_upload").with_context(resource_address.as_str());
    let request = AbortMultipartUploadRequest {
        upload_id: payload.upload_id,
        destination_resource_address: resource_address.clone(),
    };
    state
        .file_object
        .abort_multipart_upload(
            tonic::Request::from_parts(MetadataMap::from_headers(headers), Extensions::new(), request).into(),
        )
        .await
        .map_err(|e| log_http_error(e.as_ref(), &resource_address))?;
    empty_response(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/v1alpha/fileobject/by-address/{resource_address}/multipart/complete",
    params(
        ("resource_address" = String, Path, description = "The address of the file object to complete multipart upload"),
        ("payload" = CompleteMultipartUploadRequestBody, description = "The request body"),
    ),
    responses(
        (status = 200, description = "Multipart upload completed successfully", body = ResourceInfo)
    ),
    tag = "v1alpha",
)]
#[instrument(name = "rest::complete_multipart_upload", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
async fn complete_multipart_upload<S: FileObjectTrait>(
    headers: HeaderMap,
    State(state): State<FileObject<S>>,
    Path(resource_address): Path<String>,
    extract::Json(payload): extract::Json<CompleteMultipartUploadRequestBody>,
) -> Result<AxumResponse, ProblemDetails> {
    let _slow = log_if_slow!(guard: "rest::complete_multipart_upload").with_context(resource_address.as_str());
    let request = CompleteMultipartUploadRequest {
        upload_id: payload.upload_id,
        destination_resource_address: resource_address.clone(),
        parts: payload
            .parts
            .into_iter()
            .map(|p| CompletedUploadPart {
                part_number: p.part_number,
                headers: p
                    .additional_headers
                    .into_iter()
                    .map(|h| Header {
                        name: h.name,
                        value: h.value,
                    })
                    .collect(),
            })
            .collect(),
    };
    let result = state
        .file_object
        .complete_multipart_upload(
            tonic::Request::from_parts(MetadataMap::from_headers(headers), Extensions::new(), request).into(),
        )
        .await
        .map_err(|e| log_http_error(e.as_ref(), &resource_address))?;

    match result.resource_info {
        Some(resource_info) => json_response(StatusCode::OK, &resource_info),
        None => {
            error_limited!(
                code = %StatusCode::INTERNAL_SERVER_ERROR,
                context = resource_address,
                "ResourceInfo is not present in the complete multipart upload response"
            );
            Err(ProblemDetails::from(StatusCode::INTERNAL_SERVER_ERROR))
        },
    }
}

#[utoipa::path(
    post,
    path = "/v1alpha/fileobject/by-address/{resource_address}/multipart/prepare",
    params(
        ("resource_address" = String, Path, description = "The address of the file object to prepare multipart upload"),
        ("payload" = PrepareMultipartUploadRequestBody, description = "The request body"),
    ),
    responses(
        (status = 200, description = "Multipart upload prepared successfully", body = UploadPartResponse)
    ),
    tag = "v1alpha",
)]
#[instrument(name = "rest::prepare_multipart_upload", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
async fn prepare_multipart_upload<S: FileObjectTrait>(
    headers: HeaderMap,
    State(state): State<FileObject<S>>,
    Path(resource_address): Path<String>,
    extract::Json(payload): extract::Json<PrepareMultipartUploadRequestBody>,
) -> Result<AxumResponse, ProblemDetails> {
    let _slow = log_if_slow!(guard: "rest::prepare_multipart_upload").with_context(resource_address.as_str());
    let request = UploadPartRequest {
        upload_id: payload.upload_id,
        destination_resource_address: resource_address.clone(),
        part_number: payload.part_number,
        part_count: payload.part_count,
    };
    let result = state
        .file_object
        .upload_part(tonic::Request::from_parts(MetadataMap::from_headers(headers), Extensions::new(), request).into())
        .await
        .map_err(|e| log_http_error(e.as_ref(), &resource_address))?;

    json_response(StatusCode::OK, &UploadPartResponse::from(result))
}

#[utoipa::path(
    post,
    path = "/v1alpha/fileobject/by-address/{resource_address}/multipart/complete",
    params(
        ("resource_address" = String, Path, description = "The address of the file object to complete upload"),
        ("payload" = CompleteRedirectUploadRequestBody, description = "The request body"),
    ),
    responses(
        (status = 200, description = "Upload completed successfully", body = ResourceInfo)
    ),
    tag = "v1alpha",
)]
#[instrument(name = "rest::complete_redirect_upload", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
async fn complete_redirect_upload<S: FileObjectTrait>(
    headers: HeaderMap,
    State(state): State<FileObject<S>>,
    Path(resource_address): Path<String>,
    extract::Json(payload): extract::Json<CompleteRedirectUploadRequestBody>,
) -> Result<AxumResponse, ProblemDetails> {
    let _slow = log_if_slow!(guard: "rest::complete_redirect_upload").with_context(resource_address.as_str());
    let request = CompleteRedirectUploadRequest {
        destination_resource_address: resource_address.clone(),
        additional_headers: payload
            .additional_headers
            .into_iter()
            .map(|h| Header {
                name: h.name,
                value: h.value,
            })
            .collect(),
    };
    let result = state
        .file_object
        .complete_redirect_upload(
            tonic::Request::from_parts(MetadataMap::from_headers(headers), Extensions::new(), request).into(),
        )
        .await
        .map_err(|e| log_http_error(e.as_ref(), &resource_address))?;

    match result.resource_info {
        Some(resource_info) => json_response(StatusCode::OK, &resource_info),
        None => {
            error_limited!(
                code = %StatusCode::INTERNAL_SERVER_ERROR,
                context = resource_address,
                "ResourceInfo is not present in the complete multipart upload response"
            );
            Err(ProblemDetails::from(StatusCode::INTERNAL_SERVER_ERROR))
        },
    }
}

#[utoipa::path(
    get,
    path = "/v1alpha/fileobject/by-identity/{resource_identity}",
    params(
        ("resource_identity" = String, Path, description = "The identity of the file object to read"),
        ("query" = ReadByIdentityQueryParams, Query, description = "Query parameters for the read request"),
    ),
    responses(
        (status = 200, description = "File object read successful, data in body",
            headers(
                ("x-nvidia-omniverse-storage-metadata" = String, description = "Metadata of the file object encoded in JSON"),
                ("x-nvidia-omniverse-storage-resource-identity" = String, description = "Resource identity of the file object"),
            ),
        ),
        (status = 300, description = "Redirect to an HTTP server via pre-signed URL, returns redirection information as JSON data.",
            headers(
                ("x-nvidia-omniverse-storage-metadata" = String, description = "Metadata of the file object encoded in JSON"),
                ("x-nvidia-omniverse-storage-resource-identity" = String, description = "Resource identity of the file object"),
            ),
            body = Redirect,
        ),
    ),
    tag = "v1alpha",
)]
#[instrument(name = "rest::read_by_identity", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
async fn read_by_identity<S: FileObjectTrait>(
    headers: HeaderMap,
    State(state): State<FileObject<S>>,
    Path(resource_identity): Path<String>,
    Query(query): Query<ReadByIdentityQueryParams>,
) -> Result<AxumResponse, ProblemDetails> {
    let _slow = log_if_slow!(guard: "rest::read_by_identity").with_context(resource_identity.as_str());
    let request = ReadRequest {
        resource_identity: Some(ResourceIdentity {
            encoded_identity: resource_identity.clone(),
        }),
        download_preference: query.download_preference.map(|d| d.into()),
    };
    let mut stream = state
        .file_object
        .read(tonic::Request::from_parts(MetadataMap::from_headers(headers), Extensions::new(), request).into())
        .await
        .map_err(|e| log_http_error(e.as_ref(), &resource_identity))?;

    // The first message should be a ResourceInfo
    let response = match stream.next().await {
        Some(Ok(response)) => response,
        Some(Err(e)) => {
            return Err(log_http_error(e.as_ref(), &resource_identity));
        },
        None => {
            error_limited!(
                code = %StatusCode::INTERNAL_SERVER_ERROR,
                context = resource_identity,
                "Failed to receive data from Storage Service stream"
            );
            return Err(ProblemDetails::from(StatusCode::INTERNAL_SERVER_ERROR));
        },
    };
    let Some(read_response::ReplyType::Metadata(metadata)) = response.reply_type else {
        error_limited!(
            code = %StatusCode::INTERNAL_SERVER_ERROR,
            context = resource_identity,
            "First message from Storage Service stream is not Metadata"
        );
        return Err(ProblemDetails::from(StatusCode::INTERNAL_SERVER_ERROR));
    };
    let response_identity =
        canonicalize_identity(&resource_identity).map_err(|e| log_http_error(&e, &resource_identity))?;

    // Create a peekable stream to look at the next message without consuming it
    let mut peek = stream.peekable();
    match std::pin::Pin::new(&mut peek).peek().await {
        Some(Ok(response)) => match &response.reply_type {
            Some(read_response::ReplyType::Metadata(_)) => {
                error_limited!(
                    code = %StatusCode::INTERNAL_SERVER_ERROR,
                    context = resource_identity,
                    "Second message from Storage Service stream is a Metadata"
                );
                Err(ProblemDetails::from(StatusCode::INTERNAL_SERVER_ERROR))
            },
            // A chunk indicates the data is being streamed back to the client
            Some(read_response::ReplyType::Chunk(_)) => {
                let builder = AxumResponse::builder();
                let context_stream = resource_identity.clone();
                Ok(add_headers(
                    metadata,
                    ResourceIdentity {
                        encoded_identity: response_identity.clone(),
                    },
                    builder,
                )?
                .header("Content-Type", "application/octet-stream")
                // Map the stream to return the bytes from the chunk
                .body(Body::from_stream(peek.map(move |r| match r {
                    Ok(r) => match r.reply_type {
                        Some(read_response::ReplyType::Chunk(chunk)) => Ok(chunk.chunk),
                        _ => {
                                                            error_limited!(code = %StatusCode::INTERNAL_SERVER_ERROR, context = context_stream, "Unexpected error");
                            Err(Box::new(InternalError {
                                message: "Unexpected error".to_string(),
                            }) as axum::BoxError)
                        },
                    },
                                            Err(e) => {
                            log_http_error(e.as_ref(), &context_stream);
                            Err(e as axum::BoxError)
                        },
                })))
                .map_err(|e| {
                    error_limited!(code = %StatusCode::INTERNAL_SERVER_ERROR, context = resource_identity, error = %e);
                    ProblemDetails::from(StatusCode::INTERNAL_SERVER_ERROR)
                })?)
            },
            // Otherwise it's a redirect
            Some(read_response::ReplyType::Redirect(redirect)) => {
                let builder = AxumResponse::builder();
                Ok(add_headers_from_resource_info(
                    ResourceInfo {
                        metadata: Some(metadata),
                        resource_identity: Some(ResourceIdentity {
                            encoded_identity: response_identity.clone(),
                        }),
                    },
                    builder,
                )?
                .status(StatusCode::MULTIPLE_CHOICES)
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_string(&redirect).map_err(|e| {
                    error_limited!(code = %StatusCode::INTERNAL_SERVER_ERROR, context = resource_identity, error = %e);
                    ProblemDetails::from(StatusCode::INTERNAL_SERVER_ERROR)
                })?))
                .map_err(|e| {
                    error_limited!(code = %StatusCode::INTERNAL_SERVER_ERROR, context = resource_identity, error = %e);
                    ProblemDetails::from(StatusCode::INTERNAL_SERVER_ERROR)
                })?)
            },
            None => {
                error_limited!(code = %StatusCode::INTERNAL_SERVER_ERROR, context = resource_identity, "Storage Service stream ended unexpectedly");
                Err(ProblemDetails::from(StatusCode::INTERNAL_SERVER_ERROR))
            },
        },
        Some(Err(e)) => Err(ProblemDetails::from_error(e.as_ref())),
        None => {
            // If the object is empty, no chunks will be returned, but it's still a valid response
            if let Some(data_object_size) = metadata.data_object_size {
                if data_object_size == 0 {
                    let builder = AxumResponse::builder();
                    return add_headers_from_resource_info(
                        ResourceInfo {
                            metadata: Some(metadata),
                            resource_identity: Some(ResourceIdentity {
                                encoded_identity: response_identity.clone(),
                            }),
                        },
                        builder,
                    )?
                    .status(StatusCode::OK)
                    .body(Body::empty())
                    .map_err(|e| {
                        error_limited!(code = %StatusCode::INTERNAL_SERVER_ERROR, context = resource_identity, error = %e);
                        ProblemDetails::from(StatusCode::INTERNAL_SERVER_ERROR)
                    });
                }
            }

            error_limited!(code = %StatusCode::INTERNAL_SERVER_ERROR, context = resource_identity, "Storage Service stream ended unexpectedly");
            Err(ProblemDetails::from(StatusCode::INTERNAL_SERVER_ERROR))
        },
    }
}

#[utoipa::path(
    get,
    path = "/v1alpha/upload-options/by-address/{resource_address}",
    params(
        ("resource_address" = String, Path, description = "The address of the file object to fetch upload options"),
    ),
    responses(
        (status = 200, description = "Upload options fetched successfully", body = FetchWriteTypeInfoResponse),
    ),
    tag = "v1alpha",
)]
#[instrument(name = "rest::fetch_write_type_info", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
async fn fetch_write_type_info<S: FileObjectTrait>(
    headers: HeaderMap,
    State(state): State<FileObject<S>>,
    Path(resource_address): Path<String>,
) -> Result<AxumResponse, ProblemDetails> {
    let _slow = log_if_slow!(guard: "rest::fetch_write_type_info").with_context(resource_address.as_str());
    let request = FetchWriteTypeInfoRequest {
        destination_resource_address: resource_address.clone(),
    };
    let result = state
        .file_object
        .fetch_write_type_info(
            tonic::Request::from_parts(MetadataMap::from_headers(headers), Extensions::new(), request).into(),
        )
        .await
        .map_err(|e| log_http_error(e.as_ref(), &resource_address))?;

    let write_type_info = FetchWriteTypeInfoResponse::from(result);
    json_response(StatusCode::OK, &write_type_info)
}

#[utoipa::path(
    get,
    path = "/v1alpha/fileobject/data-objects/{resource_address}",
    params(
        ("resource_address" = String, Path, description = "The address of the file object to enumerate"),
        ("query" = EnumerateQueryParams, Query, description = "Query parameters for the enumerate request"),
    ),
    responses(
        (status = 200, description = "Data objects enumerated successfully", body = EnumerateResponse),
    ),
    tag = "v1alpha",
)]
#[instrument(name = "rest::enumerate", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
async fn enumerate<S: FileObjectTrait>(
    headers: HeaderMap,
    State(state): State<FileObject<S>>,
    Path(resource_address): Path<String>,
    Query(query): Query<EnumerateQueryParams>,
) -> Result<AxumResponse, ProblemDetails> {
    let _slow = log_if_slow!(guard: "rest::enumerate").with_context(resource_address.as_str());
    let request = EnumerateRequest {
        resource_address: resource_address.clone(),
    };
    let mut stream = state
        .file_object
        .enumerate(
            tonic::Request::from_parts(MetadataMap::from_headers(headers), Extensions::new(), request).into(),
            StreamingOptions::Paginated(Pagination {
                continuation_handle: query.continuation_handle,
                max_page_size: query.max_page_size,
            }),
        )
        .await
        .map_err(|e| log_http_error(e.as_ref(), &resource_address))?;

    let Some(response) = stream.next().await else {
        error_limited!(
            code = %StatusCode::INTERNAL_SERVER_ERROR,
            context = resource_address,
            "Failed to receive data from ListStat stream"
        );
        return Err(ProblemDetails::from(StatusCode::INTERNAL_SERVER_ERROR));
    };
    match response {
        Ok(response) => {
            let enum_response = EnumerateResponse {
                items: response.response.items,
                next_continuation_handle: response.continue_token,
            };
            json_response(StatusCode::OK, &enum_response)
        },
        Err(e) => Err(log_http_error(e.as_ref(), &resource_address)),
    }
}

#[utoipa::path(
    post,
    path = "/v1alpha/fileobject/by-identity/{resource_identity}/copy",
    responses(
        (status = 201, description = "Copy request successful", body = CopyResponse),
        (status = 400, description = "Invalid request: fail_if_exists combined with previous_version, or other validation error."),
        (status = 409, description = "Resource already exists at the destination (fail_if_exists=true)."),
        (status = 501, description = "The backend does not support fail_if_exists for copy."),
    ),
    tag = "v1alpha"
)]
#[instrument(name = "rest::copy", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
async fn copy<S: FileObjectTrait>(
    headers: HeaderMap,
    State(state): State<FileObject<S>>,
    resource_identity: Path<String>,
    extract::Json(payload): extract::Json<CopyRequestBody>,
) -> Result<AxumResponse, ProblemDetails> {
    let _slow = log_if_slow!(guard: "rest::copy").with_context(resource_identity.as_str());
    let request = CopyRequest {
        source_resource_identity: Some(ResourceIdentity {
            encoded_identity: resource_identity.to_string().clone(),
        }),
        destination_resource_address: payload.destination_resource_address.clone(),
        previous_version: payload
            .previous_version
            .map(|v| ResourceIdentity { encoded_identity: v }),
        fail_if_exists: payload.fail_if_exists,
    };
    let result = state
        .file_object
        .copy(tonic::Request::from_parts(MetadataMap::from_headers(headers), Extensions::new(), request).into())
        .await
        .map_err(|e| {
            log_http_error(
                e.as_ref(),
                &format!("{:?} -> {}", resource_identity, payload.destination_resource_address),
            )
        })?;

    json_response(StatusCode::CREATED, &result)
}

#[utoipa::path(
    post,
    path = "/v1alpha/fileobject/by-address/{resource_address}/move",
    responses(
        (status = 201, description = "Move request successful", body = MoveResponse),
        (status = 400, description = "Invalid request: fail_if_exists combined with destination_previous_version, or other validation error."),
        (status = 409, description = "Resource already exists at the destination (fail_if_exists=true)."),
        (status = 501, description = "The backend does not support fail_if_exists for move."),
    ),
    tag = "v1alpha"
)]
#[instrument(name = "rest::move", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
async fn move_object<S: FileObjectTrait>(
    headers: HeaderMap,
    State(state): State<FileObject<S>>,
    resource_address: Path<String>,
    extract::Json(payload): extract::Json<MoveRequestBody>,
) -> Result<AxumResponse, ProblemDetails> {
    let _slow = log_if_slow!(guard: "rest::move_object").with_context(resource_address.as_str());
    let request = MoveRequest {
        source_resource_address: resource_address.to_string().clone(),
        destination_resource_address: payload.destination_resource_address.clone(),
        destination_previous_version: payload
            .destination_previous_version
            .map(|v| ResourceIdentity { encoded_identity: v }),
        fail_if_exists: payload.fail_if_exists,
        source_previous_version: payload
            .source_previous_version
            .map(|v| ResourceIdentity { encoded_identity: v }),
    };
    let result = state
        .file_object
        .move_object(tonic::Request::from_parts(MetadataMap::from_headers(headers), Extensions::new(), request).into())
        .await
        .map_err(|e| {
            log_http_error(
                e.as_ref(),
                &format!("{:?} -> {}", resource_address, payload.destination_resource_address),
            )
        })?;

    json_response(StatusCode::CREATED, &result)
}

#[utoipa::path(
    get,
    path = "/v1alpha/fileobject/optimistic-locking-support/{resource_address}",
    responses((status = 200, description = "Optimistic locking support fetched successfully", body = GetOptimisticLockingSupportResponse)),
    tag = "v1alpha"
)]
#[instrument(name = "rest::get_optimistic_locking_support", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
async fn get_optimistic_locking_support<S: FileObjectTrait>(
    headers: HeaderMap,
    State(state): State<FileObject<S>>,
    Path(resource_address): Path<String>,
) -> Result<AxumResponse, ProblemDetails> {
    let _slow = log_if_slow!(guard: "rest::get_optimistic_locking_support").with_context(resource_address.as_str());
    let request = GetOptimisticLockingSupportRequest {
        resource_address: resource_address.clone(),
    };
    let result = state
        .file_object
        .get_optimistic_locking_support(
            tonic::Request::from_parts(MetadataMap::from_headers(headers), Extensions::new(), request).into(),
        )
        .await
        .map_err(|e| log_http_error(e.as_ref(), &resource_address))?;

    json_response(StatusCode::OK, &result)
}

#[utoipa::path(
    get,
    path = "/v1alpha/fileobject/native-metadata-support/{resource_address}",
    responses((status = 200, description = "Native metadata support fetched successfully", body = GetNativeMetadataSupportResponse)),
    tag = "v1alpha"
)]
#[instrument(name = "rest::get_native_metadata_support", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
async fn get_native_metadata_support<S: FileObjectTrait>(
    headers: HeaderMap,
    State(state): State<FileObject<S>>,
    Path(resource_address): Path<String>,
) -> Result<AxumResponse, ProblemDetails> {
    let _slow = log_if_slow!(guard: "rest::get_native_metadata_support").with_context(resource_address.as_str());
    let request = GetNativeMetadataSupportRequest {
        resource_address: resource_address.clone(),
    };
    let result = state
        .file_object
        .get_native_metadata_support(
            tonic::Request::from_parts(MetadataMap::from_headers(headers), Extensions::new(), request).into(),
        )
        .await
        .map_err(|e| log_http_error(e.as_ref(), &resource_address))?;

    json_response(StatusCode::OK, &result)
}

#[utoipa::path(
    get,
    path = "/v1alpha/fileobject/is-read-only/{resource_address}",
    params(
        ("resource_address" = String, Path, description = "The resource address to check for read-only behavior"),
    ),
    responses((status = 200, description = "Read-only state fetched successfully", body = IsReadOnlyResponse)),
    tag = "v1alpha"
)]
#[instrument(name = "rest::is_read_only", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
async fn is_read_only<S: FileObjectTrait>(
    headers: HeaderMap,
    State(state): State<FileObject<S>>,
    Path(resource_address): Path<String>,
) -> Result<AxumResponse, ProblemDetails> {
    let _slow = log_if_slow!(guard: "rest::is_read_only").with_context(resource_address.as_str());
    let request = IsReadOnlyRequest {
        resource_address: resource_address.clone(),
    };
    let result = state
        .file_object
        .is_read_only(tonic::Request::from_parts(MetadataMap::from_headers(headers), Extensions::new(), request).into())
        .await
        .map_err(|e| log_http_error(e.as_ref(), &resource_address))?;

    json_response(StatusCode::OK, &result)
}

#[derive(OpenApi)]
#[openapi(
    paths(
        stat_by_address,
        read_by_address,
        write_by_address,
        delete_by_address,
        abort_multipart_upload,
        complete_multipart_upload,
        prepare_multipart_upload,
        complete_redirect_upload,
        read_by_identity,
        fetch_write_type_info,
        enumerate,
        copy,
        move_object,
        get_optimistic_locking_support,
        get_native_metadata_support, is_read_only,
    ),
    components(schemas(
        ReadByAddressQueryParams,
        Redirect,
        WriteByAddressQueryParams,
        ResourceInfo,
        WriteResponse,
        DeleteByAddressQueryParams,
        AbortMultipartUploadRequestBody,
        CompleteMultipartUploadRequestBody,
        PrepareMultipartUploadRequestBody,
        UploadPartResponse,
        ReadByIdentityQueryParams,
        FetchWriteTypeInfoResponse,
        EnumerateQueryParams,
        EnumerateResponse,
        CopyRequestBody,
        MoveRequestBody,
        CopyResponse,
        MoveResponse,
        GetOptimisticLockingSupportResponse,
        GetNativeMetadataSupportResponse, IsReadOnlyResponse,
    ),),
    tags(
        (name = "v1alpha", description = "File Object API")
    )
)]
pub struct ApiDoc;
