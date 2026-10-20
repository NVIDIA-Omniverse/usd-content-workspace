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

//! gRPC transport smoke tests.
//!
//! Mirrors the REST test cases over the service's gRPC port so that any
//! regression specific to the gRPC stack — port wiring, HTTP/2 ingress, TLS
//! configuration, or a missing UserInfoService registration — is surfaced
//! with the same detailed remediation hints as the REST suite.

use std::time::{Duration, Instant};

use tonic::Request;
use tonic::metadata::MetadataValue;
use tonic::transport::{Channel, ClientTlsConfig};

use crate::{HINT, INFO, TestError, TestRunner};

/// Generated UserInfoService client / message types.
pub mod proto {
    tonic::include_proto!("nvidia.omniverse.userinfo.v1alpha");
}

use proto::user_info_service_client::UserInfoServiceClient;

/// Runtime configuration needed to drive the gRPC test suite.
pub struct GrpcSettings {
    /// Raw endpoint as provided by the user; accepts either `host:port` or `scheme://host:port`.
    pub endpoint: String,
    /// When true the client connects over TLS using the OS trust store.
    pub tls_enabled: bool,
    /// Per-call timeout. Also used as the connect timeout.
    pub timeout: Duration,
    /// `page_size` sent with ListUsers / ListGroups / ListUserGroups requests.
    pub page_limit: u32,
}

/// Normalizes the user-provided endpoint to a URI that `tonic::Endpoint` accepts.
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

async fn build_channel(settings: &GrpcSettings) -> Result<Channel, TestError> {
    let uri = normalize_endpoint(&settings.endpoint, settings.tls_enabled);
    let mut endpoint = tonic::transport::Endpoint::from_shared(uri.clone())
        .map_err(|e| {
            TestError::new(
                format!("Invalid gRPC endpoint '{uri}': {e}"),
                format!(
                    "{HINT}   * Endpoint must be a valid URI (host:port or scheme://host:port)\n\
                     {HINT}   * Helm value `smokeTest.grpc.endpoint` or env `GRPC_ENDPOINT`",
                ),
            )
        })?
        .timeout(settings.timeout)
        .connect_timeout(settings.timeout);

    if settings.tls_enabled {
        let tls = ClientTlsConfig::new().with_native_roots();
        endpoint = endpoint.tls_config(tls).map_err(|e| {
            TestError::new(
                format!("Invalid gRPC TLS configuration: {e}"),
                format!(
                    "{HINT}   * Helm value `smokeTest.grpc.tls.enabled` is true but TLS setup failed\n\
                     {HINT}   * Ensure the gRPC endpoint is served over HTTPS and the server certificate is trusted by the OS root store",
                ),
            )
        })?;
    }

    endpoint.connect().await.map_err(|e| {
        TestError::new(
            format!("gRPC connect to {uri} failed: {e}"),
            format!(
                "{HINT}   * Service exposes the gRPC port (Helm value `service.ports.grpc`, default 50051)\n\
                 {HINT}   * Pod listens on `GRPC_PORT` (matches `service.ports.grpc`)\n\
                 {HINT}   * When connecting through an ingress, gRPC requires HTTP/2 and usually TLS; toggle `smokeTest.grpc.tls.enabled`\n\
                 {HINT}   * Resolved endpoint: {uri}",
            ),
        )
    })
}

/// Wraps a message in a `Request` and, when a bearer token is provided,
/// attaches it in the `authorization` metadata, matching how the service's
/// gRPC handlers extract credentials. When `bearer` is `None`, the request is
/// sent without an `authorization` entry — matching deployments that do not
/// front the gRPC port with an auth extension.
fn bearer_request<T>(msg: T, bearer: Option<&str>) -> Result<Request<T>, TestError> {
    let mut req = Request::new(msg);
    if let Some(token) = bearer {
        let value: MetadataValue<_> = format!("Bearer {token}").parse().map_err(|e| {
            TestError::new(
                format!("Invalid bearer token for gRPC metadata: {e}"),
                format!(
                    "{HINT}   * The Entra ID token contains characters that cannot be placed in a gRPC metadata value\n\
                     {HINT}   * The REST tests passed earlier, so the token itself is valid; re-running should be sufficient",
                ),
            )
        })?;
        req.metadata_mut().insert("authorization", value);
    }
    Ok(req)
}

