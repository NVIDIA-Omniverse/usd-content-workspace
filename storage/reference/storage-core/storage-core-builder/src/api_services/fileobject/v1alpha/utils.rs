// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use crate::traits::fileobject::v1alpha::{Metadata, ResourceIdentity, ResourceInfo};

use http::{response::Builder, HeaderName, StatusCode};
use storage_core_util::error_limited;

pub fn parse_resource_info(resource_info: ResourceInfo) -> Result<(Metadata, ResourceIdentity), StatusCode> {
    let Some(resource_identity) = resource_info.resource_identity else {
        error_limited!(?resource_info, "ResourceIdentity is not present in the ResourceInfo");
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    };
    // `Metadata` is no longer `Copy` (it carries native_metadata), so move it out last
    // to keep `resource_info` intact for the error log above.
    let Some(metadata) = resource_info.metadata else {
        error_limited!("Metadata is not present in the ResourceInfo");
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    };
    Ok((metadata, resource_identity))
}

pub fn add_headers_from_resource_info(resource_info: ResourceInfo, builder: Builder) -> Result<Builder, StatusCode> {
    let (metadata, resource_identity) = parse_resource_info(resource_info)?;
    add_headers(metadata, resource_identity, builder)
}

pub fn add_headers(
    metadata: Metadata,
    resource_identity: ResourceIdentity,
    builder: Builder,
) -> Result<Builder, StatusCode> {
    Ok(builder
        .header(
            HeaderName::from_static("x-nvidia-omniverse-storage-metadata"),
            serde_json::to_string(&metadata).map_err(|e| {
                error_limited!(code = %StatusCode::INTERNAL_SERVER_ERROR, context = resource_identity.encoded_identity, error = %e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?,
        )
        .header(
            HeaderName::from_static("x-nvidia-omniverse-storage-resource-identity"),
            resource_identity.encoded_identity,
        ))
}
