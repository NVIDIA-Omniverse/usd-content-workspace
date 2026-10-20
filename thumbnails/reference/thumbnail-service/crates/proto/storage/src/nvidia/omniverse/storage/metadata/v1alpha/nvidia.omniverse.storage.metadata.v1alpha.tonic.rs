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
pub mod metadata_service_client {
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
    pub struct MetadataServiceClient<T> {
        inner: tonic::client::Grpc<T>,
    }
    impl MetadataServiceClient<tonic::transport::Channel> {
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
    impl<T> MetadataServiceClient<T>
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
        ) -> MetadataServiceClient<InterceptedService<T, F>>
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
            MetadataServiceClient::new(InterceptedService::new(inner, interceptor))
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
        pub async fn get_metadata(
            &mut self,
            request: impl tonic::IntoRequest<super::GetMetadataRequest>,
        ) -> std::result::Result<
            tonic::Response<super::GetMetadataResponse>,
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
                "/nvidia.omniverse.storage.metadata.v1alpha.MetadataService/GetMetadata",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "nvidia.omniverse.storage.metadata.v1alpha.MetadataService",
                        "GetMetadata",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        pub async fn update_metadata(
            &mut self,
            request: impl tonic::IntoRequest<super::UpdateMetadataRequest>,
        ) -> std::result::Result<
            tonic::Response<super::UpdateMetadataResponse>,
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
                "/nvidia.omniverse.storage.metadata.v1alpha.MetadataService/UpdateMetadata",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "nvidia.omniverse.storage.metadata.v1alpha.MetadataService",
                        "UpdateMetadata",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        pub async fn delete_metadata(
            &mut self,
            request: impl tonic::IntoRequest<super::DeleteMetadataRequest>,
        ) -> std::result::Result<
            tonic::Response<super::DeleteMetadataResponse>,
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
                "/nvidia.omniverse.storage.metadata.v1alpha.MetadataService/DeleteMetadata",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "nvidia.omniverse.storage.metadata.v1alpha.MetadataService",
                        "DeleteMetadata",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        pub async fn get_metadata_batch(
            &mut self,
            request: impl tonic::IntoRequest<super::GetMetadataBatchRequest>,
        ) -> std::result::Result<
            tonic::Response<tonic::codec::Streaming<super::GetMetadataBatchResponse>>,
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
                "/nvidia.omniverse.storage.metadata.v1alpha.MetadataService/GetMetadataBatch",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "nvidia.omniverse.storage.metadata.v1alpha.MetadataService",
                        "GetMetadataBatch",
                    ),
                );
            self.inner.server_streaming(req, path, codec).await
        }
        pub async fn query_metadata(
            &mut self,
            request: impl tonic::IntoRequest<super::QueryMetadataRequest>,
        ) -> std::result::Result<
            tonic::Response<tonic::codec::Streaming<super::QueryMetadataResponse>>,
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
                "/nvidia.omniverse.storage.metadata.v1alpha.MetadataService/QueryMetadata",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "nvidia.omniverse.storage.metadata.v1alpha.MetadataService",
                        "QueryMetadata",
                    ),
                );
            self.inner.server_streaming(req, path, codec).await
        }
        pub async fn update_metadata_batch(
            &mut self,
            request: impl tonic::IntoRequest<super::UpdateMetadataBatchRequest>,
        ) -> std::result::Result<
            tonic::Response<tonic::codec::Streaming<super::UpdateMetadataBatchResponse>>,
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
                "/nvidia.omniverse.storage.metadata.v1alpha.MetadataService/UpdateMetadataBatch",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "nvidia.omniverse.storage.metadata.v1alpha.MetadataService",
                        "UpdateMetadataBatch",
                    ),
                );
            self.inner.server_streaming(req, path, codec).await
        }
        pub async fn delete_metadata_batch(
            &mut self,
            request: impl tonic::IntoRequest<super::DeleteMetadataBatchRequest>,
        ) -> std::result::Result<
            tonic::Response<tonic::codec::Streaming<super::DeleteMetadataBatchResponse>>,
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
                "/nvidia.omniverse.storage.metadata.v1alpha.MetadataService/DeleteMetadataBatch",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "nvidia.omniverse.storage.metadata.v1alpha.MetadataService",
                        "DeleteMetadataBatch",
                    ),
                );
            self.inner.server_streaming(req, path, codec).await
        }
    }
}
/// Generated server implementations.
pub mod metadata_service_server {
    #![allow(
        unused_variables,
        dead_code,
        missing_docs,
        clippy::wildcard_imports,
        clippy::let_unit_value,
    )]
    use tonic::codegen::*;
    /// Generated trait containing gRPC methods that should be implemented for use with MetadataServiceServer.
    #[async_trait]
    pub trait MetadataService: std::marker::Send + std::marker::Sync + 'static {
        async fn get_metadata(
            &self,
            request: tonic::Request<super::GetMetadataRequest>,
        ) -> std::result::Result<
            tonic::Response<super::GetMetadataResponse>,
            tonic::Status,
        >;
        async fn update_metadata(
            &self,
            request: tonic::Request<super::UpdateMetadataRequest>,
        ) -> std::result::Result<
            tonic::Response<super::UpdateMetadataResponse>,
            tonic::Status,
        >;
        async fn delete_metadata(
            &self,
            request: tonic::Request<super::DeleteMetadataRequest>,
        ) -> std::result::Result<
            tonic::Response<super::DeleteMetadataResponse>,
            tonic::Status,
        >;
        /// Server streaming response type for the GetMetadataBatch method.
        type GetMetadataBatchStream: tonic::codegen::tokio_stream::Stream<
                Item = std::result::Result<
                    super::GetMetadataBatchResponse,
                    tonic::Status,
                >,
            >
            + std::marker::Send
            + 'static;
        async fn get_metadata_batch(
            &self,
            request: tonic::Request<super::GetMetadataBatchRequest>,
        ) -> std::result::Result<
            tonic::Response<Self::GetMetadataBatchStream>,
            tonic::Status,
        >;
        /// Server streaming response type for the QueryMetadata method.
        type QueryMetadataStream: tonic::codegen::tokio_stream::Stream<
                Item = std::result::Result<super::QueryMetadataResponse, tonic::Status>,
            >
            + std::marker::Send
            + 'static;
        async fn query_metadata(
            &self,
            request: tonic::Request<super::QueryMetadataRequest>,
        ) -> std::result::Result<
            tonic::Response<Self::QueryMetadataStream>,
            tonic::Status,
        >;
        /// Server streaming response type for the UpdateMetadataBatch method.
        type UpdateMetadataBatchStream: tonic::codegen::tokio_stream::Stream<
                Item = std::result::Result<
                    super::UpdateMetadataBatchResponse,
                    tonic::Status,
                >,
            >
            + std::marker::Send
            + 'static;
        async fn update_metadata_batch(
            &self,
            request: tonic::Request<super::UpdateMetadataBatchRequest>,
        ) -> std::result::Result<
            tonic::Response<Self::UpdateMetadataBatchStream>,
            tonic::Status,
        >;
        /// Server streaming response type for the DeleteMetadataBatch method.
        type DeleteMetadataBatchStream: tonic::codegen::tokio_stream::Stream<
                Item = std::result::Result<
                    super::DeleteMetadataBatchResponse,
                    tonic::Status,
                >,
            >
            + std::marker::Send
            + 'static;
        async fn delete_metadata_batch(
            &self,
            request: tonic::Request<super::DeleteMetadataBatchRequest>,
        ) -> std::result::Result<
            tonic::Response<Self::DeleteMetadataBatchStream>,
            tonic::Status,
        >;
    }
    #[derive(Debug)]
    pub struct MetadataServiceServer<T> {
        inner: Arc<T>,
        accept_compression_encodings: EnabledCompressionEncodings,
        send_compression_encodings: EnabledCompressionEncodings,
        max_decoding_message_size: Option<usize>,
        max_encoding_message_size: Option<usize>,
    }
    impl<T> MetadataServiceServer<T> {
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
    impl<T, B> tonic::codegen::Service<http::Request<B>> for MetadataServiceServer<T>
    where
        T: MetadataService,
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
                "/nvidia.omniverse.storage.metadata.v1alpha.MetadataService/GetMetadata" => {
                    #[allow(non_camel_case_types)]
                    struct GetMetadataSvc<T: MetadataService>(pub Arc<T>);
                    impl<
                        T: MetadataService,
                    > tonic::server::UnaryService<super::GetMetadataRequest>
                    for GetMetadataSvc<T> {
                        type Response = super::GetMetadataResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::GetMetadataRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as MetadataService>::get_metadata(&inner, request).await
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
                        let method = GetMetadataSvc(inner);
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
                "/nvidia.omniverse.storage.metadata.v1alpha.MetadataService/UpdateMetadata" => {
                    #[allow(non_camel_case_types)]
                    struct UpdateMetadataSvc<T: MetadataService>(pub Arc<T>);
                    impl<
                        T: MetadataService,
                    > tonic::server::UnaryService<super::UpdateMetadataRequest>
                    for UpdateMetadataSvc<T> {
                        type Response = super::UpdateMetadataResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::UpdateMetadataRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as MetadataService>::update_metadata(&inner, request)
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
                        let method = UpdateMetadataSvc(inner);
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
                "/nvidia.omniverse.storage.metadata.v1alpha.MetadataService/DeleteMetadata" => {
                    #[allow(non_camel_case_types)]
                    struct DeleteMetadataSvc<T: MetadataService>(pub Arc<T>);
                    impl<
                        T: MetadataService,
                    > tonic::server::UnaryService<super::DeleteMetadataRequest>
                    for DeleteMetadataSvc<T> {
                        type Response = super::DeleteMetadataResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::DeleteMetadataRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as MetadataService>::delete_metadata(&inner, request)
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
                        let method = DeleteMetadataSvc(inner);
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
                "/nvidia.omniverse.storage.metadata.v1alpha.MetadataService/GetMetadataBatch" => {
                    #[allow(non_camel_case_types)]
                    struct GetMetadataBatchSvc<T: MetadataService>(pub Arc<T>);
                    impl<
                        T: MetadataService,
                    > tonic::server::ServerStreamingService<
                        super::GetMetadataBatchRequest,
                    > for GetMetadataBatchSvc<T> {
                        type Response = super::GetMetadataBatchResponse;
                        type ResponseStream = T::GetMetadataBatchStream;
                        type Future = BoxFuture<
                            tonic::Response<Self::ResponseStream>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::GetMetadataBatchRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as MetadataService>::get_metadata_batch(&inner, request)
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
                        let method = GetMetadataBatchSvc(inner);
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
                "/nvidia.omniverse.storage.metadata.v1alpha.MetadataService/QueryMetadata" => {
                    #[allow(non_camel_case_types)]
                    struct QueryMetadataSvc<T: MetadataService>(pub Arc<T>);
                    impl<
                        T: MetadataService,
                    > tonic::server::ServerStreamingService<super::QueryMetadataRequest>
                    for QueryMetadataSvc<T> {
                        type Response = super::QueryMetadataResponse;
                        type ResponseStream = T::QueryMetadataStream;
                        type Future = BoxFuture<
                            tonic::Response<Self::ResponseStream>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::QueryMetadataRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as MetadataService>::query_metadata(&inner, request)
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
                        let method = QueryMetadataSvc(inner);
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
                "/nvidia.omniverse.storage.metadata.v1alpha.MetadataService/UpdateMetadataBatch" => {
                    #[allow(non_camel_case_types)]
                    struct UpdateMetadataBatchSvc<T: MetadataService>(pub Arc<T>);
                    impl<
                        T: MetadataService,
                    > tonic::server::ServerStreamingService<
                        super::UpdateMetadataBatchRequest,
                    > for UpdateMetadataBatchSvc<T> {
                        type Response = super::UpdateMetadataBatchResponse;
                        type ResponseStream = T::UpdateMetadataBatchStream;
                        type Future = BoxFuture<
                            tonic::Response<Self::ResponseStream>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::UpdateMetadataBatchRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as MetadataService>::update_metadata_batch(
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
                        let method = UpdateMetadataBatchSvc(inner);
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
                "/nvidia.omniverse.storage.metadata.v1alpha.MetadataService/DeleteMetadataBatch" => {
                    #[allow(non_camel_case_types)]
                    struct DeleteMetadataBatchSvc<T: MetadataService>(pub Arc<T>);
                    impl<
                        T: MetadataService,
                    > tonic::server::ServerStreamingService<
                        super::DeleteMetadataBatchRequest,
                    > for DeleteMetadataBatchSvc<T> {
                        type Response = super::DeleteMetadataBatchResponse;
                        type ResponseStream = T::DeleteMetadataBatchStream;
                        type Future = BoxFuture<
                            tonic::Response<Self::ResponseStream>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::DeleteMetadataBatchRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as MetadataService>::delete_metadata_batch(
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
                        let method = DeleteMetadataBatchSvc(inner);
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
    impl<T> Clone for MetadataServiceServer<T> {
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
    pub const SERVICE_NAME: &str = "nvidia.omniverse.storage.metadata.v1alpha.MetadataService";
    impl<T> tonic::server::NamedService for MetadataServiceServer<T> {
        const NAME: &'static str = SERVICE_NAME;
    }
}
