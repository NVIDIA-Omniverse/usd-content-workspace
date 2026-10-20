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

// @generated
/// Generated client implementations.
pub mod file_object_service_client {
    #![allow(
        unused_variables,
        dead_code,
        missing_docs,
        clippy::wildcard_imports,
        clippy::let_unit_value,
    )]
    use tonic::codegen::*;
    use tonic::codegen::http::Uri;
    #[derive(Debug, Clone)]
    pub struct FileObjectServiceClient<T> {
        inner: tonic::client::Grpc<T>,
    }
    impl FileObjectServiceClient<tonic::transport::Channel> {
        /// Attempt to create a new client by connecting to a given endpoint.
        pub async fn connect<D>(dst: D) -> Result<Self, tonic::transport::Error>
        where
            D: TryInto<tonic::transport::Endpoint>,
            D::Error: Into<StdError>,
        {
            let conn = tonic::transport::Endpoint::new(dst)?.connect().await?;
            Ok(Self::new(conn))
        }
    }
    impl<T> FileObjectServiceClient<T>
    where
        T: tonic::client::GrpcService<tonic::body::Body>,
        T::Error: Into<StdError>,
        T::ResponseBody: Body<Data = Bytes> + std::marker::Send + 'static,
        <T::ResponseBody as Body>::Error: Into<StdError> + std::marker::Send,
    {
        pub fn new(inner: T) -> Self {
            let inner = tonic::client::Grpc::new(inner);
            Self { inner }
        }
        pub fn with_origin(inner: T, origin: Uri) -> Self {
            let inner = tonic::client::Grpc::with_origin(inner, origin);
            Self { inner }
        }
        pub fn with_interceptor<F>(
            inner: T,
            interceptor: F,
        ) -> FileObjectServiceClient<InterceptedService<T, F>>
        where
            F: tonic::service::Interceptor,
            T::ResponseBody: Default,
            T: tonic::codegen::Service<
                http::Request<tonic::body::Body>,
                Response = http::Response<
                    <T as tonic::client::GrpcService<tonic::body::Body>>::ResponseBody,
                >,
            >,
            <T as tonic::codegen::Service<
                http::Request<tonic::body::Body>,
            >>::Error: Into<StdError> + std::marker::Send + std::marker::Sync,
        {
            FileObjectServiceClient::new(InterceptedService::new(inner, interceptor))
        }
        /// Compress requests with the given encoding.
        ///
        /// This requires the server to support it otherwise it might respond with an
        /// error.
        #[must_use]
        pub fn send_compressed(mut self, encoding: CompressionEncoding) -> Self {
            self.inner = self.inner.send_compressed(encoding);
            self
        }
        /// Enable decompressing responses.
        #[must_use]
        pub fn accept_compressed(mut self, encoding: CompressionEncoding) -> Self {
            self.inner = self.inner.accept_compressed(encoding);
            self
        }
        /// Limits the maximum size of a decoded message.
        ///
        /// Default: `4MB`
        #[must_use]
        pub fn max_decoding_message_size(mut self, limit: usize) -> Self {
            self.inner = self.inner.max_decoding_message_size(limit);
            self
        }
        /// Limits the maximum size of an encoded message.
        ///
        /// Default: `usize::MAX`
        #[must_use]
        pub fn max_encoding_message_size(mut self, limit: usize) -> Self {
            self.inner = self.inner.max_encoding_message_size(limit);
            self
        }
        pub async fn enumerate(
            &mut self,
            request: impl tonic::IntoRequest<super::EnumerateRequest>,
        ) -> std::result::Result<
            tonic::Response<tonic::codec::Streaming<super::EnumerateResponse>>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService/Enumerate",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService",
                        "Enumerate",
                    ),
                );
            self.inner.server_streaming(req, path, codec).await
        }
        pub async fn stat(
            &mut self,
            request: impl tonic::IntoRequest<super::StatRequest>,
        ) -> std::result::Result<tonic::Response<super::StatResponse>, tonic::Status> {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService/Stat",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService",
                        "Stat",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        pub async fn read(
            &mut self,
            request: impl tonic::IntoRequest<super::ReadRequest>,
        ) -> std::result::Result<
            tonic::Response<tonic::codec::Streaming<super::ReadResponse>>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService/Read",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService",
                        "Read",
                    ),
                );
            self.inner.server_streaming(req, path, codec).await
        }
        pub async fn read_from_address(
            &mut self,
            request: impl tonic::IntoRequest<super::ReadFromAddressRequest>,
        ) -> std::result::Result<
            tonic::Response<tonic::codec::Streaming<super::ReadFromAddressResponse>>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService/ReadFromAddress",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService",
                        "ReadFromAddress",
                    ),
                );
            self.inner.server_streaming(req, path, codec).await
        }
        pub async fn fetch_write_type_info(
            &mut self,
            request: impl tonic::IntoRequest<super::FetchWriteTypeInfoRequest>,
        ) -> std::result::Result<
            tonic::Response<super::FetchWriteTypeInfoResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService/FetchWriteTypeInfo",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService",
                        "FetchWriteTypeInfo",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        pub async fn get_native_metadata_support(
            &mut self,
            request: impl tonic::IntoRequest<super::GetNativeMetadataSupportRequest>,
        ) -> std::result::Result<
            tonic::Response<super::GetNativeMetadataSupportResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService/GetNativeMetadataSupport",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService",
                        "GetNativeMetadataSupport",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        pub async fn write(
            &mut self,
            request: impl tonic::IntoStreamingRequest<Message = super::WriteRequest>,
        ) -> std::result::Result<
            tonic::Response<tonic::codec::Streaming<super::WriteResponse>>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService/Write",
            );
            let mut req = request.into_streaming_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService",
                        "Write",
                    ),
                );
            self.inner.streaming(req, path, codec).await
        }
        pub async fn complete_redirect_upload(
            &mut self,
            request: impl tonic::IntoRequest<super::CompleteRedirectUploadRequest>,
        ) -> std::result::Result<
            tonic::Response<super::CompleteRedirectUploadResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService/CompleteRedirectUpload",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService",
                        "CompleteRedirectUpload",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        pub async fn upload_part(
            &mut self,
            request: impl tonic::IntoRequest<super::UploadPartRequest>,
        ) -> std::result::Result<
            tonic::Response<super::UploadPartResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService/UploadPart",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService",
                        "UploadPart",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        pub async fn complete_multipart_upload(
            &mut self,
            request: impl tonic::IntoRequest<super::CompleteMultipartUploadRequest>,
        ) -> std::result::Result<
            tonic::Response<super::CompleteMultipartUploadResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService/CompleteMultipartUpload",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService",
                        "CompleteMultipartUpload",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        pub async fn abort_multipart_upload(
            &mut self,
            request: impl tonic::IntoRequest<super::AbortMultipartUploadRequest>,
        ) -> std::result::Result<
            tonic::Response<super::AbortMultipartUploadResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService/AbortMultipartUpload",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService",
                        "AbortMultipartUpload",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        pub async fn delete(
            &mut self,
            request: impl tonic::IntoRequest<super::DeleteRequest>,
        ) -> std::result::Result<tonic::Response<super::DeleteResponse>, tonic::Status> {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService/Delete",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService",
                        "Delete",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        pub async fn copy(
            &mut self,
            request: impl tonic::IntoRequest<super::CopyRequest>,
        ) -> std::result::Result<tonic::Response<super::CopyResponse>, tonic::Status> {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService/Copy",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService",
                        "Copy",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        pub async fn r#move(
            &mut self,
            request: impl tonic::IntoRequest<super::MoveRequest>,
        ) -> std::result::Result<tonic::Response<super::MoveResponse>, tonic::Status> {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService/Move",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService",
                        "Move",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        pub async fn get_optimistic_locking_support(
            &mut self,
            request: impl tonic::IntoRequest<super::GetOptimisticLockingSupportRequest>,
        ) -> std::result::Result<
            tonic::Response<super::GetOptimisticLockingSupportResponse>,
            tonic::Status,
        > {
            self.inner
                .ready()
                .await
                .map_err(|e| {
                    tonic::Status::unknown(
                        format!("Service was not ready: {}", e.into()),
                    )
                })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(
                "/nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService/GetOptimisticLockingSupport",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService",
                        "GetOptimisticLockingSupport",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
    }
}
/// Generated server implementations.
pub mod file_object_service_server {
    #![allow(
        unused_variables,
        dead_code,
        missing_docs,
        clippy::wildcard_imports,
        clippy::let_unit_value,
    )]
    use tonic::codegen::*;
    /// Generated trait containing gRPC methods that should be implemented for use with FileObjectServiceServer.
    #[async_trait]
    pub trait FileObjectService: std::marker::Send + std::marker::Sync + 'static {
        /// Server streaming response type for the Enumerate method.
        type EnumerateStream: tonic::codegen::tokio_stream::Stream<
                Item = std::result::Result<super::EnumerateResponse, tonic::Status>,
            >
            + std::marker::Send
            + 'static;
        async fn enumerate(
            &self,
            request: tonic::Request<super::EnumerateRequest>,
        ) -> std::result::Result<tonic::Response<Self::EnumerateStream>, tonic::Status>;
        async fn stat(
            &self,
            request: tonic::Request<super::StatRequest>,
        ) -> std::result::Result<tonic::Response<super::StatResponse>, tonic::Status>;
        /// Server streaming response type for the Read method.
        type ReadStream: tonic::codegen::tokio_stream::Stream<
                Item = std::result::Result<super::ReadResponse, tonic::Status>,
            >
            + std::marker::Send
            + 'static;
        async fn read(
            &self,
            request: tonic::Request<super::ReadRequest>,
        ) -> std::result::Result<tonic::Response<Self::ReadStream>, tonic::Status>;
        /// Server streaming response type for the ReadFromAddress method.
        type ReadFromAddressStream: tonic::codegen::tokio_stream::Stream<
                Item = std::result::Result<super::ReadFromAddressResponse, tonic::Status>,
            >
            + std::marker::Send
            + 'static;
        async fn read_from_address(
            &self,
            request: tonic::Request<super::ReadFromAddressRequest>,
        ) -> std::result::Result<
            tonic::Response<Self::ReadFromAddressStream>,
            tonic::Status,
        >;
        async fn fetch_write_type_info(
            &self,
            request: tonic::Request<super::FetchWriteTypeInfoRequest>,
        ) -> std::result::Result<
            tonic::Response<super::FetchWriteTypeInfoResponse>,
            tonic::Status,
        >;
        async fn get_native_metadata_support(
            &self,
            request: tonic::Request<super::GetNativeMetadataSupportRequest>,
        ) -> std::result::Result<
            tonic::Response<super::GetNativeMetadataSupportResponse>,
            tonic::Status,
        >;
        /// Server streaming response type for the Write method.
        type WriteStream: tonic::codegen::tokio_stream::Stream<
                Item = std::result::Result<super::WriteResponse, tonic::Status>,
            >
            + std::marker::Send
            + 'static;
        async fn write(
            &self,
            request: tonic::Request<tonic::Streaming<super::WriteRequest>>,
        ) -> std::result::Result<tonic::Response<Self::WriteStream>, tonic::Status>;
        async fn complete_redirect_upload(
            &self,
            request: tonic::Request<super::CompleteRedirectUploadRequest>,
        ) -> std::result::Result<
            tonic::Response<super::CompleteRedirectUploadResponse>,
            tonic::Status,
        >;
        async fn upload_part(
            &self,
            request: tonic::Request<super::UploadPartRequest>,
        ) -> std::result::Result<
            tonic::Response<super::UploadPartResponse>,
            tonic::Status,
        >;
        async fn complete_multipart_upload(
            &self,
            request: tonic::Request<super::CompleteMultipartUploadRequest>,
        ) -> std::result::Result<
            tonic::Response<super::CompleteMultipartUploadResponse>,
            tonic::Status,
        >;
        async fn abort_multipart_upload(
            &self,
            request: tonic::Request<super::AbortMultipartUploadRequest>,
        ) -> std::result::Result<
            tonic::Response<super::AbortMultipartUploadResponse>,
            tonic::Status,
        >;
        async fn delete(
            &self,
            request: tonic::Request<super::DeleteRequest>,
        ) -> std::result::Result<tonic::Response<super::DeleteResponse>, tonic::Status>;
        async fn copy(
            &self,
            request: tonic::Request<super::CopyRequest>,
        ) -> std::result::Result<tonic::Response<super::CopyResponse>, tonic::Status>;
        async fn r#move(
            &self,
            request: tonic::Request<super::MoveRequest>,
        ) -> std::result::Result<tonic::Response<super::MoveResponse>, tonic::Status>;
        async fn get_optimistic_locking_support(
            &self,
            request: tonic::Request<super::GetOptimisticLockingSupportRequest>,
        ) -> std::result::Result<
            tonic::Response<super::GetOptimisticLockingSupportResponse>,
            tonic::Status,
        >;
    }
    #[derive(Debug)]
    pub struct FileObjectServiceServer<T> {
        inner: Arc<T>,
        accept_compression_encodings: EnabledCompressionEncodings,
        send_compression_encodings: EnabledCompressionEncodings,
        max_decoding_message_size: Option<usize>,
        max_encoding_message_size: Option<usize>,
    }
    impl<T> FileObjectServiceServer<T> {
        pub fn new(inner: T) -> Self {
            Self::from_arc(Arc::new(inner))
        }
        pub fn from_arc(inner: Arc<T>) -> Self {
            Self {
                inner,
                accept_compression_encodings: Default::default(),
                send_compression_encodings: Default::default(),
                max_decoding_message_size: None,
                max_encoding_message_size: None,
            }
        }
        pub fn with_interceptor<F>(
            inner: T,
            interceptor: F,
        ) -> InterceptedService<Self, F>
        where
            F: tonic::service::Interceptor,
        {
            InterceptedService::new(Self::new(inner), interceptor)
        }
        /// Enable decompressing requests with the given encoding.
        #[must_use]
        pub fn accept_compressed(mut self, encoding: CompressionEncoding) -> Self {
            self.accept_compression_encodings.enable(encoding);
            self
        }
        /// Compress responses with the given encoding, if the client supports it.
        #[must_use]
        pub fn send_compressed(mut self, encoding: CompressionEncoding) -> Self {
            self.send_compression_encodings.enable(encoding);
            self
        }
        /// Limits the maximum size of a decoded message.
        ///
        /// Default: `4MB`
        #[must_use]
        pub fn max_decoding_message_size(mut self, limit: usize) -> Self {
            self.max_decoding_message_size = Some(limit);
            self
        }
        /// Limits the maximum size of an encoded message.
        ///
        /// Default: `usize::MAX`
        #[must_use]
        pub fn max_encoding_message_size(mut self, limit: usize) -> Self {
            self.max_encoding_message_size = Some(limit);
            self
        }
    }
    impl<T, B> tonic::codegen::Service<http::Request<B>> for FileObjectServiceServer<T>
    where
        T: FileObjectService,
        B: Body + std::marker::Send + 'static,
        B::Error: Into<StdError> + std::marker::Send + 'static,
    {
        type Response = http::Response<tonic::body::Body>;
        type Error = std::convert::Infallible;
        type Future = BoxFuture<Self::Response, Self::Error>;
        fn poll_ready(
            &mut self,
            _cx: &mut Context<'_>,
        ) -> Poll<std::result::Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
        fn call(&mut self, req: http::Request<B>) -> Self::Future {
            match req.uri().path() {
                "/nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService/Enumerate" => {
                    #[allow(non_camel_case_types)]
                    struct EnumerateSvc<T: FileObjectService>(pub Arc<T>);
                    impl<
                        T: FileObjectService,
                    > tonic::server::ServerStreamingService<super::EnumerateRequest>
                    for EnumerateSvc<T> {
                        type Response = super::EnumerateResponse;
                        type ResponseStream = T::EnumerateStream;
                        type Future = BoxFuture<
                            tonic::Response<Self::ResponseStream>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::EnumerateRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as FileObjectService>::enumerate(&inner, request).await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = EnumerateSvc(inner);
                        let codec = tonic_prost::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.server_streaming(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService/Stat" => {
                    #[allow(non_camel_case_types)]
                    struct StatSvc<T: FileObjectService>(pub Arc<T>);
                    impl<
                        T: FileObjectService,
                    > tonic::server::UnaryService<super::StatRequest> for StatSvc<T> {
                        type Response = super::StatResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::StatRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as FileObjectService>::stat(&inner, request).await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = StatSvc(inner);
                        let codec = tonic_prost::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService/Read" => {
                    #[allow(non_camel_case_types)]
                    struct ReadSvc<T: FileObjectService>(pub Arc<T>);
                    impl<
                        T: FileObjectService,
                    > tonic::server::ServerStreamingService<super::ReadRequest>
                    for ReadSvc<T> {
                        type Response = super::ReadResponse;
                        type ResponseStream = T::ReadStream;
                        type Future = BoxFuture<
                            tonic::Response<Self::ResponseStream>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::ReadRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as FileObjectService>::read(&inner, request).await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = ReadSvc(inner);
                        let codec = tonic_prost::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.server_streaming(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService/ReadFromAddress" => {
                    #[allow(non_camel_case_types)]
                    struct ReadFromAddressSvc<T: FileObjectService>(pub Arc<T>);
                    impl<
                        T: FileObjectService,
                    > tonic::server::ServerStreamingService<
                        super::ReadFromAddressRequest,
                    > for ReadFromAddressSvc<T> {
                        type Response = super::ReadFromAddressResponse;
                        type ResponseStream = T::ReadFromAddressStream;
                        type Future = BoxFuture<
                            tonic::Response<Self::ResponseStream>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::ReadFromAddressRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as FileObjectService>::read_from_address(&inner, request)
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = ReadFromAddressSvc(inner);
                        let codec = tonic_prost::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.server_streaming(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService/FetchWriteTypeInfo" => {
                    #[allow(non_camel_case_types)]
                    struct FetchWriteTypeInfoSvc<T: FileObjectService>(pub Arc<T>);
                    impl<
                        T: FileObjectService,
                    > tonic::server::UnaryService<super::FetchWriteTypeInfoRequest>
                    for FetchWriteTypeInfoSvc<T> {
                        type Response = super::FetchWriteTypeInfoResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::FetchWriteTypeInfoRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as FileObjectService>::fetch_write_type_info(
                                        &inner,
                                        request,
                                    )
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = FetchWriteTypeInfoSvc(inner);
                        let codec = tonic_prost::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService/GetNativeMetadataSupport" => {
                    #[allow(non_camel_case_types)]
                    struct GetNativeMetadataSupportSvc<T: FileObjectService>(pub Arc<T>);
                    impl<
                        T: FileObjectService,
                    > tonic::server::UnaryService<super::GetNativeMetadataSupportRequest>
                    for GetNativeMetadataSupportSvc<T> {
                        type Response = super::GetNativeMetadataSupportResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<
                                super::GetNativeMetadataSupportRequest,
                            >,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as FileObjectService>::get_native_metadata_support(
                                        &inner,
                                        request,
                                    )
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = GetNativeMetadataSupportSvc(inner);
                        let codec = tonic_prost::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService/Write" => {
                    #[allow(non_camel_case_types)]
                    struct WriteSvc<T: FileObjectService>(pub Arc<T>);
                    impl<
                        T: FileObjectService,
                    > tonic::server::StreamingService<super::WriteRequest>
                    for WriteSvc<T> {
                        type Response = super::WriteResponse;
                        type ResponseStream = T::WriteStream;
                        type Future = BoxFuture<
                            tonic::Response<Self::ResponseStream>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<
                                tonic::Streaming<super::WriteRequest>,
                            >,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as FileObjectService>::write(&inner, request).await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = WriteSvc(inner);
                        let codec = tonic_prost::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.streaming(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService/CompleteRedirectUpload" => {
                    #[allow(non_camel_case_types)]
                    struct CompleteRedirectUploadSvc<T: FileObjectService>(pub Arc<T>);
                    impl<
                        T: FileObjectService,
                    > tonic::server::UnaryService<super::CompleteRedirectUploadRequest>
                    for CompleteRedirectUploadSvc<T> {
                        type Response = super::CompleteRedirectUploadResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::CompleteRedirectUploadRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as FileObjectService>::complete_redirect_upload(
                                        &inner,
                                        request,
                                    )
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = CompleteRedirectUploadSvc(inner);
                        let codec = tonic_prost::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService/UploadPart" => {
                    #[allow(non_camel_case_types)]
                    struct UploadPartSvc<T: FileObjectService>(pub Arc<T>);
                    impl<
                        T: FileObjectService,
                    > tonic::server::UnaryService<super::UploadPartRequest>
                    for UploadPartSvc<T> {
                        type Response = super::UploadPartResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::UploadPartRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as FileObjectService>::upload_part(&inner, request).await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = UploadPartSvc(inner);
                        let codec = tonic_prost::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService/CompleteMultipartUpload" => {
                    #[allow(non_camel_case_types)]
                    struct CompleteMultipartUploadSvc<T: FileObjectService>(pub Arc<T>);
                    impl<
                        T: FileObjectService,
                    > tonic::server::UnaryService<super::CompleteMultipartUploadRequest>
                    for CompleteMultipartUploadSvc<T> {
                        type Response = super::CompleteMultipartUploadResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<
                                super::CompleteMultipartUploadRequest,
                            >,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as FileObjectService>::complete_multipart_upload(
                                        &inner,
                                        request,
                                    )
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = CompleteMultipartUploadSvc(inner);
                        let codec = tonic_prost::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService/AbortMultipartUpload" => {
                    #[allow(non_camel_case_types)]
                    struct AbortMultipartUploadSvc<T: FileObjectService>(pub Arc<T>);
                    impl<
                        T: FileObjectService,
                    > tonic::server::UnaryService<super::AbortMultipartUploadRequest>
                    for AbortMultipartUploadSvc<T> {
                        type Response = super::AbortMultipartUploadResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::AbortMultipartUploadRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as FileObjectService>::abort_multipart_upload(
                                        &inner,
                                        request,
                                    )
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = AbortMultipartUploadSvc(inner);
                        let codec = tonic_prost::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService/Delete" => {
                    #[allow(non_camel_case_types)]
                    struct DeleteSvc<T: FileObjectService>(pub Arc<T>);
                    impl<
                        T: FileObjectService,
                    > tonic::server::UnaryService<super::DeleteRequest>
                    for DeleteSvc<T> {
                        type Response = super::DeleteResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::DeleteRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as FileObjectService>::delete(&inner, request).await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = DeleteSvc(inner);
                        let codec = tonic_prost::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService/Copy" => {
                    #[allow(non_camel_case_types)]
                    struct CopySvc<T: FileObjectService>(pub Arc<T>);
                    impl<
                        T: FileObjectService,
                    > tonic::server::UnaryService<super::CopyRequest> for CopySvc<T> {
                        type Response = super::CopyResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::CopyRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as FileObjectService>::copy(&inner, request).await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = CopySvc(inner);
                        let codec = tonic_prost::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService/Move" => {
                    #[allow(non_camel_case_types)]
                    struct MoveSvc<T: FileObjectService>(pub Arc<T>);
                    impl<
                        T: FileObjectService,
                    > tonic::server::UnaryService<super::MoveRequest> for MoveSvc<T> {
                        type Response = super::MoveResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::MoveRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as FileObjectService>::r#move(&inner, request).await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = MoveSvc(inner);
                        let codec = tonic_prost::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                "/nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService/GetOptimisticLockingSupport" => {
                    #[allow(non_camel_case_types)]
                    struct GetOptimisticLockingSupportSvc<T: FileObjectService>(
                        pub Arc<T>,
                    );
                    impl<
                        T: FileObjectService,
                    > tonic::server::UnaryService<
                        super::GetOptimisticLockingSupportRequest,
                    > for GetOptimisticLockingSupportSvc<T> {
                        type Response = super::GetOptimisticLockingSupportResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<
                                super::GetOptimisticLockingSupportRequest,
                            >,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as FileObjectService>::get_optimistic_locking_support(
                                        &inner,
                                        request,
                                    )
                                    .await
                            };
                            Box::pin(fut)
                        }
                    }
                    let accept_compression_encodings = self.accept_compression_encodings;
                    let send_compression_encodings = self.send_compression_encodings;
                    let max_decoding_message_size = self.max_decoding_message_size;
                    let max_encoding_message_size = self.max_encoding_message_size;
                    let inner = self.inner.clone();
                    let fut = async move {
                        let method = GetOptimisticLockingSupportSvc(inner);
                        let codec = tonic_prost::ProstCodec::default();
                        let mut grpc = tonic::server::Grpc::new(codec)
                            .apply_compression_config(
                                accept_compression_encodings,
                                send_compression_encodings,
                            )
                            .apply_max_message_size_config(
                                max_decoding_message_size,
                                max_encoding_message_size,
                            );
                        let res = grpc.unary(method, req).await;
                        Ok(res)
                    };
                    Box::pin(fut)
                }
                _ => {
                    Box::pin(async move {
                        let mut response = http::Response::new(
                            tonic::body::Body::default(),
                        );
                        let headers = response.headers_mut();
                        headers
                            .insert(
                                tonic::Status::GRPC_STATUS,
                                (tonic::Code::Unimplemented as i32).into(),
                            );
                        headers
                            .insert(
                                http::header::CONTENT_TYPE,
                                tonic::metadata::GRPC_CONTENT_TYPE,
                            );
                        Ok(response)
                    })
                }
            }
        }
    }
    impl<T> Clone for FileObjectServiceServer<T> {
        fn clone(&self) -> Self {
            let inner = self.inner.clone();
            Self {
                inner,
                accept_compression_encodings: self.accept_compression_encodings,
                send_compression_encodings: self.send_compression_encodings,
                max_decoding_message_size: self.max_decoding_message_size,
                max_encoding_message_size: self.max_encoding_message_size,
            }
        }
    }
    /// Generated gRPC service name
    pub const SERVICE_NAME: &str = "nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService";
    impl<T> tonic::server::NamedService for FileObjectServiceServer<T> {
        const NAME: &'static str = SERVICE_NAME;
    }
}
