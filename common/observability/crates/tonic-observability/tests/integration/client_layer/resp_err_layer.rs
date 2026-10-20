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

use pin_project_lite::pin_project;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use testing::body::ErrorBody;
use tonic::codegen::http;

pub(super) struct RespErrLayer {
    status: tonic::Status,
}

impl RespErrLayer {
    pub(super) fn new(status: tonic::Status) -> Self {
        Self { status }
    }
}

impl<Inner> tower_layer::Layer<Inner> for RespErrLayer {
    type Service = RespErrService<Inner>;

    fn layer(&self, inner: Inner) -> Self::Service {
        RespErrService {
            inner,
            status: self.status.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct RespErrService<Svc> {
    inner: Svc,
    status: tonic::Status,
}

impl<Svc> tower::Service<http::Request<tonic::body::Body>> for RespErrService<Svc>
where
    Svc: tonic::client::GrpcService<tonic::body::Body, Error = tonic::transport::Error>,
    Svc::ResponseBody: http_body::Body<Data = bytes::Bytes, Error = tonic::Status> + Send + 'static,
{
    type Future = RespErrFut<Svc::Future>;
    type Response = http::Response<ErrorBody<tonic::Status>>;
    type Error = tonic::transport::Error;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: http::Request<tonic::body::Body>) -> Self::Future {
        let status = self.status.clone();
        let inner = self.inner.call(request);
        RespErrFut { inner, status }
    }
}

pin_project! {
    #[derive(Debug)]
    pub(super) struct RespErrFut<Fut> {
        #[pin]
        inner: Fut,
        status: tonic::Status,
    }
}

impl<Fut, RespBody> Future for RespErrFut<Fut>
where
    Fut: Future<Output = Result<http::Response<RespBody>, tonic::transport::Error>>,
    RespBody: http_body::Body<Error = tonic::Status>,
{
    type Output = Result<http::Response<ErrorBody<tonic::Status>>, tonic::transport::Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        match this.inner.poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Ok(body)) => {
                let status = this.status.clone();
                let body = body.map(|_body| ErrorBody::new(status));
                Poll::Ready(Ok(body))
            }
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
        }
    }
}
