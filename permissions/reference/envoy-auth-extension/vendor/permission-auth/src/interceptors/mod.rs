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

//! Outbound gRPC client auth interceptors for service identity.
//!
//! These build on [`crate::service_identity`] to attach a bearer token to
//! outbound `tonic` requests, sourcing the token either from an OAuth2
//! client-credentials exchange or from an access-token file on disk.

pub mod access_token_file;
pub mod client_credentials;

#[cfg(test)]
mod tests;

#[tonic::async_trait]
pub trait TokenRefresher: Send + Sync {
    async fn refresh(&self);
}
