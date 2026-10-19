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

use opentelemetry::StringValue;
use std::fmt;
use std::sync::Arc;
use tonic::codegen::http;

/// A boxed function that maps a request to a target value, for example
/// `grpc.target`.
///
/// The current approach is to use `Arc<dyn ...>`.  However, the function passed
/// to [`TargetFn::new()`] is required to be `Clone`, so that we could use a box
/// approach instead. This is more complex, as `Clone` isn't dyn compatible by
/// default (untested):
///
/// ```rust
/// use opentelemetry::StringValue;
/// use tonic::codegen::http;
///
/// trait TargetFnClone: for<'a> Fn(&'a http::request::Parts) -> Option<StringValue> + Send + Sync + 'static {
///     fn clone_box(&self) -> Box<dyn TargetFnClone>;
/// }
///
/// impl<F> TargetFnClone for F
/// where
///     F: for<'a> Fn(&'a http::request::Parts) -> Option<StringValue> + Clone + Send + Sync + 'static,
/// {
///     fn clone_box(&self) -> Box<dyn TargetFnClone> {
///         Box::new(self.clone())
///     }
/// }
///
/// impl Clone for Box<dyn TargetFnClone> {
///     fn clone(&self) -> Self {
///         (**self).clone_box()
///     }
/// }
/// ```
///
/// Basically, we'd need to profile a client with the layer to figure out how
/// often services are cloned, and whether the atomic increments of `Arc` cause
/// CPU caches to be invalidated a lot. For now, the code is simpler with `Arc`.
#[derive(Clone)]
#[expect(
    clippy::type_complexity,
    reason = "the newtype is here to abstract the type complexity"
)]
pub(crate) struct TargetFn(
    Arc<dyn for<'a> Fn(&'a http::request::Parts) -> Option<StringValue> + Send + Sync>,
);

impl TargetFn {
    pub(crate) fn new<F>(f: F) -> Self
    where
        F: for<'a> Fn(&'a http::request::Parts) -> Option<StringValue>
            + Clone
            + Send
            + Sync
            + 'static,
    {
        Self(Arc::new(f))
    }

    pub(crate) fn call(&self, parts: &http::request::Parts) -> Option<StringValue> {
        (self.0)(parts)
    }
}

impl fmt::Debug for TargetFn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(concat!(stringify!(TargetFn), "(..)"))
    }
}

#[cfg(test)]
mod tests;
