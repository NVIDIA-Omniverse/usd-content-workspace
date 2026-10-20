// SPDX-FileCopyrightText: Copyright (c) 2024 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use crate::mocks::metadata::MockMetadata;
use crate::names::{test_names, TestNames, AZURE_CONTAINER};
use crate::service::{create_manifest_service, create_metadata_router_storage_service, create_storage_service};
use fred::clients::Pool;
use fred::interfaces::KeysInterface;
use futures::StreamExt;
use tempfile::TempDir;
use tokio::runtime::Runtime;
use tracing::{error, warn};

use storage_core::ingress::IdentityLayer;
use storage_core_test::clients::traits::capabilities::Capabilities;
use storage_core_test::clients::traits::filefolder::FileFolder;
use storage_core_test::clients::traits::fileobject::FileObject;
use storage_core_test::clients::traits::metadata::Metadata;
use storage_core_test::clients::traits::versioning::Versioning;
use storage_core_test::headers::HeadersConfig;
use storage_core_test::metrics::InMemoryMetrics;
use storage_core_test::service::{ApiVersion, Interface};
use storage_core_test::storage_service_runner::StorageServiceRunner;
use storage_core_test::temp_directory::TempDirectory;
use storage_core_util::cache::build_redis_client;

/// A long-lived multi-thread Tokio runtime used by integration tests to
/// host async work (e.g., the L2 redis cache pub/sub subscriber and the
/// fred connection task) for the lifetime of the test process. Initialized
/// lazily so existing tests that don't touch L2 still pay nothing.
///
/// The fixture must NOT call `block_on` directly: rstest fixtures run inside
/// each `#[tokio::test]`'s own runtime, and tokio refuses nested `block_on`.
/// Use [`block_on_offthread`] to run async work without nesting runtimes.
fn fixture_runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("storage-fixture")
            .build()
            .expect("failed to build fixture runtime")
    })
}

/// Run a future to completion using the fixture runtime, but from a thread
/// that doesn't currently have a tokio runtime attached. This avoids the
/// "Cannot start a runtime from within a runtime" panic that fires when the
/// rstest fixture is constructed under a `#[tokio::test]`.
fn block_on_offthread<F>(future: F) -> F::Output
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    let handle = fixture_runtime().handle().clone();
    std::thread::spawn(move || handle.block_on(future))
        .join()
        .expect("fixture off-thread block_on panicked")
}

/// Mint a unique L2 keyspace prefix for one fixture instance. Combines the
/// process id with a random nonce so parallel `cargo nextest` workers and
/// prior CI runs cannot observe each other's keys, and so leftovers from a
/// crashed prior run cannot collide either. The pattern mirrors the one in
/// `storage-core-util/src/cache/cache.rs` test code.
fn make_redis_namespace() -> String {
    format!("test-{}-{:016x}", std::process::id(), rand::random::<u64>())
}

/// Build a `fred::Client` from the env var `OMNI_STORAGE_SERVICE_REDIS_ADDRESS`.
/// Returns `None` when the env var is unset. Panics when set but unreachable
/// — this is desirable for a CI mode that explicitly opts into L2.
fn redis_client_from_env(env: &EnvConfig) -> Option<Pool> {
    let address = env.redis_address.as_ref()?.clone();
    let client = block_on_offthread(async move { build_redis_client(&address).await })
        .unwrap_or_else(|e| panic!("failed to connect to redis: {e}"));
    Some(client)
}

