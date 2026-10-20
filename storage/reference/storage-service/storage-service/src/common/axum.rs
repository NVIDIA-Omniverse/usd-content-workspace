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

use std::{future::Future, task::Poll};

use axum::{
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
};
use pin_project_lite::pin_project;
use tower::{Layer, Service};

#[derive(Debug, Clone, Copy)]
pub struct LoadShedLayer;

impl<S> Layer<S> for LoadShedLayer {
    type Service = LoadShedService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        LoadShedService { is_ready: false, inner }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct LoadShedService<S> {
    inner: S,
    is_ready: bool,
}

impl<S, Req, Res> Service<Request<Req>> for LoadShedService<S>
where
    S: Service<Request<Req>, Response = Res>,
    Res: IntoResponse,
{
    type Response = Response;
    type Error = S::Error;
    type Future = ResponseFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut std::task::Context<'_>) -> std::task::Poll<Result<(), Self::Error>> {
        self.is_ready = match self.inner.poll_ready(cx) {
            Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
            r => r.is_ready(),
        };

        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<Req>) -> Self::Future {
        if self.is_ready {
            self.is_ready = false;
            ResponseFuture {
                state: ResponseState::Called {
                    fut: self.inner.call(req),
                },
            }
        } else {
            ResponseFuture {
                state: ResponseState::Overloaded,
            }
        }
    }
}

pin_project! {
    pub struct ResponseFuture<F> {
        #[pin]
        state: ResponseState<F>,
    }
}

pin_project! {
    #[project = ResponseStateProj]
    enum ResponseState<F> {
        Called {
            #[pin]
            fut: F
        },
        Overloaded,
    }
}

impl<F, T, E> Future for ResponseFuture<F>
where
    F: Future<Output = Result<T, E>>,
    T: IntoResponse,
{
    type Output = Result<Response, E>;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Self::Output> {
        match self.project().state.project() {
            ResponseStateProj::Called { fut } => fut.poll(cx).map_ok(|response| response.into_response()),
            ResponseStateProj::Overloaded => Poll::Ready(Ok(create_load_shed_response())),
        }
    }
}

fn create_load_shed_response() -> Response {
    (StatusCode::TOO_MANY_REQUESTS, "too many requests").into_response()
}
