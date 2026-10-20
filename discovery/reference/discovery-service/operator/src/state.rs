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

use crate::proto::ServiceEntry;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrpcEndpointEntry {
    pub service: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestVersionEntry {
    pub version: String,
    pub path: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestEndpointEntry {
    pub name: String,
    pub versions: Vec<RestVersionEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapabilityEntry {
    pub service_name: String,
    pub grpc: Vec<GrpcEndpointEntry>,
    pub rest: Vec<RestEndpointEntry>,
}

impl CapabilityEntry {
    pub fn from_proto(entry: ServiceEntry) -> Self {
        let service_name = entry.service_name.clone();
        let mut grpc = Vec::new();
        for block in entry.grpc_services {
            grpc.extend(block.endpoints.into_iter().filter_map(|endpoint| {
                let service = endpoint.service_name;
                if service.is_empty() {
                    None
                } else {
                    Some(GrpcEndpointEntry { service })
                }
            }));
        }

        let mut rest = Vec::new();
        for block in entry.rest_services {
            rest.extend(block.endpoints.into_iter().filter_map(|endpoint| {
                let versions = endpoint
                    .versions
                    .into_iter()
                    .filter_map(|version| {
                        let version_id = version.version;
                        if version_id.is_empty() {
                            return None;
                        }
                        Some(RestVersionEntry {
                            version: version_id,
                            path: version.url_detail.and_then(|detail| {
                                let path = detail.path;
                                if path.is_empty() {
                                    None
                                } else {
                                    Some(path)
                                }
                            }),
                        })
                    })
                    .collect::<Vec<_>>();
                if endpoint.name.is_empty() && versions.is_empty() {
                    None
                } else {
                    Some(RestEndpointEntry {
                        name: if endpoint.name.is_empty() {
                            service_name.clone()
                        } else {
                            endpoint.name
                        },
                        versions,
                    })
                }
            }));
        }

        Self {
            service_name: entry.service_name,
            grpc,
            rest,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapabilitySource {
    Grpc,
    #[allow(dead_code)]
    OpenApi,
}

impl Serialize for CapabilitySource {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(match self {
            CapabilitySource::Grpc => "grpc",
            CapabilitySource::OpenApi => "openapi",
        })
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct VersionDetail {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct RestEndpoint {
    pub name: String,
    pub versions: BTreeMap<String, VersionDetail>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct RestBlock {
    pub url: String,
    pub endpoints: Vec<RestEndpoint>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct GrpcServiceEndpoint {
    pub service: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct GrpcBlock {
    pub url: String,
    pub endpoints: Vec<GrpcServiceEndpoint>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ServiceInfo {
    pub name: String,
    pub capability_source: CapabilitySource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rest: Option<RestBlock>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grpc: Option<GrpcBlock>,
}

impl ServiceInfo {
    pub fn from_capabilities(
        name: String,
        rest_url: Option<String>,
        grpc_url: Option<String>,
        entries: Vec<CapabilityEntry>,
    ) -> Self {
        Self {
            name,
            capability_source: CapabilitySource::Grpc,
            rest: rest_url.and_then(|url| build_rest_block(url, &entries)),
            grpc: grpc_url.and_then(|url| build_grpc_block(url, &entries)),
        }
    }
}

fn build_rest_block(url: String, entries: &[CapabilityEntry]) -> Option<RestBlock> {
    let mut endpoints_by_name: BTreeMap<String, BTreeMap<String, VersionDetail>> = BTreeMap::new();

    for entry in entries {
        for rest_endpoint in &entry.rest {
            let endpoint_name = if rest_endpoint.name.is_empty() {
                entry.service_name.clone()
            } else {
                rest_endpoint.name.clone()
            };
            let versions = endpoints_by_name.entry(endpoint_name).or_default();
            for version in &rest_endpoint.versions {
                if version.version.is_empty() {
                    continue;
                }
                versions.insert(
                    version.version.clone(),
                    VersionDetail {
                        path: version.path.clone(),
                    },
                );
            }
        }
    }

    let endpoints: Vec<RestEndpoint> = endpoints_by_name
        .into_iter()
        .filter(|(_, versions)| !versions.is_empty())
        .map(|(name, versions)| RestEndpoint { name, versions })
        .collect();

    if endpoints.is_empty() {
        None
    } else {
        Some(RestBlock { url, endpoints })
    }
}

fn build_grpc_block(url: String, entries: &[CapabilityEntry]) -> Option<GrpcBlock> {
    let mut endpoints: Vec<GrpcServiceEndpoint> = entries
        .iter()
        .flat_map(|entry| {
            entry.grpc.iter().filter_map(|grpc_endpoint| {
                if grpc_endpoint.service.is_empty() {
                    None
                } else {
                    Some(GrpcServiceEndpoint {
                        service: grpc_endpoint.service.clone(),
                    })
                }
            })
        })
        .collect();
    endpoints.sort_by(|a, b| a.service.cmp(&b.service));
    endpoints.dedup_by(|a, b| a.service == b.service);

    if endpoints.is_empty() {
        None
    } else {
        Some(GrpcBlock { url, endpoints })
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ServicesResponse {
    #[serde(rename = "schema-version")]
    pub schema_version: u32,
    pub services: Vec<ServiceInfo>,
}

pub type SharedState = Arc<RwLock<HashMap<String, ServiceInfo>>>;

pub fn new_shared_state() -> SharedState {
    Arc::new(RwLock::new(HashMap::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::{
        GrpcEndpoint as ProtoGrpcEndpoint, GrpcService as ProtoGrpcService,
        RestEndpoint as ProtoRestEndpoint, RestService as ProtoRestService,
    };

    fn test_capability_entries() -> Vec<CapabilityEntry> {
        vec![
            CapabilityEntry {
                service_name: "fileobject".to_string(),
                rest: vec![RestEndpointEntry {
                    name: "fileobject".to_string(),
                    versions: vec![
                        RestVersionEntry {
                            version: "v1".to_string(),
                            path: Some("/api/v1/fileobject".to_string()),
                        },
                        RestVersionEntry {
                            version: "v1alpha".to_string(),
                            path: Some("/api/v1alpha/fileobject".to_string()),
                        },
                    ],
                }],
                grpc: vec![],
            },
            CapabilityEntry {
                service_name: "filefolder".to_string(),
                rest: vec![],
                grpc: vec![
                    GrpcEndpointEntry {
                        service: "nvidia.omniverse.storage.filefolder.v1.FileFolderService"
                            .to_string(),
                    },
                    GrpcEndpointEntry {
                        service: "nvidia.omniverse.storage.filefolder.v1alpha.FileFolderService"
                            .to_string(),
                    },
                ],
            },
            CapabilityEntry {
                service_name: "fileobject".to_string(),
                rest: vec![],
                grpc: vec![
                    GrpcEndpointEntry {
                        service: "nvidia.omniverse.storage.fileobject.v1.FileObjectService"
                            .to_string(),
                    },
                    GrpcEndpointEntry {
                        service: "nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService"
                            .to_string(),
                    },
                ],
            },
        ]
    }

    #[test]
    fn capability_entry_from_proto_maps_rest_and_grpc() {
        let entry = CapabilityEntry::from_proto(ServiceEntry {
            service_name: "fileobject".to_string(),
            grpc_services: vec![ProtoGrpcService {
                endpoints: vec![ProtoGrpcEndpoint {
                    service_name: "nvidia.omniverse.storage.fileobject.v1.FileObjectService"
                        .to_string(),
                }],
            }],
            rest_services: vec![ProtoRestService {
                endpoints: vec![ProtoRestEndpoint {
                    name: "fileobject".to_string(),
                    versions: vec![crate::proto::RestVersion {
                        version: "v1".to_string(),
                        url_detail: Some(crate::proto::RestVersionDetail {
                            path: "/api/v1/fileobject".to_string(),
                        }),
                    }],
                }],
            }],
        });

        assert_eq!(entry.service_name, "fileobject");
        assert_eq!(entry.grpc.len(), 1);
        assert_eq!(entry.rest.len(), 1);
        assert_eq!(
            entry.rest[0].versions[0].path.as_deref(),
            Some("/api/v1/fileobject")
        );
    }

    #[test]
    fn service_info_matches_test_yml_shape() {
        let info = ServiceInfo::from_capabilities(
            "storage-service-revised".to_string(),
            Some("https://storage.example.com".to_string()),
            Some("https://grpc.storage.example.com".to_string()),
            test_capability_entries(),
        );

        assert_eq!(info.name, "storage-service-revised");
        assert_eq!(info.capability_source, CapabilitySource::Grpc);

        let rest = info.rest.expect("rest block");
        assert_eq!(rest.url, "https://storage.example.com");
        assert_eq!(rest.endpoints.len(), 1);
        assert_eq!(rest.endpoints[0].name, "fileobject");
        assert_eq!(
            rest.endpoints[0]
                .versions
                .get("v1")
                .and_then(|v| v.path.as_deref()),
            Some("/api/v1/fileobject")
        );
        assert_eq!(
            rest.endpoints[0]
                .versions
                .get("v1alpha")
                .and_then(|v| v.path.as_deref()),
            Some("/api/v1alpha/fileobject")
        );

        let grpc = info.grpc.expect("grpc block");
        assert_eq!(grpc.url, "https://grpc.storage.example.com");
        assert_eq!(grpc.endpoints.len(), 4);
        assert_eq!(
            grpc.endpoints[0].service,
            "nvidia.omniverse.storage.filefolder.v1.FileFolderService"
        );
        assert_eq!(
            grpc.endpoints[1].service,
            "nvidia.omniverse.storage.filefolder.v1alpha.FileFolderService"
        );
        assert_eq!(
            grpc.endpoints[2].service,
            "nvidia.omniverse.storage.fileobject.v1.FileObjectService"
        );
        assert_eq!(
            grpc.endpoints[3].service,
            "nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService"
        );
    }

    #[test]
    fn services_response_serializes_flat_grpc_endpoints() {
        let response = ServicesResponse {
            schema_version: 2,
            services: vec![ServiceInfo::from_capabilities(
                "storage-service".to_string(),
                None,
                Some("https://grpc.storage.example.com".to_string()),
                test_capability_entries(),
            )],
        };

        let json = serde_json::to_value(response).expect("serialize response");
        assert_eq!(
            json["services"][0]["grpc"]["endpoints"][0]["service"],
            "nvidia.omniverse.storage.filefolder.v1.FileFolderService"
        );
        assert!(json["services"][0]["grpc"]["endpoints"][0]["name"].is_null());
        assert!(json["services"][0]["grpc"]["endpoints"][0]["versions"].is_null());
    }

    #[test]
    fn grpc_block_deduplicates_services() {
        let service = "nvidia.omniverse.storage.fileobject.v1.FileObjectService";
        let entry = |name: &str| CapabilityEntry {
            service_name: name.to_string(),
            rest: vec![],
            grpc: vec![GrpcEndpointEntry {
                service: service.to_string(),
            }],
        };
        let info = ServiceInfo::from_capabilities(
            "storage-service".to_string(),
            None,
            Some("https://grpc.storage.example.com".to_string()),
            vec![entry("fileobject"), entry("fileobject-legacy")],
        );

        let grpc = info.grpc.expect("grpc block");
        assert_eq!(grpc.endpoints.len(), 1);
        assert_eq!(grpc.endpoints[0].service, service);
    }

    #[test]
    fn rest_block_omits_endpoints_without_versions() {
        let entries = vec![CapabilityEntry {
            service_name: "fileobject".to_string(),
            rest: vec![
                RestEndpointEntry {
                    name: "fileobject".to_string(),
                    versions: vec![RestVersionEntry {
                        version: "v1".to_string(),
                        path: Some("/api/v1/fileobject".to_string()),
                    }],
                },
                RestEndpointEntry {
                    name: "unversioned".to_string(),
                    versions: vec![],
                },
            ],
            grpc: vec![],
        }];
        let info = ServiceInfo::from_capabilities(
            "storage-service".to_string(),
            Some("https://storage.example.com".to_string()),
            None,
            entries,
        );

        let rest = info.rest.expect("rest block");
        assert_eq!(rest.endpoints.len(), 1);
        assert_eq!(rest.endpoints[0].name, "fileobject");
    }

    #[test]
    fn rest_block_omitted_when_no_endpoint_has_versions() {
        let entries = vec![CapabilityEntry {
            service_name: "fileobject".to_string(),
            rest: vec![RestEndpointEntry {
                name: "fileobject".to_string(),
                versions: vec![],
            }],
            grpc: vec![],
        }];
        let info = ServiceInfo::from_capabilities(
            "storage-service".to_string(),
            Some("https://storage.example.com".to_string()),
            None,
            entries,
        );

        assert!(info.rest.is_none());
    }

    #[test]
    fn services_response_serializes_schema_version_two() {
        let response = ServicesResponse {
            schema_version: 2,
            services: vec![ServiceInfo::from_capabilities(
                "storage-service-revised".to_string(),
                Some("https://storage.example.com".to_string()),
                None,
                vec![CapabilityEntry {
                    service_name: "fileobject".to_string(),
                    rest: vec![RestEndpointEntry {
                        name: "fileobject".to_string(),
                        versions: vec![RestVersionEntry {
                            version: "v1".to_string(),
                            path: Some("/api/v1/fileobject".to_string()),
                        }],
                    }],
                    grpc: vec![],
                }],
            )],
        };

        let json = serde_json::to_value(response).expect("serialize response");
        assert_eq!(json["schema-version"], 2);
        assert_eq!(json["services"][0]["name"], "storage-service-revised");
        assert_eq!(json["services"][0]["capability_source"], "grpc");
        assert_eq!(
            json["services"][0]["rest"]["endpoints"][0]["versions"]["v1"]["path"],
            "/api/v1/fileobject"
        );
        assert!(json["services"][0]["grpc"].is_null());
    }
}
