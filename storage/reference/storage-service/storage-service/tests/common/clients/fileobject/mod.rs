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

pub mod grpc;
pub mod rest;

pub mod types {
    pub mod v1alpha {
        use bytes::Bytes;
        pub use storage_core_protos::nvidia::omniverse::storage::fileobject::v1alpha::*;

        #[derive(Debug, Clone)]
        pub enum ReadResponse {
            Direct(Metadata, Bytes),
            Redirect(Metadata, Redirect),
        }

        #[derive(Debug, Clone)]
        pub enum ReadFromAddressResponse {
            Direct(ResourceInfo, Bytes),
            Redirect(ResourceInfo, Redirect),
        }

        #[derive(Debug, Clone)]
        pub enum WriteResponse {
            Direct(ResourceInfo),
            Redirect(WriteRedirectProperties),
            Multipart(CreateMultipartUploadResponse),
        }

        pub trait UnwrapIdentityExt {
            fn unwrap_identity(self) -> String;
        }

        impl UnwrapIdentityExt for ResourceInfo {
            fn unwrap_identity(self) -> String {
                self.resource_identity
                    .expect("resource identity is missing")
                    .encoded_identity
            }
        }
    }

    pub mod v1beta {
        use bytes::Bytes;
        pub use storage_core_protos::nvidia::omniverse::storage::fileobject::v1beta::*;

        #[derive(Debug, Clone)]
        pub enum ReadResponse {
            Direct(Metadata, Bytes),
            Redirect(Metadata, Redirect),
        }

        #[derive(Debug, Clone)]
        pub enum ReadFromAddressResponse {
            Direct(ResourceInfo, Bytes),
            Redirect(ResourceInfo, Redirect),
        }

        #[derive(Debug, Clone)]
        pub enum WriteResponse {
            Direct(ResourceInfo),
            Redirect(WriteRedirectProperties),
            Multipart(CreateMultipartUploadResponse),
        }

        pub trait UnwrapIdentityExt {
            fn unwrap_identity(self) -> String;
        }

        impl UnwrapIdentityExt for ResourceInfo {
            fn unwrap_identity(self) -> String {
                self.resource_identity
                    .expect("resource identity is missing")
                    .encoded_identity
            }
        }
    }
}

pub mod v1alpha {
    use bytes::Bytes;
    use futures::{stream::BoxStream, TryStreamExt};
    use reqwest::StatusCode;
    use tonic::Status;

    use crate::common::{
        clients::{
            fileobject::{grpc, rest, types::v1alpha::*},
            utils::Request,
        },
        random::get_random_bytes,
    };

    pub enum Client {
        Grpc(grpc::v1alpha::Client),
        Rest(rest::v1alpha::Client),
    }

    impl Client {
        pub async fn stat(&self, request: impl Into<Request<StatRequest>>) -> Result<StatResponse, Status> {
            match self {
                Self::Grpc(client) => client.stat(request.into()).await,
                Self::Rest(client) => client.stat(request.into()).await,
            }
        }

        pub async fn is_read_only(
            &self,
            request: impl Into<Request<IsReadOnlyRequest>>,
        ) -> Result<IsReadOnlyResponse, Status> {
            match self {
                Self::Grpc(client) => client.is_read_only(request.into()).await,
                Self::Rest(client) => client.is_read_only(request.into()).await,
            }
        }

        pub async fn read(&self, request: ReadRequest) -> Result<ReadResponse, Status> {
            match self {
                Self::Grpc(client) => client.read(request).await,
                Self::Rest(client) => client.read(request).await,
            }
        }

        pub async fn read_from_address(
            &self,
            request: ReadFromAddressRequest,
        ) -> Result<ReadFromAddressResponse, Status> {
            match self {
                Self::Grpc(client) => client.read_from_address(request).await,
                Self::Rest(client) => client.read_from_address(request).await,
            }
        }

        pub async fn write(&self, params: WriteParameters, content: impl Into<Bytes>) -> Result<WriteResponse, Status> {
            match self {
                Self::Grpc(client) => client.write(params, content.into()).await,
                Self::Rest(client) => client.write(params, content.into()).await,
            }
        }

