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

use crate::common::{random::get_random_path_item, service::Service as StorageService};
use common::clients::fileobject::types::v1beta::{StatRequest, UnwrapIdentityExt};
use common::types::Interface;
use futures::TryStreamExt;
use rstest::{fixture, rstest};
use tonic::Code;
use url_normalizer::normalize_url;

#[fixture]
#[once]
fn storage_service() -> StorageService {
    StorageService::builder()
        .enable_s3(true)
        .enable_postgres_metadata()
        .enable_url_normalization()
        .start()
}

/// Create a random path, non url-escaped.
pub fn get_random_non_normalized_path() -> String {
    format!("{}////.//{}", get_random_path_item(), get_random_path_item())
}

#[rstest]
#[test_log::test(tokio::test)]
async fn read_with_normalized_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    let path = get_random_non_normalized_path();
    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;
    let address = storage_service.s3_url_builder().path(&path).to_string();
    let (original_info, written_content) = fileobject_client
        .write_random_object(&address, 32)
        .await
        .expect("object write failed");
    let written_identity = original_info.unwrap_identity();
    assert!(written_identity.starts_with(&normalize_url(&address).unwrap()));
    // Read the object by the mapped address.
    let (identity_info, read_content) = fileobject_client
        .read_object_from_address(&address)
        .await
        .expect("object read failed");
    let identity = identity_info.unwrap_identity();

    assert!(identity.starts_with(&normalize_url(&address).unwrap()));
    assert_eq!(read_content, written_content);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn read_with_invalid_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::fileobject::types::v1beta::ReadFromAddressRequest;

    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;
    let address = "invalid-url.com".to_string();
    let status = fileobject_client
        .read_from_address(ReadFromAddressRequest {
            resource_address: address,
            ..Default::default()
        })
        .await
        .expect_err("read from address is expected to fail");
    assert_eq!(status.code(), Code::InvalidArgument);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn stat_with_normalized_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    let path = get_random_non_normalized_path();
    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;
    let address = storage_service.s3_url_builder().path(&path).to_string();
    let (original_info, _) = fileobject_client
        .write_random_object(&address, 32)
        .await
        .expect("object write failed");
    let written_identity = original_info.unwrap_identity();
    assert!(written_identity.starts_with(&normalize_url(&address).unwrap()));
    // Read the object by the mapped address.
    let identity = fileobject_client
        .stat(StatRequest {
            resource_address: address.clone(),
        })
        .await
        .expect("object stat failed")
        .resource_info
        .expect("resource info is missing")
        .unwrap_identity();

    assert!(identity.starts_with(&normalize_url(&address).unwrap()));
}

#[rstest]
#[case::double_dot("folder/child/../file")]
#[case::single_dot("folder/./file")]
#[case::empty_segment("folder//file")]
#[case::encoded_double_dot("folder%2Fchild%2f%2E%2e%2Ffile")]
#[case::encoded_single_dot("folder%2f%2E%2Ffile")]
#[case::encoded_empty_segment("folder%2f%2Ffile")]
#[test_log::test(tokio::test)]
async fn test_stat_collapses_path_segments(storage_service: &StorageService, #[case] path: &str) {
    use common::clients::fileobject::types::v1beta::StatRequest;

    let client = storage_service.fileobject_v1beta_client(Interface::Grpc).await;
    let folder_prefix = get_random_path_item();
    let short_address = storage_service
        .s3_url_builder()
        .path(format!("{folder_prefix}/folder/file"))
        .to_string();
    let long_address = storage_service
        .s3_url_builder()
        .raw_path(format!("{folder_prefix}/{path}"))
        .to_string();
    let (written_info, _) = client
        .write_random_object(&short_address, 32)
        .await
        .expect("object write failed");

    let resource_info = client
        .stat(StatRequest {
            resource_address: long_address,
        })
        .await
        .expect("object stat failed")
        .resource_info
        .expect("resource info is missing");

    assert_eq!(resource_info.unwrap_identity(), written_info.unwrap_identity());
}

