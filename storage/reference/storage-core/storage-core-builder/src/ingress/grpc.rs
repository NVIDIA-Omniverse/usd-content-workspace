// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{convert::Infallible, future::Future, net::SocketAddr, sync::Arc};

use bytes::Bytes;
use opentelemetry::metrics::Meter;
use tokio::net::TcpSocket;
use tokio_stream::wrappers::TcpListenerStream;

use tonic::{
    body::Body,
    server::NamedService,
    transport::{server::Router, Server},
};
use tonic_tracing_opentelemetry::middleware::{filters, server::OtelGrpcLayer};

use storage_core_util::observability::telemetry_middleware::GrpcServerTelemetryLayer;

/// Re-export of the identity layer for convenience.
pub type IdentityLayer = tower::layer::util::Identity;

/// The layer stack that enables trace propagation for gRPC requests.
/// This includes OpenTelemetry context propagation with healthcheck and reflection filtering.
pub type TracePropagationLayer<L> = tower::layer::util::Stack<OtelGrpcLayer, L>;

/// The layer stack that enables gRPC server metrics collection.
/// This includes metrics for call counts, durations, and message sizes per the gRPC A66 spec.
pub type GrpcMetricsLayer<L> = tower::layer::util::Stack<GrpcServerTelemetryLayer, L>;

/// A trait for converting types into gRPC services that can be added to a [`GrpcIngress`].
///
/// This trait provides a standardized way to convert service implementations into
/// Tower services that can be used with Tonic's gRPC server infrastructure.
///
/// # Requirements
///
/// The service must implement:
/// - [`tower::Service`] with [`http::Request<Body>`] as input and [`Infallible`] as error type
/// - [`NamedService`] for service discovery and reflection
/// - [`Clone`], [`Send`], [`Sync`], and `'static` for concurrent usage
///
/// # Example
///
/// ```rust,ignore
/// use std::sync::Arc;
///
/// struct MyService;
///
/// impl ToGrpcService for MyService {
///     type Service = MyGrpcService;
///
///     fn service(&self) -> Self::Service {
///         MyGrpcService::builder()
///     }
/// }
///
/// let ingress = GrpcIngress::builder()
///     .add_service(Arc::new(MyService));
/// ```
pub trait ToGrpcService {
    /// The Tower service type that this can be converted to.
    type Service: tower::Service<http::Request<Body>, Error = Infallible> + NamedService + Clone + Send + Sync + 'static;

    /// Converts this type into a Tower service.
    ///
    /// # Returns
    ///
    /// Returns a service that can handle gRPC requests and is compatible with
    /// Tonic's server infrastructure.
    fn service(&self) -> Self::Service
    where
        Self::Service: tower::Service<http::Request<Body>, Error = Infallible>,
        <Self::Service as tower::Service<http::Request<Body>>>::Response: axum::response::IntoResponse,
        <Self::Service as tower::Service<http::Request<Body>>>::Future: Send + 'static;

    /// Returns the file descriptor set for the service.
    ///
    /// This is used to register the file descriptor set with the gRPC reflection service.
    ///
    /// # Returns
    ///
    /// Some(file_descriptor_set) if the service should be registered with the gRPC reflection service.
    /// None if the service should not be registered with the gRPC reflection service.
    fn file_descriptor_set(&self) -> Option<&[u8]>;
}

/// A builder for creating gRPC ingress servers with multiple services.
///
/// `GrpcIngress` provides a fluent interface for configuring and building gRPC servers
/// with support for:
/// - Multiple gRPC services
/// - Health checking via `tonic-health`
/// - Service reflection via `tonic-reflection`
/// - Proto file descriptor registration for tooling support
///
/// # Example
///
/// ```rust,ignore
/// use std::{net::SocketAddr, sync::Arc};
///
/// let addr: SocketAddr = "127.0.0.1:50051".parse().unwrap();
///
/// let server = GrpcIngress::builder()
///     .add_service(Arc::new(service1))
///     .add_service(Arc::new(service2))
///     .serve(addr)?;
///
/// // Run the server
/// tokio::spawn(server);
/// ```
pub struct GrpcIngress<L> {
    server: Server<L>,
}

impl GrpcIngress<IdentityLayer> {
    /// Creates a new `GrpcIngress` builder without any tracing layers.
    ///
    /// The builder starts empty and services can be added using [`add_service`](Self::add_service).
    /// To enable trace propagation, call [`with_trace_propagation`](Self::with_trace_propagation) on the builder.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Without trace propagation (suitable for integration tests)
    /// let ingress = GrpcIngress::builder()
    ///     .add_service(Arc::new(my_service));
    /// ```
    pub fn builder() -> GrpcIngress<IdentityLayer> {
        GrpcIngress {
            server: Server::builder(),
        }
    }
}

