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

mod access_log;
mod authorization;
mod capabilities;
pub mod interceptors;
mod metrics;

use crate::grpc::access_log::GrpcAccessLogLayer;
use crate::grpc::authorization::PermissionServiceImpl;
use crate::grpc::capabilities::CapabilitiesServiceImpl;
use crate::grpc::interceptors::auth::AuthInterceptor;
use crate::grpc::metrics::GrpcMetricsLayer;
use crate::state::AppState;
use axum::http::Request;
use clap::Args;
use opentelemetry::trace::TraceContextExt;
use opentelemetry_http::HeaderExtractor;
use permission_service_capabilities_proto::nvidia::omniverse::discovery::capabilities::v2alpha::capabilities_service_server::CapabilitiesServiceServer;
use permission_service_proto::nvidia::omniverse::permission::v1beta::permission_service_server::PermissionServiceServer;
use std::error::Error;
use tokio::net::TcpListener;
use tonic::codegen::tokio_stream;
use tonic::server::NamedService;
use tonic::transport::Server;
use tonic_async_interceptor::async_interceptor;
use tonic_health::server::health_reporter;
use tower::ServiceBuilder;
use tower_http::trace::TraceLayer;
use tracing::{field, info_span, Span};
use tracing_opentelemetry::OpenTelemetrySpanExt;

#[derive(Args, Clone, Debug)]
pub struct GrpcArgs {
    #[arg(
        long,
        env = "TONIC_INITIAL_CONNECTION_WINDOW_SIZE",
        default_value_t = 65535
    )]
    pub tonic_initial_connection_window_size: u32,

    #[arg(
        long,
        env = "TONIC_INITIAL_STREAM_WINDOW_SIZE",
        default_value_t = 65535
    )]
    pub tonic_initial_stream_window_size: u32,

    #[arg(long, env = "TONIC_MAX_DECODING_MESSAGE_SIZE", default_value_t = 4 * 1024 * 1024)]
    pub tonic_max_decoding_message_size: usize,
}

pub async fn start_grpc_service(state: AppState) -> Result<(), Box<dyn Error + Sync + Send>> {
    let (health_reporter, health_service) = health_reporter();
    health_reporter
        .set_serving::<PermissionServiceServer<PermissionServiceImpl>>()
        .await;
    health_reporter
        .set_serving::<CapabilitiesServiceServer<CapabilitiesServiceImpl>>()
        .await;

    let reflection_service_v1 = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(
            permission_service_proto::google::protobuf::FILE_DESCRIPTOR_SET,
        )
        .register_encoded_file_descriptor_set(
            permission_service_proto::nvidia::omniverse::permission::v1beta::FILE_DESCRIPTOR_SET,
        )
        .register_encoded_file_descriptor_set(
            permission_service_capabilities_proto::nvidia::omniverse::discovery::capabilities::v2alpha::FILE_DESCRIPTOR_SET,
        )
        .register_encoded_file_descriptor_set(tonic_health::pb::FILE_DESCRIPTOR_SET)
        .build_v1()?;

    let reflection_service_v1alpha = tonic_reflection::server::Builder::configure()
        .register_encoded_file_descriptor_set(
            permission_service_proto::google::protobuf::FILE_DESCRIPTOR_SET,
        )
        .register_encoded_file_descriptor_set(
            permission_service_proto::nvidia::omniverse::permission::v1beta::FILE_DESCRIPTOR_SET,
        )
        .register_encoded_file_descriptor_set(
            permission_service_capabilities_proto::nvidia::omniverse::discovery::capabilities::v2alpha::FILE_DESCRIPTOR_SET,
        )
        .register_encoded_file_descriptor_set(tonic_health::pb::FILE_DESCRIPTOR_SET)
        .build_v1alpha()?;

    let permission_service = PermissionServiceServer::new(PermissionServiceImpl {
        state: state.clone(),
    })
    .max_decoding_message_size(state.config.grpc.tonic_max_decoding_message_size);

    let capabilities_service = CapabilitiesServiceServer::new(CapabilitiesServiceImpl);

    let auth_interceptor = AuthInterceptor::new(state.clone());

    // Infrastructure and discovery services that must never appear in access logs.
    // Everything else (business RPCs and unknown/invalid paths) is logged.
    let access_log_skip_services = vec![
        "grpc.health.v1.Health".to_string(),
        "grpc.reflection.v1.ServerReflection".to_string(),
        "grpc.reflection.v1alpha.ServerReflection".to_string(),
        <CapabilitiesServiceServer<CapabilitiesServiceImpl> as NamedService>::NAME.to_string(),
    ];

    let grpc = Server::builder()
        .layer(
            ServiceBuilder::new()
                .layer(GrpcMetricsLayer::new(state.metrics.clone()))
                .layer(GrpcAccessLogLayer::new(
                    state
                        .config
                        .access_log
                        .config(state.config.tracing.structured),
                    access_log_skip_services,
                ))
                .layer(TraceLayer::new_for_grpc().make_span_with(make_span))
                .map_request(accept_trace)
                .map_request(record_trace_id),
        )
        .initial_connection_window_size(state.config.grpc.tonic_initial_connection_window_size)
        .initial_stream_window_size(state.config.grpc.tonic_initial_stream_window_size)
        .add_service(health_service)
        .add_service(reflection_service_v1)
        .add_service(reflection_service_v1alpha)
        .add_service(capabilities_service)
        .add_service(
            ServiceBuilder::new()
                .layer(async_interceptor(auth_interceptor.clone()))
                .service(permission_service),
        );

    let listener = TcpListener::bind("0.0.0.0:3010").await?;

    tracing::info!("Listening gRPC on 0.0.0.0:3010");
    grpc.serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
        .await?;
    Ok(())
}

fn make_span<B>(request: &Request<B>) -> Span {
    let path = request.uri().path();
    let span = info_span!(
        "grpc_request",
        otel.name = path,
        otel.kind = "server",
        trace_id = field::Empty
    );
    span
}

pub fn accept_trace<B>(request: Request<B>) -> Request<B> {
    // Current context, if no or invalid data is received.
    let parent_context = opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.extract(&HeaderExtractor(request.headers()))
    });
    let _ = Span::current().set_parent(parent_context);
    request
}

pub fn record_trace_id<B>(request: Request<B>) -> Request<B> {
    let span = Span::current();

    let trace_id = span.context().span().span_context().trace_id();
    span.record("trace_id", trace_id.to_string());

    request
}