        pub async fn upload_part(&self, request: UploadPartRequest) -> Result<UploadPartResponse, Status> {
            match self {
                Self::Grpc(client) => client.upload_part(request).await,
                Self::Rest(client) => client.upload_part(request).await,
            }
        }

        pub async fn complete_redirect_upload(
            &self,
            request: CompleteRedirectUploadRequest,
        ) -> Result<CompleteRedirectUploadResponse, Status> {
            match self {
                Self::Grpc(client) => client.complete_redirect_upload(request).await,
                Self::Rest(client) => client.complete_redirect_upload(request).await,
            }
        }

        pub async fn complete_multipart_upload(
            &self,
            request: CompleteMultipartUploadRequest,
        ) -> Result<CompleteMultipartUploadResponse, Status> {
            match self {
                Self::Grpc(client) => client.complete_multipart_upload(request).await,
                Self::Rest(client) => client.complete_multipart_upload(request).await,
            }
        }

        pub async fn abort_multipart_upload(
            &self,
            request: AbortMultipartUploadRequest,
        ) -> Result<AbortMultipartUploadResponse, Status> {
            match self {
                Self::Grpc(client) => client.abort_multipart_upload(request).await,
                Self::Rest(client) => client.abort_multipart_upload(request).await,
            }
        }

        pub async fn delete(&self, request: DeleteRequest) -> Result<DeleteResponse, Status> {
            match self {
                Self::Grpc(client) => client.delete(request).await,
                Self::Rest(client) => client.delete(request).await,
            }
        }

        pub async fn fetch_write_type_info(
            &self,
            request: FetchWriteTypeInfoRequest,
        ) -> Result<FetchWriteTypeInfoResponse, Status> {
            match self {
                Self::Grpc(client) => client.fetch_write_type_info(request).await,
                Self::Rest(client) => client.fetch_write_type_info(request).await,
            }
        }

        pub async fn copy(&self, request: CopyRequest) -> Result<CopyResponse, Status> {
            match self {
                Self::Grpc(client) => client.copy(request).await,
                Self::Rest(client) => client.copy(request).await,
            }
        }

        pub async fn move_object(&self, request: MoveRequest) -> Result<MoveResponse, Status> {
            match self {
                Self::Grpc(client) => client.move_object(request).await,
                Self::Rest(client) => client.move_object(request).await,
            }
        }

        pub fn enumerate(&self, request: EnumerateRequest) -> BoxStream<'static, Result<EnumerateResponse, Status>> {
            match self {
                Self::Grpc(client) => client.enumerate(request),
                Self::Rest(client) => client.enumerate(request),
            }
        }

        pub async fn read_object(&self, identity: impl Into<String>) -> Result<(Metadata, Bytes), Status> {
            let response = self
                .read(ReadRequest {
                    resource_identity: Some(ResourceIdentity {
                        encoded_identity: identity.into(),
                    }),
                    ..Default::default()
                })
                .await?;
            match response {
                ReadResponse::Direct(metadata, content) => Ok((metadata, content)),
                ReadResponse::Redirect(metadata, params) => {
                    let content = follow_read_redirect(&Default::default(), params).await?;
                    Ok((metadata, content))
                },
            }
        }

        pub async fn read_object_from_address(
            &self,
            address: impl Into<String>,
        ) -> Result<(ResourceInfo, Bytes), Status> {
            let response = self
                .read_from_address(ReadFromAddressRequest {
                    resource_address: address.into(),
                    ..Default::default()
                })
                .await?;
            match response {
                ReadFromAddressResponse::Direct(info, content) => Ok((info, content)),
                ReadFromAddressResponse::Redirect(info, params) => {
                    let content = follow_read_redirect(&Default::default(), params).await?;
                    Ok((info, content))
                },
            }
        }

        pub async fn write_random_object(
            &self,
            address: impl Into<String>,
            len: usize,
        ) -> Result<(ResourceInfo, Bytes), Status> {
            let content = get_random_bytes(len);
            let (resource_info, _) = self
                .write_object(
                    WriteParameters {
                        destination_resource_address: address.into(),
                        data_object_size: len as u64,
                        ..Default::default()
                    },
                    content.clone(),
                )
                .await?;

            Ok((resource_info, content))
        }