#[derive(Debug, Clone, Copy, PartialEq, Hash, Eq)]
pub enum Backend {
    Azure,
    S3,
    Minio,
    GCPS3Compat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Access {
    Public,
    PrivateKey,
    Versioned,
    Empty,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Cache {
    List,
    Stat,
    Disabled,
    Manifest,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExpectedFailure {
    InvalidArgument,
    PreconditionFailed,
    AlreadyExists,
}

impl From<ExpectedFailure> for tonic::Code {
    fn from(value: ExpectedFailure) -> Self {
        match value {
            ExpectedFailure::InvalidArgument => tonic::Code::InvalidArgument,
            ExpectedFailure::PreconditionFailed => tonic::Code::FailedPrecondition,
            ExpectedFailure::AlreadyExists => tonic::Code::AlreadyExists,
        }
    }
}

impl From<ExpectedFailure> for reqwest::StatusCode {
    fn from(value: ExpectedFailure) -> Self {
        match value {
            ExpectedFailure::InvalidArgument => reqwest::StatusCode::BAD_REQUEST,
            ExpectedFailure::PreconditionFailed => reqwest::StatusCode::PRECONDITION_FAILED,
            ExpectedFailure::AlreadyExists => reqwest::StatusCode::CONFLICT,
        }
    }
}

pub struct StorageFixture {
    s3_no_cache: StorageServiceRunner,
    azure_no_cache: StorageServiceRunner,
    s3_list_cache: StorageServiceRunner,
    azure_list_cache: StorageServiceRunner,
    s3_stat_cache: StorageServiceRunner,
    azure_stat_cache: StorageServiceRunner,
    s3_manifest: StorageServiceRunner,
    azure_manifest: StorageServiceRunner,
    // Manifest runners configured with a tiny `[manifest] page_size_limit` so that listing,
    // list_stat, and enumerate are forced to paginate across the backend->manifest boundary.
    // Used only by the cross-source pagination tests; everything else uses the normal-page
    // runners above.
    s3_manifest_small_page: StorageServiceRunner,
    azure_manifest_small_page: StorageServiceRunner,
    // Runners with `native_metadata_enabled = true` (no caches), used by the native-metadata
    // round-trip test. Backend metadata is seeded out-of-band (the write API rejects it), then
    // surfaced through these runners.
    s3_native_metadata: StorageServiceRunner,
    azure_native_metadata: StorageServiceRunner,
    // Runners with `native_metadata_enabled = true` and the `list_cache` / `small_object_cache` layers
    // enabled, used by the native-metadata cache carry-through test.
    s3_native_metadata_cache: StorageServiceRunner,
    azure_native_metadata_cache: StorageServiceRunner,
    // Runner with `native_metadata_enabled = true` (+ fan-out) and only the `small_object_cache` layer
    // (no `list_cache`), used by the small-object-cache hint test. Lazily constructed for the same
    // reason as the router runners below. S3 only: Azure returns native metadata inline on the list and
    // ignores `include_native_metadata`, so the caller-flag path this covers is S3-specific.
    s3_native_metadata_soc_only: OnceLock<StorageServiceRunner>,
    native_metadata_soc_only_config: PathBuf,
    // Router runners are lazily constructed: each runner spawns its own thread + tokio
    // runtime, so eagerly creating them inflates the per-process thread count for every
    // test that touches the fixture (including tests that never use the router).
    // Initialising on first access keeps the cost off the unrelated test paths.
    s3_router: OnceLock<StorageServiceRunner>,
    s3_router_mock: Arc<MockMetadata>,
    azure_router: OnceLock<StorageServiceRunner>,
    azure_router_mock: Arc<MockMetadata>,
    router_config_path: PathBuf,
    router_headers_config: HeadersConfig,
    s3_url_map: HashMap<Backend, HashMap<Access, String>>,
    azure_url_map: HashMap<Backend, HashMap<Access, String>>,
    _tmp_dir: TempDir,
    pub minio_address: String,
    pub redis_address: Option<String>,
    /// Shared L2 (Redis) client, reused across cache-bearing runners and the
    /// lazily-initialised router runners.
    redis_client: Option<Pool>,
    /// Per-fixture L2 key prefix. Each integration-test process gets a unique
    /// value so parallel `cargo nextest` workers and prior CI runs cannot
    /// observe each other's cache state. `None` when L2 is disabled.
    redis_namespace: Option<String>,
    pub metrics_exporter: &'static InMemoryMetrics,
}

impl StorageFixture {
    pub fn new() -> Self {
        Self::with_assumed_role(None)
    }

    pub fn with_assumed_role(aws_assumed_role_arn: Option<String>) -> Self {
        let metrics_exporter = crate::global_metrics_exporter();
        let env = EnvConfig::load();
        let tmp_dir = tempfile::tempdir().unwrap();
        let redis_namespace = env.redis_address.as_ref().map(|_| make_redis_namespace());
        let config_files = ConfigFiles::create(
            tmp_dir.path(),
            &env,
            &aws_assumed_role_arn,
            false,
            redis_namespace.as_deref(),
        );
        let headers_config = create_headers_config();
        let redis = redis_client_from_env(&env);

        Self {
            s3_no_cache: create_s3_runner(&config_files.no_cache_config, &headers_config, None),
            s3_list_cache: create_s3_runner(&config_files.list_cache_config, &headers_config, redis.clone()),
            s3_stat_cache: create_s3_runner(&config_files.stat_cache_config, &headers_config, redis.clone()),
            azure_no_cache: create_azure_runner(&config_files.no_cache_config, &headers_config, None),
            azure_list_cache: create_azure_runner(&config_files.list_cache_config, &headers_config, redis.clone()),
            azure_stat_cache: create_azure_runner(&config_files.stat_cache_config, &headers_config, redis.clone()),
            s3_manifest: create_s3_manifest_runner(&config_files.stat_cache_config, &headers_config, redis.clone()),
            azure_manifest: create_azure_manifest_runner(
                &config_files.stat_cache_config,
                &headers_config,
                redis.clone(),
            ),
            s3_manifest_small_page: create_s3_manifest_runner(
                &config_files.manifest_small_page_config,
                &headers_config,
                redis.clone(),
            ),
            azure_manifest_small_page: create_azure_manifest_runner(
                &config_files.manifest_small_page_config,
                &headers_config,
                redis.clone(),
            ),
            s3_native_metadata: create_s3_runner(&config_files.native_metadata_config, &headers_config, None),
            azure_native_metadata: create_azure_runner(&config_files.native_metadata_config, &headers_config, None),
            s3_native_metadata_cache: create_s3_runner(
                &config_files.native_metadata_cache_config,
                &headers_config,
                redis.clone(),
            ),
            azure_native_metadata_cache: create_azure_runner(
                &config_files.native_metadata_cache_config,
                &headers_config,
                redis.clone(),
            ),
            s3_native_metadata_soc_only: OnceLock::new(),
            native_metadata_soc_only_config: config_files.native_metadata_soc_only_config.clone(),
            s3_router: OnceLock::new(),
            s3_router_mock: MockMetadata::new(),
            azure_router: OnceLock::new(),
            azure_router_mock: MockMetadata::new(),
            router_config_path: config_files.no_cache_config.clone(),
            router_headers_config: headers_config,
            s3_url_map: build_s3_url_map(&env.minio_address),
            azure_url_map: build_azure_url_map(),
            minio_address: env.minio_address,
            redis_address: env.redis_address.clone(),
            redis_client: redis,
            redis_namespace,
            _tmp_dir: tmp_dir,
            metrics_exporter,
        }
    }

    pub fn with_cdn() -> Self {
        let metrics_exporter = crate::global_metrics_exporter();
        let env = EnvConfig::load();
        let tmp_dir = tempfile::tempdir().unwrap();
        let redis_namespace = env.redis_address.as_ref().map(|_| make_redis_namespace());
        let config_files = ConfigFiles::create(tmp_dir.path(), &env, &None, true, redis_namespace.as_deref());
        let headers_config = create_headers_config();
        let redis = redis_client_from_env(&env);

        Self {
            s3_no_cache: create_s3_runner(&config_files.no_cache_config, &headers_config, None),
            s3_list_cache: create_s3_runner(&config_files.list_cache_config, &headers_config, redis.clone()),
            s3_stat_cache: create_s3_runner(&config_files.stat_cache_config, &headers_config, redis.clone()),
            azure_no_cache: create_azure_runner(&config_files.no_cache_config, &headers_config, None),
            azure_list_cache: create_azure_runner(&config_files.list_cache_config, &headers_config, redis.clone()),
            azure_stat_cache: create_azure_runner(&config_files.stat_cache_config, &headers_config, redis.clone()),
            s3_manifest: create_s3_manifest_runner(&config_files.stat_cache_config, &headers_config, redis.clone()),
            azure_manifest: create_azure_manifest_runner(
                &config_files.stat_cache_config,
                &headers_config,
                redis.clone(),
            ),
            s3_manifest_small_page: create_s3_manifest_runner(
                &config_files.manifest_small_page_config,
                &headers_config,
                redis.clone(),
            ),
            azure_manifest_small_page: create_azure_manifest_runner(
                &config_files.manifest_small_page_config,
                &headers_config,
                redis.clone(),
            ),
            s3_native_metadata: create_s3_runner(&config_files.native_metadata_config, &headers_config, None),
            azure_native_metadata: create_azure_runner(&config_files.native_metadata_config, &headers_config, None),
            s3_native_metadata_cache: create_s3_runner(
                &config_files.native_metadata_cache_config,
                &headers_config,
                redis.clone(),
            ),
            azure_native_metadata_cache: create_azure_runner(
                &config_files.native_metadata_cache_config,
                &headers_config,
                redis.clone(),
            ),
            s3_native_metadata_soc_only: OnceLock::new(),
            native_metadata_soc_only_config: config_files.native_metadata_soc_only_config.clone(),
            s3_router: OnceLock::new(),
            s3_router_mock: MockMetadata::new(),
            azure_router: OnceLock::new(),
            azure_router_mock: MockMetadata::new(),
            router_config_path: config_files.no_cache_config.clone(),
            router_headers_config: headers_config,
            s3_url_map: build_s3_url_map(&env.minio_address),
            azure_url_map: build_azure_url_map(),
            minio_address: env.minio_address,
            redis_address: env.redis_address.clone(),
            redis_client: redis,
            redis_namespace,
            _tmp_dir: tmp_dir,
            metrics_exporter,
        }
    }

    fn s3_router(&self) -> &StorageServiceRunner {
        self.s3_router.get_or_init(|| {
            create_s3_router_runner(
                &self.router_config_path,
                &self.router_headers_config,
                self.s3_router_mock.clone(),
                self.redis_client.clone(),
            )
        })
    }

    fn s3_native_metadata_soc_only(&self) -> &StorageServiceRunner {
        self.s3_native_metadata_soc_only.get_or_init(|| {
            // L2 is disabled in this config; keep the hints local to the runner.
            create_s3_runner(&self.native_metadata_soc_only_config, &self.router_headers_config, None)
        })
    }

    /// FileObject client backed by the `native_metadata_enabled = true` S3 runner that has only the
    /// `small_object_cache` layer (no `list_cache`).
    pub async fn native_metadata_soc_only_file_object_client(
        &self,
        interface: Interface,
        api_version: ApiVersion,
    ) -> Box<dyn FileObject> {
        self.s3_native_metadata_soc_only()
            .file_object_client(interface, api_version)
            .await
    }

    /// FileFolder client backed by the `native_metadata_enabled = true` S3 runner that has only the
    /// `small_object_cache` layer (no `list_cache`).
    pub async fn native_metadata_soc_only_file_folder_client(
        &self,
        interface: Interface,
        api_version: ApiVersion,
    ) -> Box<dyn FileFolder> {
        self.s3_native_metadata_soc_only()
            .file_folder_client(interface, api_version)
            .await
    }

    fn azure_router(&self) -> &StorageServiceRunner {
        self.azure_router.get_or_init(|| {
            create_azure_router_runner(
                &self.router_config_path,
                &self.router_headers_config,
                self.azure_router_mock.clone(),
                self.redis_client.clone(),
            )
        })
    }

    /// The L2 (Redis) address discovered from `OMNI_STORAGE_SERVICE_REDIS_ADDRESS`.
    /// Tests that depend on the L2 cache should soft-skip when this is `None`.
    pub fn has_redis_l2(&self) -> bool {
        self.redis_address.is_some()
    }

    /// Shared L2 (Redis) client. Tests that need to probe Redis directly
    /// (e.g., to verify a layer's L2 invalidation behavior) should clone this
    /// rather than building their own. Returns `None` when L2 is not configured.
    pub fn redis_client(&self) -> Option<Pool> {
        self.redis_client.clone()
    }

    /// Per-fixture L2 keyspace prefix written into the test config files'
    /// `[redis] namespace = ...`. Tests that compute exact L2 keys (e.g. to
    /// `redis.get(...)` a `list_cache:d:<url>` entry) must prepend this prefix
    /// so they probe the same keyspace the runners write to. Returns `None`
    /// when L2 is not configured (callers should already have soft-skipped).
    pub fn redis_namespace(&self) -> Option<&str> {
        self.redis_namespace.as_deref()
    }

    pub async fn file_object_client(
        &self,
        backend: Backend,
        cache: Cache,
        interface: Interface,
        api_version: ApiVersion,
    ) -> Box<dyn FileObject> {
        match backend {
            Backend::Azure => match cache {
                Cache::List => self.azure_list_cache.file_object_client(interface, api_version).await,
                Cache::Stat => self.azure_stat_cache.file_object_client(interface, api_version).await,
                Cache::Disabled => self.azure_no_cache.file_object_client(interface, api_version).await,
                Cache::Manifest => self.azure_manifest.file_object_client(interface, api_version).await,
            },
            Backend::S3 | Backend::Minio | Backend::GCPS3Compat => match cache {
                Cache::List => self.s3_list_cache.file_object_client(interface, api_version).await,
                Cache::Stat => self.s3_stat_cache.file_object_client(interface, api_version).await,
                Cache::Disabled => self.s3_no_cache.file_object_client(interface, api_version).await,
                Cache::Manifest => self.s3_manifest.file_object_client(interface, api_version).await,
            },
        }
    }

    /// FileObject client backed by the `native_metadata_enabled = true` runner (no caches).
    pub async fn native_metadata_file_object_client(
        &self,
        backend: Backend,
        interface: Interface,
        api_version: ApiVersion,
    ) -> Box<dyn FileObject> {
        match backend {
            Backend::Azure => {
                self.azure_native_metadata
                    .file_object_client(interface, api_version)
                    .await
            },
            _ => self.s3_native_metadata.file_object_client(interface, api_version).await,
        }
    }

    /// FileFolder client backed by the `native_metadata_enabled = true` runner (no caches).
    pub async fn native_metadata_file_folder_client(
        &self,
        backend: Backend,
        interface: Interface,
        api_version: ApiVersion,
    ) -> Box<dyn FileFolder> {
        match backend {
            Backend::Azure => {
                self.azure_native_metadata
                    .file_folder_client(interface, api_version)
                    .await
            },
            _ => self.s3_native_metadata.file_folder_client(interface, api_version).await,
        }
    }

    /// FileObject client backed by the `native_metadata_enabled = true` runner with `list_cache` /
    /// `small_object_cache` enabled (cache carry-through test).
    pub async fn native_metadata_cache_file_object_client(
        &self,
        backend: Backend,
        interface: Interface,
        api_version: ApiVersion,
    ) -> Box<dyn FileObject> {
        match backend {
            Backend::Azure => {
                self.azure_native_metadata_cache
                    .file_object_client(interface, api_version)
                    .await
            },
            _ => {
                self.s3_native_metadata_cache
                    .file_object_client(interface, api_version)
                    .await
            },
        }
    }

    /// FileFolder client backed by the `native_metadata_enabled = true` runner with `list_cache` /
    /// `small_object_cache` enabled (cache carry-through test).
    pub async fn native_metadata_cache_file_folder_client(
        &self,
        backend: Backend,
        interface: Interface,
        api_version: ApiVersion,
    ) -> Box<dyn FileFolder> {
        match backend {
            Backend::Azure => {
                self.azure_native_metadata_cache
                    .file_folder_client(interface, api_version)
                    .await
            },
            _ => {
                self.s3_native_metadata_cache
                    .file_folder_client(interface, api_version)
                    .await
            },
        }
    }

    pub async fn capabilities_client(
        &self,
        backend: Backend,
        interface: Interface,
        api_version: ApiVersion,
    ) -> Box<dyn Capabilities> {
        match backend {
            Backend::Azure => self.azure_no_cache.capabilities_client(interface, api_version).await,
            Backend::S3 | Backend::Minio | Backend::GCPS3Compat => {
                self.s3_no_cache.capabilities_client(interface, api_version).await
            },
        }
    }

    pub async fn file_folder_client(
        &self,
        backend: Backend,
        cache: Cache,
        interface: Interface,
        api_version: ApiVersion,
    ) -> Box<dyn FileFolder> {
        match backend {
            Backend::Azure => match cache {
                Cache::List => self.azure_list_cache.file_folder_client(interface, api_version).await,
                Cache::Stat => self.azure_stat_cache.file_folder_client(interface, api_version).await,
                Cache::Disabled => self.azure_no_cache.file_folder_client(interface, api_version).await,
                Cache::Manifest => self.azure_manifest.file_folder_client(interface, api_version).await,
            },
            Backend::S3 | Backend::Minio | Backend::GCPS3Compat => match cache {
                Cache::List => self.s3_list_cache.file_folder_client(interface, api_version).await,
                Cache::Stat => self.s3_stat_cache.file_folder_client(interface, api_version).await,
                Cache::Disabled => self.s3_no_cache.file_folder_client(interface, api_version).await,
                Cache::Manifest => self.s3_manifest.file_folder_client(interface, api_version).await,
            },
        }
    }

    /// File-folder client backed by the manifest runner whose `[manifest] page_size_limit`
    /// is tiny, forcing list / list_stat to emit many small pages that straddle the
    /// backend->manifest boundary. Used by the cross-source pagination tests.
    pub async fn manifest_small_page_file_folder_client(
        &self,
        backend: Backend,
        interface: Interface,
        api_version: ApiVersion,
    ) -> Box<dyn FileFolder> {
        match backend {
            Backend::Azure => {
                self.azure_manifest_small_page
                    .file_folder_client(interface, api_version)
                    .await
            },
            Backend::S3 | Backend::Minio | Backend::GCPS3Compat => {
                self.s3_manifest_small_page
                    .file_folder_client(interface, api_version)
                    .await
            },
        }
    }

    /// File-object client backed by the tiny-page manifest runner (see
    /// [`Self::manifest_small_page_file_folder_client`]). Used to exercise the enumerate
    /// pagination path across the backend->manifest boundary.
    pub async fn manifest_small_page_file_object_client(
        &self,
        backend: Backend,
        interface: Interface,
        api_version: ApiVersion,
    ) -> Box<dyn FileObject> {
        match backend {
            Backend::Azure => {
                self.azure_manifest_small_page
                    .file_object_client(interface, api_version)
                    .await
            },
            Backend::S3 | Backend::Minio | Backend::GCPS3Compat => {
                self.s3_manifest_small_page
                    .file_object_client(interface, api_version)
                    .await
            },
        }
    }

    pub async fn versioning_client(
        &self,
        backend: Backend,
        cache: Cache,
        interface: Interface,
        api_version: ApiVersion,
    ) -> Box<dyn Versioning> {
        match backend {
            Backend::Azure => match cache {
                Cache::List => self.azure_list_cache.versioning_client(interface, api_version).await,
                Cache::Stat => self.azure_stat_cache.versioning_client(interface, api_version).await,
                Cache::Disabled => self.azure_no_cache.versioning_client(interface, api_version).await,
                Cache::Manifest => self.azure_manifest.versioning_client(interface, api_version).await,
            },
            Backend::S3 | Backend::Minio | Backend::GCPS3Compat => match cache {
                Cache::List => self.s3_list_cache.versioning_client(interface, api_version).await,
                Cache::Stat => self.s3_stat_cache.versioning_client(interface, api_version).await,
                Cache::Disabled => self.s3_no_cache.versioning_client(interface, api_version).await,
                Cache::Manifest => self.s3_manifest.versioning_client(interface, api_version).await,
            },
        }
    }

    pub async fn metadata_client(
        &self,
        backend: Backend,
        cache: Cache,
        interface: Interface,
        api_version: ApiVersion,
    ) -> Box<dyn Metadata> {
        match backend {
            Backend::Azure => match cache {
                Cache::List => self.azure_list_cache.metadata_client(interface, api_version).await,
                Cache::Stat => self.azure_stat_cache.metadata_client(interface, api_version).await,
                Cache::Disabled => self.azure_no_cache.metadata_client(interface, api_version).await,
                Cache::Manifest => self.azure_manifest.metadata_client(interface, api_version).await,
            },
            Backend::S3 | Backend::Minio | Backend::GCPS3Compat => match cache {
                Cache::List => self.s3_list_cache.metadata_client(interface, api_version).await,
                Cache::Stat => self.s3_stat_cache.metadata_client(interface, api_version).await,
                Cache::Disabled => self.s3_no_cache.metadata_client(interface, api_version).await,
                Cache::Manifest => self.s3_manifest.metadata_client(interface, api_version).await,
            },
        }
    }

    /// The `MockMetadata` registered as the default branch of the router for the given
    /// backend. Tests use this to seed state and inspect recorded requests.
    pub fn router_mock(&self, backend: Backend) -> Arc<MockMetadata> {
        match backend {
            Backend::Azure => self.azure_router_mock.clone(),
            Backend::S3 | Backend::Minio | Backend::GCPS3Compat => self.s3_router_mock.clone(),
        }
    }

    /// A metadata client connected to the router runner for the given backend.
    pub async fn router_metadata_client(
        &self,
        backend: Backend,
        interface: Interface,
        api_version: ApiVersion,
    ) -> Box<dyn Metadata> {
        match backend {
            Backend::Azure => self.azure_router().metadata_client(interface, api_version).await,
            Backend::S3 | Backend::Minio | Backend::GCPS3Compat => {
                self.s3_router().metadata_client(interface, api_version).await
            },
        }
    }

    /// File-object client for the router runner. The router runner uses the same real
    /// backend as `s3_no_cache`/`azure_no_cache` for non-metadata services, so callers
    /// can write objects via this client and then exercise metadata against them.
    pub async fn router_file_object_client(
        &self,
        backend: Backend,
        interface: Interface,
        api_version: ApiVersion,
    ) -> Box<dyn FileObject> {
        match backend {
            Backend::Azure => self.azure_router().file_object_client(interface, api_version).await,
            Backend::S3 | Backend::Minio | Backend::GCPS3Compat => {
                self.s3_router().file_object_client(interface, api_version).await
            },
        }
    }

    /// Create a temporary directory backed by the router runner's file-object client.
    pub async fn router_temp_directory(
        &self,
        backend: Backend,
        access: Access,
        api_version: ApiVersion,
    ) -> Option<TempDirectory> {
        let base_url = self.make_url(backend, access, "")?;
        Some(TempDirectory::new(
            self.router_file_object_client(backend, Interface::Grpc, api_version)
                .await,
            &base_url,
            "unittests/router",
        ))
    }

    pub fn rest_port(&self, backend: Backend, cache: Cache) -> u16 {
        match backend {
            Backend::Azure => match cache {
                Cache::List => self.azure_list_cache.rest_port,
                Cache::Stat => self.azure_stat_cache.rest_port,
                Cache::Disabled => self.azure_no_cache.rest_port,
                Cache::Manifest => self.azure_manifest.rest_port,
            },
            Backend::S3 | Backend::Minio | Backend::GCPS3Compat => match cache {
                Cache::List => self.s3_list_cache.rest_port,
                Cache::Stat => self.s3_stat_cache.rest_port,
                Cache::Disabled => self.s3_no_cache.rest_port,
                Cache::Manifest => self.s3_manifest.rest_port,
            },
        }
    }

    pub fn flush_metrics(&self) {
        self.metrics_exporter.flush();
    }

    pub fn get_method_metric_value(
        &self,
        backend: Backend,
        api_version: ApiVersion,
        name: &str,
        method_label: &str,
        labels: &[(&str, &str)],
    ) -> Option<u64> {
        // Convert backend to string
        let storage_backend_str = match backend {
            Backend::Azure => "azure",
            Backend::S3 => "s3",
            Backend::Minio => "s3",
            Backend::GCPS3Compat => "s3",
        };

        // Convert api_version to string
        let api_version_str = match api_version {
            ApiVersion::V1Alpha => "v1alpha",
            ApiVersion::V1Beta => "v1beta",
        };

        // Build extended labels with api_version and storage_backend
        let mut extended_labels = Vec::with_capacity(labels.len() + 2);
        extended_labels.extend_from_slice(labels);
        extended_labels.push(("api_version", api_version_str));
        extended_labels.push(("storage_backend", storage_backend_str));
        extended_labels.push(("method", method_label));

        self.metrics_exporter.get_metric_value(name, &extended_labels[..])
    }

    pub fn get_metric_value(&self, name: &str, labels: &[(&str, &str)]) -> Option<u64> {
        self.metrics_exporter.get_metric_value(name, labels)
    }

    /// Print metrics filtered by backend, api_version, and optionally method
    /// Returns a formatted string of matching metrics
    pub fn print_filtered_method_metrics(
        &self,
        name: &str,
        backend: Backend,
        api_version: ApiVersion,
        labels: &[(&str, &str)],
    ) -> String {
        // Convert backend to string
        let storage_backend_str = match backend {
            Backend::Azure => "azure",
            Backend::S3 => "s3",
            Backend::Minio => "s3",
            Backend::GCPS3Compat => "s3",
        };

        // Convert api_version to string
        let api_version_str = match api_version {
            ApiVersion::V1Alpha => "v1alpha",
            ApiVersion::V1Beta => "v1beta",
        };

        // Build extended labels with api_version and storage_backend
        let mut extended_labels = Vec::with_capacity(labels.len() + 2);
        extended_labels.extend_from_slice(labels);
        extended_labels.push(("api_version", api_version_str));
        extended_labels.push(("storage_backend", storage_backend_str));

        self.metrics_exporter.print_filtered_metrics(name, &extended_labels[..])
    }

    pub fn print_filtered_metrics(&self, name: &str, labels: &[(&str, &str)]) -> String {
        self.metrics_exporter.print_filtered_metrics(name, labels)
    }

    /// Make a URL for a given backend and access level
    ///
    /// This will return None if the backend does not support the access method, in which case the test should be skipped
    /// rstest doesn't have a way to skip tests based on the values of enums, so we need to do this manually
    pub fn make_url(&self, backend: Backend, access: Access, address: &str) -> Option<String> {
        match backend {
            Backend::Azure => self
                .azure_url_map
                .get(&backend)
                .and_then(|map| map.get(&access))
                .map(|url| Self::combine_url(url, address)),
            Backend::S3 | Backend::Minio | Backend::GCPS3Compat => self
                .s3_url_map
                .get(&backend)
                .and_then(|map| map.get(&access))
                .map(|url| Self::combine_url(url, address)),
        }
    }

    fn combine_url(url: &str, address: &str) -> String {
        let safe_base = if url.ends_with('/') {
            url.to_string()
        } else {
            format!("{url}/")
        };

        let base = url::Url::parse(&safe_base).unwrap();
        let result = base.join(address).unwrap();
        result.to_string()
    }

    /// Create a temporary directory for a given backend and access level
    pub async fn temp_directory(
        &self,
        backend: Backend,
        access: Access,
        cache: Cache,
        api_version: ApiVersion,
    ) -> Option<TempDirectory> {
        let base_url = self.make_url(backend, access, "")?;
        Some(TempDirectory::new(
            self.file_object_client(backend, cache, Interface::Grpc, api_version)
                .await,
            &base_url,
            "unittests/writetests",
        ))
    }

    pub fn check_minio_server(minio_address: String) -> bool {
        // The rstest fixture cannot be async to use the #[once] attribute,
        // but we're already in a runtime, so we can't use things like block_on,
        // so we need to spawn a thread to run the check
        let th = std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                match reqwest::Client::new()
                    .head(format!("http://{minio_address}"))
                    .send()
                    .await
                {
                    Ok(response) => response
                        .headers()
                        .get("Server")
                        .and_then(|h| h.to_str().ok())
                        .map(|s| s == "MinIO")
                        .unwrap_or(false),
                    Err(_) => false,
                }
            })
        });
        th.join().unwrap()
    }

    pub fn get_folder_placeholder(folder_address: &str, backend: Backend) -> String {
        match backend {
            Backend::Azure => format!("{folder_address}.folder"),
            Backend::S3 | Backend::Minio | Backend::GCPS3Compat => folder_address.to_string(),
        }
    }
}

