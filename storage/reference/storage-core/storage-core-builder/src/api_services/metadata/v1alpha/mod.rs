// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

pub mod grpc;
pub mod rest;

use crate::service_wrapper_module;

service_wrapper_module! {
    service_name: "metadata",
    version: "v1alpha",
    struct_name: Metadata,
    field_name: metadata,
    trait_path: crate::traits::metadata::v1alpha::Metadata as MetadataTrait,
}