        pub async fn write_object(
            &self,
            params: WriteParameters,
            content: impl Into<Bytes>,
        ) -> Result<(ResourceInfo, UploadPreference), Status> {
            let content = content.into();
            let address = params.destination_resource_address.clone();
            let write_response = self.write(params, content.clone()).await?;

            match write_response {
                WriteResponse::Direct(resource_info) => Ok((resource_info, UploadPreference::Body)),
                WriteResponse::Redirect(params) => Ok((
                    self.write_via_redirect(address, content, params).await?,
                    UploadPreference::Redirect,
                )),
                WriteResponse::Multipart(params) => Ok((
                    self.write_via_multipart(address, content, params).await?,
                    UploadPreference::Multipart,
                )),
            }
        }

        pub async fn write_via_redirect(
            &self,
            address: impl Into<String>,
            content: impl Into<Bytes>,
            params: WriteRedirectProperties,
        ) -> Result<ResourceInfo, Status> {
            let completion_headers = follow_write_redirect(&reqwest::Client::new(), content, params).await?;
            let complete_response = self
                .complete_redirect_upload(CompleteRedirectUploadRequest {
                    destination_resource_address: address.into(),
                    additional_headers: completion_headers,
                })
                .await?;

            Ok(complete_response
                .resource_info
                .expect("resource info is missing from the response"))
        }

        pub async fn write_via_multipart(
            &self,
            address: impl Into<String>,
            content: impl Into<Bytes>,
            params: CreateMultipartUploadResponse,
        ) -> Result<ResourceInfo, Status> {
            let mut content = content.into();
            let address = address.into();
            let http_client = reqwest::Client::new();

            let mut completed_part_infos = Vec::new();
            let part_sizes = get_part_sizes(
                content.len(),
                params.minimum_size_per_part.unwrap_or(0) as usize,
                params.maximum_size_per_part.unwrap_or(u64::MAX) as usize,
            );
            for (idx, part_size) in part_sizes.into_iter().enumerate() {
                let params = self
                    .upload_part(UploadPartRequest {
                        upload_id: params.upload_id.clone(),
                        destination_resource_address: address.clone(),
                        part_number: idx as u32,
                        part_count: None,
                    })
                    .await?;

                let chunk = content.split_to(part_size);
                completed_part_infos.push(CompletedUploadPart {
                    part_number: idx as u32,
                    headers: follow_write_redirect(
                        &http_client,
                        chunk,
                        params
                            .part_write_redirects
                            .first()
                            .expect("part write redirects set is empty")
                            .clone(),
                    )
                    .await
                    .expect("part write failed"),
                });
            }

            let complete_response = self
                .complete_multipart_upload(CompleteMultipartUploadRequest {
                    upload_id: params.upload_id,
                    destination_resource_address: address,
                    parts: completed_part_infos,
                })
                .await?;

            Ok(complete_response.resource_info.expect("resource info is missing"))
        }

