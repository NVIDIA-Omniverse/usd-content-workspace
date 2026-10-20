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

use std::{
    env::{set_var, var},
    fs::create_dir_all,
    path::PathBuf,
};

fn build_permissions_protos(out_dir: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let permissions_proto_dir = out_dir.join("permissions-v1beta");
    create_dir_all(&permissions_proto_dir).expect("failed to create build artifact directories");

    tonic_prost_build::configure()
        .build_client(true)
        .build_server(true)
        .build_transport(true)
        .compile_well_known_types(true)
        .bytes(".")
        .type_attribute(
            "nvidia.omniverse.permission.v1beta.CheckPermissionBatchRequest",
            "#[derive(serde::Serialize, serde::Deserialize)]",
        )
        .type_attribute(
            "nvidia.omniverse.permission.v1beta.CheckPermissionBatchResponse",
            "#[derive(serde::Serialize, serde::Deserialize)]",
        )
        .type_attribute(
            "nvidia.omniverse.permission.v1beta.ResourceActionDecisionBatch",
            "#[derive(serde::Serialize, serde::Deserialize)]",
        )
        .type_attribute(
            "nvidia.omniverse.permission.v1beta.CheckPermissionBatchResponseSummary",
            "#[derive(serde::Serialize, serde::Deserialize)]",
        )
        .type_attribute(
            "nvidia.omniverse.permission.v1beta.ResourceActionDecision",
            "#[derive(serde::Serialize, serde::Deserialize)]",
        )
        .type_attribute(
            "nvidia.omniverse.permission.v1beta.CheckPermissionBatch",
            "#[derive(serde::Serialize, serde::Deserialize)]",
        )
        .field_attribute(
            "nvidia.omniverse.permission.v1beta.CheckPermissionBatch.context",
            "#[serde(skip)]",
        )
        .type_attribute(
            "nvidia.omniverse.permission.v1beta.Principal",
            "#[derive(serde::Serialize, serde::Deserialize)]",
        )
        .field_attribute("nvidia.omniverse.permission.v1beta.Principal.info", "#[serde(skip)]")
        .type_attribute(
            "nvidia.omniverse.permission.v1beta.Action",
            "#[derive(serde::Serialize, serde::Deserialize)]",
        )
        .type_attribute(
            "nvidia.omniverse.permission.v1beta.Resource",
            "#[derive(serde::Serialize, serde::Deserialize)]
                        #[serde(deny_unknown_fields)]",
        )
        .field_attribute("nvidia.omniverse.permission.v1beta.Resource.data", "#[serde(skip)]")
        .out_dir(&permissions_proto_dir)
        .file_descriptor_set_path(permissions_proto_dir.join("v1beta.bin"))
        .compile_protos(
            &["proto/nvidia/omniverse/permission/v1beta/permission_service.proto"],
            &["proto"],
        )?;
    Ok(())
}

fn build_discovery_protos(out_dir: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let discovery_proto_dir = out_dir.join("discovery-capabilities-v2alpha");
    create_dir_all(&discovery_proto_dir).expect("failed to create build artifact directories");

    tonic_prost_build::configure()
        .build_client(true)
        .build_server(true)
        .build_transport(true)
        .compile_well_known_types(true)
        .bytes(".")
        .type_attribute(
            ".nvidia.omniverse.discovery.capabilities.v2alpha",
            "#[derive(serde::Serialize, serde::Deserialize)]",
        )
        .type_attribute(
            ".nvidia.omniverse.discovery.capabilities.v2alpha",
            "#[derive(utoipa::ToSchema)]",
        )
        .out_dir(&discovery_proto_dir)
        .file_descriptor_set_path(discovery_proto_dir.join("v2alpha.bin"))
        .compile_protos(
            &["proto/nvidia/omniverse/discovery/capabilities/v2alpha/capabilities_service.proto"],
            &["proto"],
        )?;
    Ok(())
}

