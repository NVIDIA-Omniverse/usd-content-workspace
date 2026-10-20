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

use super::otel_resource;
use opentelemetry::{Array, KeyValue, StringValue, Value as OtelValue};
use opentelemetry_sdk::Resource;
use serde_json::Value;

#[track_caller]
fn assert_otel_eq(value: OtelValue, expected: Value) {
    let k = "foo";
    let resource = Resource::builder_empty()
        .with_attribute(KeyValue::new(k, value))
        .build();
    let actual = otel_resource(&resource, &[]);
    let expected = vec![(k.to_string(), expected)];
    assert_eq!(actual, expected);
}

#[test]
fn otel_filtered() {
    let k = "foo";
    let resource = Resource::builder_empty()
        .with_attribute(KeyValue::new(k, OtelValue::Bool(true)))
        .build();
    let actual = otel_resource(&resource, &[k]);
    let expected = vec![];
    assert_eq!(actual, expected);
}

macro_rules! from {
    ($v:expr) => {
        let value = OtelValue::from($v);
        let expected = Value::from($v);
        assert_otel_eq(value, expected);
    };
}

#[test]
fn otel_bool() {
    from!(true);
}

#[test]
fn otel_f64_min() {
    from!(f64::MIN);
}

#[test]
fn otel_f64_max() {
    from!(f64::MAX);
}

#[test]
fn otel_i64_min() {
    from!(i64::MIN);
}

#[test]
fn otel_i64_max() {
    from!(i64::MAX);
}

#[test]
fn otel_str() {
    from!("baz");
}

macro_rules! from_array {
    ($ty:path, $($v:expr),+ $(,)?) => {
        let value = OtelValue::Array($ty(vec![$($v),+]));
        let expected = Value::Array(vec![$(Value::from($v)),+]);
        assert_otel_eq(value, expected);
    };
}

#[test]
fn otel_array_bool() {
    from_array!(Array::Bool, true, false, true);
}

#[test]
fn otel_array_f64() {
    from_array!(Array::F64, 0f64, 1f64, 2f64);
}

#[test]
fn otel_array_i64() {
    from_array!(Array::I64, 0i64, 1i64, 2i64);
}

#[test]
fn otel_array_str() {
    let value = OtelValue::Array(Array::String(vec![
        StringValue::from("foo"),
        StringValue::from("bar"),
        StringValue::from("baz"),
    ]));
    let expected = Value::Array(vec![
        Value::from("foo"),
        Value::from("bar"),
        Value::from("baz"),
    ]);
    assert_otel_eq(value, expected);
}
