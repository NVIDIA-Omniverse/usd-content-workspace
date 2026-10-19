// SPDX-FileCopyrightText: Copyright 2026 NVIDIA CORPORATION & AFFILIATES
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

//! gRPC conformance tests for the `UserInfoService` defined in
//! `protos/nvidia/omniverse/userinfo/v1alpha/user_info_service.proto`.

pub mod groups;
pub mod users;

use crate::config::Args;
use crate::{HINT, TestError, TestRunner};
use std::time::Duration;
use tonic::Request;
use tonic::metadata::MetadataValue;
use tonic::transport::{Channel, ClientTlsConfig};

/// Generated UserInfoService client / message types.
pub mod proto {
    tonic::include_proto!("nvidia.omniverse.userinfo.v1alpha");
}

pub use proto::user_info_service_client::UserInfoServiceClient;

use crate::fixtures::SampledFixtures;

/// Top-level gRPC conformance entry point.
pub async fn run(
    runner: &mut TestRunner,
    client: &mut UserInfoServiceClient<Channel>,
    bearer: Option<&str>,
    args: &Args,
    data: &SampledFixtures,
) {
    users::run_tests(runner, client, bearer, args, data).await;
    groups::run_tests(runner, client, bearer, args, data).await;
}

/// Builds and connects a gRPC client to the configured endpoint. Reused by
/// the fixture-sampling phase so tests and fixture sampling share a single channel.
pub async fn build_client(
    grpc_url: &str,
    tls_enabled: bool,
    timeout_s: u64,
) -> Result<UserInfoServiceClient<Channel>, TestError> {
    let channel = build_channel(grpc_url, tls_enabled, timeout_s).await?;
    Ok(UserInfoServiceClient::new(channel))
}

/// Normalizes the user-provided endpoint to a URI accepted by `tonic::Endpoint`.
fn normalize_endpoint(raw: &str, tls_enabled: bool) -> String {
    let raw = raw.trim().trim_end_matches('/');
    if raw.starts_with("http://") || raw.starts_with("https://") {
        raw.to_string()
    } else if tls_enabled {
        format!("https://{raw}")
    } else {
        format!("http://{raw}")
    }
}

async fn build_channel(
    grpc_url: &str,
    tls_enabled: bool,
    timeout_s: u64,
) -> Result<Channel, TestError> {
    let uri = normalize_endpoint(grpc_url, tls_enabled);
    let mut endpoint = tonic::transport::Endpoint::from_shared(uri.clone())
        .map_err(|e| {
            TestError::new(
                format!("Invalid gRPC endpoint '{uri}': {e}"),
                format!("{HINT}   * Provide host:port or scheme://host:port (e.g. http://localhost:50051)"),
            )
        })?
        .timeout(Duration::from_secs(timeout_s))
        .connect_timeout(Duration::from_secs(timeout_s));
    if tls_enabled {
        let tls = ClientTlsConfig::new().with_native_roots();
        endpoint = endpoint.tls_config(tls).map_err(|e| {
            TestError::new(
                format!("gRPC TLS configuration error: {e}"),
                format!("{HINT}   * Ensure the OS root store contains a CA that signed the server certificate"),
            )
        })?;
    }
    endpoint
        .connect()
        .await
        .map_err(|e| TestError::connection_failed(&uri, &e.to_string()))
}

/// Wraps a message in a `Request` and attaches the bearer token in the
/// `authorization` metadata, matching the documented gRPC convention.
pub(crate) fn bearer_request<T>(msg: T, bearer: Option<&str>) -> Result<Request<T>, TestError> {
    let mut req = Request::new(msg);
    if let Some(token) = bearer {
        let value: MetadataValue<_> = format!("Bearer {token}").parse().map_err(|e| {
            TestError::new(
                format!("Invalid bearer token for gRPC metadata: {e}"),
                format!("{HINT}   * The token contains characters that cannot be placed in gRPC metadata"),
            )
        })?;
        req.metadata_mut().insert("authorization", value);
    }
    Ok(req)
}

