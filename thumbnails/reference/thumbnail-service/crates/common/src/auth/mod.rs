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

//! Thin shim over the workspace `thumbnail-auth` crate.
//!
//! All the heavy lifting (CLI args, token exchange, gRPC interceptors,
//! refresh loops) lives in `thumbnail-auth`. This module re-exports the
//! types thumbnail-common's call sites use and provides the small bearer-
//! header helpers that don't belong in `thumbnail-auth` itself.

pub mod token_gate;

pub use thumbnail_auth::{
    exchange_client_credentials_token, load_openid_configuration, parse_access_token_file,
    read_token_file, AccessTokenFileArgs, AccessTokenFileInterceptor, AccessTokenFileProvider,
    AccessTokenFileToken, AuthInterceptor, ClientCredentialsArgs, ClientCredentialsInterceptor,
    OpenIdConfigResponse, ServiceIdentityError, Token, TokenManager, TokenRefresher, TokenResponse,
};
pub use token_gate::{TokenGate, TOKEN_EXPIRY_CHECK_INTERVAL, TOKEN_READY_TIMEOUT};

use thiserror::Error;
use tracing::{debug, error};

/// An operation that requires a caller token was reached without one.
#[derive(Debug, Error, PartialEq, Eq)]
#[error("Authentication token is required for {operation}")]
pub struct MissingAuthToken {
    pub operation: String,
}

pub fn validate_auth_token_required<'a>(
    auth_token: Option<&'a str>,
    operation: &str,
) -> Result<&'a str, MissingAuthToken> {
    match auth_token {
        Some(token) if !token.trim().is_empty() => {
            debug!("[AUTH] Token present for {}", operation);
            Ok(token)
        }
        Some(_) | None => {
            error!("[AUTH] No auth token provided for {}", operation);
            Err(MissingAuthToken {
                operation: operation.to_string(),
            })
        }
    }
}

pub fn add_optional_bearer_auth(
    request_builder: reqwest::RequestBuilder,
    auth_token: Option<&str>,
    operation: &str,
) -> reqwest::RequestBuilder {
    match auth_token {
        Some(token) => {
            debug!("[AUTH] Adding Authorization header for {}", operation);
            request_builder.header("Authorization", format!("Bearer {}", token))
        }
        None => {
            debug!("[AUTH] No auth token for {} (optional)", operation);
            request_builder
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_auth_rejects_missing_and_blank_tokens() {
        for token in [None, Some(""), Some(" \t\n")] {
            assert_eq!(
                validate_auth_token_required(token, "test operation"),
                Err(MissingAuthToken {
                    operation: "test operation".to_string(),
                })
            );
        }
    }

    #[test]
    fn required_auth_preserves_a_non_blank_token() {
        assert_eq!(
            validate_auth_token_required(Some(" token "), "test operation"),
            Ok(" token ")
        );
    }
}
