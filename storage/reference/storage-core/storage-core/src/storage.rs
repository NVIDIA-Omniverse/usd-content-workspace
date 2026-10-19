// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{net::SocketAddr, sync::Arc};

use opentelemetry::metrics::Meter;
use storage_core_util::info_limited;

use storage_core_builder::{
    ingress::{
        grpc::{GrpcIngress, GrpcIngressServer, GrpcMetricsLayer, IdentityLayer, ToGrpcService, TracePropagationLayer},
        rest::{RestIngress, ToRestService},
    },
    service_registry::{ServiceInfoProvider, SERVICE_REGISTRY},
};
use storage_core_util::observability::error_severity;
use storage_core_util::signals::handle_signals;

/// A unified storage service that provides both gRPC and REST endpoints.
///
/// `StorageService` combines [`GrpcIngress`] and [`RestIngress`] to create a single
/// service that can handle both gRPC and HTTP REST requests. This is particularly
/// useful for storage services that need to support multiple client types and protocols.
///
/// # Features
///
/// - **Dual Protocol Support**: Serves both gRPC and REST endpoints simultaneously
/// - **Unified Service Management**: Add services once and they're available on both protocols
/// - **Flexible Deployment**: Can run gRPC-only, REST-only, or both protocols together
/// - **Builder Pattern**: Fluent API for easy service composition
/// - **Layer Support**: Built-in support for middleware layers (metrics, caching, throttling)
/// - **Backend Integration**: Easy integration with storage backends (S3, Azure)
///
/// # Protocol Requirements
///
/// Services added to `StorageService` must implement both:
/// - [`ToGrpcService`] - For gRPC endpoint generation
/// - [`ToRestService`] - For REST endpoint generation
///
/// # Example
///
/// ```rust,ignore
/// use std::net::SocketAddr;
/// use std::sync::Arc;
/// use storage_core::{
///     api_services::fileobject::v1alpha::FileObjectBuilder,
///     api_services::filefolder::v1alpha::FileFolderBuilder,
///     layer::LayerArc,
///     metrics::{OperationMetricsLayer, SdkMetricsLayer},
///     storage::StorageService,
/// };
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     // Configure your storage backend (S3, Azure, etc.)
///     let backend = Arc::new(MyStorageBackend::new()?);
///     
///     // Set up layers for observability and performance
///     let operation_metrics_layer = LayerArc(Arc::new(OperationMetricsLayer::new("s3")));
///     let sdk_metrics_layer = LayerArc(Arc::new(SdkMetricsLayer::new("s3")));
///     
///     // Build and serve the storage service
///     let grpc_addr: SocketAddr = "127.0.0.1:50051".parse().unwrap();
///     let rest_addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
///     
///     StorageService::builder()
///         .customize_grpc_server(|server| {
///             server.timeout(std::time::Duration::from_secs(10))
///         })
///         .add_service(
///             FileObjectBuilder::new()
///                 .layer(operation_metrics_layer.clone())
///                 .layer(sdk_metrics_layer.clone())
///                 .backend(backend.clone())
///         )
///         .add_service(
///             FileFolderBuilder::new()
///                 .layer(operation_metrics_layer.clone())
///                 .layer(sdk_metrics_layer.clone())
///                 .backend(backend.clone())
///         )
///         .serve(Some(grpc_addr), Some(rest_addr))
///         .await?;
///     
///     Ok(())
/// }
/// ```
pub struct StorageService<L> {
    grpc: GrpcIngress<L>,
    rest: RestIngress,
}

impl StorageService<IdentityLayer> {
    /// Creates a new `StorageService` builder without trace propagation.
    ///
    /// This initializes both gRPC and REST ingress builders, ready to accept services.
    /// The builder pattern allows for fluent service composition before serving.
    ///
    /// To enable trace propagation, call [`with_trace_propagation`](Self::with_trace_propagation).
    ///
    /// # Returns
    ///
    /// Returns a new `StorageService` instance with empty gRPC and REST routers.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Without trace propagation (suitable for integration tests)
    /// let service = StorageService::builder();
    ///
    /// // With trace propagation enabled
    /// let service = StorageService::builder().with_trace_propagation();
    /// ```
    pub fn builder() -> StorageService<IdentityLayer> {
        StorageService {
            grpc: GrpcIngress::builder(),
            rest: RestIngress::builder(),
        }
    }
}

