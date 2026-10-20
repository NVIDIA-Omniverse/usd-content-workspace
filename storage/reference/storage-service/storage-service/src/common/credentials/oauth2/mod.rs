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

use reqwest::{
    header::{HeaderMap, HeaderName, HeaderValue, CONTENT_TYPE},
    Client, StatusCode,
};
use serde::Deserialize;
use snafu::IntoError;
use std::time::{Duration, Instant};
use storage_service_metrics::future::FutureExt as _;
use storage_service_metrics::utils::{record_metric, record_metric_value};
use tokio::sync::Mutex;
use tonic::async_trait;
use tracing::{debug, info_span, instrument, Instrument};

use crate::common::{
    credentials::{
        self,
        oauth2::{
            error::{
                Error, InitError, InvalidHeaderNameSnafu, InvalidHeaderValueSnafu,
                OpenIdConfigEndpointUnexpectedStatusSnafu, OpenIdConfigRequestFailedSnafu,
                OpenIdConfigResponseReadFailedSnafu, TokenEndpointRequestBuildFailedSnafu,
                TokenEndpointRequestFailedSnafu, TokenEndpointResponseReadFailedSnafu,
                TokenEndpointUnexpectedStatusSnafu, UnexpectedTokenTypeSnafu,
            },
            metrics::Metrics,
        },
        Credentials,
    },
    iterator::TryCollectExt,
    secret::Secret,
};

pub mod error;

mod metrics;

const DEFAULT_TOKEN_EXPIRES_IN_SECONDS: u64 = 3600;

const TOKEN_EXPIRATION_REDUCTION_SECONDS: u64 = 30;
const TOKEN_TYPE_BEARER: &str = "bearer";

/// A parameter to pass to the token endpoint.
#[derive(Debug, Deserialize, Clone)]
pub struct Parameter {
    pub name: String,
    pub value: Secret<String>,
}

/// Credentials provider configuration.
#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub openid_configuration_uri: String,
    pub parameters: Option<Vec<Parameter>>,
    pub headers: Option<Vec<Parameter>>,
}

/// OAuth2 credentials provider.
#[derive(Debug)]
pub struct CredentialsProvider {
    client: Client,
    token_endpoint_url: String,
    token_request_params: Vec<(String, String)>,
    token_request_headers: Vec<(HeaderName, HeaderValue)>,
    cached_credentials: Mutex<Option<CachedCredentials>>,
    metrics: Metrics,
}

impl CredentialsProvider {
    /// Create a credentials provider from a configuration.
    pub async fn try_from_config(name: impl Into<String>, config: Config) -> Result<Self, InitError> {
        let client = Client::new();
        let token_endpoint = resolve_token_endpoint_url(&client, &config.openid_configuration_uri).await?;

        Self::new(
            name,
            client,
            token_endpoint,
            config
                .parameters
                .unwrap_or_default()
                .into_iter()
                .map(|p| (p.name, p.value.into_inner())),
            config
                .headers
                .unwrap_or_default()
                .into_iter()
                .map(|p| (p.name, p.value.into_inner())),
        )
    }

    /// Create a credentials provider.
    pub fn new<HN, HV>(
        name: impl Into<String>,
        client: Client,
        endpoint: impl Into<String>,
        params: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
        headers: impl IntoIterator<Item = (HN, HV)>,
    ) -> Result<Self, InitError>
    where
        HN: TryInto<HeaderName>,
        HV: TryInto<HeaderValue>,
        HN::Error: std::error::Error + Send + Sync + 'static,
        HV::Error: std::error::Error + Send + Sync + 'static,
    {
        let params = params
            .into_iter()
            .map(|(name, value)| (name.into(), value.into()))
            .chain(
                [("grant_type", "client_credentials")]
                    .into_iter()
                    .map(|(key, value)| (key.into(), value.into())),
            )
            .collect();
        let headers = headers
            .into_iter()
            .map(|(name, value)| {
                Ok((
                    name.try_into()
                        .map_err(|source| InvalidHeaderNameSnafu.into_error(Box::new(source)))?,
                    value
                        .try_into()
                        .map_err(|source| InvalidHeaderValueSnafu.into_error(Box::new(source)))?,
                ))
            })
            .chain([Ok((
                CONTENT_TYPE,
                HeaderValue::from_static("application/x-www-form-urlencoded"),
            ))])
            .try_collect()?;

        Ok(Self {
            token_endpoint_url: endpoint.into(),
            token_request_params: params,
            token_request_headers: headers,
            cached_credentials: Mutex::new(None),
            metrics: Metrics::new(name),
            client,
        })
    }