#[test_log::test(tokio::test)]
async fn test_normalization_precedes_url_rewriting() {
    use common::clients::fileobject::types::v1beta::StatRequest;

    let service = StorageService::builder()
        .enable_s3(true)
        .enable_postgres_metadata()
        .enable_url_normalization()
        .enable_s3_url_remapper("mapped/prefix/", "original/")
        .start();

    let client = service.fileobject_v1beta_client(Interface::Grpc).await;
    let path = get_random_path_item();
    let address = service.remapper_mapped_url_builder().path(&path).to_string();
    let (written_info, _) = client
        .write_random_object(&address, 32)
        .await
        .expect("object write failed");

    let resource_info = client
        .stat(StatRequest {
            resource_address: format!("omniverse://myhost.mydomain/mapped//prefix/{path}"),
        })
        .await
        .expect("object stat failed")
        .resource_info
        .expect("resource info is missing");

    assert_eq!(resource_info.unwrap_identity(), written_info.unwrap_identity());
}

#[rstest]
#[test_log::test(tokio::test)]
async fn stat_with_invalid_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;
    let status = fileobject_client
        .stat(StatRequest {
            resource_address: "invalid-url.com".to_string(),
        })
        .await
        .expect_err("stat with invalid url is expected to fail");
    assert_eq!(status.code(), Code::InvalidArgument);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn delete_with_normalized_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::fileobject::types::v1beta::DeleteRequest;

    let path = get_random_non_normalized_path();
    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;
    let address = storage_service.s3_url_builder().path(&path).to_string();

    // Write an object first
    let (original_info, _) = fileobject_client
        .write_random_object(&address, 32)
        .await
        .expect("object write failed");
    let written_identity = original_info.unwrap_identity();
    assert!(written_identity.starts_with(&normalize_url(&address).unwrap()));

    // Delete the object using the non-normalized address
    fileobject_client
        .delete(DeleteRequest {
            resource_address: normalize_url(&address).unwrap(),
            previous_version: None,
        })
        .await
        .expect("object delete failed");

    // Verify the object was deleted by trying to stat it
    let stat_result = fileobject_client
        .stat(StatRequest {
            resource_address: address.clone(),
        })
        .await;

    assert!(stat_result.is_err(), "Expected object to be deleted but stat succeeded");
}

#[rstest]
#[test_log::test(tokio::test)]
async fn delete_with_invalid_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::fileobject::types::v1beta::DeleteRequest;

    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;
    let status = fileobject_client
        .delete(DeleteRequest {
            resource_address: "invalid-url.com".to_string(),
            ..DeleteRequest::default()
        })
        .await
        .expect_err("delete with invalid url is expected to fail");
    assert_eq!(status.code(), Code::InvalidArgument);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn abort_multipart_upload_with_normalized_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::fileobject::types::v1beta::{AbortMultipartUploadRequest, UploadPreference, WriteParameters};

    let path = get_random_non_normalized_path();
    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;
    let address = storage_service.s3_url_builder().path(&path).to_string();

    // Start a multipart upload
    let write_response = fileobject_client
        .write(
            WriteParameters {
                destination_resource_address: address.clone(),
                data_object_size: 1024 * 1024, // 1MB to ensure multipart
                upload_preference: Some(UploadPreference::Multipart as i32),
                ..Default::default()
            },
            vec![],
        )
        .await
        .expect("write request failed");

    // Extract the upload_id from the multipart response
    let upload_id = match write_response {
        common::clients::fileobject::types::v1beta::WriteResponse::Multipart(response) => response.upload_id,
        _ => panic!("Expected multipart upload response"),
    };

    // Abort the multipart upload using the non-normalized address
    fileobject_client
        .abort_multipart_upload(AbortMultipartUploadRequest {
            destination_resource_address: address.clone(),
            upload_id,
        })
        .await
        .expect("abort multipart upload failed");
}

