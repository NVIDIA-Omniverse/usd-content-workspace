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

pub mod grpc;
pub mod rest;

pub mod v2alpha {
    use std::sync::Arc;

    use storage_core::service_registry::ServiceInfoProvider;
    use storage_service_protos::nvidia::omniverse::{
        discovery::capabilities::v2alpha::{
            GrpcEndpoint, GrpcService, ListServicesResponse, RestEndpoint, RestService, RestVersion, RestVersionDetail,
            ServiceEntry,
        },
        storage::metadata,
    };

    pub const LIST_SERVICES_PATH: &str = "/v2alpha/services";

    /// The page token is not one this server issued.
    #[derive(Debug)]
    pub struct InvalidPageToken;

    #[derive(Clone)]
    pub struct Capabilities {
        pub(super) services: Arc<Vec<ServiceEntry>>,
    }

    impl Capabilities {
        pub fn new(services: Vec<ServiceEntry>) -> Self {
            Self {
                services: Arc::new(services),
            }
        }

        /// Return one page of the service list. A non-positive page size returns the
        /// whole remainder; the page token is the offset issued by a prior call.
        pub(super) fn list_services_page(
            &self,
            page_size: i32,
            page_token: &str,
        ) -> Result<ListServicesResponse, InvalidPageToken> {
            let offset = if page_token.is_empty() {
                0
            } else {
                // `next_page_token` is always a canonical positive offset strictly inside the list,
                // so reject anything else instead of replaying a page or returning an empty list.
                let offset: usize = page_token.parse().map_err(|_| InvalidPageToken)?;
                if offset == 0 || offset >= self.services.len() || offset.to_string() != page_token {
                    return Err(InvalidPageToken);
                }
                offset
            };
            let services = &self.services[offset..];

            let page_size = usize::try_from(page_size).unwrap_or_default();
            if page_size == 0 || page_size >= services.len() {
                return Ok(ListServicesResponse {
                    services: services.to_vec(),
                    next_page_token: String::new(),
                });
            }

            Ok(ListServicesResponse {
                services: services[..page_size].to_vec(),
                next_page_token: (offset + page_size).to_string(),
            })
        }
    }

    // Discovery is not a storage capability, so it advertises no `ServiceInfo` and stays out of the
    // storage capabilities `ListServices`.
    impl ServiceInfoProvider for Capabilities {}

    /// The service list this deployment advertises. `storage-service` is a single
    /// logical service ("storage"); each logically grouped API (fileobject,
    /// filefolder, ...) is advertised as one endpoint, not one route. Clients
    /// resolve a group's base path and gRPC service name here, then use its
    /// OpenAPI/gRPC spec for the individual operations. The discovery service does
    /// not advertise itself — a client is already talking to it.
    pub fn service_entries(metadata_enabled: bool) -> Vec<ServiceEntry> {
        vec![storage_service_entry(metadata_enabled)]
    }