impl<L> GrpcIngress<L> {
    /// Adds the trace propagation layer to the gRPC server.
    ///
    /// This layer propagates distributed tracing context (trace_id, span_id, etc.)
    /// from incoming requests and creates spans for each gRPC request. Health check
    /// and reflection requests are automatically filtered out to reduce noise.
    ///
    /// This method can be combined with [`with_metrics`](GrpcIngress::with_metrics)
    /// by calling both methods on the builder in any order.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let ingress = GrpcIngress::builder()
    ///     .with_trace_propagation()
    ///     .add_service(Arc::new(my_service));
    ///
    /// // Or combined with metrics (in any order)
    /// let ingress = GrpcIngress::builder()
    ///     .with_metrics(&meter)
    ///     .with_trace_propagation()
    ///     .add_service(Arc::new(my_service));
    /// ```
    pub fn with_trace_propagation(self) -> GrpcIngress<TracePropagationLayer<L>> {
        GrpcIngress {
            server: self.server.layer(
                OtelGrpcLayer::default()
                    .filter(filters::reject_healthcheck)
                    .filter(|path| !path.contains("grpc.reflection.")),
            ),
        }
    }

    /// Adds a metrics layer to the gRPC server for collecting OpenTelemetry metrics.
    ///
    /// This layer collects metrics for gRPC calls as specified by the
    /// [A66: OpenTelemetry Metrics](https://github.com/grpc/proposal/blob/master/A66-otel-stats.md)
    /// proposal. Metrics include:
    /// - `grpc.server.call.started` - Number of server calls started
    /// - `grpc.server.call.duration` - End-to-end time taken to complete a call
    /// - `grpc.server.call.rcvd_total_compressed_message_size` - Total bytes received
    /// - `grpc.server.call.sent_total_compressed_message_size` - Compressed message bytes sent
    ///
    /// This method can be combined with [`with_trace_propagation`](GrpcIngress::with_trace_propagation)
    /// by calling both methods on the builder.
    ///
    /// # Parameters
    ///
    /// * `meter` - The OpenTelemetry meter to use for recording metrics
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use opentelemetry::global;
    ///
    /// let meter = global::meter("my-service");
    ///
    /// // With metrics only
    /// let ingress = GrpcIngress::builder()
    ///     .with_metrics(&meter)
    ///     .add_service(Arc::new(my_service));
    ///
    /// // Combined with trace propagation
    /// let ingress = GrpcIngress::builder()
    ///     .with_trace_propagation()
    ///     .with_metrics(&meter)
    ///     .add_service(Arc::new(my_service));
    /// ```
    pub fn with_metrics<M: AsRef<Meter>>(self, meter: &M) -> GrpcIngress<GrpcMetricsLayer<L>> {
        GrpcIngress {
            server: self.server.layer(GrpcServerTelemetryLayer::new(meter.as_ref())),
        }
    }

    /// Customizes the underlying gRPC server configuration.
    ///
    /// This method allows you to modify the Tonic server configuration before services
    /// are added. You can use this to set server-wide options like timeouts, limits,
    /// middleware layers, and other Tonic-specific configurations.
    ///
    /// # Parameters
    ///
    /// * `f` - A closure that takes the current server configuration and returns a modified version
    ///
    /// # Returns
    ///
    /// Returns `self` to allow method chaining.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use std::time::Duration;
    ///
    /// let server = GrpcIngressServer::builder()
    ///     .customize_grpc_server(|server| {
    ///         server
    ///             .timeout(Duration::from_secs(30))
    ///             .max_concurrent_streams(Some(100))
    ///             .max_frame_size(Some(1024 * 1024)) // 1MB
    ///     })
    ///     .add_service(Arc::new(my_service))
    ///     .serve(addr)?;
    /// ```
    ///
    /// # Common Customizations
    ///
    /// - **Timeouts**: Set request/response timeouts
    /// - **Limits**: Configure concurrent streams, frame sizes, etc.
    /// - **Middleware**: Add logging, metrics, or authentication layers
    /// - **TLS**: Configure transport security settings
    /// - **Keep-alive**: Set connection keep-alive parameters
    pub fn customize_grpc_server<F, NewL>(self, f: F) -> GrpcIngress<NewL>
    where
        F: FnOnce(tonic::transport::Server<L>) -> tonic::transport::Server<NewL>,
    {
        GrpcIngress { server: f(self.server) }
    }

