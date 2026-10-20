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

use super::{client, server};

#[test]
fn client_http_to_tonic_status() {
    use client::http_to_tonic_status as map;
    use tonic::Code;
    use tonic::codegen::http::StatusCode;
    assert_eq!(map(StatusCode::BAD_REQUEST), Code::Internal);
    assert_eq!(map(StatusCode::UNAUTHORIZED), Code::Unauthenticated);
    assert_eq!(map(StatusCode::FORBIDDEN), Code::PermissionDenied);
    assert_eq!(map(StatusCode::NOT_FOUND), Code::Unimplemented);
    assert_eq!(map(StatusCode::TOO_MANY_REQUESTS), Code::Unavailable);
    assert_eq!(map(StatusCode::BAD_GATEWAY), Code::Unavailable);
    assert_eq!(map(StatusCode::SERVICE_UNAVAILABLE), Code::Unavailable);
    assert_eq!(map(StatusCode::GATEWAY_TIMEOUT), Code::Unavailable);
    assert_eq!(map(StatusCode::OK), Code::Unknown);
}

#[test]
#[expect(clippy::bool_assert_comparison, reason = "readability")]
fn server_tonic_is_error() {
    use server::tonic_is_error as map;
    use tonic::Code;
    assert_eq!(map(Code::Ok), false);
    assert_eq!(map(Code::Cancelled), false);
    assert_eq!(map(Code::Unknown), true);
    assert_eq!(map(Code::InvalidArgument), false);
    assert_eq!(map(Code::DeadlineExceeded), true);
    assert_eq!(map(Code::NotFound), false);
    assert_eq!(map(Code::AlreadyExists), false);
    assert_eq!(map(Code::PermissionDenied), false);
    assert_eq!(map(Code::ResourceExhausted), false);
    assert_eq!(map(Code::FailedPrecondition), false);
    assert_eq!(map(Code::Aborted), false);
    assert_eq!(map(Code::OutOfRange), false);
    assert_eq!(map(Code::Unimplemented), true);
    assert_eq!(map(Code::Internal), true);
    assert_eq!(map(Code::Unavailable), true);
    assert_eq!(map(Code::DataLoss), true);
    assert_eq!(map(Code::Unauthenticated), false);
}
