// SPDX-FileCopyrightText: Copyright (c) 2024-2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Test harness for observability (OTLP) integration tests.
//!
//! Provides [`OtelTestHarness`] which wires together a mock OTLP collector,
//! the real `try_init_observability()` stack, and a stub storage service with
//! trace propagation + gRPC metrics layers enabled. Each test gets its own
//! process (via cargo-nextest), so global subscriber conflicts are avoided.

use std::env;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Once};

use storage_core::api_services::capabilities::v1alpha::CapabilitiesBuilder as CapabilitiesBuilderV1Alpha;
use storage_core::api_services::capabilities::v1beta::CapabilitiesBuilder as CapabilitiesBuilderV1Beta;
use storage_core::api_services::filefolder::v1alpha::FileFolderBuilder as FileFolderBuilderV1Alpha;
use storage_core::api_services::filefolder::v1beta::FileFolderBuilder as FileFolderBuilderV1Beta;
use storage_core::api_services::fileobject::v1alpha::FileObjectBuilder as FileObjectBuilderV1Alpha;
use storage_core::api_services::fileobject::v1beta::FileObjectBuilder as FileObjectBuilderV1Beta;
use storage_core::api_services::versioning::v1alpha::VersioningBuilder as VersioningBuilderV1Alpha;
use storage_core::api_services::versioning::v1beta::VersioningBuilder as VersioningBuilderV1Beta;
use storage_core::storage::StorageService;
use storage_core::traits::{
    capabilities::v1alpha::Capabilities as CapabilitiesV1Alpha,
    capabilities::v1beta::Capabilities as CapabilitiesV1Beta,
    error::{StorageError, StorageErrorExt},
    filefolder::v1alpha::FileFolder as FileFolderV1Alpha,
    filefolder::v1beta::FileFolder as FileFolderV1Beta,
    fileobject::v1alpha::FileObject as FileObjectV1Alpha,
    fileobject::v1beta::FileObject as FileObjectV1Beta,
    versioning::v1alpha::Versioning as VersioningV1Alpha,
    versioning::v1beta::Versioning as VersioningV1Beta,
    Request,
};

use storage_core_test::mock_otlp_collector::{start_mock_collector, CollectedTelemetry};
use storage_core_util::observability::metrics::Meter;
use storage_core_util::observability::Guard;

use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_stream::wrappers::TcpListenerStream;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Per-test OTEL configuration. Each field controls whether the corresponding
/// signal type is exported to the mock collector.
pub struct OtelTestConfig {
    pub traces_enabled: bool,
    pub metrics_enabled: bool,
    pub logs_enabled: bool,
    pub service_name: String,
    /// If set, configures the `LOGS_LEVEL` env var for OTLP log filtering.
    pub logs_level: Option<String>,
}

impl Default for OtelTestConfig {
    fn default() -> Self {
        Self {
            traces_enabled: true,
            metrics_enabled: true,
            logs_enabled: true,
            service_name: "otel-test".to_string(),
            logs_level: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Stub backend
// ---------------------------------------------------------------------------

/// A zero-implementation storage backend for observability tests.
///
/// All trait methods use the macro-generated defaults that return
/// `UnimplementedError`. This is sufficient because the OTEL middleware
/// layers fire on every request regardless of the response status.
struct StubBackend;

macro_rules! impl_stub_trait {
    ($struct_name:ident : $($trait_path:path),+ $(,)?) => {
        $(
            impl $trait_path for $struct_name {
                fn inner(&self) -> Option<&dyn $trait_path> {
                    None
                }
            }
        )+
    };
}

impl_stub_trait!(StubBackend:
    CapabilitiesV1Alpha, CapabilitiesV1Beta,
    FileObjectV1Alpha, FileObjectV1Beta,
    FileFolderV1Alpha, FileFolderV1Beta,
    VersioningV1Alpha, VersioningV1Beta,
);

// ---------------------------------------------------------------------------
// NotFound backend (for error severity mapping tests)
// ---------------------------------------------------------------------------

/// An error that maps to gRPC `NotFound` / HTTP 404.
#[derive(Debug)]
struct NotFoundError;

impl std::fmt::Display for NotFoundError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Not found")
    }
}

impl std::error::Error for NotFoundError {}

impl StorageErrorExt for NotFoundError {
    fn tonic_status(&self) -> tonic::Status {
        storage_core::traits::error::status_with_default_error_info(tonic::Code::NotFound, "Not found", "NotFound")
    }

    fn http_status(&self) -> http::StatusCode {
        http::StatusCode::NOT_FOUND
    }

