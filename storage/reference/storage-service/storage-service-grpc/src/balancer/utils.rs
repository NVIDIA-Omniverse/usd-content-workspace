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

use std::{collections::HashSet, net::SocketAddr};

use tonic::transport::Endpoint;

#[async_trait::async_trait]
pub trait Resolve: Send + Sync + 'static {
    type Error: std::error::Error + Send + Sync + 'static;

    async fn resolve(&mut self, host: &str, port: u16) -> Result<HashSet<SocketAddr>, Self::Error>;
}

pub trait Configure: Send + Sync + 'static {
    type Error: std::error::Error + Send + Sync + 'static;

    fn configure(&mut self, endpoint: Endpoint) -> Result<Endpoint, Self::Error>;
}

pub struct TokioLookupHostResolver;

#[async_trait::async_trait]
impl Resolve for TokioLookupHostResolver {
    type Error = std::io::Error;

    async fn resolve(&mut self, host: &str, port: u16) -> Result<HashSet<SocketAddr>, Self::Error> {
        tokio::net::lookup_host((host, port))
            .await
            .map(|addresses| addresses.collect())
    }
}

impl<T, E> Configure for T
where
    T: FnMut(Endpoint) -> Result<Endpoint, E> + Send + Sync + 'static,
    E: std::error::Error + Send + Sync + 'static,
{
    type Error = E;

    fn configure(&mut self, endpoint: Endpoint) -> Result<Endpoint, Self::Error> {
        (self)(endpoint)
    }
}
