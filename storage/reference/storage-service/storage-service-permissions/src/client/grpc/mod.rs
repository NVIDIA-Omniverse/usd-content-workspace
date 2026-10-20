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

use std::{num::NonZero, str::FromStr, time::Duration};

use bytes::Bytes;
use futures::future::try_join_all;
use http::Uri;
use opentelemetry::propagation::TextMapPropagator;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use serde::Deserialize;
use storage_service_grpc::codec::EncodedRequestCodec;
use storage_service_metrics::{
    future::FutureExt as _,
    utils::{record_metric, record_metric_value},
};
use storage_service_protos::nvidia::omniverse::permission::v1beta::{
    Action, CheckPermissionBatch, CheckPermissionBatchRequest, CheckPermissionBatchResponse,
    Condition as GrpcCondition, Decision as GrpcDecision, Resource,
};
use storage_service_util::{
    error::PrettyPrintErrorExt,
    prost::MessageExt,
    retry::{invoke_with_retry, MaybeTransient, RetryConfig, RetryStrategy},
};
use tonic::{
    metadata::{MetadataKey, MetadataMap, MetadataValue},
    transport::{Channel, Endpoint},
    Code, Request, Response, Status,
};
use tracing::{debug, instrument, trace, Span};
use tracing_opentelemetry::OpenTelemetrySpanExt;

use crate::{
    client::{
        error::{Error, InitError},
        grpc::metrics::Metrics,
        utils::{BatchExt, TryCollectVecExt},
    },
    types::{Condition, DecisionResponse, ResourceAction},
};

mod metrics;

const DEFAULT_DNS_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const DEFAULT_TCP_KEEPALIVE: Duration = Duration::from_secs(30);
const DEFAULT_HTTP2_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);
const DEFAULT_CONNECTIONS_PER_HOST: NonZero<usize> = NonZero::new(4).unwrap();

const SERVICE_NAME: &str = "storage-service";

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub endpoint: String,
    #[serde(default, with = "humantime_serde::option")]
    pub dns_refresh_interval: Option<Duration>,
    #[serde(default, with = "humantime_serde::option")]
    pub tcp_keepalive: Option<Duration>,
    #[serde(default, with = "humantime_serde::option")]
    pub http2_keep_alive_interval: Option<Duration>,
    pub connections_per_host: Option<NonZero<usize>>,
    pub batch_size: Option<NonZero<usize>>,
    #[serde(default)]
    pub retry: RetryConfig,
}

#[derive(Debug)]
pub struct Client {
    channel: Channel,
    metrics: Metrics,
    retry_strategy: RetryStrategy,
}

impl Client {
    pub async fn try_from_config(config: Config) -> Result<Self, InitError> {
        use storage_service_grpc::balancer::Builder;

        let uri = Uri::from_str(&config.endpoint).map_err(InitError::invalid_endpoint_uri)?;

        let http2_keep_alive_interval = config
            .http2_keep_alive_interval
            .unwrap_or(DEFAULT_HTTP2_KEEPALIVE_INTERVAL);
        let tcp_keepalive = config.tcp_keepalive.unwrap_or(DEFAULT_TCP_KEEPALIVE);
        let channel = Builder::new(uri)
            .map_err(InitError::channel_create)?
            .connections_per_host(config.connections_per_host.unwrap_or(DEFAULT_CONNECTIONS_PER_HOST))
            .dns_refresh_interval(config.dns_refresh_interval.unwrap_or(DEFAULT_DNS_REFRESH_INTERVAL))
            .connect(move |endpoint: Endpoint| -> Result<Endpoint, InitError> {
                let endpoint = endpoint
                    .http2_keep_alive_interval(http2_keep_alive_interval)
                    .tcp_keepalive(Some(tcp_keepalive));
                Ok(endpoint)
            })
            .await
            .map_err(InitError::channel_create)?;

        Ok(Self {
            channel,
            metrics: Metrics::new(),
            retry_strategy: RetryStrategy::from_config(config.retry),
        })
    }

