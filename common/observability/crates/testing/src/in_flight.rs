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

//! Waiting for a test server to drop every request it served.
//!
//! A graceful shutdown only waits for the connections. With HTTP/2, each
//! request is served on its own task, which may still hold the response body
//! after the client is done with it, and the server span only ends when the
//! response body is dropped. If the exporters are shut down before that, the
//! server span is lost.

use pin_project_lite::pin_project;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, ready};
use tokio::sync::mpsc;

/// Makes a layer tracking the requests in flight, and a handle to wait for
/// them.
///
/// The layer should be the outermost layer of the server, so it is dropped
/// after the observability layers.
pub fn in_flight() -> (InFlightLayer, InFlight) {
    let (tx, rx) = mpsc::channel(1);
    (InFlightLayer { tx }, InFlight { rx })
}

/// Waits for the requests in flight. See [`in_flight`].
#[derive(Debug)]
pub struct InFlight {
    rx: mpsc::Receiver<()>,
}

impl InFlight {
    /// Waits until the layer and every request it saw have been dropped,
    /// including their response bodies.
    ///
    /// The server (and so the layer) must be dropped, or this never returns.
    pub async fn wait(mut self) {
        // nothing is ever sent, this returns once every sender is dropped
        let _: Option<()> = self.rx.recv().await;
    }
}

/// Holds a sender for as long as a request is in flight. See [`in_flight`].
#[derive(Debug, Clone)]
pub struct InFlightLayer {
    tx: mpsc::Sender<()>,
}

impl<Svc> tower_layer::Layer<Svc> for InFlightLayer {
    type Service = InFlightService<Svc>;

    fn layer(&self, inner: Svc) -> Self::Service {
        InFlightService {
            inner,
            tx: self.tx.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct InFlightService<Svc> {
    inner: Svc,
    tx: mpsc::Sender<()>,
}

impl<Svc, ReqBody, RespBody> tower_service::Service<http::Request<ReqBody>> for InFlightService<Svc>
where
    Svc: tower_service::Service<http::Request<ReqBody>, Response = http::Response<RespBody>>,
{
    type Response = http::Response<InFlightBody<RespBody>>;
    type Error = Svc::Error;
    type Future = InFlightFuture<Svc::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: http::Request<ReqBody>) -> Self::Future {
        InFlightFuture {
            inner: self.inner.call(request),
            tx: Some(self.tx.clone()),
        }
    }
}

pin_project! {
    // fields are dropped in declaration order, so `inner` is dropped first
    #[derive(Debug)]
    pub struct InFlightFuture<Fut> {
        #[pin]
        inner: Fut,
        tx: Option<mpsc::Sender<()>>,
    }
}

impl<Fut, RespBody, Error> Future for InFlightFuture<Fut>
where
    Fut: Future<Output = Result<http::Response<RespBody>, Error>>,
{
    type Output = Result<http::Response<InFlightBody<RespBody>>, Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let response = ready!(this.inner.poll(cx))?;
        let tx = this.tx.take();
        Poll::Ready(Ok(response.map(|inner| InFlightBody { inner, _tx: tx })))
    }
}

pin_project! {
    // fields are dropped in declaration order, so `inner` is dropped first
    #[derive(Debug)]
    pub struct InFlightBody<Body> {
        #[pin]
        inner: Body,
        _tx: Option<mpsc::Sender<()>>,
    }
}

impl<Body: http_body::Body> http_body::Body for InFlightBody<Body> {
    type Data = Body::Data;
    type Error = Body::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        self.project().inner.poll_frame(cx)
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> http_body::SizeHint {
        self.inner.size_hint()
    }
}
