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

use crate::client::capabilities;
use crate::config::Config;
use crate::k8s::httpproxy::HTTPProxy;
use crate::k8s::namespaces::select_services;
use crate::state::{CapabilityEntry, ServiceInfo, SharedState};
use futures_util::StreamExt;
use k8s_openapi::api::core::v1::{Pod, Service};
use kube::api::ListParams;
use kube::runtime::watcher;
use kube::{Api, Client, ResourceExt};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

pub async fn run_discovery(
    client: Client,
    config: Arc<Config>,
    internal_state: SharedState,
    external_state: SharedState,
) {
    let (notify_tx, mut notify_rx) = tokio::sync::mpsc::channel::<()>(16);

    let namespaces = config.watched_namespaces();
    if namespaces.is_empty() {
        tracing::info!("watching all namespaces");
    } else {
        tracing::info!(namespaces = %namespaces.join(","), "watching namespaces");
    }

    for svc_api in scoped_apis::<Service>(&client, &namespaces) {
        spawn_watcher(svc_api, notify_tx.clone(), "service", false);
    }

    let http_proxy_apis = scoped_apis::<HTTPProxy>(&client, &namespaces);

    // HTTPProxy support is optional. Probe it once before starting a watcher:
    // a missing CRD returns 404 immediately and otherwise causes the watcher
    // to retry in a tight loop, starving service reconciliation. The CRD is
    // cluster-wide, so one probe covers every watched namespace.
    let http_proxy_enabled = match http_proxy_apis[0]
        .list(&ListParams::default().limit(1))
        .await
    {
        Ok(_) => true,
        Err(e) => {
            tracing::info!(error = %e, "HTTPProxy discovery disabled");
            false
        }
    };

    if http_proxy_enabled {
        for http_proxy_api in http_proxy_apis {
            spawn_watcher(http_proxy_api, notify_tx.clone(), "HTTPProxy", true);
        }
    }

    let periodic_notify = notify_tx.clone();
    let interval_secs = config.reconcile_interval_secs;
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
        loop {
            interval.tick().await;
            let _ = periodic_notify.try_send(());
        }
    });

    let _ = notify_tx.try_send(());

    while notify_rx.recv().await.is_some() {
        while notify_rx.try_recv().is_ok() {}
        tokio::time::sleep(Duration::from_millis(500)).await;
        while notify_rx.try_recv().is_ok() {}

        if let Err(e) = reconcile(
            &client,
            &config,
            &internal_state,
            &external_state,
            http_proxy_enabled,
        )
        .await
        {
            tracing::error!(error = %e, "reconciliation failed");
        }
    }
}

/// One `Api` per watched namespace, or a single cluster-wide `Api` when no
/// namespaces are listed. Never empty.
fn scoped_apis<K>(client: &Client, namespaces: &[String]) -> Vec<Api<K>>
where
    K: kube::Resource<Scope = k8s_openapi::NamespaceResourceScope>,
    <K as kube::Resource>::DynamicType: Default,
{
    if namespaces.is_empty() {
        vec![Api::all(client.clone())]
    } else {
        namespaces
            .iter()
            .map(|ns| Api::namespaced(client.clone(), ns))
            .collect()
    }
}

/// Nudges the reconcile loop on every change the watcher sees. A Service
/// watcher keeps going after an error (the watcher retries on its own); an
/// HTTPProxy watcher stops, as it always has.
fn spawn_watcher<K>(
    api: Api<K>,
    notify: tokio::sync::mpsc::Sender<()>,
    label: &'static str,
    stop_on_error: bool,
) where
    K: kube::Resource + Clone + serde::de::DeserializeOwned + std::fmt::Debug + Send + 'static,
    <K as kube::Resource>::DynamicType: Default + Clone + std::hash::Hash + Eq,
{
    tokio::spawn(async move {
        let stream = watcher::watcher(api, watcher::Config::default());
        futures_util::pin_mut!(stream);
        while let Some(result) = stream.next().await {
            match result {
                Ok(_) => {
                    let _ = notify.try_send(());
                }
                Err(e) if stop_on_error => {
                    tracing::warn!(error = %e, "{label} watcher stopped");
                    break;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "{label} watcher error");
                }
            }
        }
    });
}

