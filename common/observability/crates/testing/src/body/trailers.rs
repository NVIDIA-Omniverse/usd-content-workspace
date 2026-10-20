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

use super::TestBody;
use std::convert::Infallible;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

/// A [`http_body::Body`] that sends trailers.
///
/// This exists, as `axum` and `reqwests` don't let streams return trailers.
#[derive(Debug, Clone)]
#[repr(transparent)]
pub struct TrailersBody<E = Infallible> {
    data: Option<http::HeaderMap>,
    error: PhantomData<E>,
}

impl TrailersBody {
    #[inline]
    pub fn new(trailers: http::HeaderMap) -> Self {
        Self {
            data: Some(trailers),
            error: PhantomData,
        }
    }
}

impl<E> Unpin for TrailersBody<E> {}

impl<E> http_body::Body for TrailersBody<E> {
    type Data = bytes::Bytes;
    type Error = E;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        Poll::Ready(
            self.data
                .take()
                .map(|map| Ok(http_body::Frame::trailers(map))),
        )
    }

    fn is_end_stream(&self) -> bool {
        self.data.is_none()
    }
}

impl<E> TestBody<E> for TrailersBody<E>
where
    E: Unpin + std::fmt::Debug + Send + Sync + 'static,
{
    #[inline]
    fn len(&self) -> usize {
        0
    }
}
