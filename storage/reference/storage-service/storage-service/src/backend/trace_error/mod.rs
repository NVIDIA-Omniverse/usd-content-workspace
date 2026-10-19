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

use std::fmt::Debug;

use axum::http::StatusCode;
use storage_core::{layer::Layer, traits::StorageError};

use crate::common::error::PrettyPrintErrorExt;

mod capabilities;
mod filefolder;
mod fileobject;
mod metadata;
mod versioning;

#[track_caller]
pub(crate) fn log_storage_error(error: &StorageError, operation: &str) {
    match error.http_status() {
        StatusCode::UNAUTHORIZED | StatusCode::REQUEST_TIMEOUT => {
            tracing::warn!(error = %error.pretty(), operation, "storage operation failed");
        },
        // A request for a capability the service does not have is driven by the client and is
        // expected to be retried differently, so it is not reported as a service failure.
        StatusCode::NOT_IMPLEMENTED => {
            tracing::info!(error = %error.pretty(), operation, "storage operation is not supported");
        },
        e if e.is_client_error() => {
            tracing::info!(error = %error.pretty(), operation, "storage operation returned client error");
        },
        e if e.is_server_error() => {
            tracing::error!(error = %error.pretty(), operation, "storage operation failed");
        },
        _ => {
            tracing::debug!(error = %error.pretty(), operation, "storage operation returned unexpected status");
        },
    }
}

/// Operation error tracing layer.
#[derive(Default, Debug, Clone, Copy)]
pub struct TraceErrorLayer;

impl<S> Layer<S> for TraceErrorLayer {
    type Service = TraceErrorService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        TraceErrorService { inner }
    }
}

/// Traces operation errors.
#[derive(Clone)]
pub struct TraceErrorService<S> {
    pub(self) inner: S,
}

impl<S: Debug> Debug for TraceErrorService<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TraceErrorService").field("inner", &self.inner).finish()
    }
}
