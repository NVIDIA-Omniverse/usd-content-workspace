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

use super::{TestStream, assert_stream};
use std::task::Poll;

/// Creates a stream which returns a single item.
///
/// This `struct` is created by [`once()`].
#[derive(Debug)]
#[repr(transparent)]
#[must_use = "streams do nothing unless polled"]
pub struct Once<T> {
    item: Option<T>,
}

impl<T> Once<T> {
    #[inline]
    pub fn items<'a>(&'a self) -> impl Iterator<Item = &'a T> + use<'a, T> {
        self.item.iter()
    }
}

impl<T: Unpin> futures::stream::Stream for Once<T> {
    type Item = T;

    #[inline]
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let item = self.item.take();
        Poll::Ready(item)
    }
}

impl<T: Unpin> futures::stream::FusedStream for Once<T> {
    fn is_terminated(&self) -> bool {
        self.item.is_none()
    }
}

/// Creates a stream which returns a single item.
pub fn once<T: Unpin>(v: T) -> Once<T> {
    assert_stream(Once { item: Some(v) })
}

impl<E> Once<Result<bytes::Bytes, E>>
where
    E: std::fmt::Debug + Send + Sync + Unpin + 'static,
{
    pub fn try_stream<T>(v: Result<T, E>) -> Self
    where
        T: Into<bytes::Bytes>,
    {
        let item = Some(v.map(Into::into));
        assert_stream(Self { item })
    }
}

impl<E> TestStream<E> for Once<Result<bytes::Bytes, E>>
where
    E: std::fmt::Debug + Send + Sync + Unpin + 'static,
{
    fn len(&self) -> usize {
        match &self.item {
            None => 0,
            Some(Err(_e)) => 0,
            Some(Ok(bytes)) => bytes.len(),
        }
    }
}