impl Default for StorageFixture {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for StorageFixture {
    /// Best-effort cleanup of this fixture's L2 keyspace. Each test process
    /// uses a unique `redis_namespace`, so this only deletes keys it owns —
    /// it cannot disturb a parallel `cargo nextest` worker. Failures are
    /// logged and swallowed: cleanup is hygiene, not correctness.
    fn drop(&mut self) {
        let (Some(pool), Some(prefix)) = (self.redis_client.clone(), self.redis_namespace.clone()) else {
            return;
        };
        // The shared handle is a round-robin pool; pin one connection for the
        // cursor-based SCAN and the deletes it drives.
        let client = pool.next().clone();
        let pattern = format!("{prefix}:*");
        block_on_offthread(async move {
            if tokio::time::timeout(std::time::Duration::from_secs(5), async {
                let mut stream = client.scan_buffered(pattern.clone(), Some(500), None);
                let mut batch: Vec<fred::types::Key> = Vec::with_capacity(500);
                while let Some(item) = stream.next().await {
                    match item {
                        Ok(key) => batch.push(key),
                        Err(e) => {
                            warn!("L2 fixture cleanup: scan error for {pattern}: {e}");
                            return;
                        },
                    }
                    if batch.len() >= 500 {
                        if let Err(e) = client.del::<u64, _>(std::mem::take(&mut batch)).await {
                            warn!("L2 fixture cleanup: del error for {pattern}: {e}");
                        }
                    }
                }
                if !batch.is_empty() {
                    if let Err(e) = client.del::<u64, _>(batch).await {
                        warn!("L2 fixture cleanup: del error for {pattern}: {e}");
                    }
                }
            })
            .await
            .is_err()
            {
                warn!("L2 fixture cleanup: timed out for {pattern}");
            }
        });
    }
}

// =============================================================================
// Helper structs and functions for StorageFixture setup
// =============================================================================

/// Environment configuration loaded from environment variables
struct EnvConfig {
    azure_storage_key_versioned: String,
    azure_tenant_id: String,
    azure_client_id: String,
    azure_client_secret: String,
    azure_public_front_door_url: String,
    azure_private_front_door_url: String,
    azure_versioned_front_door_url: String,
    aws_access_key_id: String,
    aws_secret_access_key: String,
    minio_access_key_id: String,
    minio_secret_access_key: String,
    minio_region: String,
    minio_address: String,
    gcp_access_key_id: String,
    gcp_secret_access_key: String,
    aws_cloudfront_public_url: String,
    aws_cloudfront_private_url: String,
    aws_cloudfront_versioned_url: String,
    aws_cloudfront_private_key_ecdsa: String,
    aws_cloudfront_private_key_ecdsa_id: String,
    aws_cloudfront_private_key_rsa: String,
    aws_cloudfront_private_key_rsa_id: String,
    /// Optional Redis/Valkey address for the L2 cache, sourced from
    /// `OMNI_STORAGE_SERVICE_REDIS_ADDRESS`. When `None`, the L2 tier is
    /// disabled and integration tests that require it should soft-skip.
    redis_address: Option<String>,
}

impl EnvConfig {
    fn load() -> Self {
        use base64::{engine::general_purpose, Engine as _};

        // CI variables cannot hold multi-line values reliably, so the PEMs for cloudfront are base64 encoded
        let aws_cloudfront_private_key_ecdsa = String::from_utf8(
            general_purpose::STANDARD
                .decode(Self::get_env_or_error(
                    "OMNI_STORAGE_SERVICE_CLOUDFRONT_PRIVATE_KEY_ECDSA",
                    "Private access tests will fail.",
                ))
                .unwrap(),
        )
        .unwrap();

        let aws_cloudfront_private_key_rsa = String::from_utf8(
            general_purpose::STANDARD
                .decode(Self::get_env_or_error(
                    "OMNI_STORAGE_SERVICE_CLOUDFRONT_PRIVATE_KEY_RSA",
                    "Private access tests will fail.",
                ))
                .unwrap(),
        )
        .unwrap();

        Self {
            azure_storage_key_versioned: Self::get_env_or_error(
                "OMNI_STORAGE_SERVICE_AZURE_STORAGE_KEY_VERSIONED",
                "Versioned access tests will fail.",
            ),
            azure_tenant_id: Self::get_env_or_error(
                "OMNI_STORAGE_SERVICE_AZURE_TENANT_ID",
                "Private access tests will fail.",
            ),
            azure_client_id: Self::get_env_or_error(
                "OMNI_STORAGE_SERVICE_AZURE_CLIENT_ID",
                "Private access tests will fail.",
            ),
            azure_client_secret: Self::get_env_or_error(
                "OMNI_STORAGE_SERVICE_AZURE_CLIENT_SECRET",
                "Private access tests will fail.",
            ),
            azure_public_front_door_url: Self::get_env_or_error(
                "OMNI_STORAGE_SERVICE_AZURE_PUBLIC_FRONT_DOOR_URL",
                "Public access tests will fail.",
            ),
            azure_private_front_door_url: Self::get_env_or_error(
                "OMNI_STORAGE_SERVICE_AZURE_PRIVATE_FRONT_DOOR_URL",
                "Private access tests will fail.",
            ),
            azure_versioned_front_door_url: Self::get_env_or_error(
                "OMNI_STORAGE_SERVICE_AZURE_VERSIONED_FRONT_DOOR_URL",
                "Versioned access tests will fail.",
            ),
            aws_access_key_id: Self::get_env_or_error(
                "OMNI_STORAGE_SERVICE_AWS_ACCESS_KEY_ID",
                "Private access tests will fail.",
            ),
            aws_secret_access_key: Self::get_env_or_error(
                "OMNI_STORAGE_SERVICE_AWS_SECRET_ACCESS_KEY",
                "Private access tests will fail.",
            ),
            minio_access_key_id: Self::get_env_or_error(
                "OMNI_STORAGE_SERVICE_MINIO_ACCESS_KEY_ID",
                "Private access tests will fail.",
            ),
            minio_secret_access_key: Self::get_env_or_error(
                "OMNI_STORAGE_SERVICE_MINIO_SECRET_ACCESS_KEY",
                "Private access tests will fail.",
            ),
            minio_region: Self::get_env_or_error(
                "OMNI_STORAGE_SERVICE_MINIO_REGION",
                "Private access tests will fail.",
            ),
            minio_address: Self::get_env_or_warn("OMNI_STORAGE_SERVICE_MINIO_ADDRESS", "127.0.0.1:9000"),
            gcp_access_key_id: Self::get_env_or_error(
                "OMNI_STORAGE_SERVICE_GCP_KEY_ID",
                "Private access tests will fail.",
            ),
            gcp_secret_access_key: Self::get_env_or_error(
                "OMNI_STORAGE_SERVICE_GCP_KEY_SECRET",
                "Private access tests will fail.",
            ),
            aws_cloudfront_public_url: Self::get_env_or_error(
                "OMNI_STORAGE_SERVICE_CLOUDFRONT_PUBLIC_URL",
                "Empty access tests will fail.",
            ),
            aws_cloudfront_private_url: Self::get_env_or_error(
                "OMNI_STORAGE_SERVICE_CLOUDFRONT_PRIVATE_URL",
                "Private access tests will fail.",
            ),
            aws_cloudfront_versioned_url: Self::get_env_or_error(
                "OMNI_STORAGE_SERVICE_CLOUDFRONT_VERSIONED_URL",
                "Versioned access tests will fail.",
            ),
            aws_cloudfront_private_key_ecdsa,
            aws_cloudfront_private_key_ecdsa_id: Self::get_env_or_error(
                "OMNI_STORAGE_SERVICE_CLOUDFRONT_PRIVATE_KEY_ECDSA_ID",
                "Private access tests will fail.",
            ),
            aws_cloudfront_private_key_rsa,
            aws_cloudfront_private_key_rsa_id: Self::get_env_or_error(
                "OMNI_STORAGE_SERVICE_CLOUDFRONT_PRIVATE_KEY_RSA_ID",
                "Private access tests will fail.",
            ),
            redis_address: std::env::var("OMNI_STORAGE_SERVICE_REDIS_ADDRESS").ok(),
        }
    }