    /// Adds a gRPC service to the ingress server.
    ///
    /// Services are added in the order they are registered. Each service must implement
    /// the [`ToGrpcService`] trait to be convertible to a Tonic-compatible service.
    ///
    /// # Parameters
    ///
    /// * `service` - An [`Arc`]-wrapped service that implements [`ToGrpcService`]
    ///
    /// # Returns
    ///
    /// Returns `self` to allow method chaining.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use std::sync::Arc;
    ///
    /// let ingress = GrpcIngress::builder()
    ///     .add_service(Arc::new(storage_service))
    ///     .add_service(Arc::new(health_service));
    /// ```
    pub fn add_service<S: ToGrpcService>(mut self, service: Arc<S>) -> GrpcIngressServer<L>
    where
        <S::Service as tower::Service<http::Request<Body>>>::Response: axum::response::IntoResponse,
        <S::Service as tower::Service<http::Request<Body>>>::Future: Send + 'static,
        L: Clone,
    {
        let file_descriptor_sets = match service.file_descriptor_set() {
            Some(file_descriptor_set) => vec![Bytes::copy_from_slice(file_descriptor_set)],
            None => vec![],
        };

        GrpcIngressServer {
            router: self.server.add_service(service.service()),
            file_descriptor_sets,
        }
    }
}

pub struct GrpcIngressServer<L> {
    router: Router<L>,
    file_descriptor_sets: Vec<Bytes>,
}

impl<L> GrpcIngressServer<L> {
    /// Adds a gRPC service to the ingress server.
    ///
    /// Services are added in the order they are registered. Each service must implement
    /// the [`ToGrpcService`] trait to be convertible to a Tonic-compatible service.
    ///
    /// # Parameters
    ///
    /// * `service` - An [`Arc`]-wrapped service that implements [`ToGrpcService`]
    ///
    /// # Returns
    ///
    /// Returns `self` to allow method chaining.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use std::sync::Arc;
    ///
    /// let ingress = GrpcIngress::builder()
    ///     .add_service(Arc::new(storage_service))
    ///     .add_service(Arc::new(health_service));
    /// ```
    pub fn add_service<S: ToGrpcService>(mut self, service: Arc<S>) -> Self
    where
        <S::Service as tower::Service<http::Request<Body>>>::Response: axum::response::IntoResponse,
        <S::Service as tower::Service<http::Request<Body>>>::Future: Send + 'static,
    {
        if let Some(file_descriptor_set) = service.file_descriptor_set() {
            self.file_descriptor_sets
                .push(Bytes::copy_from_slice(file_descriptor_set));
        }

        Self {
            router: self.router.add_service(service.service()),
            file_descriptor_sets: self.file_descriptor_sets,
        }
    }

