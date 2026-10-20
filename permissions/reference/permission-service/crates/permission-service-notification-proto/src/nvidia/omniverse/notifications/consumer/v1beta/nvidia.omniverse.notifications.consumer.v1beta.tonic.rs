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
