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
use bytes::Bytes;
use std::collections::VecDeque;
use std::task::Poll;

/// Creates a stream which returns items, but also pends before each item.
///
/// The pend is important, as it forces the stream to be polled. Otherwise,
/// streams can be "flattened" into a single response as an optimization.
///
/// This `struct` is created by [`Self::from_iter()`], which means it supports
/// calling `.collect<Many<_>>()` on an iterator.
#[derive(Debug)]
#[repr(transparent)]
#[must_use = "streams do nothing unless polled"]
pub struct Many<T> {
    items: VecDeque<Option<T>>,
}

impl<T: Unpin> Many<T> {
    pub fn new<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let mut items = VecDeque::new();
        for item in iter {
            items.push_back(None);
            items.push_back(Some(item));
        }
        assert_stream(Self { items })
    }

    #[inline]
    pub fn items<'a>(&'a self) -> impl Iterator<Item = &'a T> + use<'a, T> {
        self.items.iter().filter_map(Option::as_ref)
    }
}

impl<T: Unpin> FromIterator<T> for Many<T> {
    #[inline]
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self::new(iter)
    }
}

impl<T: Unpin> futures::stream::Stream for Many<T> {
    type Item = T;

    #[inline]
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        match self.items.pop_front() {
            None => Poll::Ready(None),
            Some(None) => {
                // wake ourselves, so the runtime will call us again
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            Some(v) => Poll::Ready(v),
        }
    }
}

impl<T: Unpin> futures::stream::FusedStream for Many<T> {
    fn is_terminated(&self) -> bool {
        self.items.is_empty()
    }
}

impl<E> Many<Result<Bytes, E>>
where
    E: std::fmt::Debug + Send + Sync + Unpin + 'static,
{
    pub fn try_stream<T, I>(iter: I) -> Self
    where
        I: IntoIterator<Item = Result<T, E>>,
        T: Into<Bytes>,
    {
        let mut items = VecDeque::new();
        for item in iter {
            items.push_back(None);
            items.push_back(Some(item.map(Into::into)));
        }
        assert_stream(Self { items })
    }
}

impl<E> TestStream<E> for Many<Result<Bytes, E>>
where
    E: std::fmt::Debug + Send + Sync + Unpin + 'static,
{
    fn len(&self) -> usize {
        self.items
            .iter()
            .map(|item| match item {
                None => 0,
                Some(Err(_e)) => 0,
                Some(Ok(bytes)) => bytes.len(),
            })
            .sum()
    }
}
