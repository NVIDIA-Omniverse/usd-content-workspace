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

use crate::common::{
    random::{get_random_alphanumeric_string, get_random_path, get_random_path_item},
    service::Service as StorageService,
};
use assert_matches::assert_matches;
use common::clients::{
    fileobject::types::v1alpha::{StatRequest, UnwrapIdentityExt},
    metadata::types::v1alpha::{
        BatchErrorEntry, DeleteMetadataBatchResponseEntry, GetMetadataBatchRequestEntry, GetMetadataBatchResponseEntry,
        QueryMetadataResponseEntry, UpdateMetadataBatchResponseEntry,
    },
};
use common::types::Interface;
use futures::TryStreamExt;
use rstest::{fixture, rstest};
use storage_service_protos::google::protobuf::Value;
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
    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;
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
    use common::clients::fileobject::types::v1alpha::ReadFromAddressRequest;

    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;
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
async fn copy_object_with_normalized_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::fileobject::types::v1alpha::{CopyRequest, ResourceIdentity};

    let path = get_random_non_normalized_path();
    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;
    let address = storage_service.s3_url_builder().path(&path).to_string();
    let (original_info, written_content) = fileobject_client
        .write_random_object(&address, 32)
        .await
        .expect("object write failed");
    let written_identity = original_info.unwrap_identity();
    assert!(written_identity.starts_with(&normalize_url(&address).unwrap()));

    let destination_path = get_random_non_normalized_path();
    let destination_address = storage_service.s3_url_builder().path(&destination_path).to_string();
    fileobject_client
        .copy(CopyRequest {
            destination_resource_address: destination_address.clone(),
            source_resource_identity: Some(ResourceIdentity {
                encoded_identity: written_identity,
            }),
            previous_version: None,
            fail_if_exists: None,
        })
        .await
        .expect("object copy failed");

    let (_, read_content) = fileobject_client
        .read_object_from_address(&destination_address)
        .await
        .expect("object read failed");
    assert_eq!(read_content, written_content);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn move_object_with_normalized_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::fileobject::types::v1alpha::MoveRequest;

    let path = get_random_non_normalized_path();
    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;
    let address = storage_service.s3_url_builder().path(&path).to_string();
    let (original_info, written_content) = fileobject_client
        .write_random_object(&address, 32)
        .await
        .expect("object write failed");
    let written_identity = original_info.unwrap_identity();
    assert!(written_identity.starts_with(&normalize_url(&address).unwrap()));

    let destination_path = get_random_non_normalized_path();
    let destination_address = storage_service.s3_url_builder().path(&destination_path).to_string();
    fileobject_client
        .move_object(MoveRequest {
            destination_resource_address: destination_address.clone(),
            source_resource_address: address,
            ..MoveRequest::default()
        })
        .await
        .expect("object move failed");

    let (_, read_content) = fileobject_client
        .read_object_from_address(&destination_address)
        .await
        .expect("object read failed");
    assert_eq!(read_content, written_content);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn move_with_invalid_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::fileobject::types::v1alpha::MoveRequest;

    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;
    let address = "invalid-url.com".to_string();
    let status = fileobject_client
        .move_object(MoveRequest {
            source_resource_address: address.clone(),
            destination_resource_address: address.clone(),
            ..MoveRequest::default()
        })
        .await
        .expect_err("move with invalid url is expected to fail");
    assert_eq!(status.code(), Code::InvalidArgument);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn stat_with_normalized_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    let path = get_random_non_normalized_path();
    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;
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
    use common::clients::fileobject::types::v1alpha::StatRequest;

    let client = storage_service.fileobject_v1alpha_client(Interface::Grpc).await;
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
    use common::clients::fileobject::types::v1alpha::StatRequest;

    let service = StorageService::builder()
        .enable_s3(true)
        .enable_postgres_metadata()
        .enable_url_normalization()
        .enable_s3_url_remapper("mapped/prefix/", "original/")
        .start();

    let client = service.fileobject_v1alpha_client(Interface::Grpc).await;
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
    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;
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
    use common::clients::fileobject::types::v1alpha::DeleteRequest;

    let path = get_random_non_normalized_path();
    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;
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
    use common::clients::fileobject::types::v1alpha::DeleteRequest;

    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;
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
    use common::clients::fileobject::types::v1alpha::{AbortMultipartUploadRequest, UploadPreference, WriteParameters};

    let path = get_random_non_normalized_path();
    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;
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
        common::clients::fileobject::types::v1alpha::WriteResponse::Multipart(response) => response.upload_id,
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
    use common::clients::fileobject::types::v1alpha::AbortMultipartUploadRequest;

    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;
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
    use common::clients::fileobject::types::v1alpha::{
        CompleteMultipartUploadRequest, UploadPreference, WriteParameters,
    };

    let path = get_random_non_normalized_path();
    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;
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
        common::clients::fileobject::types::v1alpha::WriteResponse::Multipart(response) => response.upload_id,
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
    use common::clients::fileobject::types::v1alpha::CompleteMultipartUploadRequest;

    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;
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
    use common::clients::fileobject::types::v1alpha::{UploadPartRequest, UploadPreference, WriteParameters};

    let path = get_random_non_normalized_path();
    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;
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
        common::clients::fileobject::types::v1alpha::WriteResponse::Multipart(response) => response.upload_id,
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
    use common::clients::fileobject::types::v1alpha::UploadPartRequest;

    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;
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
    use common::clients::fileobject::types::v1alpha::{
        CompleteRedirectUploadRequest, UploadPreference, WriteParameters,
    };
    use common::clients::fileobject::v1alpha::follow_write_redirect;

    let path = get_random_non_normalized_path();
    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;
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
        common::clients::fileobject::types::v1alpha::WriteResponse::Redirect(props) => props,
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
    use common::clients::fileobject::types::v1alpha::CompleteRedirectUploadRequest;

    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;
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
    use common::clients::fileobject::types::v1alpha::FetchWriteTypeInfoRequest;

    let path = get_random_non_normalized_path();
    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;
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
    use common::clients::fileobject::types::v1alpha::FetchWriteTypeInfoRequest;

    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;
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
    use common::clients::fileobject::types::v1alpha::EnumerateRequest;

    let folder_path = get_random_non_normalized_path();
    let folder_address = storage_service.s3_url_builder().path(&folder_path).to_string();
    let object_path_1 = format!("{}/{}", folder_path, get_random_non_normalized_path());
    let object_path_2 = format!("{}/{}", folder_path, get_random_non_normalized_path());
    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;

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
    use common::clients::fileobject::types::v1alpha::EnumerateRequest;

    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;
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
async fn copy_with_normalized_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::fileobject::types::v1alpha::{CopyRequest, ResourceIdentity};

    let source_path = get_random_non_normalized_path();
    let target_path = get_random_non_normalized_path();
    let client = storage_service.fileobject_v1alpha_client(interface).await;

    // Create a source object with a non-normalized address
    let source_address = storage_service.s3_url_builder().path(&source_path).to_string();
    let (source_info, _content) = client
        .write_random_object(&source_address, 32)
        .await
        .expect("source object write failed");
    let source_identity = source_info.unwrap_identity();
    assert!(source_identity.starts_with(&normalize_url(&source_address).unwrap()));

    // Copy to a destination with a non-normalized address
    let target_address = storage_service.s3_url_builder().path(&target_path).to_string();
    let copy_response = client
        .copy(CopyRequest {
            destination_resource_address: target_address.clone(),
            source_resource_identity: Some(ResourceIdentity {
                encoded_identity: source_identity.clone(),
            }),
            previous_version: None,
            fail_if_exists: None,
        })
        .await
        .expect("object copy failed");

    // Verify the returned identity is normalized
    let copy_identity = copy_response
        .resource_identity
        .expect("resource identity is missing")
        .encoded_identity;
    assert!(copy_identity.starts_with(&normalize_url(&target_address).unwrap()));

    // Verify the copied object can be read
    let (read_info, _) = client
        .read_object_from_address(&target_address)
        .await
        .expect("object read failed");
    let read_identity = read_info.unwrap_identity();
    assert!(read_identity.starts_with(&normalize_url(&target_address).unwrap()));
}

