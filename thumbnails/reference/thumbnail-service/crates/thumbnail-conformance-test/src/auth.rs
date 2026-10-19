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

//! Resolves a bearer token for the run, supporting the three documented
//! authentication modes — direct token, access-token file, and OAuth2
//! client-credentials via OpenID Connect Discovery — plus an unauthenticated
//! fallback. Resolved once at startup and reused for every request; this
//! binary runs one bounded suite and exits, so token refresh mid-run is not a
//! concern the way it is for the long-lived `crates/auth` client.

use serde::Deserialize;

use crate::style::TestError;
use crate::Args;

#[derive(Debug, Deserialize)]
struct OpenIdConfiguration {
    token_endpoint: String,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
}

#[derive(Debug, Deserialize)]
struct AccessTokenFileContents {
    access_token: String,
}

/// Resolve the bearer token to use for every request this run, or `None` for
/// an unauthenticated run.
///
/// Validates that at most one auth mode is configured, and that a
/// partially-configured OAuth2 client-credentials setup is rejected loudly
/// rather than silently falling through to unauthenticated.
pub async fn resolve_token(args: &Args) -> Result<Option<String>, TestError> {
    let has_bearer = non_empty(args.bearer_token.as_deref()).is_some();
    let has_access_token_file = non_empty(args.access_token_file.as_deref()).is_some();

    let oauth2_any = non_empty(args.oauth2_openid_configuration_uri.as_deref()).is_some()
        || non_empty(args.oauth2_client_id.as_deref()).is_some()
        || non_empty(args.oauth2_client_secret.as_deref()).is_some()
        || non_empty(args.oauth2_client_secret_file.as_deref()).is_some();
    let oauth2_complete = non_empty(args.oauth2_openid_configuration_uri.as_deref()).is_some()
        && non_empty(args.oauth2_client_id.as_deref()).is_some()
        && (non_empty(args.oauth2_client_secret.as_deref()).is_some()
            ^ non_empty(args.oauth2_client_secret_file.as_deref()).is_some());

    let modes_set = [has_bearer, has_access_token_file, oauth2_any]
        .iter()
        .filter(|set| **set)
        .count();
    if modes_set > 1 {
        return Err(TestError::new(
            "multiple authentication modes configured; set at most one of \
             BEARER_TOKEN, ACCESS_TOKEN_FILE, or the OAUTH2_* client-credentials flags",
        ));
    }

    if has_bearer {
        return Ok(Some(
            non_empty(args.bearer_token.as_deref()).unwrap().to_string(),
        ));
    }

    if has_access_token_file {
        let path = non_empty(args.access_token_file.as_deref()).unwrap();
        return Ok(Some(read_access_token_file(path)?));
    }

    if oauth2_any {
        if !oauth2_complete {
            return Err(TestError::new(
                "partial OAuth2 client-credentials configuration: \
                 OAUTH2_OPENID_CONFIGURATION_URI and OAUTH2_CLIENT_ID are required, \
                 along with exactly one of OAUTH2_CLIENT_SECRET or OAUTH2_CLIENT_SECRET_FILE",
            ));
        }
        return Ok(Some(fetch_client_credentials_token(args).await?));
    }

    Ok(None)
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|v| !v.is_empty())
}

fn read_access_token_file(path: &str) -> Result<String, TestError> {
    let contents = std::fs::read_to_string(path)
        .map_err(|e| TestError::new(format!("failed to read ACCESS_TOKEN_FILE at {path}: {e}")))?;
    let trimmed = contents.trim();
    if trimmed.is_empty() {
        return Err(TestError::new(format!(
            "ACCESS_TOKEN_FILE at {path} is empty"
        )));
    }

    // Sidecar token providers write a JSON envelope; fall back to the raw
    // file contents as the token, matching thumbnail-smoke-test.
    if trimmed.starts_with('{') {
        let parsed: AccessTokenFileContents = serde_json::from_str(trimmed).map_err(|e| {
            TestError::new(format!(
                "ACCESS_TOKEN_FILE at {path} contains invalid JSON: {e}"
            ))
        })?;
        let token = parsed.access_token.trim();
        if token.is_empty() {
            return Err(TestError::new(format!(
                "ACCESS_TOKEN_FILE at {path} is JSON but access_token is empty"
            )));
        }
        return Ok(token.to_string());
    }

    Ok(trimmed.to_string())
}