async fn reconcile(
    client: &Client,
    config: &Config,
    internal_state: &SharedState,
    external_state: &SharedState,
    http_proxy_enabled: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let namespaces = config.watched_namespaces();

    // Any failure listing Services aborts the cycle, so a transient error in one
    // namespace never drops that namespace's services from the catalog.
    let mut services = Vec::new();
    for svc_api in scoped_apis::<Service>(client, &namespaces) {
        services.extend(svc_api.list(&ListParams::default()).await?.items);
    }
    let (services, collisions) =
        select_services(services, &namespaces, &config.annotations.discovery_enabled);
    for collision in &collisions {
        tracing::warn!(
            service = %collision.name,
            used_namespace = %collision.winner_namespace,
            ignored_namespace = %collision.ignored_namespace,
            "annotated Service name exists in more than one watched namespace; \
             using the first-listed namespace and ignoring the other"
        );
    }

    let mut http_proxies = Vec::new();
    if http_proxy_enabled {
        for http_proxy_api in scoped_apis::<HTTPProxy>(client, &namespaces) {
            match http_proxy_api.list(&ListParams::default()).await {
                Ok(list) => http_proxies.extend(list.items),
                Err(e) => tracing::warn!(error = %e, "failed to list HTTPProxies"),
            }
        }
    }

    let proxy_map = build_proxy_map(&http_proxies, &config.annotations.base_path);
    let mut new_internal: HashMap<String, ServiceInfo> = HashMap::new();
    let mut new_external: HashMap<String, ServiceInfo> = HashMap::new();

    for svc in &services {
        let annotations = match svc.metadata.annotations.as_ref() {
            Some(a) => a,
            None => continue,
        };

        let enabled = annotations.get(&config.annotations.discovery_enabled);
        if !matches!(enabled, Some(value) if value == "true") {
            continue;
        }

        let visibility = annotations
            .get(&config.annotations.visibility)
            .map(|v| v.as_str())
            .unwrap_or("both");
        let expose_internal = matches!(visibility, "internal" | "both");
        let expose_external = matches!(visibility, "external" | "both");

        if !expose_internal && !expose_external {
            tracing::warn!(
                service = %svc.name_any(),
                visibility = %visibility,
                "unknown visibility value, expected 'internal', 'external', or 'both'; skipping"
            );
            continue;
        }

        let svc_name = svc.name_any();
        let namespace = svc.namespace().unwrap_or_default();
        let grpc_port = match determine_grpc_port(svc, &config.annotations.grpc_port) {
            Some(port) => port,
            None => {
                tracing::warn!("detected discovery annotation for {} but no gRPC port is defined in the service", svc_name);
                continue;
            }
        };
        let rest_port = determine_rest_port(svc);

        let capabilities = if config.use_kube_portforward {
            query_via_portforward(client, &namespace, svc, &svc_name, grpc_port).await
        } else {
            let internal_grpc_endpoint = format!(
                "http://{}.{}.svc.cluster.local:{}",
                svc_name, namespace, grpc_port
            );
            capabilities::query_capabilities(&internal_grpc_endpoint).await
        };

        let capabilities = match capabilities {
            Ok(capabilities) => capabilities,
            Err(e) => {
                tracing::warn!(
                    service = %svc_name,
                    error = %e,
                    "failed to query capabilities"
                );
                if expose_internal {
                    retain_previous(&svc_name, internal_state, &mut new_internal).await;
                }
                if expose_external {
                    retain_previous(&svc_name, external_state, &mut new_external).await;
                }
                continue;
            }
        };

        if expose_internal {
            let internal_rest = rest_port.map(|port| {
                format!(
                    "http://{}.{}.svc.cluster.local:{}",
                    svc_name, namespace, port
                )
            });
            let internal_grpc = Some(format!(
                "http://{}.{}.svc.cluster.local:{}",
                svc_name, namespace, grpc_port
            ));

            new_internal.insert(
                svc_name.clone(),
                ServiceInfo::from_capabilities(
                    svc_name.clone(),
                    internal_rest,
                    internal_grpc,
                    capabilities.clone(),
                ),
            );
        }

        if expose_external {
            let (proxy_rest, proxy_grpc) = proxy_map
                .get(&(namespace.clone(), svc_name.clone()))
                .map(|entries| resolve_proxy_urls(entries, rest_port, grpc_port))
                .unwrap_or((None, None));

            let ext_rest = annotations
                .get(&config.annotations.external_rest_endpoint)
                .cloned()
                .or(proxy_rest);
            let ext_grpc = annotations
                .get(&config.annotations.external_grpc_endpoint)
                .cloned()
                .or(proxy_grpc);

            if ext_rest.is_some() || ext_grpc.is_some() {
                let info = ServiceInfo::from_capabilities(
                    svc_name.clone(),
                    ext_rest,
                    ext_grpc,
                    capabilities,
                );
                if info.rest.is_some() || info.grpc.is_some() {
                    new_external.insert(svc_name.clone(), info);
                }
            }
        }
    }

    apply_state(internal_state, new_internal, "internal").await;
    apply_state(external_state, new_external, "external").await;
    Ok(())
}

