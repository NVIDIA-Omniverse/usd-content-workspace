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

use http_body::Body;
use std::marker::PhantomData;

/// A [`http_body::Body`] that should never be polled.
#[derive(Debug, Clone, Copy)]
#[repr(transparent)]
pub struct BogusBody<E> {
    error: PhantomData<E>,
}

impl<E> BogusBody<E> {
    #[inline]
    pub fn new() -> Self {
        Self { error: PhantomData }
    }
}

impl<E> Default for BogusBody<E> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<E> Body for BogusBody<E> {
    type Data = bytes::Bytes;
    type Error = E;

    #[inline]
    fn poll_frame(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<http_body::Frame<bytes::Bytes>, Self::Error>>> {
        std::task::Poll::Ready(None)
    }

    #[inline]
    fn is_end_stream(&self) -> bool {
        false
    }

    #[inline]
    fn size_hint(&self) -> http_body::SizeHint {
        http_body::SizeHint::with_exact(42)
    }
}