async fn fetch_client_credentials_token(args: &Args) -> Result<String, TestError> {
    let discovery_uri = args
        .oauth2_openid_configuration_uri
        .as_deref()
        .filter(|u| !u.trim().is_empty())
        .ok_or_else(|| {
            TestError::new(
                "OAUTH2_OPENID_CONFIGURATION_URI is required for the client-credentials flow",
            )
        })?;
    let client_id = args
        .oauth2_client_id
        .as_deref()
        .filter(|c| !c.trim().is_empty())
        .ok_or_else(|| {
            TestError::new("OAUTH2_CLIENT_ID is required for the client-credentials flow")
        })?;
    let client_secret = resolve_client_secret(args)?;
    let scope = args.oauth2_client_scope.as_deref().unwrap_or_default();

    let client = reqwest::Client::builder()
        .timeout(args.http_timeout)
        .build()
        .map_err(|e| TestError::new(format!("failed to build HTTP client: {e}")))?;

    let discovery: OpenIdConfiguration = client
        .get(discovery_uri)
        .send()
        .await
        .map_err(|e| TestError::new(format!("GET {discovery_uri} failed: {e}")))?
        .error_for_status()
        .map_err(|e| TestError::new(format!("GET {discovery_uri} returned an error: {e}")))?
        .json()
        .await
        .map_err(|e| {
            TestError::new(format!(
                "GET {discovery_uri}: failed to parse OpenID configuration: {e}"
            ))
        })?;

    let mut form = vec![
        ("grant_type", "client_credentials"),
        ("client_id", client_id),
        ("client_secret", client_secret.as_str()),
    ];
    if !scope.is_empty() {
        form.push(("scope", scope));
    }

    let response = client
        .post(&discovery.token_endpoint)
        .form(&form)
        .send()
        .await
        .map_err(|e| TestError::new(format!("POST {} failed: {e}", discovery.token_endpoint)))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(TestError::new(format!(
            "token endpoint {} returned {status}: {body}",
            discovery.token_endpoint
        )));
    }

    let token: TokenResponse = response
        .json()
        .await
        .map_err(|e| TestError::new(format!("failed to parse token response: {e}")))?;

    let token = token.access_token.trim();
    if token.is_empty() {
        return Err(TestError::new(
            "token endpoint returned an empty access_token",
        ));
    }

    // Trimmed, not returned verbatim. This checked `trim().is_empty()` and then
    // returned the untrimmed value, so a token endpoint answering with
    // surrounding whitespace produced `Authorization: Bearer   tok  ` — the same
    // whitespace defect the access-token-file path guards against, one function
    // away. Raised by Mitri on !220.
    Ok(token.to_string())
}

