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

use prost_types::{value::Kind, ListValue, Struct, Value};
use std::collections::BTreeMap;

pub fn json_to_prost_value(json_value: &serde_json::Value) -> Result<Value, String> {
    let kind = match json_value {
        serde_json::Value::Null => Kind::NullValue(0),
        serde_json::Value::Bool(b) => Kind::BoolValue(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                // Check if the integer can be represented exactly as f64
                let f_val = i as f64;
                if (f_val as i64) == i {
                    Kind::NumberValue(f_val)
                } else {
                    // For large integers that lose precision, store as string
                    Kind::StringValue(i.to_string())
                }
            } else if let Some(u) = n.as_u64() {
                // Check if the unsigned integer can be represented exactly as f64
                let f_val = u as f64;
                if (f_val as u64) == u {
                    Kind::NumberValue(f_val)
                } else {
                    // For large integers that lose precision, store as string
                    Kind::StringValue(u.to_string())
                }
            } else if let Some(f) = n.as_f64() {
                Kind::NumberValue(f)
            } else {
                return Err("Invalid number format".to_string());
            }
        }
        serde_json::Value::String(s) => Kind::StringValue(s.clone()),
        serde_json::Value::Array(arr) => {
            let mut list_values = Vec::new();
            for item in arr {
                list_values.push(json_to_prost_value(item)?);
            }
            Kind::ListValue(ListValue {
                values: list_values,
            })
        }
        serde_json::Value::Object(obj) => {
            let mut fields = BTreeMap::new();
            for (key, value) in obj {
                fields.insert(key.clone(), json_to_prost_value(value)?);
            }
            Kind::StructValue(Struct { fields })
        }
    };

    Ok(Value { kind: Some(kind) })
}

/// Convert prost_types::Value to serde_json::Value
pub fn prost_to_json_value(value: &Value) -> serde_json::Value {
    match &value.kind {
        Some(Kind::StringValue(s)) => serde_json::Value::String(s.clone()),
        Some(Kind::NumberValue(n)) => serde_json::json!(n),
        Some(Kind::BoolValue(b)) => serde_json::Value::Bool(*b),
        Some(Kind::NullValue(_)) => serde_json::Value::Null,
        Some(Kind::StructValue(struct_val)) => {
            let mut map = serde_json::Map::new();
            for (key, field_value) in &struct_val.fields {
                map.insert(key.clone(), prost_to_json_value(field_value));
            }
            serde_json::Value::Object(map)
        }
        Some(Kind::ListValue(list_val)) => {
            let values: Vec<serde_json::Value> =
                list_val.values.iter().map(prost_to_json_value).collect();
            serde_json::Value::Array(values)
        }
        None => serde_json::Value::Null,
    }
}

pub fn convert_task_status_to_proto_status(status: &str) -> i32 {
    match status.to_lowercase().as_str() {
        "pending" => 1,                            // ThumbnailStatus::Pending
        "started" | "running" => 2,                // ThumbnailStatus::Started
        "retry" => 3,                              // ThumbnailStatus::Retry
        "success" | "completed" => 4,              // ThumbnailStatus::Success
        "revoked" | "canceled" | "cancelled" => 5, // ThumbnailStatus::Revoked
        "failed" | "failure" => 6,                 // ThumbnailStatus::Failed
        _ => 1,                                    // Default to Pending
    }
}
