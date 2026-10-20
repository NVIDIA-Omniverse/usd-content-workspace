// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::mpsc as std_mpsc;
use std::thread;

use crate::headers::HeadersConfig;
use storage_core::storage::StorageServiceServer;
use storage_core_util::ci::PortConfig;
use tokio::net::{TcpListener, TcpSocket};
use tokio::sync::mpsc;
use tokio_stream::wrappers::TcpListenerStream;

use super::service::{ApiVersion, Interface};
use crate::signals::{init_signal_handler, THREAD_REGISTRY};

/// Helper to launch a storage service in a new thread and provide clients attached to it.
pub struct StorageServiceRunner {
    /// The address of the storage service.
    pub address: String,
    /// The port of the REST server.
    pub rest_port: u16,
    /// The headers configuration to use for the clients.
    headers_config: HeadersConfig,
}

impl StorageServiceRunner {
    /// Create a new Storage Service.
    ///
    /// This function takes a StorageServiceServer, and launches it in a new thread.
    /// This function will handle finding available ports for the storage service and provides clients attached to those ports.
    /// This function also handles setting up the metrics server, using the provied label.
    ///
    /// This function will also set up the signal handler to shutdown all registered threads.
    ///
    /// Note: This function uses `TracePropagationDisabled` for the REST server, which means no
    /// trace propagation will be applied. This is appropriate for integration tests where the
    /// tracing stack may not be configured.
    pub fn new<L, ResBody>(storage_service: StorageServiceServer<L>, headers_config: HeadersConfig) -> Self
    where
        L: tower::Layer<tonic::service::Routes> + Clone + Send + Sync + 'static,
        L::Service: tower::Service<http::Request<tonic::body::Body>, Response = http::Response<ResBody>> + Clone + Send + 'static,
        <<L as tower::Layer<tonic::service::Routes>>::Service as tower::Service<http::Request<tonic::body::Body>>>::Future: Send,
        <<L as tower::Layer<tonic::service::Routes>>::Service as tower::Service<http::Request<tonic::body::Body>>>::Error:
            Into<tower::BoxError> + Send,
        ResBody: http_body::Body<Data = bytes::Bytes> + Send + 'static,
        ResBody::Error: Into<tower::BoxError>,
    {
        init_signal_handler();

        let (port_config_tx, port_config_rx) = std_mpsc::channel();

        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);

