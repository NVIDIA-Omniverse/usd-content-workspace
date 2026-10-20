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

use crate::harness::utils::rpc_map;
use opentelemetry::metrics::Meter;
use tonic::body::Body;
use tonic::client::GrpcService;
use tonic::codegen::http;
use tonic::transport::{Channel, Uri};
use tonic_observability::client::metric_a66::TonicMetricLayer as A66MetricLayer;
use tonic_observability::client::trace::TonicTraceLayer;
pub(crate) use tower_layer::Identity;
use tower_layer::{Layer, Stack};
use tower_service::Service;

type Request = http::Request<Body>;
type Response<B = Body> = http::Response<B>;
pub(crate) type TestService =
    tower::util::BoxCloneService<Request, Response, tonic::transport::Error>;

/// Wrapper type as `FnOnce()` cannot have `impl Type` signature.
#[derive(Debug)]
pub(crate) struct MakeClient<L> {
    service: Channel,
    uri: Uri,
    builder: tower::ServiceBuilder<L>,
    meter: Meter,
}

impl<L> MakeClient<L> {
    pub(crate) fn meter(&self) -> Meter {
        self.meter.clone()
    }
}

impl MakeClient<Identity> {
    pub(super) async fn new(uri: Uri, meter: Meter) -> Self {
        let endpoint = tonic::transport::Endpoint::from(uri.clone());

        let channel = endpoint
            .connect()
            .await
            .expect("failed to connect to server");

        Self {
            service: channel,
            uri,
            builder: tower::ServiceBuilder::new(),
            meter,
        }
    }

    pub(crate) fn default_layers(
        self,
    ) -> MakeClient<Stack<A66MetricLayer, Stack<TonicTraceLayer, Identity>>> {
        self.trace_layer().metric_layer()
    }

    pub(crate) fn default<C>(self) -> C
    where
        C: From<ClientInfo>,
    {
        self.default_layers().client()
    }
}

impl<L, RespBody> MakeClient<L>
where
    L: Layer<Channel>,
    <L as Layer<Channel>>::Service: GrpcService<Body, ResponseBody = RespBody, Error = tonic::transport::Error>
        + Service<Request, Response = Response<RespBody>, Error = tonic::transport::Error>
        + Clone
        + Send
        + 'static,
    RespBody: http_body::Body<Data = bytes::Bytes, Error = tonic::Status> + Send + 'static,
    <<L as Layer<Channel>>::Service as Service<Request>>::Future: Send,
{
    pub(crate) fn trace_layer(self) -> MakeClient<Stack<TonicTraceLayer, L>> {
        let layer = TonicTraceLayer::builder().build();
        self.layer(layer)
    }

    pub(crate) fn metric_layer(self) -> MakeClient<Stack<A66MetricLayer, L>> {
        let layer = A66MetricLayer::builder(self.meter.clone())
            .with_rpc_map(rpc_map())
            .build();
        self.layer(layer)
    }

    pub(crate) fn layer<Inner: Layer<Channel>>(self, layer: Inner) -> MakeClient<Stack<Inner, L>> {
        let Self {
            service,
            uri,
            builder,
            meter,
        } = self;
        let builder = builder.layer(layer);
        MakeClient {
            service,
            uri,
            builder,
            meter,
        }
    }

    pub(crate) fn build(self) -> ClientInfo {
        let Self {
            service,
            uri,
            builder,
            meter: _,
        } = self;

        let service = builder.service(service);
        // hide/erase the exact response type
        use tower::ServiceExt as _;
        let service = TestService::new(service.map_response(|r| r.map(Body::new)));

        ClientInfo {
            service,
            origin: uri,
        }
    }

    pub(crate) fn client<C>(self) -> C
    where
        C: From<ClientInfo>,
    {
        C::from(self.build())
    }
}

#[derive(Debug)]
pub(crate) struct ClientInfo {
    pub(crate) service: TestService,
    pub(crate) origin: Uri,
}

/// Clients don't implement a trait, so we must implement `From<ClientInfo>` for
/// each client for now: <https://github.com/hyperium/tonic/issues/1490>
macro_rules! known_client {
    ($ty:ty) => {
        impl From<ClientInfo> for $ty {
            fn from(info: ClientInfo) -> Self {
                let ClientInfo { service, origin } = info;
                Self::with_origin(service, origin)
            }
        }
    };
}

known_client!(protos::unary_client::UnaryClient<TestService>);
known_client!(protos::client_streaming_client::ClientStreamingClient<TestService>);
known_client!(protos::server_streaming_client::ServerStreamingClient<TestService>);
known_client!(protos::bidi_streaming_client::BidiStreamingClient<TestService>);
known_client!(protos::foo_client::FooClient<TestService>);
