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
use crate::server_verbose as verbose;
use bytes::Bytes;
use pin_project_lite::pin_project;
use std::pin::Pin;
use std::task::{Context, Poll};
use tracing::Span;

pin_project! {
    /// Tower server trace response body.
    ///
    /// This is a wrapper around the original response body.
    #[derive(Debug)]
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    pub struct TowerTraceResponseBody<RespBody> {
        #[pin]
        pub(super) inner: RespBody,
        pub(super) span: Span,
        pub(super) span_status: span::SpanStatus,
    }

    impl<RespBody> PinnedDrop for TowerTraceResponseBody<RespBody> {
        #[inline]
        fn drop(me: Pin<&mut Self>) {
            let this = me.project();
            // coverage cannot see into macros
            this.span_status.finalize_span(this.span);
        }
    }
}

impl<RespBody> http_body::Body for TowerTraceResponseBody<RespBody>
where
    RespBody: http_body::Body<Data = Bytes>,
    RespBody::Error: std::error::Error + 'static,
{
    type Data = RespBody::Data;
    type Error = RespBody::Error;

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
                // note: currently, a frame can be either data or trailers,
                // but not both. and there's no way to match exhaustively.

                if frame.is_data() {
                    // data is some kind of `Buf`, which is interesting to
                    // metrics but not for traces
                    verbose!("trace resp body data");
                }

                if frame.is_trailers() {
                    // trailers is just a `HeaderMap`, so i don't think there's
                    // anything to record
                    verbose!("trace resp body trailers");
                }
            }
            Some(Err(e)) => {
                // this is a "streaming" error, i.e. the error is returned after
                // a response has been started
                verbose!("trace resp body err: {}", e);
                this.span_status.record_body_error(e);
            }
            None => {
                // end-of-stream (EOS)
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
