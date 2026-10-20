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

use super::{proto, vec_into};
use std::time::{Duration, SystemTime};

#[cfg_attr(feature = "with-serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone)]
pub enum Value {
    String(String),
    Bool(bool),
    Int(i64),
    Double(f64),
    Array(Vec<Option<Value>>),
    Kvlist(Vec<KeyValue>),
    Bytes(Vec<u8>),
    Wildcard,
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s.as_str()),
            _ => None,
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::String(a), Value::String(b)) => a.eq(b),
            (Value::Bool(a), Value::Bool(b)) => a.eq(b),
            (Value::Int(a), Value::Int(b)) => a.eq(b),
            (Value::Double(a), Value::Double(b)) => a.eq(b),
            (Value::Array(a), Value::Array(b)) => a.eq(b),
            (Value::Kvlist(a), Value::Kvlist(b)) => a.eq(b),
            (Value::Bytes(a), Value::Bytes(b)) => a.eq(b),
            (Value::Wildcard, Value::Wildcard) => false, // something has gone wrong
            (Value::Wildcard, _) => true,
            (_, Value::Wildcard) => true,
            _unused => false,
        }
    }
}

impl From<String> for Value {
    #[inline]
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Self::String(value.to_string())
    }
}

impl From<bool> for Value {
    #[inline]
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<i64> for Value {
    #[inline]
    fn from(value: i64) -> Self {
        Self::Int(value)
    }
}

impl From<f64> for Value {
    #[inline]
    fn from(value: f64) -> Self {
        Self::Double(value)
    }
}

impl From<proto::common::any_value::Value> for Value {
    fn from(value: proto::common::any_value::Value) -> Self {
        use proto::common::any_value::Value;
        match value {
            Value::StringValue(v) => Self::String(v),
            Value::BoolValue(v) => Self::Bool(v),
            Value::IntValue(v) => Self::Int(v),
            Value::DoubleValue(v) => Self::Double(v),
            Value::ArrayValue(v) => {
                let v = v
                    .values
                    .into_iter()
                    .map(|v| v.value.map(Into::into))
                    .collect();
                Self::Array(v)
            }
            Value::KvlistValue(v) => {
                let v = v.values;
                Self::Kvlist(vec_into!(v))
            }
            Value::BytesValue(v) => Self::Bytes(v),
        }
    }
}

impl From<opentelemetry::Value> for Value {
    fn from(value: opentelemetry::Value) -> Self {
        use opentelemetry::{Array, Value};
        match value {
            Value::Bool(v) => Self::Bool(v),
            Value::I64(v) => Self::Int(v),
            Value::F64(v) => Self::Double(v),
            Value::String(v) => Self::from(v.as_str()),
            Value::Array(a) => {
                let vec = match a {
                    Array::Bool(vec) => vec.into_iter().map(|v| Some(Self::from(v))).collect(),
                    Array::I64(vec) => vec.into_iter().map(|v| Some(Self::from(v))).collect(),
                    Array::F64(vec) => vec.into_iter().map(|v| Some(Self::from(v))).collect(),
                    Array::String(vec) => vec
                        .into_iter()
                        .map(|v| Some(Self::from(v.as_str())))
                        .collect(),
                    other => panic!("unknown array: {other:?}"),
                };
                Self::Array(vec)
            }
            other => panic!("unknown value: {other:?}"),
        }
    }
}

#[cfg_attr(feature = "with-serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct KeyValue {
    pub key: String,
    pub value: Option<Value>,
}

impl KeyValue {
    pub fn new(k: &str, v: impl Into<Value>) -> Self {
        Self {
            key: k.to_string(),
            value: Some(v.into()),
        }
    }

    pub fn key(&self) -> &str {
        &self.key
    }
}

impl From<proto::common::KeyValue> for KeyValue {
    fn from(value: proto::common::KeyValue) -> Self {
        let proto::common::KeyValue { key, value } = value;
        let value = value.and_then(|v| v.value).map(Into::into);
        Self { key, value }
    }
}

#[cfg_attr(feature = "with-serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct InstrumentationScope {
    pub name: String,
    pub version: String,
    pub attributes: Vec<KeyValue>,
    pub dropped_attributes_count: u32,
}

impl InstrumentationScope {
    pub fn new(name: &str, version: &str, attrs: Vec<KeyValue>) -> Self {
        Self {
            name: name.to_string(),
            version: version.to_string(),
            attributes: attrs,
            dropped_attributes_count: 0,
        }
    }
}

impl From<proto::common::InstrumentationScope> for InstrumentationScope {
    fn from(value: proto::common::InstrumentationScope) -> Self {
        let proto::common::InstrumentationScope {
            name,
            version,
            attributes,
            dropped_attributes_count,
        } = value;
        let attributes = vec_into!(attributes);
        Self {
            name,
            version,
            attributes,
            dropped_attributes_count,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimestampRange {
    pub start: u64,
    pub end: u64,
}

impl TimestampRange {
    pub fn now() -> Self {
        let now = SystemTime::now();
        let before = now - Duration::from_secs(60 * 5);
        let after = now + Duration::from_secs(60 * 5);

        let start = before
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .try_into()
            .unwrap();
        let end = after
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
            .try_into()
            .unwrap();

        Self { start, end }
    }
}

impl PartialEq<u64> for TimestampRange {
    fn eq(&self, other: &u64) -> bool {
        let value = *other;
        self.start <= value && self.end >= value
    }
}
