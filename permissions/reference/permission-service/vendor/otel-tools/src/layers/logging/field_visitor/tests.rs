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

use super::FieldVisitor;
use serde_json::Value;
use std::fmt;
use tracing::{Event, Metadata};

struct NullCallsite;
static NULL_CALLSITE: NullCallsite = NullCallsite;

impl tracing::callsite::Callsite for NullCallsite {
    fn set_interest(&self, _: tracing::subscriber::Interest) {
        unreachable!("you somehow managed to register the null callsite?")
    }

    fn metadata(&self) -> &Metadata<'_> {
        unreachable!("you somehow managed to access the null callsite?")
    }
}

static METADATA: Metadata<'static> = Metadata::new(
    "span",
    "test",
    tracing::Level::INFO,
    Some("test.rs"),
    Some(42),
    Some("crates/test/src/test.rs"),
    tracing::field::FieldSet::new(&["foo"], tracing_core::identify_callsite!(&NULL_CALLSITE)),
    tracing::metadata::Kind::EVENT,
);

fn visit(val: &dyn tracing::Value) -> serde_json::Value {
    let field_set = METADATA.fields();
    let field = field_set.field("foo").unwrap();

    let values = [(&field, Some(&val as &dyn tracing::Value))];
    let fields = field_set.value_set(&values);

    let event = Event::new(&METADATA, &fields);

    let mut buffer = Vec::new();
    let mut serializer = serde_json::Serializer::new(&mut buffer);
    use serde::ser::{SerializeMap as _, Serializer as _};
    let mut map_serializer = serializer.serialize_map(None).unwrap();
    let mut visitor = FieldVisitor {
        serializer: &mut map_serializer,
        reserved: &[],
    };
    event.record(&mut visitor);
    map_serializer.end().unwrap();

    let mut values: serde_json::Map<String, Value> = serde_json::from_slice(&buffer).unwrap();

    let val = values.remove("foo").expect("field not recorded");
    if !values.is_empty() {
        panic!("unknown fields recorded: {values:?}");
    }
    val
}

#[test]
fn record_str() {
    let val: &str = "baz";
    let expected = Value::from(val);
    let actual = visit(&val);
    assert_eq!(actual, expected);
}

#[test]
fn record_bool() {
    let val: bool = true;
    let expected = Value::from(val);
    let actual = visit(&val);
    assert_eq!(actual, expected);
}

#[test]
fn record_f64_min() {
    let val: f64 = f64::MIN;
    let expected = Value::from(val);
    let actual = visit(&val);
    assert_eq!(actual, expected);
}

#[test]
fn record_f64_max() {
    let val: f64 = f64::MAX;
    let expected = Value::from(val);
    let actual = visit(&val);
    assert_eq!(actual, expected);
}

#[test]
fn record_i64_min() {
    let val: i64 = i64::MIN;
    let expected = Value::from(val);
    let actual = visit(&val);
    assert_eq!(actual, expected);
}

#[test]
fn record_i64_max() {
    let val: i64 = i64::MAX;
    let expected = Value::from(val);
    let actual = visit(&val);
    assert_eq!(actual, expected);
}

#[test]
fn record_u64_min() {
    let val: u64 = u64::MIN;
    let expected = Value::from(val);
    let actual = visit(&val);
    assert_eq!(actual, expected);
}

#[test]
fn record_u64_max() {
    let val: u64 = u64::MAX;
    let expected = Value::from(val);
    let actual = visit(&val);
    assert_eq!(actual, expected);
}

#[test]
fn record_i128_min() {
    let val: i128 = i128::MIN;
    let expected = Value::from(format!("{val}"));
    let actual = visit(&val);
    assert_eq!(actual, expected);
}

#[test]
fn record_i128_max() {
    let val: i128 = i128::MAX;
    let expected = Value::from(format!("{val}"));
    let actual = visit(&val);
    assert_eq!(actual, expected);
}

#[test]
fn record_u128_min() {
    let val: u128 = u128::MIN;
    let expected = Value::from(format!("{val}"));
    let actual = visit(&val);
    assert_eq!(actual, expected);
}

#[test]
fn record_u128_max() {
    let val: u128 = u128::MAX;
    let expected = Value::from(format!("{val}"));
    let actual = visit(&val);
    assert_eq!(actual, expected);
}

#[derive(Debug)]
struct TestError;

impl fmt::Display for TestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("test error")
    }
}

impl std::error::Error for TestError {}

#[test]
fn record_error() {
    let val: &dyn std::error::Error = &TestError;
    let expected = Value::from("test error");
    let actual = visit(&val);
    assert_eq!(actual, expected);
}

#[test]
fn record_debug() {
    let val = tracing::field::debug(TestError);
    let expected = Value::from("TestError");
    let actual = visit(&val);
    assert_eq!(actual, expected);
}