    /// Retrieve credentials through cache.
    #[instrument(skip_all)]
    async fn get_credentials_through_cache(&self) -> Result<Credentials, Error> {
        let mut cached_credentials = self.cached_credentials.lock().await;
        if let Some(credentials) = cached_credentials.as_ref() {
            if credentials.expires_at > Instant::now() {
                debug!("using cached client credentials");

                return Ok(credentials.credentials.clone());
            } else {
                debug!("cached client credentials have expired");
            }
        } else {
            debug!("no cached client credentials found");
        }

        let (credentials, expires_in) = self.fetch_credentials().await?;

        *cached_credentials = Some(CachedCredentials {
            credentials: credentials.clone(),
            expires_at: Instant::now()
                + Duration::from_secs(
                    expires_in.unwrap_or(DEFAULT_TOKEN_EXPIRES_IN_SECONDS) - TOKEN_EXPIRATION_REDUCTION_SECONDS,
                ),
        });

        Ok(credentials)
    }

    /// Fetch new credentials from the token endpoint.
    async fn fetch_credentials(&self) -> Result<(Credentials, Option<u64>), Error> {
        let response = self
            .client
            .post(&self.token_endpoint_url)
            .headers(self.get_request_headers())
            .body(self.get_request_body()?)
            .send()
            .with_latency_metric(&self.metrics.get_token_latency)
            .instrument(info_span!("oauth2_client::send_request"))
            .await
            .inspect_err(record_metric_value(&self.metrics.get_token_results, 0u16))
            .inspect(record_metric(&self.metrics.get_token_results))
            .map_err(|source| TokenEndpointRequestFailedSnafu.into_error(source))?;

        let status_code = response.status();
        if status_code != StatusCode::OK {
            return TokenEndpointUnexpectedStatusSnafu { status_code }.fail();
        }

        let access_token_response: AccessTokenResponse = response
            .json()
            .instrument(info_span!("oauth2_client::read_response"))
            .await
            .map_err(|source| TokenEndpointResponseReadFailedSnafu.into_error(source))?;

        let token_type = access_token_response.token_type;
        if !token_type.eq_ignore_ascii_case(TOKEN_TYPE_BEARER) {
            return UnexpectedTokenTypeSnafu { token_type }.fail();
        }

        Ok((
            Credentials::Bearer(access_token_response.access_token),
            access_token_response.expires_in,
        ))
    }

    /// Get fetch token request headers.
    fn get_request_headers(&self) -> HeaderMap<HeaderValue> {
        let mut headers = HeaderMap::<HeaderValue>::with_capacity(self.token_request_headers.len() + 1);
        for (key, value) in self.token_request_headers.iter() {
            headers.append(key, value.clone());
        }

        headers
    }

    /// Get token fetch request body.
    fn get_request_body(&self) -> Result<String, Error> {
        serde_urlencoded::to_string(&self.token_request_params)
            .map_err(|source| TokenEndpointRequestBuildFailedSnafu.into_error(source))
    }
}

#[async_trait]
impl credentials::CredentialsProvider for CredentialsProvider {
    async fn get_credentials(&self) -> credentials::Result<Credentials> {
        self.get_credentials_through_cache()
            .await
            .map_err(|error| Box::new(error) as credentials::Error)
    }
}

/// Credentials with expiration.
#[derive(Debug, Clone)]
struct CachedCredentials {
    pub credentials: Credentials,
    pub expires_at: Instant,
}

/// Access token fetch response.
#[derive(Deserialize)]
struct AccessTokenResponse {
    access_token: String,
    token_type: String,
    expires_in: Option<u64>,
}

/// OpenID provider configuration response.
#[derive(Deserialize, Debug)]
struct ProviderConfigurationResponse {
    pub token_endpoint: String,
}