    fn get_env_or_error(var_name: &str, failure_msg: &str) -> String {
        match std::env::var(var_name) {
            Ok(value) => value,
            Err(_) => {
                error!(
                    "{} not set. {} Please run 'source tools/vault/local_env.sh'",
                    var_name, failure_msg
                );
                String::new()
            },
        }
    }

    fn get_env_or_warn(var_name: &str, default: &str) -> String {
        match std::env::var(var_name) {
            Ok(value) => value,
            Err(_) => {
                warn!("{} not set, defaulting to {}", var_name, default);
                default.to_string()
            },
        }
    }
}

/// Paths to generated config files
struct ConfigFiles {
    // These files are created on disk and referenced by other configs
    #[allow(dead_code)]
    secrets_file: PathBuf,
    list_cache_config: PathBuf,
    stat_cache_config: PathBuf,
    no_cache_config: PathBuf,
    /// Config for the manifest runner whose `[manifest] page_size_limit` is tiny, so listing
    /// and enumeration paginate aggressively across the backend->manifest boundary.
    manifest_small_page_config: PathBuf,
    /// Config with `native_metadata_enabled = true` (+ the S3 ListStat HEAD fan-out), no caches, used
    /// by the native-metadata round-trip test.
    native_metadata_config: PathBuf,
    /// Config with `native_metadata_enabled = true` (+ fan-out) and the `list_cache` /
    /// `small_object_cache` layers enabled, so the native-metadata cache carry-through can be exercised
    /// (Stat/read_from_address served from a cached listing; Read served from a list-populated hint).
    native_metadata_cache_config: PathBuf,
    /// As `native_metadata_cache_config`, but with `list_cache` disabled so the `small_object_cache`
    /// talks straight to the backend. This isolates the hint the SOC publishes from its own `list_stat`;
    /// with `list_cache` in between, that layer always asks for native metadata and would mask an SOC
    /// that forwarded the caller's `include_native_metadata` untouched.
    native_metadata_soc_only_config: PathBuf,
}

impl ConfigFiles {
    fn create(
        tmp_dir: &Path,
        env: &EnvConfig,
        aws_assumed_role_arn: &Option<String>,
        use_cdn: bool,
        redis_namespace: Option<&str>,
    ) -> Self {
        let secrets_file = if use_cdn {
            Self::create_cdn_secrets_file(tmp_dir, env)
        } else {
            Self::create_secrets_file(tmp_dir, env, aws_assumed_role_arn)
        };

        // The L2 layers receive their `fred::Client` externally, but they read
        // the `[redis]` figment block to pick up the per-fixture key prefix.
        // `RedisConfig::address` is required by serde, so we write the env
        // address alongside the namespace whenever L2 is enabled for the run.
        let redis_block = match (env.redis_address.as_deref(), redis_namespace) {
            (Some(address), Some(namespace)) => Some(format!(
                "[redis]\naddress = \"{address}\"\nnamespace = \"{namespace}\"\n"
            )),
            _ => None,
        };

        let list_cache_config = Self::create_list_cache_config(tmp_dir, &secrets_file, redis_block.as_deref());
        let stat_cache_config = Self::create_stat_cache_config(tmp_dir, &secrets_file, redis_block.as_deref());
        let no_cache_config = Self::create_no_cache_config(tmp_dir, &secrets_file, redis_block.as_deref());
        let manifest_small_page_config =
            Self::create_manifest_small_page_config(tmp_dir, &secrets_file, redis_block.as_deref());
        let native_metadata_config =
            Self::create_native_metadata_config(tmp_dir, &secrets_file, redis_block.as_deref());
        let native_metadata_cache_config =
            Self::create_native_metadata_cache_config(tmp_dir, &secrets_file, redis_block.as_deref());
        let native_metadata_soc_only_config =
            Self::create_native_metadata_soc_only_config(tmp_dir, &secrets_file, redis_block.as_deref());

        Self {
            secrets_file,
            list_cache_config,
            stat_cache_config,
            no_cache_config,
            manifest_small_page_config,
            native_metadata_config,
            native_metadata_cache_config,
            native_metadata_soc_only_config,
        }
    }

