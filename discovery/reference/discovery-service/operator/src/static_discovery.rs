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
use crate::state::{ServiceInfo, SharedState};
use std::collections::HashMap;
use std::time::Duration;

pub struct StaticEndpoint {
    pub name: String,
    pub grpc_url: String,
    pub rest_url: Option<String>,
}

impl StaticEndpoint {
    /// Parse `NAME=GRPC_URL` or `NAME=GRPC_URL#REST_URL`.
    pub fn parse(s: &str) -> Result<Self, String> {
        let (name, value) = s
            .split_once('=')
            .ok_or_else(|| format!("expected NAME=GRPC_URL or NAME=GRPC_URL#REST_URL, got: {s}"))?;
        let (grpc_url, rest_url) = match value.split_once('#') {
            Some((grpc, rest)) if !rest.is_empty() => (grpc.to_string(), Some(rest.to_string())),
            _ => (value.to_string(), None),
        };
        if grpc_url.is_empty() {
            return Err(format!(
                "gRPC URL must not be empty in static endpoint: {s}"
            ));
        }
        Ok(Self {
            name: name.to_string(),
            grpc_url,
            rest_url,
        })
    }
}

pub async fn run_static_discovery(
    endpoints: Vec<StaticEndpoint>,
    interval_secs: u64,
    internal_state: SharedState,
    external_state: SharedState,
) {
    reconcile(&endpoints, &internal_state, &external_state).await;

    let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
    loop {
        interval.tick().await;
        reconcile(&endpoints, &internal_state, &external_state).await;
    }
}

async fn reconcile(
    endpoints: &[StaticEndpoint],
    internal_state: &SharedState,
    external_state: &SharedState,
) {
    let mut new_state: HashMap<String, ServiceInfo> = HashMap::new();

    for ep in endpoints {
        match capabilities::query_capabilities(&ep.grpc_url).await {
            Ok(caps) => {
                let info = ServiceInfo::from_capabilities(
                    ep.name.clone(),
                    ep.rest_url.clone(),
                    Some(ep.grpc_url.clone()),
                    caps,
                );
                new_state.insert(ep.name.clone(), info);
            }
            Err(e) => {
                tracing::warn!(endpoint = %ep.grpc_url, error = %e, "failed to query static endpoint");
                let prev = internal_state.read().await;
                if let Some(existing) = prev.get(&ep.name) {
                    new_state.insert(ep.name.clone(), existing.clone());
                }
            }
        }
    }

    let mut state = internal_state.write().await;
    *state = new_state.clone();
    drop(state);

    let mut state = external_state.write().await;
    *state = new_state;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_grpc_url_only() {
        let ep = StaticEndpoint::parse("mock=http://localhost:4770").expect("parse");
        assert_eq!(ep.name, "mock");
        assert_eq!(ep.grpc_url, "http://localhost:4770");
        assert_eq!(ep.rest_url, None);
    }

    #[test]
    fn parse_grpc_and_rest_urls() {
        let ep = StaticEndpoint::parse("mock=http://localhost:4770#http://localhost:8080")
            .expect("parse");
        assert_eq!(ep.name, "mock");
        assert_eq!(ep.grpc_url, "http://localhost:4770");
        assert_eq!(ep.rest_url.as_deref(), Some("http://localhost:8080"));
    }
}