impl<L> StorageService<L> {
    /// Enables trace propagation on both gRPC and REST servers.
    ///
    /// This adds middleware to propagate distributed tracing context (trace_id, span_id, etc.)
    /// and create spans for each request. Health check and reflection requests are automatically
    /// filtered out to reduce noise.
    ///
    /// This method can be combined with [`with_metrics`](StorageService::with_metrics)
    /// by calling both methods on the builder in any order.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let service = StorageService::builder()
    ///     .with_trace_propagation()
    ///     .add_service(my_service);
    /// ```
    pub fn with_trace_propagation(self) -> StorageService<TracePropagationLayer<L>> {
        StorageService {
            grpc: self.grpc.with_trace_propagation(),
            rest: self.rest.with_trace_propagation(),
        }
    }

    /// Stops the API documentation from being served on the REST server.
    ///
    /// Served by default. This removes the Swagger UI at `/swagger-ui` *and*
    /// the per-service `openapi.json` documents, which are mounted through the
    /// same router. See [`RestIngress::without_swagger_ui`] for the rationale.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let service = StorageService::builder().without_swagger_ui();
    /// ```
    pub fn without_swagger_ui(self) -> Self {
        StorageService {
            grpc: self.grpc,
            rest: self.rest.without_swagger_ui(),
        }
    }

    /// Configures error severity levels from a Figment configuration source.
    ///
    /// Reads the `[error_severity]` config section, which maps gRPC code names to
    /// log level names (e.g. `NotFound = "INFO"`). Any codes not specified use
    /// SRD-recommended defaults. If the section is absent or empty, defaults apply.
    ///
    /// This sets a process-wide configuration (first call wins). Must be called
    /// before any `log_grpc_error` / `log_http_error` calls to take effect.
    ///
    /// # Example
    ///
    /// ```toml
    /// [error_severity]
    /// NotFound = "INFO"
    /// InvalidArgument = "INFO"
    /// Unavailable = "WARN"
    /// ```
    ///
    /// ```rust,ignore
    /// StorageService::builder()
    ///     .with_error_severity_from_figment(&figment)
    ///     .with_trace_propagation()
    ///     .with_metrics(meter!())
    ///     .add_service(...)
    ///     .serve(...)
    ///     .await?;
    /// ```
    pub fn with_error_severity_from_figment(self, figment: &figment::Figment) -> Self {
        let config = error_severity::Config::try_from(figment).unwrap_or_default();
        error_severity::init_error_severity(config.into());
        self
    }

    /// Enables metrics collection on both gRPC and REST servers.
    ///
    /// Accepts any type that implements `AsRef<Meter>`, including the `meter!()` macro result.
    ///
    /// Please see [`GrpcIngress::with_metrics`](GrpcIngress::with_metrics) and [`RestIngress::with_metrics`](RestIngress::with_metrics) for more details.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use storage_core::StorageService;
    /// use storage_core_util::observability::metrics::meter;
    ///
    /// let service = StorageService::builder()
    ///     .with_trace_propagation()
    ///     .with_metrics(meter!())
    ///     .add_service(my_service);
    /// ```
    pub fn with_metrics<M: AsRef<Meter>>(self, meter: &M) -> StorageService<GrpcMetricsLayer<L>> {
        StorageService {
            grpc: self.grpc.with_metrics(meter),
            rest: self.rest.with_metrics(meter),
        }
    }

