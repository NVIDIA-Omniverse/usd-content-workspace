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

//! CLI/env configuration, mirroring the `serviceIdentity` auth conventions the
//! main `helm/thumbnail-service` chart and `thumbnail-smoke-test` already use.

use std::time::Duration;

use clap::{Parser, ValueEnum};

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lower")]
pub enum TestSuite {
    All,
    Rest,
    Grpc,
}

#[derive(Parser, Debug)]
#[command(
    version,
    about = "Black-box REST/gRPC conformance test for a deployed thumbnail-service"
)]
pub struct Args {
    /// HTTP base URL of the target deployment, e.g. http://localhost:3000.
    /// Required for OpenAPI verification in every suite, including --test-suite=grpc.
    #[arg(long, env = "REST_URL")]
    pub rest_url: Option<String>,

    /// gRPC endpoint of the target deployment, e.g. http://localhost:50051.
    /// Required unless --test-suite=rest.
    #[arg(long, env = "GRPC_URL")]
    pub grpc_url: Option<String>,

    /// Enable TLS on the gRPC channel.
    #[arg(long, env = "GRPC_TLS_ENABLED", default_value_t = false)]
    pub grpc_tls_enabled: bool,

    /// Which suite(s) to run.
    #[arg(long, env = "TEST_SUITE", value_enum, default_value_t = TestSuite::All)]
    pub test_suite: TestSuite,

    /// Source file the suite associates and disassociates its own fixtures
    /// against. Must already exist in storage: the association cases need no
    /// seeded data, but CreateThumbnails resolves the source identity with a
    /// storage Stat. The run also generates thumbnails from it that the suite
    /// cannot remove. And against a deployment that does not enforce the
    /// 1000-specification bound, the case checking that bound leaves one
    /// association per specification here, which can make this object's
    /// metadata too large to enumerate and break reads for it permanently — a
    /// conformant deployment refuses the request and writes nothing. Name an
    /// object whose associations can be abandoned. See AGENTS.md "Fixture
    /// sampling".
    #[arg(long, env = "SOURCE_FILE")]
    pub source_file: String,

    // ----- Authentication: exactly one of the following, or none for an
    // unauthenticated run -----
    /// Direct bearer token.
    #[arg(long, env = "BEARER_TOKEN")]
    pub bearer_token: Option<String>,

    /// Path to a JSON file with an `access_token` field (and optional
    /// `expires_on`), typically populated by a token-provisioner sidecar.
    /// Read once at startup.
    #[arg(long, env = "ACCESS_TOKEN_FILE")]
    pub access_token_file: Option<String>,

    /// OpenID Connect discovery document URL. The token endpoint is resolved
    /// dynamically from it — there is no separate token-url flag.
    #[arg(long, env = "OAUTH2_OPENID_CONFIGURATION_URI")]
    pub oauth2_openid_configuration_uri: Option<String>,

    /// OAuth2 client id.
    #[arg(long, env = "OAUTH2_CLIENT_ID")]
    pub oauth2_client_id: Option<String>,

    /// OAuth2 client secret. Mutually exclusive with --oauth2-client-secret-file.
    #[arg(long, env = "OAUTH2_CLIENT_SECRET")]
    pub oauth2_client_secret: Option<String>,

    /// Path to a file containing the OAuth2 client secret.
    #[arg(long, env = "OAUTH2_CLIENT_SECRET_FILE")]
    pub oauth2_client_secret_file: Option<String>,

    /// Scope passed to the token endpoint.
    #[arg(long, env = "OAUTH2_CLIENT_SCOPE")]
    pub oauth2_client_scope: Option<String>,

    /// REST path used by the readiness wait loop.
    #[arg(long, env = "READY_PATH", default_value = "/health")]
    pub ready_path: String,

    /// Per-request timeout, e.g. "30s". Applies to both transports: every REST
    /// request, and the gRPC channel's connect and per-call deadlines. The name
    /// predates the gRPC suite; the value has always been the run's one request
    /// bound, and leaving gRPC unbounded meant a deployment that accepted a
    /// connection and never answered hung the run with no report at all.
    #[arg(long, env = "HTTP_TIMEOUT", default_value = "30s", value_parser = humantime::parse_duration)]
    pub http_timeout: Duration,

    /// Readiness probe retry count.
    #[arg(long, env = "WAIT_RETRIES", default_value_t = 10)]
    pub wait_retries: u32,

    /// Interval between readiness retries, e.g. "5s".
    #[arg(long, env = "WAIT_INTERVAL", default_value = "5s", value_parser = humantime::parse_duration)]
    pub wait_interval: Duration,
}

impl Args {
    pub fn runs_rest(&self) -> bool {
        matches!(self.test_suite, TestSuite::All | TestSuite::Rest)
    }

    pub fn runs_grpc(&self) -> bool {
        matches!(self.test_suite, TestSuite::All | TestSuite::Grpc)
    }
}
