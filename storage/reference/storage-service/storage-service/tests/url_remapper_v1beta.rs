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

use std::{collections::HashSet, fmt::Display};

use common::{clients::fileobject::types::v1beta::UnwrapIdentityExt, types::Interface};
use futures::TryStreamExt;
use rstest::{fixture, rstest};
use storage_service_test::mocks::permissions::{
    grpc::Service as GrpcPermissionService, Context as PermissionContext, Decision,
};
use tonic::Code;

use crate::common::{
    clients::fileobject,
    env::get_env,
    random::{get_random_bytes, get_random_path, get_random_path_item},
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
    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;

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
    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;
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
    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;
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
    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;
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
    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;

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
async fn read_from_mapped_unknown_native_identity(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    let path = get_random_path();
    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;

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
    use crate::common::clients::fileobject::types::v1beta::StatRequest;

    let path = get_random_path();
    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;
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
    use crate::common::clients::fileobject::types::v1beta::StatRequest;

    let path = get_random_path();
    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;
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
    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;

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
    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;

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
    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;
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
    use crate::common::clients::fileobject::types::v1beta::{ResourceIdentity, UploadPreference, WriteParameters};

    let path = get_random_path();
    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;

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
    use crate::common::clients::fileobject::types::v1beta::{UploadPreference, WriteParameters};

    let path = get_random_path();
    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;

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
    use crate::common::clients::fileobject::types::v1beta::{UploadPreference, WriteParameters};

    let path = get_random_path();
    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;

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
    use crate::common::clients::fileobject::types::v1beta::{DeleteRequest, StatRequest};

    let path = get_random_path();
    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;

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
    use crate::common::clients::fileobject::types::v1beta::FetchWriteTypeInfoRequest;

    let path = get_random_path();
    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;

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
async fn enumerate_rewrites_resource_addresses(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::fileobject::types::v1beta::EnumerateRequest;

    let folder_path = get_random_path_item();
    let object_path_1 = format!("{}/{}", folder_path, get_random_path_item());
    let object_path_2 = format!("{}/{}", folder_path, get_random_path_item());
    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;

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
    use crate::common::clients::filefolder::types::v1beta::{FolderAddress, ListRequest};

    let folder_path = get_random_path_item();
    let object_path_1 = format!("{}/{}", folder_path, get_random_path_item());
    let object_path_2 = format!("{}/{}", folder_path, get_random_path_item());
    let subfolder_path = format!("{}/{}", folder_path, get_random_path_item());
    let object_path_3 = format!("{}/{}", subfolder_path, get_random_path_item());
    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;
    let filefolder_client = storage_service.filefolder_v1beta_client(interface).await;

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
    use crate::common::clients::filefolder::types::v1beta::{FolderAddress, ListStatRequest};

    let folder_path = get_random_path_item();
    let object_path_1 = format!("{}/{}", folder_path, get_random_path_item());
    let object_path_2 = format!("{}/{}", folder_path, get_random_path_item());
    let subfolder_path = format!("{}/{}", folder_path, get_random_path_item());
    let object_path_3 = format!("{}/{}", subfolder_path, get_random_path_item());
    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;
    let filefolder_client = storage_service.filefolder_v1beta_client(interface).await;
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
async fn list_versions_rewrites_identities(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use storage_core::traits::versioning::v1beta::EnumerateVersionsRequest;

    let path = get_random_path();
    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;
    let versioning_client = storage_service.versioning_v1beta_client(interface).await;
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
    use storage_core::traits::fileobject::v1beta::StatRequest;

    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;
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
    use storage_core::traits::fileobject::v1beta::StatRequest;

    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;
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
    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;
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
    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;
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
    use storage_core::traits::versioning::v1beta::EnumerateVersionsRequest;

    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;
    let versioning_client = storage_service.versioning_v1beta_client(interface).await;
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
            item.resource_info
                .expect("resource info is missing from a response")
                .resource_identity
                .expect("resource identity is missing from resource info")
                .encoded_identity
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
    use storage_core::traits::versioning::v1beta::EnumerateVersionsRequest;

    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;
    let versioning_client = storage_service.versioning_v1beta_client(interface).await;
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
            item.resource_info
                .expect("resource info is missing from a response")
                .resource_identity
                .expect("resource identity is missing from resource info")
                .encoded_identity
        })
        .collect();

    let expected_version_addresses_and_identities: Vec<_> = [
        resource_info_1.unwrap_identity(),
        resource_info_2.unwrap_identity(),
        resource_info_3.unwrap_identity(),
    ]
    .into_iter()
    .map(|original_identity| {
        storage_service
            .remapper_mapped_url_builder()
            .path(&path)
            .raw_query(get_query_string(&original_identity))
            .to_string()
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
    use storage_core::traits::filefolder::v1beta::{FolderAddress, ListRequest, ListResponse};

    let folder_path = get_random_path();
    let storage_service = StorageService::builder()
        .enable_az(true)
        .enable_az_url_remapper(mapped_prefix, format!("{}/", folder_path))
        .enable_postgres_metadata()
        .start();

    let fileobject_client = storage_service.fileobject_v1beta_client(Interface::Grpc).await;
    let filefolder_client = storage_service.filefolder_v1beta_client(Interface::Grpc).await;
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
    use storage_core::traits::filefolder::v1beta::{FolderAddress, ListStatRequest};

    let folder_path = get_random_path();
    let storage_service = StorageService::builder()
        .enable_az(true)
        .enable_az_url_remapper(mapped_prefix, format!("{}/", folder_path))
        .enable_postgres_metadata()
        .start();

    let fileobject_client = storage_service.fileobject_v1beta_client(Interface::Grpc).await;
    let filefolder_client = storage_service.filefolder_v1beta_client(Interface::Grpc).await;
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
    use storage_core::traits::fileobject::v1beta::EnumerateRequest;

    let folder_path = get_random_path();
    let storage_service = StorageService::builder()
        .enable_az(true)
        .enable_az_url_remapper(mapped_prefix, format!("{}/", folder_path))
        .enable_postgres_metadata()
        .start();

    let fileobject_client = storage_service.fileobject_v1beta_client(Interface::Grpc).await;
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
    use common::clients::capabilities::types::v1beta::TopLevelAddressEntry;

    let storage_service = StorageService::builder()
        .enable_az(true)
        .enable_postgres_metadata()
        .enable_url_remapper(mapped_address, "https://localhost.localdomain")
        .start();
    let client = storage_service.capabilities_v1beta_client(Interface::Grpc).await;

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
    use common::clients::capabilities::types::v1beta::TopLevelAddressEntry;

    let env = get_env().unwrap_s3();
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
    let client = storage_service.capabilities_v1beta_client(Interface::Grpc).await;

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

#[test_log::test(tokio::test)]
async fn test_alias_hidden_from_top_level_addresses() {
    use common::clients::capabilities::types::v1beta::TopLevelAddressEntry;

    let env = get_env().unwrap_s3();
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
    let client = storage_service.capabilities_v1beta_client(Interface::Grpc).await;
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

#[rstest]
#[case::allowed(Decision::Allow, true)]
#[case::missing(Decision::Missing, false)]
#[case::denied(Decision::Deny, false)]
#[test_log::test(tokio::test)]
async fn test_alias_visibility_in_top_level_addresses_follows_target_permission(
    #[case] target_decision: Decision,
    #[case] expected_listed: bool,
) {
    use common::clients::capabilities::types::v1beta::TopLevelAddressEntry;

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
    let client = storage_service.capabilities_v1beta_client(Interface::Grpc).await;
    let entry = TopLevelAddressEntry {
        top_level_address: "omniverse://alias.example.com/".into(),
    };

    let response = client
        .list_top_level_addresses(Default::default())
        .await
        .expect("top level addresses listing failed");

    assert_eq!(response.items.contains(&entry), expected_listed);
}

#[test_log::test(tokio::test)]
async fn test_alias_of_bypassed_endpoint_listed_in_top_level_addresses_without_acls() {
    use common::clients::capabilities::types::v1beta::TopLevelAddressEntry;

    let az_env = common::env::get_env().unwrap_azure();
    let permission_context = PermissionContext::new();
    let permission_service = GrpcPermissionService::new(permission_context.clone());
    let storage_service = StorageService::builder()
        .enable_az_with_permissions(false, false)
        .enable_url_remapper("omniverse://alias.example.com/", az_env.resource_url("alias-target/"))
        .enable_permissions("grpc", permission_service.endpoint_url(), false)
        .start();
    let client = storage_service.capabilities_v1beta_client(Interface::Grpc).await;
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
async fn test_stat_does_not_remap_checkpointed_addresses_for_non_omniverse_schemes() {
    use common::clients::fileobject::types::v1beta::StatRequest;

    let az_env = get_env().unwrap_azure();
    let storage_service = StorageService::builder()
        .enable_az(true)
        .enable_url_remapper("https://myhost.mydomain/mapped/", az_env.resource_url("original/"))
        .enable_postgres_metadata()
        .start();
    let fileobject_client = storage_service.fileobject_v1beta_client(Interface::Grpc).await;
    let metadata_client = storage_service.metadata_v1alpha_client(Interface::Grpc).await;
    let file_path = get_random_path();

    // Create two versions of the same object and "checkpoint" the first one.
    let original_address = storage_service
        .remapper_original_url_builder()
        .path(&file_path)
        .to_string();
    let (original_resource_info, _) = fileobject_client
        .write_random_object(&original_address, 1)
        .await
        .expect("object write failed");
    let original_identity = original_resource_info.unwrap_identity();
    fileobject_client
        .write_random_object(&original_address, 2)
        .await
        .expect("object write failed");

    metadata_client
        .set_checkpoint_metadata(&original_address, [(1, original_identity)])
        .await
        .expect("metadata set failed");

    // Attempt to read the first checkpoint. We must read the newest version since the backend ignores unknown query
    // parameters, checkpoint included.
    let mapped_address = storage_service
        .remapper_mapped_url_builder()
        .path(file_path)
        .checkpoint_query(1)
        .to_string();
    let response = fileobject_client
        .stat(StatRequest {
            resource_address: mapped_address,
        })
        .await
        .expect("stat failed");

    assert_eq!(
        response.resource_info.unwrap().metadata.unwrap().data_object_size,
        Some(2)
    );
}

#[test_log::test(tokio::test)]
async fn test_stat_checkpointed_address_with_disabled_metadata_backend() {
    use common::clients::fileobject::types::v1beta::StatRequest;

    let storage_service = StorageService::builder()
        .enable_az(false)
        .enable_az_url_remapper("mapped/", "original/")
        .start();
    let client = storage_service.fileobject_v1beta_client(Interface::Grpc).await;
    let file_path = get_random_path();

    // Write a file at the original location and ensuresure that it's reachable.
    let original_address = storage_service.az_url_builder().path(&file_path).to_string();
    client
        .write_random_object(&original_address, 32)
        .await
        .expect("object write failed");
    client
        .stat(StatRequest {
            resource_address: original_address,
        })
        .await
        .expect("object stat failed");

    // Create a checkpointed resource address and read it. The operation must fail with a not found error.
    let mapped_address = storage_service
        .remapper_mapped_url_builder()
        .path(file_path)
        .checkpoint_query(1)
        .to_string();
    let error = client
        .stat(StatRequest {
            resource_address: mapped_address,
        })
        .await
        .expect_err("stat is expected to fail");

    // TODO: a better error code?
    assert_eq!(error.code(), Code::InvalidArgument);
}

#[test_log::test(tokio::test)]
async fn test_list_stat_does_not_remap_checkpointed_addresses_for_non_omniverse_schemes() {
    use common::clients::filefolder::types::v1beta::{FolderAddress, ListStatRequest};

    let az_env = get_env().unwrap_azure();
    let storage_service = StorageService::builder()
        .enable_az(true)
        .enable_url_remapper("https://myhost.mydomain/mapped/", az_env.resource_url("original/"))
        .enable_postgres_metadata()
        .start();
    let fileobject_client = storage_service.fileobject_v1beta_client(Interface::Grpc).await;
    let filefolder_client = storage_service.filefolder_v1beta_client(Interface::Grpc).await;
    let metadata_client = storage_service.metadata_v1alpha_client(Interface::Grpc).await;
    let folder_name = get_random_path_item();
    let object_name = get_random_path_item();

    // Create an object in a folder and checkpoint it.
    let original_object_address = storage_service
        .remapper_original_url_builder()
        .path(format!("{}/{}", folder_name, object_name))
        .to_string();
    let (original_resource_info, _) = fileobject_client
        .write_random_object(&original_object_address, 32)
        .await
        .expect("object write failed");
    let original_resource_identity = original_resource_info.unwrap_identity();

    metadata_client
        .set_checkpoint_metadata(original_object_address, [(1, &original_resource_identity)])
        .await
        .expect("metadata set failed");

    // ListStat objects in the folder. We must see the original query parameters even though the checkpoint
    // mapping exists.
    let mapped_folder_address = storage_service
        .remapper_mapped_url_builder()
        .path(&folder_name)
        .to_string();
    let response = filefolder_client
        .list_stat_collect(ListStatRequest {
            folder: Some(FolderAddress {
                uri: mapped_folder_address,
            }),
        })
        .await
        .expect("liststat failed");

    assert!(response.subfolder_addresses.is_empty());

    let object_identities: Vec<_> = response
        .entries
        .into_iter()
        .map(|item| {
            item.resource_info
                .expect("resource info is missing")
                .resource_identity
                .expect("resource identity is missing")
                .encoded_identity
        })
        .collect();
    let expected_object_identities = [storage_service
        .remapper_mapped_url_builder()
        .path(format!("{}/{}", folder_name, object_name))
        .raw_query_params(get_stat_query_pairs(&original_resource_identity))
        .to_string()];
    assert_eq!(object_identities, expected_object_identities);
}

#[test_log::test(tokio::test)]
async fn test_list_stat_does_not_remap_query_if_metadata_backend_is_disabled() {
    use common::clients::filefolder::types::v1beta::{FolderAddress, ListStatRequest};

    let storage_service = StorageService::builder()
        .enable_az(false)
        .enable_az_url_remapper("mapped/", "original/")
        .start();
    let fileobject_client = storage_service.fileobject_v1beta_client(Interface::Grpc).await;
    let filefolder_client = storage_service.filefolder_v1beta_client(Interface::Grpc).await;
    let folder_name = get_random_path_item();
    let object_name = get_random_path_item();

    // Create an object in a folder.
    let original_object_address = storage_service
        .remapper_original_url_builder()
        .path(format!("{}/{}", folder_name, object_name))
        .to_string();
    let (original_resource_info, _) = fileobject_client
        .write_random_object(&original_object_address, 32)
        .await
        .expect("object write failed");
    let original_resource_identity = original_resource_info.unwrap_identity();

    // ListStat objects in the folder. We must see the original query parameters even though the remapper uses
    // the omniverse scheme.
    let mapped_folder_address = storage_service
        .remapper_mapped_url_builder()
        .path(&folder_name)
        .to_string();
    let response = filefolder_client
        .list_stat_collect(ListStatRequest {
            folder: Some(FolderAddress {
                uri: mapped_folder_address,
            }),
        })
        .await
        .expect("liststat failed");

    assert!(response.subfolder_addresses.is_empty());

    let object_identities: Vec<_> = response
        .entries
        .into_iter()
        .map(|item| {
            item.resource_info
                .expect("resource info is missing")
                .resource_identity
                .expect("resource identity is missing")
                .encoded_identity
        })
        .collect();
    let expected_object_identities = [storage_service
        .remapper_mapped_url_builder()
        .path(format!("{}/{}", folder_name, object_name))
        .raw_query_params(get_stat_query_pairs(&original_resource_identity))
        .to_string()];
    assert_eq!(object_identities, expected_object_identities);
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
        clients::versioning::types::v1beta::*,
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
    let fileobject_client = service.fileobject_v1beta_client(Interface::Grpc).await;
    let versioning_client = service.versioning_v1beta_client(Interface::Grpc).await;
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
    client: &fileobject::v1beta::Client,
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
    client: &fileobject::v1beta::Client,
    write_address: impl Into<String>,
    read_address: impl Display,
    write_previous_vesion: Option<String>,
) {
    use crate::common::clients::fileobject::types::v1beta::{ResourceIdentity, UploadPreference, WriteParameters};

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
