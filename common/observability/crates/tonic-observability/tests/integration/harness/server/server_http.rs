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

use super::serve::serve_with_incoming_shutdown;
use super::{BoxError, Guards, HttpRequest, HttpResponse, HttpResult, HttpServer, MakeServer};
use std::future::Future;
use std::marker::PhantomData;
use std::task::{Context, Poll};
use tokio::sync::oneshot;
use tonic::async_trait;
use tower::Service;

/// Implement this to make a HTTP2 servers that mimic tonic, but can return an
/// error.
///
/// Is also implemented automatically for any <code>async fn ([HttpRequest]) ->
/// [HttpResult]</code> function.
pub(crate) trait HttpService: Clone + Send + Sync + 'static {
    type Future: Future<Output = HttpResult<BoxError>> + Send + 'static;

    fn call(&mut self, req: HttpRequest) -> Self::Future;
}

impl<F, Fut> HttpService for F
where
    F: Fn(HttpRequest) -> Fut + Send + Sync + Clone + 'static,
    Fut: Future<Output = HttpResult<BoxError>> + Send + 'static,
{
    type Future = Fut;

    fn call(&mut self, req: HttpRequest) -> Self::Future {
        self(req)
    }
}

/// Private wrapper to adapt a [`HttpService`] to a tower [`Service`].
#[derive(Debug, Clone)]
#[repr(transparent)]
struct HttpAdapter<T>(T);

impl<T: HttpService> Service<HttpRequest> for HttpAdapter<T> {
    type Error = BoxError;
    type Response = HttpResponse;
    type Future = T::Future;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: HttpRequest) -> Self::Future {
        <T as HttpService>::call(&mut self.0, req)
    }
}

pub(crate) struct Http1;

impl<Svc: HttpService> HttpServer<Svc, Http1> {
    pub(crate) fn http1(handler: Svc) -> Self {
        Self {
            inner: handler,
            marker: PhantomData,
        }
    }
}

pub(crate) struct Http2;

impl<Svc: HttpService> HttpServer<Svc, Http2> {
    pub(crate) fn http2(handler: Svc) -> Self {
        Self {
            inner: handler,
            marker: PhantomData,
        }
    }
}

#[async_trait]
impl<Svc: HttpService> MakeServer for HttpServer<Svc, Http1> {
    type Fut = futures::future::BoxFuture<'static, ()>;

    async fn make(self, listener: tokio::net::TcpListener, _guards: &Guards) -> Self::Fut {
        let svc = HttpAdapter(self.inner);

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

        let fut = serve_with_incoming_shutdown(svc, listener, shutdown_rx, false);
        let handle = tokio::spawn(fut);

        Box::pin(async move {
            let _res: Result<(), ()> = shutdown_tx.send(());
            let _: () = handle.await.expect("failed to join server");
        })
    }
}

#[async_trait]
impl<Svc: HttpService> MakeServer for HttpServer<Svc, Http2> {
    type Fut = futures::future::BoxFuture<'static, ()>;

    async fn make(self, listener: tokio::net::TcpListener, _guards: &Guards) -> Self::Fut {
        let svc = HttpAdapter(self.inner);

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

        let fut = serve_with_incoming_shutdown(svc, listener, shutdown_rx, true);
        let handle = tokio::spawn(fut);

        Box::pin(async move {
            let _res: Result<(), ()> = shutdown_tx.send(());
            let _: () = handle.await.expect("failed to join server");
        })
    }
}
