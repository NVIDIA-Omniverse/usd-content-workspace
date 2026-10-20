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

use auto_discovery_service::config::Config;
use auto_discovery_service::state::new_shared_state;
use auto_discovery_service::static_discovery::StaticEndpoint;
use auto_discovery_service::v1::V1EndpointMapping;
use auto_discovery_service::{api::rest, k8s::discovery, v1};
use clap::Parser;
use std::net::SocketAddr;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config = Arc::new(Config::parse());
    let internal_state = new_shared_state();
    let external_state = new_shared_state();

    if !config.static_endpoint.is_empty() {
        let endpoints = config
            .static_endpoint
            .iter()
            .map(|s| StaticEndpoint::parse(s))
            .collect::<Result<Vec<_>, _>>()?;
        tracing::info!(
            count = endpoints.len(),
            "static endpoint mode; skipping Kubernetes discovery"
        );
        let disc_internal = internal_state.clone();
        let disc_external = external_state.clone();
        let interval = config.reconcile_interval_secs;
        tokio::spawn(async move {
            auto_discovery_service::static_discovery::run_static_discovery(
                endpoints,
                interval,
                disc_internal,
                disc_external,
            )
            .await;
        });
    } else {
        let client = match &config.kube_context {
            Some(ctx) => {
                let options = kube::config::KubeConfigOptions {
                    context: Some(ctx.clone()),
                    ..Default::default()
                };
                let kc = kube::Config::from_kubeconfig(&options).await?;
                kube::Client::try_from(kc)?
            }
            None => kube::Client::try_default().await?,
        };
        tracing::info!("connected to kubernetes cluster");

        let disc_internal = internal_state.clone();
        let disc_external = external_state.clone();
        let discovery_config = config.clone();
        let discovery_client = client.clone();
        tokio::spawn(async move {
            discovery::run_discovery(
                discovery_client,
                discovery_config,
                disc_internal,
                disc_external,
            )
            .await;
        });
    }

    let internal_addr = SocketAddr::from(([0, 0, 0, 0], config.internal_port));
    let internal_listener = tokio::net::TcpListener::bind(internal_addr).await?;

    let external_addr = SocketAddr::from(([0, 0, 0, 0], config.external_port));
    let external_listener = tokio::net::TcpListener::bind(external_addr).await?;

    tracing::info!(
        "internal port: {}, external port: {}",
        config.internal_port,
        config.external_port
    );
    let v1_mappings: Arc<Vec<V1EndpointMapping>> = Arc::new(
        config
            .v1_endpoints_config
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(v1::parse_mappings)
            .transpose()?
            .inspect(|mappings| {
                tracing::info!(count = mappings.len(), "v1 endpoints mapping loaded");
            })
            .unwrap_or_default(),
    );

    let root_redirect = rest::RootRedirect::new(
        config.root_redirect_url.clone().unwrap_or_default(),
        config.root_redirect_status,
    );
    if let Some(redirect) = root_redirect.as_ref() {
        tracing::info!(
            url = %redirect.url(),
            status = %redirect.status().as_u16(),
            "root redirect enabled"
        );
    }

    tokio::select! {
        res = axum::serve(
            internal_listener,
            rest::router(
                internal_state,
                config.auth_config_json.clone(),
                root_redirect.clone(),
                v1_mappings.clone(),
            ),
        ) => res?,
        res = axum::serve(
            external_listener,
            rest::router(
                external_state,
                config.auth_config_json.clone(),
                root_redirect.clone(),
                v1_mappings.clone(),
            ),
        ) => res?,
    }

    Ok(())
}
