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

//! OpenID Connect discovery helper, mirroring `permission-auth::config`'s
//! `load_openid_configuration` plus the small fields of `OpenIdConfigResponse`
//! that the in-process client-credentials flow actually needs.

use reqwest::Client;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct OpenIdConfigResponse {
    pub issuer: String,

    /// The identity provider endpoint used to obtain and verify access tokens.
    pub token_endpoint: String,
}

/// Fetches the OpenID Connect discovery document from `openid_configuration`
/// and parses out the fields the in-process token-exchange flow needs.
pub async fn load_openid_configuration(
    openid_configuration: &str,
    client: &Client,
) -> Result<OpenIdConfigResponse, String> {
    let response = client
        .get(openid_configuration)
        .send()
        .await
        .map_err(|err| format!("Failed to get OpenID configuration: {:?}", err))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "OpenID discovery returned HTTP {} from {}: {}",
            status, openid_configuration, body
        ));
    }

    response
        .json::<OpenIdConfigResponse>()
        .await
        .map_err(|err| {
            format!(
                "Failed to parse OpenID configuration response from {}. \
             Expected JSON with 'issuer' and 'token_endpoint' fields. \
             Error: {}",
                openid_configuration, err
            )
        })
}

#[cfg(test)]
mod tests {
    use super::load_openid_configuration;
    use reqwest::Client;
    use std::net::SocketAddr;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Answer exactly one request with a fixed status line and body, then close.
    ///
    /// A raw listener rather than a mock-HTTP crate: the two error paths under test
    /// need a status code and a body, and nothing else, so a new dependency would
    /// cost more than it explains.
    async fn serve_once(status_line: &'static str, body: &'static str) -> SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind an ephemeral loopback port");
        let addr = listener.local_addr().expect("local addr");

        tokio::spawn(async move {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            // Read and discard the request; the response does not depend on it.
            let mut scratch = [0_u8; 1024];
            let _ = socket.read(&mut scratch).await;

            let response = format!(
                "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.flush().await;
        });

        addr
    }

    #[tokio::test]
    async fn a_complete_document_is_parsed() {
        let addr = serve_once(
            "200 OK",
            r#"{"issuer":"https://idp.example","token_endpoint":"https://idp.example/token"}"#,
        )
        .await;

        let config = load_openid_configuration(
            &format!("http://{addr}/.well-known/openid-configuration"),
            &Client::new(),
        )
        .await
        .expect("a well-formed document must parse");

        assert_eq!(config.issuer, "https://idp.example");
        assert_eq!(config.token_endpoint, "https://idp.example/token");
    }

    /// A non-2xx must report the status *and* the body.
    ///
    /// Discovery failing is a startup failure, and the body is usually the only
    /// thing that says why — an identity provider answering 403 with a reason is
    /// far more useful than "discovery failed".
    #[tokio::test]
    async fn a_non_success_status_reports_the_status_and_the_body() {
        let addr = serve_once("403 Forbidden", "tenant is not entitled").await;
        let url = format!("http://{addr}/.well-known/openid-configuration");

        let error = load_openid_configuration(&url, &Client::new())
            .await
            .expect_err("a 403 is not a usable discovery document");

        assert!(error.contains("403"), "the status must be named: {error}");
        assert!(
            error.contains("tenant is not entitled"),
            "the body carries the reason and must survive: {error}"
        );
        assert!(
            error.contains(&url),
            "the URL must be named, since several endpoints are configurable: {error}"
        );
    }

    /// A 200 whose body is not the expected document must say what was expected.
    #[tokio::test]
    async fn an_unparseable_document_names_the_expected_fields() {
        let addr = serve_once("200 OK", r#"{"unexpected":"shape"}"#).await;
        let url = format!("http://{addr}/.well-known/openid-configuration");

        let error = load_openid_configuration(&url, &Client::new())
            .await
            .expect_err("a document without token_endpoint is unusable");

        assert!(
            error.contains("issuer") && error.contains("token_endpoint"),
            "the message must name the fields it needed: {error}"
        );
        assert!(error.contains(&url), "and where it looked: {error}");
    }

    /// An unreachable endpoint is a distinct failure from a bad response.
    #[tokio::test]
    async fn an_unreachable_endpoint_is_reported_as_a_transport_failure() {
        // Port 1 on loopback: nothing listens, and connecting fails fast.
        let error = load_openid_configuration(
            "http://127.0.0.1:1/.well-known/openid-configuration",
            &Client::new(),
        )
        .await
        .expect_err("nothing is listening");

        assert!(
            error.contains("Failed to get OpenID configuration"),
            "a connect failure must be distinguishable from a bad document: {error}"
        );
    }
}