    /// No caches, but `native_metadata_enabled = true` and the S3 ListStat HEAD fan-out on, so
    /// stat/read/read_from_address/list_stat surface backend-native metadata. Azure ignores the S3-only
    /// fan-out key (Config does not deny unknown fields; Azure returns metadata inline on the list).
    fn create_native_metadata_config(tmp_dir: &Path, secrets_file: &Path, redis_block: Option<&str>) -> PathBuf {
        let path = tmp_dir.join("native_metadata_config.toml");
        // The backend config is read from the `[s3]` / `[azure]` section (Storage::CONFIG_KEY), not the
        // TOML root; the cache sections below are read at the root by their own layers.
        let config = format!(
            r#"
[s3]
native_metadata_enabled = true
native_metadata_list_fanout_enabled = true
[azure]
native_metadata_enabled = true
[secrets]
secrets_file = "{}"
[additional_headers]
cache_control = "no-cache"
[list_cache]
enabled = false
[stat_cache]
enabled = false
[metadata_cache]
enabled = false
[small_object_cache]
enabled = false
[metadata_key_remapper.mappings]
"nv-omniverse-nucleus-hash" = "nv_omniverse_nucleus_hash"
{}"#,
            secrets_file.to_str().unwrap(),
            redis_block.unwrap_or(""),
        );
        fs::write(&path, config).unwrap();
        path
    }

