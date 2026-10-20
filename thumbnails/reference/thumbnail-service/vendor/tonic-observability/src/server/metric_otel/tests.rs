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

#![allow(deprecated)]
use super::{
    TonicMetricLayerBuilder, TonicMetricNoopBody, TonicMetricRequestBody, TonicMetricResponseBody,
    TonicRequestMetrics, TonicResponseMetrics,
};
use crate::rpc_map::RpcMap;
use crate::test_support::BogusBody;
use crate::test_support::BogusInstrumentProvider;
use http_body::Body as _;
use prost::Message as _;
use tower_layer::Layer as _;

#[test]
fn builder_with_rpc_map() {
    let rpc_map = RpcMap::builder()
        .add_encoded_file_descriptor_set(&test_file_descriptor_set())
        .unwrap()
        .build();

    let meter = BogusInstrumentProvider::meter();
    let layer = TonicMetricLayerBuilder::new(meter)
        .with_rpc_map(rpc_map.clone())
        .build();
    assert_eq!(layer.rpc_map, rpc_map);
}

#[test]
fn request_body_forwards_to_inner() {
    let rpc = opentelemetry::Value::from("bogus");

    let meter = BogusInstrumentProvider::meter();
    let request_size = meter.u64_histogram("foo").build();
    let requests_per_rpc = meter.u64_histogram("bar").build();
    let active_requests = meter.i64_up_down_counter("baz").build();
    let metrics = TonicRequestMetrics {
        request_size,
        requests_per_rpc,
        active_requests,
    };

    let body = TonicMetricRequestBody {
        inner: tonic::body::Body::new(BogusBody::<tonic::Status>::new()),
        count: 0,
        rpc,
        metrics,
    };

    assert!(!body.is_end_stream());
    assert_eq!(body.size_hint().exact(), Some(42));
}

#[test]
fn response_body_forwards_to_inner() {
    let rpc = opentelemetry::Value::from("bogus");

    let meter = BogusInstrumentProvider::meter();
    let response_size = meter.u64_histogram("foo").build();
    let responses_per_rpc = meter.u64_histogram("bar").build();
    let calls = meter.u64_counter("baz").build();
    let active_methods = meter.i64_up_down_counter("spam").build();
    let active_responses = meter.i64_up_down_counter("eggs").build();
    let metrics = TonicResponseMetrics {
        response_size,
        responses_per_rpc,
        calls,
        active_methods,
        active_responses,
    };

    let body = TonicMetricResponseBody {
        inner: tonic::body::Body::new(BogusBody::<tonic::Status>::new()),
        count: 0,
        rpc,
        metrics,
        code: None,
    };

    assert!(!body.is_end_stream());
    assert_eq!(body.size_hint().exact(), Some(42));
}

#[test]
fn noop_body_forwards_to_inner() {
    let rpc = opentelemetry::Value::from("bogus");

    let meter = BogusInstrumentProvider::meter();
    let active_responses = meter.i64_up_down_counter("foo").build();
    let body = TonicMetricNoopBody {
        inner: BogusBody::<tonic::Status>::new(),
        rpc,
        active_responses,
    };

    assert!(!body.is_end_stream());
    assert_eq!(body.size_hint().exact(), Some(42));

    // usually, we wouldn't test `poll_frame()`, however it is never called
    // use to MaybeEmptyBody.
    let waker = std::task::Waker::noop();
    let mut cx = std::task::Context::from_waker(waker);

    let body = std::pin::pin!(body);
    let res = body.poll_frame(&mut cx);
    assert!(matches!(res, std::task::Poll::Ready(None)), "{res:#?}");
}

/// Encode the one service needed by these tests without a generated helper crate.
fn test_file_descriptor_set() -> Vec<u8> {
    prost_types::FileDescriptorSet {
        file: vec![prost_types::FileDescriptorProto {
            name: Some("test.proto".into()),
            package: Some("test".into()),
            service: vec![prost_types::ServiceDescriptorProto {
                name: Some("Service".into()),
                method: vec![prost_types::MethodDescriptorProto {
                    name: Some("Known".into()),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
    }
    .encode_to_vec()
}

#[test]
fn rpc_metric_labels_preserve_known_methods_and_bound_unknown_paths() {
    let rpc_map = RpcMap::builder()
        .add_encoded_file_descriptor_set(&test_file_descriptor_set())
        .unwrap()
        .build();
    let service = TonicMetricLayerBuilder::new(BogusInstrumentProvider::meter())
        .with_rpc_map(rpc_map)
        .build()
        .layer(());

    for known in ["/test.Service/Known", "test.Service/Known"] {
        assert_eq!(
            service.rpc_from_request_path(known),
            opentelemetry::Value::from("test.Service/Known")
        );
    }
    for index in 0..100 {
        assert_eq!(
            service.rpc_from_request_path(&format!("/unknown.Service/Method{index}")),
            opentelemetry::Value::from("_OTHER")
        );
    }
}

#[test]
fn rpc_metric_labels_are_bounded_without_a_configured_map() {
    let service = TonicMetricLayerBuilder::new(BogusInstrumentProvider::meter())
        .build()
        .layer(());
    for path in ["/test.Service/Known", "/unknown.Service/Other", "/", ""] {
        assert_eq!(
            service.rpc_from_request_path(path),
            opentelemetry::Value::from("_OTHER")
        );
    }
}
