// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

pub mod grpc;
pub mod rest;

use crate::service_wrapper_module;

service_wrapper_module! {
    service_name: "versioning",
    version: "v1beta",
    struct_name: Versioning,
    field_name: versioning,
    trait_path: crate::traits::versioning::v1beta::Versioning as VersioningTrait,
}
