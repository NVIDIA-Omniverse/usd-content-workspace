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
    use async_stream::{stream, try_stream};
    use bytes::{Bytes, BytesMut};
    use futures::{stream::BoxStream, TryStreamExt};
    use storage_core_protos::nvidia::omniverse::storage::fileobject::v1alpha::file_object_service_client::FileObjectServiceClient;
    use tonic::{transport::Channel, Request, Response, Status};

    use crate::common::clients::{fileobject::types::v1alpha::*, utils::grpc::request_with_headers};

    #[derive(Debug, Clone)]
    pub struct Client {
        client: FileObjectServiceClient<Channel>,
    }

    impl Client {
        pub fn new(channel: Channel) -> Self {
            Self {
                client: FileObjectServiceClient::new(channel),
            }
        }

        pub async fn stat(&self, request: impl Into<Request<StatRequest>>) -> Result<StatResponse, Status> {
            self.client.clone().stat(request.into()).await.map(Response::into_inner)
        }

        pub async fn is_read_only(
            &self,
            request: impl Into<Request<IsReadOnlyRequest>>,
        ) -> Result<IsReadOnlyResponse, Status> {
            self.client
                .clone()
                .is_read_only(request.into())
                .await
                .map(Response::into_inner)
        }

        pub async fn read(&self, request: ReadRequest) -> Result<ReadResponse, Status> {
            use storage_core_protos::nvidia::omniverse::storage::fileobject::v1alpha::read_response::ReplyType;

            let mut response_stream = self
                .client
                .clone()
                .read(request_with_headers(request))
                .await?
                .into_inner();
            let metadatza = response_stream
                .try_next()
                .await?
                .expect("unexpected end of response stream");
            let Some(ReplyType::Metadata(metadata)) = metadatza.reply_type else {
                panic!("metadata message is expected");
            };

            let chunk_or_redirect = response_stream
                .try_next()
                .await?
                .expect("unexpected end of response stream");
            let chunk = match chunk_or_redirect.reply_type {
                Some(ReplyType::Chunk(chunk)) => chunk.chunk,
                Some(ReplyType::Redirect(redirect)) => return Ok(ReadResponse::Redirect(metadata, redirect)),
                _ => panic!("either chunk or redirect message is expected"),
            };

            let mut content = BytesMut::new();
            content.extend_from_slice(&chunk);

            while let Some(message) = response_stream.try_next().await? {
                let Some(ReplyType::Chunk(chunk)) = message.reply_type else {
                    panic!("chunk message is expected");
                };

                content.extend_from_slice(&chunk.chunk);
            }

            Ok(ReadResponse::Direct(metadata, content.into()))
        }

        pub async fn read_from_address(
            &self,
            request: ReadFromAddressRequest,
        ) -> Result<ReadFromAddressResponse, Status> {
            use storage_core_protos::nvidia::omniverse::storage::fileobject::v1alpha::read_from_address_response::ReplyType;

            let mut response_stream = self
                .client
                .clone()
                .read_from_address(request_with_headers(request))
                .await?
                .into_inner();
            let metadatza = response_stream
                .try_next()
                .await?
                .expect("unexpected end of response stream");
            let Some(ReplyType::ResourceInfo(resource_info)) = metadatza.reply_type else {
                panic!("resource info message is expected");
            };

            let chunk_or_redirect = response_stream
                .try_next()
                .await?
                .expect("unexpected end of response stream");
            let chunk = match chunk_or_redirect.reply_type {
                Some(ReplyType::Chunk(chunk)) => chunk.chunk,
                Some(ReplyType::Redirect(redirect)) => {
                    return Ok(ReadFromAddressResponse::Redirect(resource_info, redirect));
                },
                _ => panic!("either chunk or redirect message is expected"),
            };

            let mut content = BytesMut::new();
            content.extend_from_slice(&chunk);

            while let Some(message) = response_stream.try_next().await? {
                let Some(ReplyType::Chunk(chunk)) = message.reply_type else {
                    panic!("chunk message is expected");
                };

                content.extend_from_slice(&chunk.chunk);
            }

            Ok(ReadFromAddressResponse::Direct(resource_info, content.into()))
        }

        pub async fn write(&self, params: WriteParameters, content: Bytes) -> Result<WriteResponse, Status> {
            use storage_core_protos::nvidia::omniverse::storage::fileobject::v1alpha::{
                write_request::WriteRequestType, write_response::WriteResponseType, Chunk, WriteRequest,
            };

            let request_stream = Box::pin(stream! {
                yield WriteRequest{
                    write_request_type: Some(WriteRequestType::Params(params)),
                };

                let mut content = content;
                while !content.is_empty() {
                    let chunk = content.split_to(content.len().min(3 * 1024 * 1024));
                    yield WriteRequest {
                        write_request_type: Some(WriteRequestType::Chunk(Chunk {
                            chunk
                        })),
                    };
                }
            });

            let mut response_stream = self
                .client
                .clone()
                .write(request_with_headers(request_stream))
                .await?
                .into_inner();
            let accept_or_redirect = response_stream
                .try_next()
                .await?
                .expect("unexpected end of response stream");
            match accept_or_redirect.write_response_type {
                Some(WriteResponseType::WriteChunksAccepted(_)) => {},
                Some(WriteResponseType::ResourceInfo(resource_info)) => {
                    return Ok(WriteResponse::Direct(resource_info));
                },
                Some(WriteResponseType::WriteRedirect(params)) => {
                    return Ok(WriteResponse::Redirect(params));
                },
                Some(WriteResponseType::MultipartUpload(params)) => {
                    return Ok(WriteResponse::Multipart(params));
                },
                _ => panic!("either accepted, redirect, or multipart upload message is expected"),
            }

            let resource_info = response_stream
                .try_next()
                .await?
                .expect("unexpected end of response stream");
            let Some(WriteResponseType::ResourceInfo(resource_info)) = resource_info.write_response_type else {
                panic!("resource info message is expected");
            };

            Ok(WriteResponse::Direct(resource_info))
        }

        pub async fn upload_part(&self, request: UploadPartRequest) -> Result<UploadPartResponse, Status> {
            self.client
                .clone()
                .upload_part(request_with_headers(request))
                .await
                .map(Response::into_inner)
        }

        pub async fn complete_redirect_upload(
            &self,
            request: CompleteRedirectUploadRequest,
        ) -> Result<CompleteRedirectUploadResponse, Status> {
            self.client
                .clone()
                .complete_redirect_upload(request_with_headers(request))
                .await
                .map(Response::into_inner)
        }

        pub async fn complete_multipart_upload(
            &self,
            request: CompleteMultipartUploadRequest,
        ) -> Result<CompleteMultipartUploadResponse, Status> {
            self.client
                .clone()
                .complete_multipart_upload(request_with_headers(request))
                .await
                .map(Response::into_inner)
        }

        pub async fn abort_multipart_upload(
            &self,
            request: AbortMultipartUploadRequest,
        ) -> Result<AbortMultipartUploadResponse, Status> {
            self.client
                .clone()
                .abort_multipart_upload(request_with_headers(request))
                .await
                .map(Response::into_inner)
        }

        pub async fn delete(&self, request: DeleteRequest) -> Result<DeleteResponse, Status> {
            self.client
                .clone()
                .delete(request_with_headers(request))
                .await
                .map(Response::into_inner)
        }

        pub async fn fetch_write_type_info(
            &self,
            request: FetchWriteTypeInfoRequest,
        ) -> Result<FetchWriteTypeInfoResponse, Status> {
            self.client
                .clone()
                .fetch_write_type_info(request_with_headers(request))
                .await
                .map(Response::into_inner)
        }

        pub async fn copy(&self, request: CopyRequest) -> Result<CopyResponse, Status> {
            self.client
                .clone()
                .copy(request_with_headers(request))
                .await
                .map(Response::into_inner)
        }

        pub async fn move_object(&self, request: MoveRequest) -> Result<MoveResponse, Status> {
            self.client
                .clone()
                .r#move(request_with_headers(request))
                .await
                .map(Response::into_inner)
        }

        pub fn enumerate(&self, request: EnumerateRequest) -> BoxStream<'static, Result<EnumerateResponse, Status>> {
            let mut client = self.client.clone();
            Box::pin(try_stream! {
                let mut responses = client.enumerate(request_with_headers(request)).await?.into_inner();
                while let Some(response) = responses.try_next().await? {
                    yield response;
                }
            })
        }
    }
}

