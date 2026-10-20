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
use std::pin::Pin;
use std::task::{Context, Poll};

/// A [`http_body::Body`] that returns an error value when `poll_frame` is
/// called.
#[derive(Debug, Clone)]
#[repr(transparent)]
pub struct ErrorBody<E>(Option<E>);

impl<E: Unpin> ErrorBody<E> {
    #[inline]
    pub fn new(e: E) -> Self {
        Self(Some(e))
    }
}

impl<E: Unpin> http_body::Body for ErrorBody<E> {
    type Data = bytes::Bytes;
    type Error = E;

    #[inline]
    fn poll_frame(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        Poll::Ready(self.0.take().map(Err))
    }

    #[inline]
    fn is_end_stream(&self) -> bool {
        self.0.is_none()
    }
}

impl<E> TestBody<E> for ErrorBody<E>
where
    E: Unpin + std::fmt::Debug + Send + Sync + 'static,
{
    #[inline]
    fn len(&self) -> usize {
        0
    }
}