    fn get_size(&self) -> usize {
        std::mem::size_of::<Self>()
    }
}

/// Backend that returns `NotFound` for stat and `Unimplemented` for everything else.
/// Used to test error severity mapping (NotFound → INFO per SRD 3.2).
pub(crate) struct NotFoundBackend;

// Most traits delegate to the default (Unimplemented).
impl_stub_trait!(NotFoundBackend:
    CapabilitiesV1Alpha, CapabilitiesV1Beta,
    FileFolderV1Alpha, FileFolderV1Beta,
    VersioningV1Alpha, VersioningV1Beta,
    FileObjectV1Alpha,
);

// Override v1beta FileObject to return NotFound on stat.
#[storage_core::traits::async_trait]
impl FileObjectV1Beta for NotFoundBackend {
    fn inner(&self) -> Option<&dyn FileObjectV1Beta> {
        None
    }

    async fn stat(
        &self,
        _request: Request<storage_core::traits::fileobject::v1beta::StatRequest>,
    ) -> Result<storage_core::traits::fileobject::v1beta::StatResponse, StorageError> {
        Err(Box::new(NotFoundError))
    }
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// Test harness providing a running storage service wired to a mock OTLP collector.
///
/// The harness owns the [`Guard`] (which holds the OTEL providers) and exposes
/// [`flush()`](Self::flush) to force-export pending telemetry before assertions.
pub struct OtelTestHarness {
    /// gRPC address of the running storage service (e.g. `http://127.0.0.1:12345`).
    pub grpc_addr: String,
    /// REST address (e.g. `http://127.0.0.1:12346`).
    pub rest_addr: String,
    /// Handle to the collected telemetry — use wait/query methods for assertions.
    pub telemetry: CollectedTelemetry,
    /// The OTEL guard with public provider fields for force-flushing.
    pub guard: Guard,
    _server_handle: JoinHandle<()>,
    _collector_handle: JoinHandle<()>,
}

impl OtelTestHarness {
    /// Start a full observability test environment with the default `StubBackend`
    /// (all operations return `UnimplementedError`).
    ///
    /// Returns `None` when run under `cargo test` (no process isolation).
    /// Tests should early-return in that case: `let Some(h) = … else { return };`
    pub async fn start(config: OtelTestConfig) -> Option<Self> {
        let backend = Arc::new(StubBackend);
        Self::start_with_backend(config, backend).await
    }

    /// Start with a `NotFoundBackend` whose `stat()` returns `NotFound`.
    /// Used to test error severity mapping (NotFound → INFO per SRD 3.2).
    pub async fn start_with_not_found_backend(config: OtelTestConfig) -> Option<Self> {
        let backend = Arc::new(NotFoundBackend);
        Self::start_with_backend(config, backend).await
    }

    /// Common startup logic parametrized over the backend type.
    ///
    /// Returns `None` (and prints a one-time warning) when called more than once
    /// in the same process, i.e. when `cargo test` is used instead of `cargo nextest`.
    async fn start_with_backend<T>(config: OtelTestConfig, backend: Arc<T>) -> Option<Self>
    where
        T: CapabilitiesV1Alpha
            + CapabilitiesV1Beta
            + FileObjectV1Alpha
            + FileObjectV1Beta
            + FileFolderV1Alpha
            + FileFolderV1Beta
            + VersioningV1Alpha
            + VersioningV1Beta,
    {
        // Guard: each test should run in its own process (cargo nextest).
        // `set_global_default` can only be called once per process.
        // When run under `cargo test`, only the first test gets a real harness;
        // the rest return None (skipped) after printing a one-time warning.
        static INIT_CALLED: AtomicBool = AtomicBool::new(false);
        if INIT_CALLED.swap(true, Ordering::SeqCst) {
            static WARN_ONCE: Once = Once::new();
            WARN_ONCE.call_once(|| {
                write_to_raw_stderr(concat!(
                    "\n",
                    "========================================================================\n",
                    "WARNING: skipping observability tests — cargo-nextest required.\n",
                    "\n",
                    "  cargo nextest run -p storage-core-integration-tests \\\n",
                    "      --test observability --all-features\n",
                    "\n",
                    "`cargo test` runs all tests in one process, but each test needs its\n",
                    "own global tracing subscriber (set_global_default is once-per-process).\n",
                    "========================================================================\n",
                ));
            });
            return None;
        }

        // 1. Mock collector
        let (collector_port, telemetry, collector_handle) = start_mock_collector().await;
        let endpoint = format!("http://127.0.0.1:{collector_port}");

        // 2. Env vars (safe: nextest = one test per process)
        set_otel_env_vars(&config, &endpoint);

        // 3. Init observability (sets global subscriber)
        let guard = storage_core_util::observability::try_init_observability(&config.service_name, "0.0.0-test")
            .expect("failed to init observability");

        // 4. Build storage service with all layer stacks
        let meter = Meter::get("otel-test", "0.0.0-test", []);

        let service = StorageService::builder()
            .with_trace_propagation()
            .with_metrics(&meter)
            .add_service(FileObjectBuilderV1Alpha::new().backend(backend.clone()))
            .add_service(FileObjectBuilderV1Beta::new().backend(backend.clone()))
            .add_service(FileFolderBuilderV1Alpha::new().backend(backend.clone()))
            .add_service(FileFolderBuilderV1Beta::new().backend(backend.clone()))
            .add_service(CapabilitiesBuilderV1Alpha::new().backend(backend.clone()))
            .add_service(CapabilitiesBuilderV1Beta::new().backend(backend.clone()))
            .add_service(VersioningBuilderV1Alpha::new().backend(backend.clone()))
            .add_service(VersioningBuilderV1Beta::new().backend(backend.clone()));

        // 5. Start gRPC + REST on random ports
        let grpc_listener = TcpListener::bind("127.0.0.1:0").await.expect("bind gRPC listener");
        let grpc_port = grpc_listener.local_addr().unwrap().port();
        let rest_listener = TcpListener::bind("127.0.0.1:0").await.expect("bind REST listener");
        let rest_port = rest_listener.local_addr().unwrap().port();

        let server_handle = tokio::spawn(async move {
            service
                .serve_with_incoming(Some(TcpListenerStream::new(grpc_listener)), Some(rest_listener))
                .await
                .expect("storage service error");
        });

        // Brief yield so the server starts accepting connections.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        Some(Self {
            grpc_addr: format!("http://127.0.0.1:{grpc_port}"),
            rest_addr: format!("http://127.0.0.1:{rest_port}"),
            telemetry,
            guard,
            _server_handle: server_handle,
            _collector_handle: collector_handle,
        })
    }