    /// `native_metadata_enabled = true` (+ fan-out) with the `list_cache` and `small_object_cache`
    /// layers enabled, so the native-metadata cache carry-through is exercised: a Stat/read_from_address
    /// answered from the cached parent-folder listing, and a Read answered from a list-populated small
    /// object hint, must both still surface backend-native metadata.
    fn create_native_metadata_cache_config(tmp_dir: &Path, secrets_file: &Path, redis_block: Option<&str>) -> PathBuf {
        let path = tmp_dir.join("native_metadata_cache_config.toml");
        let config = format!(
            r#"
[s3]
native_metadata_enabled = true
native_metadata_list_fanout_enabled = true
[azure]
native_metadata_enabled = true
[secrets]
secrets_file = "{}"
[additional_headers]
cache_control = "no-cache"
[list_cache]
l2_enabled = true
invalidate_on_update = true
[stat_cache]
enabled = false
[metadata_cache]
enabled = false
[small_object_cache]
l2_enabled = true
small_object_size = "128KiB"
[metadata_key_remapper.mappings]
"nv-omniverse-nucleus-hash" = "nv_omniverse_nucleus_hash"
{}"#,
            secrets_file.to_str().unwrap(),
            redis_block.unwrap_or(""),
        );
        fs::write(&path, config).unwrap();
        path
    }

    /// As above, but with `list_cache` disabled: the `small_object_cache` sits directly on the backend,
    /// so the hint a `ListStat` publishes is built from exactly what the backend returned for that
    /// request. L2 is off so the hints stay local to this runner.
    fn create_native_metadata_soc_only_config(
        tmp_dir: &Path,
        secrets_file: &Path,
        redis_block: Option<&str>,
    ) -> PathBuf {
        let path = tmp_dir.join("native_metadata_soc_only_config.toml");
        let config = format!(
            r#"
[s3]
native_metadata_enabled = true
native_metadata_list_fanout_enabled = true
[azure]
native_metadata_enabled = true
[secrets]
secrets_file = "{}"
[additional_headers]
cache_control = "no-cache"
[list_cache]
enabled = false
[stat_cache]
enabled = false
[metadata_cache]
enabled = false
[small_object_cache]
l2_enabled = false
small_object_size = "128KiB"
[metadata_key_remapper.mappings]
"nv-omniverse-nucleus-hash" = "nv_omniverse_nucleus_hash"
{}"#,
            secrets_file.to_str().unwrap(),
            redis_block.unwrap_or(""),
        );
        fs::write(&path, config).unwrap();
        path
    }

    fn create_secrets_file(tmp_dir: &Path, env: &EnvConfig, aws_assumed_role_arn: &Option<String>) -> PathBuf {
        let path = tmp_dir.join("secrets.json");
        let mut file = File::create(&path).unwrap();
        let names = test_names();
        let minio = |bucket: &str| format!("http://{}/{bucket}", env.minio_address);
        let mut azure = serde_json::Map::new();
        azure.insert(
            TestNames::azure_host(&names.azure_private_account),
            serde_json::json!({
                "AZURE_TENANT_ID": env.azure_tenant_id,
                "AZURE_CLIENT_ID": env.azure_client_id,
                "AZURE_CLIENT_SECRET": env.azure_client_secret,
            }),
        );
        azure.insert(
            TestNames::azure_host(&names.azure_versioned_account),
            serde_json::json!({
                "AZURE_STORAGE_KEY": env.azure_storage_key_versioned,
            }),
        );
        let mut s3 = serde_json::Map::new();
        for bucket in [&names.s3_bucket, &names.s3_versioned_bucket] {
            s3.insert(
                TestNames::s3_host(bucket),
                serde_json::json!({
                    "AWS_ACCESS_KEY_ID": env.aws_access_key_id,
                    "AWS_SECRET_ACCESS_KEY": env.aws_secret_access_key,
                    "AWS_ASSUMED_ROLE_ARN": aws_assumed_role_arn,
                }),
            );
        }
        s3.insert(
            minio(&names.s3_bucket),
            serde_json::json!({
                "AWS_ACCESS_KEY_ID": env.minio_access_key_id,
                "AWS_SECRET_ACCESS_KEY": env.minio_secret_access_key,
                "AWS_ENDPOINT_URL": format!("http://{}", env.minio_address),
                "AWS_REGION": env.minio_region,
            }),
        );
        s3.insert(
            minio(&names.s3_public_bucket),
            serde_json::json!({
                "AWS_ENDPOINT_URL": format!("http://{}", env.minio_address),
                "AWS_BUCKET": names.s3_public_bucket,
                "AWS_REGION": env.minio_region,
            }),
        );
        s3.insert(
            minio(&names.s3_versioned_bucket),
            serde_json::json!({
                "AWS_ACCESS_KEY_ID": env.minio_access_key_id,
                "AWS_SECRET_ACCESS_KEY": env.minio_secret_access_key,
                "AWS_ENDPOINT_URL": format!("http://{}", env.minio_address),
                "AWS_REGION": env.minio_region,
            }),
        );
        s3.insert(
            minio(&names.s3_empty_bucket),
            serde_json::json!({
                "AWS_ACCESS_KEY_ID": env.minio_access_key_id,
                "AWS_SECRET_ACCESS_KEY": env.minio_secret_access_key,
                "AWS_ENDPOINT_URL": format!("http://{}", env.minio_address),
                "AWS_BUCKET": names.s3_empty_bucket,
                "AWS_REGION": env.minio_region,
            }),
        );
        for bucket in [&names.s3_bucket, &names.s3_empty_bucket] {
            s3.insert(
                TestNames::gcs_host_path(bucket),
                serde_json::json!({
                    "AWS_ACCESS_KEY_ID": env.gcp_access_key_id,
                    "AWS_SECRET_ACCESS_KEY": env.gcp_secret_access_key,
                    "AWS_ENDPOINT_URL": "https://storage.googleapis.com",
                }),
            );
        }
        let data = serde_json::json!({
            "storage-service-azure": azure,
            "storage-service-s3": s3,
        })
        .to_string();
        file.write_all(data.as_bytes()).unwrap();
        path
    }