/// Resolve the token endpoint url from the openid configuration url.
async fn resolve_token_endpoint_url(client: &Client, openid_config_uri: &str) -> Result<String, InitError> {
    debug!(?openid_config_uri, "resolving openid configuration");

    let response = client
        .get(openid_config_uri)
        .send()
        .await
        .map_err(|source| OpenIdConfigRequestFailedSnafu.into_error(source))?;
    let status_code = response.status();
    if status_code != StatusCode::OK {
        return OpenIdConfigEndpointUnexpectedStatusSnafu { status_code }.fail();
    }

    let response: ProviderConfigurationResponse = response
        .json()
        .await
        .map_err(|source| OpenIdConfigResponseReadFailedSnafu.into_error(source))?;

    debug!(token_endpoint = ?response.token_endpoint, "token endpoint resolved");

    Ok(response.token_endpoint)
}

#[cfg(test)]
mod tests {
    use assert_matches::assert_matches;

    use storage_service_test::mocks::credentials_provider::Service;

    use crate::common::credentials::{oauth2::CredentialsProvider, Credentials, CredentialsProvider as _};

    #[tokio::test]
    async fn test_oauth2_client_credentials_provider_fetches_bearer_token() {
        let expected_token = "some-token";
        let mock_service = Service::new();
        mock_service
            .mock_token_request()
            .client_credentials("some-client-id", "some-client-secret", "some-scope")
            .reply_with_bearer_token(expected_token);

        let provider = create_credentials_provider(
            mock_service.token_endpoint(),
            "some-client-id",
            "some-client-secret",
            "some-scope",
        );

        let credentials = provider
            .get_credentials()
            .await
            .expect("failed to fetch client credentials");
        assert_matches!(credentials, Credentials::Bearer(token) if token == expected_token);
    }

    #[tokio::test]
    async fn test_oauth2_client_credentials_provider_errors_on_unexpected_token_type() {
        let mock_service = Service::new();
        mock_service
            .mock_token_request()
            .reply_with_token("Anonymous", "missing");

        let provider = create_credentials_provider(
            mock_service.token_endpoint(),
            "some-client-id",
            "some-client-secret",
            "some-scope",
        );

        provider
            .get_credentials()
            .await
            .expect_err("get credentials call is expected to fail");
    }

    #[tokio::test]
    async fn test_oauth2_client_credentials_provider_errors_on_unexpected_status() {
        let mock_service = Service::new();
        mock_service
            .mock_token_request()
            .reply_with_body(400, "text/plain", "invalid request parameters");

        let provider = create_credentials_provider(
            mock_service.token_endpoint(),
            "some-client-id",
            "some-client-secret",
            "some-scope",
        );

        provider
            .get_credentials()
            .await
            .expect_err("get credentials call is expected to fail");
    }

    #[tokio::test]
    async fn test_oauth2_client_credentials_provider_caches_token() {
        let expected_token = "some-token";
        let mock_service = Service::new();
        let fetch_mock = mock_service
            .mock_token_request()
            .client_credentials("some-client-id", "some-client-secret", "some-scope")
            .reply_with_bearer_token(expected_token);

        let provider = create_credentials_provider(
            mock_service.token_endpoint(),
            "some-client-id",
            "some-client-secret",
            "some-scope",
        );

        let credentials_1 = provider
            .get_credentials()
            .await
            .expect("failed to fetch client credentials");
        assert_matches!(credentials_1, Credentials::Bearer(token) if token == expected_token);

        fetch_mock.delete_async().await;

        let credentials_2 = provider
            .get_credentials()
            .await
            .expect("failed to fetch client credentials");
        assert_matches!(credentials_2, Credentials::Bearer(token) if token == expected_token);
    }

    fn create_credentials_provider(
        token_endpoint: impl Into<String>,
        client_id: &str,
        client_secret: &str,
        scope: &str,
    ) -> CredentialsProvider {
        CredentialsProvider::new(
            "test",
            Default::default(),
            token_endpoint,
            [
                ("client_id", client_id),
                ("client_secret", client_secret),
                ("scope", scope),
            ],
            [] as [(String, String); 0],
        )
        .unwrap()
    }
}