    /// Builds and starts the gRPC server on the specified address.
    ///
    /// This method configures the server with:
    /// - All previously added services
    /// - Health check service endpoints
    /// - gRPC reflection service for tooling support (grpcurl, etc.)
    /// - Proto file descriptor sets for storage service protobuf definitions
    ///
    /// # Parameters
    ///
    /// * `addr` - The socket address to bind the server to (e.g., `"127.0.0.1:50051"`)
    ///
    /// # Returns
    ///
    /// Returns a [`Result`] containing:
    /// - `Ok`: A future that runs the server and resolves when the server shuts down
    /// - `Err`: A boxed error if server configuration fails
    ///
    /// # Errors
    ///
    /// This method can fail if:
    /// - The reflection service cannot be built due to invalid descriptor sets
    /// - The address cannot be bound
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use std::net::SocketAddr;
    ///
    /// let addr: SocketAddr = "127.0.0.1:50051".parse().unwrap();
    ///
    /// let server_future = GrpcIngress::builder()
    ///     .add_service(Arc::new(my_service))
    ///     .serve(addr)?;
    ///
    /// // Run the server
    /// tokio::spawn(async move {
    ///     if let Err(e) = server_future.await {
    ///         eprintln!("Server error: {}", e);
    ///     }
    /// });
    /// ```
    pub fn serve<'a, ResBody>(
        self,
        addr: impl Into<SocketAddr>,
    ) -> Result<(impl Future<Output = Result<(), tonic::transport::Error>> + 'a, u16), Box<dyn std::error::Error>>
    where
        L: tower::Layer<tonic::service::Routes> + Clone + Send + Sync + 'static,
        L::Service: tower::Service<http::Request<Body>, Response = http::Response<ResBody>> + Clone + Send + 'static,
        <<L as tower::Layer<tonic::service::Routes>>::Service as tower::Service<http::Request<Body>>>::Future: Send,
        <<L as tower::Layer<tonic::service::Routes>>::Service as tower::Service<http::Request<Body>>>::Error:
            Into<tower::BoxError> + Send,
        ResBody: http_body::Body<Data = bytes::Bytes> + Send + 'static,
        ResBody::Error: Into<tower::BoxError>,
    {
        let socket = TcpSocket::new_v4()?;
        socket.set_reuseaddr(true)?;
        socket.set_reuseport(true)?;
        socket.set_nodelay(true)?;
        socket.set_keepalive(true)?;
        socket.bind(addr.into())?;

        // Get the bound port in case 0 was provided
        let bound_port = socket.local_addr()?.port();

        let incoming = TcpListenerStream::new(socket.listen(512)?);

        Ok((self.serve_with_incoming(incoming)?, bound_port))
    }

    /// Serves the gRPC server with a custom incoming connection stream.
    ///
    /// This method allows you to provide a custom stream of incoming connections
    /// for the gRPC server, enabling advanced use cases like custom transport
    /// protocols, connection filtering, or integration with custom network stacks.
    ///
    /// # Parameters
    ///
    /// * `incoming` - A stream of incoming connections. Each item should be a
    ///   `Result<IO, IE>` where `IO` is the connection type and `IE` is the
    ///   error type for connection failures.
    ///
    /// # Connection Requirements
    ///
    /// The connection type `IO` must implement:
    /// - `AsyncRead` and `AsyncWrite` for I/O operations
    /// - `Connected` for connection metadata
    /// - `Unpin`, `Send`, and `'static` for async runtime compatibility
    ///
    /// # Returns
    ///
    /// Returns a `Future` that resolves when the server shuts down. The future
    /// will complete with `Ok(())` on graceful shutdown or an error if the
    /// server encounters a fatal error.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use tokio::net::TcpListener;
    /// use tokio_stream::wrappers::TcpListenerStream;
    /// use storage_core_builder::ingress::grpc::GrpcIngress;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let ingress = GrpcIngress::builder()
    ///         .add_service(my_service)
    ///         .build();
    ///
    ///     // Create custom listener
    ///     let listener = TcpListener::bind("0.0.0.0:8080").await?;
    ///     let incoming = TcpListenerStream::new(listener);
    ///
    ///     // Serve with custom incoming stream
    ///     let server = ingress.serve_with_incoming(incoming)?;
    ///     server.await?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn serve_with_incoming<'a, IO, IE, ResBody>(
        self,
        incoming: impl tokio_stream::Stream<Item = Result<IO, IE>> + 'a,
    ) -> Result<impl Future<Output = Result<(), tonic::transport::Error>> + 'a, Box<dyn std::error::Error>>
    where
        IO: tokio::io::AsyncRead + tokio::io::AsyncWrite + tonic::transport::server::Connected + Unpin + Send + 'static,
        IE: Into<Box<dyn std::error::Error + Send + Sync>> + 'a,
        L: tower::Layer<tonic::service::Routes> + Clone + Send + Sync + 'static,
        L::Service: tower::Service<http::Request<Body>, Response = http::Response<ResBody>> + Clone + Send + 'static,
        <<L as tower::Layer<tonic::service::Routes>>::Service as tower::Service<http::Request<Body>>>::Future: Send,
        <<L as tower::Layer<tonic::service::Routes>>::Service as tower::Service<http::Request<Body>>>::Error:
            Into<tower::BoxError> + Send,
        ResBody: http_body::Body<Data = bytes::Bytes> + Send + 'static,
        ResBody::Error: Into<tower::BoxError>,
    {
        let mut reflection = tonic_reflection::server::Builder::configure()
            .register_encoded_file_descriptor_set(tonic_health::pb::FILE_DESCRIPTOR_SET)
            .register_encoded_file_descriptor_set(tonic_reflection::pb::v1alpha::FILE_DESCRIPTOR_SET);
        for file_descriptor_set in &self.file_descriptor_sets {
            reflection = reflection.register_encoded_file_descriptor_set(file_descriptor_set);
        }
        let reflection = reflection.build_v1alpha()?;

        let b = self.router.add_service(reflection);

        let (_, health) = tonic_health::server::health_reporter();
        let b = b.add_service(health);

        Ok(b.serve_with_incoming(incoming))
    }
}
