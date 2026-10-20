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
use bytes::Bytes;
use std::convert::Infallible;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

/// A [`http_body::Body`] that causes an end-of-stream "frame".
#[derive(Debug, Clone)]
#[repr(transparent)]
pub struct EndOfStreamBody<E = Infallible> {
    data: Option<Bytes>,
    error: PhantomData<E>,
}

impl EndOfStreamBody {
    #[inline]
    pub fn new(data: impl Into<Bytes>) -> Self {
        Self {
            data: Some(data.into()),
            error: PhantomData,
        }
    }
}

impl<E> Unpin for EndOfStreamBody<E> {}

impl<E> http_body::Body for EndOfStreamBody<E> {
    type Data = bytes::Bytes;
    type Error = E;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        Poll::Ready(self.data.take().map(|buf| Ok(http_body::Frame::data(buf))))
    }

    #[inline]
    fn is_end_stream(&self) -> bool {
        // by returning false here, the body must be polled until it returns
        // `Poll(None)`. this in turn causes end-of-stream behavior.
        false
    }
}

impl<E> TestBody<E> for EndOfStreamBody<E>
where
    E: Unpin + std::fmt::Debug + Send + Sync + 'static,
{
    #[inline]
    fn len(&self) -> usize {
        self.data.iter().map(Bytes::len).sum()
    }
}