pub mod v1beta {
    use async_stream::{stream, try_stream};
    use bytes::{Bytes, BytesMut};
    use futures::{stream::BoxStream, TryStreamExt};
    use storage_core_protos::nvidia::omniverse::storage::fileobject::v1beta::file_object_service_client::FileObjectServiceClient;
    use tonic::{transport::Channel, Request, Response, Status};

    use crate::common::clients::{fileobject::types::v1beta::*, utils::grpc::request_with_headers};

    #[derive(Debug, Clone)]
    pub struct Client {
        client: FileObjectServiceClient<Channel>,
    }

    impl Client {
        pub fn new(channel: Channel) -> Self {
            Self {
                client: FileObjectServiceClient::new(channel),
            }
        }

        pub async fn stat(&self, request: impl Into<Request<StatRequest>>) -> Result<StatResponse, Status> {
            self.client.clone().stat(request.into()).await.map(Response::into_inner)
        }

        pub async fn read(&self, request: ReadRequest) -> Result<ReadResponse, Status> {
            use storage_core_protos::nvidia::omniverse::storage::fileobject::v1beta::read_response::ReplyType;

            let mut response_stream = self
                .client
                .clone()
                .read(request_with_headers(request))
                .await?
                .into_inner();
            let metadatza = response_stream
                .try_next()
                .await?
                .expect("unexpected end of response stream");
            let Some(ReplyType::Metadata(metadata)) = metadatza.reply_type else {
                panic!("metadata message is expected");
            };

            let chunk_or_redirect = response_stream
                .try_next()
                .await?
                .expect("unexpected end of response stream");
            let chunk = match chunk_or_redirect.reply_type {
                Some(ReplyType::Chunk(chunk)) => chunk.chunk,
                Some(ReplyType::Redirect(redirect)) => return Ok(ReadResponse::Redirect(metadata, redirect)),
                _ => panic!("either chunk or redirect message is expected"),
            };

            let mut content = BytesMut::new();
            content.extend_from_slice(&chunk);

            while let Some(message) = response_stream.try_next().await? {
                let Some(ReplyType::Chunk(chunk)) = message.reply_type else {
                    panic!("chunk message is expected");
                };

                content.extend_from_slice(&chunk.chunk);
            }

            Ok(ReadResponse::Direct(metadata, content.into()))
        }