        let handle = thread::spawn(move || {
            // Use a current-thread runtime instead of the default multi-thread runtime
            // (which spawns `num_cpus` worker threads per runner). Each `StorageFixture`
            // builds 8+ runners and nextest runs many test binaries in parallel, so a
            // multi-thread runtime per runner blows past `RLIMIT_NPROC` on hosts with
            // many cores. Tests are network-bound and yield properly across `.await`,
            // and the blocking pool (used by `tokio::fs::*`, the secrets watcher, etc.)
            // is unaffected — `enable_all` keeps it available, lazy as before.
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async move {
                let (grpc_incoming, grpc_port) = Self::generate_grpc_incoming().unwrap();
                let (rest_incoming, rest_port) = Self::generate_rest_incoming().unwrap();

                // Send port configuration back to the main thread
                let port_config = PortConfig { grpc_port, rest_port };
                port_config_tx.send(port_config).unwrap();

                // Run server with shutdown signal
                tokio::select! {
                    _ = shutdown_rx.recv() => {
                        tracing::info!("Shutdown signal received, stopping storage service");
                    },
                    result = storage_service.serve_with_incoming(
                        Some(grpc_incoming),
                        Some(rest_incoming),
                    ) => {
                        result.unwrap();
                    }
                }
            });
        });

        // Receive the port configuration from the spawned thread
        let port_config = port_config_rx.recv().unwrap();

        // Register with global registry
        if let Some(registry) = THREAD_REGISTRY.get() {
            if let Ok(mut registry) = registry.lock() {
                registry.register_thread(handle, shutdown_tx.clone());
            }
        }

        Self {
            address: format!("http://127.0.0.1:{}", port_config.grpc_port),
            rest_port: port_config.rest_port,
            headers_config,
        }
    }

    fn generate_grpc_incoming() -> Result<(TcpListenerStream, u16), Box<dyn std::error::Error>> {
        let socket = TcpSocket::new_v4()?;
        socket.set_reuseaddr(true)?;
        socket.set_reuseport(true)?;
        socket.set_nodelay(true)?;
        socket.set_keepalive(true)?;
        socket.set_linger(None)?;
        socket.bind("0.0.0.0:0".parse::<std::net::SocketAddr>()?)?;

        let port = socket.local_addr()?.port();

        Ok((TcpListenerStream::new(socket.listen(512)?), port))
    }

    fn generate_rest_incoming() -> Result<(TcpListener, u16), Box<dyn std::error::Error>> {
        let socket = TcpSocket::new_v4()?;
        socket.set_reuseaddr(true)?;
        socket.set_reuseport(true)?;
        socket.set_nodelay(true)?;
        socket.set_keepalive(true)?;
        socket.set_linger(None)?;
        socket.bind("0.0.0.0:0".parse::<std::net::SocketAddr>()?)?;

        let bound_port = socket.local_addr()?.port();

        Ok((socket.listen(512)?, bound_port))
    }

    /// Get a file object client.
    pub async fn file_object_client(
        &self,
        interface: Interface,
        api_version: ApiVersion,
    ) -> Box<dyn super::clients::traits::fileobject::FileObject> {
        match interface {
            Interface::Grpc => match api_version {
                ApiVersion::V1Alpha => Box::new(
                    crate::clients::grpc::fileobject::v1alpha::FileObjectClient::new(
                        self.address.clone(),
                        self.headers_config.clone(),
                    )
                    .await,
                ),
                ApiVersion::V1Beta => Box::new(
                    crate::clients::grpc::fileobject::v1beta::FileObjectClient::new(
                        self.address.clone(),
                        self.headers_config.clone(),
                    )
                    .await,
                ),
            },
            Interface::Rest => match api_version {
                ApiVersion::V1Alpha => Box::new(crate::clients::rest::fileobject::v1alpha::FileObjectClient::new(
                    self.rest_port,
                    self.headers_config.clone(),
                )),
                ApiVersion::V1Beta => Box::new(crate::clients::rest::fileobject::v1beta::FileObjectClient::new(
                    self.rest_port,
                    self.headers_config.clone(),
                )),
            },
        }
    }

    /// Get a capabilities client.
    pub async fn capabilities_client(
        &self,
        interface: Interface,
        api_version: ApiVersion,
    ) -> Box<dyn super::clients::traits::capabilities::Capabilities> {
        match interface {
            Interface::Grpc => match api_version {
                ApiVersion::V1Alpha => Box::new(
                    crate::clients::grpc::capabilities::v1alpha::CapabilitiesClient::new(
                        self.address.clone(),
                        self.headers_config.clone(),
                    )
                    .await,
                ),
                ApiVersion::V1Beta => Box::new(
                    crate::clients::grpc::capabilities::v1beta::CapabilitiesClient::new(
                        self.address.clone(),
                        self.headers_config.clone(),
                    )
                    .await,
                ),
            },
            Interface::Rest => match api_version {
                ApiVersion::V1Alpha => Box::new(crate::clients::rest::capabilities::v1alpha::CapabilitiesClient::new(
                    self.rest_port,
                    self.headers_config.clone(),
                )),
                ApiVersion::V1Beta => Box::new(crate::clients::rest::capabilities::v1beta::CapabilitiesClient::new(
                    self.rest_port,
                    self.headers_config.clone(),
                )),
            },
        }
    }

    /// Get a file folder client.
    pub async fn file_folder_client(
        &self,
        interface: Interface,
        api_version: ApiVersion,
    ) -> Box<dyn super::clients::traits::filefolder::FileFolder> {
        match interface {
            Interface::Grpc => match api_version {
                ApiVersion::V1Alpha => Box::new(
                    crate::clients::grpc::filefolder::v1alpha::FileFolderClient::new(
                        self.address.clone(),
                        self.headers_config.clone(),
                    )
                    .await,
                ),
                ApiVersion::V1Beta => Box::new(
                    crate::clients::grpc::filefolder::v1beta::FileFolderClient::new(
                        self.address.clone(),
                        self.headers_config.clone(),
                    )
                    .await,
                ),
            },
            Interface::Rest => match api_version {
                ApiVersion::V1Alpha => Box::new(crate::clients::rest::filefolder::v1alpha::FileFolderClient::new(
                    self.rest_port,
                    self.headers_config.clone(),
                )),
                ApiVersion::V1Beta => Box::new(crate::clients::rest::filefolder::v1beta::FileFolderClient::new(
                    self.rest_port,
                    self.headers_config.clone(),
                )),
            },
        }
    }

    /// Get a versioning client.
    pub async fn versioning_client(
        &self,
        interface: Interface,
        api_version: ApiVersion,
    ) -> Box<dyn super::clients::traits::versioning::Versioning> {
        match interface {
            Interface::Grpc => match api_version {
                ApiVersion::V1Alpha => Box::new(
                    crate::clients::grpc::versioning::v1alpha::VersioningClient::new(
                        self.address.clone(),
                        self.headers_config.clone(),
                    )
                    .await,
                ),
                ApiVersion::V1Beta => Box::new(
                    crate::clients::grpc::versioning::v1beta::VersioningClient::new(
                        self.address.clone(),
                        self.headers_config.clone(),
                    )
                    .await,
                ),
            },
            Interface::Rest => match api_version {
                ApiVersion::V1Alpha => Box::new(crate::clients::rest::versioning::v1alpha::VersioningClient::new(
                    self.rest_port,
                    self.headers_config.clone(),
                )),
                ApiVersion::V1Beta => Box::new(crate::clients::rest::versioning::v1beta::VersioningClient::new(
                    self.rest_port,
                    self.headers_config.clone(),
                )),
            },
        }
    }

    /// Get a metadata client.
    pub async fn metadata_client(
        &self,
        interface: Interface,
        api_version: ApiVersion,
    ) -> Box<dyn super::clients::traits::metadata::Metadata> {
        match interface {
            Interface::Grpc => match api_version {
                ApiVersion::V1Alpha => Box::new(
                    crate::clients::grpc::metadata::v1alpha::MetadataClient::new(
                        self.address.clone(),
                        self.headers_config.clone(),
                    )
                    .await,
                ),
                _ => panic!("Unsupported API version for metadata: {api_version:?}"),
            },
            Interface::Rest => match api_version {
                ApiVersion::V1Alpha => Box::new(crate::clients::rest::metadata::v1alpha::MetadataClient::new(
                    self.rest_port,
                    self.headers_config.clone(),
                )),
                _ => panic!("Unsupported API version for metadata: {api_version:?}"),
            },
        }
    }
}