    /// Customizes the underlying gRPC server configuration.
    ///
    /// This method allows you to modify the Tonic server configuration before services
    /// are added. You can use this to set server-wide options like timeouts, limits,
    /// middleware layers, and other Tonic-specific configurations that will apply to
    /// the gRPC portion of the unified storage service.
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
    /// use storage_core::{
    ///     api_services::fileobject::v1alpha::FileObjectBuilder,
    ///     storage::StorageService,
    /// };
    ///
    /// let service = StorageService::builder()
    ///     .customize_grpc_server(|server| {
    ///         server
    ///             .timeout(Duration::from_secs(30))
    ///             .max_concurrent_streams(Some(100))
    ///     })
    ///     .add_service(
    ///         FileObjectBuilder::new()
    ///             .backend(backend.clone())
    ///     )
    ///     .serve(Some(grpc_addr), Some(rest_addr))
    ///     .await?;
    /// ```
    ///
    /// # Common Customizations
    ///
    /// - **Timeouts**: Set request/response timeouts
    /// - **Limits**: Configure concurrent streams, frame sizes, etc.
    /// - **Middleware**: Add logging, metrics, or authentication layers
    /// - **TLS**: Configure transport security settings
    /// - **Keep-alive**: Set connection keep-alive parameters
    ///
    /// Full list of options: <https://docs.rs/tonic/latest/tonic/transport/server/struct.Server.html>
    pub fn customize_grpc_server<F, NewL>(self, f: F) -> StorageService<NewL>
    where
        F: FnOnce(tonic::transport::Server<L>) -> tonic::transport::Server<NewL>,
    {
        StorageService {
            grpc: self.grpc.customize_grpc_server(f),
            rest: self.rest,
        }
    }

    /// Adds a service that implements both gRPC and REST protocols.
    ///
    /// This method takes ownership of the service and registers it with both
    /// the gRPC and REST ingress systems. The service will be available on
    /// both protocols when the server is started.
    ///
    /// # Type Parameters
    ///
    /// * `S` - A service type that implements both [`ToGrpcService`] and [`ToRestService`]
    ///
    /// # Parameters
    ///
    /// * `service` - The service implementation to add
    ///
    /// # Returns
    ///
    /// Returns `self` to allow method chaining.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use storage_core::{
    ///     api_services::fileobject::v1alpha::FileObjectBuilder,
    ///     api_services::filefolder::v1alpha::FileFolderBuilder,
    ///     layer::LayerArc,
    ///     metrics::{OperationMetricsLayer, SdkMetricsLayer},
    ///     storage::StorageService,
    /// };
    /// use std::sync::Arc;
    ///
    /// let backend = Arc::new(MyStorageBackend::new()?);
    /// let operation_metrics_layer = LayerArc(Arc::new(OperationMetricsLayer::new("s3")));
    /// let sdk_metrics_layer = LayerArc(Arc::new(SdkMetricsLayer::new("s3")));
    ///
    /// let service = StorageService::builder()
    ///     .add_service(
    ///         FileObjectBuilder::new()
    ///             .layer(operation_metrics_layer.clone())
    ///             .layer(sdk_metrics_layer.clone())
    ///             .backend(backend.clone())
    ///     )
    ///     .add_service(
    ///         FileFolderBuilder::new()
    ///             .layer(operation_metrics_layer.clone())
    ///             .layer(sdk_metrics_layer.clone())
    ///             .backend(backend.clone())
    ///     );
    /// ```
    pub fn add_service<S: ToGrpcService + ToRestService + ServiceInfoProvider>(
        self,
        service: S,
    ) -> StorageServiceServer<L>
    where
        <S::Service as tower::Service<http::Request<tonic::body::Body>>>::Response: axum::response::IntoResponse,
        <S::Service as tower::Service<http::Request<tonic::body::Body>>>::Future: Send + 'static,
        L: Clone,
    {
        self.add_service_from_arc(Arc::new(service))
    }