#[rstest]
#[test_log::test(tokio::test)]
async fn copy_object_with_invalid_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::fileobject::types::{v1alpha::CopyRequest, v1alpha::ResourceIdentity};

    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;
    let source_path = get_random_non_normalized_path();
    let source_address = storage_service.s3_url_builder().path(&source_path).to_string();
    let (source_info, _) = fileobject_client
        .write_random_object(&source_address, 32)
        .await
        .expect("source object write failed");
    let source_identity = source_info.unwrap_identity();
    assert!(source_identity.starts_with(&normalize_url(&source_address).unwrap()));
    let status = fileobject_client
        .copy(CopyRequest {
            destination_resource_address: "invalid-url.com".to_string(),
            source_resource_identity: Some(ResourceIdentity {
                encoded_identity: source_identity,
            }),
            ..CopyRequest::default()
        })
        .await
        .expect_err("copy with invalid url is expected to fail");
    assert_eq!(status.code(), Code::InvalidArgument);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn create_folder_with_normalized_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::filefolder::types::v1alpha::{
        CreateFolderRequest, DeleteFolderRequest, FolderAddress, GetFolderModeRequest,
    };

    let folder_path = get_random_non_normalized_path();
    let filefolder_client = storage_service.filefolder_v1alpha_client(interface).await;

    let folder_address = storage_service.s3_url_builder().path(&folder_path).to_string();

    let create_result = filefolder_client
        .create_folder(CreateFolderRequest {
            folder: Some(FolderAddress {
                uri: folder_address.clone(),
            }),
        })
        .await;
    assert!(create_result.is_ok(), "create folder failed: {:?}", create_result);
    let normalized_folder_address = normalize_url(&folder_address).unwrap();
    let get_mode_result = filefolder_client
        .get_folder_mode(GetFolderModeRequest {
            folder: Some(FolderAddress {
                uri: normalized_folder_address.clone(),
            }),
        })
        .await;
    assert!(get_mode_result.is_ok(), "get folder mode failed: {:?}", get_mode_result);
    // Delete the folder using the normalized folder address
    let delete_result = filefolder_client
        .delete_folder(DeleteFolderRequest {
            folder: Some(FolderAddress {
                uri: normalized_folder_address.clone(),
            }),
        })
        .await;
    assert!(delete_result.is_ok(), "delete folder failed: {:?}", delete_result);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn create_folder_with_invalid_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::filefolder::types::v1alpha::{CreateFolderRequest, FolderAddress};

    let filefolder_client = storage_service.filefolder_v1alpha_client(interface).await;
    let status = filefolder_client
        .create_folder(CreateFolderRequest {
            folder: Some(FolderAddress {
                uri: "invalid-url.com".to_string(),
            }),
        })
        .await
        .expect_err("create folder with invalid url is expected to fail");
    assert_eq!(status.code(), Code::InvalidArgument);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn get_folder_mode_with_normalized_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::filefolder::types::v1alpha::{
        CreateFolderRequest, DeleteFolderRequest, FolderAddress, GetFolderModeRequest,
    };

    let folder_path = get_random_non_normalized_path();
    let filefolder_client = storage_service.filefolder_v1alpha_client(interface).await;

    let folder_address = storage_service.s3_url_builder().path(&folder_path).to_string();

    let create_result = filefolder_client
        .create_folder(CreateFolderRequest {
            folder: Some(FolderAddress {
                uri: folder_address.clone(),
            }),
        })
        .await;
    assert!(create_result.is_ok(), "create folder failed: {:?}", create_result);
    let normalized_folder_address = normalize_url(&folder_address).unwrap();
    let get_mode_result = filefolder_client
        .get_folder_mode(GetFolderModeRequest {
            folder: Some(FolderAddress {
                uri: normalized_folder_address.clone(),
            }),
        })
        .await;
    assert!(get_mode_result.is_ok(), "get folder mode failed: {:?}", get_mode_result);
    // Delete the folder using the normalized folder address
    let delete_result = filefolder_client
        .delete_folder(DeleteFolderRequest {
            folder: Some(FolderAddress {
                uri: normalized_folder_address.clone(),
            }),
        })
        .await;
    assert!(delete_result.is_ok(), "delete folder failed: {:?}", delete_result);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn get_folder_mode_with_invalid_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::filefolder::types::v1alpha::{FolderAddress, GetFolderModeRequest};

    let filefolder_client = storage_service.filefolder_v1alpha_client(interface).await;
    let status = filefolder_client
        .get_folder_mode(GetFolderModeRequest {
            folder: Some(FolderAddress {
                uri: "invalid-url.com".to_string(),
            }),
        })
        .await
        .expect_err("get folder mode with invalid url is expected to fail");
    assert_eq!(status.code(), Code::InvalidArgument);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn delete_folder_with_normalized_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::filefolder::types::v1alpha::{
        CreateFolderRequest, DeleteFolderRequest, FolderAddress, GetFolderModeRequest, ListRequest,
    };

    let folder_path = get_random_non_normalized_path();
    let filefolder_client = storage_service.filefolder_v1alpha_client(interface).await;

    let folder_address = storage_service.s3_url_builder().path(&folder_path).to_string();

    let create_result = filefolder_client
        .create_folder(CreateFolderRequest {
            folder: Some(FolderAddress {
                uri: normalize_url(&folder_address).unwrap(),
            }),
        })
        .await;
    assert!(create_result.is_ok(), "create folder failed: {:?}", create_result);
    let get_mode_result = filefolder_client
        .get_folder_mode(GetFolderModeRequest {
            folder: Some(FolderAddress {
                uri: folder_address.clone(),
            }),
        })
        .await;
    assert!(get_mode_result.is_ok(), "get folder mode failed: {:?}", get_mode_result);
    filefolder_client
        .delete_folder(DeleteFolderRequest {
            folder: Some(FolderAddress {
                uri: folder_address.clone(),
            }),
        })
        .await
        .expect("delete folder failed");
    filefolder_client
        .list(ListRequest {
            folder: Some(FolderAddress {
                uri: folder_address.clone(),
            }),
        })
        .try_next()
        .await
        .expect_err("list request failed");
}