#[rstest]
#[test_log::test(tokio::test)]
async fn abort_multipart_upload_with_invalid_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::fileobject::types::v1beta::AbortMultipartUploadRequest;

    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;
    let status = fileobject_client
        .abort_multipart_upload(AbortMultipartUploadRequest {
            destination_resource_address: "invalid-url.com".to_string(),
            upload_id: "upload-id".to_string(),
        })
        .await
        .expect_err("abort multipart upload with invalid url is expected to fail");
    assert_eq!(status.code(), Code::InvalidArgument);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn complete_multipart_upload_with_normalized_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::fileobject::types::v1beta::{
        CompleteMultipartUploadRequest, UploadPreference, WriteParameters,
    };

    let path = get_random_non_normalized_path();
    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;
    let address = storage_service.s3_url_builder().path(&path).to_string();

    // Start a multipart upload
    let write_response = fileobject_client
        .write(
            WriteParameters {
                destination_resource_address: address.clone(),
                data_object_size: 1024 * 1024, // 1MB to ensure multipart
                upload_preference: Some(UploadPreference::Multipart as i32),
                ..Default::default()
            },
            vec![],
        )
        .await
        .expect("write request failed");

    // Extract the upload_id from the multipart response
    let upload_id = match write_response {
        common::clients::fileobject::types::v1beta::WriteResponse::Multipart(response) => response.upload_id,
        _ => panic!("Expected multipart upload response"),
    };

    // Complete the multipart upload using the non-normalized address
    let complete_response = fileobject_client
        .complete_multipart_upload(CompleteMultipartUploadRequest {
            destination_resource_address: address.clone(),
            upload_id,
            parts: vec![],
        })
        .await
        .expect("complete multipart upload failed");

    // Verify the identity is normalized
    let identity = complete_response
        .resource_info
        .expect("resource info is missing")
        .unwrap_identity();
    assert!(identity.starts_with(&normalize_url(&address).unwrap()));
}

#[rstest]
#[test_log::test(tokio::test)]
async fn complete_multipart_upload_with_invalid_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::fileobject::types::v1beta::CompleteMultipartUploadRequest;

    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;
    let status = fileobject_client
        .complete_multipart_upload(CompleteMultipartUploadRequest {
            destination_resource_address: "invalid-url.com".to_string(),
            upload_id: "upload-id".to_string(),
            parts: vec![],
        })
        .await
        .expect_err("complete multipart upload with invalid url is expected to fail");
    assert_eq!(status.code(), Code::InvalidArgument);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn upload_part_with_normalized_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::fileobject::types::v1beta::{UploadPartRequest, UploadPreference, WriteParameters};

    let path = get_random_non_normalized_path();
    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;
    let address = storage_service.s3_url_builder().path(&path).to_string();

    // Start a multipart upload
    let write_response = fileobject_client
        .write(
            WriteParameters {
                destination_resource_address: address.clone(),
                data_object_size: 1024 * 1024, // 1MB to ensure multipart
                upload_preference: Some(UploadPreference::Multipart as i32),
                ..Default::default()
            },
            vec![],
        )
        .await
        .expect("write request failed");

    // Extract the upload_id from the multipart response
    let upload_id = match write_response {
        common::clients::fileobject::types::v1beta::WriteResponse::Multipart(response) => response.upload_id,
        _ => panic!("Expected multipart upload response"),
    };

    // Upload a part using the normalized address
    fileobject_client
        .upload_part(UploadPartRequest {
            destination_resource_address: normalize_url(&address).unwrap(),
            upload_id,
            part_number: 1,
            part_count: None,
        })
        .await
        .expect("upload part failed");
}

#[rstest]
#[test_log::test(tokio::test)]
async fn upload_part_with_invalid_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::fileobject::types::v1beta::UploadPartRequest;

    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;
    let status = fileobject_client
        .upload_part(UploadPartRequest {
            destination_resource_address: "invalid-url.com".to_string(),
            upload_id: "upload-id".to_string(),
            part_number: 1,
            part_count: None,
        })
        .await
        .expect_err("upload part with invalid url is expected to fail");
    assert_eq!(status.code(), Code::InvalidArgument);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn complete_redirect_upload_with_normalized_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use bytes::Bytes;
    use common::clients::fileobject::types::v1beta::{
        CompleteRedirectUploadRequest, UploadPreference, WriteParameters,
    };
    use common::clients::fileobject::v1beta::follow_write_redirect;

    let path = get_random_non_normalized_path();
    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;
    let address = storage_service.s3_url_builder().path(&path).to_string();

    // Start a redirect upload
    let content = Bytes::from(vec![0u8; 1024]);
    let write_response = fileobject_client
        .write(
            WriteParameters {
                destination_resource_address: address.clone(),
                data_object_size: content.len() as u64,
                upload_preference: Some(UploadPreference::Redirect as i32),
                ..Default::default()
            },
            content.clone(),
        )
        .await
        .expect("write request failed");

    // Extract the redirect properties from the response
    let redirect_params = match write_response {
        common::clients::fileobject::types::v1beta::WriteResponse::Redirect(props) => props,
        _ => panic!("Expected redirect upload response"),
    };

    // Follow the redirect to upload the data
    let completion_headers = follow_write_redirect(&reqwest::Client::new(), content, redirect_params)
        .await
        .expect("redirect upload failed");

    // Complete the redirect upload using the non-normalized address
    let complete_response = fileobject_client
        .complete_redirect_upload(CompleteRedirectUploadRequest {
            destination_resource_address: address.clone(),
            additional_headers: completion_headers,
        })
        .await
        .expect("complete redirect upload failed");

    // Verify the identity is normalized
    let identity = complete_response
        .resource_info
        .expect("resource info is missing")
        .unwrap_identity();
    assert!(identity.starts_with(&normalize_url(&address).unwrap()));
}

