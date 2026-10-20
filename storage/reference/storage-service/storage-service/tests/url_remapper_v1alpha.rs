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

mod common;

use std::{
    collections::{HashMap, HashSet},
    fmt::Display,
};

use assert_matches::assert_matches;
use common::{
    clients::{
        fileobject::types::v1alpha::UnwrapIdentityExt,
        metadata::types::v1alpha::{
            BatchErrorEntry, DeleteMetadataBatchRequestEntry, DeleteMetadataBatchResponseEntry,
            GetMetadataBatchRequestEntry, GetMetadataBatchResponseEntry, QueryMetadataResponseEntry, ResourceLocation,
            UpdateMetadataBatchRequestEntry,
        },
    },
    types::Interface,
};
use futures::TryStreamExt;
use rstest::{fixture, rstest};
use storage_service_protos::google::protobuf::Value;
use storage_service_test::mocks::permissions::{
    grpc::Service as GrpcPermissionService, Context as PermissionContext, Decision,
};
use tonic::Code;

use crate::common::{
    clients::{fileobject, metadata::types::v1alpha::ResourceLocationExt},
    env::get_env,
    random::{get_random_alphanumeric_string, get_random_bytes, get_random_path, get_random_path_item},
    service::Service as StorageService,
    url::{get_query_string, get_stat_query_pairs, UrlBuilder},
};

#[fixture]
#[once]
fn storage_service() -> StorageService {
    StorageService::builder()
        .enable_az(true)
        .enable_postgres_metadata()
        .enable_az_url_remapper("mapped/prefix/", "original/")
        .start()
}

