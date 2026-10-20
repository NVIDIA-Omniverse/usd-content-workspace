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
use pin_project_lite::pin_project;
use std::collections::VecDeque;
use std::task::Poll;
use std::time::Duration;
use tokio::time::Sleep;

pin_project! {
    #[derive(Debug)]
    #[must_use = "streams do nothing unless polled"]
    pub struct Slow<T> {
        items: VecDeque<T>,
        sleep: Duration,
        #[pin]
        future: Option<Sleep>,
    }
}

impl<T: Unpin> futures::stream::Stream for Slow<T> {
    type Item = T;

    #[inline]
    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let mut this = self.project();

        // do we have a sleep to execute?
        if let Some(fut) = this.future.as_mut().as_pin_mut() {
            let _: () = futures::ready!(fut.poll(cx));
            // sleep finished, continue
        }

        let item = this.items.pop_front().inspect(|_item| {
            // reload the sleep future
            this.future.set(Some(tokio::time::sleep(*this.sleep)));
        });
        Poll::Ready(item)
    }
}

impl<E> Slow<Result<Bytes, E>>
where
    E: std::fmt::Debug + Send + Sync + Unpin + 'static,
{
    pub fn try_stream<T, I>(sleep: Duration, iter: I) -> Self
    where
        I: IntoIterator<Item = Result<T, E>>,
        T: Into<Bytes>,
    {
        let items = iter.into_iter().map(|res| res.map(Into::into)).collect();
        assert_stream(Self {
            items,
            sleep,
            future: None,
        })
    }
}

impl<E> TestStream<E> for Slow<Result<Bytes, E>>
where
    E: std::fmt::Debug + Send + Sync + Unpin + 'static,
{
    fn len(&self) -> usize {
        self.items
            .iter()
            .map(|item| match item {
                Err(_e) => 0,
                Ok(bytes) => bytes.len(),
            })
            .sum()
    }
}