    /// One entry for every API group `storage-service` serves. The storage-core routers mount a
    /// group's REST routes under `/{version}/{group}`, but discovery advertises only the version
    /// mount base `/{version}`: the resource name is carried by `RestEndpoint.name` and the group's
    /// OpenAPI paths, so repeating it in the advertised path would duplicate the resource segment.
    ///
    /// Mirrors the `add_service(...)` registrations in `main.rs`: storage-core does not expose the
    /// registered REST base paths, so the served group/version set is hand-maintained in both.
    fn storage_service_entry(metadata_enabled: bool) -> ServiceEntry {
        use storage_core_protos::nvidia::omniverse::storage::{capabilities, filefolder, fileobject, versioning};

        // (group name, [(version, fully-qualified gRPC service name, served over REST)]). Every
        // storage group is served on gRPC at both versions, but its REST surface is a single
        // `/v1alpha` space -- v1beta REST clients call the v1alpha routes and convert -- so only
        // v1alpha is advertised under `rest_services`. The metadata group is the exception: its
        // top-level `nvidia.omniverse.metadata.v1beta` API has its own `/v1beta/metadata` REST
        // routes, so both of its versions are advertised over REST.
        type ApiGroup = (&'static str, Vec<(&'static str, &'static str, bool)>);
        let mut groups: Vec<ApiGroup> = vec![
            (
                "fileobject",
                vec![
                    (
                        "v1alpha",
                        fileobject::v1alpha::file_object_service_server::SERVICE_NAME,
                        true,
                    ),
                    (
                        "v1beta",
                        fileobject::v1beta::file_object_service_server::SERVICE_NAME,
                        false,
                    ),
                ],
            ),
            (
                "filefolder",
                vec![
                    (
                        "v1alpha",
                        filefolder::v1alpha::file_folder_service_server::SERVICE_NAME,
                        true,
                    ),
                    (
                        "v1beta",
                        filefolder::v1beta::file_folder_service_server::SERVICE_NAME,
                        false,
                    ),
                ],
            ),
            (
                "capabilities",
                vec![
                    (
                        "v1alpha",
                        capabilities::v1alpha::capabilities_service_server::SERVICE_NAME,
                        true,
                    ),
                    (
                        "v1beta",
                        capabilities::v1beta::capabilities_service_server::SERVICE_NAME,
                        false,
                    ),
                ],
            ),
            (
                "versioning",
                vec![
                    (
                        "v1alpha",
                        versioning::v1alpha::versioning_service_server::SERVICE_NAME,
                        true,
                    ),
                    (
                        "v1beta",
                        versioning::v1beta::versioning_service_server::SERVICE_NAME,
                        false,
                    ),
                ],
            ),
        ];

        if metadata_enabled {
            use storage_service_protos::nvidia::omniverse::metadata as top_level_metadata;

            // The metadata group is served under two proto packages: the storage-namespace
            // `nvidia.omniverse.storage.metadata.v1alpha` and the top-level
            // `nvidia.omniverse.metadata.v1beta` (see `crate::api::metadata_api`). Both are served
            // over gRPC and REST, at `/v1alpha/metadata` and `/v1beta/metadata` respectively.
            groups.push((
                "metadata",
                vec![
                    (
                        "v1alpha",
                        metadata::v1alpha::metadata_service_server::SERVICE_NAME,
                        true,
                    ),
                    (
                        "v1beta",
                        top_level_metadata::v1beta::metadata_service_server::SERVICE_NAME,
                        true,
                    ),
                ],
            ));
        }

        let grpc_endpoints: Vec<GrpcEndpoint> = groups
            .iter()
            .flat_map(|(_, versions)| versions.iter())
            .map(|(_, service, _)| GrpcEndpoint {
                service_name: (*service).into(),
            })
            .collect();

        ServiceEntry {
            // A structural hint only; discovery does not key on the service name.
            service_name: "storage".into(),
            grpc_services: vec![GrpcService {
                endpoints: grpc_endpoints,
            }],
            rest_services: vec![RestService {
                endpoints: groups
                    .iter()
                    .map(|(name, versions)| RestEndpoint {
                        name: (*name).into(),
                        versions: versions
                            .iter()
                            .filter(|(_, _, served_over_rest)| *served_over_rest)
                            .map(|(version, _, _)| RestVersion {
                                version: (*version).into(),
                                // Advertise the version mount base only; `name` above and the
                                // group's OpenAPI paths already carry the resource segment.
                                url_detail: Some(RestVersionDetail {
                                    path: format!("/{version}"),
                                }),
                            })
                            .collect(),
                    })
                    .collect(),
            }],
        }
    }
}

#[cfg(test)]
mod tests {
    use assert_matches::assert_matches;
    use storage_service_protos::nvidia::omniverse::discovery::capabilities::v2alpha::ServiceEntry;

    use super::v2alpha::{service_entries, Capabilities, InvalidPageToken};

    fn storage_entry(metadata_enabled: bool) -> ServiceEntry {
        let entries = service_entries(metadata_enabled);
        assert_eq!(entries.len(), 1);
        entries.into_iter().next().unwrap()
    }

    fn rest_endpoint_names(entry: &ServiceEntry) -> Vec<&str> {
        entry
            .rest_services
            .iter()
            .flat_map(|transport| transport.endpoints.iter())
            .map(|endpoint| endpoint.name.as_str())
            .collect()
    }

    #[test]
    fn test_registers_a_single_storage_service() {
        let entry = storage_entry(true);

        assert_eq!(entry.service_name, "storage");
    }

    #[test]
    fn test_storage_service_advertises_every_api_group_as_an_endpoint() {
        assert_eq!(
            rest_endpoint_names(&storage_entry(true)),
            ["fileobject", "filefolder", "capabilities", "versioning", "metadata"]
        );
    }

    #[test]
    fn test_metadata_endpoint_omitted_when_disabled() {
        assert_eq!(
            rest_endpoint_names(&storage_entry(false)),
            ["fileobject", "filefolder", "capabilities", "versioning"]
        );
    }

    #[test]
    fn test_grpc_names_are_fully_qualified() {
        let entry = storage_entry(true);

        let grpc_names: Vec<_> = entry
            .grpc_services
            .iter()
            .flat_map(|transport| transport.endpoints.iter())
            .map(|endpoint| endpoint.service_name.as_str())
            .collect();

        assert_eq!(
            grpc_names,
            [
                "nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService",
                "nvidia.omniverse.storage.fileobject.v1beta.FileObjectService",
                "nvidia.omniverse.storage.filefolder.v1alpha.FileFolderService",
                "nvidia.omniverse.storage.filefolder.v1beta.FileFolderService",
                "nvidia.omniverse.storage.capabilities.v1alpha.CapabilitiesService",
                "nvidia.omniverse.storage.capabilities.v1beta.CapabilitiesService",
                "nvidia.omniverse.storage.versioning.v1alpha.VersioningService",
                "nvidia.omniverse.storage.versioning.v1beta.VersioningService",
                "nvidia.omniverse.storage.metadata.v1alpha.MetadataService",
                "nvidia.omniverse.metadata.v1beta.MetadataService",
            ]
        );
    }

