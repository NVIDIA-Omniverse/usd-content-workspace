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

//! Service-identity and outbound gRPC interceptor surface for
//! `thumbnail-service`.
//!
//! The service-identity types and the outbound client-credentials /
//! access-token-file interceptors now live in the shared `permission-auth`
//! crate (consumed from the internal registry) rather than being copy-pasted
//! here. This crate re-exports that surface under the `thumbnail_auth::*`
//! paths the rest of the workspace already uses, and adds the two
//! thumbnail-specific pieces that are not part of `permission-auth`:
//! [`auth_interceptor::AuthInterceptor`] (a runtime-selected interceptor enum)
//! and [`discovery`] (the minimal OpenID discovery helper).

pub mod auth_interceptor;
pub mod discovery;

pub use auth_interceptor::AuthInterceptor;
pub use discovery::{load_openid_configuration, OpenIdConfigResponse};
pub use permission_auth::interceptors::access_token_file::{
    AccessTokenFileArgs, AccessTokenFileInterceptor, AccessTokenFileProvider,
};
pub use permission_auth::interceptors::client_credentials::{
    ClientCredentialsArgs, ClientCredentialsInterceptor, Token, TokenManager,
};
pub use permission_auth::interceptors::TokenRefresher;
pub use permission_auth::service_identity::{
    exchange_client_credentials_token, parse_access_token_file, read_token_file,
    AccessTokenFileToken, ServiceIdentityError, TokenResponse,
};
