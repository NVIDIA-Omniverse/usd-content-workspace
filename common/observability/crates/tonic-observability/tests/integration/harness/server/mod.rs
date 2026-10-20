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

mod serve;
mod server_http;
mod server_mock;
mod server_tonic;

pub(crate) use server_tonic::make_server;
use std::future::Future;
use std::marker::PhantomData;
use testing::guards::Guards;
use tokio::net::TcpListener;
use tonic::async_trait;
use tonic::codegen::http;

#[async_trait]
pub(crate) trait MakeServer {
    type Fut: Future<Output = ()>;

    async fn make(self, listener: TcpListener, guards: &Guards) -> Self::Fut;
}

pub(crate) type HttpRequest = http::Request<tonic::body::Body>;
pub(crate) type HttpResponse<B = tonic::body::Body> = http::Response<B>;
pub(crate) type BoxError = Box<dyn std::error::Error + Send + Sync>;
pub(crate) type HttpResult<E> = Result<HttpResponse, E>;

/// A HTTP2 servers that mimic tonic.
///
/// Implement [`HttpService`] or [`TonicService`].
#[derive(Debug)]
pub(crate) struct HttpServer<Svc, Kind> {
    inner: Svc,
    marker: PhantomData<Kind>,
}
