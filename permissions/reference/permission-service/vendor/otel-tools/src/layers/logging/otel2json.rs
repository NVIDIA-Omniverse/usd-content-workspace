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

use opentelemetry::{Array, StringValue, Value as OtelValue};
use serde_json::Value;

fn otel_str(v: &StringValue) -> Value {
    Value::String(v.as_str().to_string())
}

fn otel_array(array: &Array) -> Option<Value> {
    match array {
        Array::Bool(a) => {
            let v = a.iter().copied().map(Value::Bool).collect();
            Some(Value::Array(v))
        }
        Array::F64(a) => {
            let v = a.iter().copied().map(Value::from).collect();
            Some(Value::Array(v))
        }
        Array::I64(a) => {
            let v = a.iter().copied().map(Value::from).collect();
            Some(Value::Array(v))
        }
        Array::String(a) => {
            let v = a.iter().map(otel_str).collect();
            Some(Value::Array(v))
        }
        _ => None, // cov-exclude [line]: enum is marked as non_exhaustive
    }
}

fn otel_value(v: &OtelValue) -> Option<Value> {
    match v {
        OtelValue::Array(array) => otel_array(array),
        OtelValue::Bool(v) => Some(Value::Bool(*v)),
        OtelValue::F64(v) => Some(Value::from(*v)),
        OtelValue::I64(v) => Some(Value::from(*v)),
        OtelValue::String(v) => Some(otel_str(v)),
        _ => None, // cov-exclude [line]: enum is marked as non_exhaustive
    }
}

pub(crate) fn otel_resource(
    resource: &opentelemetry_sdk::Resource,
    reserved: &[&str],
) -> Vec<(String, Value)> {
    resource
        .iter()
        .filter_map(|(k, v)| {
            otel_value(v).and_then(|v| {
                let k = k.as_str();
                // do not allow resources to override reserved keys
                if reserved.contains(&k) {
                    None
                } else {
                    Some((k.to_string(), v))
                }
            })
        })
        .collect()
}

#[cfg(test)]
mod tests;