fn build_metadata_protos(out_dir: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let metadata_proto_dir = out_dir.join("metadata-v1alpha");
    create_dir_all(&metadata_proto_dir).expect("failed to create build artifact directories");

    // Metadata
    tonic_prost_build::configure()
        .build_client(true)
        .build_server(true)
        .build_transport(true)
        .compile_well_known_types(true)
        .bytes(".")
        .out_dir(&metadata_proto_dir)
        .type_attribute(
            "nvidia.omniverse.storage.metadata.v1alpha.GetMetadataBatchRequest",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.storage.metadata.v1alpha.GetMetadataBatchResponse",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.storage.metadata.v1alpha.QueryMetadataRequest",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.storage.metadata.v1alpha.QueryMetadataResponse",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.storage.metadata.v1alpha.UpdateMetadataBatchRequest",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.storage.metadata.v1alpha.UpdateMetadataBatchResponse",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.storage.metadata.v1alpha.DeleteMetadataBatchRequest",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.storage.metadata.v1alpha.DeleteMetadataBatchResponse",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.storage.metadata.v1alpha.GetMetadataBatchRequestEntry",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.storage.metadata.v1alpha.GetMetadataBatchResponseEntry",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.storage.metadata.v1alpha.GetMetadataBatchResponseEntry.outcome",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.storage.metadata.v1alpha.QueryMetadataResponseEntry",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.storage.metadata.v1alpha.UpdateMetadataBatchRequestEntry",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.storage.metadata.v1alpha.UpdateMetadataBatchResponseEntry",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.storage.metadata.v1alpha.UpdateMetadataBatchResponseEntry.outcome",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.storage.metadata.v1alpha.UpdateMetadataBatchResponseEntrySuccess",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.storage.metadata.v1alpha.UserMetadataValue",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema, get_size2::GetSize)]",
        )
        .type_attribute(
            "nvidia.omniverse.storage.metadata.v1alpha.BatchErrorEntry",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .field_attribute(
            "nvidia.omniverse.storage.metadata.v1alpha.BatchErrorEntry.code",
            "#[serde(with = \"crate::batch_entry_error_code_serde\")]",
        )
        .type_attribute(
            "nvidia.omniverse.storage.metadata.v1alpha.DeleteMetadataBatchRequestEntry",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.storage.metadata.v1alpha.DeleteMetadataBatchResponseEntry",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.storage.metadata.v1alpha.DeleteMetadataBatchResponseEntry.outcome",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.storage.metadata.v1alpha.DeleteMetadataBatchResponseEntrySuccess",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .file_descriptor_set_path(metadata_proto_dir.join("v1alpha.bin"))
        .compile_protos(
            &["proto/nvidia/omniverse/storage/metadata/v1alpha/metadata.proto"],
            &["proto"],
        )?;
    println!("successfully compiled metadata-v1alpha.bin");

    println!("starting to compile metadata-v1testmetadata.bin");

    let metadata_proto_test_dir = out_dir.join("metadata-v1testmetadata");
    create_dir_all(&metadata_proto_test_dir).expect("failed to create build artifact directories");

    tonic_prost_build::configure()
        .build_client(true)
        .build_server(true)
        .build_transport(true)
        .compile_well_known_types(true)
        .bytes(".")
        .out_dir(&metadata_proto_test_dir)
        .type_attribute(
            "nvidia.omniverse.storage.metadata.v1testmetadata.GetMetadataRequest",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.storage.metadata.v1testmetadata.GetMetadataResponse",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.storage.metadata.v1testmetadata.UpdateMetadataRequest",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.storage.metadata.v1testmetadata.UpdateMetadataResponse",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.storage.metadata.v1testmetadata.DeleteMetadataRequest",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.storage.metadata.v1testmetadata.DeleteMetadataResponse",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.storage.metadata.v1testmetadata.UserMetadataValue",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema, get_size2::GetSize)]",
        )
        .file_descriptor_set_path(metadata_proto_test_dir.join("v1testmetadata.bin"))
        .compile_protos(
            &["proto/nvidia/omniverse/storage/metadata/v1testmetadata/metadata.proto"],
            &["proto"],
        )?;

    Ok(())
}

