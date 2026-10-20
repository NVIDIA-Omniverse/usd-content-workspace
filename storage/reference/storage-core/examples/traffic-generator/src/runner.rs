// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;

use storage_core_test::clients::traits::capabilities::Capabilities;
use storage_core_test::clients::traits::filefolder::FileFolder;
use storage_core_test::clients::traits::fileobject::FileObject;
use storage_core_test::clients::traits::metadata::Metadata;
use storage_core_test::clients::traits::versioning::Versioning;
use storage_core_test::headers::HeadersConfig;
use storage_core_test::service::{ApiVersion, Interface};
use storage_core_util::info_limited;

use crate::report::TrafficReport;

/// CLI-level interface selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum InterfaceArg {
    Grpc,
    Rest,
    Both,
}

/// CLI-level API version selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ApiVersionArg {
    V1Alpha,
    V1Beta,
    Both,
}

impl ApiVersionArg {
    /// Return the list of concrete ApiVersions to iterate over.
    pub fn versions(self) -> Vec<ApiVersion> {
        match self {
            ApiVersionArg::V1Alpha => vec![ApiVersion::V1Alpha],
            ApiVersionArg::V1Beta => vec![ApiVersion::V1Beta],
            ApiVersionArg::Both => vec![ApiVersion::V1Alpha, ApiVersion::V1Beta],
        }
    }
}

impl InterfaceArg {
    /// Return the list of concrete Interfaces to iterate over.
    pub fn interfaces(self) -> Vec<Interface> {
        match self {
            InterfaceArg::Grpc => vec![Interface::Grpc],
            InterfaceArg::Rest => vec![Interface::Rest],
            InterfaceArg::Both => vec![Interface::Grpc, Interface::Rest],
        }
    }
}

/// Creates the HeadersConfig used for all client requests.
fn create_headers_config() -> HeadersConfig {
    HeadersConfig::new(
        HashMap::from([
            ("x-request-id".to_string(), "traffic-gen-request".to_string()),
            (
                "traceparent".to_string(),
                "00-12345678901234567890123456789012-1234567890123456-01".to_string(),
            ),
        ]),
        HashMap::new(), // No validation headers for traffic gen
    )
}

/// Create a FileObject client for the given interface and API version.
async fn create_file_object_client(
    grpc_addr: &str,
    rest_port: u16,
    interface: Interface,
    api_version: ApiVersion,
) -> Box<dyn FileObject> {
    let headers_config = create_headers_config();
    match interface {
        Interface::Grpc => match api_version {
            ApiVersion::V1Alpha => Box::new(
                storage_core_test::clients::grpc::fileobject::v1alpha::FileObjectClient::new(grpc_addr, headers_config)
                    .await,
            ),
            ApiVersion::V1Beta => Box::new(
                storage_core_test::clients::grpc::fileobject::v1beta::FileObjectClient::new(grpc_addr, headers_config)
                    .await,
            ),
        },
        Interface::Rest => match api_version {
            ApiVersion::V1Alpha => Box::new(
                storage_core_test::clients::rest::fileobject::v1alpha::FileObjectClient::new(rest_port, headers_config),
            ),
            ApiVersion::V1Beta => Box::new(
                storage_core_test::clients::rest::fileobject::v1beta::FileObjectClient::new(rest_port, headers_config),
            ),
        },
    }
}

/// Create a FileFolder client for the given interface and API version.
async fn create_file_folder_client(
    grpc_addr: &str,
    rest_port: u16,
    interface: Interface,
    api_version: ApiVersion,
) -> Box<dyn FileFolder> {
    let headers_config = create_headers_config();
    match interface {
        Interface::Grpc => match api_version {
            ApiVersion::V1Alpha => Box::new(
                storage_core_test::clients::grpc::filefolder::v1alpha::FileFolderClient::new(grpc_addr, headers_config)
                    .await,
            ),
            ApiVersion::V1Beta => Box::new(
                storage_core_test::clients::grpc::filefolder::v1beta::FileFolderClient::new(grpc_addr, headers_config)
                    .await,
            ),
        },
        Interface::Rest => match api_version {
            ApiVersion::V1Alpha => Box::new(
                storage_core_test::clients::rest::filefolder::v1alpha::FileFolderClient::new(rest_port, headers_config),
            ),
            ApiVersion::V1Beta => Box::new(
                storage_core_test::clients::rest::filefolder::v1beta::FileFolderClient::new(rest_port, headers_config),
            ),
        },
    }
}

/// Create a Capabilities client for the given interface and API version.
async fn create_capabilities_client(
    grpc_addr: &str,
    rest_port: u16,
    interface: Interface,
    api_version: ApiVersion,
) -> Box<dyn Capabilities> {
    let headers_config = create_headers_config();
    match interface {
        Interface::Grpc => match api_version {
            ApiVersion::V1Alpha => Box::new(
                storage_core_test::clients::grpc::capabilities::v1alpha::CapabilitiesClient::new(
                    grpc_addr,
                    headers_config,
                )
                .await,
            ),
            ApiVersion::V1Beta => Box::new(
                storage_core_test::clients::grpc::capabilities::v1beta::CapabilitiesClient::new(
                    grpc_addr,
                    headers_config,
                )
                .await,
            ),
        },
        Interface::Rest => match api_version {
            ApiVersion::V1Alpha => Box::new(
                storage_core_test::clients::rest::capabilities::v1alpha::CapabilitiesClient::new(
                    rest_port,
                    headers_config,
                ),
            ),
            ApiVersion::V1Beta => Box::new(
                storage_core_test::clients::rest::capabilities::v1beta::CapabilitiesClient::new(
                    rest_port,
                    headers_config,
                ),
            ),
        },
    }
}