        pub async fn read_from_address(
            &self,
            request: ReadFromAddressRequest,
        ) -> Result<ReadFromAddressResponse, Status> {
            use storage_core_protos::nvidia::omniverse::storage::fileobject::v1beta::read_from_address_response::ReplyType;

            let mut response_stream = self
                .client
                .clone()
                .read_from_address(request_with_headers(request))
                .await?
                .into_inner();
            let metadatza = response_stream
                .try_next()
                .await?
                .expect("unexpected end of response stream");
            let Some(ReplyType::ResourceInfo(resource_info)) = metadatza.reply_type else {
                panic!("resource info message is expected");
            };

            let chunk_or_redirect = response_stream
                .try_next()
                .await?
                .expect("unexpected end of response stream");
            let chunk = match chunk_or_redirect.reply_type {
                Some(ReplyType::Chunk(chunk)) => chunk.chunk,
                Some(ReplyType::Redirect(redirect)) => {
                    return Ok(ReadFromAddressResponse::Redirect(resource_info, redirect));
                },
                _ => panic!("either chunk or redirect message is expected"),
            };

            let mut content = BytesMut::new();
            content.extend_from_slice(&chunk);

            while let Some(message) = response_stream.try_next().await? {
                let Some(ReplyType::Chunk(chunk)) = message.reply_type else {
                    panic!("chunk message is expected");
                };

                content.extend_from_slice(&chunk.chunk);
            }

            Ok(ReadFromAddressResponse::Direct(resource_info, content.into()))
        }

