// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
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

//! Small local replacements for the unpublished upstream test helpers.

use std::marker::PhantomData;

#[derive(Debug)]
pub(crate) struct BogusBody<E>(PhantomData<E>);

impl<E> BogusBody<E> {
    pub(crate) fn new() -> Self {
        Self(PhantomData)
    }
}

impl<E> http_body::Body for BogusBody<E> {
    type Data = bytes::Bytes;
    type Error = E;

    fn poll_frame(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        std::task::Poll::Ready(None)
    }

    fn is_end_stream(&self) -> bool {
        false
    }

    fn size_hint(&self) -> http_body::SizeHint {
        http_body::SizeHint::with_exact(42)
    }
}

#[derive(Debug)]
pub(crate) struct BogusInstrumentProvider;

impl BogusInstrumentProvider {
    pub(crate) fn meter() -> opentelemetry::metrics::Meter {
        opentelemetry::metrics::Meter::new(std::sync::Arc::new(Self))
    }
}

impl opentelemetry::metrics::InstrumentProvider for BogusInstrumentProvider {}
