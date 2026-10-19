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

use std::sync::Arc;

use httpmock::{
    Method::{GET, POST},
    Mock, MockServer, When,
};
use serde_json::json;
use tracing::info;

/// Mock credentials provider service.
#[derive(Clone)]
pub struct Service {
    inner: Arc<MockServer>,
}

impl Default for Service {
    fn default() -> Self {
        Self::new()
    }
}

impl Service {
    /// Create a service.
    pub fn new() -> Self {
        let server = MockServer::start();
        info!("Credentials provider mock is listening on {}", server.address());

        server.mock(|when, then| {
            when.method(GET).path("/config");
            then.status(200).json_body(json!({
                "token_endpoint": server.url("/token"),
            }));
        });

        Self {
            inner: Arc::new(server),
        }
    }

    /// Return the token endpoint address.
    pub fn token_endpoint(&self) -> String {
        self.inner.url("/token")
    }

    /// Return the configuration endpoint address.
    pub fn config_endpoint(&self) -> String {
        self.inner.url("/config")
    }

    /// Create a token request mock.
    pub fn mock_token_request<'a>(&'a self) -> MockTokenRequest<'a> {
        MockTokenRequest {
            mock: &self.inner,
            client_id: None,
            client_secret: None,
            scope: None,
        }
    }
}

pub struct MockTokenRequest<'a> {
    mock: &'a MockServer,
    client_id: Option<String>,
    client_secret: Option<String>,
    scope: Option<String>,
}

impl<'a> MockTokenRequest<'a> {
    pub fn client_credentials(
        self,
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
        scope: impl Into<String>,
    ) -> Self {
        Self {
            client_id: Some(client_id.into()),
            client_secret: Some(client_secret.into()),
            scope: Some(scope.into()),
            ..self
        }
    }

    pub fn reply_with_bearer_token(self, access_token: impl Into<String>) -> Mock<'a> {
        self.reply_with_token("Bearer", access_token)
    }

    pub fn reply_with_token(self, token_type: impl Into<String>, access_token: impl Into<String>) -> Mock<'a> {
        self.reply_with_body(
            200,
            "application/json",
            serde_json::to_vec(&json!({
                "token_type": token_type.into(),
                "access_token": access_token.into(),
            }))
            .expect("failed to encode response body"),
        )
    }

    pub fn reply_with_body(self, status: u16, content_type: impl Into<String>, content: impl AsRef<[u8]>) -> Mock<'a> {
        self.mock.mock(move |when, then| {
            when.method(POST)
                .path("/token")
                .header("content-type", "application/x-www-form-urlencoded")
                .maybe_x_www_form_urlencoded_tuple("client_id", self.client_id)
                .maybe_x_www_form_urlencoded_tuple("client_secret", self.client_secret)
                .maybe_x_www_form_urlencoded_tuple("scope", self.scope)
                .x_www_form_urlencoded_tuple("grant_type", "client_credentials");

            then.status(status)
                .header("content-type", content_type.into())
                .body(content);
        })
    }
}

trait WhenExt {
    fn maybe_x_www_form_urlencoded_tuple(self, key: impl Into<String>, value: Option<impl Into<String>>) -> Self;
}

impl WhenExt for When {
    fn maybe_x_www_form_urlencoded_tuple(self, key: impl Into<String>, value: Option<impl Into<String>>) -> Self {
        if let Some(value) = value {
            self.x_www_form_urlencoded_tuple(key, value)
        } else {
            self
        }
    }
}
