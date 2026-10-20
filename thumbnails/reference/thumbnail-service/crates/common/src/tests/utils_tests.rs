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

use crate::utils::*;
use prost_types::value::Kind;
use serde_json::json;

#[test]
fn test_json_to_prost_value_null() {
    let json_val = serde_json::Value::Null;
    let result = json_to_prost_value(&json_val).unwrap();

    assert!(matches!(result.kind, Some(Kind::NullValue(0))));
}

#[test]
fn test_json_to_prost_value_bool() {
    let json_true = json!(true);
    let result_true = json_to_prost_value(&json_true).unwrap();
    assert!(matches!(result_true.kind, Some(Kind::BoolValue(true))));

    let json_false = json!(false);
    let result_false = json_to_prost_value(&json_false).unwrap();
    assert!(matches!(result_false.kind, Some(Kind::BoolValue(false))));
}

#[test]
fn test_json_to_prost_value_small_integers() {
    let json_int = json!(42);
    let result = json_to_prost_value(&json_int).unwrap();

    if let Some(Kind::NumberValue(val)) = result.kind {
        assert_eq!(val, 42.0);
    } else {
        panic!("Expected NumberValue");
    }
}

#[test]
fn test_json_to_prost_value_large_integers() {
    // Test large integer that should be stored as string to preserve precision
    let large_int = 9007199254740993i64; // Beyond f64 precision
    let json_val = json!(large_int);
    let result = json_to_prost_value(&json_val).unwrap();

    if let Some(Kind::StringValue(val)) = result.kind {
        assert_eq!(val, large_int.to_string());
    } else {
        panic!("Expected StringValue for large integer");
    }
}

#[test]
fn test_json_to_prost_value_float() {
    let json_float = json!(std::f64::consts::PI);
    let result = json_to_prost_value(&json_float).unwrap();

    if let Some(Kind::NumberValue(val)) = result.kind {
        assert!((val - std::f64::consts::PI).abs() < f64::EPSILON);
    } else {
        panic!("Expected NumberValue");
    }
}

#[test]
fn test_json_to_prost_value_string() {
    let json_str = json!("hello world");
    let result = json_to_prost_value(&json_str).unwrap();

    if let Some(Kind::StringValue(val)) = result.kind {
        assert_eq!(val, "hello world");
    } else {
        panic!("Expected StringValue");
    }
}

#[test]
fn test_json_to_prost_value_array() {
    let json_array = json!([1, "hello", true, null]);
    let result = json_to_prost_value(&json_array).unwrap();

    if let Some(Kind::ListValue(list)) = result.kind {
        assert_eq!(list.values.len(), 4);

        // Check first element (number)
        if let Some(Kind::NumberValue(val)) = &list.values[0].kind {
            assert_eq!(*val, 1.0);
        } else {
            panic!("Expected NumberValue at index 0");
        }

        // Check second element (string)
        if let Some(Kind::StringValue(val)) = &list.values[1].kind {
            assert_eq!(val, "hello");
        } else {
            panic!("Expected StringValue at index 1");
        }

        // Check third element (bool)
        if let Some(Kind::BoolValue(val)) = &list.values[2].kind {
            assert!(*val);
        } else {
            panic!("Expected BoolValue at index 2");
        }

        // Check fourth element (null)
        assert!(matches!(list.values[3].kind, Some(Kind::NullValue(0))));
    } else {
        panic!("Expected ListValue");
    }
}

#[test]
fn test_json_to_prost_value_object() {
    let json_obj = json!({
        "name": "test",
        "age": 25,
        "active": true,
        "data": null
    });
    let result = json_to_prost_value(&json_obj).unwrap();

    if let Some(Kind::StructValue(struct_val)) = result.kind {
        assert_eq!(struct_val.fields.len(), 4);

        // Check name field
        if let Some(Kind::StringValue(val)) = &struct_val.fields.get("name").unwrap().kind {
            assert_eq!(val, "test");
        } else {
            panic!("Expected StringValue for name");
        }

        // Check age field
        if let Some(Kind::NumberValue(val)) = &struct_val.fields.get("age").unwrap().kind {
            assert_eq!(*val, 25.0);
        } else {
            panic!("Expected NumberValue for age");
        }

        // Check active field
        if let Some(Kind::BoolValue(val)) = &struct_val.fields.get("active").unwrap().kind {
            assert!(*val);
        } else {
            panic!("Expected BoolValue for active");
        }

        // Check data field (null)
        assert!(matches!(
            struct_val.fields.get("data").unwrap().kind,
            Some(Kind::NullValue(0))
        ));
    } else {
        panic!("Expected StructValue");
    }
}

