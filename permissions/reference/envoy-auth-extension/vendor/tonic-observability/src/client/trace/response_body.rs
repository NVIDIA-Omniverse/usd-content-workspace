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

use super::span;
use crate::client_verbose as verbose;
use crate::common::extract::{GRPC_MESSAGE, GRPC_STATUS};
use pin_project_lite::pin_project;
use std::pin::Pin;
use std::task::{Context, Poll};
use tracing::Span;

pin_project! {
    /// Tonic client trace response body.
    ///
    /// This is a wrapper around the original response body.
    #[derive(Debug)]
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    pub struct TonicTraceResponseBody<RespBody> {
        #[pin]
        pub(super) inner: RespBody,
        pub(super) span: Span,
        pub(super) span_status: span::SpanStatus,
    }

    impl<RespBody> PinnedDrop for TonicTraceResponseBody<RespBody> {
        #[inline]
        fn drop(me: Pin<&mut Self>) {
            let this = me.project();
            // coverage cannot see into macros
            this.span_status.finalize_span(this.span);
        }
    }
}

impl<RespBody> http_body::Body for TonicTraceResponseBody<RespBody>
where
    RespBody: http_body::Body<Error = tonic::Status>,
{
    type Data = RespBody::Data;
    type Error = tonic::Status;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        let this = self.project();
        let _enter = this.span.enter();

        let outcome = match this.inner.poll_frame(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(outcome) => outcome,
        };

        match &outcome {
            Some(Ok(frame)) => {
                if frame.is_data() {
                    verbose!("trace resp body data");
                }

                // trailers signify the end of the response
                // https://github.com/grpc/grpc/blob/master/doc/PROTOCOL-HTTP2.md
                if let Some(trailers) = frame.trailers_ref() {
                    verbose!("trace resp body trailers");
                    if let Some(status) = trailers.get(GRPC_STATUS) {
                        let message = trailers.get(GRPC_MESSAGE);
                        this.span_status.record_grpc_status(status, message);
                    }
                }
            }
            Some(Err(status)) => {
                verbose!("trace resp body err: {}", status);
                this.span_status.record_tonic_status(status);
            }
            None => {
                // responses shouldn't use end-of-stream (EOS)
                // https://github.com/grpc/grpc/blob/master/doc/PROTOCOL-HTTP2.md
                verbose!("trace resp body eos");
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
