// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::fs::{self};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use std::sync::mpsc;
use std::thread::JoinHandle;

use crate::death_signal::DeathSignal;
use crate::headers::HeadersConfig;
use storage_core_util::ci::read_ports;
use tracing::info;

/// The interface to use for connecting to the Storage Service.
/// This is used to select which client to use for connecting to the Storage Service.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Interface {
    Grpc,
    Rest,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ApiVersion {
    V1Alpha,
    V1Beta,
}

impl std::fmt::Display for ApiVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiVersion::V1Alpha => write!(f, "v1alpha"),
            ApiVersion::V1Beta => write!(f, "v1beta"),
        }
    }
}

impl ApiVersion {
    pub fn as_str(&self) -> &'static str {
        match self {
            ApiVersion::V1Alpha => "v1alpha",
            ApiVersion::V1Beta => "v1beta",
        }
    }
}

/// The Storage Service handles launching the storage service process and providing clients to connect to it.
pub struct StorageService {
    /// The thread that launched the storage service process.
    _launch_thread: JoinHandle<()>,
    /// The address of the storage service.
    pub address: String,
    /// The port of the REST server.
    pub rest_port: u16,
    /// The headers configuration to use for the clients.
    headers_config: HeadersConfig,
}

impl StorageService {
    /// Create a new Storage Service.
    pub fn new(
        exe: impl Into<String>,
        configuration_file: impl Into<String>,
        logs_path: PathBuf,
        headers_config: HeadersConfig,
    ) -> Self {
        // Create a channel to receive ports from the launch thread
        let (tx, rx) = mpsc::channel();

        // This is a little jank, but the deathsignal code use PR_SET_PDEATHSIG, which will send the
        // death signal to the child when the parent *thread* exits, so we need to spawn the subprocess
        // from a thread that lives as long as the process
        let exe: String = exe.into();
        let configuration_file: String = configuration_file.into();
        let launch_exe: String = exe.clone();
        let launch_configuration_file: String = configuration_file.clone();
        let launch_thread = std::thread::spawn(move || {
            let mut command = Command::new(launch_exe);
            command.death_signal();

            let cmd = command
                .args(["--configuration-file", &launch_configuration_file])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());

            info!(?cmd, "launching storage service process");
            let mut child = cmd.spawn().unwrap();

            let mut stdout_file = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(logs_path.join(format!("storage-service.{}.stdout.log", child.id())))
                .unwrap();

            let mut stderr_file = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(logs_path.join(format!("storage-service.{}.stderr.log", child.id())))
                .unwrap();

            // Capture and log subprocess output at debug level
            let stdout_pipe = child.stdout.take().unwrap();
            let stdout_thread = std::thread::spawn(move || {
                for line in BufReader::new(stdout_pipe).lines() {
                    let line = line.unwrap();
                    info!(stdout = line);
                    writeln!(stdout_file, "{line}").unwrap();
                }
            });

            let stderr_pipe = child.stderr.take().unwrap();
            let stderr_thread = std::thread::spawn(move || {
                for line in BufReader::new(stderr_pipe).lines() {
                    let line = line.unwrap();
                    info!(stderr = line);
                    writeln!(stderr_file, "{line}").unwrap();
                }
            });

            tx.send(child.id()).unwrap();

            stdout_thread.join().expect("failed to join stdout thread");
            stderr_thread.join().expect("failed to join stderr thread");
            child
                .wait()
                .unwrap_or_else(|e| panic!("child process failed. error={e:?} command={command:?}"));
        });

        let child_id = rx.recv().unwrap();
        let ports = read_ports(child_id, None).unwrap();
        info!("Storage Service Executable: {exe} Configuration File: {configuration_file} listening on {ports:?}");
        let address = format!("http://127.0.0.1:{}", ports.grpc_port);

        Self {
            _launch_thread: launch_thread,
            address,
            rest_port: ports.rest_port,
            headers_config,
        }
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
            Interface::Rest => Box::new(crate::clients::rest::capabilities::v1alpha::CapabilitiesClient::new(
                self.rest_port,
                self.headers_config.clone(),
            )),
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