#[rstest]
#[test_log::test(tokio::test)]
async fn read_from_mapped_address_missing_checkpoint_info(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    let path = get_random_path();
    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;

    // Write an object at the original address.
    let original_address = storage_service.remapper_original_url_builder().path(&path).to_string();
    let (original_info, content) = fileobject_client
        .write_random_object(original_address, 32)
        .await
        .expect("object write failed");
    let original_identity = original_info.unwrap_identity();

    // Read the object by the mapped address.
    let mapped_address = storage_service.remapper_mapped_url_builder().path(&path).to_string();
    let (mapped_info, mapped_content) = fileobject_client
        .read_object_from_address(mapped_address)
        .await
        .expect("object read failed");
    let mapped_identity = mapped_info.unwrap_identity();

    assert_eq!(content, mapped_content);
    assert_eq!(
        storage_service
            .remapper_mapped_url_builder()
            .path(&path)
            .raw_query_params(get_stat_query_pairs(&original_identity))
            .to_string(),
        mapped_identity,
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn read_from_mapped_address_known_checkpoint(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    let path = get_random_path();
    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;
    let metadata_client = storage_service.metadata_v1alpha_client(interface).await;

    // Write an object at the original address.
    let original_address = storage_service.remapper_original_url_builder().path(&path).to_string();
    let (original_info, content) = fileobject_client
        .write_random_object(&original_address, 32)
        .await
        .expect("object write failed");
    let original_identity = original_info.unwrap_identity();

    // Configure checkpoint metadata for the object.
    metadata_client
        .set_checkpoint_metadata(original_address, [(41, &original_identity)])
        .await
        .expect("checkpoint metadata update failed");

    // Read from the mapped address. We expect to see the mapped identity in the response.
    let (mapped_info, mapped_content) = fileobject_client
        .read_object_from_address(storage_service.remapper_mapped_url_builder().path(&path).to_string())
        .await
        .expect("object read failed");

    assert_eq!(
        storage_service
            .remapper_mapped_url_builder()
            .path(path)
            .checkpoint_query(41)
            .to_string(),
        mapped_info.unwrap_identity(),
    );
    assert_eq!(content, mapped_content);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn read_from_mapped_address_unknown_checkpoint(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    let path = get_random_path();
    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;
    let metadata_client = storage_service.metadata_v1alpha_client(interface).await;

    // Write an object at the original address.
    let original_address = storage_service.remapper_original_url_builder().path(&path).to_string();
    let (original_info, content) = fileobject_client
        .write_random_object(&original_address, 32)
        .await
        .expect("object write failed");
    let original_identity = original_info.unwrap_identity();

    // Configure checkpoint metadata for the object.
    metadata_client
        .set_checkpoint_metadata(original_address, [(41, "unknown-identity")])
        .await
        .expect("checkpoint metadata update failed");

    // Read object. The returned identity must contain query parameters from the original identity.
    let (mapped_info, mapped_content) = fileobject_client
        .read_object_from_address(storage_service.remapper_mapped_url_builder().path(&path).to_string())
        .await
        .expect("object read failed");

    assert_eq!(content, mapped_content);
    assert_eq!(
        storage_service
            .remapper_mapped_url_builder()
            .path(path)
            .raw_query_params(get_stat_query_pairs(&original_identity))
            .to_string(),
        mapped_info.unwrap_identity(),
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn read_from_mapped_native_identity(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    let path = get_random_path();
    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;
    let metadata_client = storage_service.metadata_v1alpha_client(interface).await;

    // Write multiple versions of an object at the original address.
    let original_address = storage_service.remapper_original_url_builder().path(&path).to_string();
    let (original_info_1, content_1) = fileobject_client
        .write_random_object(&original_address, 31)
        .await
        .expect("object write failed");
    let original_identity_1 = original_info_1.unwrap_identity();
    let (original_info_2, _) = fileobject_client
        .write_random_object(&original_address, 33)
        .await
        .expect("object write failed");
    let original_identity_2 = original_info_2.unwrap_identity();

    // Update checkpoint metadata.
    metadata_client
        .set_checkpoint_metadata(original_address, [(99, original_identity_1), (66, original_identity_2)])
        .await
        .expect("checkpoint metadata update failed");

    // Read from a mapped identity including a checkpoint number.
    let (_, mapped_content) = fileobject_client
        .read_object(
            storage_service
                .remapper_mapped_url_builder()
                .path(&path)
                .checkpoint_query(99)
                .to_string(),
        )
        .await
        .expect("object read failed");

    assert_eq!(mapped_content, content_1);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn read_from_mapped_foreign_identity(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    let path = get_random_path();
    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;

    // Write an object at the original address.
    let (original_info, content) = fileobject_client
        .write_random_object(
            storage_service.remapper_original_url_builder().path(&path).to_string(),
            32,
        )
        .await
        .expect("object write failed");
    let original_identity = original_info.unwrap_identity();

    // Read from a mapped identity with the original query parameters.
    let (_, mapped_content) = fileobject_client
        .read_object(
            storage_service
                .remapper_mapped_url_builder()
                .path(&path)
                .raw_query(get_query_string(&original_identity))
                .to_string(),
        )
        .await
        .expect("object read failed");

    assert_eq!(content, mapped_content);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn copy_from_mapped_foreign_identity(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::fileobject::types::v1alpha::{CopyRequest, ResourceIdentity};
    let path = get_random_path();
    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;

    // Write an object at the original address.
    let (original_info, written_content) = fileobject_client
        .write_random_object(
            storage_service.remapper_original_url_builder().path(&path).to_string(),
            32,
        )
        .await
        .expect("object write failed");
    let original_identity = original_info.unwrap_identity();
    let mapped_original_identity = storage_service
        .remapper_mapped_url_builder()
        .path(&path)
        .raw_query(get_query_string(&original_identity))
        .to_string();
    let destination_path = get_random_path();
    let mapped_destination_address = storage_service
        .remapper_mapped_url_builder()
        .path(&destination_path)
        .to_string();
    tracing::info!("original_identity: {}", original_identity);
    tracing::info!("mapped_destination_address: {}", mapped_destination_address);
    tracing::info!("mapped_original_identity: {}", mapped_original_identity);
    let copy_response = fileobject_client
        .copy(CopyRequest {
            destination_resource_address: mapped_destination_address.clone(),
            source_resource_identity: Some(ResourceIdentity {
                encoded_identity: mapped_original_identity,
            }),
            ..CopyRequest::default()
        })
        .await
        .expect("object copy failed");
    tracing::info!("copy_response: {:?}", copy_response);
    let copy_identity = copy_response
        .resource_identity
        .expect("resource identity is missing")
        .encoded_identity;

    assert_eq!(
        copy_identity.clone(),
        storage_service
            .remapper_mapped_url_builder()
            .path(&destination_path)
            .raw_query(get_query_string(&copy_identity))
            .to_string(),
    );

    let (_, read_content) = fileobject_client
        .read_object_from_address(&mapped_destination_address)
        .await
        .expect("object read failed");
    assert_eq!(read_content, written_content);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn move_from_mapped_foreign_identity(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::fileobject::types::v1alpha::MoveRequest;
    let path = get_random_path();
    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;
    let original_address = storage_service.remapper_original_url_builder().path(&path).to_string();
    let mapped_original_address = storage_service.remapper_mapped_url_builder().path(&path).to_string();
    // Write an object at the original address.
    let (_, written_content) = fileobject_client
        .write_random_object(&original_address, 32)
        .await
        .expect("object write failed");

    let destination_path = get_random_path();
    let destination_address = storage_service
        .remapper_original_url_builder()
        .path(&destination_path)
        .to_string();
    let mapped_destination_address = storage_service
        .remapper_mapped_url_builder()
        .path(&destination_path)
        .to_string();
    let move_response = fileobject_client
        .move_object(MoveRequest {
            source_resource_address: mapped_original_address,
            destination_resource_address: mapped_destination_address,
            ..MoveRequest::default()
        })
        .await
        .expect("object move failed");
    let move_identity = move_response
        .resource_identity
        .expect("resource identity is missing")
        .encoded_identity;
    assert_eq!(
        move_identity,
        storage_service
            .remapper_mapped_url_builder()
            .path(&destination_path)
            .raw_query(get_query_string(&move_identity))
            .to_string(),
    );

    let (_, read_content) = fileobject_client
        .read_object_from_address(&destination_address)
        .await
        .expect("object read failed");
    assert_eq!(read_content, written_content);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn read_from_mapped_unknown_native_identity(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    let path = get_random_path();
    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;

    // Write an object at the original address.
    let original_address = storage_service.remapper_original_url_builder().path(&path).to_string();
    fileobject_client
        .write_random_object(&original_address, 32)
        .await
        .expect("object write failed");

    // Read from a mapped identity using an unknown checkpoint number. The request must fail.
    let status = fileobject_client
        .read_object(
            storage_service
                .remapper_mapped_url_builder()
                .path(path)
                .checkpoint_query(111)
                .to_string(),
        )
        .await
        .expect_err("read request is expected to fail");

    assert_eq!(status.code(), Code::NotFound);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn stat_returns_foreign_identity(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::fileobject::types::v1alpha::StatRequest;

    let path = get_random_path();
    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;
    let metadata_client = storage_service.metadata_v1alpha_client(interface).await;

    // Write an object at the original address.
    let original_address = storage_service.remapper_original_url_builder().path(&path).to_string();
    let original_identity = fileobject_client
        .write_random_object(&original_address, 32)
        .await
        .expect("object write failed")
        .0
        .unwrap_identity();

    // Set checkpoint metadata.
    metadata_client
        .set_checkpoint_metadata(original_address, [(1, original_identity)])
        .await
        .expect("checkpoint metadata update failed");

    // Stat the mapped address. We must receive an identity with the proper checkpoint number.
    let mapped_identity = fileobject_client
        .stat(StatRequest {
            resource_address: storage_service.remapper_mapped_url_builder().path(&path).to_string(),
        })
        .await
        .expect("object stat failed")
        .resource_info
        .expect("resource info is missing")
        .unwrap_identity();

    assert_eq!(
        storage_service
            .remapper_mapped_url_builder()
            .path(path)
            .checkpoint_query(1)
            .to_string(),
        mapped_identity
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn stat_returns_native_identity(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::fileobject::types::v1alpha::StatRequest;

    let path = get_random_path();
    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;
    let metadata_client = storage_service.metadata_v1alpha_client(interface).await;

    // Write an object at the original address.
    let original_address = storage_service.remapper_original_url_builder().path(&path).to_string();
    let original_identity = fileobject_client
        .write_random_object(&original_address, 32)
        .await
        .expect("object write failed")
        .0
        .unwrap_identity();

    // Set checkpoint metadata.
    metadata_client
        .set_checkpoint_metadata(original_address, [(1, "unknown-identity")])
        .await
        .expect("checkpoint metadata update failed");

    // Stat the object using the mapped address. The returned identity must contain the original query parameters.
    let mapped_identity = fileobject_client
        .stat(StatRequest {
            resource_address: storage_service.remapper_mapped_url_builder().path(&path).to_string(),
        })
        .await
        .expect("object stat failed")
        .resource_info
        .expect("resource info is missing")
        .unwrap_identity();

    assert_eq!(
        storage_service
            .remapper_mapped_url_builder()
            .path(path)
            .raw_query_params(get_stat_query_pairs(&original_identity))
            .to_string(),
        mapped_identity
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn write_direct_returns_modified_identity(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    let path = get_random_path();
    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;

    // Write against a mapped address must succeed. Read against the original address must resolve to the same object.
    let original_address = storage_service.remapper_original_url_builder().path(&path).to_string();
    let mapped_address = storage_service.remapper_mapped_url_builder().path(&path).to_string();

    assert_read_original_after_write_mapped_succeeds(&fileobject_client, mapped_address, original_address, None).await;
}

#[rstest]
#[test_log::test(tokio::test)]
async fn write_direct_with_valid_previous_version_returns_modified_identity(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    let path = get_random_path();
    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;

    // Write an object at the original address.
    let original_address = storage_service.remapper_original_url_builder().path(&path).to_string();
    let original_identity = fileobject_client
        .write_random_object(&original_address, 32)
        .await
        .expect("object write failed")
        .0
        .unwrap_identity();

    // Conditional write against a mapped address with the original query parameters must succeed.
    // Read against the original address must resolve to the same object.
    let mapped_address = storage_service.remapper_mapped_url_builder().path(&path).to_string();
    let mapped_identity = storage_service
        .remapper_mapped_url_builder()
        .path(&path)
        .raw_query(get_query_string(&original_identity))
        .to_string();

    assert_read_original_after_write_mapped_succeeds(
        &fileobject_client,
        mapped_address,
        original_address,
        Some(mapped_identity),
    )
    .await;
}

#[rstest]
#[test_log::test(tokio::test)]
async fn write_direct_with_valid_previous_numbered_version_returns_modified_identity(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    let path = get_random_path();
    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;
    let metadata_client = storage_service.metadata_v1alpha_client(interface).await;

    // Write an object at the original address.
    let original_address = storage_service.remapper_original_url_builder().path(&path).to_string();
    let original_identity = fileobject_client
        .write_random_object(&original_address, 32)
        .await
        .expect("object write failed")
        .0
        .unwrap_identity();

    // Set checkpoint metadata for the object.
    metadata_client
        .set_checkpoint_metadata(&original_address, [(10, &original_identity)])
        .await
        .expect("checkpoint metadata update failed");

    // Conditional write against a mapped address with the checkpoint number must succeed.
    // Read against the original address must resolve to the same object.
    let mapped_address = storage_service.remapper_mapped_url_builder().path(&path).to_string();
    let mapped_identity = storage_service
        .remapper_mapped_url_builder()
        .path(&path)
        .checkpoint_query(10)
        .to_string();

    assert_read_original_after_write_mapped_succeeds(
        &fileobject_client,
        mapped_address,
        original_address,
        Some(mapped_identity),
    )
    .await;
}

#[rstest]
#[test_log::test(tokio::test)]
async fn write_with_invalid_previous_version_returns_invalid_argument(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::fileobject::types::v1alpha::{ResourceIdentity, UploadPreference, WriteParameters};

    let path = get_random_path();
    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;

    let status = fileobject_client
        .write(
            WriteParameters {
                destination_resource_address: storage_service.remapper_mapped_url_builder().path(&path).to_string(),
                previous_version: Some(ResourceIdentity {
                    encoded_identity: storage_service
                        .remapper_original_url_builder()
                        .path(&path)
                        .checkpoint_query(111)
                        .to_string(),
                }),
                data_object_size: 32,
                upload_preference: Some(UploadPreference::Body as i32),
                fail_if_exists: None,
                native_metadata: Default::default(),
            },
            get_random_bytes(32),
        )
        .await
        .expect_err("write is expected to fail");

    assert_eq!(status.code(), Code::InvalidArgument);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn complete_redirect_upload_returns_modified_identity(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::fileobject::types::v1alpha::{UploadPreference, WriteParameters};

    let path = get_random_path();
    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;

    // Perform a redirect write against a mapped address.
    let mapped_address = storage_service.remapper_mapped_url_builder().path(&path).to_string();
    let content = get_random_bytes(32);
    let (mapped_resource_info, used_write_method) = fileobject_client
        .write_object(
            WriteParameters {
                destination_resource_address: mapped_address.clone(),
                previous_version: None,
                data_object_size: content.len() as u64,
                upload_preference: Some(UploadPreference::Redirect as i32),
                fail_if_exists: None,
                native_metadata: Default::default(),
            },
            content.clone(),
        )
        .await
        .expect("write failed");
    let mapped_resource_identity = mapped_resource_info.unwrap_identity();

    assert_eq!(used_write_method, UploadPreference::Redirect);

    // The returned identity must contain the original query parameter. Hence, we must be able to forge the original
    // identity using them, which will resolve to the same object.
    let (_, original_content) = fileobject_client
        .read_object(
            storage_service
                .remapper_original_url_builder()
                .path(&path)
                .raw_query(get_query_string(&mapped_resource_identity))
                .to_string(),
        )
        .await
        .expect("object read failed");

    assert_eq!(original_content, content);

    let (_, mapped_content) = fileobject_client
        .read_object(mapped_resource_identity)
        .await
        .expect("object read failed");

    assert_eq!(mapped_content, content);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn complete_multipart_upload_returns_modified_identity(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::fileobject::types::v1alpha::{UploadPreference, WriteParameters};

    let path = get_random_path();
    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;

    // Perform a redirect write against a mapped address.
    let mapped_address = storage_service.remapper_mapped_url_builder().path(&path).to_string();
    let content = get_random_bytes(512 * 1024);
    let (mapped_resource_info, used_write_method) = fileobject_client
        .write_object(
            WriteParameters {
                destination_resource_address: mapped_address.clone(),
                previous_version: None,
                data_object_size: content.len() as u64,
                upload_preference: Some(UploadPreference::Multipart as i32),
                fail_if_exists: None,
                native_metadata: Default::default(),
            },
            content.clone(),
        )
        .await
        .expect("write failed");
    let _mapped_resource_identity = mapped_resource_info.unwrap_identity();

    assert_eq!(used_write_method, UploadPreference::Multipart);

    // todo: uncomment the following once the regression issues have been fixed.

    // The returned identity must contain the original query parameter. Hence, we must be able to forge the original
    // identity using them, which will resolve to the same object.
    // let (_, original_content) = fileobject_client
    //     .read_object(
    //         storage_service
    //             .remapper_original_url_builder()
    //             .path(&path)
    //             .raw_query(get_query_string(&mapped_resource_identity))
    //             .to_string(),
    //     )
    //     .await
    //     .expect("object read failed");

    // assert_eq!(original_content, content);

    // let (_, mapped_content) = fileobject_client
    //     .read_object(mapped_resource_identity)
    //     .await
    //     .expect("object read failed");

    // assert_eq!(mapped_content, content);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn delete_deletes_underlying_object(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::fileobject::types::v1alpha::{DeleteRequest, StatRequest};

    let path = get_random_path();
    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;

    // Create an object at the original address.
    let original_address = storage_service.remapper_original_url_builder().path(&path).to_string();
    fileobject_client
        .write_random_object(&original_address, 32)
        .await
        .expect("object write failed");

    // Delete the object by the mapped address.
    let mapped_address = storage_service.remapper_mapped_url_builder().path(&path).to_string();
    fileobject_client
        .delete(DeleteRequest {
            resource_address: mapped_address,
            previous_version: None,
        })
        .await
        .expect("object delete failed");

    // The deleted object must be not stat-able.
    let status = fileobject_client
        .stat(StatRequest {
            resource_address: original_address,
        })
        .await
        .expect_err("stat is expected to fail");

    assert_eq!(status.code(), Code::NotFound);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn fetch_write_type_info_returns_write_type_info_ranges(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::fileobject::types::v1alpha::FetchWriteTypeInfoRequest;

    let path = get_random_path();
    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;

    let original_write_type_info = fileobject_client
        .fetch_write_type_info(FetchWriteTypeInfoRequest {
            destination_resource_address: storage_service.remapper_original_url_builder().path(&path).to_string(),
        })
        .await
        .expect("write type info fetch failed");
    let mapped_write_type_info = fileobject_client
        .fetch_write_type_info(FetchWriteTypeInfoRequest {
            destination_resource_address: storage_service.remapper_mapped_url_builder().path(&path).to_string(),
        })
        .await
        .expect("write type info fetch failed");

    assert_eq!(original_write_type_info, mapped_write_type_info);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn copy_mapped_to_original_returns_original_identity(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::fileobject::types::v1alpha::{CopyRequest, ResourceIdentity};

    let source_path = get_random_path();
    let target_path = get_random_path();
    let client = storage_service.fileobject_v1alpha_client(interface).await;

    // Create an object at the original address.
    let source_original_address = storage_service
        .remapper_original_url_builder()
        .path(&source_path)
        .to_string();
    let (original_info, _content) = client
        .write_random_object(&source_original_address, 32)
        .await
        .expect("object write failed");
    let source_original_identity = original_info.unwrap_identity();

    // Copy from mapped to original. The returned identity must belong to the original destination address.
    let source_mapped_identity = storage_service
        .remapper_mapped_url_builder()
        .path(&source_path)
        .raw_query(get_query_string(&source_original_identity))
        .to_string();
    let target_original_address = storage_service
        .remapper_original_url_builder()
        .path(&target_path)
        .to_string();
    let copy_response = client
        .copy(CopyRequest {
            destination_resource_address: target_original_address,
            source_resource_identity: Some(ResourceIdentity {
                encoded_identity: source_mapped_identity,
            }),
            previous_version: None,
            fail_if_exists: None,
        })
        .await
        .expect("object copy failed");
    let copy_identity = copy_response
        .resource_identity
        .expect("resource identity is missing")
        .encoded_identity;

    assert_eq!(
        copy_identity,
        storage_service
            .remapper_original_url_builder()
            .path(&target_path)
            .raw_query(get_query_string(&copy_identity))
            .to_string(),
    );

    // todo: uncomment the following once the regression issues have been fixed.

    // The returned identity must be readable.
    // let (_, read_content) = fileobject_client
    //     .read_object(copy_identity)
    //     .await
    //     .expect("object read failed");

    // assert_eq!(content, read_content);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn copy_mapped_to_mapped_returns_modified_identity(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::fileobject::types::v1alpha::{CopyRequest, ResourceIdentity};

    let source_path = get_random_path();
    let target_path = get_random_path();
    let client = storage_service.fileobject_v1alpha_client(interface).await;

    // Create an object at the original address.
    let source_original_address = storage_service
        .remapper_original_url_builder()
        .path(&source_path)
        .to_string();
    let (original_info, _content) = client
        .write_random_object(&source_original_address, 32)
        .await
        .expect("object write failed");
    let source_original_identity = original_info.unwrap_identity();

    // Copy from mapped to mapped. The returned identity must belong to the mapped destination address.
    let source_mapped_identity = storage_service
        .remapper_mapped_url_builder()
        .path(&source_path)
        .raw_query(get_query_string(&source_original_identity))
        .to_string();
    let target_mapped_address = storage_service
        .remapper_mapped_url_builder()
        .path(&target_path)
        .to_string();
    let copy_response = client
        .copy(CopyRequest {
            destination_resource_address: target_mapped_address,
            source_resource_identity: Some(ResourceIdentity {
                encoded_identity: source_mapped_identity,
            }),
            previous_version: None,
            fail_if_exists: None,
        })
        .await
        .expect("object copy failed");
    let copy_identity = copy_response
        .resource_identity
        .expect("resource identity is missing")
        .encoded_identity;

    assert_eq!(
        copy_identity,
        storage_service
            .remapper_mapped_url_builder()
            .path(&target_path)
            .raw_query(get_query_string(&copy_identity))
            .to_string(),
    );

    // todo: uncomment the following once the regression issues have been fixed.

    // The returned identity must be readable.
    // let (_, read_content) = fileobject_client
    //     .read_object(copy_identity)
    //     .await
    //     .expect("object read failed");

    // assert_eq!(content, read_content);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn copy_original_to_mapped_returns_modified_identity(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::fileobject::types::v1alpha::{CopyRequest, ResourceIdentity};

    let source_path = get_random_path();
    let target_path = get_random_path();
    let client = storage_service.fileobject_v1alpha_client(interface).await;

    // Create an object at the original address.
    let source_original_address = storage_service
        .remapper_original_url_builder()
        .path(&source_path)
        .to_string();
    let (original_info, _content) = client
        .write_random_object(&source_original_address, 32)
        .await
        .expect("object write failed");
    let source_original_identity = original_info.unwrap_identity();

    // Copy from original to mapped. The returned identity must belong to the mapped address.
    let target_mapped_address = storage_service
        .remapper_mapped_url_builder()
        .path(&target_path)
        .to_string();
    let copy_response = client
        .copy(CopyRequest {
            destination_resource_address: target_mapped_address,
            source_resource_identity: Some(ResourceIdentity {
                encoded_identity: source_original_identity,
            }),
            previous_version: None,
            fail_if_exists: None,
        })
        .await
        .expect("object copy failed");
    let copy_identity = copy_response
        .resource_identity
        .expect("resource identity is missing")
        .encoded_identity;

    assert_eq!(
        copy_identity,
        storage_service
            .remapper_mapped_url_builder()
            .path(&target_path)
            .raw_query(get_query_string(&copy_identity))
            .to_string(),
    );

    // todo: uncomment the following once the regression issues have been fixed.

    // The returned identity must be readable.
    // let (_, read_content) = fileobject_client
    //     .read_object(copy_identity)
    //     .await
    //     .expect("object read failed");

    // assert_eq!(content, read_content);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn enumerate_rewrites_resource_addresses(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::fileobject::types::v1alpha::EnumerateRequest;

    let folder_path = get_random_path_item();
    let object_path_1 = format!("{}/{}", folder_path, get_random_path_item());
    let object_path_2 = format!("{}/{}", folder_path, get_random_path_item());
    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;

    // Create objects in the folder.
    create_objects(
        &fileobject_client,
        [&object_path_1, &object_path_2]
            .into_iter()
            .map(|path| storage_service.remapper_original_url_builder().path(path).to_string()),
    )
    .await;

    // Enumerate objects in the folder. The returned identities must be rewritten.
    let mut responses = fileobject_client.enumerate(EnumerateRequest {
        resource_address: storage_service
            .remapper_mapped_url_builder()
            .path(&folder_path)
            .to_string(),
    });

    let mut object_addresses = HashSet::new();
    while let Some(response) = responses.try_next().await.expect("enumerate request failed") {
        object_addresses.extend(response.items.into_iter().map(|info| info.resource_address));
    }

    assert_eq!(
        object_addresses,
        HashSet::from([
            storage_service
                .remapper_mapped_url_builder()
                .path(object_path_1)
                .to_string(),
            storage_service
                .remapper_mapped_url_builder()
                .path(object_path_2)
                .to_string(),
        ])
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn list_rewrites_resource_addresses(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::filefolder::types::v1alpha::{FolderAddress, ListRequest};

    let folder_path = get_random_path_item();
    let object_path_1 = format!("{}/{}", folder_path, get_random_path_item());
    let object_path_2 = format!("{}/{}", folder_path, get_random_path_item());
    let subfolder_path = format!("{}/{}", folder_path, get_random_path_item());
    let object_path_3 = format!("{}/{}", subfolder_path, get_random_path_item());
    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;
    let filefolder_client = storage_service.filefolder_v1alpha_client(interface).await;

    // Populate objects.
    create_objects(
        &fileobject_client,
        [&object_path_1, &object_path_2, &object_path_3]
            .into_iter()
            .map(|path| storage_service.remapper_original_url_builder().path(path).to_string()),
    )
    .await;

    // List the mapped folder address. The returned addresses must be mapped.
    let mut responses = filefolder_client.list(ListRequest {
        folder: Some(FolderAddress {
            uri: storage_service
                .remapper_mapped_url_builder()
                .path(folder_path)
                .to_string(),
        }),
    });

    let mut object_addresses = HashSet::new();
    let mut folder_addresses = HashSet::new();
    while let Some(response) = responses.try_next().await.expect("list request failed") {
        object_addresses.extend(response.sub_resource_addresses);
        folder_addresses.extend(response.subfolder_addresses.into_iter().map(|address| address.uri));
    }

    assert_eq!(
        folder_addresses,
        HashSet::from([storage_service
            .remapper_mapped_url_builder()
            .path(format!("{}/", subfolder_path))
            .to_string(),]),
    );
    assert_eq!(
        object_addresses,
        HashSet::from([
            storage_service
                .remapper_mapped_url_builder()
                .path(object_path_1)
                .to_string(),
            storage_service
                .remapper_mapped_url_builder()
                .path(object_path_2)
                .to_string(),
        ]),
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn list_stat_rewrites_resource_addresses(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::filefolder::types::v1alpha::{FolderAddress, ListStatRequest};

    let folder_path = get_random_path_item();
    let object_path_1 = format!("{}/{}", folder_path, get_random_path_item());
    let object_path_2 = format!("{}/{}", folder_path, get_random_path_item());
    let subfolder_path = format!("{}/{}", folder_path, get_random_path_item());
    let object_path_3 = format!("{}/{}", subfolder_path, get_random_path_item());
    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;
    let filefolder_client = storage_service.filefolder_v1alpha_client(interface).await;
    let metadata_client = storage_service.metadata_v1alpha_client(interface).await;

    // Populate objects.
    let identities = create_objects(
        &fileobject_client,
        [&object_path_1, &object_path_2, &object_path_3]
            .into_iter()
            .map(|path| storage_service.remapper_original_url_builder().path(path).to_string()),
    )
    .await;

    // Populate checkpoint metadata for the objects.
    metadata_client
        .set_checkpoint_metadata(
            storage_service
                .remapper_original_url_builder()
                .path(&object_path_1)
                .to_string(),
            [(11u64, &identities[0])],
        )
        .await
        .expect("checkpoint metadata set failed");
    metadata_client
        .set_checkpoint_metadata(
            storage_service
                .remapper_original_url_builder()
                .path(&object_path_2)
                .to_string(),
            [(13u64, &identities[1])],
        )
        .await
        .expect("checkpoint metadata set failed");

    // List and stat the mapped folder address. The returned addresses and identities must be mapped.
    let mut responses = filefolder_client.list_stat(ListStatRequest {
        folder: Some(FolderAddress {
            uri: storage_service
                .remapper_mapped_url_builder()
                .path(folder_path)
                .to_string(),
        }),
        include_native_metadata: false,
    });

    let mut object_infos = HashSet::new();
    let mut folder_addresses = HashSet::new();
    while let Some(response) = responses.try_next().await.expect("list request failed") {
        object_infos.extend(response.entries.into_iter().map(|item| {
            (
                item.resource_address,
                item.resource_info
                    .expect("resource info is missing")
                    .resource_identity
                    .expect("resource identity is missing")
                    .encoded_identity,
            )
        }));
        folder_addresses.extend(response.subfolder_addresses.into_iter().map(|address| address.uri));
    }

    assert_eq!(
        folder_addresses,
        HashSet::from([storage_service
            .remapper_mapped_url_builder()
            .path(format!("{}/", subfolder_path))
            .to_string(),]),
    );
    assert_eq!(
        object_infos,
        HashSet::from([
            (
                storage_service
                    .remapper_mapped_url_builder()
                    .path(&object_path_1)
                    .to_string(),
                storage_service
                    .remapper_mapped_url_builder()
                    .path(object_path_1)
                    .checkpoint_query(11)
                    .to_string(),
            ),
            (
                storage_service
                    .remapper_mapped_url_builder()
                    .path(&object_path_2)
                    .to_string(),
                storage_service
                    .remapper_mapped_url_builder()
                    .path(object_path_2)
                    .checkpoint_query(13)
                    .to_string(),
            )
        ]),
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn create_folder_rewrites_resource_addresses(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::filefolder::types::v1alpha::{
        CreateFolderRequest, CreateFolderResponse, DeleteFolderRequest, FolderAddress,
    };

    let folder_path = get_random_path_item();
    let filefolder_client = storage_service.filefolder_v1alpha_client(interface).await;
    let create_folder_response = filefolder_client
        .create_folder(CreateFolderRequest {
            folder: Some(FolderAddress {
                uri: storage_service
                    .remapper_mapped_url_builder()
                    .path(&folder_path)
                    .to_string(),
            }),
        })
        .await
        .expect("create folder failed");
    assert_eq!(create_folder_response, CreateFolderResponse::default());

    filefolder_client
        .delete_folder(DeleteFolderRequest {
            folder: Some(FolderAddress {
                uri: storage_service
                    .remapper_mapped_url_builder()
                    .path(&folder_path)
                    .to_string(),
            }),
        })
        .await
        .expect("delete folder failed");
}

#[rstest]
#[test_log::test(tokio::test)]
async fn get_folder_mode_rewrites_resource_addresses(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::filefolder::types::v1alpha::{
        CreateFolderRequest, CreateFolderResponse, DeleteFolderRequest, FolderAddress, GetFolderModeRequest,
    };

    let folder_path = get_random_path_item();
    let filefolder_client = storage_service.filefolder_v1alpha_client(interface).await;
    let create_folder_response = filefolder_client
        .create_folder(CreateFolderRequest {
            folder: Some(FolderAddress {
                uri: storage_service
                    .remapper_mapped_url_builder()
                    .path(&folder_path)
                    .to_string(),
            }),
        })
        .await
        .expect("create folder failed");
    assert_eq!(create_folder_response, CreateFolderResponse::default());

    filefolder_client
        .get_folder_mode(GetFolderModeRequest {
            folder: Some(FolderAddress {
                uri: storage_service
                    .remapper_mapped_url_builder()
                    .path(&folder_path)
                    .to_string(),
            }),
        })
        .await
        .expect("get folder mode failed");
    filefolder_client
        .delete_folder(DeleteFolderRequest {
            folder: Some(FolderAddress {
                uri: storage_service
                    .remapper_mapped_url_builder()
                    .path(&folder_path)
                    .to_string(),
            }),
        })
        .await
        .expect("delete folder failed");
}

#[rstest]
#[test_log::test(tokio::test)]
async fn delete_folder_rewrites_resource_addresses(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::filefolder::types::v1alpha::{
        CreateFolderRequest, CreateFolderResponse, DeleteFolderRequest, DeleteFolderResponse, FolderAddress,
        ListRequest,
    };

    let folder_path = get_random_path_item();
    let filefolder_client = storage_service.filefolder_v1alpha_client(interface).await;
    let create_folder_response = filefolder_client
        .create_folder(CreateFolderRequest {
            folder: Some(FolderAddress {
                uri: storage_service
                    .remapper_mapped_url_builder()
                    .path(&folder_path)
                    .to_string(),
            }),
        })
        .await
        .expect("create folder failed");
    assert_eq!(create_folder_response, CreateFolderResponse::default());

    let delete_folder_response = filefolder_client
        .delete_folder(DeleteFolderRequest {
            folder: Some(FolderAddress {
                uri: storage_service
                    .remapper_mapped_url_builder()
                    .path(&folder_path)
                    .to_string(),
            }),
        })
        .await
        .expect("delete folder failed");
    assert_eq!(delete_folder_response, DeleteFolderResponse::default());
    filefolder_client
        .list(ListRequest {
            folder: Some(FolderAddress {
                uri: storage_service
                    .remapper_mapped_url_builder()
                    .path(&folder_path)
                    .to_string(),
            }),
        })
        .try_next()
        .await
        .expect_err("list request failed");
}

#[rstest]
#[test_log::test(tokio::test)]
async fn get_set_metadata_manage_original_object_metadata(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::{
        GetMetadataRequest, GetMetadataResponse, UpdateMetadataRequest, UserMetadataValue,
    };
    use storage_service_protos::google::protobuf::{value::Kind, Value};

    let path = get_random_path();
    let metadata_client = storage_service.metadata_v1alpha_client(interface).await;

    // Update metadata for the mapped address.
    let metadata = Value {
        kind: Some(Kind::StringValue("somestring".into())),
    };
    let update_metadata_response = metadata_client
        .update_metadata(UpdateMetadataRequest {
            uri: storage_service.remapper_mapped_url_builder().path(&path).to_string(),
            user_metadata_key: "key".into(),
            user_metadata: Some(metadata.clone()),
            ..Default::default()
        })
        .await
        .expect("metadata update failed");

    // Reading metadata from the original address must return the same metadata.
    let get_metadata_response = metadata_client
        .get_metadata(GetMetadataRequest {
            uri: storage_service.remapper_original_url_builder().path(&path).to_string(),
            ..Default::default()
        })
        .await
        .expect("metadata fetch failed");

    assert_eq!(
        get_metadata_response,
        GetMetadataResponse {
            user_metadata: HashMap::from([(
                "key".to_string(),
                UserMetadataValue {
                    value: Some(metadata),
                    etag: update_metadata_response.etag,
                },
            )]),
        },
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn delete_metadata_deletes_original_object_metadata(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::{
        DeleteMetadataRequest, GetMetadataRequest, GetMetadataResponse, UpdateMetadataRequest,
    };
    use storage_service_protos::google::protobuf::{value::Kind, Value};

    let path = get_random_path();
    let metadata_client = storage_service.metadata_v1alpha_client(interface).await;

    // Update metadata for the original address.
    let metadata = Value {
        kind: Some(Kind::StringValue("somestring".into())),
    };
    let update_metadata_response = metadata_client
        .update_metadata(UpdateMetadataRequest {
            uri: storage_service.remapper_original_url_builder().path(&path).to_string(),
            user_metadata_key: "key".into(),
            user_metadata: Some(metadata.clone()),
            ..Default::default()
        })
        .await
        .expect("metadata update failed");

    // Delete metadata for the mapped address.
    metadata_client
        .delete_metadata(DeleteMetadataRequest {
            uri: storage_service.remapper_original_url_builder().path(&path).to_string(),
            user_metadata_key: "key".into(),
            expected_etag: Some(update_metadata_response.etag),
        })
        .await
        .expect("delete metadata failed");

    // Read metadata from the original address. It must not appear in the response.
    let get_metadata_response = metadata_client
        .get_metadata(GetMetadataRequest {
            uri: storage_service.remapper_original_url_builder().path(&path).to_string(),
            user_metadata_keys: vec!["key".into()],
        })
        .await
        .expect("metadata fetch failed");

    assert_eq!(get_metadata_response, GetMetadataResponse::default());
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_get_metadata_batch(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::{GetMetadataBatchRequest, GetMetadataBatchResponse};

    let client = storage_service.metadata_v1alpha_client(interface).await;

    let path_1 = get_random_path();
    let path_2 = get_random_path();
    let path_3 = get_random_path();
    let path_4 = get_random_path();
    let path_5 = get_random_path();
    let path_6 = get_random_path();
    let resource_uri_1 = storage_service
        .remapper_original_url_builder()
        .path(&path_1)
        .to_string();
    let resource_uri_2 = storage_service.remapper_mapped_url_builder().path(&path_1).to_string();
    let resource_uri_3 = storage_service
        .remapper_original_url_builder()
        .path(&path_2)
        .query("etag=abcd")
        .to_string();
    let resource_uri_4 = storage_service
        .remapper_mapped_url_builder()
        .path(&path_2)
        .checkpoint_query(222)
        .to_string();
    let resource_uri_5 = storage_service
        .remapper_original_url_builder()
        .path(&path_3)
        .to_string();
    let resource_uri_6 = storage_service
        .remapper_mapped_url_builder()
        .path(&path_3)
        .checkpoint_query(333)
        .to_string();
    let resource_uri_7 = storage_service
        .remapper_original_url_builder()
        .path(&path_4)
        .query("etag=pqrs")
        .to_string();
    let resource_uri_8 = storage_service
        .remapper_mapped_url_builder()
        .path(&path_4)
        .query("etag=pqrs")
        .to_string();
    let resource_uri_9 = storage_service
        .remapper_original_url_builder()
        .path(&path_5)
        .query("etag=1234")
        .to_string();
    let resource_uri_10 = storage_service
        .remapper_mapped_url_builder()
        .path(&path_5)
        .query("etag=1234")
        .to_string();
    let resource_uri_11 = storage_service
        .remapper_original_url_builder()
        .path(&path_6)
        .query("etag=2345")
        .to_string();
    let resource_uri_12 = storage_service
        .remapper_mapped_url_builder()
        .path(&path_6)
        .checkpoint_query(333)
        .to_string();

    client
        .set_checkpoint_metadata(
            storage_service
                .remapper_original_url_builder()
                .path(&path_2)
                .to_string(),
            [(222, &resource_uri_3)],
        )
        .await
        .expect("checkpoint metadata set failed");
    client
        .set_checkpoint_metadata(
            storage_service
                .remapper_original_url_builder()
                .path(&path_6)
                .to_string(),
            [(333, &resource_uri_11)],
        )
        .await
        .expect("checkpoint metadata set failed");

    let (value_1, etag_1) = client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri_1), "key-1")
        .await
        .expect("metadata set failed");
    let (value_2, etag_2) = client
        .set_random_metadata_value(ResourceLocation::resource_identity(&resource_uri_3), "key-2")
        .await
        .expect("metadata set failed");
    let (value_3, etag_3) = client
        .set_random_metadata_value(ResourceLocation::folder_address(&resource_uri_5), "key-3")
        .await
        .expect("metadata set failed");
    let (value_4, etag_4) = client
        .set_random_metadata_value(ResourceLocation::resource_identity(&resource_uri_7), "key-4")
        .await
        .expect("metadata set failed");
    let (value_5, etag_5) = client
        .set_random_metadata_value(ResourceLocation::resource_identity(&resource_uri_9), "key-5")
        .await
        .expect("metadata set failed");
    let (value_6, etag_6) = client
        .set_random_metadata_value(ResourceLocation::resource_identity(&resource_uri_11), "key-6")
        .await
        .expect("metadata set failed");

    // Checkpoint query params are only rewritten for resource addresses and identities.
    // Query in folder addresses is ignored.
    let response = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_1, "key-1"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_2, "key-1"),
                create_get_metadata_batch_resource_identity_request_entry(&resource_uri_3, "key-2"),
                create_get_metadata_batch_resource_identity_request_entry(&resource_uri_4, "key-2"),
                create_get_metadata_batch_folder_address_request_entry(&resource_uri_5, "key-3"),
                create_get_metadata_batch_folder_address_request_entry(&resource_uri_6, "key-3"),
                create_get_metadata_batch_resource_identity_request_entry(&resource_uri_7, "key-4"),
                create_get_metadata_batch_resource_identity_request_entry(&resource_uri_8, "key-4"),
                create_get_metadata_batch_resource_identity_request_entry(&resource_uri_9, "key-5"),
                create_get_metadata_batch_resource_identity_request_entry(&resource_uri_10, "key-5"),
                create_get_metadata_batch_resource_identity_request_entry(&resource_uri_11, "key-6"),
                create_get_metadata_batch_resource_identity_request_entry(&resource_uri_12, "key-6"),
            ],
        })
        .await
        .expect("get metadata failed");

    assert_eq!(
        response,
        GetMetadataBatchResponse {
            results: vec![
                create_get_metadata_batch_success_response_entry(0, value_1.clone(), etag_1.clone()),
                create_get_metadata_batch_success_response_entry(1, value_1, etag_1),
                create_get_metadata_batch_success_response_entry(2, value_2.clone(), etag_2.clone()),
                create_get_metadata_batch_success_response_entry(3, value_2, etag_2),
                create_get_metadata_batch_success_response_entry(4, value_3.clone(), etag_3.clone()),
                create_get_metadata_batch_success_response_entry(5, value_3, etag_3),
                create_get_metadata_batch_success_response_entry(6, value_4.clone(), etag_4.clone()),
                create_get_metadata_batch_success_response_entry(7, value_4, etag_4),
                create_get_metadata_batch_success_response_entry(8, value_5.clone(), etag_5.clone()),
                create_get_metadata_batch_success_response_entry(9, value_5, etag_5),
                create_get_metadata_batch_success_response_entry(10, value_6.clone(), etag_6.clone()),
                create_get_metadata_batch_success_response_entry(11, value_6, etag_6),
            ]
        }
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_get_metadata_batch_rejects_missing_checkpoint_queries(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let client = storage_service.metadata_v1alpha_client(interface).await;

    let path_1 = get_random_path();
    let path_2 = get_random_path();
    let resource_uri_1 = storage_service
        .remapper_mapped_url_builder()
        .path(&path_1)
        .checkpoint_query(123)
        .to_string();
    let resource_uri_2 = storage_service
        .remapper_mapped_url_builder()
        .path(&path_2)
        .query("etag=abcd")
        .to_string();
    let resource_uri_3 = storage_service
        .remapper_original_url_builder()
        .path(&path_2)
        .query("etag=abcd")
        .to_string();
    let resource_uri_4 = storage_service
        .remapper_mapped_url_builder()
        .path(&path_2)
        .checkpoint_query(456)
        .to_string();

    let (value_2, etag_2) = client
        .set_random_metadata_value(ResourceLocation::resource_identity(&resource_uri_3), "key")
        .await
        .expect("metadata set failed");

    let response = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                create_get_metadata_batch_resource_identity_request_entry(&resource_uri_1, "key"),
                create_get_metadata_batch_resource_identity_request_entry(&resource_uri_2, "key"),
                create_get_metadata_batch_resource_identity_request_entry(&resource_uri_3, "key"),
                create_get_metadata_batch_resource_identity_request_entry(&resource_uri_4, "key"),
            ],
        })
        .await
        .expect("get metadata failed");

    assert_eq!(
        response,
        GetMetadataBatchResponse {
            results: vec![
                create_get_metadata_batch_missing_checkpoint_metadata_response_entry(0, 123),
                create_get_metadata_batch_success_response_entry(1, value_2.clone(), etag_2.clone()),
                create_get_metadata_batch_success_response_entry(2, value_2, etag_2),
                create_get_metadata_batch_missing_checkpoint_metadata_response_entry(3, 456),
            ]
        }
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_query_metadata_for_resource_address(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let client = storage_service.metadata_v1alpha_client(interface).await;

    let path = get_random_path();
    let original_uri = storage_service.remapper_original_url_builder().path(&path).to_string();
    let (value, etag) = client
        .set_random_metadata_value(ResourceLocation::resource_address(&original_uri), "a")
        .await
        .unwrap();

    let mapped_uri = storage_service
        .remapper_mapped_url_builder()
        .path(&path)
        .query("etag=123")
        .to_string();
    let query_response = client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::resource_address(mapped_uri)),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(
        query_response,
        QueryMetadataResponse {
            results: vec![create_query_metadata_response_entry("a", value, etag),]
        }
    );
}

#[ignore = "checkpointed resource addresses are not yet supported"]
#[rstest]
#[test_log::test(tokio::test)]
async fn test_query_metadata_for_checkpointed_resource_address(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let client = storage_service.metadata_v1alpha_client(interface).await;

    let path = get_random_path();
    let original_uri = storage_service
        .remapper_original_url_builder()
        .path(&path)
        .query("etag=123")
        .to_string();
    let (value, etag) = client
        .set_random_metadata_value(ResourceLocation::resource_identity(&original_uri), "a")
        .await
        .unwrap();

    client
        .set_checkpoint_metadata(
            storage_service.remapper_original_url_builder().path(&path).to_string(),
            [(22, &original_uri)],
        )
        .await
        .unwrap();

    let mapped_uri = storage_service
        .remapper_mapped_url_builder()
        .path(&path)
        .checkpoint_query(22)
        .to_string();
    let query_response = client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::resource_address(mapped_uri)),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(
        query_response,
        QueryMetadataResponse {
            results: vec![create_query_metadata_response_entry("a", value, etag),]
        }
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_query_metadata_for_resource_identity(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let client = storage_service.metadata_v1alpha_client(interface).await;

    let path = get_random_path();
    let original_uri = storage_service
        .remapper_original_url_builder()
        .path(&path)
        .query("etag=123")
        .to_string();
    let (value, etag) = client
        .set_random_metadata_value(ResourceLocation::resource_identity(&original_uri), "a")
        .await
        .unwrap();

    let mapped_uri = storage_service
        .remapper_mapped_url_builder()
        .path(&path)
        .query("etag=123")
        .to_string();
    let query_response = client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::resource_identity(mapped_uri)),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(
        query_response,
        QueryMetadataResponse {
            results: vec![create_query_metadata_response_entry("a", value, etag),]
        }
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_query_metadata_for_checkpointed_resource_identity(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let client = storage_service.metadata_v1alpha_client(interface).await;

    let path = get_random_path();
    let original_uri = storage_service
        .remapper_original_url_builder()
        .path(&path)
        .query("etag=123")
        .to_string();
    let (value, etag) = client
        .set_random_metadata_value(ResourceLocation::resource_identity(&original_uri), "a")
        .await
        .unwrap();

    client
        .set_checkpoint_metadata(
            storage_service.remapper_original_url_builder().path(&path).to_string(),
            [(22, &original_uri)],
        )
        .await
        .unwrap();

    let mapped_uri = storage_service
        .remapper_mapped_url_builder()
        .path(&path)
        .checkpoint_query(22)
        .to_string();
    let query_response = client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::resource_identity(mapped_uri)),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(
        query_response,
        QueryMetadataResponse {
            results: vec![create_query_metadata_response_entry("a", value, etag),]
        }
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_query_metadata_for_folder_address(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let client = storage_service.metadata_v1alpha_client(interface).await;

    let path = get_random_path();
    let original_uri = storage_service.remapper_original_url_builder().path(&path).to_string();
    let (value, etag) = client
        .set_random_metadata_value(ResourceLocation::folder_address(original_uri), "a")
        .await
        .unwrap();

    // Query parameters are ignored in folder addresses.
    let mapped_uri = storage_service
        .remapper_mapped_url_builder()
        .path(&path)
        .checkpoint_query(32)
        .to_string();
    let query_response = client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::folder_address(mapped_uri)),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(
        query_response,
        QueryMetadataResponse {
            results: vec![create_query_metadata_response_entry("a", value, etag),]
        }
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_query_metadata_rejects_missing_checkpoint_queries(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let client = storage_service.metadata_v1alpha_client(interface).await;
    let resource_uri = storage_service
        .remapper_mapped_url_builder()
        .path(get_random_path())
        .checkpoint_query(32)
        .to_string();

    let query_error = client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::resource_address(resource_uri)),
            ..Default::default()
        })
        .await
        .unwrap_err();

    assert_eq!(query_error.code(), Code::NotFound);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_update_metadata_batch(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::{
        batch_error_entry::Code, update_metadata_batch_response_entry::Outcome, UpdateMetadataBatchRequest,
    };

    let client = storage_service.metadata_v1alpha_client(interface).await;

    let path_3 = get_random_path();
    let resource_uri_1 = storage_service
        .remapper_mapped_url_builder()
        .path(get_random_path())
        .to_string();
    let resource_uri_2 = storage_service
        .remapper_mapped_url_builder()
        .path(get_random_path())
        .query("etag=abcd")
        .to_string();
    let resource_uri_3 = storage_service
        .remapper_mapped_url_builder()
        .path(&path_3)
        .checkpoint_query(33)
        .to_string();
    let resource_uri_4 = storage_service
        .remapper_mapped_url_builder()
        .path(get_random_path())
        .checkpoint_query(44)
        .to_string();

    client
        .set_checkpoint_metadata(
            storage_service
                .remapper_original_url_builder()
                .path(&path_3)
                .to_string(),
            [(
                33,
                storage_service
                    .remapper_original_url_builder()
                    .path(&path_3)
                    .query("etag=bcde")
                    .to_string(),
            )],
        )
        .await
        .expect("checkpoint metadata set failed");

    let response = client
        .update_metadata_batch_sorted(UpdateMetadataBatchRequest {
            requests: vec![
                create_update_metadata_batch_resource_address_request_entry(&resource_uri_1, "key-1"),
                create_update_metadata_batch_resource_address_request_entry(&resource_uri_2, "key-2"),
                create_update_metadata_batch_resource_address_request_entry(&resource_uri_3, "key-3"),
                create_update_metadata_batch_resource_identity_request_entry(&resource_uri_2, "key-4"),
                create_update_metadata_batch_resource_identity_request_entry(&resource_uri_3, "key-5"),
                create_update_metadata_batch_folder_address_request_entry(&resource_uri_4, "key-6"),
                UpdateMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri_3)),
                    user_metadata_key: "cond-1".into(),
                    user_metadata: Some(create_random_value()),
                    expected_etag: Some("some-etag".into()),
                    ..Default::default()
                },
                UpdateMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::folder_address(&resource_uri_3)),
                    user_metadata_key: "cond-2".into(),
                    user_metadata: Some(create_random_value()),
                    expected_etag: Some("some-etag".into()),
                    ..Default::default()
                },
                UpdateMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_identity(&resource_uri_3)),
                    user_metadata_key: "cond-3".into(),
                    user_metadata: Some(create_random_value()),
                    expected_etag: Some("some-etag".into()),
                    ..Default::default()
                },
            ],
        })
        .await
        .expect("update metadata batch failed");

    assert_eq!(response.results.len(), 9);
    assert_eq!(response.results[0].request_index, 0);
    assert_eq!(response.results[1].request_index, 1);
    assert_eq!(response.results[2].request_index, 2);
    assert_eq!(response.results[3].request_index, 3);
    assert_eq!(response.results[4].request_index, 4);
    assert_eq!(response.results[5].request_index, 5);
    assert_eq!(response.results[6].request_index, 6);
    assert_eq!(response.results[7].request_index, 7);
    assert_eq!(response.results[8].request_index, 8);
    assert_matches!(&response.results[0].outcome, Some(Outcome::Success(e)) if !e.etag.is_empty());
    assert_matches!(&response.results[1].outcome, Some(Outcome::Success(e)) if !e.etag.is_empty());
    assert_matches!(&response.results[2].outcome, Some(Outcome::Success(e)) if !e.etag.is_empty());
    assert_matches!(&response.results[3].outcome, Some(Outcome::Success(e)) if !e.etag.is_empty());
    assert_matches!(&response.results[4].outcome, Some(Outcome::Success(e)) if !e.etag.is_empty());
    assert_matches!(&response.results[5].outcome, Some(Outcome::Success(e)) if !e.etag.is_empty());
    assert_matches!(&response.results[6].outcome, Some(Outcome::Error(e)) if e.code == Code::FailedPrecondition as i32);
    assert_matches!(&response.results[7].outcome, Some(Outcome::Error(e)) if e.code == Code::FailedPrecondition as i32);
    assert_matches!(&response.results[8].outcome, Some(Outcome::Error(e)) if e.code == Code::FailedPrecondition as i32);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_update_metadata_batch_rejects_missing_checkpoint_queries(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::{
        batch_error_entry::Code, update_metadata_batch_response_entry::Outcome, UpdateMetadataBatchRequest,
    };

    let client = storage_service.metadata_v1alpha_client(interface).await;

    let resource_uri_1 = storage_service
        .remapper_mapped_url_builder()
        .path(get_random_path())
        .checkpoint_query(11)
        .to_string();
    let resource_uri_2 = storage_service
        .remapper_mapped_url_builder()
        .path(get_random_path())
        .query("etag=1234")
        .to_string();
    let resource_uri_3 = storage_service
        .remapper_mapped_url_builder()
        .path(get_random_path())
        .checkpoint_query(22)
        .to_string();

    let response = client
        .update_metadata_batch_sorted(UpdateMetadataBatchRequest {
            requests: vec![
                create_update_metadata_batch_resource_identity_request_entry(&resource_uri_1, "key-1"),
                create_update_metadata_batch_resource_identity_request_entry(&resource_uri_2, "key-2"),
                create_update_metadata_batch_resource_identity_request_entry(&resource_uri_3, "key-3"),
            ],
        })
        .await
        .expect("metadata update failed");

    assert_eq!(response.results.len(), 3);
    assert_eq!(response.results[0].request_index, 0);
    assert_eq!(response.results[1].request_index, 1);
    assert_eq!(response.results[2].request_index, 2);
    assert_matches!(&response.results[0].outcome, Some(Outcome::Error(e)) if e.code == Code::NotFound as i32);
    assert_matches!(&response.results[1].outcome, Some(Outcome::Success(e)) if !e.etag.is_empty());
    assert_matches!(&response.results[2].outcome, Some(Outcome::Error(e)) if e.code == Code::NotFound as i32);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_delete_metadata_batch(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::{DeleteMetadataBatchRequest, DeleteMetadataBatchResponse};

    let client = storage_service.metadata_v1alpha_client(interface).await;

    let path_3 = get_random_path();
    let resource_uri_1 = storage_service
        .remapper_mapped_url_builder()
        .path(get_random_path())
        .to_string();
    let resource_uri_2 = storage_service
        .remapper_mapped_url_builder()
        .path(get_random_path())
        .query("etag=abcd")
        .to_string();
    let resource_uri_3 = storage_service
        .remapper_mapped_url_builder()
        .path(&path_3)
        .checkpoint_query(33)
        .to_string();
    let resource_uri_4 = storage_service
        .remapper_mapped_url_builder()
        .path(get_random_path())
        .checkpoint_query(44)
        .to_string();

    client
        .set_checkpoint_metadata(
            storage_service
                .remapper_original_url_builder()
                .path(&path_3)
                .to_string(),
            [(
                33,
                storage_service
                    .remapper_original_url_builder()
                    .path(&path_3)
                    .query("etag=bcde")
                    .to_string(),
            )],
        )
        .await
        .expect("checkpoint metadata set failed");

    let response = client
        .delete_metadata_batch_sorted(DeleteMetadataBatchRequest {
            requests: vec![
                create_delete_metadata_batch_resource_address_request_entry(&resource_uri_1, "key-1"),
                create_delete_metadata_batch_resource_address_request_entry(&resource_uri_2, "key-2"),
                create_delete_metadata_batch_resource_address_request_entry(&resource_uri_3, "key-3"),
                create_delete_metadata_batch_resource_identity_request_entry(&resource_uri_2, "key-4"),
                create_delete_metadata_batch_resource_identity_request_entry(&resource_uri_3, "key-5"),
                create_delete_metadata_batch_folder_address_request_entry(&resource_uri_4, "key-6"),
                DeleteMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri_3)),
                    user_metadata_key: "cond-1".into(),
                    expected_etag: Some("some-etag".into()),
                    ..Default::default()
                },
                DeleteMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::folder_address(&resource_uri_3)),
                    user_metadata_key: "cond-2".into(),
                    expected_etag: Some("some-etag".into()),
                    ..Default::default()
                },
                DeleteMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_identity(&resource_uri_3)),
                    user_metadata_key: "cond-3".into(),
                    expected_etag: Some("some-etag".into()),
                    ..Default::default()
                },
            ],
        })
        .await
        .expect("metadata delete failed");

    assert_eq!(
        response,
        DeleteMetadataBatchResponse {
            results: vec![
                create_delete_metadata_batch_success_response_entry(0),
                create_delete_metadata_batch_success_response_entry(1),
                create_delete_metadata_batch_success_response_entry(2),
                create_delete_metadata_batch_success_response_entry(3),
                create_delete_metadata_batch_success_response_entry(4),
                create_delete_metadata_batch_success_response_entry(5),
                create_delete_metadata_batch_metadata_modified_response_entry(6),
                create_delete_metadata_batch_metadata_modified_response_entry(7),
                create_delete_metadata_batch_metadata_modified_response_entry(8),
            ],
        },
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_delete_metadata_batch_rejects_missing_checkpoint_queries(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::{DeleteMetadataBatchRequest, DeleteMetadataBatchResponse};

    let client = storage_service.metadata_v1alpha_client(interface).await;

    let resource_uri_1 = storage_service
        .remapper_mapped_url_builder()
        .path(get_random_path())
        .checkpoint_query(11)
        .to_string();
    let resource_uri_2 = storage_service
        .remapper_mapped_url_builder()
        .path(get_random_path())
        .query("etag=1234")
        .to_string();
    let resource_uri_3 = storage_service
        .remapper_mapped_url_builder()
        .path(get_random_path())
        .checkpoint_query(22)
        .to_string();

    let response = client
        .delete_metadata_batch_sorted(DeleteMetadataBatchRequest {
            requests: vec![
                create_delete_metadata_batch_resource_identity_request_entry(&resource_uri_1, "key-1"),
                create_delete_metadata_batch_resource_identity_request_entry(&resource_uri_2, "key-2"),
                create_delete_metadata_batch_resource_identity_request_entry(&resource_uri_3, "key-3"),
            ],
        })
        .await
        .expect("metadata delete failed");

    assert_eq!(
        response,
        DeleteMetadataBatchResponse {
            results: vec![
                create_delete_metadata_batch_missing_checkpoint_metadata_response_entry(0, 11),
                create_delete_metadata_batch_success_response_entry(1),
                create_delete_metadata_batch_missing_checkpoint_metadata_response_entry(2, 22),
            ],
        },
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn list_versions_rewrites_identities(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use storage_core::traits::versioning::v1alpha::EnumerateVersionsRequest;

    let path = get_random_path();
    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;
    let versioning_client = storage_service.versioning_v1alpha_client(interface).await;
    let metadata_client = storage_service.metadata_v1alpha_client(interface).await;

    // Create multiple versions of the object.
    let original_address = storage_service.remapper_original_url_builder().path(&path).to_string();
    let original_identity_1 = fileobject_client
        .write_random_object(&original_address, 32)
        .await
        .expect("object write failed")
        .0
        .unwrap_identity();
    let original_identity_2 = fileobject_client
        .write_random_object(&original_address, 32)
        .await
        .expect("object write failed")
        .0
        .unwrap_identity();

    // Populate checkpoint metadata just for a single version.
    metadata_client
        .set_checkpoint_metadata(&original_address, [(99u64, original_identity_1)])
        .await
        .expect("metadata update failed");

    // List versions of the object via the mapped address. We must see one native and one foreign identity.
    let mut responses = versioning_client.enumerate_versions(EnumerateVersionsRequest {
        resource_address: storage_service.remapper_mapped_url_builder().path(&path).to_string(),
    });

    let mut identities = HashSet::new();
    while let Some(response) = responses.try_next().await.expect("enumerate versions failed") {
        identities.extend(response.items.into_iter().map(|item| {
            item.resource_info
                .expect("resource info is missing")
                .resource_identity
                .expect("resource identity is missing")
                .encoded_identity
        }));
    }

    assert_eq!(
        identities,
        HashSet::from([
            storage_service
                .remapper_mapped_url_builder()
                .path(&path)
                .checkpoint_query(99)
                .to_string(),
            storage_service
                .remapper_mapped_url_builder()
                .path(&path)
                .raw_query(get_query_string(&original_identity_2))
                .to_string(),
        ]),
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn stat_versioned_resource_address_with_checkpoint_query(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use storage_core::traits::fileobject::v1alpha::StatRequest;

    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;
    let metadata_client = storage_service.metadata_v1alpha_client(interface).await;
    let path = get_random_path();

    // Write an object.
    let original_address = storage_service.remapper_original_url_builder().path(&path).to_string();
    let (resource_info_1, _) = fileobject_client
        .write_random_object(&original_address, 32)
        .await
        .expect("object write failed");
    let (resource_info_2, _) = fileobject_client
        .write_random_object(&original_address, 32)
        .await
        .expect("object write failed");

    // Set checkpoint metadata for the address.
    metadata_client
        .set_checkpoint_metadata(
            original_address,
            [
                (1, resource_info_1.unwrap_identity()),
                (2, resource_info_2.unwrap_identity()),
            ],
        )
        .await
        .expect("checkpoint metadata set failed");

    // Stat the versioned address. The operation must succeed.
    let mapped_address = storage_service
        .remapper_mapped_url_builder()
        .path(path)
        .checkpoint_query(1)
        .to_string();
    let response = fileobject_client
        .stat(StatRequest {
            resource_address: mapped_address.clone(),
        })
        .await
        .expect("stat failed");

    // Nucleus resource addresses and identities are encoded the same way. Hence, we compare the resource identity from
    // the response with the address that we used in the query.
    let resource_identity = response
        .resource_info
        .expect("resource info is missing from a response")
        .resource_identity
        .expect("resource identity is missing from resource info")
        .encoded_identity;
    assert_eq!(resource_identity, mapped_address);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn stat_versioned_resource_addresses_with_original_query(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use storage_core::traits::fileobject::v1alpha::StatRequest;

    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;
    let path = get_random_path();

    // Write an object.
    let original_address = storage_service.remapper_original_url_builder().path(&path).to_string();
    let (resource_info_1, _) = fileobject_client
        .write_random_object(&original_address, 32)
        .await
        .expect("object write failed");
    fileobject_client
        .write_random_object(&original_address, 32)
        .await
        .expect("object write failed");

    // Stat the versioned address. The operation must succeed.
    let original_identity = resource_info_1.unwrap_identity();
    let mapped_address = storage_service
        .remapper_mapped_url_builder()
        .path(&path)
        .raw_query(get_version_id_query_segment(&original_identity))
        .to_string();
    let response = fileobject_client
        .stat(StatRequest {
            resource_address: mapped_address,
        })
        .await
        .expect("stat failed");

    // Since no checkpoint metadata is present for the specified identity, an identity containing the original query
    // parameters must be returned.
    let expected_resource_identity = storage_service
        .remapper_mapped_url_builder()
        .path(path)
        .raw_query(get_query_string(&original_identity))
        .to_string();
    let resource_identity = response
        .resource_info
        .expect("resource info is missing from a response")
        .resource_identity
        .expect("resource identity is missing from resource info")
        .encoded_identity;
    assert_eq!(resource_identity, expected_resource_identity);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn read_from_versioned_resource_addresses_with_checkpoint_query(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;
    let metadata_client = storage_service.metadata_v1alpha_client(interface).await;
    let path = get_random_path();

    // Write an object.
    let original_address = storage_service.remapper_original_url_builder().path(&path).to_string();
    let (resource_info_1, content_1) = fileobject_client
        .write_random_object(&original_address, 32)
        .await
        .expect("object write failed");
    let (resource_info_2, _) = fileobject_client
        .write_random_object(&original_address, 32)
        .await
        .expect("object write failed");

    // Set checkpoint metadata for the object.
    metadata_client
        .set_checkpoint_metadata(
            original_address,
            [
                (10, resource_info_1.unwrap_identity()),
                (20, resource_info_2.unwrap_identity()),
            ],
        )
        .await
        .expect("checkpoint metadata set failed");

    // Read an object version by the checkpoint address. The identity and the content must match.
    let mapped_address = storage_service
        .remapper_mapped_url_builder()
        .path(path)
        .checkpoint_query(10)
        .to_string();
    let (metadata, content) = fileobject_client
        .read_object_from_address(&mapped_address)
        .await
        .expect("object read failed");

    assert_eq!(content, content_1);

    let resource_identity = metadata
        .resource_identity
        .expect("resource identity is missing from the response")
        .encoded_identity;
    assert_eq!(resource_identity, mapped_address);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn read_from_versioned_resource_addresses_with_original_query(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;
    let path = get_random_path();

    // Write an object.
    let original_address = storage_service.remapper_original_url_builder().path(&path).to_string();
    let (resource_info_1, content_1) = fileobject_client
        .write_random_object(&original_address, 32)
        .await
        .expect("object write failed");
    fileobject_client
        .write_random_object(&original_address, 32)
        .await
        .expect("object write failed");

    // Read an object version by the mapped address containing the original version id parameter.
    let resource_identity_1 = resource_info_1
        .resource_identity
        .expect("resource identity is missing from a response")
        .encoded_identity;
    let mapped_address = storage_service
        .remapper_mapped_url_builder()
        .path(&path)
        .raw_query(get_version_id_query_segment(&resource_identity_1))
        .to_string();
    let (metadata, content) = fileobject_client
        .read_object_from_address(&mapped_address)
        .await
        .expect("object read failed");

    assert_eq!(content, content_1);

    let expected_resource_identity = storage_service
        .remapper_mapped_url_builder()
        .path(path)
        .raw_query(get_query_string(&resource_identity_1))
        .to_string();
    let resource_identity = metadata
        .resource_identity
        .expect("resource identity is missing from the response")
        .encoded_identity;
    assert_eq!(resource_identity, expected_resource_identity);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn enumerate_versions_returns_checkpointed_addresses_and_identities(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use storage_core::traits::versioning::v1alpha::EnumerateVersionsRequest;

    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;
    let versioning_client = storage_service.versioning_v1alpha_client(interface).await;
    let metadata_client = storage_service.metadata_v1alpha_client(interface).await;
    let path = get_random_path();

    // Create an object with multiple versions.
    let original_address = storage_service.remapper_original_url_builder().path(&path).to_string();
    let (resource_info_1, _) = fileobject_client
        .write_random_object(&original_address, 11)
        .await
        .expect("object write failed");
    let (resource_info_2, _) = fileobject_client
        .write_random_object(&original_address, 13)
        .await
        .expect("object write failed");
    let (resource_info_3, _) = fileobject_client
        .write_random_object(&original_address, 17)
        .await
        .expect("object write failed");

    // Set checkpoint metadata for the object.
    metadata_client
        .set_checkpoint_metadata(
            original_address,
            [
                (9, resource_info_1.unwrap_identity()),
                (10, resource_info_2.unwrap_identity()),
                (11, resource_info_3.unwrap_identity()),
            ],
        )
        .await
        .expect("checkpoint metadata set failed");

    // Enumerate versions of the object. All addresses must be checkpointed.
    let mapped_address = storage_service.remapper_mapped_url_builder().path(&path).to_string();
    let responses: Vec<_> = versioning_client
        .enumerate_versions(EnumerateVersionsRequest {
            resource_address: mapped_address,
        })
        .try_collect()
        .await
        .expect("versions enumerate failed");

    // Compare the list of actual (address, identity) pairs with the expectation.
    let version_addresses_and_identities: Vec<_> = responses
        .into_iter()
        .flat_map(|r| r.items.into_iter())
        .map(|item| {
            (
                item.resource_address
                    .expect("resource address is missing from a response"),
                item.resource_info
                    .expect("resource info is missing from a response")
                    .resource_identity
                    .expect("resource identity is missing from resource info")
                    .encoded_identity,
            )
        })
        .collect();
    let expected_version_addresses_and_identities: Vec<_> = [9, 10, 11]
        .into_iter()
        .map(|checkpoint| {
            storage_service
                .remapper_mapped_url_builder()
                .path(&path)
                .checkpoint_query(checkpoint)
                .to_string()
        })
        .map(|url| (url.clone(), url))
        .collect();
    assert_eq!(
        version_addresses_and_identities,
        expected_version_addresses_and_identities
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn enumerate_versions_returns_addresses_and_identities_with_original_query(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use storage_core::traits::versioning::v1alpha::EnumerateVersionsRequest;

    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;
    let versioning_client = storage_service.versioning_v1alpha_client(interface).await;
    let path = get_random_path();

    // Create an object with multiple versions.
    let original_address = storage_service.remapper_original_url_builder().path(&path).to_string();
    let (resource_info_1, _) = fileobject_client
        .write_random_object(&original_address, 11)
        .await
        .expect("object write failed");
    let (resource_info_2, _) = fileobject_client
        .write_random_object(&original_address, 13)
        .await
        .expect("object write failed");
    let (resource_info_3, _) = fileobject_client
        .write_random_object(&original_address, 17)
        .await
        .expect("object write failed");

    // Enumerate versions of the object. All addresses must be checkpointed.
    let mapped_address = storage_service.remapper_mapped_url_builder().path(&path).to_string();
    let responses: Vec<_> = versioning_client
        .enumerate_versions(EnumerateVersionsRequest {
            resource_address: mapped_address,
        })
        .try_collect()
        .await
        .expect("versions enumerate failed");

    // Compare the list of actual (address, identity) pairs with the expectation.
    let version_addresses_and_identities: Vec<_> = responses
        .into_iter()
        .flat_map(|r| r.items.into_iter())
        .map(|item| {
            (
                item.resource_address
                    .expect("resource address is missing from a response"),
                item.resource_info
                    .expect("resource info is missing from a response")
                    .resource_identity
                    .expect("resource identity is missing from resource info")
                    .encoded_identity,
            )
        })
        .collect();

    let expected_version_addresses_and_identities: Vec<_> = [
        resource_info_1.unwrap_identity(),
        resource_info_2.unwrap_identity(),
        resource_info_3.unwrap_identity(),
    ]
    .into_iter()
    .map(|original_identity| {
        (
            storage_service
                .remapper_mapped_url_builder()
                .path(&path)
                .raw_query(get_version_id_query_segment(&original_identity))
                .to_string(),
            storage_service
                .remapper_mapped_url_builder()
                .path(&path)
                .raw_query(get_query_string(&original_identity))
                .to_string(),
        )
    })
    .collect();
    assert_eq!(
        version_addresses_and_identities,
        expected_version_addresses_and_identities
    );
}

#[rstest]
#[case("", "omniverse://myhost.mydomain", "omniverse://myhost.mydomain/")]
#[case("", "omniverse://myhost.mydomain/", "omniverse://myhost.mydomain/")]
#[case(
    "prefix/",
    "omniverse://myhost.mydomain/prefix",
    "omniverse://myhost.mydomain/prefix/"
)]
#[case(
    "prefix/",
    "omniverse://myhost.mydomain/prefix/",
    "omniverse://myhost.mydomain/prefix/"
)]
#[test_log::test(tokio::test)]
async fn test_list_rewrites_addresses_without_explicit_path(
    #[case] mapped_prefix: &str,
    #[case] folder_uri: &str,
    #[case] response_uri_prefix: &str,
) {
    use storage_core::traits::filefolder::v1alpha::{FolderAddress, ListRequest, ListResponse};

    let folder_path = get_random_path();
    let storage_service = StorageService::builder()
        .enable_az(true)
        .enable_az_url_remapper(mapped_prefix, format!("{}/", folder_path))
        .enable_postgres_metadata()
        .start();

    let fileobject_client = storage_service.fileobject_v1alpha_client(Interface::Grpc).await;
    let filefolder_client = storage_service.filefolder_v1alpha_client(Interface::Grpc).await;
    let file_name = get_random_path_item();

    // Write an object.
    let original_address = storage_service.remapper_original_url_builder().path(&file_name);
    fileobject_client
        .write_random_object(original_address, 32)
        .await
        .expect("object write failed");

    // List the mapped root resource address.
    let response = filefolder_client
        .list_collect(ListRequest {
            folder: Some(FolderAddress { uri: folder_uri.into() }),
        })
        .await
        .expect("list failed");

    assert_eq!(
        response,
        ListResponse {
            sub_resource_addresses: vec![UrlBuilder::new(response_uri_prefix).path(&file_name).to_string()],
            ..Default::default()
        }
    );
}

#[rstest]
#[case("", "omniverse://myhost.mydomain")]
#[case("", "omniverse://myhost.mydomain/")]
#[case("prefix/", "omniverse://myhost.mydomain/prefix")]
#[case("prefix/", "omniverse://myhost.mydomain/prefix/")]
#[test_log::test(tokio::test)]
async fn test_list_stat_rewrites_addresses_without_explicit_path(
    #[case] mapped_prefix: &str,
    #[case] folder_uri: &str,
) {
    use storage_core::traits::filefolder::v1alpha::{FolderAddress, ListStatRequest};

    let folder_path = get_random_path();
    let storage_service = StorageService::builder()
        .enable_az(true)
        .enable_az_url_remapper(mapped_prefix, format!("{}/", folder_path))
        .enable_postgres_metadata()
        .start();

    let fileobject_client = storage_service.fileobject_v1alpha_client(Interface::Grpc).await;
    let filefolder_client = storage_service.filefolder_v1alpha_client(Interface::Grpc).await;
    let file_name = get_random_path_item();

    // Write an object.
    let original_address = storage_service.remapper_original_url_builder().path(&file_name);
    let (original_resource_info, _) = fileobject_client
        .write_random_object(original_address, 32)
        .await
        .expect("object write failed");

    // List the mapped root resource address.
    let response = filefolder_client
        .list_stat_collect(ListStatRequest {
            folder: Some(FolderAddress { uri: folder_uri.into() }),
            include_native_metadata: false,
        })
        .await
        .expect("liststat failed");

    assert!(response.subfolder_addresses.is_empty());

    let response_identities: Vec<_> = response
        .entries
        .into_iter()
        .map(|entry| entry.resource_info.expect("resource info is missing").unwrap_identity())
        .collect();
    let expected_identities = [storage_service
        .remapper_mapped_url_builder()
        .path(file_name)
        .raw_query_params(get_stat_query_pairs(&original_resource_info.unwrap_identity()))
        .to_string()];
    assert_eq!(response_identities, expected_identities);
}

#[rstest]
#[case("", "omniverse://myhost.mydomain")]
#[case("", "omniverse://myhost.mydomain/")]
#[case("prefix/", "omniverse://myhost.mydomain/prefix")]
#[case("prefix/", "omniverse://myhost.mydomain/prefix/")]
#[test_log::test(tokio::test)]
async fn test_enumerate_rewrites_addresses_without_explicit_path(
    #[case] mapped_prefix: &str,
    #[case] folder_address: &str,
) {
    use storage_core::traits::fileobject::v1alpha::EnumerateRequest;

    let folder_path = get_random_path();
    let storage_service = StorageService::builder()
        .enable_az(true)
        .enable_az_url_remapper(mapped_prefix, format!("{}/", folder_path))
        .enable_postgres_metadata()
        .start();

    let fileobject_client = storage_service.fileobject_v1alpha_client(Interface::Grpc).await;
    let file_name = get_random_path_item();

    // Write an object.
    let original_address = storage_service.remapper_original_url_builder().path(&file_name);
    fileobject_client
        .write_random_object(original_address, 32)
        .await
        .expect("object write failed");

    // Enumerate the root address.
    let response = fileobject_client
        .enumerate_collect(EnumerateRequest {
            resource_address: folder_address.into(),
        })
        .await
        .expect("enumerate failed");

    let response_addresses: Vec<_> = response.items.into_iter().map(|entry| entry.resource_address).collect();
    let expected_addresses = [storage_service
        .remapper_mapped_url_builder()
        .path(file_name)
        .to_string()];
    assert_eq!(response_addresses, expected_addresses);
}

#[rstest]
#[case("omniverse://myhost.mydomain", "omniverse://myhost.mydomain/")]
#[case("omniverse://myhost.mydomain/", "omniverse://myhost.mydomain/")]
#[case("omniverse://myhost.mydomain/root/", "omniverse://myhost.mydomain/root/")]
#[test_log::test(tokio::test)]
async fn test_get_top_level_addresses_returns_a_correct_entry(
    #[case] mapped_address: &str,
    #[case] expected_top_level_address: &str,
) {
    use common::clients::capabilities::types::v1alpha::TopLevelAddressEntry;

    let storage_service = StorageService::builder()
        .enable_az(true)
        .enable_postgres_metadata()
        .enable_url_remapper(mapped_address, "https://localhost.localdomain")
        .start();
    let client = storage_service.capabilities_v1alpha_client(Interface::Grpc).await;

    let response = client
        .list_top_level_addresses(Default::default())
        .await
        .expect("top level addresses listing failed");

    assert!(response.items.contains(&TopLevelAddressEntry {
        top_level_address: expected_top_level_address.into()
    }));
}

#[test_log::test(tokio::test)]
async fn test_get_top_level_addresses_hides_nested_mounts() {
    use common::clients::capabilities::types::v1alpha::TopLevelAddressEntry;

    let env = common::env::get_env().unwrap_s3();
    let storage_service = StorageService::builder()
        .enable_s3(false)
        .with_url_remapper_config(serde_json::json!({
            "enabled": true,
            "expose_child_rules_as_folders": true,
            "rewrite_rules": {
                "omniverse://content.example.com/": { "target": env.resource_url("root-target/") },
                "omniverse://content.example.com/NVIDIA/": { "target": env.resource_url("nvidia-target/") },
            },
        }))
        .start();
    let client = storage_service.capabilities_v1alpha_client(Interface::Grpc).await;

    let response = client
        .list_top_level_addresses(Default::default())
        .await
        .expect("top level addresses listing failed");

    let tla = |address: &str| TopLevelAddressEntry {
        top_level_address: address.into(),
    };
    assert!(response.items.contains(&tla("omniverse://content.example.com/")));
    assert!(!response.items.contains(&tla("omniverse://content.example.com/NVIDIA/")));
}

/// A mount configured directly under a real backend endpoint — with no
/// parent rewrite rule of its own — must be accepted at startup and then
/// hidden from the top-level address listing (its parent, the endpoint,
/// is what surfaces the mount). This is the "MountedBucket folder under a
/// configured storage account" scenario: previously the service refused
/// to start because the mount's parent was not itself a rewrite rule.
#[test_log::test(tokio::test)]
async fn test_get_top_level_addresses_hides_endpoint_hosted_mount() {
    use common::clients::capabilities::types::v1alpha::TopLevelAddressEntry;

    let env = common::env::get_env().unwrap_s3();
    // `resource_url("MountedBucket/")` sits directly under the configured S3
    // endpoint (`http://host:port/bucket`); there is no rule for its parent.
    let mount_source = env.resource_url("MountedBucket/");
    let storage_service = StorageService::builder()
        .enable_s3(false)
        .with_url_remapper_config(serde_json::json!({
            "enabled": true,
            "expose_child_rules_as_folders": true,
            "rewrite_rules": {
                &mount_source: { "target": env.resource_url("mounted-target/") },
            },
        }))
        .start();
    let client = storage_service.capabilities_v1alpha_client(Interface::Grpc).await;

    let response = client
        .list_top_level_addresses(Default::default())
        .await
        .expect("top level addresses listing failed");

    assert!(!response.items.contains(&TopLevelAddressEntry {
        top_level_address: mount_source,
    }));
}

/// Azure counterpart to the test above. An account is exposed as its containers, not its root, so a
/// mount hosted directly under a declared account keeps its own top-level entry (the account root is
/// not self-listed) and is listed alongside the backend's real container -- unlike the self-listed
/// S3 bucket above, which suppresses its hosted mount.
#[test_log::test(tokio::test)]
async fn test_get_top_level_addresses_keeps_account_root_hosted_mount() {
    use common::clients::capabilities::types::v1alpha::TopLevelAddressEntry;

    let env = common::env::get_env().unwrap_azure();
    // Mounted directly under the declared account endpoint; there is no rule for its parent.
    let mount_source = format!("{}MountedBucket/", env.storage_account_url());
    let mount_target = format!("{}mounted-target/", env.storage_container_url());
    let storage_service = StorageService::builder()
        .enable_az(true)
        .enable_postgres_metadata()
        .with_url_remapper_config(serde_json::json!({
            "enabled": true,
            "expose_child_rules_as_folders": true,
            "rewrite_rules": {
                &mount_source: { "target": mount_target },
            },
        }))
        .start();
    let client = storage_service.capabilities_v1alpha_client(Interface::Grpc).await;

    let response = client
        .list_top_level_addresses(Default::default())
        .await
        .expect("top level addresses listing failed");

    // The backend expands the declared account into its container, the real browsable top-level
    // address (returned without a trailing slash).
    let container = {
        let mut url = storage_service.az_url_builder().to_string();
        url.pop();
        url
    };
    assert!(
        response.items.contains(&TopLevelAddressEntry {
            top_level_address: container.clone(),
        }),
        "expected the account's container {container:?} in {:?}",
        response.items
    );

    // And the endpoint-hosted mount is kept alongside it, not suppressed.
    assert!(
        response.items.contains(&TopLevelAddressEntry {
            top_level_address: mount_source.clone(),
        }),
        "expected the endpoint-hosted mount {mount_source:?} in {:?}",
        response.items
    );
}

#[test_log::test(tokio::test)]
async fn test_alias_hidden_from_top_level_addresses() {
    use common::clients::capabilities::types::v1alpha::TopLevelAddressEntry;

    let env = common::env::get_env().unwrap_s3();
    let storage_service = StorageService::builder()
        .enable_s3(false)
        .with_url_remapper_config(serde_json::json!({
            "enabled": true,
            "rewrite_rules": {
                "omniverse://shown.example.com/": { "target": env.resource_url("shown-target/") },
                "omniverse://hidden.example.com/": {
                    "target": env.resource_url("hidden-target/"),
                    "list_in_top_level_addresses": false,
                },
            },
        }))
        .start();
    let client = storage_service.capabilities_v1alpha_client(Interface::Grpc).await;
    let tla = |address: String| TopLevelAddressEntry {
        top_level_address: address,
    };

    let response = client
        .list_top_level_addresses(Default::default())
        .await
        .expect("top level addresses listing failed");

    assert_eq!(
        response.items,
        vec![
            tla(format!("http://{}:{}/{}", env.host, env.port, env.bucket)),
            tla("omniverse://shown.example.com/".into()),
        ]
    );
}

#[test_log::test(tokio::test)]
async fn test_alias_hidden_from_top_level_addresses_is_still_routed() {
    use common::clients::capabilities::types::v1alpha::{route::RouteType, Route};

    let env = common::env::get_env().unwrap_s3();
    let storage_service = StorageService::builder()
        .enable_s3(false)
        .with_url_remapper_config(serde_json::json!({
            "enabled": true,
            "rewrite_rules": {
                "omniverse://shown.example.com/": { "target": env.resource_url("shown-target/") },
                "omniverse://hidden.example.com/": {
                    "target": env.resource_url("hidden-target/"),
                    "list_in_top_level_addresses": false,
                },
            },
        }))
        .start();
    let client = storage_service.capabilities_v1alpha_client(Interface::Grpc).await;
    let route = |pattern: &str| Route {
        route_type: Some(RouteType::WildcardPattern(pattern.into())),
    };

    let response = client
        .list_routes(Default::default())
        .await
        .expect("routes listing failed");

    let mut items = response.items;
    items.sort_by_key(|route| match &route.route_type {
        Some(RouteType::WildcardPattern(pattern)) => pattern.clone(),
        None => String::new(),
    });
    assert_eq!(
        items,
        vec![
            route(&format!("http://{}:{}/{}/**", env.host, env.port, env.bucket)),
            route("https://*.blob.core.windows.net/**"),
            route("https://*.s3-*.amazonaws.com/**"),
            route("https://*.s3.*.amazonaws.com/**"),
            route("omniverse://hidden.example.com/**"),
            route("omniverse://shown.example.com/**"),
        ]
    );
}

#[rstest]
#[case::allowed(Decision::Allow, true)]
#[case::missing(Decision::Missing, false)]
#[case::denied(Decision::Deny, false)]
#[test_log::test(tokio::test)]
async fn test_alias_visibility_in_top_level_addresses_follows_target_permission(
    #[case] target_decision: Decision,
    #[case] expected_listed: bool,
) {
    use common::clients::capabilities::types::v1alpha::TopLevelAddressEntry;

    let env = common::env::get_env().unwrap_s3();
    let permission_context = PermissionContext::new();
    let permission_service = GrpcPermissionService::new(permission_context.clone());
    let rule_set = permission_context.new_rule_set();
    let target_url = env.resource_url("alias-target/");
    rule_set.set_folder_read(&target_url, target_decision);
    let storage_service = StorageService::builder()
        .enable_s3(false)
        .enable_url_remapper("omniverse://alias.example.com/", &target_url)
        .enable_permissions("grpc", permission_service.endpoint_url(), false)
        .start();
    let client = storage_service.capabilities_v1alpha_client(Interface::Grpc).await;
    let entry = TopLevelAddressEntry {
        top_level_address: "omniverse://alias.example.com/".into(),
    };

    let response = client
        .list_top_level_addresses(Default::default())
        .await
        .expect("top level addresses listing failed");

    assert_eq!(response.items.contains(&entry), expected_listed);
}

/// A rewrite rule takes precedence over a backend address it shadows. When a rule overrides an
/// existing container under a declared account, that container is advertised only when the caller
/// may read the rule's *target* -- never merely because it can read the shadowed container itself.
#[rstest]
#[case::target_allowed(Decision::Allow, true)]
#[case::target_missing(Decision::Missing, false)]
#[case::target_denied(Decision::Deny, false)]
#[test_log::test(tokio::test)]
async fn test_container_shadowed_by_rule_follows_target_permission(
    #[case] target_decision: Decision,
    #[case] expected_listed: bool,
) {
    let env = common::env::get_env().unwrap_azure();
    let permission_context = PermissionContext::new();
    let permission_service = GrpcPermissionService::new(permission_context.clone());
    let rule_set = permission_context.new_rule_set();

    // The rule's source is an existing container under the declared account; its target lives on a
    // different host so the caller's access to the container cannot grant access to the target.
    let container = env.storage_container_url();
    let target = "omniverse://shadow-target.example.com/".to_string();

    // The caller can read the shadowed container itself -- so, without the rule, the backend would
    // list it -- while the rule's target follows `target_decision`.
    rule_set.allow_folder_read(&container);
    rule_set.set_folder_read(&target, target_decision);

    let storage_service = StorageService::builder()
        .enable_az_with_permissions(true, true)
        .enable_postgres_metadata()
        .enable_url_remapper(&container, &target)
        .enable_permissions("grpc", permission_service.endpoint_url(), false)
        .start();
    let client = storage_service.capabilities_v1alpha_client(Interface::Grpc).await;

    let response = client
        .list_top_level_addresses(Default::default())
        .await
        .expect("top level addresses listing failed");

    // The rule takes precedence over the container it shadows: the caller sees the container only
    // when it may read the rule's target, never via the backend's own entry for the container. The
    // container is matched in either form -- the backend lists a bucket/account root without a
    // trailing slash, while the rule advertises its source folder with one; neither may leak in on
    // the container's own permissions.
    let container_root = container.trim_end_matches('/');
    let listed = response
        .items
        .iter()
        .any(|item| item.top_level_address == container || item.top_level_address.as_str() == container_root);
    assert_eq!(listed, expected_listed);
}

#[rstest]
#[case::allowed(Decision::Allow, true)]
#[case::missing(Decision::Missing, false)]
#[case::denied(Decision::Deny, false)]
#[test_log::test(tokio::test)]
async fn test_alias_visibility_in_routes_follows_target_permission(
    #[case] target_decision: Decision,
    #[case] expected_listed: bool,
) {
    use common::clients::capabilities::types::v1alpha::{route::RouteType, Route};

    let env = common::env::get_env().unwrap_s3();
    let permission_context = PermissionContext::new();
    let permission_service = GrpcPermissionService::new(permission_context.clone());
    let rule_set = permission_context.new_rule_set();
    let target_url = env.resource_url("alias-target/");
    rule_set.set_folder_read(&target_url, target_decision);
    let storage_service = StorageService::builder()
        .enable_s3(false)
        .enable_url_remapper("omniverse://alias.example.com/", &target_url)
        .enable_permissions("grpc", permission_service.endpoint_url(), false)
        .start();
    let client = storage_service.capabilities_v1alpha_client(Interface::Grpc).await;
    let entry = Route {
        route_type: Some(RouteType::WildcardPattern("omniverse://alias.example.com/**".into())),
    };

    let response = client
        .list_routes(Default::default())
        .await
        .expect("routes listing failed");

    assert_eq!(response.items.contains(&entry), expected_listed);
}

#[test_log::test(tokio::test)]
async fn test_alias_of_bypassed_endpoint_listed_in_top_level_addresses_without_acls() {
    use common::clients::capabilities::types::v1alpha::TopLevelAddressEntry;

    let az_env = common::env::get_env().unwrap_azure();
    let permission_context = PermissionContext::new();
    let permission_service = GrpcPermissionService::new(permission_context.clone());
    let storage_service = StorageService::builder()
        .enable_az_with_permissions(false, false)
        .enable_url_remapper("omniverse://alias.example.com/", az_env.resource_url("alias-target/"))
        .enable_permissions("grpc", permission_service.endpoint_url(), false)
        .start();
    let client = storage_service.capabilities_v1alpha_client(Interface::Grpc).await;
    let entry = TopLevelAddressEntry {
        top_level_address: "omniverse://alias.example.com/".into(),
    };

    // No ACLs are configured; the alias of the bypassed endpoint is listed anyway.
    let response = client
        .list_top_level_addresses(Default::default())
        .await
        .expect("top level addresses listing failed");

    assert!(response.items.contains(&entry));
}

#[test_log::test(tokio::test)]
async fn test_alias_of_bypassed_endpoint_listed_in_routes_despite_deny_rule() {
    use common::clients::capabilities::types::v1alpha::{route::RouteType, Route};

    let az_env = common::env::get_env().unwrap_azure();
    let permission_context = PermissionContext::new();
    let permission_service = GrpcPermissionService::new(permission_context.clone());
    let rule_set = permission_context.new_rule_set();
    let target_url = az_env.resource_url("alias-target/");
    rule_set.forbid_folder_read(&target_url);
    let storage_service = StorageService::builder()
        .enable_az_with_permissions(false, false)
        .enable_url_remapper("omniverse://alias.example.com/", &target_url)
        .enable_permissions("grpc", permission_service.endpoint_url(), false)
        .start();
    let client = storage_service.capabilities_v1alpha_client(Interface::Grpc).await;
    let entry = Route {
        route_type: Some(RouteType::WildcardPattern("omniverse://alias.example.com/**".into())),
    };

    let response = client
        .list_routes(Default::default())
        .await
        .expect("routes listing failed");

    assert!(response.items.contains(&entry));
}

#[rstest]
#[case("omniverse://myhost.mydomain", "omniverse://myhost.mydomain/**")]
#[case("omniverse://myhost.mydomain/", "omniverse://myhost.mydomain/**")]
#[case("omniverse://myhost.mydomain/root/", "omniverse://myhost.mydomain/root/**")]
#[test_log::test(tokio::test)]
async fn test_list_routest_returns_a_correct_entry(#[case] mapped_address: &str, #[case] expected_route: &str) {
    use common::clients::capabilities::types::v1alpha::{route::RouteType, Route};

    let storage_service = StorageService::builder()
        .enable_az(true)
        .enable_postgres_metadata()
        .enable_url_remapper(mapped_address, "https://localhost.localdomain")
        .start();
    let client = storage_service.capabilities_v1alpha_client(Interface::Grpc).await;

    let response = client
        .list_routes(Default::default())
        .await
        .expect("routes listing failed");

    assert!(response.items.contains(&Route {
        route_type: Some(RouteType::WildcardPattern(expected_route.into())),
    }));
}

#[test_log::test(tokio::test)]
async fn test_stat_doesnt_error_on_metadata_routing_miss() {
    use common::clients::fileobject::types::v1alpha::StatRequest;

    let env = common::env::get_env();
    let az_env = env.unwrap_azure();
    let s3_env = env.unwrap_s3();
    let pg_env = env.unwrap_postgres();

    let s3_mapped_url_prefix = "omniverse://s3/mapped/";
    let s3_orig_url_prefix = s3_env.resource_url("original/");

    let service = StorageService::start_with_config(serde_json::json!({
        "rest": {
            "addr": "0.0.0.0",
            "port": 0,
        },
        "grpc": {
            "addr": "0.0.0.0",
            "port": 0,
        },
        "storage": {
            "s3": {
                "endpoints": {
                    s3_env.bucket_url(): {
                        "credentials": {
                            "access_key": {
                                "access_key_id": s3_env.access_key_id,
                                "secret_access_key": s3_env.secret_access_key,
                            },
                        },
                        "endpoint_url": s3_env.endpoint_url(),
                        "addressing_style": "path",
                    },
                },
            },
            "azure_blob": {
                "endpoints": {
                    az_env.storage_account_url(): {
                        "credentials": {
                            "anonymous": true,
                        },
                        "metadata_backend": "metadata-pg",
                    },
                },
            },
        },
        "metadata": {
            "metadata-pg": {
                "postgres": {
                    "host": pg_env.host,
                    "port": pg_env.port,
                    "dbname": pg_env.dbname,
                    "credentials": {
                        "password": {
                            "user": pg_env.user,
                            "password": pg_env.password,
                        },
                    },
                },
            },
        },
        "url_remapper": {
            "enabled": true,
            "rewrite_rules": {
                s3_mapped_url_prefix: { "target": s3_orig_url_prefix.clone() },
            },
        },
    }));

    let client = service.fileobject_v1alpha_client(Interface::Grpc).await;
    let path = get_random_path();
    let original_address = format!("{s3_orig_url_prefix}{path}");
    let mapped_address = format!("{s3_mapped_url_prefix}{path}");

    // Create a file using the original address. The request must succeed.
    client
        .write_random_object(&original_address, 32)
        .await
        .expect("object write failed");

    // Stat the file by the mapped address. The request must also succeed.
    client
        .stat(StatRequest {
            resource_address: mapped_address,
        })
        .await
        .expect("object stat failed");
}

#[test_log::test(tokio::test)]
async fn test_enumerate_versions_rewrites_unversioned_addresses() {
    use common::{
        clients::versioning::types::v1alpha::*,
        url::{get_query_string, UrlBuilder},
    };
    use serde_json::json;

    let s3_env = get_env().unwrap_s3_unversioned();
    let service = StorageService::start_with_config(json!({
        "storage": {
            "s3": {
                "endpoints": {
                    s3_env.bucket_url(): {
                        "credentials": {
                            "access_key": {
                                "access_key_id": &s3_env.access_key_id,
                                "secret_access_key": &s3_env.secret_access_key,
                            },
                        },
                        "endpoint_url": s3_env.endpoint_url(),
                        "addressing_style": "path",
                    },
                },
            },
        },
        "url_remapper": {
            "enabled": true,
            "rewrite_rules": {
                "https://myhost.mydomain": {
                    "target": s3_env.bucket_url(),
                },
            },
        }
    }));
    let fileobject_client = service.fileobject_v1alpha_client(Interface::Grpc).await;
    let versioning_client = service.versioning_v1alpha_client(Interface::Grpc).await;
    let object_path = get_random_path_item();

    // Create an object at the specified path in the source bucket.
    let original_address = UrlBuilder::new(s3_env.bucket_url()).path(&object_path).to_string();
    let (resource_info, _) = fileobject_client
        .write_random_object(original_address, 32)
        .await
        .unwrap();

    // List versions of the object using the bucket alias. We must see an unversioned resource address
    // and a resource identity with the original parameters.
    let mapped_address_base = UrlBuilder::new("https://myhost.mydomain/").path(&object_path);
    let mapped_address = mapped_address_base.to_string();
    let response = versioning_client
        .enumerate_versions_collect(EnumerateVersionsRequest {
            resource_address: mapped_address.clone(),
        })
        .await
        .unwrap();

    assert_eq!(response.items.len(), 1);
    assert_eq!(response.items[0].resource_address.as_ref().unwrap(), &mapped_address);

    let mapped_identity = mapped_address_base
        .raw_query(get_query_string(&resource_info.unwrap_identity()))
        .to_string();
    assert_eq!(
        response.items[0]
            .resource_info
            .as_ref()
            .unwrap()
            .resource_identity
            .as_ref()
            .unwrap()
            .encoded_identity,
        mapped_identity,
    );
}

/// Create objects for the specified addresses and return the list of their identities.
async fn create_objects(
    client: &fileobject::v1alpha::Client,
    addresses: impl IntoIterator<Item = impl Into<String>>,
) -> Vec<String> {
    let mut identities = Vec::new();
    for address in addresses.into_iter() {
        identities.push(
            client
                .write_random_object(address, 32)
                .await
                .expect("object write failed")
                .0
                .unwrap_identity(),
        );
    }

    identities
}

/// Write against a mapped address and read from the source identity.
async fn assert_read_original_after_write_mapped_succeeds(
    client: &fileobject::v1alpha::Client,
    write_address: impl Into<String>,
    read_address: impl Display,
    write_previous_vesion: Option<String>,
) {
    use crate::common::clients::fileobject::types::v1alpha::{ResourceIdentity, UploadPreference, WriteParameters};

    let write_content = get_random_bytes(32);
    let (write_info, write_method) = client
        .write_object(
            WriteParameters {
                destination_resource_address: write_address.into(),
                previous_version: write_previous_vesion.map(|identity| ResourceIdentity {
                    encoded_identity: identity,
                }),
                data_object_size: write_content.len() as u64,
                upload_preference: Some(UploadPreference::Body as i32),
                fail_if_exists: None,
                native_metadata: Default::default(),
            },
            write_content.clone(),
        )
        .await
        .expect("object write failed");

    assert_eq!(write_method, UploadPreference::Body);

    let write_identity = write_info.unwrap_identity();
    let (_, read_content) = client
        .read_object(format!("{}?{}", read_address, get_query_string(&write_identity)))
        .await
        .expect("object read failed");

    assert_eq!(write_content, read_content);
}

/// Return the version id query segment (including the name and value of the parameter, separated with =) for the
/// specified url.
fn get_version_id_query_segment(url: &str) -> &str {
    get_query_string(url)
        .split('&')
        .find(|p| p.starts_with("version-id="))
        .expect("version-id query parameter is missing")
}

fn create_missing_checkpoint_metadata_batch_error_entry(checkpoint: usize) -> BatchErrorEntry {
    use crate::common::clients::metadata::types::v1alpha::batch_error_entry::Code;

    BatchErrorEntry {
        code: Code::NotFound as i32,
        message: format!("checkpoint metadata is missing for #{checkpoint}"),
    }
}

fn create_metadata_modified_batch_error_entry() -> BatchErrorEntry {
    use crate::common::clients::metadata::types::v1alpha::batch_error_entry::Code;

    BatchErrorEntry {
        code: Code::FailedPrecondition as i32,
        message: "metadata was modified".into(),
    }
}

fn create_get_metadata_batch_resource_address_request_entry(
    uri: impl Into<String>,
    key: impl Into<String>,
) -> GetMetadataBatchRequestEntry {
    GetMetadataBatchRequestEntry {
        resource_location: Some(ResourceLocation::resource_address(uri)),
        user_metadata_key: key.into(),
        namespace: None,
    }
}

fn create_get_metadata_batch_resource_identity_request_entry(
    uri: impl Into<String>,
    key: impl Into<String>,
) -> GetMetadataBatchRequestEntry {
    GetMetadataBatchRequestEntry {
        resource_location: Some(ResourceLocation::resource_identity(uri)),
        user_metadata_key: key.into(),
        namespace: None,
    }
}

fn create_get_metadata_batch_folder_address_request_entry(
    uri: impl Into<String>,
    key: impl Into<String>,
) -> GetMetadataBatchRequestEntry {
    GetMetadataBatchRequestEntry {
        resource_location: Some(ResourceLocation::folder_address(uri)),
        user_metadata_key: key.into(),
        namespace: None,
    }
}

fn create_update_metadata_batch_resource_address_request_entry(
    uri: impl Into<String>,
    key: impl Into<String>,
) -> UpdateMetadataBatchRequestEntry {
    UpdateMetadataBatchRequestEntry {
        resource_location: Some(ResourceLocation::resource_address(uri)),
        user_metadata_key: key.into(),
        user_metadata: Some(create_random_value()),
        expected_etag: None,
        namespace: None,
    }
}

fn create_update_metadata_batch_resource_identity_request_entry(
    uri: impl Into<String>,
    key: impl Into<String>,
) -> UpdateMetadataBatchRequestEntry {
    UpdateMetadataBatchRequestEntry {
        resource_location: Some(ResourceLocation::resource_identity(uri)),
        user_metadata_key: key.into(),
        user_metadata: Some(create_random_value()),
        expected_etag: None,
        namespace: None,
    }
}

fn create_update_metadata_batch_folder_address_request_entry(
    uri: impl Into<String>,
    key: impl Into<String>,
) -> UpdateMetadataBatchRequestEntry {
    UpdateMetadataBatchRequestEntry {
        resource_location: Some(ResourceLocation::folder_address(uri)),
        user_metadata_key: key.into(),
        user_metadata: Some(create_random_value()),
        expected_etag: None,
        namespace: None,
    }
}

fn create_get_metadata_batch_success_response_entry(
    request_index: u32,
    value: Value,
    etag: String,
) -> GetMetadataBatchResponseEntry {
    use crate::common::clients::metadata::types::v1alpha::{
        get_metadata_batch_response_entry::Outcome, UserMetadataValue,
    };

    GetMetadataBatchResponseEntry {
        request_index,
        outcome: Some(Outcome::Success(UserMetadataValue {
            value: Some(value),
            etag,
        })),
    }
}

fn create_get_metadata_batch_missing_checkpoint_metadata_response_entry(
    request_index: u32,
    checkpoint: usize,
) -> GetMetadataBatchResponseEntry {
    use crate::common::clients::metadata::types::v1alpha::get_metadata_batch_response_entry::Outcome;

    GetMetadataBatchResponseEntry {
        outcome: Some(Outcome::Error(create_missing_checkpoint_metadata_batch_error_entry(
            checkpoint,
        ))),
        request_index,
    }
}

fn create_delete_metadata_batch_resource_address_request_entry(
    uri: impl Into<String>,
    key: impl Into<String>,
) -> DeleteMetadataBatchRequestEntry {
    DeleteMetadataBatchRequestEntry {
        resource_location: Some(ResourceLocation::resource_address(uri)),
        user_metadata_key: key.into(),
        expected_etag: None,
        namespace: None,
    }
}

fn create_delete_metadata_batch_resource_identity_request_entry(
    uri: impl Into<String>,
    key: impl Into<String>,
) -> DeleteMetadataBatchRequestEntry {
    DeleteMetadataBatchRequestEntry {
        resource_location: Some(ResourceLocation::resource_identity(uri)),
        user_metadata_key: key.into(),
        expected_etag: None,
        namespace: None,
    }
}

fn create_delete_metadata_batch_folder_address_request_entry(
    uri: impl Into<String>,
    key: impl Into<String>,
) -> DeleteMetadataBatchRequestEntry {
    DeleteMetadataBatchRequestEntry {
        resource_location: Some(ResourceLocation::folder_address(uri)),
        user_metadata_key: key.into(),
        expected_etag: None,
        namespace: None,
    }
}

fn create_delete_metadata_batch_success_response_entry(request_index: u32) -> DeleteMetadataBatchResponseEntry {
    use crate::common::clients::metadata::types::v1alpha::delete_metadata_batch_response_entry::Outcome;

    DeleteMetadataBatchResponseEntry {
        request_index,
        outcome: Some(Outcome::Success(Default::default())),
    }
}

fn create_delete_metadata_batch_metadata_modified_response_entry(
    request_index: u32,
) -> DeleteMetadataBatchResponseEntry {
    use crate::common::clients::metadata::types::v1alpha::delete_metadata_batch_response_entry::Outcome;

    DeleteMetadataBatchResponseEntry {
        request_index,
        outcome: Some(Outcome::Error(create_metadata_modified_batch_error_entry())),
    }
}

fn create_delete_metadata_batch_missing_checkpoint_metadata_response_entry(
    request_index: u32,
    checkpoint: usize,
) -> DeleteMetadataBatchResponseEntry {
    use crate::common::clients::metadata::types::v1alpha::delete_metadata_batch_response_entry::Outcome;

    DeleteMetadataBatchResponseEntry {
        request_index,
        outcome: Some(Outcome::Error(create_missing_checkpoint_metadata_batch_error_entry(
            checkpoint,
        ))),
    }
}

fn create_query_metadata_response_entry(
    key: impl Into<String>,
    value: impl Into<Value>,
    etag: impl Into<String>,
) -> QueryMetadataResponseEntry {
    use crate::common::clients::metadata::types::v1alpha::*;

    QueryMetadataResponseEntry {
        user_metadata: Some(UserMetadataValue {
            value: Some(value.into()),
            etag: etag.into(),
        }),
        user_metadata_key: key.into(),
    }
}

fn create_random_value() -> Value {
    use storage_service_protos::google::protobuf::value::Kind;

    Value {
        kind: Some(Kind::StringValue(get_random_alphanumeric_string(32))),
    }
}