fn resolve_client_secret(args: &Args) -> Result<String, TestError> {
    if let Some(secret) = args
        .oauth2_client_secret
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Ok(secret.to_string());
    }

    if let Some(path) = args
        .oauth2_client_secret_file
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        let contents = std::fs::read_to_string(path).map_err(|e| {
            TestError::new(format!(
                "failed to read OAUTH2_CLIENT_SECRET_FILE at {path}: {e}"
            ))
        })?;
        let trimmed = contents.trim();
        if trimmed.is_empty() {
            return Err(TestError::new(format!(
                "OAUTH2_CLIENT_SECRET_FILE at {path} is empty"
            )));
        }
        return Ok(trimmed.to_string());
    }

    Err(TestError::new(
        "OAUTH2_CLIENT_ID is set but neither OAUTH2_CLIENT_SECRET nor OAUTH2_CLIENT_SECRET_FILE was provided",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// One named way of mutating `Args`, so a table of cases reads as a table.
    /// Aliased because `&[(&str, fn(&mut Args))]` trips `clippy::type_complexity`.
    type Case = (&'static str, fn(&mut Args));

    /// An `Args` with no authentication configured.
    ///
    /// A struct literal, which reads no environment. An earlier version of this
    /// module claimed `Args` could not be built in a test — that it declares
    /// `env = "..."` on every field, so parsing one reads the process
    /// environment and a test would pass or fail on whether `BEARER_TOKEN`
    /// happened to be exported — and left `resolve_token` untested on that
    /// basis. The attribute only affects `Args::parse`. Every field is `pub`, so
    /// a literal has no ambient input and no ordering dependence. Raised by
    /// Mitri on !220: the justification was wrong, and it was shielding the
    /// branchiest logic in the crate.
    fn args() -> Args {
        Args {
            rest_url: Some("http://localhost:3000".to_string()),
            grpc_url: Some("http://localhost:50051".to_string()),
            grpc_tls_enabled: false,
            test_suite: crate::config::TestSuite::All,
            source_file: "https://storage.test/source.png".to_string(),
            bearer_token: None,
            access_token_file: None,
            oauth2_openid_configuration_uri: None,
            oauth2_client_id: None,
            oauth2_client_secret: None,
            oauth2_client_secret_file: None,
            oauth2_client_scope: None,
            ready_path: "/health".to_string(),
            http_timeout: std::time::Duration::from_secs(30),
            wait_retries: 10,
            wait_interval: std::time::Duration::from_secs(5),
        }
    }

    /// Every case below stops at validation or at a file read, so none of them
    /// awaits anything — only the client-credentials path makes a request, and
    /// no case here reaches one. A current-thread runtime rather than a new
    /// dependency, and rather than `#[tokio::test]` on each case, which would
    /// spin a runtime to run code that never yields.
    fn resolve(args: &Args) -> Result<Option<String>, TestError> {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("test runtime must build")
            .block_on(resolve_token(args))
    }

    /// A path no other test in this binary will use.
    fn unique_path() -> std::path::PathBuf {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        std::env::temp_dir().join(format!(
            "conformance-token-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ))
    }

    fn temp_file(contents: &str) -> std::path::PathBuf {
        let path = unique_path();
        std::fs::write(&path, contents).expect("temp file must be writable");
        path
    }

    #[test]
    fn a_json_envelope_yields_the_access_token_field() {
        let path = temp_file(r#"{"access_token":"tok-from-json","expires_in":3599}"#);
        let token = read_access_token_file(path.to_str().unwrap()).expect("valid envelope");
        assert_eq!(token, "tok-from-json");
        let _ = std::fs::remove_file(path);
    }

    /// The sidecar writes JSON, but the smoke test accepts a bare token and this
    /// suite matches it, so a file that is not JSON is the token itself.
    #[test]
    fn raw_contents_are_taken_as_the_token_and_trimmed() {
        let path = temp_file("  tok-raw\n");
        let token = read_access_token_file(path.to_str().unwrap()).expect("raw token");
        assert_eq!(token, "tok-raw");
        let _ = std::fs::remove_file(path);
    }

    /// Each of these would otherwise become an `Authorization: Bearer ` header
    /// with nothing after it, and the failure would surface as every
    /// authenticated case returning 401 — a deployment-looking symptom with a
    /// configuration cause.
    #[test]
    fn an_unusable_token_file_is_an_error_rather_than_an_empty_token() {
        for (label, contents) in [
            ("empty file", ""),
            ("whitespace only", "   \n  "),
            (
                "json with an empty access_token",
                r#"{"access_token":"  "}"#,
            ),
            ("json missing access_token", r#"{"expires_in":3599}"#),
            ("malformed json", "{not json"),
        ] {
            let path = temp_file(contents);
            let result = read_access_token_file(path.to_str().unwrap());
            assert!(result.is_err(), "{label} must not yield a token");
            let _ = std::fs::remove_file(path);
        }
    }

    /// The path is unique and never created. A fixed basename in the shared
    /// temp directory would make this fail whenever such a file happened to
    /// exist — from a previous run, another user, or anything else — so the
    /// negative case needs the same uniqueness the positive ones get.
    #[test]
    fn no_credentials_resolves_to_an_unauthenticated_run() {
        assert_eq!(resolve(&args()).expect("no modes set is valid"), None);
    }

    #[test]
    fn a_direct_bearer_token_is_used_verbatim_after_trimming() {
        let mut a = args();
        a.bearer_token = Some("  tok-direct  ".to_string());
        assert_eq!(resolve(&a).unwrap(), Some("tok-direct".to_string()));
    }

    /// The rule that makes an unauthenticated run silent.
    ///
    /// `non_empty` trims and discards, so a blank `--bearer-token` is not an
    /// error — it resolves to `None` and every request goes out with no
    /// `Authorization` header. Against a deployment with authentication enabled
    /// that surfaces as every authenticated case returning `401`: a
    /// deployment-shaped symptom with a configuration cause. This is the same
    /// whitespace-as-unset class the `read_access_token_file` cases below cover,
    /// and it was untested in the sibling path.
    #[test]
    fn a_blank_bearer_token_silently_becomes_an_unauthenticated_run() {
        for blank in ["", "   ", "\t\n"] {
            let mut a = args();
            a.bearer_token = Some(blank.to_string());
            assert_eq!(
                resolve(&a).expect("a blank token is not treated as an error"),
                None,
                "{blank:?} should resolve to no credentials"
            );
        }
    }

    /// At most one mode may be set, so a run cannot silently prefer one
    /// credential over another the caller also supplied.
    #[test]
    fn configuring_more_than_one_mode_is_rejected() {
        let combinations: &[Case] = &[
            ("bearer + access-token file", |a| {
                a.bearer_token = Some("tok".to_string());
                a.access_token_file = Some("/tmp/whatever".to_string());
            }),
            ("bearer + oauth2", |a| {
                a.bearer_token = Some("tok".to_string());
                a.oauth2_client_id = Some("client".to_string());
            }),
            ("access-token file + oauth2", |a| {
                a.access_token_file = Some("/tmp/whatever".to_string());
                a.oauth2_client_id = Some("client".to_string());
            }),
        ];

        for (label, configure) in combinations {
            let mut a = args();
            configure(&mut a);
            let error = resolve(&a).expect_err(label);
            assert!(
                format!("{error}").contains("multiple authentication modes"),
                "{label}: the error must say which rule was broken, got: {error}"
            );
        }
    }

    /// A half-configured OAuth2 setup is an error, not a fall-through to an
    /// unauthenticated run — which is the failure that would look like a
    /// deployment rejecting a valid client.
    #[test]
    fn a_partial_oauth2_configuration_is_rejected_rather_than_ignored() {
        let partials: &[Case] = &[
            ("client id only", |a| {
                a.oauth2_client_id = Some("client".to_string());
            }),
            ("discovery uri only", |a| {
                a.oauth2_openid_configuration_uri =
                    Some("https://idp.test/.well-known".to_string());
            }),
            ("id and uri, no secret", |a| {
                a.oauth2_client_id = Some("client".to_string());
                a.oauth2_openid_configuration_uri =
                    Some("https://idp.test/.well-known".to_string());
            }),
            ("both secret forms at once", |a| {
                a.oauth2_client_id = Some("client".to_string());
                a.oauth2_openid_configuration_uri =
                    Some("https://idp.test/.well-known".to_string());
                a.oauth2_client_secret = Some("s".to_string());
                a.oauth2_client_secret_file = Some("/tmp/whatever".to_string());
            }),
        ];

        for (label, configure) in partials {
            let mut a = args();
            configure(&mut a);
            let error = resolve(&a).expect_err(label);
            assert!(
                format!("{error}").contains("partial OAuth2"),
                "{label}: expected the partial-configuration error, got: {error}"
            );
        }
    }

    /// The access-token file wins over nothing else being set, and is read at
    /// resolve time rather than deferred.
    #[test]
    fn the_access_token_file_mode_reads_the_file_it_is_given() {
        let path = temp_file(r#"{"access_token":"tok-from-mode"}"#);
        let mut a = args();
        a.access_token_file = Some(path.to_string_lossy().to_string());
        assert_eq!(resolve(&a).unwrap(), Some("tok-from-mode".to_string()));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_missing_token_file_names_the_path_it_could_not_read() {
        let missing = unique_path();
        let error = read_access_token_file(missing.to_str().unwrap())
            .expect_err("a missing file cannot yield a token");
        assert!(
            format!("{error}").contains(missing.to_str().unwrap()),
            "the error must name the path so a misconfigured mount is diagnosable: {error}"
        );
    }
}
