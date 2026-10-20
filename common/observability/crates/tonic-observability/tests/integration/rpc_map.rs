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

use crate::assert_matches;
use prost_types::{
    FileDescriptorProto, FileDescriptorSet, MethodDescriptorProto, ServiceDescriptorProto,
};
use protos::FILE_DESCRIPTOR_SET;
use std::error::Error as _;
use tonic_observability::rpc_map::{RpcMap, RpcMapBuildError, RpcMapOptimize};

#[test]
fn duplicate_fds() {
    let rpc_map1 = RpcMap::builder()
        .add_encoded_file_descriptor_set(FILE_DESCRIPTOR_SET)
        .unwrap()
        .build();

    let rpc_map2 = RpcMap::builder()
        .add_encoded_file_descriptor_set(FILE_DESCRIPTOR_SET)
        .unwrap()
        .add_encoded_file_descriptor_set(FILE_DESCRIPTOR_SET)
        .unwrap()
        .build();

    assert_eq!(rpc_map1, rpc_map2);
}

#[test]
fn invalid_encoded_fds() {
    let res = RpcMap::builder().add_encoded_file_descriptor_set(&[0xff]);
    assert_matches!(res, Err(RpcMapBuildError::Decode(_)));

    let e = res.unwrap_err();
    assert_matches!(e.source(), Some(_));

    let outer = format!("{e}");
    let inner = format!("{}", e.source().unwrap());
    assert_eq!(outer, inner);
}

#[test]
fn missing_file_name() {
    let file_descriptor_set = FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: None,
            package: Some("com.example".into()),
            ..Default::default()
        }],
    };
    let res = RpcMap::builder().add_file_descriptor_set(file_descriptor_set);
    assert_matches!(res, Err(RpcMapBuildError::MissingFileName));

    let e = res.unwrap_err();
    assert_matches!(e.source(), None);

    assert_eq!(format!("{e}"), "file descriptor has no name");
}

#[test]
fn missing_service_name() {
    let file_descriptor_set = FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("foo.proto".into()),
            package: Some("com.example".into()),
            service: vec![ServiceDescriptorProto {
                name: None,
                ..Default::default()
            }],
            ..Default::default()
        }],
    };
    let res = RpcMap::builder().add_file_descriptor_set(file_descriptor_set);
    assert_matches!(res, Err(RpcMapBuildError::MissingServiceName));

    let e = res.unwrap_err();
    assert_matches!(e.source(), None);

    assert_eq!(format!("{e}"), "service has no name");
}

#[test]
fn missing_method_name() {
    let file_descriptor_set = FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("foo.proto".into()),
            package: Some("com.example".into()),
            service: vec![ServiceDescriptorProto {
                name: Some("Greeter".into()),
                method: vec![MethodDescriptorProto {
                    name: None,
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
    };
    let res = RpcMap::builder().add_file_descriptor_set(file_descriptor_set);
    assert_matches!(res, Err(RpcMapBuildError::MissingMethodName));

    let e = res.unwrap_err();
    assert_matches!(e.source(), None);

    assert_eq!(format!("{e}"), "service method has no name");
}

#[test]
fn duplicate_method_name() {
    let file_descriptor_set = FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("foo.proto".into()),
            package: Some("com.example".into()),
            service: vec![ServiceDescriptorProto {
                name: Some("Greeter".into()),
                method: vec![
                    MethodDescriptorProto {
                        name: Some("SayHello".into()),
                        ..Default::default()
                    },
                    MethodDescriptorProto {
                        name: Some("SayHello".into()),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        }],
    };
    let res = RpcMap::builder().add_file_descriptor_set(file_descriptor_set);
    assert_matches!(res, Err(RpcMapBuildError::DuplicateMethodName));

    let e = res.unwrap_err();
    assert_matches!(e.source(), None);

    assert_eq!(format!("{e}"), "duplicate method name");
}

#[test]
fn missing_package_name() {
    let file_descriptor_set = FileDescriptorSet {
        file: vec![FileDescriptorProto {
            name: Some("foo.proto".into()),
            package: None,
            service: vec![ServiceDescriptorProto {
                name: Some("Greeter".into()),
                method: vec![MethodDescriptorProto {
                    name: Some("SayHello".into()),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
    };
    let _rpc_map = RpcMap::builder()
        .add_file_descriptor_set(file_descriptor_set)
        .unwrap()
        .build();
}

#[test]
fn all_strategies_are_eq() {
    // this is the only code-path that tests ref-counted
    let rpc_map1 = RpcMap::builder()
        .optimize(RpcMapOptimize::OWNED)
        .add_encoded_file_descriptor_set(FILE_DESCRIPTOR_SET)
        .unwrap()
        .build();

    let rpc_map2 = RpcMap::builder()
        .optimize(RpcMapOptimize::STATIC)
        .add_encoded_file_descriptor_set(FILE_DESCRIPTOR_SET)
        .unwrap()
        .build();

    let rpc_map3 = RpcMap::builder()
        .optimize(RpcMapOptimize::REF_COUNTED)
        .add_encoded_file_descriptor_set(FILE_DESCRIPTOR_SET)
        .unwrap()
        .build();

    assert_eq!(rpc_map1, rpc_map2);
    assert_eq!(rpc_map1, rpc_map3);
}
