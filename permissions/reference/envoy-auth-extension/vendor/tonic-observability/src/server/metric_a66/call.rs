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

use super::attributes::{Attributes, attrs};
use super::metrics::ResponseMetrics;
use super::response_body::TonicMetricResponseBody;
use crate::common::extract::{self, GRPC_STATUS};
use crate::server_verbose as verbose;
use pin_project_lite::pin_project;
use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Instant;
use tonic::body::Body;
use tonic::codegen::http;

pin_project! {
    /// Server metric call handler, returned by the metric service.
    ///
    /// This is exported for type signatures when using a
    /// [`TonicMetricService`](super::service::TonicMetricService) and cannot be
    /// constructed directly.
    #[derive(Debug)]
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    pub struct TonicMetricHandler<Fut> {
        #[pin]
        pub(super) inner: Fut,
        pub(super) attrs: Attributes,
        pub(super) metrics: ResponseMetrics,
        pub(super) call_start: Instant,
    }

    impl<Fut> PinnedDrop for TonicMetricHandler<Fut> {
        #[inline]
        fn drop(me: Pin<&mut Self>) {
            // coverage cannot see into macros
            TonicMetricHandler::pinned_drop(me)
        }
    }
}

impl<Fut> TonicMetricHandler<Fut> {
    fn pinned_drop(self: Pin<&mut Self>) {
        let this = self.project();
        let attributes = attrs!(this.attrs);
        this.metrics.active_handlers.add(-1, attributes);
    }
}

impl<Fut, RespBody> Future for TonicMetricHandler<Fut>
where
    Fut: Future<Output = Result<http::Response<RespBody>, Infallible>>,
    RespBody: http_body::Body<Data = bytes::Bytes, Error = tonic::Status> + Send + 'static,
{
    type Output = Result<http::Response<Body>, Infallible>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        // poll the future
        let outcome = match this.inner.poll(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(outcome) => outcome,
        };

        match outcome {
            Ok(response) => {
                let headers = response.headers();
                // if the `grpc-status` header is present, record it. otherwise,
                // it should be in the trailers.
                let code = headers
                    .get(GRPC_STATUS)
                    .and_then(extract::resp_grpc_status_from_header_value);

                // some metric layers require handling end-of-stream/empty
                // bodies, because they are never polled. this layer doesn't, as
                // metadata is not included in the size.
                if response.body().is_end_stream() {
                    verbose!("metrics response end stream");
                } else {
                    verbose!("metrics response start stream");
                }

                // wrap the response stream
                let response = response.map(|inner| {
                    Body::new(TonicMetricResponseBody {
                        inner,
                        attrs: this.attrs.clone(),
                        metrics: this.metrics.clone(),
                        call_start: *this.call_start,
                        code,
                        deframer: Default::default(),
                    })
                });

                let attributes = attrs!(this.attrs);
                this.metrics.active_responses.add(1, attributes);

                Poll::Ready(Ok(response))
            }
        }
    }
}