async fn apply_state(state: &SharedState, new_state: HashMap<String, ServiceInfo>, label: &str) {
    let mut state_lock = state.write().await;
    if *state_lock != new_state {
        for (name, info) in &new_state {
            let event = if !state_lock.contains_key(name) {
                "discovered"
            } else if state_lock.get(name) != Some(info) {
                "updated"
            } else {
                continue;
            };
            tracing::info!("{event} {label} service:\n{}", format_service_yaml(info));
        }
        for name in state_lock.keys() {
            if !new_state.contains_key(name) {
                tracing::info!("removed {label} service:\n  name: {name}");
            }
        }
        tracing::info!(total = new_state.len(), "{label} reconciliation complete");

        *state_lock = new_state;
    } else {
        tracing::debug!(total = state_lock.len(), "{label} reconciled (unchanged)");
    }
}

async fn query_via_portforward(
    client: &Client,
    namespace: &str,
    svc: &Service,
    svc_name: &str,
    grpc_port: u16,
) -> Result<Vec<CapabilityEntry>, Box<dyn std::error::Error + Send + Sync>> {
    let pod_api: Api<Pod> = Api::namespaced(client.clone(), namespace);

    let selector = svc
        .spec
        .as_ref()
        .and_then(|s| s.selector.as_ref())
        .map(|labels| {
            labels
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(",")
        })
        .unwrap_or_default();

    let pods = pod_api
        .list(&ListParams::default().labels(&selector))
        .await?;

    let pod_name = pods
        .items
        .iter()
        .filter(|p| p.status.as_ref().and_then(|s| s.phase.as_deref()) == Some("Running"))
        .find_map(|p| p.metadata.name.as_deref())
        .ok_or_else(|| format!("no running pods found for service {svc_name}"))?;

    tracing::debug!(
        service = %svc_name,
        pod = %pod_name,
        port = grpc_port,
        "creating portforward"
    );

    let mut pf = pod_api.portforward(pod_name, &[grpc_port]).await?;
    let stream = pf
        .take_stream(grpc_port)
        .ok_or_else(|| format!("failed to get portforward stream for {svc_name}"))?;

    let pf_driver = tokio::spawn(async move {
        let _ = pf.join().await;
    });

    let result = capabilities::query_capabilities_via_stream(stream, svc_name).await;

    pf_driver.abort();

    result
}

struct ProxyEntry {
    url: String,
    port: i32,
    is_h2: bool,
    base_path: String,
}

/// Routes by (namespace, Service name). An HTTPProxy route can only reference a
/// Service in the proxy's own namespace, so the proxy's namespace is part of the
/// key: otherwise a proxy in one watched namespace would also be applied to a
/// same-named Service in another.
type ProxyMap = HashMap<(String, String), Vec<ProxyEntry>>;

fn build_proxy_map(proxies: &[HTTPProxy], base_path_annotation: &str) -> ProxyMap {
    let mut map: ProxyMap = HashMap::new();

    for proxy in proxies {
        let fqdn = match proxy.spec.virtualhost.as_ref().map(|v| &v.fqdn) {
            Some(f) => f.clone(),
            None => continue,
        };

        let tls = proxy
            .spec
            .virtualhost
            .as_ref()
            .and_then(|v| v.tls.as_ref())
            .is_some();

        let scheme = if tls { "https" } else { "http" };
        let proxy_namespace = proxy.namespace().unwrap_or_default();

        let base_path = proxy
            .metadata
            .annotations
            .as_ref()
            .and_then(|a| a.get(base_path_annotation))
            .cloned()
            .unwrap_or_default();

        for route in &proxy.spec.routes {
            for route_svc in &route.services {
                let url = format!("{}://{}", scheme, fqdn);
                map.entry((proxy_namespace.clone(), route_svc.name.clone()))
                    .or_default()
                    .push(ProxyEntry {
                        url,
                        port: route_svc.port,
                        is_h2: route_svc.protocol.as_deref() == Some("h2"),
                        base_path: base_path.clone(),
                    });
            }
        }
    }

    map
}

fn resolve_proxy_urls(
    entries: &[ProxyEntry],
    rest_port: Option<u16>,
    grpc_port: u16,
) -> (Option<String>, Option<String>) {
    let grpc_url = entries
        .iter()
        .find(|e| e.is_h2 || e.port as u16 == grpc_port)
        .map(|e| e.url.clone());

    let rest_url = rest_port.and_then(|port| {
        entries
            .iter()
            .find(|e| !e.is_h2 && e.port as u16 == port)
            .map(|e| format!("{}{}", e.url, e.base_path))
    });
    (rest_url, grpc_url)
}