        pub async fn enumerate_collect(&self, request: EnumerateRequest) -> Result<EnumerateResponse, Status> {
            let mut response = EnumerateResponse::default();
            let mut responses = self.enumerate(request);
            while let Some(page) = responses.try_next().await? {
                response.items.extend(page.items);
            }

            Ok(response)
        }
    }

    pub async fn follow_read_redirect(client: &reqwest::Client, params: Redirect) -> Result<Bytes, Status> {
        let mut builder = client.get(params.redirect_target_url);
        for header in params.additional_headers.into_iter() {
            builder = builder.header(header.name, header.value);
        }

        let response = builder.send().await.expect("read redirect request failed");
        match response.status() {
            StatusCode::PRECONDITION_FAILED => Err(Status::failed_precondition("conditional http request failed")),
            StatusCode::NOT_FOUND => Err(Status::not_found("target resource was not found")),
            status if status.is_success() => Ok(response.bytes().await.expect("response read failed")),
            status => panic!("read redirect request failed with status {:?}", status),
        }
    }

    pub async fn follow_write_redirect(
        client: &reqwest::Client,
        content: impl Into<Bytes>,
        params: WriteRedirectProperties,
    ) -> Result<Vec<Header>, Status> {
        let mut builder = match params.method {
            m if m == UploadMethod::Put as i32 => client.put(params.redirect_target_url),
            m if m == UploadMethod::Post as i32 => client.post(params.redirect_target_url),
            m => panic!("unexpected upload method {:?}", m),
        };

        for header in params.additional_headers.into_iter() {
            builder = builder.header(header.name, header.value);
        }

        let response = builder.body(content.into()).send().await.expect("request failed");
        let status_code = response.status();
        if status_code == StatusCode::PRECONDITION_FAILED {
            return Err(Status::failed_precondition("conditional http request failed"));
        }

        if !status_code.is_success() {
            panic!("redirect write request failed with status {:?}", status_code);
        }

        Ok(params
            .completion_header_names
            .into_iter()
            .filter_map(|name| {
                response.headers().get(&name).map(|value| Header {
                    name,
                    value: value.to_str().expect("failed to decode header value").into(),
                })
            })
            .collect())
    }

    pub fn get_part_sizes(size: usize, min_part_size: usize, max_part_size: usize) -> Vec<usize> {
        let first_chunk = size.div_ceil(2);
        if first_chunk >= min_part_size && first_chunk <= max_part_size {
            vec![first_chunk, size - first_chunk]
        } else {
            vec![size]
        }
    }
}

pub mod v1beta {
    use bytes::Bytes;
    use futures::{stream::BoxStream, TryStreamExt};
    use reqwest::StatusCode;
    use tonic::Status;

    use crate::common::{
        clients::{
            fileobject::{grpc, rest, types::v1beta::*},
            utils::Request,
        },
        random::get_random_bytes,
    };

    pub enum Client {
        Grpc(grpc::v1beta::Client),
        Rest(rest::v1beta::Client),
    }

    impl Client {
        pub async fn stat(&self, request: impl Into<Request<StatRequest>>) -> Result<StatResponse, Status> {
            match self {
                Self::Grpc(client) => client.stat(request.into()).await,
                Self::Rest(client) => client.stat(request.into()).await,
            }
        }

        pub async fn read(&self, request: ReadRequest) -> Result<ReadResponse, Status> {
            match self {
                Self::Grpc(client) => client.read(request).await,
                Self::Rest(client) => client.read(request).await,
            }
        }

        pub async fn read_from_address(
            &self,
            request: ReadFromAddressRequest,
        ) -> Result<ReadFromAddressResponse, Status> {
            match self {
                Self::Grpc(client) => client.read_from_address(request).await,
                Self::Rest(client) => client.read_from_address(request).await,
            }
        }

        pub async fn write(&self, params: WriteParameters, content: impl Into<Bytes>) -> Result<WriteResponse, Status> {
            match self {
                Self::Grpc(client) => client.write(params, content.into()).await,
                Self::Rest(client) => client.write(params, content.into()).await,
            }
        }

        pub async fn upload_part(&self, request: UploadPartRequest) -> Result<UploadPartResponse, Status> {
            match self {
                Self::Grpc(client) => client.upload_part(request).await,
                Self::Rest(client) => client.upload_part(request).await,
            }
        }

        pub async fn complete_redirect_upload(
            &self,
            request: CompleteRedirectUploadRequest,
        ) -> Result<CompleteRedirectUploadResponse, Status> {
            match self {
                Self::Grpc(client) => client.complete_redirect_upload(request).await,
                Self::Rest(client) => client.complete_redirect_upload(request).await,
            }
        }

        pub async fn complete_multipart_upload(
            &self,
            request: CompleteMultipartUploadRequest,
        ) -> Result<CompleteMultipartUploadResponse, Status> {
            match self {
                Self::Grpc(client) => client.complete_multipart_upload(request).await,
                Self::Rest(client) => client.complete_multipart_upload(request).await,
            }
        }

