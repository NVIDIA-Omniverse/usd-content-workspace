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

use super::attributes::Attributes;
use super::metrics::ResponseMetrics;
use super::response_body::TowerMetricResponseBody;
use crate::server_verbose as verbose;
use bytes::Bytes;
use pin_project_lite::pin_project;
use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Instant;

pin_project! {
    /// Server metric call handler, returned by the metric service.
    ///
    /// This wraps the original handler to instrument responses.
    ///
    /// This is exported for type signatures when using a
    /// [`TowerMetricService`](super::service::TowerMetricService) and cannot be
    /// constructed directly.
    #[derive(Debug)]
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    pub struct TowerMetricHandler<Fut> {
        #[pin]
        pub(super) inner: Fut,
        pub(super) attrs: Attributes,
        pub(super) start: Instant,
        pub(super) metrics: ResponseMetrics,
    }
}

impl<Fut, RespBody> Future for TowerMetricHandler<Fut>
where
    Fut: Future<Output = Result<http::Response<RespBody>, Infallible>>,
    RespBody: http_body::Body<Data = Bytes>,
    RespBody::Error: std::error::Error + 'static,
{
    type Output = Result<http::Response<TowerMetricResponseBody<RespBody>>, Infallible>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        // poll the future
        let outcome = match this.inner.poll(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(outcome) => outcome,
        };

        match outcome {
            Ok(response) => {
                verbose!("metrics response start stream");

                let status = response.status();

                // wrap the response stream
                let response = response.map(|inner| TowerMetricResponseBody {
                    inner,
                    attrs: this.attrs.clone(),
                    metrics: this.metrics.clone(),
                    start: *this.start,
                    status,
                    error_type: None,
                    size: 0,
                });

                Poll::Ready(Ok(response))
            }
        }
    }
}
