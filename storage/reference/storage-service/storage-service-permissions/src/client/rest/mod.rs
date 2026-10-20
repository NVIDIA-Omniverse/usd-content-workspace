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

use std::num::NonZero;

use bytes::{BufMut, BytesMut};
use futures::future::try_join_all;
use http::{header::CONTENT_TYPE, HeaderMap, HeaderValue, StatusCode};
use opentelemetry::propagation::TextMapPropagator;
use opentelemetry_http::HeaderInjector;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use serde::Deserialize;
use storage_service_metrics::{future::FutureExt as _, utils::record_metric};
use storage_service_util::{
    error::PrettyPrintErrorExt as _,
    retry::{invoke_with_retry, MaybeTransient, RetryConfig, RetryStrategy},
};
use tracing::{debug, info_span, instrument, trace, Instrument, Span};
use tracing_opentelemetry::OpenTelemetrySpanExt;

pub mod models;

mod metrics;

use crate::{
    client::{
        error::Error,
        rest::metrics::Metrics,
        utils::{BatchExt, TryCollectVecExt},
    },
    types::{Condition, DecisionResponse, ResourceAction},
};

const SERVICE_NAME: &str = "storage-service";

const CONTENT_TYPE_APPLICATION_JSON: HeaderValue = HeaderValue::from_static("application/json");

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub endpoint: String,
    pub batch_size: Option<NonZero<usize>>,
    #[serde(default)]
    pub retry: RetryConfig,
}

#[derive(Debug)]
pub struct Client {
    client: reqwest::Client,
    endpoint: String,
    metrics: Metrics,
    retry_strategy: RetryStrategy,
}

impl Client {
    pub fn from_config(config: Config) -> Self {
        Self {
            client: Default::default(),
            endpoint: config.endpoint,
            metrics: Metrics::new(),
            retry_strategy: RetryStrategy::from_config(config.retry),
        }
    }

    #[instrument(skip_all)]
    pub async fn query(
        &self,
        request_headers: &HeaderMap,
        items: impl IntoIterator<Item = ResourceAction>,
        condition: Condition,
        batch_size: NonZero<usize>,
    ) -> Result<Vec<DecisionResponse>, Error> {
        let responses = try_join_all(
            items
                .into_iter()
                .map(|item| models::Request {
                    resource: models::Resource {
                        id: item.resource,
                        resource_type: item.resource_type,
                    },
                    actions: vec![models::Action {
                        name: item.action,
                        service: SERVICE_NAME.into(),
                    }],
                })
                .batches(batch_size)
                .map(|batches| {
                    self.check_permission_batch(
                        models::BatchRequest {
                            condition: match condition {
                                Condition::None => None,
                                Condition::And => Some(models::Condition::And),
                                Condition::Or => Some(models::Condition::Or),
                            },
                            batches,
                        },
                        request_headers,
                    )
                }),
        )
        .await?;

        responses
            .into_iter()
            .flat_map(|response| response.decisions.into_iter())
            .map(|decisions| {
                if let Some(decision) = decisions.into_values().next() {
                    use models::Decision::*;

                    Ok(match (decision.decision, decision.reason.is_none()) {
                        (Allow, _) => DecisionResponse::Allow,
                        (Deny, true) => DecisionResponse::Missing,
                        (Skip, _) => DecisionResponse::Skip,
                        _ => DecisionResponse::Deny,
                    })
                } else {
                    Err(Error::response_decision_missing())
                }
            })
            .try_collect_vec()
    }

    async fn check_permission_batch(
        &self,
        request: models::BatchRequest,
        request_headers: &HeaderMap,
    ) -> Result<models::BatchResponse, Error> {
        trace!(?request, "encoding a batch permission check request");

        let mut buffer = BytesMut::new().writer();
        serde_json::to_writer(&mut buffer, &request).map_err(Error::request_encode_failed)?;

        let request_body = buffer.into_inner().freeze();

        trace!(body_len = ?request_body.len(), "request body encoded");

        self.metrics
            .check_permission_batch_batch_size
            .record(request.batches.len() as u64);

        let propagator = TraceContextPropagator::new();
        let current_span = Span::current();
        current_span.set_attribute("otel.kind", "client");

        let current_context = current_span.context();
        let mut tracing_headers = HeaderMap::with_capacity(2);
        propagator.inject_context(&current_context, &mut HeaderInjector(&mut tracing_headers));

        invoke_with_retry(self.retry_strategy.iter_retry_intervals(), move || {
                let request_builder = self
                    .client
                    .post(format!("{}/v1beta/authorization/batch/", self.endpoint))
                    .headers(request_headers.clone())
                    .headers(tracing_headers.clone())
                    .header(CONTENT_TYPE, CONTENT_TYPE_APPLICATION_JSON)
                    .body(request_body.clone());

                debug!("sending a batch permission check request");
                let response_future = request_builder.send();

                async move {
                    let maybe_response = response_future
                        .instrument(info_span!("rest::check_permission_batch"))
                        .with_latency_metric(&self.metrics.check_permission_batch_latency)
                        .await
                        .inspect(record_metric(&self.metrics.check_permission_batch_count));
                    match maybe_response {
                        Ok(response) => {
                            let status = response.status();
                            debug!(?status, "batch permission check response headers received");

                            if status == StatusCode::OK {
                                response
                                    .json()
                                    .await
                                    .inspect_err(
                                        |error| debug!(error = %error.pretty(), "batch permission check response body read failed"),
                                    )
                                    .map_err(Error::response_decode_failed)
                                    .map_err(MaybeTransient::Transient)
                            } else if status == StatusCode::UNAUTHORIZED {
                                debug!(?status, "batch permission check request failed");
                                Err(MaybeTransient::Permanent(Error::invalid_principal()))
                            } else if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
                                debug!(?status, "batch permission check request failed with a transient error");
                                Err(MaybeTransient::Transient(Error::unexpected_response_status(status)))
                            } else {
                                debug!(?status, "batch permission check request failed");
                                Err(MaybeTransient::Permanent(Error::unexpected_response_status(status)))
                            }
                        },
                        Err(error) => Err(MaybeTransient::Transient(Error::request_failed(error))),
                    }
                }
            })
            .await
            .inspect(|response| debug!(?response, "batch response received"))
    }
}

