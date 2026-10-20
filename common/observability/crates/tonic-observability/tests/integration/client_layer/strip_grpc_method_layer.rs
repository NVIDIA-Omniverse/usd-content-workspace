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

use std::task::{Context, Poll};
use tonic::codegen::http;

pub(super) struct StripGrpcMethodLayer;

impl<Inner> tower_layer::Layer<Inner> for StripGrpcMethodLayer {
    type Service = StripGrpcMethodService<Inner>;

    fn layer(&self, inner: Inner) -> Self::Service {
        StripGrpcMethodService { inner }
    }
}

#[derive(Debug, Clone)]
pub(super) struct StripGrpcMethodService<Svc> {
    inner: Svc,
}

impl<Svc> tower::Service<http::Request<tonic::body::Body>> for StripGrpcMethodService<Svc>
where
    Svc: tonic::client::GrpcService<tonic::body::Body, Error = tonic::transport::Error>,
{
    type Future = Svc::Future;
    type Response = http::Response<Svc::ResponseBody>;
    type Error = tonic::transport::Error;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: http::Request<tonic::body::Body>) -> Self::Future {
        let (mut parts, body) = request.into_parts();
        parts.extensions.remove::<tonic::codegen::GrpcMethod>();
        let request = http::Request::from_parts(parts, body);
        self.inner.call(request)
    }
}