    #[instrument(skip_all)]
    pub async fn query(
        &self,
        request_metadata: &MetadataMap,
        items: impl IntoIterator<Item = ResourceAction>,
        condition: Condition,
        batch_size: NonZero<usize>,
    ) -> Result<Vec<DecisionResponse>, Error> {
        let responses = try_join_all(
            items
                .into_iter()
                .map(|item| CheckPermissionBatch {
                    resource: Some(Resource {
                        id: item.resource,
                        r#type: item.resource_type,
                        ..Default::default()
                    }),
                    actions: vec![Action {
                        name: item.action,
                        service: SERVICE_NAME.into(),
                    }],
                    ..Default::default()
                })
                .batches(batch_size)
                .map(|batches| {
                    self.check_permission_batch_with_retry(
                        CheckPermissionBatchRequest {
                            condition: match condition {
                                Condition::None => None,
                                Condition::And => Some(GrpcCondition::And as i32),
                                Condition::Or => Some(GrpcCondition::Or as i32),
                            },
                            batches,
                        },
                        request_metadata,
                    )
                }),
        )
        .await?;

        responses
            .into_iter()
            .flat_map(|response| response.decisions)
            .map(|decision| {
                if let Some(decision) = decision.results.first() {
                    const DECISION_ALLOW: i32 = GrpcDecision::Allow as i32;
                    const DECISION_DENY: i32 = GrpcDecision::Deny as i32;
                    const DECISION_SKIP: i32 = GrpcDecision::Skip as i32;

                    Ok(match (decision.decision, decision.reason.is_none()) {
                        (DECISION_ALLOW, _) => DecisionResponse::Allow,
                        (DECISION_DENY, true) => DecisionResponse::Missing,
                        (DECISION_SKIP, _) => DecisionResponse::Skip,
                        _ => DecisionResponse::Deny,
                    })
                } else {
                    Err(Error::response_decision_missing())
                }
            })
            .try_collect_vec()
    }

    async fn check_permission_batch_with_retry(
        &self,
        request: CheckPermissionBatchRequest,
        request_metadata: &MetadataMap,
    ) -> Result<CheckPermissionBatchResponse, Error> {
        trace!(?request, "encoding a batch permission check request");

        self.metrics
            .check_permission_batch_batch_size
            .record(request.batches.len() as u64);

        let message = request.encode_to_bytes();
        trace!(message_len = ?message.len(), "request message encoded");

        let mut metadata = request_metadata.clone();

        let propagator = TraceContextPropagator::new();
        let current_span = Span::current();
        current_span.set_attribute("otel.kind", "client");
        let current_context = current_span.context();
        propagator.inject_context(&current_context, &mut MetadataMapCarrier(&mut metadata));

        invoke_with_retry(self.retry_strategy.iter_retry_intervals(), move || {
            let response_future = self.send_check_permission_batch_pre_encoded(Request::from_parts(
                metadata.clone(),
                Default::default(),
                message.clone(),
            ));
            async {
                debug!("sending a check permission batch request");
                let maybe_response = response_future
                    .with_latency_metric(&self.metrics.check_permission_batch_latency)
                    .await
                    .inspect(record_metric_value(
                        &self.metrics.check_permission_batch_count,
                        Code::Ok,
                    ))
                    .inspect_err(record_metric(&self.metrics.check_permission_batch_count));

                match maybe_response {
                    Ok(response) => Ok(response.into_inner()),
                    Err(error) if error.is_retryable() => {
                        debug!(error = %error.pretty(), "batch permission check request failed with a transient error");
                        Err(MaybeTransient::Transient(Error::request_failed(error)))
                    },
                    Err(error) if error.is_invalid_principal() => {
                        debug!(error = %error.pretty(), "batch permission check request failed");
                        Err(MaybeTransient::Permanent(Error::invalid_principal()))
                    },
                    Err(error) => {
                        debug!(error = %error.pretty(), "batch permission check request failed");
                        Err(MaybeTransient::Permanent(Error::request_failed(error)))
                    },
                }
            }
        })
        .await
        .inspect(|response| debug!(?response, "batch response received"))
    }

