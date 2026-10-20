// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::pin::Pin;
use std::task::{Context, Poll};

use futures::Stream;
use storage_core_util::observability::slow_operation::SlowOperationGuard;

/// A [`Stream`] wrapper that holds a [`SlowOperationGuard`].
///
/// The guard is dropped when the stream completes or is dropped (e.g., client disconnect),
/// so the slow-operation check covers the full lifetime of the stream — not just the initial
/// setup in the handler.
///
/// ```ignore
/// let stream = backend.list(request).await?;
/// let guard = log_if_slow!(guard: "grpc::list").with_context(folder_address);
/// Ok(Response::new(Box::pin(SlowGuardedStream::new(stream, guard))))
/// ```
pub struct SlowGuardedStream<S> {
    inner: Pin<Box<S>>,
    _guard: SlowOperationGuard,
}

impl<S> SlowGuardedStream<S> {
    pub fn new(stream: S, guard: SlowOperationGuard) -> Self {
        Self {
            inner: Box::pin(stream),
            _guard: guard,
        }
    }
}

impl<S: Stream> Stream for SlowGuardedStream<S> {
    type Item = S::Item;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}