/// Maps a `tonic::Status` onto a `TestError` with remediation hints that mirror
/// the REST status-code hints so operators see a consistent diagnosis regardless
/// of transport.
fn status_to_error(step: &str, status: &tonic::Status) -> TestError {
    use tonic::Code;
    let code = status.code();
    let msg = status.message();
    let hint = match code {
        Code::Unauthenticated => format!(
            "gRPC Unauthenticated: the service refused the bearer token.\n\
             {HINT}   * Same causes as HTTP 401 on REST: `permission.enabled`, token issuer / audience\n\
             {HINT}   * Check service logs for 'missing or invalid authorization metadata'\n\
             {HINT}   * Server message: {msg}",
        ),
        Code::PermissionDenied => format!(
            "gRPC PermissionDenied: authorization was denied by the Permission Service.\n\
             {HINT}   * Same causes as HTTP 403 on REST\n\
             {HINT}   * Helm value `permission.serviceName` (default 'userinfo')\n\
             {HINT}   * Server message: {msg}",
        ),
        Code::NotFound => format!(
            "gRPC NotFound: the target resource does not exist or was filtered out by permissions.\n\
             {HINT}   * Same causes as HTTP 404 on REST\n\
             {HINT}   * Server message: {msg}",
        ),
        Code::Unavailable => format!(
            "gRPC Unavailable: the server is unreachable or not ready.\n\
             {HINT}   * Initial directory sync may still be in progress (see REST readiness hints)\n\
             {HINT}   * When behind an ingress, confirm HTTP/2 support and that `smokeTest.grpc.tls.enabled` matches the ingress scheme\n\
             {HINT}   * `kubectl get endpoints` should show the pod IP on the gRPC port\n\
             {HINT}   * Server message: {msg}",
        ),
        Code::ResourceExhausted => format!(
            "gRPC ResourceExhausted: typically Microsoft Graph rate-limiting.\n\
             {HINT}   * Same causes as HTTP 429 on REST; increase `providers.entra.maxRetries`\n\
             {HINT}   * Server message: {msg}",
        ),
        Code::Internal => format!(
            "gRPC Internal: the service hit an unexpected error.\n\
             {HINT}   * Check service logs: `kubectl logs -l app.kubernetes.io/name=microsoft-graph-userinfo-service`\n\
             {HINT}   * Server message: {msg}",
        ),
        _ => format!(
            "gRPC call failed with status {code:?}.\n\
             {HINT}   * Server message: {msg}",
        ),
    };
    TestError::new(format!("{step} failed: {code:?}"), hint)
}

/// Invokes the standard `grpc.health.v1.Health/Check` RPC and treats anything other
/// than `SERVING` as a failure with readiness-oriented hints.
async fn grpc_health_check(channel: Channel) -> Result<(), TestError> {
    use tonic_health::pb::HealthCheckRequest;
    use tonic_health::pb::health_check_response::ServingStatus;
    use tonic_health::pb::health_client::HealthClient;

    let mut client = HealthClient::new(channel);
    let resp = client
        .check(HealthCheckRequest {
            service: String::new(),
        })
        .await
        .map_err(|s| status_to_error("grpc.health.v1.Health/Check", &s))?;
    let status = resp.into_inner().status;
    if status == ServingStatus::Serving as i32 {
        Ok(())
    } else {
        let name = match ServingStatus::try_from(status) {
            Ok(ServingStatus::Unknown) => "UNKNOWN",
            Ok(ServingStatus::Serving) => "SERVING",
            Ok(ServingStatus::NotServing) => "NOT_SERVING",
            Ok(ServingStatus::ServiceUnknown) => "SERVICE_UNKNOWN",
            _ => "UNRECOGNIZED",
        };
        Err(TestError::new(
            format!("gRPC health status is {name} (expected SERVING)"),
            format!(
                "{HINT}   * The service is not marked SERVING yet; initial sync has not completed\n\
                 {HINT}   * Same readiness hints as REST /readyz 503 apply",
            ),
        ))
    }
}

