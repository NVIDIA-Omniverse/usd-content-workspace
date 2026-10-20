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
use super::metrics::RequestMetrics;
use crate::client_verbose as verbose;
use crate::common::as_u64;
use pin_project_lite::pin_project;
use std::pin::Pin;
use std::task::{Context, Poll};

pin_project! {
    /// Client metric request body, returned by the metric service.
    ///
    /// This is a wrapper around the original request body.
    #[derive(Debug)]
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    pub struct TonicMetricRequestBody<ReqBody> {
        #[pin]
        pub(super) inner: ReqBody,
        pub(super) attrs: Attributes,
        pub(super) metrics: RequestMetrics,
        pub(super) size: usize,
    }

    impl<ReqBody> PinnedDrop for TonicMetricRequestBody<ReqBody> {
        #[inline]
        fn drop(me: Pin<&mut Self>) {
            // coverage cannot see into macros
            TonicMetricRequestBody::pinned_drop(me)
        }
    }
}

impl<ReqBody> TonicMetricRequestBody<ReqBody> {
    fn pinned_drop(self: Pin<&mut Self>) {
        let this = self.project();
        let attributes = attrs!(this.attrs);
        this.metrics.active_requests.add(-1, attributes);
        let value = as_u64!(*this.size);
        this.metrics.request_size.record(value, attributes);
    }
}

impl<ReqBody> http_body::Body for TonicMetricRequestBody<ReqBody>
where
    ReqBody: http_body::Body<Data = bytes::Bytes, Error = tonic::Status>,
{
    type Data = bytes::Bytes;
    type Error = tonic::Status;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        let this = self.project();

        // poll the future
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
                    verbose!("metrics req body data ({})", size);
                    *this.size = this.size.wrapping_add(size);
                } // cov-exclude [branch]: gRPC requests may not have trailers

                // gRPC requests may not have trailers, instead EOS is used
                // https://github.com/grpc/grpc/blob/master/doc/PROTOCOL-HTTP2.md
                if frame.is_trailers() {
                    verbose!("metrics req body trailers (!!!)"); // cov-exclude [line]: see above
                }
            }
            Some(Err(status)) => {
                // we cannot know whether any data was sent; don't record it
                verbose!("metrics req body err: {}", status);
                let _ = status;
            }
            None => {
                // end-of-stream (EOS)
                verbose!("metrics req body eos");
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
