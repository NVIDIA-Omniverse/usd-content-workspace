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
pub mod event_consumer_service_client {
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
    pub struct EventConsumerServiceClient<T> {
        inner: tonic::client::Grpc<T>,
    }
    impl EventConsumerServiceClient<tonic::transport::Channel> {
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
    impl<T> EventConsumerServiceClient<T>
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
        ) -> EventConsumerServiceClient<InterceptedService<T, F>>
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
            EventConsumerServiceClient::new(InterceptedService::new(inner, interceptor))
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
        pub async fn consume_durable_events(
            &mut self,
            request: impl tonic::IntoRequest<super::ConsumeDurableEventsRequest>,
        ) -> std::result::Result<
            tonic::Response<
                tonic::codec::Streaming<super::ConsumeDurableEventsResponse>,
            >,
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
                "/nvidia.omniverse.notifications.consumer.v1beta.EventConsumerService/ConsumeDurableEvents",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "nvidia.omniverse.notifications.consumer.v1beta.EventConsumerService",
                        "ConsumeDurableEvents",
                    ),
                );
            self.inner.server_streaming(req, path, codec).await
        }
        pub async fn consume_durable_events_with_acks(
            &mut self,
            request: impl tonic::IntoStreamingRequest<
                Message = super::ConsumeDurableEventsWithAcksRequest,
            >,
        ) -> std::result::Result<
            tonic::Response<
                tonic::codec::Streaming<super::ConsumeDurableEventsWithAcksResponse>,
            >,
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
                "/nvidia.omniverse.notifications.consumer.v1beta.EventConsumerService/ConsumeDurableEventsWithAcks",
            );
            let mut req = request.into_streaming_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "nvidia.omniverse.notifications.consumer.v1beta.EventConsumerService",
                        "ConsumeDurableEventsWithAcks",
                    ),
                );
            self.inner.streaming(req, path, codec).await
        }
        pub async fn consume_non_durable_events(
            &mut self,
            request: impl tonic::IntoStreamingRequest<
                Message = super::ConsumeNonDurableEventsRequest,
            >,
        ) -> std::result::Result<
            tonic::Response<
                tonic::codec::Streaming<super::ConsumeNonDurableEventsResponse>,
            >,
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
                "/nvidia.omniverse.notifications.consumer.v1beta.EventConsumerService/ConsumeNonDurableEvents",
            );
            let mut req = request.into_streaming_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "nvidia.omniverse.notifications.consumer.v1beta.EventConsumerService",
                        "ConsumeNonDurableEvents",
                    ),
                );
            self.inner.streaming(req, path, codec).await
        }
        pub async fn create_durable_queue(
            &mut self,
            request: impl tonic::IntoRequest<super::CreateDurableQueueRequest>,
        ) -> std::result::Result<
            tonic::Response<super::CreateDurableQueueResponse>,
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
                "/nvidia.omniverse.notifications.consumer.v1beta.EventConsumerService/CreateDurableQueue",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "nvidia.omniverse.notifications.consumer.v1beta.EventConsumerService",
                        "CreateDurableQueue",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        pub async fn delete_durable_queue(
            &mut self,
            request: impl tonic::IntoRequest<super::DeleteDurableQueueRequest>,
        ) -> std::result::Result<
            tonic::Response<super::DeleteDurableQueueResponse>,
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
                "/nvidia.omniverse.notifications.consumer.v1beta.EventConsumerService/DeleteDurableQueue",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "nvidia.omniverse.notifications.consumer.v1beta.EventConsumerService",
                        "DeleteDurableQueue",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        pub async fn update_durable_queue(
            &mut self,
            request: impl tonic::IntoRequest<super::UpdateDurableQueueRequest>,
        ) -> std::result::Result<
            tonic::Response<super::UpdateDurableQueueResponse>,
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
                "/nvidia.omniverse.notifications.consumer.v1beta.EventConsumerService/UpdateDurableQueue",
            );
            let mut req = request.into_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "nvidia.omniverse.notifications.consumer.v1beta.EventConsumerService",
                        "UpdateDurableQueue",
                    ),
                );
            self.inner.unary(req, path, codec).await
        }
        pub async fn manage_dead_lettered_events(
            &mut self,
            request: impl tonic::IntoStreamingRequest<
                Message = super::ManageDeadLetteredEventsRequest,
            >,
        ) -> std::result::Result<
            tonic::Response<
                tonic::codec::Streaming<super::ManageDeadLetteredEventsResponse>,
            >,
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
                "/nvidia.omniverse.notifications.consumer.v1beta.EventConsumerService/ManageDeadLetteredEvents",
            );
            let mut req = request.into_streaming_request();
            req.extensions_mut()
                .insert(
                    GrpcMethod::new(
                        "nvidia.omniverse.notifications.consumer.v1beta.EventConsumerService",
                        "ManageDeadLetteredEvents",
                    ),
                );
            self.inner.streaming(req, path, codec).await
        }
    }
}
/// Generated server implementations.
pub mod event_consumer_service_server {
    #![allow(
        unused_variables,
        dead_code,
        missing_docs,
        clippy::wildcard_imports,
        clippy::let_unit_value,
    )]
    use tonic::codegen::*;
    /// Generated trait containing gRPC methods that should be implemented for use with EventConsumerServiceServer.
    #[async_trait]
    pub trait EventConsumerService: std::marker::Send + std::marker::Sync + 'static {
        /// Server streaming response type for the ConsumeDurableEvents method.
        type ConsumeDurableEventsStream: tonic::codegen::tokio_stream::Stream<
                Item = std::result::Result<
                    super::ConsumeDurableEventsResponse,
                    tonic::Status,
                >,
            >
            + std::marker::Send
            + 'static;
        async fn consume_durable_events(
            &self,
            request: tonic::Request<super::ConsumeDurableEventsRequest>,
        ) -> std::result::Result<
            tonic::Response<Self::ConsumeDurableEventsStream>,
            tonic::Status,
        >;
        /// Server streaming response type for the ConsumeDurableEventsWithAcks method.
        type ConsumeDurableEventsWithAcksStream: tonic::codegen::tokio_stream::Stream<
                Item = std::result::Result<
                    super::ConsumeDurableEventsWithAcksResponse,
                    tonic::Status,
                >,
            >
            + std::marker::Send
            + 'static;
        async fn consume_durable_events_with_acks(
            &self,
            request: tonic::Request<
                tonic::Streaming<super::ConsumeDurableEventsWithAcksRequest>,
            >,
        ) -> std::result::Result<
            tonic::Response<Self::ConsumeDurableEventsWithAcksStream>,
            tonic::Status,
        >;
        /// Server streaming response type for the ConsumeNonDurableEvents method.
        type ConsumeNonDurableEventsStream: tonic::codegen::tokio_stream::Stream<
                Item = std::result::Result<
                    super::ConsumeNonDurableEventsResponse,
                    tonic::Status,
                >,
            >
            + std::marker::Send
            + 'static;
        async fn consume_non_durable_events(
            &self,
            request: tonic::Request<
                tonic::Streaming<super::ConsumeNonDurableEventsRequest>,
            >,
        ) -> std::result::Result<
            tonic::Response<Self::ConsumeNonDurableEventsStream>,
            tonic::Status,
        >;
        async fn create_durable_queue(
            &self,
            request: tonic::Request<super::CreateDurableQueueRequest>,
        ) -> std::result::Result<
            tonic::Response<super::CreateDurableQueueResponse>,
            tonic::Status,
        >;
        async fn delete_durable_queue(
            &self,
            request: tonic::Request<super::DeleteDurableQueueRequest>,
        ) -> std::result::Result<
            tonic::Response<super::DeleteDurableQueueResponse>,
            tonic::Status,
        >;
        async fn update_durable_queue(
            &self,
            request: tonic::Request<super::UpdateDurableQueueRequest>,
        ) -> std::result::Result<
            tonic::Response<super::UpdateDurableQueueResponse>,
            tonic::Status,
        >;
        /// Server streaming response type for the ManageDeadLetteredEvents method.
        type ManageDeadLetteredEventsStream: tonic::codegen::tokio_stream::Stream<
                Item = std::result::Result<
                    super::ManageDeadLetteredEventsResponse,
                    tonic::Status,
                >,
            >
            + std::marker::Send
            + 'static;
        async fn manage_dead_lettered_events(
            &self,
            request: tonic::Request<
                tonic::Streaming<super::ManageDeadLetteredEventsRequest>,
            >,
        ) -> std::result::Result<
            tonic::Response<Self::ManageDeadLetteredEventsStream>,
            tonic::Status,
        >;
    }
    #[derive(Debug)]
    pub struct EventConsumerServiceServer<T> {
        inner: Arc<T>,
        accept_compression_encodings: EnabledCompressionEncodings,
        send_compression_encodings: EnabledCompressionEncodings,
        max_decoding_message_size: Option<usize>,
        max_encoding_message_size: Option<usize>,
    }
    impl<T> EventConsumerServiceServer<T> {
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
    impl<T, B> tonic::codegen::Service<http::Request<B>>
    for EventConsumerServiceServer<T>
    where
        T: EventConsumerService,
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
                "/nvidia.omniverse.notifications.consumer.v1beta.EventConsumerService/ConsumeDurableEvents" => {
                    #[allow(non_camel_case_types)]
                    struct ConsumeDurableEventsSvc<T: EventConsumerService>(pub Arc<T>);
                    impl<
                        T: EventConsumerService,
                    > tonic::server::ServerStreamingService<
                        super::ConsumeDurableEventsRequest,
                    > for ConsumeDurableEventsSvc<T> {
                        type Response = super::ConsumeDurableEventsResponse;
                        type ResponseStream = T::ConsumeDurableEventsStream;
                        type Future = BoxFuture<
                            tonic::Response<Self::ResponseStream>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::ConsumeDurableEventsRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventConsumerService>::consume_durable_events(
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
                        let method = ConsumeDurableEventsSvc(inner);
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
                "/nvidia.omniverse.notifications.consumer.v1beta.EventConsumerService/ConsumeDurableEventsWithAcks" => {
                    #[allow(non_camel_case_types)]
                    struct ConsumeDurableEventsWithAcksSvc<T: EventConsumerService>(
                        pub Arc<T>,
                    );
                    impl<
                        T: EventConsumerService,
                    > tonic::server::StreamingService<
                        super::ConsumeDurableEventsWithAcksRequest,
                    > for ConsumeDurableEventsWithAcksSvc<T> {
                        type Response = super::ConsumeDurableEventsWithAcksResponse;
                        type ResponseStream = T::ConsumeDurableEventsWithAcksStream;
                        type Future = BoxFuture<
                            tonic::Response<Self::ResponseStream>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<
                                tonic::Streaming<super::ConsumeDurableEventsWithAcksRequest>,
                            >,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventConsumerService>::consume_durable_events_with_acks(
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
                        let method = ConsumeDurableEventsWithAcksSvc(inner);
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
                "/nvidia.omniverse.notifications.consumer.v1beta.EventConsumerService/ConsumeNonDurableEvents" => {
                    #[allow(non_camel_case_types)]
                    struct ConsumeNonDurableEventsSvc<T: EventConsumerService>(
                        pub Arc<T>,
                    );
                    impl<
                        T: EventConsumerService,
                    > tonic::server::StreamingService<
                        super::ConsumeNonDurableEventsRequest,
                    > for ConsumeNonDurableEventsSvc<T> {
                        type Response = super::ConsumeNonDurableEventsResponse;
                        type ResponseStream = T::ConsumeNonDurableEventsStream;
                        type Future = BoxFuture<
                            tonic::Response<Self::ResponseStream>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<
                                tonic::Streaming<super::ConsumeNonDurableEventsRequest>,
                            >,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventConsumerService>::consume_non_durable_events(
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
                        let method = ConsumeNonDurableEventsSvc(inner);
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
                "/nvidia.omniverse.notifications.consumer.v1beta.EventConsumerService/CreateDurableQueue" => {
                    #[allow(non_camel_case_types)]
                    struct CreateDurableQueueSvc<T: EventConsumerService>(pub Arc<T>);
                    impl<
                        T: EventConsumerService,
                    > tonic::server::UnaryService<super::CreateDurableQueueRequest>
                    for CreateDurableQueueSvc<T> {
                        type Response = super::CreateDurableQueueResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::CreateDurableQueueRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventConsumerService>::create_durable_queue(
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
                        let method = CreateDurableQueueSvc(inner);
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
                "/nvidia.omniverse.notifications.consumer.v1beta.EventConsumerService/DeleteDurableQueue" => {
                    #[allow(non_camel_case_types)]
                    struct DeleteDurableQueueSvc<T: EventConsumerService>(pub Arc<T>);
                    impl<
                        T: EventConsumerService,
                    > tonic::server::UnaryService<super::DeleteDurableQueueRequest>
                    for DeleteDurableQueueSvc<T> {
                        type Response = super::DeleteDurableQueueResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::DeleteDurableQueueRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventConsumerService>::delete_durable_queue(
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
                        let method = DeleteDurableQueueSvc(inner);
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
                "/nvidia.omniverse.notifications.consumer.v1beta.EventConsumerService/UpdateDurableQueue" => {
                    #[allow(non_camel_case_types)]
                    struct UpdateDurableQueueSvc<T: EventConsumerService>(pub Arc<T>);
                    impl<
                        T: EventConsumerService,
                    > tonic::server::UnaryService<super::UpdateDurableQueueRequest>
                    for UpdateDurableQueueSvc<T> {
                        type Response = super::UpdateDurableQueueResponse;
                        type Future = BoxFuture<
                            tonic::Response<Self::Response>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<super::UpdateDurableQueueRequest>,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventConsumerService>::update_durable_queue(
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
                        let method = UpdateDurableQueueSvc(inner);
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
                "/nvidia.omniverse.notifications.consumer.v1beta.EventConsumerService/ManageDeadLetteredEvents" => {
                    #[allow(non_camel_case_types)]
                    struct ManageDeadLetteredEventsSvc<T: EventConsumerService>(
                        pub Arc<T>,
                    );
                    impl<
                        T: EventConsumerService,
                    > tonic::server::StreamingService<
                        super::ManageDeadLetteredEventsRequest,
                    > for ManageDeadLetteredEventsSvc<T> {
                        type Response = super::ManageDeadLetteredEventsResponse;
                        type ResponseStream = T::ManageDeadLetteredEventsStream;
                        type Future = BoxFuture<
                            tonic::Response<Self::ResponseStream>,
                            tonic::Status,
                        >;
                        fn call(
                            &mut self,
                            request: tonic::Request<
                                tonic::Streaming<super::ManageDeadLetteredEventsRequest>,
                            >,
                        ) -> Self::Future {
                            let inner = Arc::clone(&self.0);
                            let fut = async move {
                                <T as EventConsumerService>::manage_dead_lettered_events(
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
                        let method = ManageDeadLetteredEventsSvc(inner);
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
    impl<T> Clone for EventConsumerServiceServer<T> {
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
    pub const SERVICE_NAME: &str = "nvidia.omniverse.notifications.consumer.v1beta.EventConsumerService";
    impl<T> tonic::server::NamedService for EventConsumerServiceServer<T> {
        const NAME: &'static str = SERVICE_NAME;
    }
}
