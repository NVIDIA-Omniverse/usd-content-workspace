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

use crate::{TestError, CHECK, HINT, INFO};
use clap::Parser;
use permission_auth::config::load_openid_configuration;
use permission_auth::service_identity::{self, AccessTokenFileArgs, ClientCredentialsArgs};
use std::path::Path;

#[derive(Clone, Debug, Default, clap::ValueEnum, PartialEq)]
pub enum TestSuite {
    #[default]
    All,
    Rest,
    Grpc,
}

#[derive(Parser, Debug)]
#[command(version, about = "Conformance test for permission-service", long_about = None)]
pub struct Args {
    #[arg(long, env = "REST_URL")]
    pub rest_url: Option<String>,

    #[arg(long, env = "GRPC_URL")]
    pub grpc_url: Option<String>,

    #[arg(long, env = "TEST_SUITE", value_enum, default_value_t = TestSuite::All)]
    pub test_suite: TestSuite,

    #[arg(long, env = "PRINCIPAL")]
    pub principal: String,

    #[arg(long, env = "ACTION_NAME")]
    pub action_name: String,

    #[arg(long, env = "ACTION_SERVICE")]
    pub action_service: String,

    #[arg(long, env = "RESOURCE_ID")]
    pub resource_id: String,

    #[arg(long, env = "RESOURCE_TYPE")]
    pub resource_type: String,

    #[arg(long, env = "WAIT_RETRIES", default_value = "10")]
    pub wait_retries: u32,

    #[arg(long, env = "WAIT_INTERVAL", default_value = "5")]
    pub wait_interval: u64,

    #[command(flatten)]
    pub client_credentials: ClientCredentialsArgs,

    #[command(flatten)]
    pub access_token_file: AccessTokenFileArgs,
}

/// Acquires a Bearer token via client credentials, access token file, or returns None if auth is disabled.
pub async fn acquire_auth_token(args: &Args) -> Result<Option<String>, TestError> {
    if args.client_credentials.enabled {
        acquire_via_client_credentials(args).await
    } else if args.access_token_file.enabled {
        let token = service_identity::read_token_file(Path::new(&args.access_token_file.file_path))
            .map_err(|e| {
                TestError::new(
                    e.to_string(),
                    format!(
                        "{}   Check that ACCESS_TOKEN_FILE_PATH is readable and contains access_token plus RFC3339 expires_on JSON",
                        HINT
                    ),
                )
            })?;
        println!("{} Access token loaded from file", CHECK);
        Ok(Some(token))
    } else {
        println!(
            "{} Authentication disabled, running without credentials",
            INFO
        );
        Ok(None)
    }
}

async fn acquire_via_client_credentials(args: &Args) -> Result<Option<String>, TestError> {
    let cc = &args.client_credentials;

    let client_secret = cc.get_client_secret().map_err(|e| {
        TestError::new(
            format!("Failed to resolve client secret: {}", e),
            format!("{}   Set CLIENT_SECRET or CLIENT_SECRET_FILE", HINT),
        )
    })?;

    let extra_headers = cc.get_extra_headers().map_err(|e| {
        TestError::new(
            format!("Failed to parse extra headers: {}", e),
            format!(
                "{}   CLIENT_CREDENTIALS_EXTRA_HEADERS must be valid JSON",
                HINT
            ),
        )
    })?;

    let extra_body_params = cc.get_extra_body_params().map_err(|e| {
        TestError::new(
            format!("Failed to parse extra body params: {}", e),
            format!(
                "{}   CLIENT_CREDENTIALS_EXTRA_BODY_PARAMS must be valid JSON",
                HINT
            ),
        )
    })?;

    println!(
        "{} Loading OpenID configuration from {}...",
        INFO, cc.openid_configuration
    );
    let http = reqwest::Client::new();
    let openid_response = load_openid_configuration(&cc.openid_configuration, &http)
        .await
        .map_err(|e| {
            TestError::new(
                format!("Failed to load OpenID configuration: {}", e),
                format!(
                    "{}   Ensure CLIENT_CREDENTIALS_OPENID_CONFIGURATION returns valid OpenID metadata",
                    HINT
                ),
            )
        })?;

    println!("{} Obtaining access token...", INFO);
    let response = service_identity::exchange_client_credentials_token(
        &http,
        &openid_response.token_endpoint,
        &cc.client_id,
        &client_secret,
        &cc.client_scope,
        &extra_headers,
        &extra_body_params,
    )
    .await
    .map_err(|e| {
        TestError::new(
            e.to_string(),
            format!(
                "{}   Check token endpoint and credentials configuration",
                HINT
            ),
        )
    })?;

    println!("{} Access token obtained", CHECK);
    Ok(Some(response.access_token))
}