#[rstest]
#[test_log::test(tokio::test)]
async fn delete_folder_with_invalid_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::filefolder::types::v1alpha::{DeleteFolderRequest, FolderAddress};

    let filefolder_client = storage_service.filefolder_v1alpha_client(interface).await;
    let status = filefolder_client
        .delete_folder(DeleteFolderRequest {
            folder: Some(FolderAddress {
                uri: "invalid-url.com".to_string(),
            }),
        })
        .await
        .expect_err("delete folder with invalid url is expected to fail");
    assert_eq!(status.code(), Code::InvalidArgument);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn list_with_normalized_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::filefolder::types::v1alpha::{FolderAddress, ListRequest};

    let folder_path = get_random_non_normalized_path();
    let object_path = format!("{}////.//{}", folder_path, get_random_path_item());
    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;
    let filefolder_client = storage_service.filefolder_v1alpha_client(interface).await;

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
    use common::clients::filefolder::types::v1alpha::{FolderAddress, ListRequest};

    let filefolder_client = storage_service.filefolder_v1alpha_client(interface).await;
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
    use common::clients::filefolder::types::v1alpha::{FolderAddress, ListStatRequest};

    let folder_path = get_random_non_normalized_path();
    let object_path = format!("{}//./{}", folder_path, get_random_path_item());
    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;
    let filefolder_client = storage_service.filefolder_v1alpha_client(interface).await;

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
        include_native_metadata: false,
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
    use common::clients::filefolder::types::v1alpha::{FolderAddress, ListStatRequest};

    let filefolder_client = storage_service.filefolder_v1alpha_client(interface).await;
    let mut stream = filefolder_client.list_stat(ListStatRequest {
        folder: Some(FolderAddress {
            uri: "invalid-url.com".to_string(),
        }),
        include_native_metadata: false,
    });
    let status = stream.try_next().await.expect_err("list is expected to fail");
    assert_eq!(status.code(), Code::InvalidArgument);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn update_metadata_with_normalized_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::metadata::types::v1alpha::UpdateMetadataRequest;
    use storage_service_protos::google::protobuf::{value::Kind, Value};

    let path = get_random_non_normalized_path();
    let metadata_client = storage_service.metadata_v1alpha_client(interface).await;

    // Update metadata using a non-normalized URL
    let uri = storage_service.s3_url_builder().path(&path).to_string();
    let metadata = Value {
        kind: Some(Kind::StringValue("test_value".into())),
    };

    let response = metadata_client
        .update_metadata(UpdateMetadataRequest {
            uri: uri.clone(),
            user_metadata_key: "test_key".into(),
            user_metadata: Some(metadata.clone()),
            ..Default::default()
        })
        .await
        .expect("metadata update failed");

    // Verify that an etag was returned (indicating success)
    assert!(!response.etag.is_empty(), "Expected non-empty etag from update");
}

