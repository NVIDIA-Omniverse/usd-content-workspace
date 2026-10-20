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

use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};

use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::http::{self, HttpResponseError, Retryable};
use crate::metrics;

#[derive(Debug, Clone, Serialize)]
pub struct Resource {
    pub id: String,
    #[serde(rename = "type")]
    pub resource_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl Hash for Resource {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
        self.resource_type.hash(state);
    }
}

impl Resource {
    pub fn user(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            resource_type: "User".to_owned(),
            data: None,
        }
    }

    pub fn group(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            resource_type: "Group".to_owned(),
            data: None,
        }
    }
}

#[derive(Debug, Serialize)]
struct AuthorizationPayload {
    action: Action,
    #[serde(skip_serializing_if = "Option::is_none")]
    resource: Option<Resource>,
}

#[derive(Debug, Serialize)]
struct AuthorizationBatchRequest {
    batches: Vec<AuthorizationBatch>,
}

#[derive(Debug, Serialize)]
struct AuthorizationBatch {
    actions: Vec<Action>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resource: Option<Resource>,
}

#[derive(Debug, Deserialize)]
struct AuthorizationBatchResponse {
    decisions: Vec<HashMap<String, AuthorizationResponse>>,
}

#[derive(Debug, Clone, Serialize)]
struct Action {
    name: String,
    service: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AuthorizationResponse {
    pub decision: AuthorizationDecision,
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum AuthorizationDecision {
    Deny,
    Allow,
    Skip,
}

/// HTTP client for the Omniverse Permission Service authorization API.
pub struct PermissionClient {
    http: Client,
    base_url: String,
    service_name: String,
}

impl PermissionClient {
    pub fn new(base_url: String, service_name: String) -> Self {
        Self {
            http: Client::new(),
            base_url: base_url.trim_end_matches('/').to_owned(),
            service_name,
        }
    }

    #[cfg(test)]
    pub fn with_base_url(base_url: &str, service_name: String) -> Self {
        Self::new(base_url.to_owned(), service_name)
    }

    fn action(&self, name: &str) -> Action {
        Action {
            name: name.to_owned(),
            service: self.service_name.clone(),
        }
    }

    #[instrument(skip(self, bearer_token, extra_headers))]
    pub async fn check_authorization(
        &self,
        bearer_token: &str,
        action_name: &str,
        resource: Option<Resource>,
        extra_headers: reqwest::header::HeaderMap,
    ) -> Result<AuthorizationResponse, PermissionError> {
        let start = std::time::Instant::now();
        let url = format!("{}/v1beta/authorization/", self.base_url);
        let payload = AuthorizationPayload {
            action: self.action(action_name),
            resource,
        };

        let mut headers = extra_headers;
        crate::telemetry::inject_trace_context(&mut headers);
        let result = async {
            let resp = self
                .http
                .post(&url)
                .headers(headers)
                .bearer_auth(bearer_token)
                .json(&payload)
                .send()
                .await
                .map_err(PermissionError::Http)?;

            let resp = http::classify_response(resp).await?;
            resp.json().await.map_err(PermissionError::Http)
        }
        .await;
        let status = permission_status_label(&result);
        metrics::record_permission_request(status, start.elapsed().as_secs_f64());
        result
    }

    #[instrument(skip(self, bearer_token, resources, extra_headers))]
    pub async fn check_authorization_batch(
        &self,
        bearer_token: &str,
        action_name: &str,
        resources: Vec<Resource>,
        extra_headers: reqwest::header::HeaderMap,
    ) -> Result<Vec<AuthorizationResponse>, PermissionError> {
        let start = std::time::Instant::now();
        let url = format!("{}/v1beta/authorization/batch/", self.base_url);
        let action = self.action(action_name);
        let batches = resources
            .into_iter()
            .map(|r| AuthorizationBatch {
                actions: vec![action.clone()],
                resource: Some(r),
            })
            .collect();

        let mut headers = extra_headers;
        crate::telemetry::inject_trace_context(&mut headers);
        let result = async {
            let resp = self
                .http
                .post(&url)
                .headers(headers)
                .bearer_auth(bearer_token)
                .json(&AuthorizationBatchRequest { batches })
                .send()
                .await
                .map_err(PermissionError::Http)?;

            let resp = http::classify_response(resp).await?;
            let batch_resp: AuthorizationBatchResponse =
                resp.json().await.map_err(PermissionError::Http)?;

            let deny = AuthorizationResponse {
                decision: AuthorizationDecision::Deny,
                reason: None,
            };
            Ok(batch_resp
                .decisions
                .into_iter()
                .map(|map| map.into_values().next().unwrap_or(deny.clone()))
                .collect())
        }
        .await;
        let status = permission_status_label(&result);
        metrics::record_permission_request(status, start.elapsed().as_secs_f64());
        result
    }
}

#[derive(Debug)]
pub enum PermissionError {
    Http(reqwest::Error),
    Unauthorized(String),
    RateLimited { retry_after: u64 },
    ServiceError { status: u16, body: String },
}

impl From<HttpResponseError> for PermissionError {
    fn from(e: HttpResponseError) -> Self {
        match e {
            HttpResponseError::RateLimited { retry_after } => Self::RateLimited { retry_after },
            HttpResponseError::Api { status: 401, body } => Self::Unauthorized(body),
            HttpResponseError::Api { status, body } => Self::ServiceError { status, body },
        }
    }
}

impl Retryable for PermissionError {
    fn is_retryable(&self) -> bool {
        match self {
            Self::RateLimited { .. } => true,
            Self::ServiceError { status, .. } => matches!(status, 503 | 504),
            _ => false,
        }
    }

    fn retry_after(&self) -> Option<u64> {
        match self {
            Self::RateLimited { retry_after } => Some(*retry_after),
            _ => None,
        }
    }
}

impl fmt::Display for PermissionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http(e) => write!(f, "HTTP error: {e}"),
            Self::Unauthorized(body) => write!(f, "unauthorized: {body}"),
            Self::RateLimited { retry_after } => {
                write!(f, "rate limited (retry after {retry_after}s)")
            }
            Self::ServiceError { status, body } => {
                write!(f, "permission service {status}: {body}")
            }
        }
    }
}

impl std::error::Error for PermissionError {}

fn permission_status_label<T>(result: &Result<T, PermissionError>) -> &'static str {
    match result {
        Ok(_) => "ok",
        Err(PermissionError::Unauthorized(_)) => "401",
        Err(PermissionError::RateLimited { .. }) => "429",
        Err(PermissionError::ServiceError { .. }) => "error",
        Err(PermissionError::Http(_)) => "network_error",
    }
}