    /// Force-flush all OTEL providers so pending telemetry is exported to the
    /// mock collector before assertions.
    pub fn flush(&self) {
        if let Some(ref tp) = self.guard.tracing_guard {
            drop(tp.force_flush());
        }
        if let Some(ref mp) = self.guard.metrics_guard {
            drop(mp.force_flush());
        }
        if let Some(ref lp) = self.guard.log_provider {
            drop(lp.force_flush());
        }
    }
}

/// Configure OTEL environment variables based on the test config.
///
/// # Safety
///
/// `env::set_var` is unsafe in multi-threaded contexts. This is safe here because
/// nextest runs each test in its own process, and we call this before spawning any
/// tasks that read env vars.
fn set_otel_env_vars(config: &OtelTestConfig, endpoint: &str) {
    // Safety: single-threaded at this point (before server/exporter tasks start)
    unsafe {
        env::set_var("OTEL_SERVICE_NAME", &config.service_name);
        // Ensure INFO-level events reach the OTLP log exporter
        env::set_var("RUST_LOG", "info");
        // Disable periodic config re-logging in tests
        env::set_var("LOGS_CONFIG_RELOG_INTERVAL", "0s");

        if !config.traces_enabled && !config.metrics_enabled && !config.logs_enabled {
            env::set_var("OTEL_SDK_DISABLED", "true");
            return;
        }

        // Base endpoint and protocol for all signals.
        // The protocol must be explicit because init-tracing-opentelemetry infers
        // "http/protobuf" for non-4317 ports, but our mock collector speaks gRPC.
        env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", endpoint);
        env::set_var("OTEL_EXPORTER_OTLP_PROTOCOL", "grpc");

        // -- Traces --
        if config.traces_enabled {
            env::set_var("OTEL_TRACES_EXPORTER", "otlp");
            env::set_var("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT", endpoint);
            env::set_var("OTEL_PROPAGATORS", "tracecontext,baggage");
        } else {
            env::set_var("OTEL_TRACES_EXPORTER", "none");
        }

        // -- Metrics --
        if !config.metrics_enabled {
            env::set_var("OTEL_METRICS_EXPORTER", "none");
        }

        // -- Logs --
        if config.logs_enabled {
            env::set_var("OTEL_LOGS_EXPORTER", "otlp");
            env::set_var("OTEL_EXPORTER_OTLP_LOGS_ENDPOINT", endpoint);
        } else {
            env::set_var("OTEL_LOGS_EXPORTER", "none");
        }

        // -- Log level filtering --
        if let Some(ref level) = config.logs_level {
            env::set_var("LOGS_LEVEL", level);
        }
    }
}

/// Write a message directly to file descriptor 2, bypassing the test runner's
/// stderr capture so the user actually sees it.
#[cfg(unix)]
fn write_to_raw_stderr(msg: &str) {
    use std::io::Write;
    use std::os::unix::io::FromRawFd;
    // ManuallyDrop: we must NOT close fd 2 when the File is dropped.
    let mut f = std::mem::ManuallyDrop::new(unsafe { std::fs::File::from_raw_fd(2) });
    let _ = f.write_all(msg.as_bytes());
    let _ = f.flush();
}

#[cfg(not(unix))]
fn write_to_raw_stderr(msg: &str) {
    eprintln!("{msg}");
}
