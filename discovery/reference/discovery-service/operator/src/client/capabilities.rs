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

use crate::proto::capabilities_service_client::CapabilitiesServiceClient;
use crate::proto::{ListServicesRequest, ListServicesResponse};
use crate::state::CapabilityEntry;
use http::uri::PathAndQuery;
use std::time::Duration;
use tonic::transport::Channel;
use tonic::{Code, Status};

/// Deprecated v2alpha method path, used only when a service does not implement v2beta.
const V2ALPHA_LIST_SERVICES: &str =
    "/nvidia.omniverse.discovery.capabilities.v2alpha.CapabilitiesService/ListServices";

/// Upper bound on `ListServices` pages per query, guarding against a server that
/// never returns an empty `next_page_token`.
const MAX_PAGES: usize = 100;

#[derive(Clone, Copy)]
enum CapabilitiesVersion {
    V2beta,
    V2alpha,
}

/// Calls the deprecated v2alpha `ListServices`. The v2alpha and v2beta messages are
/// wire-identical, so the v2beta types are reused against the v2alpha method path.
async fn list_services_v2alpha(
    channel: Channel,
    request: ListServicesRequest,
) -> Result<ListServicesResponse, Status> {
    let mut grpc = tonic::client::Grpc::new(channel);
    grpc.ready()
        .await
        .map_err(|e| Status::unknown(format!("Service was not ready: {e}")))?;
    grpc.unary(
        tonic::Request::new(request),
        PathAndQuery::from_static(V2ALPHA_LIST_SERVICES),
        tonic_prost::ProstCodec::default(),
    )
    .await
    .map(tonic::Response::into_inner)
}

async fn list_services_page(
    channel: Channel,
    version: CapabilitiesVersion,
    page_token: String,
) -> Result<ListServicesResponse, Status> {
    let request = ListServicesRequest {
        page_token,
        ..Default::default()
    };
    match version {
        CapabilitiesVersion::V2beta => CapabilitiesServiceClient::new(channel)
            .list_services(request)
            .await
            .map(tonic::Response::into_inner),
        CapabilitiesVersion::V2alpha => list_services_v2alpha(channel, request).await,
    }
}

/// Fetches every `ListServices` page. The API version is chosen on the first page
/// (v2beta, falling back to v2alpha on `UNIMPLEMENTED`) and reused for later pages.
/// Any page failure fails the whole query so a partial result never replaces a
/// previously complete one.
async fn query_from_channel(
    channel: Channel,
    label: &str,
) -> Result<Vec<CapabilityEntry>, Box<dyn std::error::Error + Send + Sync>> {
    let (version, first_page) =
        match list_services_page(channel.clone(), CapabilitiesVersion::V2beta, String::new()).await
        {
            Ok(page) => (CapabilitiesVersion::V2beta, page),
            Err(status) if status.code() == Code::Unimplemented => {
                let page = list_services_page(
                    channel.clone(),
                    CapabilitiesVersion::V2alpha,
                    String::new(),
                )
                .await
                .map_err(|e| {
                    format!(
                        "ListServices RPC on {label} (v2beta unimplemented, v2alpha fallback): {e}"
                    )
                })?;
                tracing::warn!(
                    endpoint = %label,
                    "service only implements the deprecated capabilities v2alpha API; \
                     v2alpha will be removed in a future release, migrate to v2beta"
                );
                (CapabilitiesVersion::V2alpha, page)
            }
            Err(e) => return Err(format!("ListServices RPC on {label}: {e}").into()),
        };

    let mut services = first_page.services;
    let mut page_token = first_page.next_page_token;
    let mut pages = 1;
    while !page_token.is_empty() {
        if pages >= MAX_PAGES {
            return Err(
                format!("ListServices on {label} returned more than {MAX_PAGES} pages").into(),
            );
        }
        let page = list_services_page(channel.clone(), version, page_token.clone())
            .await
            .map_err(|e| format!("ListServices RPC on {label} (page {}): {e}", pages + 1))?;
        if page.next_page_token == page_token {
            return Err(format!(
                "ListServices on {label} returned the same page token it was sent"
            )
            .into());
        }
        services.extend(page.services);
        page_token = page.next_page_token;
        pages += 1;
    }

    let mut entries: Vec<CapabilityEntry> = services
        .into_iter()
        .map(CapabilityEntry::from_proto)
        .collect();
    entries.sort_by(|a, b| a.service_name.cmp(&b.service_name));

    Ok(entries)
}

pub async fn query_capabilities(
    endpoint: &str,
) -> Result<Vec<CapabilityEntry>, Box<dyn std::error::Error + Send + Sync>> {
    let channel = Channel::from_shared(endpoint.to_string())?
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .connect()
        .await
        .map_err(|e| format!("connect to {endpoint}: {e}"))?;

    query_from_channel(channel, endpoint).await
}

pub async fn query_capabilities_via_stream(
    stream: impl tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + Unpin + 'static,
    label: &str,
) -> Result<Vec<CapabilityEntry>, Box<dyn std::error::Error + Send + Sync>> {
    let io = hyper_util::rt::TokioIo::new(stream);
    let io_slot = std::sync::Mutex::new(Some(io));

    let channel =
        tonic::transport::Endpoint::from_static("http://[::]:50051")
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .connect_with_connector(tower::service_fn(move |_: http::Uri| {
                let io = io_slot.lock().unwrap().take();
                async move {
                    io.ok_or_else(|| std::io::Error::other("portforward stream already consumed"))
                }
            }))
            .await
            .map_err(|e| format!("connect via portforward for {label}: {e}"))?;

    query_from_channel(channel, label).await
}