/// Runs the gRPC test suite. Stops on the first failure and returns `Err(())`.
/// When `bearer` is `None`, requests are sent without an `authorization`
/// metadata entry — matching deployments without an auth extension.
pub async fn run_grpc_tests(
    runner: &mut TestRunner,
    settings: &GrpcSettings,
    bearer: Option<&str>,
) -> Result<(), ()> {
    println!();
    println!("Test 5: gRPC connectivity and health");
    println!(
        "   {INFO} connects to the gRPC endpoint '{}' and invokes grpc.health.v1.Health/Check",
        settings.endpoint,
    );
    let start = Instant::now();
    let channel = match build_channel(settings).await {
        Ok(c) => c,
        Err(e) => {
            runner.record_fail("Connect to gRPC endpoint", start.elapsed().as_millis(), &e);
            return Err(());
        }
    };
    if let Err(e) = grpc_health_check(channel.clone()).await {
        runner.record_fail("gRPC health check", start.elapsed().as_millis(), &e);
        return Err(());
    }
    runner.record_pass("gRPC health check", start.elapsed().as_millis());

    let mut client = UserInfoServiceClient::new(channel);

    println!();
    println!("Test 5a: UserInfoService.ListUsers");
    println!("   {INFO} mirrors REST 4a: returns a page with user entries and a next_page_token");
    let start = Instant::now();
    let req = match bearer_request(
        proto::ListUsersRequest {
            page_size: settings.page_limit as i32,
            ..Default::default()
        },
        bearer,
    ) {
        Ok(r) => r,
        Err(e) => {
            runner.record_fail("ListUsers (gRPC)", start.elapsed().as_millis(), &e);
            return Err(());
        }
    };
    let users_resp = match client.list_users(req).await {
        Ok(r) => r.into_inner(),
        Err(s) => {
            runner.record_fail(
                "ListUsers (gRPC)",
                start.elapsed().as_millis(),
                &status_to_error("UserInfoService/ListUsers", &s),
            );
            return Err(());
        }
    };
    if users_resp.users.is_empty() {
        runner.record_fail(
            "ListUsers (gRPC)",
            start.elapsed().as_millis(),
            &TestError::new(
                "UserInfoService/ListUsers returned 0 users",
                format!(
                    "{HINT}   * Same causes as an empty REST /api/v1alpha/users response (see test 4a hints)\n\
                     {HINT}   * If REST 4a passed but gRPC 5a is empty, the gRPC service is reading a different provider state — check logs",
                ),
            ),
        );
        return Err(());
    }
    println!(
        "   {INFO} received {} user(s), next_page_token={}",
        users_resp.users.len(),
        if users_resp.next_page_token.is_empty() {
            "<none>"
        } else {
            "<present>"
        },
    );
    runner.record_pass("ListUsers (gRPC)", start.elapsed().as_millis());
    let sample_user_id = users_resp.users[0].id.clone();

    println!();
    println!("Test 5b: UserInfoService.GetUser");
    println!("   {INFO} mirrors REST 4b: fetches user '{sample_user_id}' via gRPC");
    let start = Instant::now();
    let req = match bearer_request(
        proto::GetUserRequest {
            user_id: sample_user_id.clone(),
        },
        bearer,
    ) {
        Ok(r) => r,
        Err(e) => {
            runner.record_fail("GetUser (gRPC)", start.elapsed().as_millis(), &e);
            return Err(());
        }
    };
    if let Err(s) = client.get_user(req).await {
        runner.record_fail(
            "GetUser (gRPC)",
            start.elapsed().as_millis(),
            &status_to_error("UserInfoService/GetUser", &s),
        );
        return Err(());
    }
    runner.record_pass("GetUser (gRPC)", start.elapsed().as_millis());

    println!();
    println!("Test 5c: UserInfoService.ListGroups");
    println!("   {INFO} mirrors REST 4c: returns a page with group entries and a next_page_token");
    let start = Instant::now();
    let req = match bearer_request(
        proto::ListGroupsRequest {
            page_size: settings.page_limit as i32,
            ..Default::default()
        },
        bearer,
    ) {
        Ok(r) => r,
        Err(e) => {
            runner.record_fail("ListGroups (gRPC)", start.elapsed().as_millis(), &e);
            return Err(());
        }
    };
    let groups_resp = match client.list_groups(req).await {
        Ok(r) => r.into_inner(),
        Err(s) => {
            runner.record_fail(
                "ListGroups (gRPC)",
                start.elapsed().as_millis(),
                &status_to_error("UserInfoService/ListGroups", &s),
            );
            return Err(());
        }
    };
    if groups_resp.groups.is_empty() {
        runner.record_fail(
            "ListGroups (gRPC)",
            start.elapsed().as_millis(),
            &TestError::new(
                "UserInfoService/ListGroups returned 0 groups",
                format!(
                    "{HINT}   * Same causes as an empty REST /api/v1alpha/groups response (see test 4c hints)",
                ),
            ),
        );
        return Err(());
    }
    println!(
        "   {INFO} received {} group(s), next_page_token={}",
        groups_resp.groups.len(),
        if groups_resp.next_page_token.is_empty() {
            "<none>"
        } else {
            "<present>"
        },
    );
    runner.record_pass("ListGroups (gRPC)", start.elapsed().as_millis());
    let sample_group_id = groups_resp.groups[0].id.clone();

    println!();
    println!("Test 5d: UserInfoService.GetGroup");
    println!("   {INFO} mirrors REST 4d: fetches group '{sample_group_id}' via gRPC");
    let start = Instant::now();
    let req = match bearer_request(
        proto::GetGroupRequest {
            group_id: sample_group_id.clone(),
        },
        bearer,
    ) {
        Ok(r) => r,
        Err(e) => {
            runner.record_fail("GetGroup (gRPC)", start.elapsed().as_millis(), &e);
            return Err(());
        }
    };
    if let Err(s) = client.get_group(req).await {
        runner.record_fail(
            "GetGroup (gRPC)",
            start.elapsed().as_millis(),
            &status_to_error("UserInfoService/GetGroup", &s),
        );
        return Err(());
    }
    runner.record_pass("GetGroup (gRPC)", start.elapsed().as_millis());

    println!();
    println!("Test 5e: UserInfoService.ListUserGroups");
    println!(
        "   {INFO} mirrors REST 4e: fetches group membership of user '{sample_user_id}' via gRPC"
    );
    let start = Instant::now();
    let req = match bearer_request(
        proto::ListUserGroupsRequest {
            user_id: sample_user_id.clone(),
            page_size: settings.page_limit as i32,
            ..Default::default()
        },
        bearer,
    ) {
        Ok(r) => r,
        Err(e) => {
            runner.record_fail("ListUserGroups (gRPC)", start.elapsed().as_millis(), &e);
            return Err(());
        }
    };
    if let Err(s) = client.list_user_groups(req).await {
        runner.record_fail(
            "ListUserGroups (gRPC)",
            start.elapsed().as_millis(),
            &status_to_error("UserInfoService/ListUserGroups", &s),
        );
        return Err(());
    }
    runner.record_pass("ListUserGroups (gRPC)", start.elapsed().as_millis());

    Ok(())
}