#[rstest]
#[test_log::test(tokio::test)]
async fn update_metadata_with_invalid_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::metadata::types::v1alpha::UpdateMetadataRequest;
    use storage_service_protos::google::protobuf::{value::Kind, Value};

    let metadata_client = storage_service.metadata_v1alpha_client(interface).await;
    let status = metadata_client
        .update_metadata(UpdateMetadataRequest {
            uri: "invalid-url.com".to_string(),
            user_metadata_key: "test_key".into(),
            user_metadata: Some(Value {
                kind: Some(Kind::StringValue("test_value".into())),
            }),
            ..Default::default()
        })
        .await
        .expect_err("metadata update with invalid url is expected to fail");
    assert_eq!(status.code(), Code::InvalidArgument);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_get_metadata_batch(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::metadata::types::v1alpha::*;

    let client = storage_service.metadata_v1alpha_client(interface).await;

    let (long_path_1, norm_path_1) = create_path_pair();
    let (long_path_2, norm_path_2) = create_path_pair();
    let (long_path_3, norm_path_3) = create_path_pair();
    let long_uri_1 = storage_service.s3_url_builder().path(&long_path_1).to_string();
    let norm_uri_1 = storage_service.s3_url_builder().path(&norm_path_1).to_string();
    let long_uri_2 = storage_service
        .s3_url_builder()
        .path(&long_path_2)
        .query("etag=1")
        .to_string();
    let norm_uri_2 = storage_service
        .s3_url_builder()
        .path(&norm_path_2)
        .query("etag=1")
        .to_string();
    let long_uri_3 = storage_service.s3_url_builder().path(&long_path_3).to_string();
    let norm_uri_3 = storage_service.s3_url_builder().path(&norm_path_3).to_string();

    let (value_1, etag_1) = client
        .set_random_metadata_value(ResourceLocation::resource_address(norm_uri_1), "a")
        .await
        .expect("metadata set failed");
    client
        .set_random_metadata_value(ResourceLocation::resource_identity(norm_uri_2), "b")
        .await
        .expect("metadata set failed");
    let (value_3, etag_3) = client
        .set_random_metadata_value(ResourceLocation::folder_address(norm_uri_3), "c")
        .await
        .expect("metadata set failed");

    // Make a request. Metadata for the resource address and the folder address must be
    // successfully resolved. The resource identity query must return a not found error
    // because resource identities remain unchanged.
    let response = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                create_get_metadata_batch_resource_address_request_entry(&long_uri_1, "a"),
                create_get_metadata_batch_resource_identity_request_entry(&long_uri_2, "b"),
                create_get_metadata_batch_folder_address_request_entry(&long_uri_3, "c"),
            ],
        })
        .await
        .expect("metadata get failed");

    assert_eq!(
        response,
        GetMetadataBatchResponse {
            results: vec![
                create_get_metadata_batch_success_response_entry(0, value_1, etag_1),
                create_get_metadata_batch_not_found_response_entry(1),
                create_get_metadata_batch_success_response_entry(2, value_3, etag_3),
            ],
        },
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_get_metadata_batch_rejects_invalid_request_uris(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::metadata::types::v1alpha::{GetMetadataBatchRequest, GetMetadataBatchResponse};

    let client = storage_service.metadata_v1alpha_client(interface).await;
    let resource_uri = storage_service
        .s3_url_builder()
        .path(get_random_non_normalized_path())
        .to_string();

    let response = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                create_get_metadata_batch_resource_address_request_entry("text", "a"),
                create_get_metadata_batch_resource_address_request_entry(resource_uri, "b"),
                create_get_metadata_batch_resource_address_request_entry("text", "c"),
            ],
        })
        .await
        .expect("metadata fetch failed");

    assert_eq!(
        response,
        GetMetadataBatchResponse {
            results: vec![
                create_get_metadata_batch_invalid_resource_uri_entry(0),
                create_get_metadata_batch_not_found_response_entry(1),
                create_get_metadata_batch_invalid_resource_uri_entry(2),
            ],
        },
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_query_metadata_normalizes_folder_addresses(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::metadata::types::v1alpha::*;

    let client = storage_service.metadata_v1alpha_client(interface).await;

    let (long_path, short_path) = create_path_pair();
    let long_uri = storage_service
        .s3_url_builder()
        .path(long_path)
        .query("query")
        .to_string();
    let short_uri = storage_service.s3_url_builder().path(short_path).to_string();

    let (value_1, etag_1) = client
        .set_random_metadata_value(ResourceLocation::resource_address(&short_uri), "a")
        .await
        .unwrap();
    let (value_2, etag_2) = client
        .set_random_metadata_value(ResourceLocation::resource_address(&short_uri), "b")
        .await
        .unwrap();

    let query_response = client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::resource_address(long_uri)),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(
        query_response,
        QueryMetadataResponse {
            results: vec![
                create_query_metadata_response_entry("a", value_1, etag_1),
                create_query_metadata_response_entry("b", value_2, etag_2),
            ]
        }
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_query_metadata_normalizes_resource_addresses(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::metadata::types::v1alpha::*;

    let client = storage_service.metadata_v1alpha_client(interface).await;

    let (long_path, short_path) = create_path_pair();
    let long_uri = storage_service
        .s3_url_builder()
        .path(long_path)
        .query("query")
        .to_string();
    let short_uri = storage_service
        .s3_url_builder()
        .path(format!("{short_path}/"))
        .to_string();

    let (value_1, etag_1) = client
        .set_random_metadata_value(ResourceLocation::folder_address(&short_uri), "a")
        .await
        .unwrap();
    let (value_2, etag_2) = client
        .set_random_metadata_value(ResourceLocation::folder_address(&short_uri), "b")
        .await
        .unwrap();

    let query_response = client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::folder_address(long_uri)),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(
        query_response,
        QueryMetadataResponse {
            results: vec![
                create_query_metadata_response_entry("a", value_1, etag_1),
                create_query_metadata_response_entry("b", value_2, etag_2),
            ]
        }
    );
}

#[ignore = "this test scenario requires disabling the uri canonicalization layer, which is currently not possible"]
#[test_log::test(tokio::test)]
async fn test_query_metadata_ignores_resource_identities() {
    todo!();
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_query_metadata_rejects_invalid_uris(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::metadata::types::v1alpha::*;

    let client = storage_service.metadata_v1alpha_client(interface).await;

    let query_error = client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::resource_address("aboba")),
            ..Default::default()
        })
        .await
        .unwrap_err();

    assert_eq!(query_error.code(), Code::InvalidArgument);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_update_metadata_batch(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::metadata::types::v1alpha::{
        batch_error_entry::Code, update_metadata_batch_response_entry::Outcome, *,
    };

    let client = storage_service.metadata_v1alpha_client(interface).await;

    let (long_path_1, norm_path_1) = create_path_pair();
    let (long_path_2, norm_path_2) = create_path_pair();
    let (long_path_3, norm_path_3) = create_path_pair();
    let long_uri_1 = storage_service.s3_url_builder().path(&long_path_1).to_string();
    let norm_uri_1 = storage_service.s3_url_builder().path(&norm_path_1).to_string();
    let long_uri_2 = storage_service
        .s3_url_builder()
        .path(&long_path_2)
        .query("etag=1")
        .to_string();
    let norm_uri_2 = storage_service
        .s3_url_builder()
        .path(&norm_path_2)
        .query("etag=1")
        .to_string();
    let long_uri_3 = storage_service.s3_url_builder().path(&long_path_3).to_string();
    let norm_uri_3 = storage_service.s3_url_builder().path(&norm_path_3).to_string();
    let value_1 = create_random_value();
    let value_2 = create_random_value();
    let value_3 = create_random_value();

    let update_response = client
        .update_metadata_batch_sorted(UpdateMetadataBatchRequest {
            requests: vec![
                UpdateMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&long_uri_1)),
                    user_metadata_key: "a".into(),
                    user_metadata: Some(value_1.clone()),
                    ..Default::default()
                },
                UpdateMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_identity(&long_uri_2)),
                    user_metadata_key: "b".into(),
                    user_metadata: Some(value_2.clone()),
                    ..Default::default()
                },
                UpdateMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::folder_address(&long_uri_3)),
                    user_metadata_key: "c".into(),
                    user_metadata: Some(value_3.clone()),
                    ..Default::default()
                },
                UpdateMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&long_uri_1)),
                    user_metadata_key: "d".into(),
                    user_metadata: Some(create_random_value()),
                    expected_etag: Some("some-etag".into()),
                    ..Default::default()
                },
                UpdateMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_identity(&long_uri_2)),
                    user_metadata_key: "e".into(),
                    user_metadata: Some(create_random_value()),
                    expected_etag: Some("some-etag".into()),
                    ..Default::default()
                },
                UpdateMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::folder_address(&long_uri_3)),
                    user_metadata_key: "f".into(),
                    user_metadata: Some(create_random_value()),
                    expected_etag: Some("some-etag".into()),
                    ..Default::default()
                },
            ],
        })
        .await
        .expect("metadata update failed");

    assert_eq!(update_response.results.len(), 6);
    assert_eq!(update_response.results[0].request_index, 0);
    assert_eq!(update_response.results[1].request_index, 1);
    assert_eq!(update_response.results[2].request_index, 2);
    assert_eq!(update_response.results[3].request_index, 3);
    assert_eq!(update_response.results[4].request_index, 4);
    assert_eq!(update_response.results[5].request_index, 5);
    assert_matches!(&update_response.results[0].outcome, Some(Outcome::Success(e)) if !e.etag.is_empty());
    assert_matches!(&update_response.results[1].outcome, Some(Outcome::Success(e)) if !e.etag.is_empty());
    assert_matches!(&update_response.results[2].outcome, Some(Outcome::Success(e)) if !e.etag.is_empty());
    assert_matches!(&update_response.results[3].outcome, Some(Outcome::Error(e)) if e.code == Code::FailedPrecondition as i32);
    assert_matches!(&update_response.results[4].outcome, Some(Outcome::Error(e)) if e.code == Code::FailedPrecondition as i32);
    assert_matches!(&update_response.results[5].outcome, Some(Outcome::Error(e)) if e.code == Code::FailedPrecondition as i32);

    // Read the metadata back.
    let get_response = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                create_get_metadata_batch_resource_address_request_entry(&norm_uri_1, "a"),
                create_get_metadata_batch_resource_identity_request_entry(&norm_uri_2, "b"),
                create_get_metadata_batch_resource_identity_request_entry(&long_uri_2, "b"),
                create_get_metadata_batch_folder_address_request_entry(&norm_uri_3, "c"),
            ],
        })
        .await
        .expect("metadata get failed");

    assert_eq!(
        get_response,
        GetMetadataBatchResponse {
            results: vec![
                create_get_metadata_batch_success_response_entry(
                    0,
                    value_1,
                    unwrap_update_metadata_batch_response_entry_etag(&update_response.results[0])
                ),
                create_get_metadata_batch_not_found_response_entry(1),
                create_get_metadata_batch_success_response_entry(
                    2,
                    value_2,
                    unwrap_update_metadata_batch_response_entry_etag(&update_response.results[1])
                ),
                create_get_metadata_batch_success_response_entry(
                    3,
                    value_3,
                    unwrap_update_metadata_batch_response_entry_etag(&update_response.results[2])
                ),
            ],
        },
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_update_metadata_batch_rejects_invalid_request_uris(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::metadata::types::v1alpha::{
        batch_error_entry::Code, update_metadata_batch_response_entry::Outcome, *,
    };

    let client = storage_service.metadata_v1alpha_client(interface).await;
    let resource_uri = storage_service.s3_url_builder().path(get_random_path()).to_string();

    let response = client
        .update_metadata_batch_sorted(UpdateMetadataBatchRequest {
            requests: vec![
                UpdateMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address("text")),
                    user_metadata_key: "key".into(),
                    user_metadata: Some(create_random_value()),
                    ..Default::default()
                },
                UpdateMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::folder_address("text")),
                    user_metadata_key: "key".into(),
                    user_metadata: Some(create_random_value()),
                    ..Default::default()
                },
                UpdateMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(resource_uri)),
                    user_metadata_key: "key".into(),
                    user_metadata: Some(create_random_value()),
                    ..Default::default()
                },
            ],
        })
        .await
        .expect("metadata update failed");

    assert_eq!(response.results.len(), 3);
    assert_matches!(&response.results[0].outcome, Some(Outcome::Error(e)) if e.code == Code::InvalidArgument as i32);
    assert_matches!(&response.results[1].outcome, Some(Outcome::Error(e)) if e.code == Code::InvalidArgument as i32);
    assert_matches!(&response.results[2].outcome, Some(Outcome::Success(e)) if !e.etag.is_empty());
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_update_metadata_batch_all_invalid_urls_are_rejected(storage_service: &StorageService) {
    use common::clients::metadata::types::v1alpha::{
        batch_error_entry::Code as BatchCode, update_metadata_batch_response_entry::Outcome, BatchErrorEntry,
        ResourceLocation, ResourceLocationExt, UpdateMetadataBatchRequest, UpdateMetadataBatchRequestEntry,
        UpdateMetadataBatchResponse, UpdateMetadataBatchResponseEntry,
    };
    use storage_service_protos::google::protobuf::{value::Kind, Value};

    let client = storage_service.metadata_v1alpha_client(Interface::Grpc).await;
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

#[rstest]
#[test_log::test(tokio::test)]
async fn test_delete_metadata_batch(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::metadata::types::v1alpha::{
        batch_error_entry::Code, delete_metadata_batch_response_entry::Outcome, *,
    };

    let client = storage_service.metadata_v1alpha_client(interface).await;

    let (long_path_1, short_path_1) = create_path_pair();
    let (long_path_2, short_path_2) = create_path_pair();
    let (long_path_3, short_path_3) = create_path_pair();
    let long_uri_1 = storage_service.s3_url_builder().path(long_path_1).to_string();
    let long_uri_2 = storage_service.s3_url_builder().path(long_path_2).to_string();
    let long_uri_3 = storage_service
        .s3_url_builder()
        .path(long_path_3)
        .query("etag=abcd")
        .to_string();
    let short_uri_1 = storage_service.s3_url_builder().path(short_path_1).to_string();
    let short_uri_2 = storage_service.s3_url_builder().path(short_path_2).to_string();
    let short_uri_3 = storage_service
        .s3_url_builder()
        .path(short_path_3)
        .query("etag=abcd")
        .to_string();

    let (_value_1, etag_1) = client
        .set_random_metadata_value(ResourceLocation::resource_address(short_uri_1), "a")
        .await
        .unwrap();
    let (_value_2, etag_2) = client
        .set_random_metadata_value(ResourceLocation::folder_address(short_uri_2), "b")
        .await
        .unwrap();
    let (_value_3, etag_3) = client
        .set_random_metadata_value(ResourceLocation::resource_identity(short_uri_3), "c")
        .await
        .unwrap();

    // Delete metadata conditionally. Only properly normalzied requests must succeed.
    let delete_response = client
        .delete_metadata_batch_sorted(DeleteMetadataBatchRequest {
            requests: vec![
                DeleteMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(long_uri_1)),
                    user_metadata_key: "a".into(),
                    expected_etag: Some(etag_1),
                    ..Default::default()
                },
                DeleteMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::folder_address(long_uri_2)),
                    user_metadata_key: "b".into(),
                    expected_etag: Some(etag_2),
                    ..Default::default()
                },
                DeleteMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_identity(long_uri_3)),
                    user_metadata_key: "c".into(),
                    expected_etag: Some(etag_3),
                    ..Default::default()
                },
            ],
        })
        .await
        .unwrap();

    assert_eq!(
        delete_response,
        DeleteMetadataBatchResponse {
            results: vec![
                create_delete_metadata_batch_success_entry(0),
                create_delete_metadata_batch_success_entry(1),
                DeleteMetadataBatchResponseEntry {
                    outcome: Some(Outcome::Error(BatchErrorEntry {
                        code: Code::FailedPrecondition as i32,
                        message: "metadata was modified".into()
                    })),
                    request_index: 2,
                },
            ],
        }
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_delete_metadata_batch_rejects_invalid_request_uris(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::metadata::types::v1alpha::*;

    let client = storage_service.metadata_v1alpha_client(interface).await;
    let resource_uri = storage_service.s3_url_builder().path(get_random_path()).to_string();

    // Pass an invalid resource address, resource identity, and a folder identity alongside a valid
    // resource address. The request against the resource uri must not undergo normalization and
    // forwarded to the underlying layer.
    let response = client
        .delete_metadata_batch_sorted(DeleteMetadataBatchRequest {
            requests: vec![
                DeleteMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address("text")),
                    user_metadata_key: "key".into(),
                    ..Default::default()
                },
                DeleteMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(resource_uri)),
                    user_metadata_key: "key".into(),
                    ..Default::default()
                },
                DeleteMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_identity("text")),
                    user_metadata_key: "key".into(),
                    ..Default::default()
                },
                DeleteMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::folder_address("text")),
                    user_metadata_key: "key".into(),
                    ..Default::default()
                },
            ],
        })
        .await
        .expect("metadata update failed");

    assert_eq!(
        response,
        DeleteMetadataBatchResponse {
            results: vec![
                create_delete_metadata_batch_invalid_resource_uri_entry(0, "resource address is invalid"),
                create_delete_metadata_batch_success_entry(1),
                // This item is rejected by the non-optional uri canonicalization layer.
                create_delete_metadata_batch_invalid_resource_uri_entry(2, "resource uri is invalid"),
                create_delete_metadata_batch_invalid_resource_uri_entry(3, "folder address is invalid"),
            ]
        },
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn get_metadata_with_normalized_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::metadata::types::v1alpha::{GetMetadataRequest, UpdateMetadataRequest};
    use storage_service_protos::google::protobuf::{value::Kind, Value};

    let path = get_random_non_normalized_path();
    let metadata_client = storage_service.metadata_v1alpha_client(interface).await;

    // First, update metadata using a non-normalized URL
    let uri = storage_service.s3_url_builder().path(&path).to_string();
    let metadata = Value {
        kind: Some(Kind::StringValue("test_value".into())),
    };

    let update_response = metadata_client
        .update_metadata(UpdateMetadataRequest {
            uri: uri.clone(),
            user_metadata_key: "test_key".into(),
            user_metadata: Some(metadata.clone()),
            ..Default::default()
        })
        .await
        .expect("metadata update failed");

    // Get metadata using a non-normalized URL
    let get_response = metadata_client
        .get_metadata(GetMetadataRequest {
            uri: uri.clone(),
            ..Default::default()
        })
        .await
        .expect("metadata get failed");

    // Verify the metadata was retrieved correctly
    assert!(
        get_response.user_metadata.contains_key("test_key"),
        "Expected to find test_key in metadata"
    );
    let retrieved_metadata = &get_response.user_metadata["test_key"];
    assert_eq!(retrieved_metadata.value, Some(metadata));
    assert_eq!(retrieved_metadata.etag, update_response.etag);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn get_metadata_with_invalid_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::metadata::types::v1alpha::GetMetadataRequest;

    let metadata_client = storage_service.metadata_v1alpha_client(interface).await;
    let status = metadata_client
        .get_metadata(GetMetadataRequest {
            uri: "invalid-url.com".to_string(),
            ..Default::default()
        })
        .await
        .expect_err("metadata get with invalid url is expected to fail");
    assert_eq!(status.code(), Code::InvalidArgument);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn delete_metadata_with_normalized_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::metadata::types::v1alpha::{DeleteMetadataRequest, GetMetadataRequest, UpdateMetadataRequest};

    use storage_service_protos::google::protobuf::{value::Kind, Value};

    let path = get_random_non_normalized_path();
    let metadata_client = storage_service.metadata_v1alpha_client(interface).await;

    // First, update metadata using a non-normalized URL
    let uri = storage_service.s3_url_builder().path(&path).to_string();
    let metadata = Value {
        kind: Some(Kind::StringValue("test_value".into())),
    };

    let update_response = metadata_client
        .update_metadata(UpdateMetadataRequest {
            uri: uri.clone(),
            user_metadata_key: "test_key".into(),
            user_metadata: Some(metadata.clone()),
            ..Default::default()
        })
        .await
        .expect("metadata update failed");

    // Delete metadata using a non-normalized URL
    metadata_client
        .delete_metadata(DeleteMetadataRequest {
            uri: uri.clone(),
            user_metadata_key: "test_key".into(),
            expected_etag: Some(update_response.etag),
        })
        .await
        .expect("metadata delete failed");

    // Verify the metadata was deleted by trying to get it
    let get_response = metadata_client
        .get_metadata(GetMetadataRequest {
            uri: uri.clone(),
            user_metadata_keys: vec!["test_key".into()],
        })
        .await
        .expect("metadata get failed");

    // The metadata should not be present
    assert!(
        !get_response.user_metadata.contains_key("test_key"),
        "Expected test_key to be deleted"
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn delete_metadata_with_invalid_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::metadata::types::v1alpha::DeleteMetadataRequest;

    let metadata_client = storage_service.metadata_v1alpha_client(interface).await;
    let status = metadata_client
        .delete_metadata(DeleteMetadataRequest {
            uri: "invalid-url.com".to_string(),
            user_metadata_key: "test_key".into(),
            expected_etag: Some("test_etag".into()),
        })
        .await
        .expect_err("metadata delete with invalid url is expected to fail");
    assert_eq!(status.code(), Code::InvalidArgument);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn enumerate_versions_with_normalized_url(
    storage_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::versioning::types::v1alpha::EnumerateVersionsRequest;

    let path = get_random_non_normalized_path();
    let fileobject_client = storage_service.fileobject_v1alpha_client(interface).await;
    let versioning_client = storage_service.versioning_v1alpha_client(interface).await;

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
    use common::clients::versioning::types::v1alpha::EnumerateVersionsRequest;

    let versioning_client = storage_service.versioning_v1alpha_client(interface).await;
    let mut stream = versioning_client.enumerate_versions(EnumerateVersionsRequest {
        resource_address: "invalid-url.com".to_string(),
    });
    let status = stream
        .try_next()
        .await
        .expect_err("enumerate versions is expected to fail");
    assert_eq!(status.code(), Code::InvalidArgument);
}

fn create_path_pair() -> (String, String) {
    let root = get_random_path_item();
    let name = get_random_path_item();

    (
        format!("{}/{}/.././/{}", root, get_random_path_item(), name),
        format!("{}/{}", root, name),
    )
}

fn create_random_value() -> Value {
    use storage_service_protos::google::protobuf::value::Kind;

    Value {
        kind: Some(Kind::StringValue(get_random_alphanumeric_string(32))),
    }
}

fn create_get_metadata_batch_resource_address_request_entry(
    uri: impl Into<String>,
    key: impl Into<String>,
) -> GetMetadataBatchRequestEntry {
    use crate::common::clients::metadata::types::v1alpha::*;

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
    use crate::common::clients::metadata::types::v1alpha::*;

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
    use crate::common::clients::metadata::types::v1alpha::*;

    GetMetadataBatchRequestEntry {
        resource_location: Some(ResourceLocation::folder_address(uri)),
        user_metadata_key: key.into(),
        namespace: None,
    }
}

fn create_not_found_batch_error_entry() -> BatchErrorEntry {
    use crate::common::clients::metadata::types::v1alpha::batch_error_entry::Code;

    BatchErrorEntry {
        code: Code::NotFound as i32,
        message: "metadata key not found".into(),
    }
}

fn create_resource_uri_invalid_error_entry(message: &str) -> BatchErrorEntry {
    use crate::common::clients::metadata::types::v1alpha::batch_error_entry::Code;

    BatchErrorEntry {
        code: Code::InvalidArgument as i32,
        message: message.into(),
    }
}

fn create_get_metadata_batch_success_response_entry(
    request_index: u32,
    value: Value,
    etag: impl Into<String>,
) -> GetMetadataBatchResponseEntry {
    use crate::common::clients::metadata::types::v1alpha::{
        get_metadata_batch_response_entry::Outcome, UserMetadataValue,
    };

    GetMetadataBatchResponseEntry {
        request_index,
        outcome: Some(Outcome::Success(UserMetadataValue {
            value: Some(value),
            etag: etag.into(),
        })),
    }
}

fn create_get_metadata_batch_not_found_response_entry(request_index: u32) -> GetMetadataBatchResponseEntry {
    use crate::common::clients::metadata::types::v1alpha::get_metadata_batch_response_entry::Outcome;

    GetMetadataBatchResponseEntry {
        request_index,
        outcome: Some(Outcome::Error(create_not_found_batch_error_entry())),
    }
}

fn create_get_metadata_batch_invalid_resource_uri_entry(request_index: u32) -> GetMetadataBatchResponseEntry {
    use crate::common::clients::metadata::types::v1alpha::get_metadata_batch_response_entry::Outcome;

    GetMetadataBatchResponseEntry {
        request_index,
        outcome: Some(Outcome::Error(create_resource_uri_invalid_error_entry(
            "resource address is invalid",
        ))),
    }
}

fn create_delete_metadata_batch_success_entry(request_index: u32) -> DeleteMetadataBatchResponseEntry {
    use crate::common::clients::metadata::types::v1alpha::delete_metadata_batch_response_entry::Outcome;

    DeleteMetadataBatchResponseEntry {
        request_index,
        outcome: Some(Outcome::Success(Default::default())),
    }
}

fn create_delete_metadata_batch_invalid_resource_uri_entry(
    request_index: u32,
    message: &str,
) -> DeleteMetadataBatchResponseEntry {
    use crate::common::clients::metadata::types::v1alpha::delete_metadata_batch_response_entry::Outcome;

    DeleteMetadataBatchResponseEntry {
        request_index,
        outcome: Some(Outcome::Error(create_resource_uri_invalid_error_entry(message))),
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

fn unwrap_update_metadata_batch_response_entry_etag(response: &UpdateMetadataBatchResponseEntry) -> &str {
    use crate::common::clients::metadata::types::v1alpha::update_metadata_batch_response_entry::Outcome;

    let Outcome::Success(value) = response.outcome.as_ref().unwrap() else {
        panic!("success outcome is expected");
    };

    value.etag.as_str()
}