#[test]
fn test_json_to_prost_value_nested_structure() {
    let json_nested = json!({
        "user": {
            "name": "Alice",
            "hobbies": ["reading", "coding"],
            "stats": {
                "posts": 42,
                "active": true
            }
        }
    });
    let result = json_to_prost_value(&json_nested).unwrap();

    if let Some(Kind::StructValue(struct_val)) = result.kind {
        assert!(struct_val.fields.contains_key("user"));

        // Check nested user object
        if let Some(Kind::StructValue(user_struct)) = &struct_val.fields.get("user").unwrap().kind {
            assert!(user_struct.fields.contains_key("name"));
            assert!(user_struct.fields.contains_key("hobbies"));
            assert!(user_struct.fields.contains_key("stats"));
        } else {
            panic!("Expected nested StructValue for user");
        }
    } else {
        panic!("Expected StructValue");
    }
}

#[test]
fn test_convert_task_status_to_proto_status_pending() {
    assert_eq!(convert_task_status_to_proto_status("pending"), 1);
    assert_eq!(convert_task_status_to_proto_status("PENDING"), 1);
    assert_eq!(convert_task_status_to_proto_status("Pending"), 1);
}

#[test]
fn test_convert_task_status_to_proto_status_running() {
    assert_eq!(convert_task_status_to_proto_status("running"), 2);
    assert_eq!(convert_task_status_to_proto_status("started"), 2);
    assert_eq!(convert_task_status_to_proto_status("RUNNING"), 2);
    assert_eq!(convert_task_status_to_proto_status("STARTED"), 2);
}

#[test]
fn test_convert_task_status_to_proto_status_completed() {
    assert_eq!(convert_task_status_to_proto_status("completed"), 4);
    assert_eq!(convert_task_status_to_proto_status("success"), 4);
    assert_eq!(convert_task_status_to_proto_status("COMPLETED"), 4);
    assert_eq!(convert_task_status_to_proto_status("SUCCESS"), 4);
}

#[test]
fn test_convert_task_status_to_proto_status_canceled() {
    assert_eq!(convert_task_status_to_proto_status("canceled"), 5);
    assert_eq!(convert_task_status_to_proto_status("cancelled"), 5);
    assert_eq!(convert_task_status_to_proto_status("CANCELED"), 5);
    assert_eq!(convert_task_status_to_proto_status("CANCELLED"), 5);
}

#[test]
fn test_convert_task_status_to_proto_status_failed() {
    assert_eq!(convert_task_status_to_proto_status("failed"), 6);
    assert_eq!(convert_task_status_to_proto_status("failure"), 6);
    assert_eq!(convert_task_status_to_proto_status("FAILED"), 6);
    assert_eq!(convert_task_status_to_proto_status("FAILURE"), 6);
}

#[test]
fn test_convert_task_status_to_proto_status_unknown() {
    // Unknown statuses should default to Pending (1)
    assert_eq!(convert_task_status_to_proto_status("unknown"), 1);
    assert_eq!(convert_task_status_to_proto_status("invalid"), 1);
    assert_eq!(convert_task_status_to_proto_status(""), 1);
    assert_eq!(convert_task_status_to_proto_status("xyz"), 1);
}

#[test]
fn test_convert_task_status_mixed_case() {
    assert_eq!(convert_task_status_to_proto_status("PeNdInG"), 1);
    assert_eq!(convert_task_status_to_proto_status("RuNnInG"), 2);
    assert_eq!(convert_task_status_to_proto_status("CoMpLeTeD"), 4);
    assert_eq!(convert_task_status_to_proto_status("CaNcElEd"), 5);
    assert_eq!(convert_task_status_to_proto_status("FaIlEd"), 6);
}
