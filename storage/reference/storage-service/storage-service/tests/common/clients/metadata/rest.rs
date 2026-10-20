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
    use crate::common::clients::metadata::types::v1alpha::*;
    use crate::common::clients::utils::{
        get_headers,
        rest::{encode_path_segment, RequestBuilderExt, ResponseExt},
    };
    use async_stream::{stream, try_stream};
    use futures::stream::BoxStream;
    use reqwest::StatusCode;
    use tonic::Status;
    use tracing::debug;

    pub mod models {
        use std::collections::HashMap;

        use serde::{Deserialize, Serialize};
        use storage_service_protos::{google::protobuf::Value, nvidia::omniverse::storage::metadata::v1alpha};

        pub type GetMetadataRequest = Vec<String>;

        pub type GetMetadataResponse = HashMap<String, MetadataValue>;

        pub type UpdateMetadataRequest = Value;

        pub type GetMetadataBatchRequest = BatchRequest<GetMetadataBatchRequestEntry>;

        pub type GetMetadataBatchResponse = BatchResponse<MetadataValue>;

        pub type UpdateMetadataBatchRequest = BatchRequest<UpdateMetadataBatchRequestEntry>;

        pub type UpdateMetadataBatchResponse = BatchResponse<UpdateMetadataBatchResponseEntrySuccess>;

        pub type DeleteMetadataBatchRequest = BatchRequest<DeleteMetadataBatchRequestEntry>;

        pub type DeleteMetadataBatchResponse = BatchResponse<DeleteMetadataBatchResponseEntrySuccess>;

        impl From<v1alpha::GetMetadataBatchRequest> for GetMetadataBatchRequest {
            fn from(value: v1alpha::GetMetadataBatchRequest) -> Self {
                Self {
                    requests: value.requests.into_iter().map(Into::into).collect(),
                }
            }
        }

        impl From<GetMetadataBatchResponse> for v1alpha::GetMetadataBatchResponse {
            fn from(value: GetMetadataBatchResponse) -> Self {
                Self {
                    results: value
                        .results
                        .into_iter()
                        .map(|entry| v1alpha::GetMetadataBatchResponseEntry {
                            request_index: entry.request_index,
                            outcome: Some(match entry.outcome {
                                Outcome::Success(value) => {
                                    v1alpha::get_metadata_batch_response_entry::Outcome::Success(value.into())
                                },
                                Outcome::Error(value) => {
                                    v1alpha::get_metadata_batch_response_entry::Outcome::Error(value.into())
                                },
                            }),
                        })
                        .collect(),
                }
            }
        }

        impl From<v1alpha::UpdateMetadataBatchRequest> for UpdateMetadataBatchRequest {
            fn from(value: v1alpha::UpdateMetadataBatchRequest) -> Self {
                Self {
                    requests: value.requests.into_iter().map(Into::into).collect(),
                }
            }
        }

        impl From<UpdateMetadataBatchResponse> for v1alpha::UpdateMetadataBatchResponse {
            fn from(value: UpdateMetadataBatchResponse) -> Self {
                Self {
                    results: value
                        .results
                        .into_iter()
                        .map(|entry| v1alpha::UpdateMetadataBatchResponseEntry {
                            request_index: entry.request_index,
                            outcome: Some(match entry.outcome {
                                Outcome::Success(value) => {
                                    v1alpha::update_metadata_batch_response_entry::Outcome::Success(value.into())
                                },
                                Outcome::Error(value) => {
                                    v1alpha::update_metadata_batch_response_entry::Outcome::Error(value.into())
                                },
                            }),
                        })
                        .collect(),
                }
            }
        }

        impl From<v1alpha::DeleteMetadataBatchRequest> for DeleteMetadataBatchRequest {
            fn from(value: v1alpha::DeleteMetadataBatchRequest) -> Self {
                Self {
                    requests: value.requests.into_iter().map(Into::into).collect(),
                }
            }
        }

        impl From<DeleteMetadataBatchResponse> for v1alpha::DeleteMetadataBatchResponse {
            fn from(value: DeleteMetadataBatchResponse) -> Self {
                Self {
                    results: value
                        .results
                        .into_iter()
                        .map(|entry| v1alpha::DeleteMetadataBatchResponseEntry {
                            request_index: entry.request_index,
                            outcome: Some(match entry.outcome {
                                Outcome::Success(value) => {
                                    v1alpha::delete_metadata_batch_response_entry::Outcome::Success(value.into())
                                },
                                Outcome::Error(value) => {
                                    v1alpha::delete_metadata_batch_response_entry::Outcome::Error(value.into())
                                },
                            }),
                        })
                        .collect(),
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct MetadataValue {
            pub etag: String,
            pub value: Value,
        }

        impl From<MetadataValue> for v1alpha::UserMetadataValue {
            fn from(value: MetadataValue) -> Self {
                Self {
                    etag: value.etag,
                    value: Some(value.value),
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub enum Outcome<T> {
            Error(BatchErrorEntry),
            Success(T),
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub enum ResourceLocation {
            #[serde(rename = "resource_address")]
            ResourceAddress(String),
            #[serde(rename = "resource_identity")]
            ResourceIdentity(String),
            #[serde(rename = "folder_address")]
            FolderAddress(String),
        }

        impl From<v1alpha::ResourceLocation> for ResourceLocation {
            fn from(value: v1alpha::ResourceLocation) -> Self {
                use v1alpha::resource_location::Resource;

                match value.resource.unwrap() {
                    Resource::ResourceAddress(value) => Self::ResourceAddress(value),
                    Resource::ResourceIdentity(value) => Self::ResourceIdentity(value.encoded_identity),
                    Resource::FolderAddress(value) => Self::FolderAddress(value.uri),
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct BatchResponseEntry<T> {
            pub request_index: u32,
            pub outcome: Outcome<T>,
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub enum BatchErrorEntryCode {
            #[serde(rename = "invalid_argument")]
            InvalidArgument,
            #[serde(rename = "not_found")]
            NotFound,
            #[serde(rename = "permission_denied")]
            PermissionDenied,
            #[serde(rename = "failed_precondition")]
            FailedPrecondition,
            #[serde(rename = "internal")]
            Internal,
            #[serde(rename = "resource_exhausted")]
            ResourceExhausted,
            #[serde(rename = "unavailable")]
            Unavailable,
        }

        impl From<BatchErrorEntryCode> for v1alpha::batch_error_entry::Code {
            fn from(value: BatchErrorEntryCode) -> Self {
                match value {
                    BatchErrorEntryCode::InvalidArgument => Self::InvalidArgument,
                    BatchErrorEntryCode::NotFound => Self::NotFound,
                    BatchErrorEntryCode::PermissionDenied => Self::PermissionDenied,
                    BatchErrorEntryCode::FailedPrecondition => Self::FailedPrecondition,
                    BatchErrorEntryCode::Internal => Self::Internal,
                    BatchErrorEntryCode::ResourceExhausted => Self::ResourceExhausted,
                    BatchErrorEntryCode::Unavailable => Self::Unavailable,
                }
            }
        }

        impl From<BatchErrorEntryCode> for i32 {
            fn from(value: BatchErrorEntryCode) -> Self {
                v1alpha::batch_error_entry::Code::from(value) as i32
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct BatchErrorEntry {
            pub code: BatchErrorEntryCode,
            pub message: String,
        }

        impl From<BatchErrorEntry> for v1alpha::BatchErrorEntry {
            fn from(value: BatchErrorEntry) -> Self {
                Self {
                    code: value.code.into(),
                    message: value.message,
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct BatchRequest<T> {
            pub requests: Vec<T>,
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct BatchResponse<T> {
            pub results: Vec<BatchResponseEntry<T>>,
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct GetMetadataBatchRequestEntry {
            pub resource_location: ResourceLocation,
            pub user_metadata_key: String,
            pub namespace: Option<String>,
        }

        impl From<v1alpha::GetMetadataBatchRequestEntry> for GetMetadataBatchRequestEntry {
            fn from(value: v1alpha::GetMetadataBatchRequestEntry) -> Self {
                Self {
                    resource_location: value.resource_location.unwrap().into(),
                    user_metadata_key: value.user_metadata_key,
                    namespace: value.namespace,
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct UpdateMetadataBatchRequestEntry {
            pub resource_location: ResourceLocation,
            pub user_metadata_key: String,
            pub user_metadata: Value,
            pub expected_etag: Option<String>,
            pub namespace: Option<String>,
        }

        impl From<v1alpha::UpdateMetadataBatchRequestEntry> for UpdateMetadataBatchRequestEntry {
            fn from(value: v1alpha::UpdateMetadataBatchRequestEntry) -> Self {
                Self {
                    resource_location: value.resource_location.unwrap().into(),
                    user_metadata_key: value.user_metadata_key,
                    user_metadata: value.user_metadata.unwrap(),
                    expected_etag: value.expected_etag,
                    namespace: value.namespace,
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct UpdateMetadataBatchResponseEntrySuccess {
            pub etag: String,
        }

        impl From<UpdateMetadataBatchResponseEntrySuccess> for v1alpha::UpdateMetadataBatchResponseEntrySuccess {
            fn from(value: UpdateMetadataBatchResponseEntrySuccess) -> Self {
                Self { etag: value.etag }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct DeleteMetadataBatchRequestEntry {
            pub resource_location: ResourceLocation,
            pub user_metadata_key: String,
            pub expected_etag: Option<String>,
            pub namespace: Option<String>,
        }

        impl From<v1alpha::DeleteMetadataBatchRequestEntry> for DeleteMetadataBatchRequestEntry {
            fn from(value: v1alpha::DeleteMetadataBatchRequestEntry) -> Self {
                Self {
                    resource_location: value.resource_location.unwrap().into(),
                    user_metadata_key: value.user_metadata_key,
                    expected_etag: value.expected_etag,
                    namespace: value.namespace,
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct DeleteMetadataBatchResponseEntrySuccess {}

        impl From<DeleteMetadataBatchResponseEntrySuccess> for v1alpha::DeleteMetadataBatchResponseEntrySuccess {
            fn from(_value: DeleteMetadataBatchResponseEntrySuccess) -> Self {
                Self {}
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct QueryMetadataRequest {
            pub resource_location: ResourceLocation,
            pub namespace: Option<String>,
            pub continuation_handle: Option<String>,
            pub max_page_size: Option<usize>,
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct QueryMetadataResponse {
            pub results: Vec<QueryMetadataResponseEntry>,
            pub next_continuation_handle: Option<String>,
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct QueryMetadataResponseEntry {
            pub user_metadata_key: String,
            pub user_metadata: MetadataValue,
        }

        impl From<QueryMetadataResponseEntry> for v1alpha::QueryMetadataResponseEntry {
            fn from(value: QueryMetadataResponseEntry) -> Self {
                Self {
                    user_metadata_key: value.user_metadata_key,
                    user_metadata: Some(value.user_metadata.into()),
                }
            }
        }
    }

    #[derive(Debug, Clone)]
    pub struct Client {
        client: reqwest::Client,
        base_url: String,
    }

    impl Client {
        pub fn new(client: reqwest::Client, base_url: impl Into<String>) -> Self {
            Self {
                client,
                base_url: base_url.into(),
            }
        }

        pub async fn get_metadata(&self, request: GetMetadataRequest) -> Result<GetMetadataResponse, Status> {
            let request_body: models::GetMetadataRequest = request.user_metadata_keys;
            let response = self
                .client
                .post(format!(
                    "{}/v1alpha/metadata/{}",
                    self.base_url,
                    encode_path_segment(&request.uri)
                ))
                .headers(get_headers())
                .json(&request_body)
                .send_with_retry()
                .await
                .expect("get metadata request failed");

            debug!(?response);

            if response.status() != StatusCode::OK {
                return Err(response.to_tonic_status());
            }

            let response_body: models::GetMetadataResponse =
                response.json().await.expect("failed to read response body");

            Ok(GetMetadataResponse {
                user_metadata: response_body
                    .into_iter()
                    .map(|(key, value)| {
                        (
                            key,
                            UserMetadataValue {
                                value: Some(value.value),
                                etag: value.etag,
                            },
                        )
                    })
                    .collect(),
            })
        }

        pub async fn update_metadata(&self, request: UpdateMetadataRequest) -> Result<UpdateMetadataResponse, Status> {
            let request_body: models::UpdateMetadataRequest =
                request.user_metadata.expect("user metadata value is missing");
            let response = self
                .client
                .put(format!(
                    "{}/v1alpha/metadata/{}/{}",
                    self.base_url,
                    encode_path_segment(&request.uri),
                    encode_path_segment(&request.user_metadata_key)
                ))
                .headers(get_headers())
                .maybe_if_match_header(request.expected_etag.as_deref())
                .json(&request_body)
                .send_with_retry()
                .await
                .expect("update metadata request failed");

            debug!(?response);

            if response.status() != StatusCode::CREATED {
                return Err(response.to_tonic_status());
            }

            Ok(UpdateMetadataResponse {
                etag: response.etag_header(),
            })
        }

        pub async fn delete_metadata(&self, request: DeleteMetadataRequest) -> Result<DeleteMetadataResponse, Status> {
            let response = self
                .client
                .delete(format!(
                    "{}/v1alpha/metadata/{}/{}",
                    self.base_url,
                    encode_path_segment(&request.uri),
                    encode_path_segment(&request.user_metadata_key)
                ))
                .headers(get_headers())
                .maybe_if_match_header(request.expected_etag.as_deref())
                .send_with_retry()
                .await
                .expect("delete metadata request failed");

            debug!(?response);

            if response.status() != StatusCode::NO_CONTENT {
                return Err(response.to_tonic_status());
            }

            Ok(DeleteMetadataResponse {})
        }

        pub fn get_metadata_batch(
            &self,
            request: GetMetadataBatchRequest,
        ) -> BoxStream<'static, Result<GetMetadataBatchResponse, Status>> {
            let request = self
                .client
                .post(format!("{}/v1alpha/metadata/get", self.base_url))
                .headers(get_headers())
                .json(&models::GetMetadataBatchRequest::from(request));
            Box::pin(stream! {
                let response = request
                    .send_with_retry()
                    .await
                    .expect("get metadata batch request failed");
                debug!(?response);

                if response.status() != StatusCode::OK {
                    yield Err(response.to_tonic_status());
                    return;
                }

                let response: models::GetMetadataBatchResponse =
                    response.json().await.expect("failed to read response body");

                yield Ok(response.into());
            })
        }

        pub fn query_metadata(
            &self,
            request: QueryMetadataRequest,
            max_page_size: Option<usize>,
        ) -> BoxStream<'static, Result<QueryMetadataResponse, Status>> {
            let client = self.clone();
            Box::pin(try_stream! {
                let mut continuation_handle = None;
                loop {
                    let (response, next_continuation_handle) = client.query_metadata_page(
                        request.clone(),
                        std::mem::take(&mut continuation_handle),
                        max_page_size,
                    ).await?;
                    yield response;

                    if next_continuation_handle.is_none() {
                        return;
                    }

                    continuation_handle = next_continuation_handle;
                }
            })
        }

        pub fn update_metadata_batch(
            &self,
            request: UpdateMetadataBatchRequest,
        ) -> BoxStream<'static, Result<UpdateMetadataBatchResponse, Status>> {
            let request = self
                .client
                .post(format!("{}/v1alpha/metadata", self.base_url))
                .headers(get_headers())
                .json(&models::UpdateMetadataBatchRequest::from(request));
            Box::pin(stream! {
                let response = request
                    .send_with_retry()
                    .await
                    .expect("update metadata batch request failed");
                debug!(?response);

                if response.status() != StatusCode::OK {
                    yield Err(response.to_tonic_status());
                    return;
                }

                let response: models::UpdateMetadataBatchResponse =
                    response.json().await.expect("failed to read response body");

                yield Ok(response.into());
            })
        }

        pub fn delete_metadata_batch(
            &self,
            request: DeleteMetadataBatchRequest,
        ) -> BoxStream<'static, Result<DeleteMetadataBatchResponse, Status>> {
            let request = self
                .client
                .delete(format!("{}/v1alpha/metadata", self.base_url))
                .headers(get_headers())
                .json(&models::DeleteMetadataBatchRequest::from(request));
            Box::pin(stream! {
                let response = request
                    .send_with_retry()
                    .await
                    .expect("delete metadata batch request failed");
                debug!(?response);

                if response.status() != StatusCode::OK {
                    yield Err(response.to_tonic_status());
                    return;
                }

                let response: models::DeleteMetadataBatchResponse =
                    response.json().await.expect("failed to read response body");

                yield Ok(response.into());
            })
        }

        async fn query_metadata_page(
            &self,
            request: QueryMetadataRequest,
            continuation_handle: Option<String>,
            max_page_size: Option<usize>,
        ) -> Result<(QueryMetadataResponse, Option<String>), Status> {
            let response = self
                .client
                .post(format!("{}/v1alpha/metadata/query", self.base_url))
                .headers(get_headers())
                .json(&models::QueryMetadataRequest {
                    namespace: request.namespace,
                    resource_location: request.resource_location.unwrap().into(),
                    continuation_handle,
                    max_page_size,
                })
                .send_with_retry()
                .await
                .expect("query metadata request failed");

            if response.status() != StatusCode::OK {
                return Err(response.to_tonic_status());
            }

            let response: models::QueryMetadataResponse = response.json().await.expect("failed to read response body");

            Ok((
                QueryMetadataResponse {
                    results: response
                        .results
                        .into_iter()
                        .map(|entry| QueryMetadataResponseEntry {
                            user_metadata_key: entry.user_metadata_key,
                            user_metadata: Some(entry.user_metadata.into()),
                        })
                        .collect(),
                },
                response.next_continuation_handle,
            ))
        }
    }
}

pub mod v1beta {
    use crate::common::clients::metadata::types::v1beta::*;
    use crate::common::clients::utils::{
        get_headers,
        rest::{RequestBuilderExt, ResponseExt},
    };
    use async_stream::{stream, try_stream};
    use futures::stream::BoxStream;
    use reqwest::StatusCode;
    use tonic::Status;
    use tracing::debug;

    // Hand-written REST wire models. The tests deliberately assert against these instead of the
    // generated `nvidia.omniverse.metadata.v1beta` types so the JSON wire format is pinned
    // independently of the protocol codegen.
    pub mod models {
        use serde::{Deserialize, Serialize};
        use storage_service_protos::{google::protobuf::Value, nvidia::omniverse::metadata::v1beta};

        pub type GetMetadataBatchRequest = BatchRequest<GetMetadataBatchRequestEntry>;

        pub type GetMetadataBatchResponse = BatchResponse<MetadataValue>;

        pub type UpdateMetadataBatchRequest = BatchRequest<UpdateMetadataBatchRequestEntry>;

        pub type UpdateMetadataBatchResponse = BatchResponse<UpdateMetadataBatchResponseEntrySuccess>;

        pub type DeleteMetadataBatchRequest = BatchRequest<DeleteMetadataBatchRequestEntry>;

        pub type DeleteMetadataBatchResponse = BatchResponse<DeleteMetadataBatchResponseEntrySuccess>;

        impl From<v1beta::GetMetadataBatchRequest> for GetMetadataBatchRequest {
            fn from(value: v1beta::GetMetadataBatchRequest) -> Self {
                Self {
                    requests: value.requests.into_iter().map(Into::into).collect(),
                }
            }
        }

        impl From<GetMetadataBatchResponse> for v1beta::GetMetadataBatchResponse {
            fn from(value: GetMetadataBatchResponse) -> Self {
                Self {
                    results: value
                        .results
                        .into_iter()
                        .map(|entry| v1beta::GetMetadataBatchResponseEntry {
                            request_index: entry.request_index,
                            outcome: Some(match entry.outcome {
                                Outcome::Success(value) => {
                                    v1beta::get_metadata_batch_response_entry::Outcome::Success(value.into())
                                },
                                Outcome::Error(value) => {
                                    v1beta::get_metadata_batch_response_entry::Outcome::Error(value.into())
                                },
                            }),
                        })
                        .collect(),
                }
            }
        }

        impl From<v1beta::UpdateMetadataBatchRequest> for UpdateMetadataBatchRequest {
            fn from(value: v1beta::UpdateMetadataBatchRequest) -> Self {
                Self {
                    requests: value.requests.into_iter().map(Into::into).collect(),
                }
            }
        }

        impl From<UpdateMetadataBatchResponse> for v1beta::UpdateMetadataBatchResponse {
            fn from(value: UpdateMetadataBatchResponse) -> Self {
                Self {
                    results: value
                        .results
                        .into_iter()
                        .map(|entry| v1beta::UpdateMetadataBatchResponseEntry {
                            request_index: entry.request_index,
                            outcome: Some(match entry.outcome {
                                Outcome::Success(value) => {
                                    v1beta::update_metadata_batch_response_entry::Outcome::Success(value.into())
                                },
                                Outcome::Error(value) => {
                                    v1beta::update_metadata_batch_response_entry::Outcome::Error(value.into())
                                },
                            }),
                        })
                        .collect(),
                }
            }
        }

        impl From<v1beta::DeleteMetadataBatchRequest> for DeleteMetadataBatchRequest {
            fn from(value: v1beta::DeleteMetadataBatchRequest) -> Self {
                Self {
                    requests: value.requests.into_iter().map(Into::into).collect(),
                }
            }
        }

        impl From<DeleteMetadataBatchResponse> for v1beta::DeleteMetadataBatchResponse {
            fn from(value: DeleteMetadataBatchResponse) -> Self {
                Self {
                    results: value
                        .results
                        .into_iter()
                        .map(|entry| v1beta::DeleteMetadataBatchResponseEntry {
                            request_index: entry.request_index,
                            outcome: Some(match entry.outcome {
                                Outcome::Success(value) => {
                                    v1beta::delete_metadata_batch_response_entry::Outcome::Success(value.into())
                                },
                                Outcome::Error(value) => {
                                    v1beta::delete_metadata_batch_response_entry::Outcome::Error(value.into())
                                },
                            }),
                        })
                        .collect(),
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct MetadataValue {
            pub etag: String,
            pub value: Value,
        }

        impl From<MetadataValue> for v1beta::UserMetadataValue {
            fn from(value: MetadataValue) -> Self {
                Self {
                    etag: value.etag,
                    value: Some(value.value),
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub enum Outcome<T> {
            Error(BatchErrorEntry),
            Success(T),
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub enum ResourceLocation {
            #[serde(rename = "resource_address")]
            ResourceAddress(String),
            #[serde(rename = "resource_identity")]
            ResourceIdentity(String),
            #[serde(rename = "folder_address")]
            FolderAddress(String),
        }

        impl From<v1beta::ResourceLocation> for ResourceLocation {
            fn from(value: v1beta::ResourceLocation) -> Self {
                use v1beta::resource_location::Resource;

                match value.resource.unwrap() {
                    Resource::ResourceAddress(value) => Self::ResourceAddress(value),
                    Resource::ResourceIdentity(value) => Self::ResourceIdentity(value.encoded_identity),
                    Resource::FolderAddress(value) => Self::FolderAddress(value.uri),
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct BatchResponseEntry<T> {
            pub request_index: u32,
            pub outcome: Outcome<T>,
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub enum BatchErrorEntryCode {
            #[serde(rename = "invalid_argument")]
            InvalidArgument,
            #[serde(rename = "not_found")]
            NotFound,
            #[serde(rename = "permission_denied")]
            PermissionDenied,
            #[serde(rename = "failed_precondition")]
            FailedPrecondition,
            #[serde(rename = "internal")]
            Internal,
            #[serde(rename = "resource_exhausted")]
            ResourceExhausted,
            #[serde(rename = "unavailable")]
            Unavailable,
        }

        impl From<BatchErrorEntryCode> for v1beta::batch_error_entry::Code {
            fn from(value: BatchErrorEntryCode) -> Self {
                match value {
                    BatchErrorEntryCode::InvalidArgument => Self::InvalidArgument,
                    BatchErrorEntryCode::NotFound => Self::NotFound,
                    BatchErrorEntryCode::PermissionDenied => Self::PermissionDenied,
                    BatchErrorEntryCode::FailedPrecondition => Self::FailedPrecondition,
                    BatchErrorEntryCode::Internal => Self::Internal,
                    BatchErrorEntryCode::ResourceExhausted => Self::ResourceExhausted,
                    BatchErrorEntryCode::Unavailable => Self::Unavailable,
                }
            }
        }

        impl From<BatchErrorEntryCode> for i32 {
            fn from(value: BatchErrorEntryCode) -> Self {
                v1beta::batch_error_entry::Code::from(value) as i32
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct BatchErrorEntry {
            pub code: BatchErrorEntryCode,
            pub message: String,
        }

        impl From<BatchErrorEntry> for v1beta::BatchErrorEntry {
            fn from(value: BatchErrorEntry) -> Self {
                Self {
                    code: value.code.into(),
                    message: value.message,
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct BatchRequest<T> {
            pub requests: Vec<T>,
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct BatchResponse<T> {
            pub results: Vec<BatchResponseEntry<T>>,
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct GetMetadataBatchRequestEntry {
            pub resource_location: ResourceLocation,
            pub user_metadata_key: String,
            pub namespace: Option<String>,
        }

        impl From<v1beta::GetMetadataBatchRequestEntry> for GetMetadataBatchRequestEntry {
            fn from(value: v1beta::GetMetadataBatchRequestEntry) -> Self {
                Self {
                    resource_location: value.resource_location.unwrap().into(),
                    user_metadata_key: value.user_metadata_key,
                    namespace: value.namespace,
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct UpdateMetadataBatchRequestEntry {
            pub resource_location: ResourceLocation,
            pub user_metadata_key: String,
            pub user_metadata: Value,
            pub expected_etag: Option<String>,
            pub namespace: Option<String>,
        }

        impl From<v1beta::UpdateMetadataBatchRequestEntry> for UpdateMetadataBatchRequestEntry {
            fn from(value: v1beta::UpdateMetadataBatchRequestEntry) -> Self {
                Self {
                    resource_location: value.resource_location.unwrap().into(),
                    user_metadata_key: value.user_metadata_key,
                    user_metadata: value.user_metadata.unwrap(),
                    expected_etag: value.expected_etag,
                    namespace: value.namespace,
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct UpdateMetadataBatchResponseEntrySuccess {
            pub etag: String,
        }

        impl From<UpdateMetadataBatchResponseEntrySuccess> for v1beta::UpdateMetadataBatchResponseEntrySuccess {
            fn from(value: UpdateMetadataBatchResponseEntrySuccess) -> Self {
                Self { etag: value.etag }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct DeleteMetadataBatchRequestEntry {
            pub resource_location: ResourceLocation,
            pub user_metadata_key: String,
            pub expected_etag: Option<String>,
            pub namespace: Option<String>,
        }

        impl From<v1beta::DeleteMetadataBatchRequestEntry> for DeleteMetadataBatchRequestEntry {
            fn from(value: v1beta::DeleteMetadataBatchRequestEntry) -> Self {
                Self {
                    resource_location: value.resource_location.unwrap().into(),
                    user_metadata_key: value.user_metadata_key,
                    expected_etag: value.expected_etag,
                    namespace: value.namespace,
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct DeleteMetadataBatchResponseEntrySuccess {}

        impl From<DeleteMetadataBatchResponseEntrySuccess> for v1beta::DeleteMetadataBatchResponseEntrySuccess {
            fn from(_value: DeleteMetadataBatchResponseEntrySuccess) -> Self {
                Self {}
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct QueryMetadataRequest {
            pub resource_location: ResourceLocation,
            pub namespace: Option<String>,
            pub continuation_handle: Option<String>,
            pub max_page_size: Option<usize>,
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct QueryMetadataResponse {
            pub results: Vec<QueryMetadataResponseEntry>,
            pub next_continuation_handle: Option<String>,
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct QueryMetadataResponseEntry {
            pub user_metadata_key: String,
            pub user_metadata: MetadataValue,
        }

        impl From<QueryMetadataResponseEntry> for v1beta::QueryMetadataResponseEntry {
            fn from(value: QueryMetadataResponseEntry) -> Self {
                Self {
                    user_metadata_key: value.user_metadata_key,
                    user_metadata: Some(value.user_metadata.into()),
                }
            }
        }
    }

    #[derive(Debug, Clone)]
    pub struct Client {
        client: reqwest::Client,
        base_url: String,
    }

    impl Client {
        pub fn new(client: reqwest::Client, base_url: impl Into<String>) -> Self {
            Self {
                client,
                base_url: base_url.into(),
            }
        }

        pub fn get_metadata_batch(
            &self,
            request: GetMetadataBatchRequest,
        ) -> BoxStream<'static, Result<GetMetadataBatchResponse, Status>> {
            let request = self
                .client
                .post(format!("{}/v1beta/metadata", self.base_url))
                .headers(get_headers())
                .json(&models::GetMetadataBatchRequest::from(request));
            Box::pin(stream! {
                let response = request
                    .send_with_retry()
                    .await
                    .expect("get metadata batch request failed");
                debug!(?response);

                if response.status() != StatusCode::OK {
                    yield Err(response.to_tonic_status());
                    return;
                }

                let response: models::GetMetadataBatchResponse =
                    response.json().await.expect("failed to read response body");

                yield Ok(response.into());
            })
        }

        pub fn query_metadata(
            &self,
            request: QueryMetadataRequest,
            max_page_size: Option<usize>,
        ) -> BoxStream<'static, Result<QueryMetadataResponse, Status>> {
            let client = self.clone();
            Box::pin(try_stream! {
                let mut continuation_handle = None;
                loop {
                    let (response, next_continuation_handle) = client.query_metadata_page(
                        request.clone(),
                        std::mem::take(&mut continuation_handle),
                        max_page_size,
                    ).await?;
                    yield response;

                    if next_continuation_handle.is_none() {
                        return;
                    }

                    continuation_handle = next_continuation_handle;
                }
            })
        }

        pub fn update_metadata_batch(
            &self,
            request: UpdateMetadataBatchRequest,
        ) -> BoxStream<'static, Result<UpdateMetadataBatchResponse, Status>> {
            let request = self
                .client
                .put(format!("{}/v1beta/metadata", self.base_url))
                .headers(get_headers())
                .json(&models::UpdateMetadataBatchRequest::from(request));
            Box::pin(stream! {
                let response = request
                    .send_with_retry()
                    .await
                    .expect("update metadata batch request failed");
                debug!(?response);

                if response.status() != StatusCode::OK {
                    yield Err(response.to_tonic_status());
                    return;
                }

                let response: models::UpdateMetadataBatchResponse =
                    response.json().await.expect("failed to read response body");

                yield Ok(response.into());
            })
        }

        pub fn delete_metadata_batch(
            &self,
            request: DeleteMetadataBatchRequest,
        ) -> BoxStream<'static, Result<DeleteMetadataBatchResponse, Status>> {
            let request = self
                .client
                .delete(format!("{}/v1beta/metadata", self.base_url))
                .headers(get_headers())
                .json(&models::DeleteMetadataBatchRequest::from(request));
            Box::pin(stream! {
                let response = request
                    .send_with_retry()
                    .await
                    .expect("delete metadata batch request failed");
                debug!(?response);

                if response.status() != StatusCode::OK {
                    yield Err(response.to_tonic_status());
                    return;
                }

                let response: models::DeleteMetadataBatchResponse =
                    response.json().await.expect("failed to read response body");

                yield Ok(response.into());
            })
        }

        async fn query_metadata_page(
            &self,
            request: QueryMetadataRequest,
            continuation_handle: Option<String>,
            max_page_size: Option<usize>,
        ) -> Result<(QueryMetadataResponse, Option<String>), Status> {
            let response = self
                .client
                .post(format!("{}/v1beta/metadata/query", self.base_url))
                .headers(get_headers())
                .json(&models::QueryMetadataRequest {
                    namespace: request.namespace,
                    resource_location: request.resource_location.unwrap().into(),
                    continuation_handle,
                    max_page_size,
                })
                .send_with_retry()
                .await
                .expect("query metadata request failed");

            if response.status() != StatusCode::OK {
                return Err(response.to_tonic_status());
            }

            let response: models::QueryMetadataResponse = response.json().await.expect("failed to read response body");

            Ok((
                QueryMetadataResponse {
                    results: response
                        .results
                        .into_iter()
                        .map(|entry| QueryMetadataResponseEntry {
                            user_metadata_key: entry.user_metadata_key,
                            user_metadata: Some(entry.user_metadata.into()),
                        })
                        .collect(),
                },
                response.next_continuation_handle,
            ))
        }
    }
}
