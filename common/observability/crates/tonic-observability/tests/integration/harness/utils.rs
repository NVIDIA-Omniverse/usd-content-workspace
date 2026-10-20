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

use prost::Message;
use protos::{EchoRequest, EchoResponse};
use std::fmt::Debug;
use std::sync::LazyLock;
use testing::stream;
use tonic::{Code, async_trait};

pub(crate) fn resource() -> otel_tools::resource::Resource {
    otel_tools::resource::ResourceBuilder::without_default("nv.test").build()
}

static RPC_MAP: LazyLock<tonic_observability::rpc_map::RpcMap> = LazyLock::new(|| {
    tonic_observability::rpc_map::RpcMap::builder()
        .optimize(tonic_observability::rpc_map::RpcMapOptimize::STATIC)
        .add_encoded_file_descriptor_set(protos::EXAMPLE_FILE_DESCRIPTOR_SET)
        .unwrap()
        .build()
});

pub(crate) fn rpc_map() -> tonic_observability::rpc_map::RpcMap {
    (*RPC_MAP).clone()
}

pub(crate) fn echo_req(s: impl Into<String>) -> EchoRequest {
    EchoRequest { message: s.into() }
}

pub(crate) fn echo_resp(s: impl Into<String>) -> EchoResponse {
    EchoResponse { message: s.into() }
}

pub(crate) trait CalcSize: Unpin {
    fn size(&self) -> usize;
}

fn grpc_size<T: Message>(v: &T) -> usize {
    let mut buf = bytes::BytesMut::with_capacity(128);
    v.encode(&mut buf).unwrap();
    buf.len()
}

impl CalcSize for EchoRequest {
    fn size(&self) -> usize {
        grpc_size(self)
    }
}

impl CalcSize for EchoResponse {
    fn size(&self) -> usize {
        grpc_size(self)
    }
}

impl CalcSize for tonic::Status {
    fn size(&self) -> usize {
        self.message().len() + self.details().len()
    }
}

impl<T: CalcSize, E: CalcSize> CalcSize for Result<T, E> {
    fn size(&self) -> usize {
        match self {
            Ok(v) => v.size(),
            Err(e) => e.size(),
        }
    }
}

impl<T: CalcSize> CalcSize for Vec<T> {
    fn size(&self) -> usize {
        self.iter().map(CalcSize::size).sum()
    }
}

impl<T: CalcSize> CalcSize for stream::Once<T> {
    fn size(&self) -> usize {
        self.items().map(CalcSize::size).sum()
    }
}

impl<T: CalcSize> CalcSize for stream::Many<T> {
    fn size(&self) -> usize {
        self.items().map(CalcSize::size).sum()
    }
}

pub(crate) fn make_req<T: CalcSize>(v: T) -> (tonic::Request<T>, f64) {
    let size = v.size();
    let req = tonic::Request::new(v);
    (req, size as _)
}

pub(crate) trait IntoResp<T: Sized>: Sized {
    fn into(self) -> T;
}

impl IntoResp<Result<Resp<EchoResponse>, Stat>> for EchoResponse {
    fn into(self) -> Result<Resp<EchoResponse>, Stat> {
        Ok(Resp::new(self))
    }
}

impl IntoResp<Result<Resp<Vec<Result<EchoResponse, Stat>>>, Stat>>
    for Vec<Result<EchoResponse, tonic::Status>>
{
    fn into(self) -> Result<Resp<Vec<Result<EchoResponse, Stat>>>, Stat> {
        let stream = self
            .into_iter()
            .map(|message| message.map_err(Stat::from))
            .collect();
        Ok(Resp::new(stream))
    }
}

impl IntoResp<Result<Resp<EchoResponse>, Stat>> for tonic::Status {
    fn into(self) -> Result<Resp<EchoResponse>, Stat> {
        Err(Stat::from(self))
    }
}

impl IntoResp<Result<Resp<Vec<Result<EchoResponse, Stat>>>, Stat>> for tonic::Status {
    fn into(self) -> Result<Resp<Vec<Result<EchoResponse, Stat>>>, Stat> {
        Err(Stat::from(self))
    }
}

pub(crate) fn make_resp<O, T: IntoResp<O> + CalcSize>(v: T) -> (O, f64) {
    let size = v.size();
    let resp = v.into();
    (resp, size as _)
}

