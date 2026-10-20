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

//! Which Services take part in discovery when several namespaces are watched.
//!
//! The registries are keyed by Service name only, so two annotated Services
//! with the same name in different namespaces cannot both be published. The
//! first-listed namespace wins; when no namespaces are listed (every namespace
//! is watched) the namespace name decides, so the winner never depends on the
//! order the API server happens to return items in.

use k8s_openapi::api::core::v1::Service;
use kube::ResourceExt;
use std::collections::HashMap;

/// A Service that was left out because another namespace already supplied the
/// same name.
#[derive(Debug, PartialEq, Eq)]
pub struct Collision {
    pub name: String,
    pub winner_namespace: String,
    pub ignored_namespace: String,
}

/// Orders `services` by (namespace position, namespace, name), keeps those
/// annotated `enabled_key: "true"`, and drops later Services whose name is
/// already taken. `namespaces` is the configured list; empty means all.
pub fn select_services(
    mut services: Vec<Service>,
    namespaces: &[String],
    enabled_key: &str,
) -> (Vec<Service>, Vec<Collision>) {
    let rank = |namespace: &str| {
        namespaces
            .iter()
            .position(|n| n == namespace)
            .unwrap_or(usize::MAX)
    };
    services.sort_by_cached_key(|svc| {
        let namespace = svc.namespace().unwrap_or_default();
        (rank(&namespace), namespace, svc.name_any())
    });

    let mut selected = Vec::new();
    let mut collisions = Vec::new();
    let mut taken: HashMap<String, String> = HashMap::new();
    for svc in services {
        let enabled = svc
            .metadata
            .annotations
            .as_ref()
            .and_then(|a| a.get(enabled_key))
            .is_some_and(|v| v == "true");
        if !enabled {
            continue;
        }
        let name = svc.name_any();
        let namespace = svc.namespace().unwrap_or_default();
        match taken.get(&name) {
            Some(winner) => collisions.push(Collision {
                name,
                winner_namespace: winner.clone(),
                ignored_namespace: namespace,
            }),
            None => {
                taken.insert(name, namespace);
                selected.push(svc);
            }
        }
    }
    (selected, collisions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
    use std::collections::BTreeMap;

    const KEY: &str = "discovery.omniverse.nvidia.com/enabled";

    fn svc(namespace: &str, name: &str, enabled: Option<&str>) -> Service {
        Service {
            metadata: ObjectMeta {
                name: Some(name.to_string()),
                namespace: Some(namespace.to_string()),
                annotations: enabled.map(|v| BTreeMap::from([(KEY.to_string(), v.to_string())])),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn ns(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn ids(services: &[Service]) -> Vec<String> {
        services
            .iter()
            .map(|s| format!("{}/{}", s.namespace().unwrap(), s.name_any()))
            .collect()
    }

    #[test]
    fn only_services_annotated_true_are_selected() {
        let (selected, collisions) = select_services(
            vec![
                svc("a", "on", Some("true")),
                svc("a", "off", Some("false")),
                svc("a", "none", None),
            ],
            &ns(&["a"]),
            KEY,
        );
        assert_eq!(ids(&selected), ["a/on"]);
        assert!(collisions.is_empty());
    }

    #[test]
    fn listed_namespaces_are_processed_in_configured_order() {
        let services = vec![
            svc("a", "x", Some("true")),
            svc("b", "y", Some("true")),
            svc("b", "w", Some("true")),
            svc("a", "z", Some("true")),
        ];
        let (selected, _) = select_services(services, &ns(&["b", "a"]), KEY);
        assert_eq!(ids(&selected), ["b/w", "b/y", "a/x", "a/z"]);
    }

    #[test]
    fn first_listed_namespace_wins_a_name_collision() {
        let services = vec![
            svc("a", "usdsearch", Some("true")),
            svc("b", "usdsearch", Some("true")),
        ];

        let (selected, collisions) = select_services(services.clone(), &ns(&["a", "b"]), KEY);
        assert_eq!(ids(&selected), ["a/usdsearch"]);
        assert_eq!(
            collisions,
            [Collision {
                name: "usdsearch".into(),
                winner_namespace: "a".into(),
                ignored_namespace: "b".into(),
            }]
        );

        let (selected, collisions) = select_services(services, &ns(&["b", "a"]), KEY);
        assert_eq!(ids(&selected), ["b/usdsearch"]);
        assert_eq!(collisions[0].ignored_namespace, "a");
    }

    #[test]
    fn input_order_does_not_change_the_winner() {
        let forward = vec![svc("a", "s", Some("true")), svc("b", "s", Some("true"))];
        let reversed = vec![svc("b", "s", Some("true")), svc("a", "s", Some("true"))];
        for input in [forward, reversed] {
            let (selected, _) = select_services(input, &ns(&["a", "b"]), KEY);
            assert_eq!(ids(&selected), ["a/s"]);
        }
    }

    #[test]
    fn a_disabled_service_does_not_collide_with_an_enabled_one() {
        let (selected, collisions) = select_services(
            vec![svc("a", "s", Some("false")), svc("b", "s", Some("true"))],
            &ns(&["a", "b"]),
            KEY,
        );
        assert_eq!(ids(&selected), ["b/s"]);
        assert!(collisions.is_empty());
    }

    #[test]
    fn all_namespaces_is_ordered_by_namespace_then_name() {
        let services = vec![
            svc("zeta", "s", Some("true")),
            svc("alpha", "t", Some("true")),
            svc("alpha", "s", Some("true")),
            svc("mid", "s", Some("true")),
        ];
        let (selected, collisions) = select_services(services.clone(), &[], KEY);
        assert_eq!(ids(&selected), ["alpha/s", "alpha/t"]);
        assert_eq!(collisions.len(), 2);
        assert!(collisions.iter().all(|c| c.winner_namespace == "alpha"));

        let mut shuffled = services;
        shuffled.reverse();
        let (again, _) = select_services(shuffled, &[], KEY);
        assert_eq!(ids(&again), ids(&selected));
    }

    #[test]
    fn unlisted_namespaces_sort_after_listed_ones() {
        let (selected, _) = select_services(
            vec![svc("c", "s", Some("true")), svc("b", "s", Some("true"))],
            &ns(&["b"]),
            KEY,
        );
        assert_eq!(ids(&selected), ["b/s"]);
    }
}
