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
    use crate::common::clients::{
        fileobject::types::v1alpha::*,
        utils::{
            get_headers,
            rest::{encode_path_segment, encode_query_params, RequestBuilderExt, ResponseExt},
            Request,
        },
    };
    use async_stream::try_stream;
    use bytes::Bytes;
    use futures::stream::BoxStream;
    use reqwest::StatusCode;
    use tonic::Status;

    pub mod models {
        use serde::{Deserialize, Serialize};
        use std::{collections::HashMap, fmt::Display};
        use storage_core::traits::fileobject::v1alpha::ResourceIdentity;

        use crate::common::clients::{
            fileobject::types::v1alpha,
            utils::{grpc::create_timestamp_from_datetime, rest::OffsetDateTimeWrapper},
        };

        #[derive(Serialize, Deserialize, Debug)]
        pub enum WriteRequestMethod {
            #[serde(rename = "post")]
            Post,
            #[serde(rename = "put")]
            Put,
        }

        impl From<WriteRequestMethod> for i32 {
            fn from(value: WriteRequestMethod) -> Self {
                match value {
                    WriteRequestMethod::Post => v1alpha::UploadMethod::Post as i32,
                    WriteRequestMethod::Put => v1alpha::UploadMethod::Put as i32,
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub enum WriteRedirectResponse {
            #[serde(rename = "redirect")]
            Redirect(WriteRedirectProperties),
            #[serde(rename = "multipart")]
            Multipart(MultipartUploadProperties),
        }

        impl From<WriteRedirectResponse> for v1alpha::WriteResponse {
            fn from(value: WriteRedirectResponse) -> Self {
                match value {
                    WriteRedirectResponse::Redirect(properties) => Self::Redirect(properties.into()),
                    WriteRedirectResponse::Multipart(properties) => Self::Multipart(properties.into()),
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct Header {
            pub name: String,
            pub value: String,
        }

        impl From<Header> for v1alpha::Header {
            fn from(value: Header) -> Self {
                Self {
                    name: value.name,
                    value: value.value,
                }
            }
        }

        impl From<v1alpha::Header> for Header {
            fn from(value: v1alpha::Header) -> Self {
                Self {
                    name: value.name,
                    value: value.value,
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct Redirect {
            pub redirect_target_url: String,
            pub additional_headers: HashMap<String, String>,
        }

        impl From<Redirect> for v1alpha::Redirect {
            fn from(value: Redirect) -> Self {
                Self {
                    additional_headers: value
                        .additional_headers
                        .into_iter()
                        .map(|(name, value)| v1alpha::Header { name, value })
                        .collect(),
                    redirect_target_url: value.redirect_target_url,
                    ..Default::default()
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct WriteRedirectProperties {
            pub redirect_target_url: String,
            pub additional_headers: Vec<Header>,
            pub method: WriteRequestMethod,
            pub completion_header_names: Vec<String>,
        }

        impl From<WriteRedirectProperties> for v1alpha::WriteRedirectProperties {
            fn from(value: WriteRedirectProperties) -> Self {
                Self {
                    redirect_target_url: value.redirect_target_url,
                    additional_headers: value.additional_headers.into_iter().map(Into::into).collect(),
                    method: value.method.into(),
                    completion_header_names: value.completion_header_names,
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct ResourceInfo {
            pub resource_identity: String,
            pub metadata: Metadata,
        }

        impl From<ResourceInfo> for v1alpha::ResourceInfo {
            fn from(value: ResourceInfo) -> Self {
                Self {
                    resource_identity: Some(ResourceIdentity {
                        encoded_identity: value.resource_identity,
                    }),
                    metadata: Some(value.metadata.into()),
                }
            }
        }

        impl From<ResourceInfo> for v1alpha::WriteResponse {
            fn from(value: ResourceInfo) -> Self {
                Self::Direct(value.into())
            }
        }

        impl From<ResourceInfo> for v1alpha::CompleteRedirectUploadResponse {
            fn from(value: ResourceInfo) -> Self {
                Self {
                    resource_info: Some(value.into()),
                }
            }
        }

        impl From<ResourceInfo> for v1alpha::CompleteMultipartUploadResponse {
            fn from(value: ResourceInfo) -> Self {
                Self {
                    resource_info: Some(value.into()),
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct Metadata {
            #[serde(default)]
            pub data_object_size: Option<u64>,
            #[serde(default)]
            pub last_modified_timestamp: Option<OffsetDateTimeWrapper>,
        }

        impl From<Metadata> for v1alpha::Metadata {
            fn from(value: Metadata) -> Self {
                Self {
                    data_object_size: value.data_object_size,
                    last_modified_timestamp: value.last_modified_timestamp.map(create_timestamp_from_datetime),
                    native_metadata: Default::default(),
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct UploadPartRequest {
            pub upload_id: String,
            pub part_number: u32,
            #[serde(default)]
            pub part_count: Option<u32>,
        }

        impl From<v1alpha::UploadPartRequest> for UploadPartRequest {
            fn from(value: v1alpha::UploadPartRequest) -> Self {
                Self {
                    upload_id: value.upload_id,
                    part_number: value.part_number,
                    part_count: value.part_count,
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct UploadPartResponse {
            pub part_write_redirects: Vec<WriteRedirectProperties>,
        }

        impl From<UploadPartResponse> for v1alpha::UploadPartResponse {
            fn from(value: UploadPartResponse) -> Self {
                Self {
                    part_write_redirects: value.part_write_redirects.into_iter().map(Into::into).collect(),
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct MultipartUploadProperties {
            pub upload_id: String,
            #[serde(default)]
            pub minimum_size_per_part: Option<u64>,
            #[serde(default)]
            pub maximum_size_per_part: Option<u64>,
            #[serde(default)]
            pub maximum_parts_number: Option<u32>,
            pub first_part_write_redirect: WriteRedirectProperties,
        }

        impl From<MultipartUploadProperties> for v1alpha::CreateMultipartUploadResponse {
            fn from(value: MultipartUploadProperties) -> Self {
                Self {
                    upload_id: value.upload_id,
                    minimum_size_per_part: value.minimum_size_per_part,
                    maximum_size_per_part: value.maximum_size_per_part,
                    maximum_parts_number: value.maximum_parts_number,
                    first_part_write_redirect: Some(value.first_part_write_redirect.into()),
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct CompleteMultipartUploadRequest {
            pub upload_id: String,
            pub parts: Vec<CompletedUploadPart>,
        }

        impl From<v1alpha::CompleteMultipartUploadRequest> for CompleteMultipartUploadRequest {
            fn from(value: v1alpha::CompleteMultipartUploadRequest) -> Self {
                Self {
                    upload_id: value.upload_id,
                    parts: value.parts.into_iter().map(Into::into).collect(),
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct AbortMultipartUploadRequest {
            pub upload_id: String,
        }

        impl From<v1alpha::AbortMultipartUploadRequest> for AbortMultipartUploadRequest {
            fn from(value: v1alpha::AbortMultipartUploadRequest) -> Self {
                Self {
                    upload_id: value.upload_id,
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct CompletedUploadPart {
            pub part_number: u32,
            pub additional_headers: Vec<Header>,
        }

        impl From<v1alpha::CompletedUploadPart> for CompletedUploadPart {
            fn from(value: v1alpha::CompletedUploadPart) -> Self {
                Self {
                    part_number: value.part_number,
                    additional_headers: value.headers.into_iter().map(Into::into).collect(),
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct CompleteUploadRequest {
            pub additional_headers: Vec<Header>,
        }

        impl From<v1alpha::CompleteRedirectUploadRequest> for CompleteUploadRequest {
            fn from(value: v1alpha::CompleteRedirectUploadRequest) -> Self {
                Self {
                    additional_headers: value.additional_headers.into_iter().map(Into::into).collect(),
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct UploadOptionsResponse {
            pub write_type_intervals: Vec<WriteTypeInterval>,
        }

        impl From<UploadOptionsResponse> for v1alpha::FetchWriteTypeInfoResponse {
            fn from(value: UploadOptionsResponse) -> Self {
                Self {
                    write_type_intervals: value.write_type_intervals.into_iter().map(Into::into).collect(),
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct WriteTypeInterval {
            pub minimum_data_object_size: u64,
            pub maximum_data_object_size: u64,
            pub preferred_upload_method: UploadPreference,
        }

        impl From<WriteTypeInterval> for v1alpha::WriteTypeForSizeInterval {
            fn from(value: WriteTypeInterval) -> Self {
                Self {
                    maximum_data_object_size: value.maximum_data_object_size,
                    minimum_data_object_size: value.minimum_data_object_size,
                    preferred_upload_method: value.preferred_upload_method.into(),
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub enum UploadPreference {
            #[serde(rename = "body")]
            Body,
            #[serde(rename = "redirect")]
            Redirect,
            #[serde(rename = "multipart")]
            Multipart,
        }

        impl Display for UploadPreference {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                match self {
                    Self::Body => f.write_str("body"),
                    Self::Redirect => f.write_str("redirect"),
                    Self::Multipart => f.write_str("multipart"),
                }
            }
        }

        impl From<i32> for UploadPreference {
            fn from(value: i32) -> Self {
                match value {
                    v if v == v1alpha::UploadPreference::Body as i32 => Self::Body,
                    v if v == v1alpha::UploadPreference::Redirect as i32 => Self::Redirect,
                    v if v == v1alpha::UploadPreference::Multipart as i32 => Self::Multipart,
                    v => panic!("unexpected upload preference {:?}", v),
                }
            }
        }

        impl From<UploadPreference> for i32 {
            fn from(value: UploadPreference) -> Self {
                match value {
                    UploadPreference::Body => v1alpha::UploadPreference::Body as i32,
                    UploadPreference::Redirect => v1alpha::UploadPreference::Redirect as i32,
                    UploadPreference::Multipart => v1alpha::UploadPreference::Multipart as i32,
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct EnumerateResult {
            pub next_continuation_handle: Option<String>,
            pub items: Vec<AddressInfo>,
        }

        impl From<EnumerateResult> for v1alpha::EnumerateResponse {
            fn from(value: EnumerateResult) -> Self {
                Self {
                    items: value.items.into_iter().map(Into::into).collect(),
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct CopyRequest {
            pub source_resource_identity: Option<ResourceIdentity>,
            pub destination_resource_address: String,
        }

        impl From<v1alpha::CopyRequest> for CopyRequest {
            fn from(value: v1alpha::CopyRequest) -> Self {
                Self {
                    source_resource_identity: value.source_resource_identity,
                    destination_resource_address: value.destination_resource_address,
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct MoveRequest {
            pub source_resource_address: String,
            pub destination_resource_address: String,
        }

        impl From<v1alpha::MoveRequest> for MoveRequest {
            fn from(value: v1alpha::MoveRequest) -> Self {
                Self {
                    source_resource_address: value.source_resource_address,
                    destination_resource_address: value.destination_resource_address,
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct AddressInfo {
            pub resource_address: String,
            pub metadata: Metadata,
        }

        impl From<AddressInfo> for v1alpha::AddressInfo {
            fn from(value: AddressInfo) -> Self {
                Self {
                    resource_address: value.resource_address,
                    metadata: Some(value.metadata.into()),
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct IsReadOnlyResponse {
            pub is_read_only: bool,
        }

        impl From<IsReadOnlyResponse> for v1alpha::IsReadOnlyResponse {
            fn from(value: IsReadOnlyResponse) -> Self {
                Self {
                    is_read_only: value.is_read_only,
                }
            }
        }

        #[track_caller]
        pub fn get_download_preference_string(preference: i32) -> &'static str {
            use v1alpha::DownloadPreference;

            match preference {
                p if p == DownloadPreference::Body as i32 => "body",
                p if p == DownloadPreference::Redirect as i32 => "redirect",
                p => panic!("unexpected download preference {:?}", p),
            }
        }

        #[track_caller]
        pub fn get_read_response_resource_info(response: &reqwest::Response) -> ResourceInfo {
            ResourceInfo {
                resource_identity: get_read_response_resource_identity(response),
                metadata: get_read_response_metadata(response),
            }
        }

        #[track_caller]
        pub fn get_read_response_metadata(response: &reqwest::Response) -> Metadata {
            serde_json::from_slice(
                response
                    .headers()
                    .get("x-nvidia-omniverse-storage-metadata")
                    .expect("resource metadata header is missing")
                    .as_bytes(),
            )
            .expect("failed to decode resource metadata")
        }

        #[track_caller]
        pub fn get_read_response_resource_identity(response: &reqwest::Response) -> String {
            response
                .headers()
                .get("x-nvidia-omniverse-storage-resource-identity")
                .expect("resource identity header is missing")
                .to_str()
                .expect("failed to decode resource identity")
                .into()
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

        pub async fn stat(&self, request: Request<StatRequest>) -> Result<StatResponse, Status> {
            let (request, headers) = request.into_parts();
            let response = self
                .client
                .head(format!(
                    "{}/v1alpha/fileobject/by-address/{}",
                    self.base_url,
                    encode_path_segment(&request.resource_address)
                ))
                .headers(headers)
                .send_with_retry()
                .await
                .expect("stat request failed");
            if response.status() != StatusCode::NO_CONTENT {
                return Err(response.to_tonic_status());
            }

            Ok(StatResponse {
                resource_info: Some(models::get_read_response_resource_info(&response).into()),
            })
        }

        pub async fn is_read_only(&self, request: Request<IsReadOnlyRequest>) -> Result<IsReadOnlyResponse, Status> {
            let (request, headers) = request.into_parts();
            let response = self
                .client
                .get(format!(
                    "{}/v1alpha/fileobject/is-read-only/{}",
                    self.base_url,
                    encode_path_segment(&request.resource_address)
                ))
                .headers(headers)
                .send_with_retry()
                .await
                .expect("is-read-only request failed");
            if response.status() != StatusCode::OK {
                return Err(response.to_tonic_status());
            }

            let response: models::IsReadOnlyResponse = response.json().await.expect("response body read failed");

            Ok(response.into())
        }

        pub async fn read(&self, request: ReadRequest) -> Result<ReadResponse, Status> {
            let mut query_params = Vec::new();
            if let Some(preference) = request.download_preference {
                query_params.push(format!(
                    "download_preference={}",
                    models::get_download_preference_string(preference)
                ));
            };

            let response = self
                .client
                .get(format!(
                    "{}/v1alpha/fileobject/by-identity/{}{}",
                    self.base_url,
                    encode_path_segment(
                        request
                            .resource_identity
                            .as_ref()
                            .expect("resource identity is missing")
                            .encoded_identity
                            .as_ref()
                    ),
                    encode_query_params(&query_params),
                ))
                .headers(get_headers())
                .send_with_retry()
                .await
                .expect("read request failed");
            match response.status() {
                StatusCode::OK => {
                    let metadata = models::get_read_response_metadata(&response);
                    let content = response.bytes().await.expect("failed to read response body");

                    Ok(ReadResponse::Direct(metadata.into(), content))
                },
                StatusCode::MULTIPLE_CHOICES => {
                    let metadata = models::get_read_response_metadata(&response);
                    let redirect: models::Redirect = response.json().await.expect("failed to read response body");

                    Ok(ReadResponse::Redirect(metadata.into(), redirect.into()))
                },
                _ => Err(response.to_tonic_status()),
            }
        }

        pub async fn read_from_address(
            &self,
            request: ReadFromAddressRequest,
        ) -> Result<ReadFromAddressResponse, Status> {
            let mut query_params = Vec::new();
            if let Some(preference) = request.download_preference {
                query_params.push(format!(
                    "download_preference={}",
                    models::get_download_preference_string(preference)
                ));
            };

            let response = self
                .client
                .get(format!(
                    "{}/v1alpha/fileobject/by-address/{}{}",
                    self.base_url,
                    encode_path_segment(&request.resource_address),
                    encode_query_params(&query_params),
                ))
                .headers(get_headers())
                .send_with_retry()
                .await
                .expect("read from address request failed");
            match response.status() {
                StatusCode::OK => {
                    let resource_info = models::get_read_response_resource_info(&response);
                    let content = response.bytes().await.expect("failed to read response body");

                    Ok(ReadFromAddressResponse::Direct(resource_info.into(), content))
                },
                StatusCode::MULTIPLE_CHOICES => {
                    let resource_info = models::get_read_response_resource_info(&response);
                    let redirect: models::Redirect = response.json().await.expect("failed to read response body");

                    Ok(ReadFromAddressResponse::Redirect(resource_info.into(), redirect.into()))
                },
                _ => Err(response.to_tonic_status()),
            }
        }

        pub async fn write(&self, params: WriteParameters, content: Bytes) -> Result<WriteResponse, Status> {
            let mut query_params = vec![format!("data_object_size={}", params.data_object_size)];
            if let Some(preference) = params.upload_preference {
                query_params.push(format!(
                    "upload_preference={}",
                    models::UploadPreference::from(preference)
                ));
            }

            if let Some(previous_version) = params.previous_version {
                query_params.push(format!("previous_version={}", previous_version.encoded_identity));
            };

            let response = self
                .client
                .put(format!(
                    "{}/v1alpha/fileobject/by-address/{}{}",
                    self.base_url,
                    encode_path_segment(&params.destination_resource_address),
                    encode_query_params(&query_params),
                ))
                .headers(get_headers())
                .body(content)
                .send_with_retry()
                .await
                .expect("write request failed");

            match response.status() {
                StatusCode::CREATED => {
                    let resource_info: models::ResourceInfo =
                        response.json().await.expect("failed to read response body");

                    Ok(resource_info.into())
                },
                StatusCode::MULTIPLE_CHOICES => {
                    let redirect_response: models::WriteRedirectResponse =
                        response.json().await.expect("failed to read response body");

                    Ok(redirect_response.into())
                },
                _ => Err(response.to_tonic_status()),
            }
        }

        pub async fn upload_part(&self, request: UploadPartRequest) -> Result<UploadPartResponse, Status> {
            let response = self
                .client
                .post(format!(
                    "{}/v1alpha/fileobject/by-address/{}/multipart/prepare",
                    self.base_url,
                    encode_path_segment(&request.destination_resource_address)
                ))
                .headers(get_headers())
                .json(&models::UploadPartRequest::from(request))
                .send_with_retry()
                .await
                .expect("upload part request failed");

            if response.status() != StatusCode::OK {
                return Err(response.to_tonic_status());
            }

            let response: models::UploadPartResponse = response.json().await.expect("failed to read response body");

            Ok(response.into())
        }

        pub async fn complete_redirect_upload(
            &self,
            request: CompleteRedirectUploadRequest,
        ) -> Result<CompleteRedirectUploadResponse, Status> {
            let response = self
                .client
                .post(format!(
                    "{}/v1alpha/fileobject/by-address/{}/redirect/complete",
                    self.base_url,
                    encode_path_segment(&request.destination_resource_address)
                ))
                .headers(get_headers())
                .json(&models::CompleteUploadRequest::from(request))
                .send_with_retry()
                .await
                .expect("complete redirect upload request failed");

            if response.status() != StatusCode::OK {
                return Err(response.to_tonic_status());
            }

            let response: models::ResourceInfo = response.json().await.expect("failed to read response body");

            Ok(response.into())
        }

        pub async fn complete_multipart_upload(
            &self,
            request: CompleteMultipartUploadRequest,
        ) -> Result<CompleteMultipartUploadResponse, Status> {
            let response = self
                .client
                .post(format!(
                    "{}/v1alpha/fileobject/by-address/{}/multipart/complete",
                    self.base_url,
                    encode_path_segment(&request.destination_resource_address)
                ))
                .headers(get_headers())
                .json(&models::CompleteMultipartUploadRequest::from(request))
                .send_with_retry()
                .await
                .expect("complete multipart upload request failed");

            if response.status() != StatusCode::OK {
                return Err(response.to_tonic_status());
            }

            let response: models::ResourceInfo = response.json().await.expect("failed to read response body");

            Ok(response.into())
        }

        pub async fn abort_multipart_upload(
            &self,
            request: AbortMultipartUploadRequest,
        ) -> Result<AbortMultipartUploadResponse, Status> {
            let response = self
                .client
                .post(format!(
                    "{}/v1alpha/fileobject/by-address/{}/multipart/abort",
                    self.base_url,
                    encode_path_segment(&request.destination_resource_address)
                ))
                .headers(get_headers())
                .json(&models::AbortMultipartUploadRequest::from(request))
                .send_with_retry()
                .await
                .expect("abort multipart upload request failed");

            if response.status() != StatusCode::NO_CONTENT {
                return Err(response.to_tonic_status());
            }

            Ok(Default::default())
        }

        pub async fn delete(&self, request: DeleteRequest) -> Result<DeleteResponse, Status> {
            let mut query_params = Vec::new();
            if let Some(previous_version) = request.previous_version {
                query_params.push(format!("previous_version={}", previous_version.encoded_identity));
            }

            let response = self
                .client
                .delete(format!(
                    "{}/v1alpha/fileobject/by-address/{}{}",
                    self.base_url,
                    encode_path_segment(&request.resource_address),
                    encode_query_params(&query_params)
                ))
                .headers(get_headers())
                .send_with_retry()
                .await
                .expect("delete request failed");

            if response.status() != StatusCode::NO_CONTENT {
                Err(response.to_tonic_status())
            } else {
                Ok(DeleteResponse {})
            }
        }

        pub async fn fetch_write_type_info(
            &self,
            request: FetchWriteTypeInfoRequest,
        ) -> Result<FetchWriteTypeInfoResponse, Status> {
            let response = self
                .client
                .get(format!(
                    "{}/v1alpha/fileobject/upload-options/by-address/{}",
                    self.base_url,
                    encode_path_segment(&request.destination_resource_address)
                ))
                .headers(get_headers())
                .send_with_retry()
                .await
                .expect("fetch write type info request failed");

            if response.status() != StatusCode::OK {
                return Err(response.to_tonic_status());
            }

            let response: models::UploadOptionsResponse = response.json().await.expect("response body read failed");

            Ok(response.into())
        }

        pub async fn copy(&self, request: CopyRequest) -> Result<CopyResponse, Status> {
            let response = self
                .client
                .post(format!(
                    "{}/v1alpha/fileobject/by-identity/{}/copy",
                    self.base_url,
                    encode_path_segment(
                        request
                            .source_resource_identity
                            .as_ref()
                            .expect("resource identity is missing")
                            .encoded_identity
                            .as_str()
                    )
                ))
                .json(&models::CopyRequest::from(request))
                .headers(get_headers())
                .send_with_retry()
                .await
                .expect("copy request failed");

            if response.status() != StatusCode::CREATED {
                return Err(response.to_tonic_status());
            }

            let response: CopyResponse = response.json().await.expect("copy response body read failed");
            Ok(response)
        }

        pub async fn move_object(&self, request: MoveRequest) -> Result<MoveResponse, Status> {
            let response = self
                .client
                .post(format!(
                    "{}/v1alpha/fileobject/by-address/{}/move",
                    self.base_url,
                    encode_path_segment(&request.source_resource_address)
                ))
                .json(&models::MoveRequest::from(request))
                .headers(get_headers())
                .send_with_retry()
                .await
                .expect("move object request failed");
            if response.status() != StatusCode::CREATED {
                return Err(response.to_tonic_status());
            }
            let response: MoveResponse = response.json().await.expect("move response body read failed");
            Ok(response)
        }

        pub fn enumerate(&self, request: EnumerateRequest) -> BoxStream<'static, Result<EnumerateResponse, Status>> {
            let client = self.clone();
            Box::pin(try_stream! {
                let mut continuation_handle: Option<String> = None;
                loop {
                    let (response, next_continuation_handle) = client.enumerate_once(
                        request.clone(),
                        continuation_handle.as_deref(),
                    ).await?;
                    yield response;

                    if next_continuation_handle.is_some() {
                        continuation_handle = next_continuation_handle;
                    } else {
                        break;
                    }
                }
            })
        }

        pub async fn enumerate_once(
            &self,
            request: EnumerateRequest,
            continuation_handle: Option<&str>,
        ) -> Result<(EnumerateResponse, Option<String>), Status> {
            let mut query_params = Vec::new();
            if let Some(continuation_handle) = continuation_handle {
                query_params.push(format!("continuation_handle={}", continuation_handle));
            }

            let response = self
                .client
                .get(format!(
                    "{}/v1alpha/fileobject/data-objects/{}{}",
                    self.base_url,
                    encode_path_segment(&request.resource_address),
                    encode_query_params(&query_params)
                ))
                .headers(get_headers())
                .send_with_retry()
                .await
                .expect("enumerate request failed");

            if response.status() != StatusCode::OK {
                return Err(response.to_tonic_status());
            }

            let mut response: models::EnumerateResult = response.json().await.expect("response body read failed");
            let next_continuation_handle = std::mem::take(&mut response.next_continuation_handle);

            Ok((response.into(), next_continuation_handle))
        }
    }
}

pub mod v1beta {
    use crate::common::clients::{
        fileobject::types::v1beta::*,
        utils::{
            get_headers,
            rest::{encode_path_segment, encode_query_params, RequestBuilderExt, ResponseExt},
            Request,
        },
    };
    use async_stream::try_stream;
    use bytes::Bytes;
    use futures::stream::BoxStream;
    use reqwest::StatusCode;
    use tonic::Status;

    pub mod models {
        use serde::{Deserialize, Serialize};
        use std::{collections::HashMap, fmt::Display};
        use storage_core::traits::fileobject::v1beta::ResourceIdentity;

        use crate::common::clients::{
            fileobject::types::v1beta,
            utils::{grpc::create_timestamp_from_datetime, rest::OffsetDateTimeWrapper},
        };

        #[derive(Serialize, Deserialize, Debug)]
        pub enum WriteRequestMethod {
            #[serde(rename = "post")]
            Post,
            #[serde(rename = "put")]
            Put,
        }

        impl From<WriteRequestMethod> for i32 {
            fn from(value: WriteRequestMethod) -> Self {
                match value {
                    WriteRequestMethod::Post => v1beta::UploadMethod::Post as i32,
                    WriteRequestMethod::Put => v1beta::UploadMethod::Put as i32,
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub enum WriteRedirectResponse {
            #[serde(rename = "redirect")]
            Redirect(WriteRedirectProperties),
            #[serde(rename = "multipart")]
            Multipart(MultipartUploadProperties),
        }

        impl From<WriteRedirectResponse> for v1beta::WriteResponse {
            fn from(value: WriteRedirectResponse) -> Self {
                match value {
                    WriteRedirectResponse::Redirect(properties) => Self::Redirect(properties.into()),
                    WriteRedirectResponse::Multipart(properties) => Self::Multipart(properties.into()),
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct Header {
            pub name: String,
            pub value: String,
        }

        impl From<Header> for v1beta::Header {
            fn from(value: Header) -> Self {
                Self {
                    name: value.name,
                    value: value.value,
                }
            }
        }

        impl From<v1beta::Header> for Header {
            fn from(value: v1beta::Header) -> Self {
                Self {
                    name: value.name,
                    value: value.value,
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct Redirect {
            pub redirect_target_url: String,
            pub additional_headers: HashMap<String, String>,
        }

        impl From<Redirect> for v1beta::Redirect {
            fn from(value: Redirect) -> Self {
                Self {
                    additional_headers: value
                        .additional_headers
                        .into_iter()
                        .map(|(name, value)| v1beta::Header { name, value })
                        .collect(),
                    redirect_target_url: value.redirect_target_url,
                    ..Default::default()
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct WriteRedirectProperties {
            pub redirect_target_url: String,
            pub additional_headers: Vec<Header>,
            pub method: WriteRequestMethod,
            pub completion_header_names: Vec<String>,
        }

        impl From<WriteRedirectProperties> for v1beta::WriteRedirectProperties {
            fn from(value: WriteRedirectProperties) -> Self {
                Self {
                    redirect_target_url: value.redirect_target_url,
                    additional_headers: value.additional_headers.into_iter().map(Into::into).collect(),
                    method: value.method.into(),
                    completion_header_names: value.completion_header_names,
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct ResourceInfo {
            pub resource_identity: String,
            pub metadata: Metadata,
        }

        impl From<ResourceInfo> for v1beta::ResourceInfo {
            fn from(value: ResourceInfo) -> Self {
                Self {
                    resource_identity: Some(ResourceIdentity {
                        encoded_identity: value.resource_identity,
                    }),
                    metadata: Some(value.metadata.into()),
                }
            }
        }

        impl From<ResourceInfo> for v1beta::WriteResponse {
            fn from(value: ResourceInfo) -> Self {
                Self::Direct(value.into())
            }
        }

        impl From<ResourceInfo> for v1beta::CompleteRedirectUploadResponse {
            fn from(value: ResourceInfo) -> Self {
                Self {
                    resource_info: Some(value.into()),
                }
            }
        }

        impl From<ResourceInfo> for v1beta::CompleteMultipartUploadResponse {
            fn from(value: ResourceInfo) -> Self {
                Self {
                    resource_info: Some(value.into()),
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct Metadata {
            #[serde(default)]
            pub data_object_size: Option<u64>,
            #[serde(default)]
            pub last_modified_timestamp: Option<OffsetDateTimeWrapper>,
        }

        impl From<Metadata> for v1beta::Metadata {
            fn from(value: Metadata) -> Self {
                Self {
                    data_object_size: value.data_object_size,
                    last_modified_timestamp: value.last_modified_timestamp.map(create_timestamp_from_datetime),
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct UploadPartRequest {
            pub upload_id: String,
            pub part_number: u32,
            #[serde(default)]
            pub part_count: Option<u32>,
        }

        impl From<v1beta::UploadPartRequest> for UploadPartRequest {
            fn from(value: v1beta::UploadPartRequest) -> Self {
                Self {
                    upload_id: value.upload_id,
                    part_number: value.part_number,
                    part_count: value.part_count,
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct UploadPartResponse {
            pub part_write_redirects: Vec<WriteRedirectProperties>,
        }

        impl From<UploadPartResponse> for v1beta::UploadPartResponse {
            fn from(value: UploadPartResponse) -> Self {
                Self {
                    part_write_redirects: value.part_write_redirects.into_iter().map(Into::into).collect(),
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct MultipartUploadProperties {
            pub upload_id: String,
            #[serde(default)]
            pub minimum_size_per_part: Option<u64>,
            #[serde(default)]
            pub maximum_size_per_part: Option<u64>,
            #[serde(default)]
            pub maximum_parts_number: Option<u32>,
            pub first_part_write_redirect: WriteRedirectProperties,
        }

        impl From<MultipartUploadProperties> for v1beta::CreateMultipartUploadResponse {
            fn from(value: MultipartUploadProperties) -> Self {
                Self {
                    upload_id: value.upload_id,
                    minimum_size_per_part: value.minimum_size_per_part,
                    maximum_size_per_part: value.maximum_size_per_part,
                    maximum_parts_number: value.maximum_parts_number,
                    first_part_write_redirect: Some(value.first_part_write_redirect.into()),
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct CompleteMultipartUploadRequest {
            pub upload_id: String,
            pub parts: Vec<CompletedUploadPart>,
        }

        impl From<v1beta::CompleteMultipartUploadRequest> for CompleteMultipartUploadRequest {
            fn from(value: v1beta::CompleteMultipartUploadRequest) -> Self {
                Self {
                    upload_id: value.upload_id,
                    parts: value.parts.into_iter().map(Into::into).collect(),
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct AbortMultipartUploadRequest {
            pub upload_id: String,
        }

        impl From<v1beta::AbortMultipartUploadRequest> for AbortMultipartUploadRequest {
            fn from(value: v1beta::AbortMultipartUploadRequest) -> Self {
                Self {
                    upload_id: value.upload_id,
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct CompletedUploadPart {
            pub part_number: u32,
            pub additional_headers: Vec<Header>,
        }

        impl From<v1beta::CompletedUploadPart> for CompletedUploadPart {
            fn from(value: v1beta::CompletedUploadPart) -> Self {
                Self {
                    part_number: value.part_number,
                    additional_headers: value.headers.into_iter().map(Into::into).collect(),
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct CompleteUploadRequest {
            pub additional_headers: Vec<Header>,
        }

        impl From<v1beta::CompleteRedirectUploadRequest> for CompleteUploadRequest {
            fn from(value: v1beta::CompleteRedirectUploadRequest) -> Self {
                Self {
                    additional_headers: value.additional_headers.into_iter().map(Into::into).collect(),
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct UploadOptionsResponse {
            pub write_type_intervals: Vec<WriteTypeInterval>,
        }

        impl From<UploadOptionsResponse> for v1beta::FetchWriteTypeInfoResponse {
            fn from(value: UploadOptionsResponse) -> Self {
                Self {
                    write_type_intervals: value.write_type_intervals.into_iter().map(Into::into).collect(),
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct WriteTypeInterval {
            pub minimum_data_object_size: u64,
            pub maximum_data_object_size: u64,
            pub preferred_upload_method: UploadPreference,
        }

        impl From<WriteTypeInterval> for v1beta::WriteTypeForSizeInterval {
            fn from(value: WriteTypeInterval) -> Self {
                Self {
                    maximum_data_object_size: value.maximum_data_object_size,
                    minimum_data_object_size: value.minimum_data_object_size,
                    preferred_upload_method: value.preferred_upload_method.into(),
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub enum UploadPreference {
            #[serde(rename = "body")]
            Body,
            #[serde(rename = "redirect")]
            Redirect,
            #[serde(rename = "multipart")]
            Multipart,
        }

        impl Display for UploadPreference {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                match self {
                    Self::Body => f.write_str("body"),
                    Self::Redirect => f.write_str("redirect"),
                    Self::Multipart => f.write_str("multipart"),
                }
            }
        }

        impl From<i32> for UploadPreference {
            fn from(value: i32) -> Self {
                match value {
                    v if v == v1beta::UploadPreference::Body as i32 => Self::Body,
                    v if v == v1beta::UploadPreference::Redirect as i32 => Self::Redirect,
                    v if v == v1beta::UploadPreference::Multipart as i32 => Self::Multipart,
                    v => panic!("unexpected upload preference {:?}", v),
                }
            }
        }

        impl From<UploadPreference> for i32 {
            fn from(value: UploadPreference) -> Self {
                match value {
                    UploadPreference::Body => v1beta::UploadPreference::Body as i32,
                    UploadPreference::Redirect => v1beta::UploadPreference::Redirect as i32,
                    UploadPreference::Multipart => v1beta::UploadPreference::Multipart as i32,
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct EnumerateResult {
            pub next_continuation_handle: Option<String>,
            pub items: Vec<AddressInfo>,
        }

        impl From<EnumerateResult> for v1beta::EnumerateResponse {
            fn from(value: EnumerateResult) -> Self {
                Self {
                    items: value.items.into_iter().map(Into::into).collect(),
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct AddressInfo {
            pub resource_address: String,
            pub metadata: Metadata,
        }

        impl From<AddressInfo> for v1beta::AddressInfo {
            fn from(value: AddressInfo) -> Self {
                Self {
                    resource_address: value.resource_address,
                    metadata: Some(value.metadata.into()),
                }
            }
        }

        #[track_caller]
        pub fn get_download_preference_string(preference: i32) -> &'static str {
            use v1beta::DownloadPreference;

            match preference {
                p if p == DownloadPreference::Body as i32 => "body",
                p if p == DownloadPreference::Redirect as i32 => "redirect",
                p => panic!("unexpected download preference {:?}", p),
            }
        }

        #[track_caller]
        pub fn get_read_response_resource_info(response: &reqwest::Response) -> ResourceInfo {
            ResourceInfo {
                resource_identity: get_read_response_resource_identity(response),
                metadata: get_read_response_metadata(response),
            }
        }

        #[track_caller]
        pub fn get_read_response_metadata(response: &reqwest::Response) -> Metadata {
            serde_json::from_slice(
                response
                    .headers()
                    .get("x-nvidia-omniverse-storage-metadata")
                    .expect("resource metadata header is missing")
                    .as_bytes(),
            )
            .expect("failed to decode resource metadata")
        }

        #[track_caller]
        pub fn get_read_response_resource_identity(response: &reqwest::Response) -> String {
            response
                .headers()
                .get("x-nvidia-omniverse-storage-resource-identity")
                .expect("resource identity header is missing")
                .to_str()
                .expect("failed to decode resource identity")
                .into()
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

        pub async fn stat(&self, request: Request<StatRequest>) -> Result<StatResponse, Status> {
            let (request, headers) = request.into_parts();
            let response = self
                .client
                .head(format!(
                    "{}/v1alpha/fileobject/by-address/{}",
                    self.base_url,
                    encode_path_segment(&request.resource_address)
                ))
                .headers(headers)
                .send_with_retry()
                .await
                .expect("stat request failed");
            if response.status() != StatusCode::NO_CONTENT {
                return Err(response.to_tonic_status());
            }

            Ok(StatResponse {
                resource_info: Some(models::get_read_response_resource_info(&response).into()),
            })
        }

        pub async fn read(&self, request: ReadRequest) -> Result<ReadResponse, Status> {
            let mut query_params = Vec::new();
            if let Some(preference) = request.download_preference {
                query_params.push(format!(
                    "download_preference={}",
                    models::get_download_preference_string(preference)
                ));
            };

            let response = self
                .client
                .get(format!(
                    "{}/v1alpha/fileobject/by-identity/{}{}",
                    self.base_url,
                    encode_path_segment(
                        request
                            .resource_identity
                            .as_ref()
                            .expect("resource identity is missing")
                            .encoded_identity
                            .as_ref()
                    ),
                    encode_query_params(&query_params),
                ))
                .headers(get_headers())
                .send_with_retry()
                .await
                .expect("read request failed");
            match response.status() {
                StatusCode::OK => {
                    let metadata = models::get_read_response_metadata(&response);
                    let content = response.bytes().await.expect("failed to read response body");

                    Ok(ReadResponse::Direct(metadata.into(), content))
                },
                StatusCode::MULTIPLE_CHOICES => {
                    let metadata = models::get_read_response_metadata(&response);
                    let redirect: models::Redirect = response.json().await.expect("failed to read response body");

                    Ok(ReadResponse::Redirect(metadata.into(), redirect.into()))
                },
                _ => Err(response.to_tonic_status()),
            }
        }

        pub async fn read_from_address(
            &self,
            request: ReadFromAddressRequest,
        ) -> Result<ReadFromAddressResponse, Status> {
            let mut query_params = Vec::new();
            if let Some(preference) = request.download_preference {
                query_params.push(format!(
                    "download_preference={}",
                    models::get_download_preference_string(preference)
                ));
            };

            let response = self
                .client
                .get(format!(
                    "{}/v1alpha/fileobject/by-address/{}{}",
                    self.base_url,
                    encode_path_segment(&request.resource_address),
                    encode_query_params(&query_params),
                ))
                .headers(get_headers())
                .send_with_retry()
                .await
                .expect("read from address request failed");
            match response.status() {
                StatusCode::OK => {
                    let resource_info = models::get_read_response_resource_info(&response);
                    let content = response.bytes().await.expect("failed to read response body");

                    Ok(ReadFromAddressResponse::Direct(resource_info.into(), content))
                },
                StatusCode::MULTIPLE_CHOICES => {
                    let resource_info = models::get_read_response_resource_info(&response);
                    let redirect: models::Redirect = response.json().await.expect("failed to read response body");

                    Ok(ReadFromAddressResponse::Redirect(resource_info.into(), redirect.into()))
                },
                _ => Err(response.to_tonic_status()),
            }
        }

        pub async fn write(&self, params: WriteParameters, content: Bytes) -> Result<WriteResponse, Status> {
            let mut query_params = vec![format!("data_object_size={}", params.data_object_size)];
            if let Some(preference) = params.upload_preference {
                query_params.push(format!(
                    "upload_preference={}",
                    models::UploadPreference::from(preference)
                ));
            }

            if let Some(previous_version) = params.previous_version {
                query_params.push(format!("previous_version={}", previous_version.encoded_identity));
            };

            let response = self
                .client
                .put(format!(
                    "{}/v1alpha/fileobject/by-address/{}{}",
                    self.base_url,
                    encode_path_segment(&params.destination_resource_address),
                    encode_query_params(&query_params),
                ))
                .headers(get_headers())
                .body(content)
                .send_with_retry()
                .await
                .expect("write request failed");

            match response.status() {
                StatusCode::CREATED => {
                    let resource_info: models::ResourceInfo =
                        response.json().await.expect("failed to read response body");

                    Ok(resource_info.into())
                },
                StatusCode::MULTIPLE_CHOICES => {
                    let redirect_response: models::WriteRedirectResponse =
                        response.json().await.expect("failed to read response body");

                    Ok(redirect_response.into())
                },
                _ => Err(response.to_tonic_status()),
            }
        }

        pub async fn upload_part(&self, request: UploadPartRequest) -> Result<UploadPartResponse, Status> {
            let response = self
                .client
                .post(format!(
                    "{}/v1alpha/fileobject/by-address/{}/multipart/prepare",
                    self.base_url,
                    encode_path_segment(&request.destination_resource_address)
                ))
                .headers(get_headers())
                .json(&models::UploadPartRequest::from(request))
                .send_with_retry()
                .await
                .expect("upload part request failed");

            if response.status() != StatusCode::OK {
                return Err(response.to_tonic_status());
            }

            let response: models::UploadPartResponse = response.json().await.expect("failed to read response body");

            Ok(response.into())
        }

        pub async fn complete_redirect_upload(
            &self,
            request: CompleteRedirectUploadRequest,
        ) -> Result<CompleteRedirectUploadResponse, Status> {
            let response = self
                .client
                .post(format!(
                    "{}/v1alpha/fileobject/by-address/{}/redirect/complete",
                    self.base_url,
                    encode_path_segment(&request.destination_resource_address)
                ))
                .headers(get_headers())
                .json(&models::CompleteUploadRequest::from(request))
                .send_with_retry()
                .await
                .expect("complete redirect upload request failed");

            if response.status() != StatusCode::OK {
                return Err(response.to_tonic_status());
            }

            let response: models::ResourceInfo = response.json().await.expect("failed to read response body");

            Ok(response.into())
        }

        pub async fn complete_multipart_upload(
            &self,
            request: CompleteMultipartUploadRequest,
        ) -> Result<CompleteMultipartUploadResponse, Status> {
            let response = self
                .client
                .post(format!(
                    "{}/v1alpha/fileobject/by-address/{}/multipart/complete",
                    self.base_url,
                    encode_path_segment(&request.destination_resource_address)
                ))
                .headers(get_headers())
                .json(&models::CompleteMultipartUploadRequest::from(request))
                .send_with_retry()
                .await
                .expect("complete multipart upload request failed");

            if response.status() != StatusCode::OK {
                return Err(response.to_tonic_status());
            }

            let response: models::ResourceInfo = response.json().await.expect("failed to read response body");

            Ok(response.into())
        }

        pub async fn abort_multipart_upload(
            &self,
            request: AbortMultipartUploadRequest,
        ) -> Result<AbortMultipartUploadResponse, Status> {
            let response = self
                .client
                .post(format!(
                    "{}/v1alpha/fileobject/by-address/{}/multipart/abort",
                    self.base_url,
                    encode_path_segment(&request.destination_resource_address)
                ))
                .headers(get_headers())
                .json(&models::AbortMultipartUploadRequest::from(request))
                .send_with_retry()
                .await
                .expect("abort multipart upload request failed");

            if response.status() != StatusCode::NO_CONTENT {
                return Err(response.to_tonic_status());
            }

            Ok(Default::default())
        }

        pub async fn delete(&self, request: DeleteRequest) -> Result<DeleteResponse, Status> {
            let mut query_params = Vec::new();
            if let Some(previous_version) = request.previous_version {
                query_params.push(format!("previous_version={}", previous_version.encoded_identity));
            }

            let response = self
                .client
                .delete(format!(
                    "{}/v1alpha/fileobject/by-address/{}{}",
                    self.base_url,
                    encode_path_segment(&request.resource_address),
                    encode_query_params(&query_params)
                ))
                .headers(get_headers())
                .send_with_retry()
                .await
                .expect("delete request failed");

            if response.status() != StatusCode::NO_CONTENT {
                Err(response.to_tonic_status())
            } else {
                Ok(DeleteResponse {})
            }
        }

        pub async fn fetch_write_type_info(
            &self,
            request: FetchWriteTypeInfoRequest,
        ) -> Result<FetchWriteTypeInfoResponse, Status> {
            let response = self
                .client
                .get(format!(
                    "{}/v1alpha/fileobject/upload-options/by-address/{}",
                    self.base_url,
                    encode_path_segment(&request.destination_resource_address)
                ))
                .headers(get_headers())
                .send_with_retry()
                .await
                .expect("fetch write type info request failed");

            if response.status() != StatusCode::OK {
                return Err(response.to_tonic_status());
            }

            let response: models::UploadOptionsResponse = response.json().await.expect("response body read failed");

            Ok(response.into())
        }

        pub fn enumerate(&self, request: EnumerateRequest) -> BoxStream<'static, Result<EnumerateResponse, Status>> {
            let client = self.clone();
            Box::pin(try_stream! {
                let mut continuation_handle: Option<String> = None;
                loop {
                    let (response, next_continuation_handle) = client.enumerate_once(
                        request.clone(),
                        continuation_handle.as_deref(),
                    ).await?;
                    yield response;

                    if next_continuation_handle.is_some() {
                        continuation_handle = next_continuation_handle;
                    } else {
                        break;
                    }
                }
            })
        }

        pub async fn enumerate_once(
            &self,
            request: EnumerateRequest,
            continuation_handle: Option<&str>,
        ) -> Result<(EnumerateResponse, Option<String>), Status> {
            let mut query_params = Vec::new();
            if let Some(continuation_handle) = continuation_handle {
                query_params.push(format!("continuation_handle={}", continuation_handle));
            }

            let response = self
                .client
                .get(format!(
                    "{}/v1beta/fileobject/data-objects/{}{}",
                    self.base_url,
                    encode_path_segment(&request.resource_address),
                    encode_query_params(&query_params)
                ))
                .headers(get_headers())
                .send_with_retry()
                .await
                .expect("enumerate request failed");

            if response.status() != StatusCode::OK {
                return Err(response.to_tonic_status());
            }

            let mut response: models::EnumerateResult = response.json().await.expect("response body read failed");
            let next_continuation_handle = std::mem::take(&mut response.next_continuation_handle);

            Ok((response.into(), next_continuation_handle))
        }
    }
}
