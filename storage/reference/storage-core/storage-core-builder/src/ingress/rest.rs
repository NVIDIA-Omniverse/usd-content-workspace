// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use axum::Router;
use axum_tracing_opentelemetry::middleware::{OtelAxumLayer, OtelInResponseLayer};
use opentelemetry::metrics::Meter;
use std::{
    error::Error,
    fmt::Debug,
    future::{Future, IntoFuture},
    net::SocketAddr,
    sync::Arc,
};
use tokio::net::TcpSocket;
use tower_otel::metrics::HttpLayer;
use utoipa_swagger_ui::{SwaggerUi, Url};

/// Configuration for OpenAPI documentation within a REST service.
///
/// This struct holds the metadata and OpenAPI specification for a service
/// that will be exposed through Swagger UI documentation.
///
/// # Fields
///
/// * `name` - A human-readable name for the API (displayed in Swagger UI)
/// * `url` - The URL path where the OpenAPI JSON will be served
/// * `primary` - Whether this API should be the default selection in Swagger UI
/// * `api` - The complete OpenAPI specification for the service
///
/// # Example
///
/// ```rust,ignore
/// use utoipa::OpenApi;
///
/// let openapi = OpenAPI {
///     name: "Storage API",
///     url: "/api/v1/openapi.json",
///     primary: true,
///     api: MyApiDoc::openapi(),
/// };
/// ```
pub struct OpenAPI {
    name: &'static str,
    url: &'static str,
    primary: bool,
    api: utoipa::openapi::OpenApi,
}

impl OpenAPI {
    pub fn new(name: &'static str, url: &'static str, primary: bool, api: utoipa::openapi::OpenApi) -> Self {
        Self {
            name,
            url,
            primary,
            api,
        }
    }
}

/// A trait for converting types into REST services that can be added to a [`RestIngress`].
///
/// This trait provides a standardized way to convert service implementations into
/// Axum routers with optional OpenAPI documentation support.
///
/// # Required Methods
///
/// - [`service()`](Self::service) - Must return an Axum [`Router`] with the service endpoints
///
/// # Optional Methods
///
/// - [`openapi()`](Self::openapi) - Can return [`OpenAPI`] configuration for Swagger UI
///
/// # Example
///
/// ```rust,ignore
/// use axum::{Router, routing::get};
/// use utoipa::OpenApi;
///
/// struct MyRestService;
///
/// impl ToRestService for MyRestService {
///     fn service(&self) -> Router {
///         Router::new()
///             .route("/health", get(health_check))
///             .route("/api/data", get(get_data))
///     }
///
///     fn openapi(&self) -> Option<OpenAPI> {
///         Some(OpenAPI {
///             name: "My Service",
///             url: "/openapi.json",
///             primary: true,
///             api: MyApiDoc::openapi(),
///         })
///     }
/// }
/// ```
pub trait ToRestService {
    /// Converts this type into an Axum router.
    ///
    /// # Returns
    ///
    /// Returns a [`Router`] containing all the HTTP endpoints for this service.
    /// The router can include routes, middleware, state, and other Axum configuration.
    fn service(&self) -> Router;

    /// Optionally provides OpenAPI documentation for this service.
    ///
    /// If implemented, the returned [`OpenAPI`] configuration will be used to
    /// generate Swagger UI documentation accessible at `/swagger-ui`.
    ///
    /// # Returns
    ///
    /// Returns `Some(OpenAPI)` if the service provides API documentation,
    /// or `None` if no documentation should be generated.
    ///
    /// # Default Implementation
    ///
    /// By default, returns `None` (no API documentation).
    fn openapi(&self) -> Option<OpenAPI> {
        None
    }
}

/// A builder for creating REST ingress servers with multiple services.
///
/// `RestIngress` provides a fluent interface for configuring and building HTTP servers
/// with support for:
/// - Multiple REST services combined into a single router
/// - Automatic Swagger UI generation from OpenAPI specifications
/// - Optimized TCP socket configuration for high performance
/// - Service composition through router merging
/// - Optional trace propagation via [`with_trace_propagation`](RestIngress::with_trace_propagation)
///
/// # Example
///
/// ```rust,ignore
/// use std::{net::SocketAddr, sync::Arc};
///
/// let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
///
/// // Without trace propagation (suitable for integration tests)
/// let server = RestIngress::builder()
///     .add_service(Arc::new(auth_service))
///     .serve(addr)?;
///
/// // With trace propagation enabled
/// let server = RestIngress::builder()
///     .with_trace_propagation()
///     .add_service(Arc::new(storage_service))
///     .serve(addr)?;
///
/// // Run the server
/// tokio::spawn(server);
/// ```
pub struct RestIngress {
    router: Router,
    openapi: Vec<OpenAPI>,
    trace_propagation: bool,
    metrics: Option<HttpLayer>,
    swagger_ui: bool,
}