/// Maps a `tonic::Status` to a `TestError` with hints tied back to the
/// gRPC API contract's error code table.
pub(crate) fn status_to_error(step: &str, status: &tonic::Status) -> TestError {
    use tonic::Code;
    let code = status.code();
    let msg = status.message();
    let hint = match code {
        Code::Unauthenticated => format!(
            "gRPC UNAUTHENTICATED: the service rejected the bearer token.\n\
             {HINT}   * Confirm credentials are configured (--bearer-token / --bearer-token-file / --oauth2-*)\n\
             {HINT}   * Server message: {msg}",
        ),
        Code::PermissionDenied => format!(
            "gRPC PERMISSION_DENIED: the caller is authenticated but lacks permission.\n\
             {HINT}   * Same causes as REST 403\n\
             {HINT}   * Server message: {msg}",
        ),
        Code::NotFound => format!(
            "gRPC NOT_FOUND: the target entity does not exist or is filtered out by post-authorization.\n\
             {HINT}   * The conformance test sourced the target identifier from a prior list call, so a NOT_FOUND here\n\
             {HINT}     usually means the entity was deleted between sampling and this call, or post-filter authorization\n\
             {HINT}     is hiding an entity the caller was allowed to list\n\
             {HINT}   * Server message: {msg}",
        ),
        Code::Unavailable => format!(
            "gRPC UNAVAILABLE: server unreachable or not ready.\n\
             {HINT}   * Initial directory sync may not have completed; increase --wait-retries / --wait-interval\n\
             {HINT}   * When behind ingress, gRPC requires HTTP/2 — toggle --grpc-tls-enabled to match the ingress scheme\n\
             {HINT}   * Server message: {msg}",
        ),
        Code::ResourceExhausted => format!(
            "gRPC RESOURCE_EXHAUSTED: typically identity-provider rate-limiting.\n\
             {HINT}   * Equivalent to REST 429; retry after a delay\n\
             {HINT}   * Server message: {msg}",
        ),
        Code::Internal => format!(
            "gRPC INTERNAL: unexpected server error.\n\
             {HINT}   * Inspect service logs and upstream dependencies\n\
             {HINT}   * Server message: {msg}",
        ),
        _ => format!(
            "gRPC call failed with status {code:?}.\n\
             {HINT}   * Server message: {msg}",
        ),
    };
    TestError::new(format!("{step} failed: {code:?}"), hint)
}

/// Asserts that the proto `User` satisfies the documented contract: required
/// `id`, `display_name`, and `username` are non-empty.
pub(crate) fn validate_user_proto(user: &proto::User) -> Result<(), TestError> {
    if user.id.is_empty() {
        return Err(TestError::new(
            "proto User.id is empty",
            format!("{HINT}   * proto User.id is required"),
        ));
    }
    if user.display_name.is_empty() {
        return Err(TestError::new(
            "proto User.display_name is empty",
            format!("{HINT}   * proto User.display_name is required"),
        ));
    }
    if user.username.is_empty() {
        return Err(TestError::new(
            "proto User.username is empty",
            format!("{HINT}   * proto User.username is required"),
        ));
    }
    Ok(())
}

/// Asserts that the proto `Group` satisfies the documented contract: required
/// `id` and `display_name` are non-empty.
pub(crate) fn validate_group_proto(group: &proto::Group) -> Result<(), TestError> {
    if group.id.is_empty() {
        return Err(TestError::new(
            "proto Group.id is empty",
            format!("{HINT}   * proto Group.id is required"),
        ));
    }
    if group.display_name.is_empty() {
        return Err(TestError::new(
            "proto Group.display_name is empty",
            format!("{HINT}   * proto Group.display_name is required"),
        ));
    }
    Ok(())
}

/// Formats a short human-readable summary of a gRPC request for failure logs.
pub(crate) fn format_grpc_request(method: &str, fields: &[(&str, &str)]) -> String {
    let mut out = format!("UserInfoService/{method}");
    for (k, v) in fields {
        out.push_str(&format!("\n  {k}: \"{v}\""));
    }
    out
}
