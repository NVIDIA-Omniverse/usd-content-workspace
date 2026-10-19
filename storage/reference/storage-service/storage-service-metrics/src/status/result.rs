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

use storage_core_util::observability::metrics::{BoundUIntCounter, UIntCounter};

use crate::{
    status::utils::BindResultExt,
    types::{AsObservable, Observe},
};

/// An operation result.
#[derive(Debug, Clone, Copy)]
pub enum Status {
    Ok,
    Err,
}

/// Operation result metrics.
pub struct ResultMetrics {
    status_ok: BoundUIntCounter,
    status_err: BoundUIntCounter,
}

impl ResultMetrics {
    /// Create metrics from a base counter.
    pub fn new(base: &UIntCounter) -> Self {
        Self::new_bind(|result| base.bind_result(result))
    }

    pub fn new_bind(mut bind_result: impl FnMut(&'static str) -> BoundUIntCounter) -> Self {
        Self {
            status_ok: bind_result("Ok"),
            status_err: bind_result("Err"),
        }
    }

    /// Return the counter for a value.
    fn get_counter_for_value(&self, value: Status) -> &BoundUIntCounter {
        match value {
            Status::Ok => &self.status_ok,
            Status::Err => &self.status_err,
        }
    }
}

impl Observe for &ResultMetrics {
    type Value = Status;

    fn observe(self, value: &Status) {
        self.get_counter_for_value(*value).add(1);
    }
}

impl std::fmt::Debug for ResultMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResultMetrics").finish_non_exhaustive()
    }
}

impl<T, E> AsObservable<Status> for Result<T, E> {
    fn as_observable(&self) -> Status {
        match self {
            Self::Ok(_) => Status::Ok,
            Self::Err(_) => Status::Err,
        }
    }
}

impl AsObservable<Status> for Status {
    fn as_observable(&self) -> Status {
        *self
    }
}
