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

use crate::state::ServiceInfo;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub fn parse_mappings(json: &str) -> Result<Vec<V1EndpointMapping>, serde_json::Error> {
    serde_json::from_str(json)
}

pub fn build_v1_response(
    mappings: &[V1EndpointMapping],
    services: &HashMap<String, ServiceInfo>,
) -> V1ServicesResponse {
    let mut sorted: Vec<&ServiceInfo> = services.values().collect();
    sorted.sort_by(|a, b| a.name.cmp(&b.name));

    let mut result = Vec::new();
    for mapping in mappings {
        let matches: Vec<V1Service> = sorted
            .iter()
            .filter_map(|info| match_service(mapping, info))
            .collect();

        let single = matches.len() == 1;
        for (index, mut service) in matches.into_iter().enumerate() {
            service.id = if single {
                mapping.id_prefix.clone()
            } else {
                format!("{}-{}", mapping.id_prefix, index)
            };
            result.push(service);
        }
    }

    V1ServicesResponse {
        schema_version: 1,
        services: result,
    }
}

fn match_service(mapping: &V1EndpointMapping, info: &ServiceInfo) -> Option<V1Service> {
    let grpc = info.grpc.as_ref().filter(|block| {
        block
            .endpoints
            .iter()
            .any(|endpoint| mapping.grpc_from().contains(&endpoint.service))
    });

    let rest = info.rest.as_ref().filter(|block| {
        block
            .endpoints
            .iter()
            .any(|endpoint| mapping.rest_from().contains(&endpoint.name))
    });

    if grpc.is_none() && rest.is_none() {
        return None;
    }

    Some(V1Service {
        grpc: grpc.map(|block| block.url.clone()),
        id: String::new(),
        name: mapping.name.clone(),
        rest: rest.map(|block| block.url.clone()),
        service_type: mapping.service_type.clone(),
    })
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct V1ServicesResponse {
    #[serde(rename = "schema-version")]
    pub schema_version: u32,
    pub services: Vec<V1Service>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct V1Service {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grpc: Option<String>,
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rest: Option<String>,
    #[serde(rename = "type")]
    pub service_type: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct V1EndpointMapping {
    pub name: String,
    #[serde(rename = "idPrefix")]
    pub id_prefix: String,
    #[serde(rename = "type")]
    pub service_type: String,
    #[serde(default)]
    pub endpoints: V1EndpointSources,
}

impl V1EndpointMapping {
    fn grpc_from(&self) -> &[String] {
        self.endpoints
            .grpc
            .as_ref()
            .map(|list| list.from.as_slice())
            .unwrap_or_default()
    }

    fn rest_from(&self) -> &[String] {
        self.endpoints
            .rest
            .as_ref()
            .map(|list| list.from.as_slice())
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct V1EndpointSources {
    #[serde(default)]
    pub grpc: Option<V1FromList>,
    #[serde(default)]
    pub rest: Option<V1FromList>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct V1FromList {
    #[serde(default)]
    pub from: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{
        CapabilitySource, GrpcBlock, GrpcServiceEndpoint, RestBlock, RestEndpoint, VersionDetail,
    };
    use std::collections::BTreeMap;

    fn grpc_block(url: &str, services: &[&str]) -> GrpcBlock {
        GrpcBlock {
            url: url.to_string(),
            endpoints: services
                .iter()
                .map(|service| GrpcServiceEndpoint {
                    service: service.to_string(),
                })
                .collect(),
        }
    }

    fn rest_block(url: &str, names: &[&str]) -> RestBlock {
        RestBlock {
            url: url.to_string(),
            endpoints: names
                .iter()
                .map(|name| RestEndpoint {
                    name: name.to_string(),
                    versions: BTreeMap::from([("v1".to_string(), VersionDetail { path: None })]),
                })
                .collect(),
        }
    }

    fn service(name: &str, rest: Option<RestBlock>, grpc: Option<GrpcBlock>) -> ServiceInfo {
        ServiceInfo {
            name: name.to_string(),
            capability_source: CapabilitySource::Grpc,
            rest,
            grpc,
        }
    }

    fn storage_mapping() -> V1EndpointMapping {
        V1EndpointMapping {
            name: "Storage Service".to_string(),
            id_prefix: "storage-service".to_string(),
            service_type: "storage".to_string(),
            endpoints: V1EndpointSources {
                grpc: Some(V1FromList {
                    from: vec![
                        "nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService".to_string(),
                    ],
                }),
                rest: Some(V1FromList {
                    from: vec!["filefolder".to_string()],
                }),
            },
        }
    }

    #[test]
    fn single_match_uses_id_prefix_and_both_urls() {
        let services = HashMap::from([(
            "storage-service".to_string(),
            service(
                "storage-service",
                Some(rest_block("http://rest.local", &["filefolder"])),
                Some(grpc_block(
                    "http://grpc.local",
                    &["nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService"],
                )),
            ),
        )]);

        let response = build_v1_response(&[storage_mapping()], &services);

        assert_eq!(response.schema_version, 1);
        assert_eq!(response.services.len(), 1);
        let entry = &response.services[0];
        assert_eq!(entry.id, "storage-service");
        assert_eq!(entry.name, "Storage Service");
        assert_eq!(entry.service_type, "storage");
        assert_eq!(entry.grpc.as_deref(), Some("http://grpc.local"));
        assert_eq!(entry.rest.as_deref(), Some("http://rest.local"));
    }

    #[test]
    fn multiple_matches_get_suffixed_ids() {
        let services = HashMap::from([
            (
                "storage-a".to_string(),
                service(
                    "storage-a",
                    None,
                    Some(grpc_block(
                        "http://grpc-a.local",
                        &["nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService"],
                    )),
                ),
            ),
            (
                "storage-b".to_string(),
                service(
                    "storage-b",
                    None,
                    Some(grpc_block(
                        "http://grpc-b.local",
                        &["nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService"],
                    )),
                ),
            ),
        ]);

        let response = build_v1_response(&[storage_mapping()], &services);

        assert_eq!(response.services.len(), 2);
        assert_eq!(response.services[0].id, "storage-service-0");
        assert_eq!(
            response.services[0].grpc.as_deref(),
            Some("http://grpc-a.local")
        );
        assert_eq!(response.services[1].id, "storage-service-1");
        assert_eq!(
            response.services[1].grpc.as_deref(),
            Some("http://grpc-b.local")
        );
    }

    #[test]
    fn grpc_only_match_omits_rest() {
        let services = HashMap::from([(
            "storage-service".to_string(),
            service(
                "storage-service",
                Some(rest_block("http://rest.local", &["unrelated"])),
                Some(grpc_block(
                    "http://grpc.local",
                    &["nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService"],
                )),
            ),
        )]);

        let response = build_v1_response(&[storage_mapping()], &services);

        assert_eq!(response.services.len(), 1);
        assert_eq!(
            response.services[0].grpc.as_deref(),
            Some("http://grpc.local")
        );
        assert_eq!(response.services[0].rest, None);
    }

    #[test]
    fn no_match_omits_entry() {
        let services = HashMap::from([(
            "other-service".to_string(),
            service(
                "other-service",
                Some(rest_block("http://rest.local", &["unrelated"])),
                Some(grpc_block("http://grpc.local", &["some.other.Service"])),
            ),
        )]);

        let response = build_v1_response(&[storage_mapping()], &services);

        assert!(response.services.is_empty());
    }

    #[test]
    fn parse_mappings_reads_values_yaml_shape() {
        let json = r#"[
          {
            "name": "Storage Service",
            "idPrefix": "storage-service",
            "type": "storage",
            "endpoints": {
              "grpc": { "from": ["nvidia.omniverse.storage.fileobject.v1alpha.FileObjectService"] },
              "rest": { "from": ["filefolder"] }
            }
          }
        ]"#;

        let mappings = parse_mappings(json).expect("parse");
        assert_eq!(mappings.len(), 1);
        assert_eq!(mappings[0].id_prefix, "storage-service");
        assert_eq!(mappings[0].service_type, "storage");
        assert_eq!(mappings[0].grpc_from().len(), 1);
        assert_eq!(mappings[0].rest_from(), &["filefolder".to_string()]);
    }

    #[test]
    fn parse_mappings_rejects_unknown_fields() {
        let configs = [
            r#"[{"name":"Storage","idPrefix":"storage","type":"storage","typo":true}]"#,
            r#"[{"name":"Storage","idPrefix":"storage","type":"storage","endpoints":{"typo":{}}}]"#,
            r#"[{"name":"Storage","idPrefix":"storage","type":"storage","endpoints":{"grpc":{"from":[],"typo":true}}}]"#,
        ];

        for config in configs {
            assert!(parse_mappings(config).is_err());
        }
    }
}