/// Convert a tonic status [`Code`]'s name/description to a value compatible
/// with the metric asserts.
pub(crate) fn sn(code: Code) -> otel_asserts::types::common::Value {
    let name = match code {
        Code::Ok => "OK",
        Code::Cancelled => "CANCELLED",
        Code::Unknown => "UNKNOWN",
        Code::InvalidArgument => "INVALID_ARGUMENT",
        Code::DeadlineExceeded => "DEADLINE_EXCEEDED",
        Code::NotFound => "NOT_FOUND",
        Code::AlreadyExists => "ALREADY_EXISTS",
        Code::PermissionDenied => "PERMISSION_DENIED",
        Code::ResourceExhausted => "RESOURCE_EXHAUSTED",
        Code::FailedPrecondition => "FAILED_PRECONDITION",
        Code::Aborted => "ABORTED",
        Code::OutOfRange => "OUT_OF_RANGE",
        Code::Unimplemented => "UNIMPLEMENTED",
        Code::Internal => "INTERNAL",
        Code::Unavailable => "UNAVAILABLE",
        Code::DataLoss => "DATA_LOSS",
        Code::Unauthenticated => "UNAUTHENTICATED",
    };
    otel_asserts::types::common::Value::String(name.to_string())
}

/// [`tonic::Response`] does not implement [`PartialEq`].
///
/// Currently, this doesn't track metadata, but it could.
#[derive(Debug)]
pub(crate) struct Resp<T> {
    inner: T,
}

impl<T> Resp<T> {
    pub(crate) fn new(inner: T) -> Self {
        Self { inner }
    }
}

impl<T: PartialEq> PartialEq<Resp<T>> for Resp<T> {
    fn eq(&self, other: &Resp<T>) -> bool {
        self.inner.eq(&other.inner)
    }
}

/// [`tonic::Status`] does not implement [`PartialEq`].
///
/// Currently, this doesn't track metadata or details, but it could - just be
/// careful around how PartialEq is implemented (see [`Resp`]).
#[derive(Debug, PartialEq)]
pub(crate) struct Stat {
    code: tonic::Code,
    message: String,
}

impl Stat {
    pub(crate) fn code(&self) -> Code {
        self.code
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }
}

impl From<tonic::Status> for Stat {
    fn from(value: tonic::Status) -> Self {
        Self {
            code: value.code(),
            message: value.message().to_string(),
        }
    }
}

#[async_trait]
pub(crate) trait ConsumeResponse {
    type Output;

    async fn consume(self) -> ::std::result::Result<Self::Output, Stat>;
}

#[async_trait]
impl ConsumeResponse for Result<tonic::Response<EchoResponse>, tonic::Status> {
    type Output = Resp<EchoResponse>;

    async fn consume(self) -> Result<Self::Output, Stat> {
        match self {
            Ok(resp) => {
                let (_metadata, inner, _extensions) = resp.into_parts();
                Ok(Resp { inner })
            }
            Err(e) => Err(Stat::from(e)),
        }
    }
}

#[async_trait]
impl ConsumeResponse for Result<tonic::Response<tonic::Streaming<EchoResponse>>, tonic::Status> {
    type Output = Resp<Vec<Result<EchoResponse, Stat>>>;

    async fn consume(self) -> std::result::Result<Self::Output, Stat> {
        match self {
            Ok(resp) => {
                use futures::StreamExt as _;
                use tracing::Instrument as _;

                let (_metadata, stream, _extensions) = resp.into_parts();

                let inner = stream
                    .map(|item| item.map_err(Stat::from))
                    .collect::<Vec<Result<EchoResponse, Stat>>>()
                    .instrument(tracing::info_span!("streaming (client)"))
                    .await;

                Ok(Resp { inner })
            }
            Err(e) => Err(Stat::from(e)),
        }
    }
}

pub(crate) fn get_histogram_sum(metrics: &testing::output::OutputMetrics<'_>, name: &str) -> f64 {
    metrics
        .raw
        .iter()
        .find(|m| m.name == name)
        .and_then(|m| m.data.as_ref())
        .and_then(|data| match data {
            otel_asserts::types::metrics::Data::Histogram(h) => h.data_points.first(),
            _ => None,
        })
        .and_then(|dp| dp.sum)
        .unwrap_or(0.0)
}

pub(crate) async fn consume_request(body: tonic::body::Body) {
    let mut body = std::pin::pin!(body);
    use http_body::Body as _;
    let _data = std::future::poll_fn(|cx| body.as_mut().poll_frame(cx)).await;
}