    fn create_cdn_secrets_file(tmp_dir: &Path, env: &EnvConfig) -> PathBuf {
        let path = tmp_dir.join("cdn_secrets.json");
        let mut file = File::create(&path).unwrap();
        let names = test_names();
        let mut azure = serde_json::Map::new();
        azure.insert(
            TestNames::azure_host(&names.azure_public_account),
            serde_json::json!({
                "AZURE_FRONT_DOOR_URL": env.azure_public_front_door_url,
            }),
        );
        azure.insert(
            TestNames::azure_host(&names.azure_private_account),
            serde_json::json!({
                "AZURE_FRONT_DOOR_URL": env.azure_private_front_door_url,
                "AZURE_TENANT_ID": env.azure_tenant_id,
                "AZURE_CLIENT_ID": env.azure_client_id,
                "AZURE_CLIENT_SECRET": env.azure_client_secret,
            }),
        );
        azure.insert(
            TestNames::azure_host(&names.azure_versioned_account),
            serde_json::json!({
                "AZURE_FRONT_DOOR_URL": env.azure_versioned_front_door_url,
                "AZURE_STORAGE_KEY": env.azure_storage_key_versioned,
            }),
        );
        let mut s3 = serde_json::Map::new();
        s3.insert(
            TestNames::s3_host(&names.s3_public_bucket),
            serde_json::json!({
                "CLOUDFRONT": {
                    "DISTRIBUTION_URL": env.aws_cloudfront_public_url,
                },
            }),
        );
        s3.insert(
            TestNames::s3_host(&names.s3_bucket),
            serde_json::json!({
                "AWS_ACCESS_KEY_ID": env.aws_access_key_id,
                "AWS_SECRET_ACCESS_KEY": env.aws_secret_access_key,
                "CLOUDFRONT": {
                    "DISTRIBUTION_URL": env.aws_cloudfront_private_url,
                    "KEY_PAIR_ID": env.aws_cloudfront_private_key_ecdsa_id,
                    "PRIVATE_KEY_PEM": env.aws_cloudfront_private_key_ecdsa,
                },
            }),
        );
        s3.insert(
            TestNames::s3_host(&names.s3_versioned_bucket),
            serde_json::json!({
                "AWS_ACCESS_KEY_ID": env.aws_access_key_id,
                "AWS_SECRET_ACCESS_KEY": env.aws_secret_access_key,
                "CLOUDFRONT": {
                    "DISTRIBUTION_URL": env.aws_cloudfront_versioned_url,
                    "KEY_PAIR_ID": env.aws_cloudfront_private_key_rsa_id,
                    "PRIVATE_KEY_PEM": env.aws_cloudfront_private_key_rsa,
                },
            }),
        );
        let data = serde_json::json!({
            "storage-service-azure": azure,
            "storage-service-s3": s3,
        })
        .to_string();
        file.write_all(data.as_bytes()).unwrap();
        path
    }

    fn create_list_cache_config(tmp_dir: &Path, secrets_file: &Path, redis_block: Option<&str>) -> PathBuf {
        let path = tmp_dir.join("list_cache_config.toml");
        let config = format!(
            r#"
[secrets]
secrets_file = "{}"
[metrics]
enabled = true
addr = "0.0.0.0"
port = 0
[additional_headers]
cache_control = "no-cache"
[list_cache]
l2_enabled = true
invalidate_on_update = true
[stat_cache]
enabled = false
[metadata_cache]
enabled = false
[small_object_cache]
l2_enabled = true
small_object_size = "128KiB"
[metadata_key_remapper.mappings]
"nv-omniverse-nucleus-hash" = "nv_omniverse_nucleus_hash"
{}"#,
            secrets_file.to_str().unwrap(),
            redis_block.unwrap_or(""),
        );
        fs::write(&path, config).unwrap();
        path
    }

    fn create_stat_cache_config(tmp_dir: &Path, secrets_file: &Path, redis_block: Option<&str>) -> PathBuf {
        let path = tmp_dir.join("stat_cache_config.toml");
        let config = format!(
            r#"
[secrets]
secrets_file = "{}"
[metrics]
enabled = true
addr = "0.0.0.0"
port = 0
[additional_headers]
cache_control = "no-cache"
[list_cache]
enabled = false
[stat_cache]
l2_enabled = true
invalidate_on_update = true
[metadata_cache]
enabled = true
l2_enabled = true
invalidate_on_update = true
[small_object_cache]
l2_enabled = true
small_object_size = "128KiB"
[manifest]
l2_enabled = true
[metadata_key_remapper.mappings]
"nv-omniverse-nucleus-hash" = "nv_omniverse_nucleus_hash"
{}"#,
            secrets_file.to_str().unwrap(),
            redis_block.unwrap_or(""),
        );
        fs::write(&path, config).unwrap();
        path
    }

    fn create_no_cache_config(tmp_dir: &Path, secrets_file: &Path, redis_block: Option<&str>) -> PathBuf {
        let path = tmp_dir.join("no_cache_config.toml");
        let config = format!(
            r#"
[secrets]
secrets_file = "{}"
[additional_headers]
cache_control = "no-cache"
[list_cache]
enabled = false
[stat_cache]
enabled = false
[metadata_cache]
enabled = false
[small_object_cache]
enabled = false
[metadata_key_remapper.mappings]
"nv-omniverse-nucleus-hash" = "nv_omniverse_nucleus_hash"
{}"#,
            secrets_file.to_str().unwrap(),
            redis_block.unwrap_or(""),
        );
        fs::write(&path, config).unwrap();
        path
    }