    /// Adds a service from an [`Arc`] that implements both gRPC and REST protocols.
    ///
    /// This method is useful when you already have a service wrapped in an [`Arc`],
    /// which is common in concurrent environments where the same service instance
    /// needs to be shared across multiple contexts.
    ///
    /// # Type Parameters
    ///
    /// * `S` - A service type that implements both [`ToGrpcService`] and [`ToRestService`]
    ///
    /// # Parameters
    ///
    /// * `service` - An [`Arc`]-wrapped service implementation to add
    ///
    /// # Returns
    ///
    /// Returns `self` to allow method chaining.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use std::sync::Arc;
    /// use storage_core::{
    ///     api_services::fileobject::v1alpha::FileObjectBuilder,
    ///     layer::LayerArc,
    ///     metrics::{OperationMetricsLayer, SdkMetricsLayer},
    ///     storage::StorageService,
    /// };
    ///
    /// let backend = Arc::new(MyStorageBackend::new()?);
    /// let operation_metrics_layer = LayerArc(Arc::new(OperationMetricsLayer::new("s3")));
    /// let sdk_metrics_layer = LayerArc(Arc::new(SdkMetricsLayer::new("s3")));
    ///
    /// // Create a shared service that can be used in multiple contexts
    /// let shared_file_service = Arc::new(
    ///     FileObjectBuilder::new()
    ///         .layer(operation_metrics_layer.clone())
    ///         .layer(sdk_metrics_layer.clone())
    ///         .backend(backend.clone())
    /// );
    ///
    /// let service = StorageService::builder()
    ///     .add_service_from_arc(shared_file_service.clone())
    ///     .add_service_from_arc(Arc::new(
    ///         FileFolderBuilder::new()
    ///             .layer(operation_metrics_layer.clone())
    ///             .layer(sdk_metrics_layer.clone())
    ///             .backend(backend.clone())
    ///     ));
    ///
    /// // shared_file_service can still be used elsewhere
    /// ```
    pub fn add_service_from_arc<S: ToGrpcService + ToRestService + ServiceInfoProvider>(
        self,
        service: Arc<S>,
    ) -> StorageServiceServer<L>
    where
        <S::Service as tower::Service<http::Request<tonic::body::Body>>>::Response: axum::response::IntoResponse,
        <S::Service as tower::Service<http::Request<tonic::body::Body>>>::Future: Send + 'static,
        L: Clone,
    {
        // Register the service info with the service registry
        if let Some(service_info) = service.service_info() {
            SERVICE_REGISTRY.register(service_info);
        }

        StorageServiceServer {
            grpc: self.grpc.add_service(service.clone()),
            rest: self.rest.add_service(service),
        }
    }
}

/// Once a service is added to the storage service, it is wrapped in this type.
///
/// This second type is used because the internal Tonic service becomes a different type once
/// a service is added to it.
pub struct StorageServiceServer<L> {
    grpc: GrpcIngressServer<L>,
    rest: RestIngress,
}

impl<L> StorageServiceServer<L> {
    /// Customizes the underlying REST server configuration.
    ///
    /// This method allows you to modify the Axum router configuration after services
    /// have been added. You can use this to add global middleware, state, error handlers,
    /// and other Axum-specific configurations that will apply to the REST portion
    /// of the unified storage service.
    ///
    /// # Parameters
    ///
    /// * `f` - A closure that takes the current router configuration and returns a modified version
    ///
    /// # Returns
    ///
    /// Returns `self` to allow method chaining.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use axum::middleware;
    /// use tower::ServiceBuilder;
    /// use tower_http::trace::TraceLayer;
    /// use storage_core::{
    ///     api_services::fileobject::v1alpha::FileObjectBuilder,
    ///     storage::StorageService,
    /// };
    ///
    /// let service = StorageService::builder()
    ///     .add_service(
    ///         FileObjectBuilder::new()
    ///             .backend(backend.clone())
    ///     )
    ///     .customize_rest_server(|router| {
    ///         router
    ///             .layer(TraceLayer::new_for_http())
    ///             .layer(ServiceBuilder::new()
    ///                 .timeout(Duration::from_secs(30))
    ///                 .rate_limit(100, Duration::from_secs(60)))
    ///     })
    ///     .serve(Some(grpc_addr), Some(rest_addr))
    ///     .await?;
    /// ```
    ///
    /// # Common Customizations
    ///
    /// - **Middleware**: Add logging, CORS, authentication, rate limiting
    /// - **State**: Share application state across all routes
    /// - **Error Handling**: Configure global error responses
    /// - **Compression**: Enable gzip/brotli compression
    /// - **Security Headers**: Add security-related HTTP headers
    /// - **Metrics**: Add observability and monitoring layers
    ///
    /// Full list of options: <https://docs.rs/axum/latest/axum/struct.Router.html>
    ///
    /// # Note
    ///
    /// Any middleware added here will only be applied to services already added to this builder.
    /// This function is not available until after at least one service has been added because it would
    /// be a no-op before services are added.
    pub fn customize_rest_server(self, f: impl FnOnce(axum::Router) -> axum::Router) -> Self {
        Self {
            grpc: self.grpc,
            rest: self.rest.customize_rest_server(f),
        }
    }