    #[test]
    fn test_rest_endpoint_base_paths_are_versioned() {
        let entry = storage_entry(true);

        let rest_paths: Vec<_> = entry
            .rest_services
            .iter()
            .flat_map(|transport| transport.endpoints.iter())
            .flat_map(|endpoint| endpoint.versions.iter())
            .map(|version| {
                version
                    .url_detail
                    .as_ref()
                    .expect("missing version detail")
                    .path
                    .as_str()
            })
            .collect();

        // Every group advertises the version mount base only; the resource name lives in
        // `RestEndpoint.name`. The storage groups expose a single `/v1alpha` REST space, while the
        // metadata group also advertises its top-level `/v1beta` surface.
        assert_eq!(
            rest_paths,
            ["/v1alpha", "/v1alpha", "/v1alpha", "/v1alpha", "/v1alpha", "/v1beta"]
        );
    }

    // The storage groups advertise v1beta over gRPC only, while the metadata group advertises its
    // top-level v1beta over both gRPC and REST.
    #[test]
    fn test_metadata_v1beta_is_advertised_over_rest() {
        let entry = storage_entry(true);

        let metadata_rest_versions: Vec<_> = entry
            .rest_services
            .iter()
            .flat_map(|transport| transport.endpoints.iter())
            .filter(|endpoint| endpoint.name == "metadata")
            .flat_map(|endpoint| endpoint.versions.iter())
            .map(|version| version.version.as_str())
            .collect();
        assert_eq!(metadata_rest_versions, ["v1alpha", "v1beta"]);

        // The storage groups keep REST at v1alpha only.
        let storage_rest_versions: Vec<_> = entry
            .rest_services
            .iter()
            .flat_map(|transport| transport.endpoints.iter())
            .filter(|endpoint| endpoint.name != "metadata")
            .flat_map(|endpoint| endpoint.versions.iter())
            .map(|version| version.version.as_str())
            .collect();
        assert!(
            storage_rest_versions.iter().all(|version| *version == "v1alpha"),
            "storage groups must advertise only v1alpha over REST, got {storage_rest_versions:?}"
        );
    }

    fn named_entries(names: &[&str]) -> Vec<ServiceEntry> {
        names
            .iter()
            .map(|name| ServiceEntry {
                service_name: (*name).into(),
                grpc_services: Vec::new(),
                rest_services: Vec::new(),
            })
            .collect()
    }

    #[test]
    fn test_list_services_page_walks_all_entries() {
        let all = named_entries(&["a", "b", "c", "d", "e"]);
        let capabilities = Capabilities::new(all.clone());

        let first = capabilities.list_services_page(3, "").expect("first page");
        let second = capabilities
            .list_services_page(3, &first.next_page_token)
            .expect("second page");

        assert_eq!(first.services, all[..3]);
        assert_eq!(first.next_page_token, "3");
        assert_eq!(second.services, all[3..]);
        assert_eq!(second.next_page_token, "");
    }

    #[test]
    fn test_list_services_page_returns_everything_without_page_size() {
        let capabilities = Capabilities::new(service_entries(true));

        let response = capabilities.list_services_page(0, "").expect("single page");

        assert_eq!(response.services, service_entries(true));
        assert_eq!(response.next_page_token, "");
    }

    #[test]
    fn test_list_services_page_rejects_invalid_page_token() {
        let capabilities = Capabilities::new(named_entries(&["a", "b", "c"]));

        assert_matches!(capabilities.list_services_page(0, "bogus"), Err(InvalidPageToken));
        assert_matches!(capabilities.list_services_page(0, "100"), Err(InvalidPageToken));
        // Tokens the server never issues: the end boundary (`== len`), `"0"` (page 1 uses the empty
        // token), and a non-canonical spelling of an in-range offset.
        assert_matches!(capabilities.list_services_page(0, "3"), Err(InvalidPageToken));
        assert_matches!(capabilities.list_services_page(0, "0"), Err(InvalidPageToken));
        assert_matches!(capabilities.list_services_page(0, "01"), Err(InvalidPageToken));
    }

    // Production advertises one entry and never issues a token, so `"1"` (== the count) is unknown.
    #[test]
    fn test_list_services_page_rejects_token_equal_to_service_count() {
        let capabilities = Capabilities::new(service_entries(true));

        assert_matches!(capabilities.list_services_page(0, "1"), Err(InvalidPageToken));
    }
}
