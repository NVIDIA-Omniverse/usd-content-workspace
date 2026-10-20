// SPDX-FileCopyrightText: Copyright (c) 2024-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
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

use super::response_body::TonicTraceResponseBody;
use super::span;
use crate::client_verbose as verbose;
use crate::common::extract::{GRPC_MESSAGE, GRPC_STATUS};
use pin_project_lite::pin_project;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tonic::codegen::http;
use tracing::Span;

type ServiceError = tonic::transport::Error;

pin_project! {
    /// Tonic client trace response.
    ///
    /// This is exported for type signatures when using a
    /// [`TonicTraceLayer`](super::layer::TonicTraceLayer)/[`TonicTraceService`](super::service::TonicTraceService). A response cannot be
    /// constructed directly.
    ///
    /// # Internals
    ///
    /// This type contains a request future and the corresponding span. When
    /// the future completes, the span is updated based on the response. So it
    /// is basically a wrapper around the future.
    ///
    /// In reality, it may also be necessary to wait for the response body to
    /// complete before updating the span.
    #[derive(Debug)]
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    pub struct TonicTraceResponse<Fut> {
        #[pin]
        pub(super) inner: Fut,
        pub(super) span: Span,
    }
}

impl<Fut, RespBody> Future for TonicTraceResponse<Fut>
where
    Fut: Future<Output = Result<http::Response<RespBody>, ServiceError>>,
    RespBody: http_body::Body<Error = tonic::Status>,
{
    type Output = Result<http::Response<TonicTraceResponseBody<RespBody>>, ServiceError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let _guard = this.span.enter();

        // poll the future
        let outcome = match this.inner.poll(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(outcome) => outcome,
        };

        match outcome {
            Ok(response) => {
                verbose!("trace response ok");

                let mut span_status = span::SpanStatus::new();
                // if the `grpc-status` header is present, record it. otherwise,
                // it should be in the trailers.
                if let Some(status) = response.headers().get(GRPC_STATUS) {
                    let message = response.headers().get(GRPC_MESSAGE);
                    span_status.record_grpc_status(status, message);
                } else {
                    // for a client, it's possible we're not talking to a gRPC
                    // server
                    let status = response.status();
                    if status != http::StatusCode::OK {
                        verbose!("trace response http: {}", status);
                        span_status.record_http_status(status);
                    }
                }

                // wrap the body
                let response = response.map(|body| TonicTraceResponseBody {
                    inner: body,
                    span: this.span.clone(),
                    span_status,
                });
                Poll::Ready(Ok(response))
            }
            Err(e) => {
                verbose!("trace response err: {0}\n{0:#?}", e);
                span::record_transport_err(this.span, &e);
                Poll::Ready(Err(e))
            }
        }
    }
}
