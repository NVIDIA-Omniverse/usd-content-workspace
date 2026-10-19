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
use tonic::{Code, Status};

use crate::{
    status::utils::BindResultExt,
    types::{AsObservable, Observe},
};

/// GRPC operation result metrics.
pub struct ResultMetrics {
    status_ok: BoundUIntCounter,
    status_cancelled: BoundUIntCounter,
    status_unknown: BoundUIntCounter,
    status_invalid_argument: BoundUIntCounter,
    status_deadline_exceeded: BoundUIntCounter,
    status_not_found: BoundUIntCounter,
    status_already_exists: BoundUIntCounter,
    status_permission_denied: BoundUIntCounter,
    status_resource_exhausted: BoundUIntCounter,
    status_failed_precondition: BoundUIntCounter,
    status_aborted: BoundUIntCounter,
    status_out_of_range: BoundUIntCounter,
    status_unimplemented: BoundUIntCounter,
    status_internal: BoundUIntCounter,
    status_unavailable: BoundUIntCounter,
    status_data_loss: BoundUIntCounter,
    status_unauthenticated: BoundUIntCounter,
}

impl ResultMetrics {
    /// Create a metric set for the specified base counter.
    pub fn new(base: &UIntCounter) -> Self {
        Self {
            status_ok: base.bind_result("Ok"),
            status_cancelled: base.bind_result("Cancelled"),
            status_unknown: base.bind_result("Unknown"),
            status_invalid_argument: base.bind_result("InvalidArgument"),
            status_deadline_exceeded: base.bind_result("DeadlineExceeded"),
            status_not_found: base.bind_result("NotFound"),
            status_already_exists: base.bind_result("AlreadyExists"),
            status_permission_denied: base.bind_result("PermissionDenied"),
            status_resource_exhausted: base.bind_result("ResourceExhausted"),
            status_failed_precondition: base.bind_result("FailedPrecondition"),
            status_aborted: base.bind_result("Aborted"),
            status_out_of_range: base.bind_result("OutOfRange"),
            status_unimplemented: base.bind_result("Unimplemented"),
            status_internal: base.bind_result("Internal"),
            status_unavailable: base.bind_result("Unavailable"),
            status_data_loss: base.bind_result("DataLoss"),
            status_unauthenticated: base.bind_result("Unauthenticated"),
        }
    }

    /// Return the counter for a status code.
    fn get_counter_for_status(&self, code: Code) -> &BoundUIntCounter {
        match code {
            Code::Ok => &self.status_ok,
            Code::Cancelled => &self.status_cancelled,
            Code::Unknown => &self.status_unknown,
            Code::InvalidArgument => &self.status_invalid_argument,
            Code::DeadlineExceeded => &self.status_deadline_exceeded,
            Code::NotFound => &self.status_not_found,
            Code::AlreadyExists => &self.status_already_exists,
            Code::PermissionDenied => &self.status_permission_denied,
            Code::ResourceExhausted => &self.status_resource_exhausted,
            Code::FailedPrecondition => &self.status_failed_precondition,
            Code::Aborted => &self.status_aborted,
            Code::OutOfRange => &self.status_out_of_range,
            Code::Unimplemented => &self.status_unimplemented,
            Code::Internal => &self.status_internal,
            Code::Unavailable => &self.status_unavailable,
            Code::DataLoss => &self.status_data_loss,
            Code::Unauthenticated => &self.status_unauthenticated,
        }
    }
}

impl Observe for &ResultMetrics {
    type Value = Code;

    fn observe(self, value: &Code) {
        self.get_counter_for_status(*value).add(1);
    }
}

impl std::fmt::Debug for ResultMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResultMetrics").finish_non_exhaustive()
    }
}

impl AsObservable<Code> for Code {
    fn as_observable(&self) -> Code {
        *self
    }
}

impl AsObservable<Code> for Status {
    fn as_observable(&self) -> Code {
        self.code()
    }
}

impl<T> AsObservable<Code> for Result<T, Status> {
    fn as_observable(&self) -> Code {
        if let Err(err) = self {
            err.as_observable()
        } else {
            Code::Ok
        }
    }
}
