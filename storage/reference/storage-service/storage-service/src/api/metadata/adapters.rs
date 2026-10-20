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

use futures::{future::try_join_all, TryStreamExt};
use snafu::{IntoError, Location, Snafu};
use std::collections::HashMap;
use storage_core::traits::{Request, Result, StorageErrorExt, StreamingOptions};

use crate::{backend::metadata::*, common::request::RequestExt as _};

/// The maximum number of items that clients can receive from a GetMetadata call
/// invoked with an empty key list.
const GET_METADATA_MAX_QUERY_ITEMS: usize = 100;

/// The maximum number of items in any GetMetadata call.
const GET_METADATA_MAX_ITEMS: usize = 100;

/// The maximum number of keys in a forwarded GetMetadataBatch call.
const GET_METADATA_BATCH_SIZE: usize = 100;

/// Emulate a GetMetadata call using the Metadata interface.
pub async fn invoke_get_metadata(
    service: &impl Metadata,
    request: Request<GetMetadataRequest>,
) -> Result<GetMetadataResponse> {
    use batch_error_entry::Code;
    use get_metadata_batch_response_entry::Outcome;

    let (metadata, extensions, mut request) = request.into_parts();

    let resource_location = create_resource_location(request.uri)?;
    if request.user_metadata_keys.is_empty() {
        let mut stream = service
            .query_metadata(
                Request::from_parts(
                    metadata,
                    extensions,
                    QueryMetadataRequest {
                        resource_location: Some(resource_location),
                        namespace: None,
                    },
                ),
                StreamingOptions::Streamed,
            )
            .await?;

        let mut results: HashMap<_, _> = Default::default();
        while let Some(page) = stream.try_next().await? {
            for item in page.response.results.into_iter() {
                if let Some(value) = item.user_metadata {
                    results.insert(item.user_metadata_key, value);
                }

                if results.len() > GET_METADATA_MAX_QUERY_ITEMS {
                    return Err(error::ResponseTooLarge.build().into());
                }
            }
        }

        return Ok(GetMetadataResponse { user_metadata: results });
    }

    if request.user_metadata_keys.len() > GET_METADATA_MAX_ITEMS {
        return Err(error::RequestBatchTooLarge.build().into());
    }

    let responses = try_join_all(request.user_metadata_keys.chunks(GET_METADATA_BATCH_SIZE).map(|chunk| {
        service.get_metadata_batch(Request::from_parts(
            metadata.clone(),
            extensions.clone(),
            GetMetadataBatchRequest {
                requests: chunk
                    .iter()
                    .map(|key| GetMetadataBatchRequestEntry {
                        resource_location: Some(resource_location.clone()),
                        user_metadata_key: key.clone(),
                        namespace: None,
                    })
                    .collect(),
            },
        ))
    }))
    .await?;

    let mut results: HashMap<_, _> = Default::default();
    for (chunk, response) in request
        .user_metadata_keys
        .chunks_mut(GET_METADATA_BATCH_SIZE)
        .zip(responses)
    {
        debug_assert_eq!(chunk.len(), response.results.len());

        for entry in response.results.into_iter() {
            debug_assert!((entry.request_index as usize) < chunk.len());
            debug_assert!(entry.outcome.is_some());

            let key = std::mem::take(&mut chunk[entry.request_index as usize]);
            let outcome = entry
                .outcome
                .ok_or_else(|| error::BackendResponseOutcomeMissing.build())?;
            match outcome {
                Outcome::Success(value) => {
                    results.insert(key, value);
                },
                Outcome::Error(error) => {
                    if error.code != Code::NotFound as i32 {
                        return Err(error::Batch {
                            message: error.message,
                            code: error.code,
                        }
                        .build()
                        .into());
                    }
                },
            }
        }
    }

    Ok(GetMetadataResponse { user_metadata: results })
}

/// Emulate an UpdateMetadata call using the Metadata interface.
pub async fn invoke_update_metadata(
    service: &impl Metadata,
    request: Request<UpdateMetadataRequest>,
) -> Result<UpdateMetadataResponse> {
    use update_metadata_batch_response_entry::Outcome;

    let (metadata, extensions, request) = request.into_parts();

    let resource_location = create_resource_location(request.uri)?;
    let response = service
        .update_metadata_batch(Request::from_parts(
            metadata,
            extensions,
            UpdateMetadataBatchRequest {
                requests: vec![UpdateMetadataBatchRequestEntry {
                    resource_location: Some(resource_location),
                    user_metadata_key: request.user_metadata_key,
                    user_metadata: request.user_metadata,
                    expected_etag: request.expected_etag,
                    namespace: None,
                }],
            },
        ))
        .await?;

    debug_assert_eq!(response.results.len(), 1);
    debug_assert_eq!(response.results[0].request_index, 0);

    let outcome = response
        .results
        .into_iter()
        .next()
        .ok_or_else(|| error::BackendResponseTooShort.build())?
        .outcome
        .ok_or_else(|| error::BackendResponseOutcomeMissing.build())?;
    match outcome {
        Outcome::Error(error) => Err(error::Batch {
            message: error.message,
            code: error.code,
        }
        .build()
        .into()),
        Outcome::Success(value) => Ok(UpdateMetadataResponse { etag: value.etag }),
    }
}

