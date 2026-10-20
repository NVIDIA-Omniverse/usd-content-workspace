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

//! Unified [`tonic::service::Interceptor`] that wraps either flow's interceptor
//! as a single concrete type, so outbound gRPC clients can be typed
//! `Client<InterceptedService<Channel, AuthInterceptor>>` regardless of which
//! outbound-auth mode is active at runtime.
//!
//! This is a thumbnail-specific addition (not part of permission-service's
//! canonical pattern, which uses an `impl_<service>!` macro to provide
//! per-service trait impls for each concrete interceptor type). Thumbnail
//! has fewer outbound services and the enum keeps the call sites simpler.

use permission_auth::interceptors::access_token_file::AccessTokenFileInterceptor;
use permission_auth::interceptors::client_credentials::ClientCredentialsInterceptor;
use tonic::service::Interceptor;
use tonic::{Request, Status};

#[derive(Clone)]
pub enum AuthInterceptor {
    ClientCredentials(ClientCredentialsInterceptor),
    AccessTokenFile(AccessTokenFileInterceptor),
}

impl Interceptor for AuthInterceptor {
    fn call(&mut self, request: Request<()>) -> Result<Request<()>, Status> {
        match self {
            Self::ClientCredentials(i) => i.call(request),
            Self::AccessTokenFile(i) => i.call(request),
        }
    }
}
