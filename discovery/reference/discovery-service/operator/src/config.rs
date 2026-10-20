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

use clap::{Args, Parser};

#[derive(Parser, Clone)]
#[command(
    name = "auto-discovery-service",
    about = "Discovers gRPC/REST services in a Kubernetes cluster"
)]
pub struct Config {
    #[arg(long, default_value = "8080", env = "INTERNAL_PORT")]
    pub internal_port: u16,

    #[arg(long, default_value = "8081", env = "EXTERNAL_PORT")]
    pub external_port: u16,

    #[command(flatten)]
    pub annotations: AnnotationConfig,

    /// Namespaces to discover Services and HTTPProxies in, comma-separated
    /// (e.g. `storage-api-dev,usdsearch`). Empty or unset watches every
    /// namespace. Use `watched_namespaces()` rather than reading this directly:
    /// it trims, drops empty entries and removes duplicates.
    #[arg(long, env = "KUBE_NAMESPACE", value_delimiter = ',')]
    pub kube_namespace: Vec<String>,

    #[arg(long, env = "KUBE_CONTEXT")]
    pub kube_context: Option<String>,

    #[arg(long, default_value = "30", env = "RECONCILE_INTERVAL")]
    pub reconcile_interval_secs: u64,

    #[arg(long, default_value = "false", env = "USE_KUBE_PORTFORWARD")]
    pub use_kube_portforward: bool,

    #[arg(long, env = "AUTH_CONFIG_JSON")]
    pub auth_config_json: Option<String>,

    #[arg(long, env = "V1_ENDPOINTS_CONFIG")]
    pub v1_endpoints_config: Option<String>,

    /// When set, GET / returns an HTTP redirect to this absolute URL.
    /// Used so Contour can disable ext_authz on `/` (service routes honor
    /// authPolicy.disabled; requestRedirectPolicy routes do not).
    #[arg(long, env = "ROOT_REDIRECT_URL")]
    pub root_redirect_url: Option<String>,

    /// Status code for the root redirect. Defaults to 302.
    #[arg(long, default_value = "302", env = "ROOT_REDIRECT_STATUS")]
    pub root_redirect_status: u16,

    /// Skip Kubernetes discovery and query these gRPC endpoints directly.
    /// Format: NAME=GRPC_URL or NAME=GRPC_URL#REST_URL,
    /// e.g. --static-endpoint mock=http://localhost:4770#http://localhost:8080
    /// Can be repeated for multiple services.
    #[arg(
        long,
        value_name = "NAME=GRPC_URL[#REST_URL]",
        env = "STATIC_ENDPOINTS",
        value_delimiter = ','
    )]
    pub static_endpoint: Vec<String>,
}

impl Config {
    /// The namespaces to watch, in the order given. Empty means all namespaces.
    pub fn watched_namespaces(&self) -> Vec<String> {
        normalize_namespaces(&self.kube_namespace)
    }
}

/// Trims each entry, drops empty ones and removes duplicates, keeping the first
/// occurrence so the configured order (which decides name collisions) survives.
pub fn normalize_namespaces(raw: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for entry in raw {
        let ns = entry.trim();
        if !ns.is_empty() && !out.iter().any(|seen| seen == ns) {
            out.push(ns.to_string());
        }
    }
    out
}

#[derive(Args, Clone)]
pub struct AnnotationConfig {
    #[arg(
        long,
        default_value = "discovery.omniverse.nvidia.com/enabled",
        env = "ANNOTATION_DISCOVERY"
    )]
    pub discovery_enabled: String,

    #[arg(
        long,
        default_value = "discovery.omniverse.nvidia.com/grpc-port",
        env = "ANNOTATION_GRPC_PORT"
    )]
    pub grpc_port: String,

    #[arg(
        long,
        default_value = "discovery.omniverse.nvidia.com/base-path",
        env = "ANNOTATION_BASE_PATH"
    )]
    pub base_path: String,

    #[arg(
        long,
        default_value = "discovery.omniverse.nvidia.com/visibility",
        env = "ANNOTATION_VISIBILITY"
    )]
    pub visibility: String,

    #[arg(
        long,
        default_value = "discovery.omniverse.nvidia.com/external-rest-endpoint",
        env = "ANNOTATION_EXTERNAL_REST_ENDPOINT"
    )]
    pub external_rest_endpoint: String,

    #[arg(
        long,
        default_value = "discovery.omniverse.nvidia.com/external-grpc-endpoint",
        env = "ANNOTATION_EXTERNAL_GRPC_ENDPOINT"
    )]
    pub external_grpc_endpoint: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(env_value: Option<&str>) -> Vec<String> {
        let mut args = vec!["auto-discovery-service"];
        let flag;
        if let Some(v) = env_value {
            flag = format!("--kube-namespace={v}");
            args.push(&flag);
        }
        Config::parse_from(args).watched_namespaces()
    }

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn unset_means_all_namespaces() {
        assert!(parse(None).is_empty());
    }

    #[test]
    fn single_namespace_is_unchanged() {
        assert_eq!(
            parse(Some("storage-api-dev")),
            strings(&["storage-api-dev"])
        );
    }

    #[test]
    fn comma_separated_list_keeps_order() {
        assert_eq!(
            parse(Some("storage-api-dev,usdsearch")),
            strings(&["storage-api-dev", "usdsearch"])
        );
    }

    #[test]
    fn whitespace_empty_entries_and_duplicates_are_dropped() {
        assert_eq!(parse(Some(" b , a,,b, ,a ,c,")), strings(&["b", "a", "c"]));
    }

    #[test]
    fn only_separators_and_blanks_mean_all_namespaces() {
        assert!(parse(Some(" , ,")).is_empty());
        assert!(parse(Some("")).is_empty());
    }

    #[test]
    fn normalize_handles_entries_that_were_not_split() {
        assert_eq!(
            normalize_namespaces(&strings(&[" a ", "", "a", "b"])),
            strings(&["a", "b"])
        );
    }
}