fn determine_grpc_port(svc: &Service, annotation_key: &str) -> Option<u16> {
    if let Some(annotations) = &svc.metadata.annotations {
        if let Some(port_str) = annotations.get(annotation_key) {
            if let Ok(port) = port_str.parse::<u16>() {
                return Some(port);
            }
        }
    }

    if let Some(spec) = &svc.spec {
        for port in spec.ports.as_deref().unwrap_or_default() {
            if port.name.as_deref() == Some("grpc") {
                return Some(port.port as u16);
            }
        }
    }

    None
}

fn format_service_yaml(info: &ServiceInfo) -> String {
    let mut out = format!("  name: {}", info.name);
    out.push_str(&format!(
        "\n  capability_source: {}",
        match info.capability_source {
            crate::state::CapabilitySource::Grpc => "grpc",
            crate::state::CapabilitySource::OpenApi => "openapi",
        }
    ));

    if let Some(rest) = &info.rest {
        out.push_str(&format!("\n  rest:\n    url: {}", rest.url));
        if !rest.endpoints.is_empty() {
            out.push_str("\n    endpoints:");
            append_endpoints_yaml(&mut out, &rest.endpoints);
        }
    }

    if let Some(grpc) = &info.grpc {
        out.push_str(&format!("\n  grpc:\n    url: {}", grpc.url));
        if !grpc.endpoints.is_empty() {
            out.push_str("\n    endpoints:");
            for endpoint in &grpc.endpoints {
                out.push_str(&format!("\n      - service: {}", endpoint.service));
            }
        }
    }

    out
}

fn append_endpoints_yaml(out: &mut String, endpoints: &[crate::state::RestEndpoint]) {
    for endpoint in endpoints {
        out.push_str(&format!("\n      - name: {}", endpoint.name));
        if !endpoint.versions.is_empty() {
            out.push_str("\n        versions:");
            for (version, detail) in &endpoint.versions {
                out.push_str(&format!("\n          {version}:"));
                if let Some(path) = &detail.path {
                    out.push_str(&format!("\n            path: {path}"));
                }
            }
        }
    }
}

fn determine_rest_port(svc: &Service) -> Option<u16> {
    if let Some(spec) = &svc.spec {
        for port in spec.ports.as_deref().unwrap_or_default() {
            if port.name.as_deref() == Some("http") || port.name.as_deref() == Some("rest") {
                return Some(port.port as u16);
            }
        }
    }
    None
}

async fn retain_previous(
    svc_name: &str,
    state: &SharedState,
    target: &mut HashMap<String, ServiceInfo>,
) {
    let prev = state.read().await;
    if let Some(existing) = prev.get(svc_name) {
        target.insert(svc_name.to_string(), existing.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE_PATH_KEY: &str = "discovery.omniverse.nvidia.com/base-path";

    fn proxy(namespace: &str, fqdn: &str, route_service: &str, port: i32) -> HTTPProxy {
        serde_json::from_value(serde_json::json!({
            "apiVersion": "projectcontour.io/v1",
            "kind": "HTTPProxy",
            "metadata": {"name": format!("{namespace}-proxy"), "namespace": namespace},
            "spec": {
                "virtualhost": {"fqdn": fqdn},
                "routes": [{"services": [{"name": route_service, "port": port}]}]
            }
        }))
        .expect("valid HTTPProxy")
    }

    fn key(namespace: &str, name: &str) -> (String, String) {
        (namespace.to_string(), name.to_string())
    }

    #[test]
    fn proxy_routes_are_scoped_to_the_proxys_namespace() {
        let proxies = vec![
            proxy("ns-a", "a.example.test", "shared", 80),
            proxy("ns-b", "b.example.test", "shared", 80),
        ];
        let map = build_proxy_map(&proxies, BASE_PATH_KEY);

        assert_eq!(map[&key("ns-a", "shared")][0].url, "http://a.example.test");
        assert_eq!(map[&key("ns-b", "shared")][0].url, "http://b.example.test");
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn a_proxy_does_not_route_to_a_same_named_service_elsewhere() {
        let proxies = vec![proxy("ns-a", "a.example.test", "only-in-a", 80)];
        let map = build_proxy_map(&proxies, BASE_PATH_KEY);

        assert!(map.contains_key(&key("ns-a", "only-in-a")));
        assert!(!map.contains_key(&key("ns-b", "only-in-a")));
    }
}
