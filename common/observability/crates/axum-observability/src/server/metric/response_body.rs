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
use crate::common::as_u64;
use crate::server_verbose as verbose;
use bytes::Bytes;
use pin_project_lite::pin_project;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Instant;

pin_project! {
    /// Server metric response body, returned by the metric call handler.
    ///
    /// This wraps the original response body to instrument it.
    ///
    /// This is exported for type signatures when using a
    /// [`TowerMetricService`](super::service::TowerMetricService) and cannot be
    /// constructed directly.
    #[derive(Debug)]
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    pub struct TowerMetricResponseBody<RespBody> {
        #[pin]
        pub(super) inner: RespBody,
        pub(super) attrs: Attributes,
        pub(super) metrics: ResponseMetrics,
        pub(super) start: Instant,
        pub(super) status: http::StatusCode,
        pub(super) error_type: Option<&'static str>,
        pub(super) size: usize,
    }

    impl<RespBody> PinnedDrop for TowerMetricResponseBody<RespBody> {
        #[inline]
        fn drop(me: Pin<&mut Self>) {
            // coverage cannot see into macros
            TowerMetricResponseBody::pinned_drop(me)
        }
    }
}

impl<RespBody> TowerMetricResponseBody<RespBody> {
    fn pinned_drop(self: Pin<&mut Self>) {
        let this = self.project();

        // body streaming errors take precedence; otherwise, 5xx status codes
        // are reported as the status code string
        let error_type = this.error_type.or_else(|| {
            if this.status.is_server_error() {
                Some(this.status.as_str())
            } else {
                None
            }
        });

        let attributes = attrs!(this.attrs, *this.status, error_type);

        let value = Instant::now()
            .saturating_duration_since(*this.start)
            .as_secs_f64();
        this.metrics.request_duration.record(value, attributes);

        let value = as_u64!(*this.size);
        this.metrics.response_size.record(value, attributes);
    }
}

impl<RespBody> http_body::Body for TowerMetricResponseBody<RespBody>
where
    RespBody: http_body::Body<Data = Bytes>,
    RespBody::Error: std::error::Error + 'static,
{
    type Data = Bytes;
    type Error = RespBody::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        let mut this = self.project();

        let outcome = match this.inner.as_mut().poll_frame(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(outcome) => outcome,
        };

        match &outcome {
            Some(Ok(frame)) => {
                // note: currently, a frame can be either data or trailers,
                // but not both. and there's no way to match exhaustively.

                if let Some(data) = frame.data_ref() {
                    let size = data.len();
                    verbose!("metrics resp body data ({})", size);
                    *this.size = this.size.saturating_add(size);
                }

                if frame.is_trailers() {
                    // just like headers, trailers aren't counted
                    verbose!("metrics resp body trailers");
                }
            }
            Some(Err(e)) => {
                // an error here causes `RST_STREAM` frame to be emitted on
                // the HTTP2 connection
                verbose!("metrics resp body err: {}", e);
                let _ = e;
                *this.error_type = Some(std::any::type_name::<RespBody::Error>());
            }
            None => {
                verbose!("metrics resp body eos");
            }
        }

        Poll::Ready(outcome)
    }

    #[inline]
    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    #[inline]
    fn size_hint(&self) -> http_body::SizeHint {
        self.inner.size_hint()
    }
}
