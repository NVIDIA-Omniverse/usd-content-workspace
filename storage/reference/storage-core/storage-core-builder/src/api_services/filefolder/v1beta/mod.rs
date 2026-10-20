// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

pub mod grpc;
pub mod rest;

use crate::service_wrapper_module;

service_wrapper_module! {
    service_name: "filefolder",
    version: "v1beta",
    struct_name: FileFolder,
    field_name: file_folder,
    trait_path: crate::traits::filefolder::v1beta::FileFolder as FileFolderTrait,
}