    /// Adds a service that implements both gRPC and REST protocols.
    ///
    /// This method takes ownership of the service and registers it with both
    /// the gRPC and REST ingress systems. The service will be available on
    /// both protocols when the server is started.
    ///
    /// # Type Parameters
    ///
    /// * `S` - A service type that implements both [`ToGrpcService`] and [`ToRestService`]
    ///
    /// # Parameters
    ///
    /// * `service` - The service implementation to add
    ///
    /// # Returns
    ///
    /// Returns `self` to allow method chaining.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use storage_core::{
    ///     api_services::capabilities::v1alpha::CapabilitiesBuilder,
    ///     api_services::versioning::v1alpha::VersioningBuilder,
    ///     layer::LayerArc,
    ///     metrics::{OperationMetricsLayer, SdkMetricsLayer},
    ///     storage::StorageService,
    /// };
    /// use std::sync::Arc;
    ///
    /// let backend = Arc::new(MyStorageBackend::new()?);
    /// let operation_metrics_layer = LayerArc(Arc::new(OperationMetricsLayer::new("s3")));
    /// let sdk_metrics_layer = LayerArc(Arc::new(SdkMetricsLayer::new("s3")));
    ///
    /// let service = StorageService::builder()
    ///     .add_service(
    ///         CapabilitiesBuilder::new()
    ///             .layer(operation_metrics_layer.clone())
    ///             .layer(sdk_metrics_layer.clone())
    ///             .backend(backend.clone())
    ///     )
    ///     .add_service(
    ///         VersioningBuilder::new()
    ///             .layer(operation_metrics_layer.clone())
    ///             .layer(sdk_metrics_layer.clone())
    ///             .backend(backend.clone())
    ///     );
    /// ```
    pub fn add_service<S: ToGrpcService + ToRestService + ServiceInfoProvider>(self, service: S) -> Self
    where
        <S::Service as tower::Service<http::Request<tonic::body::Body>>>::Response: axum::response::IntoResponse,
        <S::Service as tower::Service<http::Request<tonic::body::Body>>>::Future: Send + 'static,
        L: Clone,
    {
        self.add_service_from_arc(Arc::new(service))
    }

    /// Adds a service from an [`Arc`] that implements both gRPC and REST protocols.
    ///
    /// This method is useful when you already have a service wrapped in an [`Arc`],
    /// which is common in concurrent environments where the same service instance
    /// needs to be shared across multiple contexts.
    ///
    /// # Type Parameters
    ///
    /// * `S` - A service type that implements both [`ToGrpcService`] and [`ToRestService`]
    ///
    /// # Parameters
    ///
    /// * `service` - An [`Arc`]-wrapped service implementation to add
    ///
    /// # Returns
    ///
    /// Returns `self` to allow method chaining.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use std::sync::Arc;
    /// use storage_core::{
    ///     api_services::capabilities::v1alpha::CapabilitiesBuilder,
    ///     layer::LayerArc,
    ///     metrics::{OperationMetricsLayer, SdkMetricsLayer},
    ///     storage::StorageService,
    /// };
    ///
    /// let backend = Arc::new(MyStorageBackend::new()?);
    /// let operation_metrics_layer = LayerArc(Arc::new(OperationMetricsLayer::new("s3")));
    /// let sdk_metrics_layer = LayerArc(Arc::new(SdkMetricsLayer::new("s3")));
    ///
    /// // Create a shared service that can be used in multiple contexts
    /// let shared_capabilities_service = Arc::new(
    ///     CapabilitiesBuilder::new()
    ///         .layer(operation_metrics_layer.clone())
    ///         .layer(sdk_metrics_layer.clone())
    ///         .backend(backend.clone())
    /// );
    ///
    /// let service = StorageService::builder()
    ///     .add_service_from_arc(shared_capabilities_service.clone())
    ///     .add_service_from_arc(Arc::new(
    ///         VersioningBuilder::new()
    ///             .layer(operation_metrics_layer.clone())
    ///             .layer(sdk_metrics_layer.clone())
    ///             .backend(backend.clone())
    ///     ));
    ///
    /// // shared_capabilities_service can still be used elsewhere
    /// ```
    pub fn add_service_from_arc<S: ToGrpcService + ToRestService + ServiceInfoProvider>(self, service: Arc<S>) -> Self
    where
        <S::Service as tower::Service<http::Request<tonic::body::Body>>>::Response: axum::response::IntoResponse,
        <S::Service as tower::Service<http::Request<tonic::body::Body>>>::Future: Send + 'static,
        L: Clone,
    {
        // Register the service info with the service registry
        if let Some(service_info) = service.service_info() {
            SERVICE_REGISTRY.register(service_info);
        }
        Self {
            grpc: self.grpc.add_service(service.clone()),
            rest: self.rest.add_service(service),
        }
    }

