// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

pub mod grpc;
pub mod rest;
pub mod types;
mod utils;

use crate::{composite_service_builder, service_wrapper_module};

service_wrapper_module! {
    service_name: "fileobject",
    version: "v1beta",
    struct_name: FileObject,
    field_name: file_object,
    trait_path: crate::traits::fileobject::v1beta::FileObject as FileObjectTrait,
}

composite_service_builder! {
    composite_trait: FileObjectFileFolder,
    builder: BoxFileObjectFileFolderBuilder,
    alias: BoxFileObjectFileFolder,
    erased_layer: BoxFileObjectFileFolderErasedLayer,
    erased_layer_impl: BoxFileObjectFileFolderErasedLayerImpl,
    parts: BoxFileObjectFileFolderParts,
    output: FileObject,
    boxed_output: BoxFileObject,
    primary_trait: FileObjectTrait,
    secondary_trait: crate::traits::filefolder::v1beta::FileFolder as FileFolderTrait,
}