        pub async fn abort_multipart_upload(
            &self,
            request: AbortMultipartUploadRequest,
        ) -> Result<AbortMultipartUploadResponse, Status> {
            match self {
                Self::Grpc(client) => client.abort_multipart_upload(request).await,
                Self::Rest(client) => client.abort_multipart_upload(request).await,
            }
        }

        pub async fn delete(&self, request: DeleteRequest) -> Result<DeleteResponse, Status> {
            match self {
                Self::Grpc(client) => client.delete(request).await,
                Self::Rest(client) => client.delete(request).await,
            }
        }

        pub async fn fetch_write_type_info(
            &self,
            request: FetchWriteTypeInfoRequest,
        ) -> Result<FetchWriteTypeInfoResponse, Status> {
            match self {
                Self::Grpc(client) => client.fetch_write_type_info(request).await,
                Self::Rest(client) => client.fetch_write_type_info(request).await,
            }
        }

        pub fn enumerate(&self, request: EnumerateRequest) -> BoxStream<'static, Result<EnumerateResponse, Status>> {
            match self {
                Self::Grpc(client) => client.enumerate(request),
                Self::Rest(client) => client.enumerate(request),
            }
        }

        pub async fn read_object(&self, identity: impl Into<String>) -> Result<(Metadata, Bytes), Status> {
            let response = self
                .read(ReadRequest {
                    resource_identity: Some(ResourceIdentity {
                        encoded_identity: identity.into(),
                    }),
                    ..Default::default()
                })
                .await?;
            match response {
                ReadResponse::Direct(metadata, content) => Ok((metadata, content)),
                ReadResponse::Redirect(metadata, params) => {
                    let content = follow_read_redirect(&Default::default(), params).await?;
                    Ok((metadata, content))
                },
            }
        }

        pub async fn read_object_from_address(
            &self,
            address: impl Into<String>,
        ) -> Result<(ResourceInfo, Bytes), Status> {
            let response = self
                .read_from_address(ReadFromAddressRequest {
                    resource_address: address.into(),
                    ..Default::default()
                })
                .await?;
            match response {
                ReadFromAddressResponse::Direct(info, content) => Ok((info, content)),
                ReadFromAddressResponse::Redirect(info, params) => {
                    let content = follow_read_redirect(&Default::default(), params).await?;
                    Ok((info, content))
                },
            }
        }

        pub async fn write_random_object(
            &self,
            address: impl Into<String>,
            len: usize,
        ) -> Result<(ResourceInfo, Bytes), Status> {
            let content = get_random_bytes(len);
            let (resource_info, _) = self
                .write_object(
                    WriteParameters {
                        destination_resource_address: address.into(),
                        data_object_size: len as u64,
                        ..Default::default()
                    },
                    content.clone(),
                )
                .await?;

            Ok((resource_info, content))
        }

        pub async fn write_object(
            &self,
            params: WriteParameters,
            content: impl Into<Bytes>,
        ) -> Result<(ResourceInfo, UploadPreference), Status> {
            let content = content.into();
            let address = params.destination_resource_address.clone();
            let write_response = self.write(params, content.clone()).await?;

            match write_response {
                WriteResponse::Direct(resource_info) => Ok((resource_info, UploadPreference::Body)),
                WriteResponse::Redirect(params) => Ok((
                    self.write_via_redirect(address, content, params).await?,
                    UploadPreference::Redirect,
                )),
                WriteResponse::Multipart(params) => Ok((
                    self.write_via_multipart(address, content, params).await?,
                    UploadPreference::Multipart,
                )),
            }
        }

        pub async fn write_via_redirect(
            &self,
            address: impl Into<String>,
            content: impl Into<Bytes>,
            params: WriteRedirectProperties,
        ) -> Result<ResourceInfo, Status> {
            let completion_headers = follow_write_redirect(&reqwest::Client::new(), content, params).await?;
            let complete_response = self
                .complete_redirect_upload(CompleteRedirectUploadRequest {
                    destination_resource_address: address.into(),
                    additional_headers: completion_headers,
                })
                .await?;

            Ok(complete_response
                .resource_info
                .expect("resource info is missing from the response"))
        }