    /// Config for the manifest runner whose `[manifest] page_size_limit` is intentionally tiny
    /// ("256 B"), so that list / list_stat / enumerate responses are split into many small pages
    /// and the backend->manifest boundary is crossed mid-stream. Mirrors the manifest-relevant
    /// blocks of `create_stat_cache_config` (the manifest runner only consumes `[secrets]`,
    /// `[additional_headers]`, and `[manifest]`). L2 is left disabled here so paging behaviour is
    /// deterministic regardless of whether Redis is configured.
    fn create_manifest_small_page_config(tmp_dir: &Path, secrets_file: &Path, redis_block: Option<&str>) -> PathBuf {
        let path = tmp_dir.join("manifest_small_page_config.toml");
        let config = format!(
            r#"
[secrets]
secrets_file = "{}"
[additional_headers]
cache_control = "no-cache"
[list_cache]
enabled = false
[stat_cache]
enabled = false
[metadata_cache]
enabled = false
[small_object_cache]
enabled = false
[manifest]
page_size_limit = "256 B"
[metadata_key_remapper.mappings]
"nv-omniverse-nucleus-hash" = "nv_omniverse_nucleus_hash"
{}"#,
            secrets_file.to_str().unwrap(),
            redis_block.unwrap_or(""),
        );
        fs::write(&path, config).unwrap();
        path
    }
}

fn build_s3_url_map(minio_address: &str) -> HashMap<Backend, HashMap<Access, String>> {
    let names = test_names();
    let s3 = |bucket: &str| format!("{}/", TestNames::s3_url(bucket));
    let gcs = |bucket: &str| format!("{}/", TestNames::gcs_url(bucket));
    let minio = |bucket: &str| format!("http://{minio_address}/{bucket}/");
    let mut urls = HashMap::from([
        (
            Backend::S3,
            HashMap::from([
                (Access::Public, s3(&names.s3_public_bucket)),
                (Access::PrivateKey, s3(&names.s3_bucket)),
                (Access::Versioned, s3(&names.s3_versioned_bucket)),
                (Access::Empty, s3(&names.s3_empty_bucket)),
            ]),
        ),
        (
            // We can't create public buckets in GCP, and GCP doesn't support versioning in its S3-compatible API
            Backend::GCPS3Compat,
            HashMap::from([
                (Access::PrivateKey, gcs(&names.s3_bucket)),
                (Access::Empty, gcs(&names.s3_empty_bucket)),
            ]),
        ),
    ]);

    // Only add Minio if the server is running
    if StorageFixture::check_minio_server(minio_address.to_string()) {
        urls.insert(
            Backend::Minio,
            HashMap::from([
                (Access::Public, minio(&names.s3_public_bucket)),
                (Access::PrivateKey, minio(&names.s3_bucket)),
                (Access::Versioned, minio(&names.s3_versioned_bucket)),
                (Access::Empty, minio(&names.s3_empty_bucket)),
            ]),
        );
    } else if std::env::var("OMNI_STORAGE_SERVICE_MINIO_REQUIRED").is_ok() {
        panic!("Minio server not running, but required by test suite");
    } else {
        warn!(
            "Minio server not running, skipping Minio tests. Run 'docker run --rm -p 9000:9000 -p 9001:9001 \
             minio/minio:RELEASE.2025-04-22T22-12-26Z server /data --console-address :9001' \
             to launch it, then see the integration-tests README for the buckets and credentials the suite expects"
        );
    }

    urls
}

fn build_azure_url_map() -> HashMap<Backend, HashMap<Access, String>> {
    let names = test_names();
    let unittests = |account: &str| {
        format!(
            "{}/{AZURE_CONTAINER}/storage-service-unittests/",
            TestNames::azure_url(account)
        )
    };
    HashMap::from([(
        Backend::Azure,
        HashMap::from([
            (
                Access::Public,
                format!(
                    "{}/{}/storage-service-unittests/",
                    TestNames::azure_url(&names.azure_public_account),
                    names.azure_public_container
                ),
            ),
            (Access::PrivateKey, unittests(&names.azure_private_account)),
            (Access::Versioned, unittests(&names.azure_versioned_account)),
            (
                Access::Empty,
                format!(
                    "{}/{}/",
                    TestNames::azure_url(&names.azure_public_account),
                    names.azure_empty_container
                ),
            ),
        ]),
    )])
}

fn create_headers_config() -> HeadersConfig {
    HeadersConfig::new(
        HashMap::from([
            ("x-request-id".to_string(), "test-request-id".to_string()),
            ("x-omniverse-request-id".to_string(), "zomgwtfbbq".to_string()),
            ("authorization".to_string(), "Bearer test-token".to_string()),
            (
                "traceparent".to_string(),
                "00-12345678901234567890123456789012-1234567890123456-01".to_string(),
            ),
            (
                "tracestate".to_string(),
                "ro=00f067aa0ba902b7,r3=00f067aa0ba902b7,r3=00f067aa0ba902b7".to_string(),
            ),
            ("baggage".to_string(), "key1=value1,key2=value2".to_string()),
        ]),
        HashMap::from([
            ("cache_control".to_string(), "no-cache".to_string()),
            ("x-request-id".to_string(), "test-request-id".to_string()),
            (
                "traceparent".to_string(),
                "00-12345678901234567890123456789012-1234567890123456-01".to_string(),
            ),
            (
                "tracestate".to_string(),
                "ro=00f067aa0ba902b7,r3=00f067aa0ba902b7,r3=00f067aa0ba902b7".to_string(),
            ),
            ("baggage".to_string(), "key1=value1,key2=value2".to_string()),
        ]),
    )
}

fn create_s3_runner(config_file: &Path, headers_config: &HeadersConfig, redis: Option<Pool>) -> StorageServiceRunner {
    let path = config_file.to_str().unwrap().to_string();
    let service = block_on_offthread(async move {
        create_storage_service(
            path,
            "s3",
            |figment, secrets| {
                Ok(std::sync::Arc::new(storage_core::s3::Storage::from_figment(
                    figment, secrets,
                )?))
            },
            redis,
        )
        .await
    });
    StorageServiceRunner::new::<IdentityLayer, tonic::body::Body>(service, headers_config.clone())
}

fn create_azure_runner(
    config_file: &Path,
    headers_config: &HeadersConfig,
    redis: Option<Pool>,
) -> StorageServiceRunner {
    let path = config_file.to_str().unwrap().to_string();
    let service = block_on_offthread(async move {
        create_storage_service(
            path,
            "azure",
            |figment, secrets| {
                Ok(std::sync::Arc::new(storage_core::azure::Storage::from_figment(
                    figment, secrets,
                )?))
            },
            redis,
        )
        .await
    });
    StorageServiceRunner::new::<IdentityLayer, tonic::body::Body>(service, headers_config.clone())
}

fn create_s3_manifest_runner(
    config_file: &Path,
    headers_config: &HeadersConfig,
    redis: Option<Pool>,
) -> StorageServiceRunner {
    let path = config_file.to_str().unwrap().to_string();
    let service = block_on_offthread(async move {
        create_manifest_service(
            path,
            |figment, secrets| {
                Ok(std::sync::Arc::new(storage_core::s3::Storage::from_figment(
                    figment, secrets,
                )?))
            },
            redis,
        )
        .await
    });
    StorageServiceRunner::new::<IdentityLayer, tonic::body::Body>(service, headers_config.clone())
}

fn create_azure_manifest_runner(
    config_file: &Path,
    headers_config: &HeadersConfig,
    redis: Option<Pool>,
) -> StorageServiceRunner {
    let path = config_file.to_str().unwrap().to_string();
    let service = block_on_offthread(async move {
        create_manifest_service(
            path,
            |figment, secrets| {
                Ok(std::sync::Arc::new(storage_core::azure::Storage::from_figment(
                    figment, secrets,
                )?))
            },
            redis,
        )
        .await
    });
    StorageServiceRunner::new::<IdentityLayer, tonic::body::Body>(service, headers_config.clone())
}

fn create_s3_router_runner(
    config_file: &Path,
    headers_config: &HeadersConfig,
    mock: Arc<MockMetadata>,
    redis: Option<Pool>,
) -> StorageServiceRunner {
    let path = config_file.to_str().unwrap().to_string();
    let service = block_on_offthread(async move {
        create_metadata_router_storage_service(
            path,
            "s3",
            |figment, secrets| {
                Ok(std::sync::Arc::new(storage_core::s3::Storage::from_figment(
                    figment, secrets,
                )?))
            },
            mock,
            redis,
        )
        .await
    });
    StorageServiceRunner::new::<IdentityLayer, tonic::body::Body>(service, headers_config.clone())
}

fn create_azure_router_runner(
    config_file: &Path,
    headers_config: &HeadersConfig,
    mock: Arc<MockMetadata>,
    redis: Option<Pool>,
) -> StorageServiceRunner {
    let path = config_file.to_str().unwrap().to_string();
    let service = block_on_offthread(async move {
        create_metadata_router_storage_service(
            path,
            "azure",
            |figment, secrets| {
                Ok(std::sync::Arc::new(storage_core::azure::Storage::from_figment(
                    figment, secrets,
                )?))
            },
            mock,
            redis,
        )
        .await
    });
    StorageServiceRunner::new::<IdentityLayer, tonic::body::Body>(service, headers_config.clone())
}

#[macro_export]
macro_rules! assert_metric {
    ($fx:expr, $name:expr, [$($k:expr => $v:expr),* $(,)?], $hint:expr) => {{
        $crate::assert_metric!($fx, $name, [$($k => $v),*], $hint, 0);
    }};
    ($fx:expr, $name:expr, [$($k:expr => $v:expr),* $(,)?], $hint:expr, $min:expr) => {{
        let labels = &[$(($k, $v)),*];
        assert!(
            $fx.get_metric_value($name, labels).unwrap_or(0) > $min,
            "\n=== Filtered metrics for {} {} (expected > {}) ===\n{}\n========================================================",
            $name,
            $hint,
            $min,
            $fx.print_filtered_metrics($name, labels),
        );
    }};
}

#[macro_export]
macro_rules! assert_method_metric {
    ($fx:expr, $backend:expr, $api:expr, $name:expr, $method:expr, [$($k:expr => $v:expr),* $(,)?]) => {{
        $crate::assert_method_metric!($fx, $backend, $api, $name, $method, [$($k => $v),*], 0);
    }};
    ($fx:expr, $backend:expr, $api:expr, $name:expr, $method:expr, [$($k:expr => $v:expr),* $(,)?], $min:expr) => {{
        let labels = &[$(($k, $v)),*];
        assert!(
            $fx
                .get_method_metric_value($backend, $api, $name, $method, labels)
                .unwrap_or(0)
                > $min,
            "\n=== Filtered metrics for {} method={} backend={:?}, api_version={:?} (expected > {}) ===\n{}\n========================================================",
            $name,
            $method,
            $backend,
            $api,
            $min,
            $fx.print_filtered_method_metrics(
                $name,
                $backend,
                $api,
                &[("method", $method), $(($k, $v)),*],
            ),
        );
    }};
}