#[rstest]
#[test_log::test(tokio::test)]
async fn complete_redirect_upload_with_invalid_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::fileobject::types::v1beta::CompleteRedirectUploadRequest;

    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;
    let status = fileobject_client
        .complete_redirect_upload(CompleteRedirectUploadRequest {
            destination_resource_address: "invalid-url.com".to_string(),
            additional_headers: vec![],
        })
        .await
        .expect_err("complete redirect upload with invalid url is expected to fail");
    assert_eq!(status.code(), Code::InvalidArgument);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn fetch_write_type_info_with_normalized_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::fileobject::types::v1beta::FetchWriteTypeInfoRequest;

    let path = get_random_non_normalized_path();
    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;
    let address = storage_service.s3_url_builder().path(&path).to_string();

    // Fetch write type info using the non-normalized address
    let response = fileobject_client
        .fetch_write_type_info(FetchWriteTypeInfoRequest {
            destination_resource_address: address.clone(),
        })
        .await
        .expect("fetch write type info failed");

    // Verify that the response contains valid write type intervals
    assert!(
        !response.write_type_intervals.is_empty(),
        "write type intervals are empty"
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn fetch_write_type_info_with_invalid_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::fileobject::types::v1beta::FetchWriteTypeInfoRequest;

    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;
    let status = fileobject_client
        .fetch_write_type_info(FetchWriteTypeInfoRequest {
            destination_resource_address: "invalid-url.com".to_string(),
        })
        .await
        .expect_err("fetch write type info with invalid url is expected to fail");
    assert_eq!(status.code(), Code::InvalidArgument);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn enumerate_with_normalized_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::fileobject::types::v1beta::EnumerateRequest;

    let folder_path = get_random_non_normalized_path();
    let folder_address = storage_service.s3_url_builder().path(&folder_path).to_string();
    let object_path_1 = format!("{}/{}", folder_path, get_random_non_normalized_path());
    let object_path_2 = format!("{}/{}", folder_path, get_random_non_normalized_path());
    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;

    let object_address_1 = storage_service.s3_url_builder().path(&object_path_1).to_string();
    let object_address_2 = storage_service.s3_url_builder().path(&object_path_2).to_string();
    fileobject_client
        .write_random_object(&object_address_1, 32)
        .await
        .expect("object write failed");
    fileobject_client
        .write_random_object(&object_address_2, 32)
        .await
        .expect("object write failed");

    // Enumerate using the non-normalized address (parent directory)
    let mut stream = fileobject_client.enumerate(EnumerateRequest {
        resource_address: folder_address,
    });

    // Verify that we can read from the stream
    let expected_object_addresses: std::collections::HashSet<String> = [
        normalize_url(&object_address_1).unwrap(),
        normalize_url(&object_address_2).unwrap(),
    ]
    .into_iter()
    .collect();
    let mut found_items = false;
    while let Some(response) = stream.try_next().await.expect("enumerate request failed") {
        if !response.items.is_empty() {
            found_items = true;
            for item in response.items {
                assert!(expected_object_addresses.contains(&item.resource_address));
            }
        }
    }

    assert!(found_items, "Expected to find at least one item during enumeration");
}

