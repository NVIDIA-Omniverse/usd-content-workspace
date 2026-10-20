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

mod common;

use common::{
    clients::discovery::types::v2alpha::{
        GrpcEndpoint, GrpcService, ListServicesRequest, ListServicesResponse, RestEndpoint, RestService, RestVersion,
        RestVersionDetail, ServiceEntry,
    },
    service::Service as StorageService,
    types::Interface,
};
use rstest::rstest;
use tonic::Code;

#[rstest]
#[test_log::test(tokio::test)]
async fn test_list_services_returns_the_storage_service(
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    let storage_service = StorageService::builder().enable_az(false).start();
    let client = storage_service.discovery_v2alpha_client(interface).await;
    let expected = ListServicesResponse {
        services: vec![storage_entry(false)],
        next_page_token: String::new(),
    };

    let response = client
        .list_services(Default::default())
        .await
        .expect("services list failed");

    assert_eq!(response, expected);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_list_services_includes_metadata_endpoint_when_enabled(
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    let storage_service = StorageService::builder()
        .enable_az(true)
        .enable_postgres_metadata()
        .start();
    let client = storage_service.discovery_v2alpha_client(interface).await;
    let expected = ListServicesResponse {
        services: vec![storage_entry(true)],
        next_page_token: String::new(),
    };

    let response = client
        .list_services(Default::default())
        .await
        .expect("services list failed");

    assert_eq!(response, expected);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_list_services_returns_the_single_entry_within_a_page(
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    let storage_service = StorageService::builder().enable_az(false).start();
    let client = storage_service.discovery_v2alpha_client(interface).await;

    // storage-service is one logical service, so any page holds the whole list
    // and there is no continuation token.
    let response = client
        .list_services(ListServicesRequest {
            page_size: 1,
            page_token: String::new(),
        })
        .await
        .expect("services list failed");

    assert_eq!(response.services, vec![storage_entry(false)]);
    assert_eq!(response.next_page_token, "");
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_list_services_rejects_invalid_page_token(
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    let storage_service = StorageService::builder().enable_az(false).start();
    let client = storage_service.discovery_v2alpha_client(interface).await;

    let status = client
        .list_services(ListServicesRequest {
            page_size: 0,
            page_token: "bogus".into(),
        })
        .await
        .expect_err("invalid page token must be rejected");

    assert_eq!(status.code(), Code::InvalidArgument);

    // `"1"` == the advertised service count: an end-boundary token the server never issues.
    let status = client
        .list_services(ListServicesRequest {
            page_size: 0,
            page_token: "1".into(),
        })
        .await
        .expect_err("a page token equal to the service count must be rejected");

    assert_eq!(status.code(), Code::InvalidArgument);
}

// The advertised REST mount base must actually resolve on the running service once the client adds
// the resource name -- a v1beta REST path would 404. Take the capabilities endpoint out of the
// ListServices response and issue a real request under `<base>/<name>`, asserting it is served (not
// 404), so the advertisement is self-checking rather than a second copy of the same hand-built list.
#[rstest]
#[test_log::test(tokio::test)]
async fn test_advertised_rest_base_path_is_served() {
    let storage_service = StorageService::builder().enable_az(false).start();
    let response = storage_service
        .discovery_v2alpha_client(Interface::Grpc)
        .await
        .list_services(Default::default())
        .await
        .expect("services list failed");

    let endpoint = response
        .services
        .iter()
        .flat_map(|service| service.rest_services.iter())
        .flat_map(|rest| rest.endpoints.iter())
        .find(|endpoint| endpoint.name == "capabilities")
        .expect("capabilities REST endpoint advertised");
    let base = endpoint
        .versions
        .first()
        .and_then(|version| version.url_detail.as_ref())
        .map(|detail| detail.path.as_str())
        .expect("capabilities REST base path advertised");
    assert_eq!(base, "/v1alpha", "REST must advertise the version mount base");

    // The client combines the mount base with the resource name; `<base>/<name>/services` is a real
    // capabilities REST route, while a bogus `/v1beta/...` base would 404.
    let name = &endpoint.name;
    let url = format!("{}{base}/{name}/services", storage_service.rest_endpoint());
    let status = reqwest::Client::new()
        .get(&url)
        .send()
        .await
        .expect("request to the advertised REST path failed")
        .status();
    assert_ne!(
        status,
        reqwest::StatusCode::NOT_FOUND,
        "advertised REST base path {base} does not resolve ({url})"
    );
}

/// The single `storage` service entry: one endpoint per logically grouped API,
/// each pointing at the group's base path and gRPC service name.
fn storage_entry(metadata_enabled: bool) -> ServiceEntry {
    let mut groups = vec![
        api_group(
            "fileobject",
            &[
                (
                    "v1alpha",
                    "nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService",
                ),
                ("v1beta", "nvidia.omniverse.storage.fileobject.v1beta.FileObjectService"),
            ],
        ),
        api_group(
            "filefolder",
            &[
                (
                    "v1alpha",
                    "nvidia.omniverse.storage.filefolder.v1alpha.FileFolderService",
                ),
                ("v1beta", "nvidia.omniverse.storage.filefolder.v1beta.FileFolderService"),
            ],
        ),
        api_group(
            "capabilities",
            &[
                (
                    "v1alpha",
                    "nvidia.omniverse.storage.capabilities.v1alpha.CapabilitiesService",
                ),
                (
                    "v1beta",
                    "nvidia.omniverse.storage.capabilities.v1beta.CapabilitiesService",
                ),
            ],
        ),
        api_group(
            "versioning",
            &[
                (
                    "v1alpha",
                    "nvidia.omniverse.storage.versioning.v1alpha.VersioningService",
                ),
                ("v1beta", "nvidia.omniverse.storage.versioning.v1beta.VersioningService"),
            ],
        ),
    ];

    if metadata_enabled {
        // The metadata group is served under two packages: the storage-namespace
        // `nvidia.omniverse.storage.metadata.v1alpha` and the top-level
        // `nvidia.omniverse.metadata.v1beta`. Both are served over gRPC and REST, at
        // `/v1alpha/metadata` and `/v1beta/metadata` respectively.
        groups.push(ApiGroup {
            grpc: vec![
                "nvidia.omniverse.storage.metadata.v1alpha.MetadataService".into(),
                "nvidia.omniverse.metadata.v1beta.MetadataService".into(),
            ],
            rest: RestEndpoint {
                name: "metadata".into(),
                versions: ["v1alpha", "v1beta"]
                    .into_iter()
                    .map(|version| RestVersion {
                        version: version.into(),
                        url_detail: Some(RestVersionDetail {
                            path: format!("/{version}"),
                        }),
                    })
                    .collect(),
            },
        });
    }

    let grpc_endpoints: Vec<GrpcEndpoint> = groups
        .iter()
        .flat_map(|group| group.grpc.iter())
        .map(|service_name| GrpcEndpoint {
            service_name: service_name.clone(),
        })
        .collect();

    ServiceEntry {
        service_name: "storage".into(),
        grpc_services: vec![GrpcService {
            endpoints: grpc_endpoints,
        }],
        rest_services: vec![RestService {
            endpoints: groups.into_iter().map(|group| group.rest).collect(),
        }],
    }
}

struct ApiGroup {
    grpc: Vec<String>,
    rest: RestEndpoint,
}

fn api_group(name: &str, versions: &[(&str, &str)]) -> ApiGroup {
    ApiGroup {
        grpc: versions.iter().map(|(_, service)| (*service).into()).collect(),
        rest: RestEndpoint {
            name: name.into(),
            // REST is a single `/v1alpha` space; v1beta is gRPC-only, so only v1alpha is advertised.
            versions: versions
                .iter()
                .filter(|(version, _)| *version == "v1alpha")
                .map(|(version, _)| RestVersion {
                    version: (*version).into(),
                    // The advertised path is the version mount base only; the resource name is
                    // carried by `RestEndpoint.name`.
                    url_detail: Some(RestVersionDetail {
                        path: format!("/{version}"),
                    }),
                })
                .collect(),
        },
    }
}
