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

//! One retry classification for every outbound gRPC call the thumbnail pipeline
//! makes.
//!
//! The policy is the one `TaskQueueError::is_retryable_submission_failure`
//! established over HTTP status codes — transport and dependency failures are
//! worth another delivery, poison input and misconfiguration are not — expressed
//! over `tonic::Code`, because after the cutover every one of these calls is
//! gRPC. The HTTP matrix it replaces (5xx plus 408/425/429) has no meaning here.
//!
//! Publish, storage read, storage write, and associate all classify through this
//! function, so a decision made for one of them cannot drift from the others.

use tonic::Code;

/// Whether a gRPC status code is worth another delivery.
///
/// `Unauthenticated` and `PermissionDenied` are permanent here, and that is a
/// deliberate carry-over rather than an oversight: every call site refreshes the
/// service-account token once and reissues the same request before the status
/// reaches this function, so a code that survives that is a credential or policy
/// problem, exactly as HTTP 401/403 was under the previous matrix. Redelivering
/// would reproduce it until the message dead-letters.
pub fn is_retryable_tonic_code(code: Code) -> bool {
    match code {
        Code::Unavailable
        | Code::DeadlineExceeded
        | Code::ResourceExhausted
        | Code::Aborted
        | Code::Internal
        | Code::Unknown
        | Code::Cancelled
        | Code::DataLoss => true,

        Code::Ok
        | Code::InvalidArgument
        | Code::NotFound
        | Code::AlreadyExists
        | Code::PermissionDenied
        | Code::Unauthenticated
        | Code::FailedPrecondition
        | Code::OutOfRange
        | Code::Unimplemented => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_and_dependency_codes_are_retryable() {
        for code in [
            Code::Unavailable,
            Code::DeadlineExceeded,
            Code::ResourceExhausted,
            Code::Aborted,
            Code::Internal,
            Code::Unknown,
            Code::Cancelled,
            Code::DataLoss,
        ] {
            assert!(
                is_retryable_tonic_code(code),
                "{code:?} is a dependency failure and should be retried"
            );
        }
    }

    #[test]
    fn poison_input_and_credential_codes_are_permanent() {
        for code in [
            Code::InvalidArgument,
            Code::NotFound,
            Code::AlreadyExists,
            Code::PermissionDenied,
            Code::Unauthenticated,
            Code::FailedPrecondition,
            Code::OutOfRange,
            Code::Unimplemented,
        ] {
            assert!(
                !is_retryable_tonic_code(code),
                "{code:?} will fail the same way on redelivery"
            );
        }
    }

    /// The gRPC classification keeps the HTTP matrix's shape: server-side and
    /// transport failures retry, client-side rejections do not.
    #[test]
    fn classification_matches_the_http_policy_it_replaces() {
        assert!(
            is_retryable_tonic_code(Code::Internal),
            "gRPC Internal ~ 500"
        );
        assert!(
            is_retryable_tonic_code(Code::ResourceExhausted),
            "gRPC ResourceExhausted ~ 429"
        );
        assert!(
            is_retryable_tonic_code(Code::DeadlineExceeded),
            "gRPC DeadlineExceeded ~ 408"
        );
        assert!(
            !is_retryable_tonic_code(Code::InvalidArgument),
            "gRPC InvalidArgument ~ 400"
        );
    }
}
