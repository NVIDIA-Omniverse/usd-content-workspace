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

use super::{Resource, ResourceBuilder, ResourceMut, default_resource};
use crate::testing::unsafe_env;
use std::collections::{HashMap, HashSet};

macro_rules! assert_resource {
    ($resource:ident == { $($k:tt : $v:literal),* $(,)? }) => {
        assert_resource(&$resource, &[
            $((assert_resource!(@key $k), $v),)*
        ]);
    };
    (@key $k:ident) => { ::opentelemetry_semantic_conventions::resource::$k };
    (@key $k:literal) => { $k };
}

#[test]
fn resource_mut_debug() {
    let mut r = ResourceMut::new();
    r.insert("foo", "bar".into());
    assert_eq!(format!("{r:?}"), r#"{"foo": Static("bar")}"#);

    let mut r = ResourceMut::new();
    r.insert("foo", "bar".to_string().into());
    assert_eq!(format!("{r:?}"), r#"{"foo": Owned("bar")}"#);
}

#[test]
fn sdk_resource_default_merge_behavior() {
    // SAFETY: only safe due to `cargo nextest`/per-process isolation
    unsafe_env! {
        "OTEL_SERVICE_NAME": "foo",
    }

    let resource = Resource::builder().with_service_name("nv.ov.test").build();
    assert_resource!(resource == {
        SERVICE_NAME: "nv.ov.test",
        TELEMETRY_SDK_LANGUAGE: "rust",
        TELEMETRY_SDK_NAME: "opentelemetry",
        TELEMETRY_SDK_VERSION: "*",
    });

    let resource = ResourceBuilder::new("nv.ov.test").build();
    assert_resource!(resource == {
        SERVICE_NAME: "nv.ov.test",
        TELEMETRY_SDK_LANGUAGE: "rust",
        TELEMETRY_SDK_NAME: "opentelemetry",
        TELEMETRY_SDK_VERSION: "*",
    });
}

#[test]
fn default_resource_env_empty() {
    // SAFETY: only safe due to `cargo nextest`/per-process isolation
    unsafe_env! {}

    let resource = default_resource();
    assert_resource!(resource == {
        SERVICE_NAME: "unknown_service",
        TELEMETRY_SDK_LANGUAGE: "rust",
        TELEMETRY_SDK_NAME: "opentelemetry",
        TELEMETRY_SDK_VERSION: "*",
    });
}

#[test]
fn resource_builder_env_empty() {
    // SAFETY: only safe due to `cargo nextest`/per-process isolation
    unsafe_env! {}

    let resource = ResourceBuilder::new("nv.ov.test").build();
    assert_resource!(resource == {
        SERVICE_NAME: "nv.ov.test",
        TELEMETRY_SDK_LANGUAGE: "rust",
        TELEMETRY_SDK_NAME: "opentelemetry",
        TELEMETRY_SDK_VERSION: "*",
    });
}

#[test]
fn resource_builder_env_service_name_ignored() {
    // SAFETY: only safe due to `cargo nextest`/per-process isolation
    unsafe_env! {
        "OTEL_SERVICE_NAME": "foo",
    }

    let resource = ResourceBuilder::new("nv.ov.test").build();
    assert_resource!(resource == {
        SERVICE_NAME: "nv.ov.test",
        TELEMETRY_SDK_LANGUAGE: "rust",
        TELEMETRY_SDK_NAME: "opentelemetry",
        TELEMETRY_SDK_VERSION: "*",
    });
}

#[test]
fn resource_builder_env_resource_attributes() {
    // SAFETY: only safe due to `cargo nextest`/per-process isolation
    unsafe_env! {
        "OTEL_RESOURCE_ATTRIBUTES": "foo=bar,spam=eggs",
    }

    let resource = ResourceBuilder::new("nv.ov.test").build();
    assert_resource!(resource == {
        SERVICE_NAME: "nv.ov.test",
        TELEMETRY_SDK_LANGUAGE: "rust",
        TELEMETRY_SDK_NAME: "opentelemetry",
        TELEMETRY_SDK_VERSION: "*",
        "foo": "bar",
        "spam": "eggs",
    });
}

#[test]
fn resource_builder_with_default_empty() {
    // SAFETY: only safe due to `cargo nextest`/per-process isolation
    unsafe_env! {
        "OTEL_RESOURCE_ATTRIBUTES": "foo=bar,spam=eggs",
    }

    let resource = ResourceBuilder::new("nv.ov.test")
        .with_service_namespace("nv.ov")
        .with_service_version("0.1.0")
        .with_default(Resource::builder_empty().build())
        .build();

    assert_resource!(resource == {
        SERVICE_NAME: "nv.ov.test",
        SERVICE_NAMESPACE: "nv.ov",
        SERVICE_VERSION: "0.1.0",
    });
}

// --- helpers

fn resource_to_kv(resource: &Resource) -> HashMap<&str, &str> {
    let mut kvs: HashMap<&str, &str> = HashMap::new();
    for (k, v) in resource.iter() {
        let k: &str = k.as_str();
        let v = match v {
            opentelemetry::Value::String(s) => s.as_str(),
            _ => panic!("expected `{k}` to be a string, but was: {v:?}"),
        };
        if let Some(o) = kvs.insert(k, v) {
            panic!("duplicate key `{k}`: new: `{v}`, old: `{o}`");
        }
    }
    kvs
}

fn macro_args_to_kv(args: &[(&'static str, &'static str)]) -> HashMap<&'static str, &'static str> {
    let mut kvs: HashMap<&str, &str> = HashMap::new();
    for (k, v) in args.iter().copied() {
        if let Some(o) = kvs.insert(k, v) {
            panic!("duplicate key `{k}`: new: `{v}`, old: `{o}`");
        }
    }
    kvs
}

#[track_caller]
fn assert_resource(resource: &Resource, args: &[(&'static str, &'static str)]) {
    let expected = macro_args_to_kv(args);
    let actual = resource_to_kv(resource);

    let mut keys = expected
        .keys()
        .copied()
        .chain(actual.keys().copied())
        .collect::<HashSet<&str>>()
        .into_iter()
        .collect::<Vec<&str>>();
    keys.sort();

    for k in keys {
        let Some(expected_val) = expected.get(k).copied() else {
            panic!("resource has extra key `{k}`: {actual:?}");
        };
        let Some(actual_val) = actual.get(k).copied() else {
            panic!("resource is missing key `{k}`: {actual:?}");
        };
        if expected_val == "*" {
            continue;
        }
        assert_eq!(expected_val, actual_val, "key `{k}`");
    }
}