    #[instrument(skip_all)]
    async fn send_check_permission_batch_pre_encoded(
        &self,
        encoded_request: Request<Bytes>,
    ) -> Result<Response<CheckPermissionBatchResponse>, Status> {
        use http::uri::PathAndQuery;
        use tonic::client::Grpc;

        let mut grpc = Grpc::new(self.channel.clone());
        grpc.ready()
            .await
            .map_err(|source| Status::unknown(format!("service was not ready: {:?}", source)))?;

        // No need to insert additional request metadata as we don't declare any client middleware.
        let codec: EncodedRequestCodec<CheckPermissionBatchResponse> = Default::default();
        let path =
            PathAndQuery::from_static("/nvidia.omniverse.permission.v1beta.PermissionService/CheckPermissionBatch");

        grpc.unary(encoded_request, path, codec).await
    }
}

struct MetadataMapCarrier<'a>(&'a mut tonic::metadata::MetadataMap);

impl opentelemetry::propagation::Injector for MetadataMapCarrier<'_> {
    fn set(&mut self, key: &str, value: String) {
        if let (Ok(key), Ok(val)) = (MetadataKey::from_bytes(key.as_bytes()), MetadataValue::try_from(&value)) {
            self.0.insert(key, val);
        }
    }
}

trait StatusExt {
    fn is_retryable(&self) -> bool;
    fn is_invalid_principal(&self) -> bool;
}

impl StatusExt for Status {
    fn is_retryable(&self) -> bool {
        matches!(
            self.code(),
            Code::Unknown | Code::Unavailable | Code::Internal | Code::DeadlineExceeded | Code::ResourceExhausted
        )
    }

    fn is_invalid_principal(&self) -> bool {
        matches!(self.code(), Code::Unauthenticated)
    }
}

#[cfg(test)]
mod tests {
    use std::{assert_matches, collections::VecDeque, sync::Arc};

    use http::{header::AUTHORIZATION, HeaderValue};
    use storage_service_protos::nvidia::omniverse::permission::v1beta::{
        permission_service_server::{PermissionService, PermissionServiceServer},
        *,
    };
    use tokio::{net::TcpSocket, sync::Mutex};
    use tokio_stream::wrappers::TcpListenerStream;
    use tokio_util::sync::CancellationToken;
    use tonic::{transport::Server, Code, Request, Response, Result};

    use crate::types::Condition;

    use super::*;

    const REQUEST_AUTHORIZATION: &str = "Bearer token";
    const RESOURCE_ID: &str = "resource-id";
    const RESOURCE_TYPE: &str = "resource-type";
    const RESOURCE_ACTION: &str = "resource-action";

    #[derive(Clone)]
    struct MockService {
        inner: Arc<MockServiceInner>,
    }

    struct MockServiceInner {
        response_queue: Mutex<VecDeque<Result<Response<CheckPermissionBatchResponse>>>>,
    }

    #[tonic::async_trait]
    impl PermissionService for MockService {
        async fn check_permission(
            &self,
            _request: Request<CheckPermissionRequest>,
        ) -> Result<Response<CheckPermissionResponse>> {
            unimplemented!()
        }

