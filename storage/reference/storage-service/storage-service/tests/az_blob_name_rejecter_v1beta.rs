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

use rstest::{fixture, rstest};
use tonic::Code;

use common::{service::Service, types::Interface};

#[fixture]
#[once]
fn storage_service() -> Service {
    Service::builder().enable_az(false).start()
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_fileobject_stat_invalid_blob_path_is_rejected(storage_service: &Service) {
    use common::{clients::fileobject::types::v1beta::StatRequest, env::get_env};

    let client = storage_service.fileobject_v1beta_client(Interface::Grpc).await;
    let env = get_env().unwrap_azure();
    let uri = format!("https://{}.blob.core.windows.net/container/blob.", env.storage_account);

    let error = client.stat(StatRequest { resource_address: uri }).await.unwrap_err();

    assert_eq!(error.code(), Code::InvalidArgument);
    assert_eq!(
        error.message(),
        "azure blob resource address is invalid: decoded path segment ends in a dot"
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_fileobject_stat_percent_encoded_path_invalid_blob_path_is_rejected(storage_service: &Service) {
    use common::{clients::fileobject::types::v1beta::StatRequest, env::get_env};

    let client = storage_service.fileobject_v1beta_client(Interface::Grpc).await;
    let env = get_env().unwrap_azure();
    let uri = format!(
        "https://{}.blob.core.windows.net/%63%6F%6E%74%61%69%6E%65%72%2F%62%6C%6F%62%2E",
        env.storage_account
    );

    let error = client.stat(StatRequest { resource_address: uri }).await.unwrap_err();

    assert_eq!(error.code(), Code::InvalidArgument);
    assert_eq!(
        error.message(),
        "azure blob resource address is invalid: decoded path segment ends in a dot"
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_fileobject_read_from_address_invalid_blob_path_is_rejected(storage_service: &Service) {
    use common::clients::fileobject::types::v1beta::ReadFromAddressRequest;

    let client = storage_service.fileobject_v1beta_client(Interface::Grpc).await;
    let uri = storage_service.az_url_builder().path("blob.").to_string();

    let error = client
        .read_from_address(ReadFromAddressRequest {
            resource_address: uri,
            ..Default::default()
        })
        .await
        .unwrap_err();

    assert_eq!(error.code(), Code::InvalidArgument);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_fileobject_delete_invalid_blob_path_is_rejected(storage_service: &Service) {
    use common::clients::fileobject::types::v1beta::DeleteRequest;

    let client = storage_service.fileobject_v1beta_client(Interface::Grpc).await;
    let uri = storage_service.az_url_builder().path("blob.").to_string();

    let error = client
        .delete(DeleteRequest {
            resource_address: uri,
            ..Default::default()
        })
        .await
        .unwrap_err();

    assert_eq!(error.code(), Code::InvalidArgument);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_fileobject_enumerate_invalid_blob_path_is_rejected(storage_service: &Service) {
    use common::clients::fileobject::types::v1beta::EnumerateRequest;

    let client = storage_service.fileobject_v1beta_client(Interface::Grpc).await;
    let uri = storage_service.az_url_builder().path("blob.").to_string();

    let error = client
        .enumerate_collect(EnumerateRequest { resource_address: uri })
        .await
        .unwrap_err();

    assert_eq!(error.code(), Code::InvalidArgument);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_fileobject_abort_multipart_upload_invalid_blob_path_is_rejected(storage_service: &Service) {
    use common::clients::fileobject::types::v1beta::AbortMultipartUploadRequest;

    let client = storage_service.fileobject_v1beta_client(Interface::Grpc).await;
    let uri = storage_service.az_url_builder().path("blob.").to_string();

    let error = client
        .abort_multipart_upload(AbortMultipartUploadRequest {
            destination_resource_address: uri,
            ..Default::default()
        })
        .await
        .unwrap_err();

    assert_eq!(error.code(), Code::InvalidArgument);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_fileobject_complete_multipart_upload_invalid_blob_path_is_rejected(storage_service: &Service) {
    use common::clients::fileobject::types::v1beta::CompleteMultipartUploadRequest;

    let client = storage_service.fileobject_v1beta_client(Interface::Grpc).await;
    let uri = storage_service.az_url_builder().path("blob.").to_string();

    let error = client
        .complete_multipart_upload(CompleteMultipartUploadRequest {
            destination_resource_address: uri,
            ..Default::default()
        })
        .await
        .unwrap_err();

    assert_eq!(error.code(), Code::InvalidArgument);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_fileobject_upload_part_invalid_blob_path_is_rejected(storage_service: &Service) {
    use common::clients::fileobject::types::v1beta::UploadPartRequest;

    let client = storage_service.fileobject_v1beta_client(Interface::Grpc).await;
    let uri = storage_service.az_url_builder().path("blob.").to_string();

    let error = client
        .upload_part(UploadPartRequest {
            destination_resource_address: uri,
            ..Default::default()
        })
        .await
        .unwrap_err();

    assert_eq!(error.code(), Code::InvalidArgument);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_fileobject_complete_redirect_upload_invalid_blob_path_is_rejected(storage_service: &Service) {
    use common::clients::fileobject::types::v1beta::CompleteRedirectUploadRequest;

    let client = storage_service.fileobject_v1beta_client(Interface::Grpc).await;
    let uri = storage_service.az_url_builder().path("blob.").to_string();

    let error = client
        .complete_redirect_upload(CompleteRedirectUploadRequest {
            destination_resource_address: uri,
            ..Default::default()
        })
        .await
        .unwrap_err();

    assert_eq!(error.code(), Code::InvalidArgument);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_fileobject_fetch_write_type_info_invalid_blob_path_is_rejected(storage_service: &Service) {
    use common::clients::fileobject::types::v1beta::FetchWriteTypeInfoRequest;

    let client = storage_service.fileobject_v1beta_client(Interface::Grpc).await;
    let uri = storage_service.az_url_builder().path("blob.").to_string();

    let error = client
        .fetch_write_type_info(FetchWriteTypeInfoRequest {
            destination_resource_address: uri,
        })
        .await
        .unwrap_err();

    assert_eq!(error.code(), Code::InvalidArgument);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_fileobject_write_invalid_blob_path_is_rejected(storage_service: &Service) {
    use common::clients::fileobject::types::v1beta::WriteParameters;

    let client = storage_service.fileobject_v1beta_client(Interface::Grpc).await;
    let uri = storage_service.az_url_builder().path("blob.").to_string();

    let error = client
        .write(
            WriteParameters {
                destination_resource_address: uri,
                ..Default::default()
            },
            b"".as_slice(),
        )
        .await
        .unwrap_err();

    assert_eq!(error.code(), Code::InvalidArgument);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_fileobject_read_invalid_blob_path_is_rejected(storage_service: &Service) {
    use common::clients::fileobject::types::v1beta::{ReadRequest, ResourceIdentity};

    let client = storage_service.fileobject_v1beta_client(Interface::Grpc).await;
    let uri = storage_service.az_url_builder().path("blob.").to_string();

    let error = client
        .read(ReadRequest {
            resource_identity: Some(ResourceIdentity { encoded_identity: uri }),
            ..Default::default()
        })
        .await
        .unwrap_err();

    assert_eq!(error.code(), Code::InvalidArgument);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_filefolder_list_invalid_blob_path_is_rejected(storage_service: &Service) {
    use common::clients::filefolder::types::v1beta::{FolderAddress, ListRequest};

    let client = storage_service.filefolder_v1beta_client(Interface::Grpc).await;
    let uri = storage_service.az_url_builder().path("blob.").to_string();

    let error = client
        .list_collect(ListRequest {
            folder: Some(FolderAddress { uri }),
        })
        .await
        .unwrap_err();

    assert_eq!(error.code(), Code::InvalidArgument);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_filefolder_list_stat_invalid_blob_path_is_rejected(storage_service: &Service) {
    use common::clients::filefolder::types::v1beta::{FolderAddress, ListStatRequest};

    let client = storage_service.filefolder_v1beta_client(Interface::Grpc).await;
    let uri = storage_service.az_url_builder().path("blob.").to_string();

    let error = client
        .list_stat_collect(ListStatRequest {
            folder: Some(FolderAddress { uri }),
        })
        .await
        .unwrap_err();

    assert_eq!(error.code(), Code::InvalidArgument);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_filefolder_delete_folder_invalid_blob_path_is_rejected(storage_service: &Service) {
    use common::clients::filefolder::types::v1beta::{DeleteFolderRequest, FolderAddress};

    let client = storage_service.filefolder_v1beta_client(Interface::Grpc).await;
    let uri = storage_service.az_url_builder().path("blob.").to_string();

    let error = client
        .delete_folder(DeleteFolderRequest {
            folder: Some(FolderAddress { uri }),
        })
        .await
        .unwrap_err();

    assert_eq!(error.code(), Code::InvalidArgument);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_versioning_enumerate_versions_invalid_blob_path_is_rejected(storage_service: &Service) {
    use common::clients::versioning::types::v1beta::EnumerateVersionsRequest;

    let client = storage_service.versioning_v1beta_client(Interface::Grpc).await;
    let uri = storage_service.az_url_builder().path("blob.").to_string();

    let error = client
        .enumerate_versions_collect(EnumerateVersionsRequest { resource_address: uri })
        .await
        .unwrap_err();

    assert_eq!(error.code(), Code::InvalidArgument);
}
