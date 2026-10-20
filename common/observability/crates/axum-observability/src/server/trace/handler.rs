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

use super::response_body::TowerTraceResponseBody;
use super::span;
use crate::server_verbose as verbose;
use bytes::Bytes;
use pin_project_lite::pin_project;
use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tracing::Span;

pin_project! {
    /// Tower server trace response.
    ///
    /// This is exported for type signatures when using a
    /// [`TowerTraceLayer`](super::layer::TowerTraceLayer)/[`TowerTraceService`](super::service::TowerTraceService).
    /// A response cannot be constructed directly.
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
    pub struct TowerTraceResponse<Fut> {
        #[pin]
        pub(super) inner: Fut,
        pub(super) span: Span,
    }
}

impl<Fut, RespBody> Future for TowerTraceResponse<Fut>
where
    Fut: Future<Output = Result<http::Response<RespBody>, Infallible>>,
    RespBody: http_body::Body<Data = Bytes>,
    RespBody::Error: std::error::Error + 'static,
{
    type Output = Result<http::Response<TowerTraceResponseBody<RespBody>>, Infallible>;

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
                span::record_http_status(this.span, response.status());
                let span_status = span::SpanStatus::new(response.status());

                // wrap the body, as it may be a streaming response
                let response = response.map(|body| TowerTraceResponseBody {
                    inner: body,
                    span: this.span.clone(),
                    span_status,
                });
                Poll::Ready(Ok(response))
            }
        }
    }
}