#[cfg(test)]
mod tests {
    use std::{
        assert_matches,
        collections::{HashMap, VecDeque},
        sync::Arc,
    };

    use axum::{extract::State, http::HeaderMap, routing::post, Json, Router};
    use bytes::Bytes;
    use http::{header::AUTHORIZATION, StatusCode};
    use tokio::{net::TcpListener, sync::Mutex};
    use tokio_util::sync::CancellationToken;

    use super::*;

    use crate::types::DecisionResponse;

    const REQUEST_AUTHORIZATION: &str = "Bearer token";
    const RESOURCE_ID: &str = "resource-id";
    const RESOURCE_TYPE: &str = "resource-type";
    const RESOURCE_ACTION: &str = "resource-action";

    type Responses = Arc<Mutex<VecDeque<(StatusCode, Bytes)>>>;

    #[test_log::test(tokio::test)]
    async fn test_query_returns_authentication_errors() {
        let maybe_result = query([response(StatusCode::UNAUTHORIZED)]).await;

        assert_matches!(maybe_result, Err(Error::InvalidPrincipal { .. }));
    }

    #[rstest::rstest]
    #[case(StatusCode::TOO_MANY_REQUESTS)]
    #[case(StatusCode::INTERNAL_SERVER_ERROR)]
    #[test_log::test(tokio::test)]
    async fn test_query_retries_transient_errors(#[case] code: StatusCode) {
        let maybe_result = query([response(code), allow_response()]).await;

        assert_matches!(maybe_result, Ok(r) if r == vec![DecisionResponse::Allow]);
    }

    #[rstest::rstest]
    #[case(StatusCode::BAD_REQUEST)]
    #[case(StatusCode::PRECONDITION_FAILED)]
    #[test_log::test(tokio::test)]
    async fn test_query_returns_permanent_errors(#[case] code: StatusCode) {
        let maybe_result = query([response(code)]).await;

        assert_matches!(maybe_result, Err(Error::UnexpectedResponseStatus { status_code, .. }) if status_code == code);
    }

    #[test_log::test(tokio::test)]
    async fn test_query_retries_on_invalid_json() {
        let maybe_result = query([(StatusCode::OK, Bytes::from_static(b"invalid json")), allow_response()]).await;

        assert_matches!(maybe_result, Ok(r) if r == vec![DecisionResponse::Allow]);
    }

    async fn query(responses: impl IntoIterator<Item = (StatusCode, Bytes)>) -> Result<Vec<DecisionResponse>, Error> {
        let responses: Responses = Arc::new(Mutex::new(responses.into_iter().collect()));
        let router = Router::new()
            .route("/v1beta/authorization/batch/", post(handle_query))
            .with_state(responses);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());

        let cancel = CancellationToken::new();
        tokio::spawn({
            let cancel = cancel.clone();
            async move {
                cancel
                    .run_until_cancelled(async move {
                        axum::serve(listener, router).await.unwrap();
                    })
                    .await;
            }
        });
        let _guard = cancel.drop_guard();

        let client = Client::from_config(Config {
            endpoint,
            batch_size: None,
            retry: Default::default(),
        });
        client
            .query(
                &[(AUTHORIZATION, HeaderValue::from_static(REQUEST_AUTHORIZATION))]
                    .into_iter()
                    .collect(),
                [ResourceAction {
                    resource: RESOURCE_ID.into(),
                    resource_type: RESOURCE_TYPE.into(),
                    action: RESOURCE_ACTION.into(),
                }],
                Condition::None,
                NonZero::new(1).unwrap(),
            )
            .await
    }

    async fn handle_query(
        State(responses): State<Responses>,
        headers: HeaderMap,
        Json(request): Json<models::BatchRequest>,
    ) -> (StatusCode, Bytes) {
        assert_eq!(headers.get(AUTHORIZATION).unwrap(), REQUEST_AUTHORIZATION);
        assert_eq!(headers.get(CONTENT_TYPE).unwrap(), CONTENT_TYPE_APPLICATION_JSON);
        assert_matches!(request.condition, None);
        assert_eq!(request.batches.len(), 1);
        assert_eq!(request.batches[0].resource.id, RESOURCE_ID);
        assert_eq!(request.batches[0].resource.resource_type, RESOURCE_TYPE);
        assert_eq!(request.batches[0].actions.len(), 1);
        assert_eq!(request.batches[0].actions[0].name, RESOURCE_ACTION);
        assert_eq!(request.batches[0].actions[0].service, SERVICE_NAME);

        responses.lock().await.pop_front().unwrap()
    }

    fn response(status: StatusCode) -> (StatusCode, Bytes) {
        (status, Bytes::new())
    }

    fn allow_response() -> (StatusCode, Bytes) {
        let response = models::BatchResponse {
            decisions: vec![HashMap::from([(
                RESOURCE_ACTION.into(),
                models::ActionDecision {
                    decision: models::Decision::Allow,
                    reason: None,
                    action: RESOURCE_ACTION.into(),
                },
            )])],
        };
        (StatusCode::OK, Bytes::from(serde_json::to_vec(&response).unwrap()))
    }
}