        pub async fn write(&self, params: WriteParameters, content: Bytes) -> Result<WriteResponse, Status> {
            use storage_core_protos::nvidia::omniverse::storage::fileobject::v1beta::{
                write_request::WriteRequestType, write_response::WriteResponseType, Chunk, WriteRequest,
            };

            let request_stream = Box::pin(stream! {
                yield WriteRequest{
                    write_request_type: Some(WriteRequestType::Params(params)),
                };

                let mut content = content;
                while !content.is_empty() {
                    let chunk = content.split_to(content.len().min(3 * 1024 * 1024));
                    yield WriteRequest {
                        write_request_type: Some(WriteRequestType::Chunk(Chunk {
                            chunk
                        })),
                    };
                }
            });

            let mut response_stream = self
                .client
                .clone()
                .write(request_with_headers(request_stream))
                .await?
                .into_inner();
            let accept_or_redirect = response_stream
                .try_next()
                .await?
                .expect("unexpected end of response stream");
            match accept_or_redirect.write_response_type {
                Some(WriteResponseType::WriteChunksAccepted(_)) => {},
                Some(WriteResponseType::ResourceInfo(resource_info)) => {
                    return Ok(WriteResponse::Direct(resource_info));
                },
                Some(WriteResponseType::WriteRedirect(params)) => {
                    return Ok(WriteResponse::Redirect(params));
                },
                Some(WriteResponseType::MultipartUpload(params)) => {
                    return Ok(WriteResponse::Multipart(params));
                },
                _ => panic!("either accepted, redirect, or multipart upload message is expected"),
            }

            let resource_info = response_stream
                .try_next()
                .await?
                .expect("unexpected end of response stream");
            let Some(WriteResponseType::ResourceInfo(resource_info)) = resource_info.write_response_type else {
                panic!("resource info message is expected");
            };

            Ok(WriteResponse::Direct(resource_info))
        }

        pub async fn upload_part(&self, request: UploadPartRequest) -> Result<UploadPartResponse, Status> {
            self.client
                .clone()
                .upload_part(request_with_headers(request))
                .await
                .map(Response::into_inner)
        }

        pub async fn complete_redirect_upload(
            &self,
            request: CompleteRedirectUploadRequest,
        ) -> Result<CompleteRedirectUploadResponse, Status> {
            self.client
                .clone()
                .complete_redirect_upload(request_with_headers(request))
                .await
                .map(Response::into_inner)
        }

        pub async fn complete_multipart_upload(
            &self,
            request: CompleteMultipartUploadRequest,
        ) -> Result<CompleteMultipartUploadResponse, Status> {
            self.client
                .clone()
                .complete_multipart_upload(request_with_headers(request))
                .await
                .map(Response::into_inner)
        }

        pub async fn abort_multipart_upload(
            &self,
            request: AbortMultipartUploadRequest,
        ) -> Result<AbortMultipartUploadResponse, Status> {
            self.client
                .clone()
                .abort_multipart_upload(request_with_headers(request))
                .await
                .map(Response::into_inner)
        }

        pub async fn delete(&self, request: DeleteRequest) -> Result<DeleteResponse, Status> {
            self.client
                .clone()
                .delete(request_with_headers(request))
                .await
                .map(Response::into_inner)
        }

        pub async fn fetch_write_type_info(
            &self,
            request: FetchWriteTypeInfoRequest,
        ) -> Result<FetchWriteTypeInfoResponse, Status> {
            self.client
                .clone()
                .fetch_write_type_info(request_with_headers(request))
                .await
                .map(Response::into_inner)
        }

        pub fn enumerate(&self, request: EnumerateRequest) -> BoxStream<'static, Result<EnumerateResponse, Status>> {
            let mut client = self.client.clone();
            Box::pin(try_stream! {
                let mut responses = client.enumerate(request_with_headers(request)).await?.into_inner();
                while let Some(response) = responses.try_next().await? {
                    yield response;
                }
            })
        }
    }
}
