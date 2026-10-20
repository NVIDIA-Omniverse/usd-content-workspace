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
use crate::client_verbose as verbose;
use crate::common::as_u64;
use crate::common::extract::{self, GRPC_STATUS};
use pin_project_lite::pin_project;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Instant;

pin_project! {
    /// A66 client metric response body, returned by the metric attempt.
    ///
    /// This is a wrapper around the original response body.
    #[derive(Debug)]
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    pub struct TonicMetricResponseBody<RespBody> {
        #[pin]
        pub(super) inner: RespBody,
        pub(super) attrs: Attributes,
        pub(super) metrics: ResponseMetrics,
        pub(super) call_start: Instant,
        pub(super) code: Option<u16>,
        pub(super) size: usize,
    }

    impl<RespBody> PinnedDrop for TonicMetricResponseBody<RespBody> {
        #[inline]
        fn drop(me: Pin<&mut Self>) {
            // coverage cannot see into macros
            TonicMetricResponseBody::pinned_drop(me)
        }
    }
}

impl<RespBody> TonicMetricResponseBody<RespBody> {
    fn pinned_drop(self: Pin<&mut Self>) {
        let this = self.project();

        let attributes = attrs!(this.attrs);
        this.metrics.active_responses.add(-1, attributes);

        let code = *this.code;
        let attributes = attrs!(this.attrs, code);

        let value = Instant::now()
            .saturating_duration_since(*this.call_start)
            .as_secs_f64();
        this.metrics.call_duration.record(value, attributes);

        let value = as_u64!(*this.size);
        this.metrics.response_size.record(value, attributes);
    }
}

impl<RespBody> http_body::Body for TonicMetricResponseBody<RespBody>
where
    RespBody: http_body::Body<Data = bytes::Bytes, Error = tonic::Status>,
{
    type Data = bytes::Bytes;
    type Error = tonic::Status;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        let this = self.project();

        let outcome = match this.inner.poll_frame(cx) {
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
                    *this.size = this.size.wrapping_add(size);
                }

                // trailers signify the end of the response
                // https://github.com/grpc/grpc/blob/master/doc/PROTOCOL-HTTP2.md
                if let Some(trailers) = frame.trailers_ref() {
                    let status = trailers.get(GRPC_STATUS);

                    *this.code = status
                        .and_then(extract::resp_grpc_status_from_header_value)
                        .or(*this.code);

                    verbose!("metrics resp body trailer");
                    // metadata is not included in the size
                }
            }
            Some(Err(status)) => {
                verbose!("metrics resp body err: {}", status);
                // metadata is not included in the size
                *this.code = Some(status.code() as _);
            }
            None => {
                // responses shouldn't use end-of-stream (EOS)
                // https://github.com/grpc/grpc/blob/master/doc/PROTOCOL-HTTP2.md
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

#[cfg(test)]
mod tests;