    /// Starts serving the storage service on the specified addresses.
    ///
    /// This method can serve gRPC-only, REST-only, or both protocols together
    /// depending on which addresses are provided. The servers will run concurrently
    /// and the method will complete when any server shuts down or when Ctrl+C is received.
    ///
    /// # Serving Modes
    ///
    /// - **gRPC + REST**: Provide both `grpc_addr` and `rest_addr` - servers run concurrently
    /// - **gRPC Only**: Provide `grpc_addr` and `None` for `rest_addr`
    /// - **REST Only**: Provide `None` for `grpc_addr` and provide `rest_addr`
    /// - **No Operation**: Provide `None` for both addresses - waits for signal then exits
    ///
    /// # Parameters
    ///
    /// * `grpc_addr` - Optional socket address for the gRPC server (e.g., `"127.0.0.1:50051"`)
    /// * `rest_addr` - Optional socket address for the REST server (e.g., `"127.0.0.1:8080"`)
    ///
    /// # Returns
    ///
    /// Returns a [`Result`] that resolves when the server(s) shut down:
    /// - `Ok(())` - Server(s) shut down cleanly (including via Ctrl+C)
    /// - `Err` - Server startup or runtime error occurred
    ///
    /// # Errors
    ///
    /// This method can fail if:
    /// - Any server fails to bind to its address
    /// - Server configuration is invalid
    /// - Runtime errors occur during serving
    ///
    /// # Examples
    ///
    /// ## Serve Both Protocols
    ///
    /// ```rust,ignore
    /// use std::net::SocketAddr;
    /// use std::sync::Arc;
    /// use storage_core::{
    ///     api_services::fileobject::v1alpha::FileObjectBuilder,
    ///     api_services::filefolder::v1alpha::FileFolderBuilder,
    ///     api_services::capabilities::v1alpha::CapabilitiesBuilder,
    ///     layer::LayerArc,
    ///     metrics::{OperationMetricsLayer, SdkMetricsLayer},
    ///     storage::StorageService,
    /// };
    ///
    /// let grpc_addr: SocketAddr = "127.0.0.1:50051".parse().unwrap();
    /// let rest_addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
    /// let backend = Arc::new(S3Storage::new(&figment, &secrets)?);
    /// let operation_metrics_layer = LayerArc(Arc::new(OperationMetricsLayer::new("s3")));
    /// let sdk_metrics_layer = LayerArc(Arc::new(SdkMetricsLayer::new("s3")));
    ///
    /// StorageService::builder()
    ///     .customize_grpc_server(|server| {
    ///         server.timeout(std::time::Duration::from_secs(10))
    ///     })
    ///     .add_service(
    ///         FileObjectBuilder::new()
    ///             .layer(operation_metrics_layer.clone())
    ///             .layer(sdk_metrics_layer.clone())
    ///             .backend(backend.clone())
    ///     )
    ///     .add_service(
    ///         FileFolderBuilder::new()
    ///             .layer(operation_metrics_layer.clone())
    ///             .layer(sdk_metrics_layer.clone())
    ///             .backend(backend.clone())
    ///     )
    ///     .add_service(
    ///         CapabilitiesBuilder::new()
    ///             .layer(operation_metrics_layer.clone())
    ///             .layer(sdk_metrics_layer.clone())
    ///             .backend(backend.clone())
    ///     )
    ///     .serve(Some(grpc_addr), Some(rest_addr))
    ///     .await?;
    ///
    /// println!("Servers running:");
    /// println!("  gRPC: {}", grpc_addr);
    /// println!("  REST: http://{}", rest_addr);
    /// println!("  Swagger UI: http://{}/swagger-ui", rest_addr);
    /// ```
    ///
    /// ## gRPC Only
    ///
    /// ```rust,ignore
    /// use std::net::SocketAddr;
    /// use std::sync::Arc;
    /// use storage_core::{
    ///     api_services::fileobject::v1alpha::FileObjectBuilder,
    ///     layer::LayerArc,
    ///     metrics::{OperationMetricsLayer, SdkMetricsLayer},
    ///     storage::StorageService,
    /// };
    ///
    /// let grpc_addr: SocketAddr = "127.0.0.1:50051".parse().unwrap();
    /// let backend = Arc::new(S3Storage::new(&figment, &secrets)?);
    /// let operation_metrics_layer = LayerArc(Arc::new(OperationMetricsLayer::new("s3")));
    /// let sdk_metrics_layer = LayerArc(Arc::new(SdkMetricsLayer::new("s3")));
    ///
    /// StorageService::builder()
    ///     .add_service(
    ///         FileObjectBuilder::new()
    ///             .layer(operation_metrics_layer.clone())
    ///             .layer(sdk_metrics_layer.clone())
    ///             .backend(backend.clone())
    ///     )
    ///     .serve(Some(grpc_addr), None)
    ///     .await?;
    /// ```
    ///
    /// ## REST Only
    ///
    /// ```rust,ignore
    /// use std::net::SocketAddr;
    /// use std::sync::Arc;
    /// use storage_core::{
    ///     api_services::fileobject::v1alpha::FileObjectBuilder,
    ///     layer::LayerArc,
    ///     metrics::{OperationMetricsLayer, SdkMetricsLayer},
    ///     storage::StorageService,
    /// };
    ///
    /// let rest_addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
    /// let backend = Arc::new(S3Storage::new(&figment, &secrets)?);
    /// let operation_metrics_layer = LayerArc(Arc::new(OperationMetricsLayer::new("s3")));
    /// let sdk_metrics_layer = LayerArc(Arc::new(SdkMetricsLayer::new("s3")));
    ///
    /// StorageService::builder()
    ///     .add_service(
    ///         FileObjectBuilder::new()
    ///             .layer(operation_metrics_layer.clone())
    ///             .layer(sdk_metrics_layer.clone())
    ///             .backend(backend.clone())
    ///     )
    ///     .serve(None, Some(rest_addr))
    ///     .await?;
    /// ```
    // Runs the servers and waits for any server to complete or Ctrl+C
    async fn run_servers<'a>(
        grpc_future: Option<impl std::future::Future<Output = Result<(), tonic::transport::Error>> + 'a>,
        rest_future: Option<impl std::future::Future<Output = Result<(), std::io::Error>> + 'a>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Wait for any server to complete or Ctrl+C
        match (grpc_future, rest_future) {
            (Some(grpc), Some(rest)) => {
                tokio::select! {
                    res = handle_signals() => {
                        info_limited!(?res, "signal received, exiting");
                        res.map(|_| ())
                    }
                    res = grpc => {
                        info_limited!(?res, "grpc server exited");
                        res.map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
                    }
                    res = rest => {
                        info_limited!(?res, "rest server exited");
                        res.map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
                    }
                }
            },
            (Some(grpc), None) => {
                tokio::select! {
                    res = handle_signals() => {
                        info_limited!(?res, "signal received, exiting");
                        res.map(|_| ())
                    }
                    res = grpc => {
                        info_limited!(?res, "grpc server exited");
                        res.map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
                    }
                }
            },
            (None, Some(rest)) => {
                tokio::select! {
                    res = handle_signals() => {
                        info_limited!(?res, "signal received, exiting");
                        res.map(|_| ())
                    }
                    res = rest => {
                        info_limited!(?res, "rest server exited");
                        res.map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
                    }
                }
            },
            (None, None) => {
                tokio::select! {
                    res = handle_signals() => {
                        info_limited!(?res, "signal received, exiting");
                        res.map(|_| ())
                    }
                }
            },
        }
    }

    /// Starts serving the storage service on the specified addresses.
    ///
    /// Trace propagation is applied based on whether [`with_trace_propagation`](StorageService::with_trace_propagation)
    /// was called on the builder.
    pub async fn serve<ResBody>(
        self,
        grpc_addr: Option<impl Into<SocketAddr>>,
        rest_addr: Option<impl Into<SocketAddr>>,
    ) -> Result<(), Box<dyn std::error::Error>>
    where
        L: tower::Layer<tonic::service::Routes> + Clone + Send + Sync + 'static,
        L::Service: tower::Service<http::Request<tonic::body::Body>, Response = http::Response<ResBody>> + Clone + Send + 'static,
        <<L as tower::Layer<tonic::service::Routes>>::Service as tower::Service<http::Request<tonic::body::Body>>>::Future: Send,
        <<L as tower::Layer<tonic::service::Routes>>::Service as tower::Service<http::Request<tonic::body::Body>>>::Error:
            Into<tower::BoxError> + Send,
        ResBody: http_body::Body<Data = bytes::Bytes> + Send + 'static,
        ResBody::Error: Into<tower::BoxError>,
    {
        let grpc_future = if let Some(grpc_addr) = grpc_addr {
            Some(self.grpc.serve(grpc_addr)?)
        } else {
            None
        };

        let rest_future = if let Some(rest_addr) = rest_addr {
            Some(self.rest.serve(rest_addr)?)
        } else {
            None
        };

        #[cfg(feature = "test")]
        {
            let _ = storage_core_util::ci::serialize_ports(
                grpc_future.as_ref().map(|f| f.1).unwrap_or(0),
                rest_future.as_ref().map(|f| f.1).unwrap_or(0),
                None,
            )
            .inspect_err(|e| storage_core_util::error_limited!(?e, "failed to serialize ports"));
        }

        Self::run_servers(grpc_future.map(|f| f.0), rest_future.map(|f| f.0)).await
    }

    /// Serves the storage service with custom incoming streams.
    ///
    /// Trace propagation is applied based on whether [`with_trace_propagation`](StorageService::with_trace_propagation)
    /// was called on the builder.
    pub async fn serve_with_incoming<'a, ResBody, IO, IE, LI>(
        self,
        grpc_incoming: Option<impl tokio_stream::Stream<Item = Result<IO, IE>> + 'a>,
        rest_incoming: Option<LI>,
    ) -> Result<(), Box<dyn std::error::Error>>
    where
        L: tower::Layer<tonic::service::Routes> + Clone + Send + Sync + 'static,
        L::Service: tower::Service<http::Request<tonic::body::Body>, Response = http::Response<ResBody>> + Clone + Send + 'static,
        <<L as tower::Layer<tonic::service::Routes>>::Service as tower::Service<http::Request<tonic::body::Body>>>::Future: Send,
        <<L as tower::Layer<tonic::service::Routes>>::Service as tower::Service<http::Request<tonic::body::Body>>>::Error:
            Into<tower::BoxError> + Send,
        ResBody: http_body::Body<Data = bytes::Bytes> + Send + 'static,
        ResBody::Error: Into<tower::BoxError>,
        IO: tokio::io::AsyncRead + tokio::io::AsyncWrite + tonic::transport::server::Connected + Unpin + Send + 'static,
        IE: Into<Box<dyn std::error::Error + Send + Sync>> + 'a,
        LI: axum::serve::Listener,
        LI::Addr: std::fmt::Debug,
    {
        let grpc_future = if let Some(grpc_incoming) = grpc_incoming {
            Some(self.grpc.serve_with_incoming(grpc_incoming)?)
        } else {
            None
        };

        let rest_future = if let Some(rest_incoming) = rest_incoming {
            Some(self.rest.serve_with_incoming(rest_incoming)?)
        } else {
            None
        };

        Self::run_servers(grpc_future, rest_future).await
    }
}