/// Emulate a DeleteMetadata call using the metadata interface.
pub async fn invoke_delete_metadata(
    service: &impl Metadata,
    request: Request<DeleteMetadataRequest>,
) -> Result<DeleteMetadataResponse> {
    use delete_metadata_batch_response_entry::Outcome;

    let (metadata, extensions, request) = request.into_parts();

    let resource_location = create_resource_location(request.uri)?;
    let response = service
        .delete_metadata_batch(Request::from_parts(
            metadata,
            extensions,
            DeleteMetadataBatchRequest {
                requests: vec![DeleteMetadataBatchRequestEntry {
                    resource_location: Some(resource_location),
                    user_metadata_key: request.user_metadata_key,
                    expected_etag: request.expected_etag,
                    namespace: None,
                }],
            },
        ))
        .await?;

    debug_assert_eq!(response.results.len(), 1);
    debug_assert_eq!(response.results[0].request_index, 0);

    let outcome = response
        .results
        .into_iter()
        .next()
        .ok_or_else(|| error::BackendResponseTooShort.build())?
        .outcome
        .ok_or_else(|| error::BackendResponseOutcomeMissing.build())?;
    if let Outcome::Error(error) = outcome {
        return Err(error::Batch {
            message: error.message,
            code: error.code,
        }
        .build()
        .into());
    }

    Ok(DeleteMetadataResponse {})
}

#[derive(Snafu, Debug)]
#[snafu(module, visibility(pub), context(suffix(false)))]
enum Error {
    #[snafu(display("resource uri is invalid"))]
    ResourceUriInvalid {
        source: fluent_uri::ParseError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("response is too large"))]
    ResponseTooLarge {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("backend returned less responses than expected"))]
    BackendResponseTooShort {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("response entry outcome is missing"))]
    BackendResponseOutcomeMissing {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("request batch is too large"))]
    RequestBatchTooLarge {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("{message}"))]
    Batch {
        message: String,
        code: i32,
        #[snafu(provide, implicit)]
        location: Location,
    },
}

impl StorageErrorExt for Error {
    fn get_size(&self) -> usize {
        0
    }

    fn http_status(&self) -> axum::http::StatusCode {
        use axum::http::StatusCode;
        use batch_error_entry::Code;

        match self {
            Self::ResourceUriInvalid { .. } | Self::RequestBatchTooLarge { .. } => StatusCode::BAD_REQUEST,
            Self::BackendResponseTooShort { .. } | Self::BackendResponseOutcomeMissing { .. } => {
                StatusCode::INTERNAL_SERVER_ERROR
            },
            Self::ResponseTooLarge { .. } => StatusCode::PAYLOAD_TOO_LARGE,
            Self::Batch { code, .. } => match *code {
                c if c == Code::InvalidArgument as i32 => StatusCode::BAD_REQUEST,
                c if c == Code::NotFound as i32 => StatusCode::NOT_FOUND,
                c if c == Code::PermissionDenied as i32 => StatusCode::FORBIDDEN,
                c if c == Code::FailedPrecondition as i32 => StatusCode::PRECONDITION_FAILED,
                c if c == Code::Internal as i32 => StatusCode::INTERNAL_SERVER_ERROR,
                c if c == Code::ResourceExhausted as i32 => StatusCode::TOO_MANY_REQUESTS,
                c if c == Code::Unavailable as i32 => StatusCode::SERVICE_UNAVAILABLE,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            },
        }
    }

    fn tonic_status(&self) -> tonic::Status {
        use batch_error_entry::Code;
        use tonic::Status;

        match self {
            Self::ResourceUriInvalid { .. } | Self::RequestBatchTooLarge { .. } => {
                Status::invalid_argument(self.to_string())
            },
            Self::BackendResponseTooShort { .. } | Self::BackendResponseOutcomeMissing { .. } => {
                Status::internal("unexpected error")
            },
            Self::ResponseTooLarge { .. } => Status::invalid_argument(self.to_string()),
            Self::Batch { code, .. } => match *code {
                c if c == Code::InvalidArgument as i32 => Status::invalid_argument(self.to_string()),
                c if c == Code::NotFound as i32 => Status::not_found(self.to_string()),
                c if c == Code::PermissionDenied as i32 => Status::permission_denied(self.to_string()),
                c if c == Code::FailedPrecondition as i32 => Status::failed_precondition(self.to_string()),
                c if c == Code::Internal as i32 => Status::internal(self.to_string()),
                c if c == Code::ResourceExhausted as i32 => Status::resource_exhausted(self.to_string()),
                c if c == Code::Unavailable as i32 => Status::unavailable(self.to_string()),
                _ => Status::internal(self.to_string()),
            },
        }
    }
}

fn create_resource_location(uri: String) -> Result<ResourceLocation, Error> {
    use fluent_uri::Uri;

    let parsed = Uri::parse(uri.as_str()).map_err(|source| error::ResourceUriInvalid.into_error(source))?;

    let path = parsed.path().as_str();
    if path.is_empty() {
        return Ok(create_folder_address(uri));
    }

    if let Some(query) = parsed.query() {
        for param in query.split('&') {
            if param.as_str().starts_with("etag=") {
                return Ok(create_resource_identity(uri));
            }
        }
    }

    if path.ends_with('/') || path.ends_with("%2F") || path.ends_with("%2f") {
        return Ok(create_folder_address(uri));
    }

    Ok(create_resource_address(uri))
}

fn create_resource_address(uri: String) -> ResourceLocation {
    ResourceLocation {
        resource: Some(resource_location::Resource::ResourceAddress(uri)),
    }
}

fn create_resource_identity(uri: String) -> ResourceLocation {
    use storage_service_protos::nvidia::omniverse::storage::fileobject::v1alpha::ResourceIdentity;

    ResourceLocation {
        resource: Some(resource_location::Resource::ResourceIdentity(ResourceIdentity {
            encoded_identity: uri,
        })),
    }
}

fn create_folder_address(uri: String) -> ResourceLocation {
    use storage_service_protos::nvidia::omniverse::storage::filefolder::v1alpha::FolderAddress;

    ResourceLocation {
        resource: Some(resource_location::Resource::FolderAddress(FolderAddress { uri })),
    }
}