        async fn check_permission_batch(
            &self,
            request: Request<CheckPermissionBatchRequest>,
        ) -> Result<Response<CheckPermissionBatchResponse>> {
            assert_eq!(
                request.metadata().get("authorization").map(|v| v.as_bytes()),
                Some(REQUEST_AUTHORIZATION.as_bytes())
            );

            let request = request.into_inner();
            assert_matches!(request.condition, None);
            assert_eq!(request.batches.len(), 1);
            assert_eq!(request.batches[0].actions.len(), 1);
            assert_matches!(request.batches[0].context, None);
            assert_matches!(request.batches[0].principal, None);
            assert_matches!(
                request.batches[0].resource.as_ref(),
                Some(r) if r.id == RESOURCE_ID && r.r#type == RESOURCE_TYPE && r.data.is_none()
            );
            assert_eq!(request.batches[0].actions[0].name, RESOURCE_ACTION);
            assert_eq!(request.batches[0].actions[0].service, "storage-service");

            self.inner.response_queue.lock().await.pop_front().unwrap()
        }
    }

    #[rstest::rstest]
    #[case(Code::Unknown)]
    #[case(Code::ResourceExhausted)]
    #[case(Code::Unavailable)]
    #[case(Code::DeadlineExceeded)]
    #[case(Code::Internal)]
    #[test_log::test(tokio::test)]
    async fn test_query_retries_transient_errors(#[case] code: Code) {
        let maybe_result = query([
            Err(Status::new(code, "error")),
            Ok(Response::new(CheckPermissionBatchResponse {
                summary: None,
                decisions: vec![ResourceActionDecisionBatch {
                    results: vec![ResourceActionDecision {
                        decision: Decision::Allow as i32,
                        ..Default::default()
                    }],
                }],
            })),
        ])
        .await;

        assert_matches!(maybe_result, Ok(r) if r == vec![DecisionResponse::Allow]);
    }

    #[test_log::test(tokio::test)]
    async fn test_query_returns_authentication_errors() {
        let maybe_result = query([Err(Status::unauthenticated("unauthenticated"))]).await;

        assert_matches!(maybe_result, Err(Error::InvalidPrincipal { .. }));
    }

    #[rstest::rstest]
    #[case(Code::InvalidArgument)]
    #[case(Code::NotFound)]
    #[case(Code::AlreadyExists)]
    #[case(Code::FailedPrecondition)]
    #[case(Code::Aborted)]
    #[case(Code::OutOfRange)]
    #[case(Code::Unimplemented)]
    #[case(Code::DataLoss)]
    #[case(Code::PermissionDenied)]
    #[test_log::test(tokio::test)]
    async fn test_query_returns_permanent_errors(#[case] code: Code) {
        let maybe_result = query([Err(Status::new(code, "error"))]).await;

        assert_matches!(maybe_result, Err(Error::RequestFailed { .. }));
    }

    async fn query(
        responses: impl IntoIterator<Item = Result<Response<CheckPermissionBatchResponse>>>,
    ) -> Result<Vec<DecisionResponse>, Error> {
        let service = MockService {
            inner: Arc::new(MockServiceInner {
                response_queue: Mutex::new(responses.into_iter().collect()),
            }),
        };
        let socket = TcpSocket::new_v4().unwrap();
        socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();

        let listen_addr = socket.local_addr().unwrap();
        let listener = TcpListenerStream::new(socket.listen(1).unwrap());

        let cancel = CancellationToken::new();
        tokio::spawn({
            let cancel = cancel.clone();
            let service = service.clone();
            async move {
                Server::builder()
                    .add_service(PermissionServiceServer::new(service))
                    .serve_with_incoming_shutdown(listener, cancel.cancelled_owned())
                    .await
            }
        });

        let _guard = cancel.drop_guard();

        let client = Client::try_from_config(Config {
            endpoint: format!("http://{}", listen_addr),
            dns_refresh_interval: None,
            tcp_keepalive: None,
            http2_keep_alive_interval: None,
            connections_per_host: None,
            batch_size: None,
            retry: Default::default(),
        })
        .await
        .unwrap();

        client
            .query(
                &MetadataMap::from_headers(
                    [(AUTHORIZATION, HeaderValue::from_static(REQUEST_AUTHORIZATION))]
                        .into_iter()
                        .collect(),
                ),
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
}