#[rstest]
#[test_log::test(tokio::test)]
async fn enumerate_with_invalid_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::fileobject::types::v1beta::EnumerateRequest;

    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;
    let mut stream = fileobject_client.enumerate(EnumerateRequest {
        resource_address: "invalid-url.com".to_string(),
    });
    let status = stream
        .try_next()
        .await
        .expect_err("enumerate with invalid url is expected to fail");
    assert_eq!(status.code(), Code::InvalidArgument);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn list_with_normalized_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::filefolder::types::v1beta::{FolderAddress, ListRequest};

    let folder_path = get_random_non_normalized_path();
    let object_path = format!("{}////.//{}", folder_path, get_random_path_item());
    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;
    let filefolder_client = storage_service.filefolder_v1beta_client(interface).await;

    // Create an object in a folder with a non-normalized path
    let object_address = storage_service.s3_url_builder().path(&object_path).to_string();
    fileobject_client
        .write_random_object(&object_address, 32)
        .await
        .expect("object write failed");

    // List the folder using a non-normalized folder address
    let folder_address = storage_service.s3_url_builder().path(&folder_path).to_string();
    let mut responses = filefolder_client.list(ListRequest {
        folder: Some(FolderAddress {
            uri: folder_address.clone(),
        }),
    });

    // Verify that the list operation succeeds and returns items
    let mut found_items = false;
    while let Some(response) = responses.try_next().await.expect("list request failed") {
        if !response.sub_resource_addresses.is_empty() {
            found_items = true;
            // Verify that returned addresses are normalized
            for address in response.sub_resource_addresses {
                assert!(address.starts_with(&normalize_url(&folder_address).unwrap()));
            }
        }
    }

    assert!(found_items, "Expected to find at least one item in the folder");
}

#[rstest]
#[test_log::test(tokio::test)]
async fn list_with_invalid_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::filefolder::types::v1beta::{FolderAddress, ListRequest};

    let filefolder_client = storage_service.filefolder_v1beta_client(interface).await;
    let mut stream = filefolder_client.list(ListRequest {
        folder: Some(FolderAddress {
            uri: "invalid-url.com".to_string(),
        }),
    });
    let status = stream.try_next().await.expect_err("list is expected to fail");
    assert_eq!(status.code(), Code::InvalidArgument);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn list_stat_with_normalized_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::filefolder::types::v1beta::{FolderAddress, ListStatRequest};

    let folder_path = get_random_non_normalized_path();
    let object_path = format!("{}//./{}", folder_path, get_random_path_item());
    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;
    let filefolder_client = storage_service.filefolder_v1beta_client(interface).await;

    // Create an object in a folder with a non-normalized path
    let object_address = storage_service.s3_url_builder().path(&object_path).to_string();
    let (info, _) = fileobject_client
        .write_random_object(&object_address, 32)
        .await
        .expect("object write failed");
    let written_identity = info.unwrap_identity();
    assert!(written_identity.starts_with(&normalize_url(&object_address).unwrap()));

    // List and stat the folder using a non-normalized folder address
    let folder_address = storage_service.s3_url_builder().path(&folder_path).to_string();
    let mut responses = filefolder_client.list_stat(ListStatRequest {
        folder: Some(FolderAddress {
            uri: folder_address.clone(),
        }),
    });

    // Verify that the list_stat operation succeeds and returns items with normalized data
    let mut found_items = false;
    while let Some(response) = responses.try_next().await.expect("list_stat request failed") {
        if !response.entries.is_empty() {
            found_items = true;
            // Verify that returned addresses and identities are normalized
            for entry in response.entries {
                assert!(entry
                    .resource_address
                    .starts_with(&normalize_url(&folder_address).unwrap()));
                if let Some(resource_info) = entry.resource_info {
                    let identity = resource_info.unwrap_identity();
                    assert!(identity.starts_with(&normalize_url(&object_address).unwrap()));
                }
            }
        }
    }

    assert!(found_items, "Expected to find at least one item in the folder");
}

