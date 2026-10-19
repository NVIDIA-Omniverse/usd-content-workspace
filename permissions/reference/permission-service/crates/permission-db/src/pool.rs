// SPDX-FileCopyrightText: Copyright 2026 NVIDIA CORPORATION & AFFILIATES
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

use async_trait::async_trait;
use deadpool_postgres::{Object, Pool, PoolError};
use std::sync::Arc;

/// Abstraction over a Postgres connection pool used by the storage layer.
///
/// Decouples storage from the concrete pool so callers can supply additional
/// behavior (such as instrumentation) without the storage layer being aware.
#[async_trait]
pub trait ConnectionPool: Send + Sync {
    async fn get(&self) -> Result<Object, PoolError>;
}

/// Pass-through `ConnectionPool` that delegates directly to a deadpool `Pool`.
pub struct DirectPool {
    inner: Arc<Pool>,
}

impl DirectPool {
    pub fn new(pool: Arc<Pool>) -> Self {
        Self { inner: pool }
    }
}

#[async_trait]
impl ConnectionPool for DirectPool {
    async fn get(&self) -> Result<Object, PoolError> {
        self.inner.get().await
    }
}