impl RestIngress {
    /// Creates a new `RestIngress` builder without trace propagation.
    ///
    /// The builder starts with an empty router and no OpenAPI documentation.
    /// Services can be added using [`add_service`](Self::add_service).
    /// To enable trace propagation, call [`with_trace_propagation`](Self::with_trace_propagation).
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// // Without trace propagation (suitable for integration tests)
    /// let ingress = RestIngress::builder();
    /// ```
    pub fn builder() -> Self {
        Self {
            router: Router::new(),
            openapi: Vec::new(),
            trace_propagation: false,
            metrics: None,
            swagger_ui: true,
        }
    }

    /// Enables trace propagation layers on the REST server.
    ///
    /// This adds middleware that propagates distributed tracing context
    /// and creates spans for each HTTP request.
    ///
    /// The layers are only installed when OpenTelemetry tracing is actually
    /// enabled at runtime (see [`storage_core_util::observability::is_tracing_enabled`]).
    /// When tracing is disabled, calling this method is a no-op — this avoids
    /// per-request `LayerNotFound` warnings from the axum middleware when no
    /// `OpenTelemetryLayer` is registered on the subscriber.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let ingress = RestIngress::builder()
    ///     .with_trace_propagation()
    ///     .add_service(Arc::new(my_service));
    /// ```
    pub fn with_trace_propagation(mut self) -> Self {
        self.trace_propagation = true;
        self
    }

    /// Enables metrics collection on the REST server.
    ///
    /// This adds middleware that collects HTTP server metrics as specified by the
    /// [OpenTelemetry HTTP Semantic Conventions](https://opentelemetry.io/docs/specs/semconv/http/http-metrics/).
    ///
    /// Metrics collected include:
    /// - `http.server.request.duration` - Duration of HTTP server requests (Histogram, required)
    /// - `http.server.active_requests` - Number of active HTTP server requests (UpDownCounter, optional)
    /// - `http.server.request.body.size` - Size of HTTP server request bodies (Histogram, optional)
    /// - `http.server.response.body.size` - Size of HTTP server response bodies (Histogram, optional)
    ///
    /// Attributes collected per the semantic conventions include:
    /// - `http.request.method` - HTTP request method (GET, POST, etc.)
    /// - `http.response.status_code` - HTTP response status code
    /// - `url.scheme` - The URI scheme (http, https)
    /// - `http.route` - The matched route template (if available)
    /// - `network.protocol.version` - The HTTP protocol version (1.1, 2, 3)
    ///
    /// This method can be combined with [`with_trace_propagation`](RestIngress::with_trace_propagation)
    /// by calling both methods on the builder in any order.
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
    /// let ingress = RestIngress::builder()
    ///     .with_metrics(&meter)
    ///     .add_service(Arc::new(my_service));
    ///
    /// // Combined with trace propagation
    /// let ingress = RestIngress::builder()
    ///     .with_trace_propagation()
    ///     .with_metrics(&meter)
    ///     .add_service(Arc::new(my_service));
    /// ```
    pub fn with_metrics<M: AsRef<Meter>>(mut self, meter: &M) -> Self {
        self.metrics = Some(HttpLayer::server(meter.as_ref()));
        self
    }

    /// Stops the API documentation from being served.
    ///
    /// The documentation is served by default. This turns off *all* of it, not
    /// just the HTML page at `/swagger-ui`: the per-service OpenAPI documents
    /// (`/{version}/{service}/openapi.json`) are registered through the same
    /// [`SwaggerUi`] mount, so they return `404` as well.
    ///
    /// That is deliberate. The point of the opt-out is that a deployment does
    /// not want anything that can reach the REST port to enumerate its API
    /// surface, and the machine-readable schema enumerates it just as well as
    /// the UI does. Consumers that need the schemas for client generation or
    /// contract checks should keep the default, or take the schemas out of band
    /// at build time.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let ingress = RestIngress::builder()
    ///     .without_swagger_ui()
    ///     .add_service(Arc::new(my_service));
    /// ```
    pub fn without_swagger_ui(mut self) -> Self {
        self.swagger_ui = false;
        self
    }