#[rstest]
#[test_log::test(tokio::test)]
async fn list_stat_with_invalid_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::filefolder::types::v1beta::{FolderAddress, ListStatRequest};

    let filefolder_client = storage_service.filefolder_v1beta_client(interface).await;
    let mut stream = filefolder_client.list_stat(ListStatRequest {
        folder: Some(FolderAddress {
            uri: "invalid-url.com".to_string(),
        }),
    });
    let status = stream.try_next().await.expect_err("list is expected to fail");
    assert_eq!(status.code(), Code::InvalidArgument);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn enumerate_versions_with_normalized_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::versioning::types::v1beta::EnumerateVersionsRequest;

    let path = get_random_non_normalized_path();
    let fileobject_client = storage_service.fileobject_v1beta_client(interface).await;
    let versioning_client = storage_service.versioning_v1beta_client(interface).await;

    // Create multiple versions of an object with a non-normalized path
    let address = storage_service.s3_url_builder().path(&path).to_string();

    // Write first version
    let (info1, _) = fileobject_client
        .write_random_object(&address, 32)
        .await
        .expect("first object write failed");
    let identity1 = info1.unwrap_identity();
    assert!(identity1.starts_with(&normalize_url(&address).unwrap()));

    // Write second version (overwrite)
    let (info2, _) = fileobject_client
        .write_random_object(&address, 32)
        .await
        .expect("second object write failed");
    let identity2 = info2.unwrap_identity();
    assert!(identity2.starts_with(&normalize_url(&address).unwrap()));

    // Enumerate versions using the non-normalized address
    let mut responses = versioning_client.enumerate_versions(EnumerateVersionsRequest {
        resource_address: address.clone(),
    });

    // Verify that we can enumerate versions
    let mut version_count = 0;
    let mut found_identities = vec![];

    while let Some(response) = responses.try_next().await.expect("enumerate versions failed") {
        for item in response.items {
            version_count += 1;
            if let Some(resource_info) = item.resource_info {
                let identity = resource_info.unwrap_identity();
                // Verify that returned identities are normalized
                assert!(identity.starts_with(&normalize_url(&address).unwrap()));
                found_identities.push(identity);
            }
        }
    }

    // Verify that we found at least the versions we created
    assert!(
        version_count >= 2,
        "Expected at least 2 versions, found {}",
        version_count
    );

    // Verify that the identities we wrote are in the results
    assert!(
        found_identities.contains(&identity1) || found_identities.contains(&identity2),
        "Expected to find at least one of the written identities in enumerate results"
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn enumerate_versions_with_invalid_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::versioning::types::v1beta::EnumerateVersionsRequest;

    let versioning_client = storage_service.versioning_v1beta_client(interface).await;
    let mut stream = versioning_client.enumerate_versions(EnumerateVersionsRequest {
        resource_address: "invalid-url.com".to_string(),
    });
    let status = stream
        .try_next()
        .await
        .expect_err("enumerate versions is expected to fail");
    assert_eq!(status.code(), Code::InvalidArgument);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_update_metadata_batch_all_invalid_urls_are_rejected(storage_service: &StorageService) {
    use common::clients::metadata::types::v1beta::{
        batch_error_entry::Code as BatchCode, update_metadata_batch_response_entry::Outcome, BatchErrorEntry,
        ResourceLocation, ResourceLocationExt, UpdateMetadataBatchRequest, UpdateMetadataBatchRequestEntry,
        UpdateMetadataBatchResponse, UpdateMetadataBatchResponseEntry,
    };
    use storage_service_protos::google::protobuf::{value::Kind, Value};

    let client = storage_service.metadata_v1beta_client(Interface::Grpc).await;
    let locations = [
        ResourceLocation::resource_address("text"),
        ResourceLocation::folder_address("s3://host/folder%0G"),
        ResourceLocation::resource_address("s3://host/file%"),
        ResourceLocation::folder_address("s3://host:8a/folder"),
    ];
    let requests = locations
        .into_iter()
        .map(|location| UpdateMetadataBatchRequestEntry {
            resource_location: Some(location),
            user_metadata_key: "key".into(),
            user_metadata: Some(Value {
                kind: Some(Kind::StringValue("value".into())),
            }),
            ..Default::default()
        })
        .collect();

    let response = client
        .update_metadata_batch_sorted(UpdateMetadataBatchRequest { requests })
        .await
        .expect("metadata update failed");

    assert_eq!(
        response,
        UpdateMetadataBatchResponse {
            results: [
                (0, "resource address is invalid"),
                (1, "folder address is invalid"),
                (2, "resource address is invalid"),
                (3, "folder address is invalid"),
            ]
            .into_iter()
            .map(|(request_index, message)| UpdateMetadataBatchResponseEntry {
                request_index,
                outcome: Some(Outcome::Error(BatchErrorEntry {
                    code: BatchCode::InvalidArgument as i32,
                    message: message.into(),
                })),
            })
            .collect(),
        }
    );
}