/// Create a Versioning client for the given interface and API version.
async fn create_versioning_client(
    grpc_addr: &str,
    rest_port: u16,
    interface: Interface,
    api_version: ApiVersion,
) -> Box<dyn Versioning> {
    let headers_config = create_headers_config();
    match interface {
        Interface::Grpc => match api_version {
            ApiVersion::V1Alpha => Box::new(
                storage_core_test::clients::grpc::versioning::v1alpha::VersioningClient::new(grpc_addr, headers_config)
                    .await,
            ),
            ApiVersion::V1Beta => Box::new(
                storage_core_test::clients::grpc::versioning::v1beta::VersioningClient::new(grpc_addr, headers_config)
                    .await,
            ),
        },
        Interface::Rest => match api_version {
            ApiVersion::V1Alpha => Box::new(
                storage_core_test::clients::rest::versioning::v1alpha::VersioningClient::new(rest_port, headers_config),
            ),
            ApiVersion::V1Beta => Box::new(
                storage_core_test::clients::rest::versioning::v1beta::VersioningClient::new(rest_port, headers_config),
            ),
        },
    }
}

/// Create a Metadata client for the given interface and API version.
async fn create_metadata_client(
    grpc_addr: &str,
    rest_port: u16,
    interface: Interface,
    api_version: ApiVersion,
) -> Box<dyn Metadata> {
    let headers_config = create_headers_config();
    match interface {
        Interface::Grpc => match api_version {
            ApiVersion::V1Alpha => Box::new(
                storage_core_test::clients::grpc::metadata::v1alpha::MetadataClient::new(grpc_addr, headers_config)
                    .await,
            ),
            _ => panic!("Metadata does not support {api_version:?}"),
        },
        Interface::Rest => match api_version {
            ApiVersion::V1Alpha => Box::new(
                storage_core_test::clients::rest::metadata::v1alpha::MetadataClient::new(rest_port, headers_config),
            ),
            _ => panic!("Metadata does not support {api_version:?}"),
        },
    }
}

/// Run the full traffic generation workflow.
pub async fn run(
    grpc_addr: &str,
    rest_port: u16,
    base_url: &str,
    interface_arg: InterfaceArg,
    api_version_arg: ApiVersionArg,
    report: &TrafficReport,
) {
    let interfaces = interface_arg.interfaces();
    let versions = api_version_arg.versions();

    for interface in &interfaces {
        for version in &versions {
            let version_label = match version {
                ApiVersion::V1Alpha => "v1alpha",
                ApiVersion::V1Beta => "v1beta",
            };
            let interface_label = match interface {
                Interface::Grpc => "gRPC",
                Interface::Rest => "REST",
            };

            println!();
            println!("═══ Running traffic: {} / {} ═══", interface_label, version_label);

            let api_version_for_traffic = match version {
                ApiVersion::V1Alpha => ApiVersionArg::V1Alpha,
                ApiVersion::V1Beta => ApiVersionArg::V1Beta,
            };

            // Phase 1: Capabilities
            info_limited!("Phase 1: Capabilities");
            let mut cap_client = create_capabilities_client(grpc_addr, rest_port, *interface, *version).await;
            crate::capabilities::run(&mut cap_client, api_version_for_traffic, report).await;

            // Phase 2: FileObject Body Upload Lifecycle
            info_limited!("Phase 2: FileObject Body Upload Lifecycle");
            let mut fo_client = create_file_object_client(grpc_addr, rest_port, *interface, *version).await;
            crate::fileobject::run_body_lifecycle(&mut fo_client, base_url, api_version_for_traffic, report).await;

            // Phase 3: FileObject Multipart Upload Lifecycle
            info_limited!("Phase 3: FileObject Multipart Upload Lifecycle");
            let mut fo_client = create_file_object_client(grpc_addr, rest_port, *interface, *version).await;
            crate::fileobject::run_multipart_lifecycle(&mut fo_client, base_url, report).await;

            // Phase 4: FileObject Redirect Upload Lifecycle
            info_limited!("Phase 4: FileObject Redirect Upload Lifecycle");
            let mut fo_client = create_file_object_client(grpc_addr, rest_port, *interface, *version).await;
            crate::fileobject::run_redirect_lifecycle(&mut fo_client, base_url, report).await;

            // Phase 5: FileFolder
            info_limited!("Phase 5: FileFolder");
            let folder_address = format!("{base_url}traffic-gen/folder-test-{}/", rand::random::<u32>());
            let mut ff_client = create_file_folder_client(grpc_addr, rest_port, *interface, *version).await;
            crate::filefolder::run(&mut ff_client, &folder_address, api_version_for_traffic, report).await;

            // Phase 6: Versioning
            info_limited!("Phase 6: Versioning");
            let mut ver_client = create_versioning_client(grpc_addr, rest_port, *interface, *version).await;
            let mut fo_client = create_file_object_client(grpc_addr, rest_port, *interface, *version).await;
            crate::versioning::run(&mut ver_client, &mut fo_client, base_url, report).await;

            // Phase 7: Metadata
            tracing::info!("Phase 7: Metadata");
            let mut meta_client = create_metadata_client(grpc_addr, rest_port, *interface, *version).await;
            let mut fo_client = create_file_object_client(grpc_addr, rest_port, *interface, *version).await;
            crate::metadata::run(&mut meta_client, &mut fo_client, base_url, report).await;
        }
    }
}