    /// Customizes the underlying REST server configuration.
    ///
    /// This method allows you to modify the Axum router configuration before services
    /// are added. You can use this to add global middleware, state, error handlers,
    /// and other Axum-specific configurations that apply to all routes.
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
    ///
    /// let server = RestIngress::builder()
    ///     .customize_rest_server(|router| {
    ///         router
    ///             .layer(TraceLayer::new_for_http())
    ///             .layer(ServiceBuilder::new()
    ///                 .timeout(Duration::from_secs(30))
    ///                 .rate_limit(100, Duration::from_secs(60)))
    ///     })
    ///     .add_service(Arc::new(my_service))
    ///     .serve(addr)?;
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
    pub fn customize_rest_server(mut self, f: impl FnOnce(Router) -> Router) -> Self {
        self.router = f(self.router);
        self
    }

    /// Adds a REST service to the ingress server.
    ///
    /// Services are merged into the main router in the order they are added.
    /// If a service provides OpenAPI documentation, it will be included in
    /// the generated Swagger UI.
    ///
    /// # Parameters
    ///
    /// * `service` - An [`Arc`]-wrapped service that implements [`ToRestService`]
    ///
    /// # Returns
    ///
    /// Returns `self` to allow method chaining.
    ///
    /// # Route Conflicts
    ///
    /// If multiple services define the same route, the behavior depends on Axum's
    /// router merging logic. Generally, you should ensure services have non-overlapping
    /// route paths to avoid conflicts.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use std::sync::Arc;
    ///
    /// let ingress = RestIngress::builder()
    ///     .add_service(Arc::new(api_v1_service))
    ///     .add_service(Arc::new(health_service))
    ///     .add_service(Arc::new(metrics_service));
    /// ```
    pub fn add_service<S: ToRestService>(mut self, service: Arc<S>) -> Self {
        self.router = self.router.merge(service.service());
        if let Some(openapi) = service.openapi() {
            self.openapi.push(openapi);
        }
        self
    }

    fn create_socket(addr: impl Into<SocketAddr>) -> Result<(tokio::net::TcpListener, u16), Box<dyn Error>> {
        let socket = TcpSocket::new_v4()?;
        socket.set_reuseaddr(true)?;
        socket.set_reuseport(true)?;
        socket.set_nodelay(true)?;
        socket.set_keepalive(true)?;
        socket.bind(addr.into())?;

        let bound_port = socket.local_addr()?.port();
        Ok((socket.listen(512)?, bound_port))
    }

    fn build_swagger_ui(&self) -> SwaggerUi {
        let urls = self
            .openapi
            .iter()
            .map(|openapi| {
                (
                    Url::with_primary(openapi.name, openapi.url, openapi.primary),
                    openapi.api.clone(),
                )
            })
            .collect();

        SwaggerUi::new("/swagger-ui").urls(urls)
    }

