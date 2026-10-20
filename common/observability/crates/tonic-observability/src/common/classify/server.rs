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

/// Only certain codes are server errors.
///
/// See <https://opentelemetry.io/docs/specs/semconv/rpc/grpc/#grpc-status>.
pub(crate) fn tonic_is_error(code: tonic::Code) -> bool {
    match code {
        tonic::Code::Ok => false,
        tonic::Code::Cancelled => false,
        tonic::Code::Unknown => true,
        tonic::Code::InvalidArgument => false,
        tonic::Code::DeadlineExceeded => true,
        tonic::Code::NotFound => false,
        tonic::Code::AlreadyExists => false,
        tonic::Code::PermissionDenied => false,
        tonic::Code::ResourceExhausted => false,
        tonic::Code::FailedPrecondition => false,
        tonic::Code::Aborted => false,
        tonic::Code::OutOfRange => false,
        tonic::Code::Unimplemented => true,
        tonic::Code::Internal => true,
        tonic::Code::Unavailable => true,
        tonic::Code::DataLoss => true,
        tonic::Code::Unauthenticated => false,
    }
}

/// Only certain codes are server errors.
///
/// See <https://opentelemetry.io/docs/specs/semconv/rpc/grpc/#grpc-status>.
pub(crate) fn code_is_error(code: u16) -> bool {
    // unknown codes are mapped to `Code::Unknown`, which is always an error
    tonic_is_error(tonic::Code::from_i32(code as _))
}