fn build_metadata_api_protos(out_dir: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let metadata_proto_dir = out_dir.join("metadata-api-v1beta");
    create_dir_all(&metadata_proto_dir).expect("failed to create build artifact directories");

    tonic_prost_build::configure()
        .build_client(true)
        .build_server(true)
        .build_transport(true)
        .compile_well_known_types(true)
        .bytes(".")
        .out_dir(&metadata_proto_dir)
        .type_attribute(
            "nvidia.omniverse.metadata.v1beta.GetMetadataBatchRequest",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.metadata.v1beta.GetMetadataBatchResponse",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.metadata.v1beta.QueryMetadataRequest",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.metadata.v1beta.QueryMetadataResponse",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.metadata.v1beta.UpdateMetadataBatchRequest",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.metadata.v1beta.UpdateMetadataBatchResponse",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.metadata.v1beta.DeleteMetadataBatchRequest",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.metadata.v1beta.DeleteMetadataBatchResponse",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.metadata.v1beta.GetMetadataBatchRequestEntry",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.metadata.v1beta.GetMetadataBatchResponseEntry",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.metadata.v1beta.GetMetadataBatchResponseEntry.outcome",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.metadata.v1beta.QueryMetadataResponseEntry",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.metadata.v1beta.UpdateMetadataBatchRequestEntry",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.metadata.v1beta.UpdateMetadataBatchResponseEntry",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.metadata.v1beta.UpdateMetadataBatchResponseEntry.outcome",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.metadata.v1beta.UpdateMetadataBatchResponseEntrySuccess",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.metadata.v1beta.UserMetadataValue",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema, get_size2::GetSize)]",
        )
        .type_attribute(
            "nvidia.omniverse.metadata.v1beta.BatchErrorEntry",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .field_attribute(
            "nvidia.omniverse.metadata.v1beta.BatchErrorEntry.code",
            "#[serde(with = \"crate::batch_entry_error_code_serde\")]",
        )
        .type_attribute(
            "nvidia.omniverse.metadata.v1beta.DeleteMetadataBatchRequestEntry",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.metadata.v1beta.DeleteMetadataBatchResponseEntry",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.metadata.v1beta.DeleteMetadataBatchResponseEntry.outcome",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .type_attribute(
            "nvidia.omniverse.metadata.v1beta.DeleteMetadataBatchResponseEntrySuccess",
            "#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]",
        )
        .file_descriptor_set_path(metadata_proto_dir.join("v1beta.bin"))
        .compile_protos(&["proto/nvidia/omniverse/metadata/v1beta/metadata.proto"], &["proto"])?;

    Ok(())
}

fn main() {
    set_var("PROTOC", protoc_bin_vendored::protoc_bin_path().unwrap());
    set_var("PROTOC_INCLUDE", protoc_bin_vendored::include_path().unwrap());

    let out_dir = PathBuf::from(var("OUT_DIR").unwrap());
    let event_publisher_proto_dir = out_dir.join("event-publisher-v1beta");
    create_dir_all(&event_publisher_proto_dir).expect("failed to create build artifact directories");

    tonic_prost_build::configure()
        .build_client(true)
        .build_server(true)
        .build_transport(true)
        .compile_well_known_types(true)
        .bytes(".")
        .field_attribute("nvidia.omniverse.permission.v1beta.Resource.data", "#[serde(skip)]")
        .out_dir(&event_publisher_proto_dir)
        .file_descriptor_set_path(event_publisher_proto_dir.join("v1beta.bin"))
        .compile_protos(
            &["proto/nvidia/omniverse/notifications/publisher/v1beta/event_publisher.proto"],
            &["proto"],
        )
        .expect("failed to compile notification service protos");
    build_permissions_protos(out_dir.clone()).expect("failed to compile permissions service protos");
    build_metadata_protos(out_dir.clone()).expect("failed to compile metadata service protos");
    build_metadata_api_protos(out_dir.clone()).expect("failed to compile top-level metadata API protos");
    build_discovery_protos(out_dir).expect("failed to compile discovery service protos");
}