    /// Builds and starts the HTTP server on the specified address.
    ///
    /// This method configures the server with:
    /// - All previously added service routers merged together
    /// - Trace propagation layers (if enabled via [`with_trace_propagation`](RestIngress::with_trace_propagation))
    /// - API documentation — the Swagger UI at `/swagger-ui` plus each service's
    ///   `openapi.json` — if any services provide OpenAPI docs, unless
    ///   [`without_swagger_ui`](Self::without_swagger_ui) was called
    /// - Optimized TCP socket settings for performance and reliability
    ///
    /// # TCP Socket Configuration
    ///
    /// The server automatically configures the following socket options:
    /// - `SO_REUSEADDR` - Allows address reuse after server shutdown
    /// - `SO_REUSEPORT` - Enables multiple processes to bind to the same port
    /// - `TCP_NODELAY` - Disables Nagle's algorithm for lower latency
    /// - `SO_KEEPALIVE` - Enables TCP keepalive for connection health monitoring
    /// - Linger disabled - Allows immediate socket closure
    /// - Listen backlog of 512 connections
    ///
    /// # Parameters
    ///
    /// * `addr` - The socket address to bind the server to (e.g., `"127.0.0.1:8080"`)
    ///
    /// # Returns
    ///
    /// Returns a [`Result`] containing:
    /// - `Ok`: A future that runs the server and resolves when the server shuts down
    /// - `Err`: A boxed error if server configuration or binding fails
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use std::net::SocketAddr;
    ///
    /// let addr: SocketAddr = "0.0.0.0:8080".parse().unwrap();
    ///
    /// // Without trace propagation
    /// let server_future = RestIngress::builder()
    ///     .add_service(Arc::new(my_api_service))
    ///     .serve(addr)?;
    ///
    /// // With trace propagation
    /// let server_future = RestIngress::builder()
    ///     .with_trace_propagation()
    ///     .add_service(Arc::new(my_api_service))
    ///     .serve(addr)?;
    /// ```
    pub fn serve<'a>(
        self,
        addr: impl Into<SocketAddr>,
    ) -> Result<(impl Future<Output = Result<(), std::io::Error>> + 'a, u16), Box<dyn Error>> {
        let (listener, bound_port) = Self::create_socket(addr)?;
        Ok((self.serve_with_incoming(listener)?, bound_port))
    }

    /// Serves the REST server with a custom incoming listener.
    ///
    /// Trace propagation layers are applied based on whether [`with_trace_propagation`](RestIngress::with_trace_propagation)
    /// was called on the builder.
    pub fn serve_with_incoming<'a, L>(
        self,
        incoming: L,
    ) -> Result<impl Future<Output = Result<(), std::io::Error>> + 'a, Box<dyn Error>>
    where
        L: axum::serve::Listener,
        L::Addr: Debug,
    {
        Ok(axum::serve(incoming, self.build_router()).into_future())
    }

    /// Assembles the final router: the merged service routes plus the
    /// observability layers and, unless disabled, the API documentation
    /// (the Swagger UI and the per-service `openapi.json` documents, both of
    /// which are mounted by [`SwaggerUi`]).
    fn build_router(self) -> Router {
        let swagger_ui = self.swagger_ui.then(|| self.build_swagger_ui());
        let mut router = self.router;
        if self.trace_propagation && storage_core_util::observability::is_tracing_enabled() {
            router = router.layer(OtelInResponseLayer).layer(OtelAxumLayer::default());
        }
        if let Some(metrics) = self.metrics {
            router = router.layer(metrics);
        }
        if let Some(swagger_ui) = swagger_ui {
            router = router.merge(swagger_ui);
        }
        router
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use http::{Request, StatusCode};
    use tower::ServiceExt;

    /// URL a documented service publishes its schema at, mirroring the real
    /// services (e.g. `api_services/versioning/v1alpha/rest.rs`).
    const OPENAPI_URL: &str = "/v1test/documented/openapi.json";

    /// A service that publishes an OpenAPI document, like every real service does.
    /// The empty-ingress tests below cannot catch what happens to `openapi.json`,
    /// because those routes only exist once a service contributes a document.
    struct DocumentedService;

    impl ToRestService for DocumentedService {
        fn service(&self) -> Router {
            Router::new()
        }

        fn openapi(&self) -> Option<OpenAPI> {
            Some(OpenAPI::new(
                "documented/v1test",
                OPENAPI_URL,
                true,
                utoipa::openapi::OpenApiBuilder::new().build(),
            ))
        }
    }

    async fn status_of(ingress: RestIngress, uri: &str) -> StatusCode {
        let request = Request::builder().uri(uri).body(Body::empty()).unwrap();

        ingress.build_router().oneshot(request).await.unwrap().status()
    }

    async fn swagger_ui_status(ingress: RestIngress) -> StatusCode {
        status_of(ingress, "/swagger-ui/").await
    }

    #[tokio::test]
    async fn swagger_ui_is_served_by_default() {
        assert_eq!(swagger_ui_status(RestIngress::builder()).await, StatusCode::OK);
    }

    #[tokio::test]
    async fn swagger_ui_is_not_served_when_opted_out() {
        assert_eq!(
            swagger_ui_status(RestIngress::builder().without_swagger_ui()).await,
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn service_openapi_json_is_served_by_default() {
        let ingress = RestIngress::builder().add_service(Arc::new(DocumentedService));

        assert_eq!(status_of(ingress, OPENAPI_URL).await, StatusCode::OK);
    }

    /// Opting out removes the machine-readable schema too, not just the HTML UI —
    /// `openapi.json` is mounted by `SwaggerUi::urls()` and nothing else routes it.
    /// This is the documented contract; if the schema is ever mounted independently,
    /// this test is the one that has to change.
    #[tokio::test]
    async fn service_openapi_json_is_not_served_when_opted_out() {
        let ingress = RestIngress::builder()
            .without_swagger_ui()
            .add_service(Arc::new(DocumentedService));

        assert_eq!(status_of(ingress, OPENAPI_URL).await, StatusCode::NOT_FOUND);
    }
}