        pub async fn write_via_multipart(
            &self,
            address: impl Into<String>,
            content: impl Into<Bytes>,
            params: CreateMultipartUploadResponse,
        ) -> Result<ResourceInfo, Status> {
            let mut content = content.into();
            let address = address.into();
            let http_client = reqwest::Client::new();

            let mut completed_part_infos = Vec::new();
            let part_sizes = get_part_sizes(
                content.len(),
                params.minimum_size_per_part.unwrap_or(0) as usize,
                params.maximum_size_per_part.unwrap_or(u64::MAX) as usize,
            );
            for (idx, part_size) in part_sizes.into_iter().enumerate() {
                let params = self
                    .upload_part(UploadPartRequest {
                        upload_id: params.upload_id.clone(),
                        destination_resource_address: address.clone(),
                        part_number: idx as u32,
                        part_count: None,
                    })
                    .await?;

                let chunk = content.split_to(part_size);
                completed_part_infos.push(CompletedUploadPart {
                    part_number: idx as u32,
                    headers: follow_write_redirect(
                        &http_client,
                        chunk,
                        params
                            .part_write_redirects
                            .first()
                            .expect("part write redirects set is empty")
                            .clone(),
                    )
                    .await
                    .expect("part write failed"),
                });
            }

            let complete_response = self
                .complete_multipart_upload(CompleteMultipartUploadRequest {
                    upload_id: params.upload_id,
                    destination_resource_address: address,
                    parts: completed_part_infos,
                })
                .await?;

            Ok(complete_response.resource_info.expect("resource info is missing"))
        }

        pub async fn enumerate_collect(&self, request: EnumerateRequest) -> Result<EnumerateResponse, Status> {
            let mut response = EnumerateResponse::default();
            let mut responses = self.enumerate(request);
            while let Some(page) = responses.try_next().await? {
                response.items.extend(page.items);
            }

            Ok(response)
        }
    }

    pub async fn follow_read_redirect(client: &reqwest::Client, params: Redirect) -> Result<Bytes, Status> {
        let mut builder = client.get(params.redirect_target_url);
        for header in params.additional_headers.into_iter() {
            builder = builder.header(header.name, header.value);
        }

        let response = builder.send().await.expect("read redirect request failed");
        match response.status() {
            StatusCode::PRECONDITION_FAILED => Err(Status::failed_precondition("conditional http request failed")),
            StatusCode::NOT_FOUND => Err(Status::not_found("target resource was not found")),
            status if status.is_success() => Ok(response.bytes().await.expect("response read failed")),
            status => panic!("read redirect request failed with status {:?}", status),
        }
    }

    pub async fn follow_write_redirect(
        client: &reqwest::Client,
        content: impl Into<Bytes>,
        params: WriteRedirectProperties,
    ) -> Result<Vec<Header>, Status> {
        let mut builder = match params.method {
            m if m == UploadMethod::Put as i32 => client.put(params.redirect_target_url),
            m if m == UploadMethod::Post as i32 => client.post(params.redirect_target_url),
            m => panic!("unexpected upload method {:?}", m),
        };

        for header in params.additional_headers.into_iter() {
            builder = builder.header(header.name, header.value);
        }

        let response = builder.body(content.into()).send().await.expect("request failed");
        let status_code = response.status();
        if status_code == StatusCode::PRECONDITION_FAILED {
            return Err(Status::failed_precondition("conditional http request failed"));
        }

        if !status_code.is_success() {
            panic!("redirect write request failed with status {:?}", status_code);
        }

        Ok(params
            .completion_header_names
            .into_iter()
            .filter_map(|name| {
                response.headers().get(&name).map(|value| Header {
                    name,
                    value: value.to_str().expect("failed to decode header value").into(),
                })
            })
            .collect())
    }

    pub fn get_part_sizes(size: usize, min_part_size: usize, max_part_size: usize) -> Vec<usize> {
        let first_chunk = size.div_ceil(2);
        if first_chunk >= min_part_size && first_chunk <= max_part_size {
            vec![first_chunk, size - first_chunk]
        } else {
            vec![size]
        }
    }
}
