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

use std::future::Future;

use futures::{Stream, TryStreamExt};
use rstest::{fixture, rstest};
use tonic::{Code, Status};

use crate::common::{
    clients::fileobject::types::v1beta::UnwrapIdentityExt,
    random::{get_random_bytes, get_random_path},
    service::Service,
    types::{Backend, Interface},
    url::UrlBuilder,
};

mod common;

#[fixture]
#[once]
fn service() -> Service {
    Service::builder()
        .enable_az(true)
        .enable_s3(true)
        .enable_postgres_metadata()
        .start()
}

const VISIBLE_ENDPOINT: &str = "https://visible.s3.us-west-2.amazonaws.com";
const UNLISTED_ENDPOINT: &str = "https://unlisted.s3.us-west-2.amazonaws.com";
const HIDDEN_ENDPOINT: &str = "https://hidden.s3.us-west-2.amazonaws.com";

/// Start a service with three fictional endpoints exercising every visibility
/// flag. The buckets are never accessed: the listings only reflect the
/// configuration.
fn start_visibility_service() -> Service {
    use serde_json::json;

    let credentials = json!({
        "access_key": {
            "access_key_id": "unused",
            "secret_access_key": "unused",
        },
    });

    Service::start_with_config(json!({
        "storage": {
            "azure_blob": { "enabled": false },
            "s3": {
                "endpoints": {
                    VISIBLE_ENDPOINT: { "credentials": &credentials },
                    UNLISTED_ENDPOINT: {
                        "credentials": &credentials,
                        "list_in_top_level_addresses": false,
                    },
                    HIDDEN_ENDPOINT: {
                        "credentials": &credentials,
                        "list_in_routes": false,
                    },
                },
            },
        },
        "rest": { "addr": "0.0.0.0", "port": 0 },
        "grpc": { "addr": "0.0.0.0", "port": 0 },
    }))
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_fileobject_stat(service: &Service, #[values(Backend::Azure, Backend::S3)] backend: Backend) {
    use common::clients::fileobject::types::v1beta::StatRequest;

    let path = get_random_path();
    let fileobject_client = service.fileobject_v1beta_client(Interface::Grpc).await;
    let address = service.url_builder(backend).path(&path).to_string();

    // Create an object.
    fileobject_client
        .write_random_object(&address, 32)
        .await
        .expect("object write failed");

    // Stat the object. The returned identity must belong to the object created above.
    let stat_response = fileobject_client
        .stat(StatRequest {
            resource_address: address.clone(),
        })
        .await
        .expect("stat failed");
    let resource_info = stat_response.resource_info.expect("resource info is missing");

    let resource_identity = resource_info
        .resource_identity
        .expect("resource identity is missing")
        .encoded_identity;
    assert_identity_belongs_to_address(resource_identity, address);

    let metadata = resource_info.metadata.expect("metadata is missing");
    assert_eq!(metadata.data_object_size, Some(32));
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_fileobject_stat_unknown(service: &Service) {
    use common::clients::fileobject::types::v1beta::StatRequest;

    let fileobject_client = service.fileobject_v1beta_client(Interface::Grpc).await;
    let status = fileobject_client
        .stat(StatRequest {
            resource_address: "https://unknown".into(),
        })
        .await
        .expect_err("stat is expected to fail");

    assert_unsupported_address_status(status);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_fileobject_read(service: &Service, #[values(Backend::Azure, Backend::S3)] backend: Backend) {
    let path = get_random_path();
    let fileobject_client = service.fileobject_v1beta_client(Interface::Grpc).await;

    // Create an object.
    let address = service.url_builder(backend).path(path).to_string();
    let (resource_info, content) = fileobject_client
        .write_random_object(address, 32)
        .await
        .expect("object write failed");

    // Read the object. The returned content must match.
    let (_, read_content) = fileobject_client
        .read_object(resource_info.unwrap_identity())
        .await
        .expect("object read failed");

    assert_eq!(read_content, content);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_fileobject_read_unknown(service: &Service) {
    let fileobject_client = service.fileobject_v1beta_client(Interface::Grpc).await;

    let status = fileobject_client
        .read_object("https://unknown")
        .await
        .expect_err("read is expected to fail");

    assert_unsupported_identity_status(status);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_fileobject_read_from_address(service: &Service, #[values(Backend::Azure, Backend::S3)] backend: Backend) {
    let path = get_random_path();
    let fileobject_client = service.fileobject_v1beta_client(Interface::Grpc).await;

    // Write an object.
    let address = service.url_builder(backend).path(path).to_string();
    let (_, content) = fileobject_client
        .write_random_object(&address, 32)
        .await
        .expect("object write failed");

    // Read the object. The returned content must match.
    let (_, read_content) = fileobject_client
        .read_object_from_address(address)
        .await
        .expect("object read failed");

    assert_eq!(read_content, content);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_fileobject_read_from_address_unknown(service: &Service) {
    let fileobject_client = service.fileobject_v1beta_client(Interface::Grpc).await;

    let status = fileobject_client
        .read_object_from_address("https://unknown")
        .await
        .expect_err("read from address is expected to fail");

    assert_unsupported_address_status(status);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_fileobject_write_direct(service: &Service, #[values(Backend::Azure, Backend::S3)] backend: Backend) {
    use common::clients::fileobject::types::v1beta::{UploadPreference, WriteParameters};

    let path = get_random_path();
    let fileobject_client = service.fileobject_v1beta_client(Interface::Grpc).await;

    let (_, preference) = fileobject_client
        .write_object(
            WriteParameters {
                destination_resource_address: service.url_builder(backend).path(path).to_string(),
                data_object_size: 32,
                upload_preference: Some(UploadPreference::Body as i32),
                ..Default::default()
            },
            get_random_bytes(32),
        )
        .await
        .expect("object write failed");

    assert_eq!(preference, UploadPreference::Body);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_fileobject_write_redirect(service: &Service, #[values(Backend::Azure, Backend::S3)] backend: Backend) {
    use common::clients::fileobject::types::v1beta::{UploadPreference, WriteParameters};

    let path = get_random_path();
    let fileobject_client = service.fileobject_v1beta_client(Interface::Grpc).await;

    let (_, preference) = fileobject_client
        .write_object(
            WriteParameters {
                destination_resource_address: service.url_builder(backend).path(path).to_string(),
                data_object_size: 32,
                upload_preference: Some(UploadPreference::Redirect as i32),
                ..Default::default()
            },
            get_random_bytes(32),
        )
        .await
        .expect("object write failed");

    assert_eq!(preference, UploadPreference::Redirect);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_fileobject_write_multipart(service: &Service, #[values(Backend::Azure, Backend::S3)] backend: Backend) {
    use common::clients::fileobject::types::v1beta::{UploadPreference, WriteParameters};

    let path = get_random_path();
    let fileobject_client = service.fileobject_v1beta_client(Interface::Grpc).await;

    let (_, preference) = fileobject_client
        .write_object(
            WriteParameters {
                destination_resource_address: service.url_builder(backend).path(path).to_string(),
                data_object_size: 512 * 1024,
                upload_preference: Some(UploadPreference::Multipart as i32),
                ..Default::default()
            },
            get_random_bytes(512 * 1024),
        )
        .await
        .expect("object write failed");

    assert_eq!(preference, UploadPreference::Multipart);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_fileobject_direct_write_unknown(service: &Service) {
    let fileobject_client = service.fileobject_v1beta_client(Interface::Grpc).await;

    let status = fileobject_client
        .write_random_object("https://unknown", 32)
        .await
        .expect_err("write is expected to fail");

    assert_unsupported_address_status(status);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_fileobject_complete_redirect_write_unknown(service: &Service) {
    use common::clients::fileobject::types::v1beta::CompleteRedirectUploadRequest;

    let fileobject_client = service.fileobject_v1beta_client(Interface::Grpc).await;

    let status = fileobject_client
        .complete_redirect_upload(CompleteRedirectUploadRequest {
            destination_resource_address: "https://unsupported".into(),
            ..Default::default()
        })
        .await
        .expect_err("complete redirect upload is expected to fail");

    assert_unsupported_address_status(status);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_fileobject_complete_multipart_write_unknown(service: &Service) {
    use common::clients::fileobject::types::v1beta::CompleteMultipartUploadRequest;

    let fileobject_client = service.fileobject_v1beta_client(Interface::Grpc).await;

    let status = fileobject_client
        .complete_multipart_upload(CompleteMultipartUploadRequest {
            destination_resource_address: "https://unsupported".into(),
            ..Default::default()
        })
        .await
        .expect_err("complete multipart upload is expected to fail");

    assert_unsupported_address_status(status);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_fileobject_abort_multipart_write_unknown(service: &Service) {
    use common::clients::fileobject::types::v1beta::AbortMultipartUploadRequest;

    let fileobject_client = service.fileobject_v1beta_client(Interface::Grpc).await;

    let status = fileobject_client
        .abort_multipart_upload(AbortMultipartUploadRequest {
            destination_resource_address: "https://unsupported".into(),
            ..Default::default()
        })
        .await
        .expect_err("abort multipart upload is expected to fail");

    assert_unsupported_address_status(status);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_fileobject_delete(service: &Service, #[values(Backend::Azure, Backend::S3)] backend: Backend) {
    use common::clients::fileobject::types::v1beta::{DeleteRequest, StatRequest};

    let path = get_random_path();
    let fileobject_client = service.fileobject_v1beta_client(Interface::Grpc).await;

    // Create an object. We must be able to stat it.
    let address = service.url_builder(backend).path(&path).to_string();
    fileobject_client
        .write_random_object(&address, 32)
        .await
        .expect("object write failed");

    fileobject_client
        .stat(StatRequest {
            resource_address: address.clone(),
        })
        .await
        .expect("object stat failed");

    // Delete the object.
    fileobject_client
        .delete(DeleteRequest {
            resource_address: address.clone(),
            ..Default::default()
        })
        .await
        .expect("object delete failed");

    // Stat the object. The operation must finish with a "not found" status.
    let status = fileobject_client
        .stat(StatRequest {
            resource_address: address,
        })
        .await
        .expect_err("stat is expected to fail");

    assert_eq!(status.code(), Code::NotFound);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_fileobject_delete_unknown(service: &Service) {
    use common::clients::fileobject::types::v1beta::DeleteRequest;

    let fileobject_client = service.fileobject_v1beta_client(Interface::Grpc).await;

    let status = fileobject_client
        .delete(DeleteRequest {
            resource_address: "https://unsupported".into(),
            ..Default::default()
        })
        .await
        .expect_err("delete is expected to fail");

    assert_unsupported_address_status(status);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_fileobject_fetch_write_type_info(
    service: &Service,
    #[values(Backend::Azure, Backend::S3)] backend: Backend,
) {
    use common::clients::fileobject::types::v1beta::FetchWriteTypeInfoRequest;

    let path = get_random_path();
    let fileobject_client = service.fileobject_v1beta_client(Interface::Grpc).await;

    fileobject_client
        .fetch_write_type_info(FetchWriteTypeInfoRequest {
            destination_resource_address: service.url_builder(backend).path(path).to_string(),
        })
        .await
        .expect("write type info fetch failed");
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_fileobject_fetch_write_type_info_unknown(service: &Service) {
    use common::clients::fileobject::types::v1beta::FetchWriteTypeInfoRequest;

    let fileobject_client = service.fileobject_v1beta_client(Interface::Grpc).await;

    let status = fileobject_client
        .fetch_write_type_info(FetchWriteTypeInfoRequest {
            destination_resource_address: "https://unsupported".into(),
        })
        .await
        .expect_err("fetch write type info is expected to fail");

    assert_unsupported_address_status(status);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_fileobject_enumerate(service: &Service, #[values(Backend::Azure, Backend::S3)] backend: Backend) {
    use common::clients::fileobject::types::v1beta::EnumerateRequest;

    let path = get_random_path();
    let fileobject_client = service.fileobject_v1beta_client(Interface::Grpc).await;

    // Create an object under the path.
    fileobject_client
        .write_random_object(
            service
                .url_builder(backend)
                .path(format!("{}/object", path))
                .to_string(),
            32,
        )
        .await
        .expect("object write failed");

    // Enumerate the folder.
    fileobject_client
        .enumerate(EnumerateRequest {
            resource_address: service.url_builder(backend).path(path).to_string(),
        })
        .first_error()
        .await
        .expect("enumerate failed");
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_fileobject_enumerate_bucket_root(
    service: &Service,
    #[values(Backend::Azure, Backend::S3)] backend: Backend,
    #[values("", "/")] trailing_slash: &str,
) {
    use common::{clients::fileobject::types::v1beta::EnumerateRequest, env::get_env};

    let bucket_uri = match backend {
        Backend::Azure => get_env().unwrap_azure().storage_container_url(),
        Backend::S3 => get_env().unwrap_s3().bucket_url(),
    };
    let resource_address = format!("{}{trailing_slash}", bucket_uri.trim_end_matches('/'));
    let client = service.fileobject_v1beta_client(Interface::Grpc).await;

    let _ = client
        .enumerate(EnumerateRequest { resource_address })
        .try_next()
        .await
        .expect("bucket root enumerate failed");
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_fileobject_enumerate_unknown(service: &Service) {
    use common::clients::fileobject::types::v1beta::EnumerateRequest;

    let fileobject_client = service.fileobject_v1beta_client(Interface::Grpc).await;

    let status = fileobject_client
        .enumerate(EnumerateRequest {
            resource_address: "https://unsupported".into(),
        })
        .first_error()
        .await
        .expect_err("enumerate is expected to fail");

    assert_unsupported_address_status(status);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_filefolder_list(service: &Service, #[values(Backend::Azure, Backend::S3)] backend: Backend) {
    use common::clients::filefolder::types::v1beta::{FolderAddress, ListRequest};

    let path = get_random_path();
    let fileobject_client = service.fileobject_v1beta_client(Interface::Grpc).await;
    let filefolder_client = service.filefolder_v1beta_client(Interface::Grpc).await;

    // Create an object under the folder.
    fileobject_client
        .write_random_object(
            service
                .url_builder(backend)
                .path(format!("{}/object", path))
                .to_string(),
            32,
        )
        .await
        .expect("object write failed");

    // List the folder.
    filefolder_client
        .list(ListRequest {
            folder: Some(FolderAddress {
                uri: service.url_builder(backend).path(path).to_string(),
            }),
        })
        .first_error()
        .await
        .expect("list failed");
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_filefolder_list_unknown(service: &Service) {
    use common::clients::filefolder::types::v1beta::{FolderAddress, ListRequest};

    let filefolder_client = service.filefolder_v1beta_client(Interface::Grpc).await;

    let status = filefolder_client
        .list(ListRequest {
            folder: Some(FolderAddress {
                uri: "https://unknown".into(),
            }),
        })
        .first_error()
        .await
        .expect_err("list is expected to fail");

    assert_unsupported_address_status(status);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_filefolder_list_stat(service: &Service, #[values(Backend::Azure, Backend::S3)] backend: Backend) {
    use common::clients::filefolder::types::v1beta::{FolderAddress, ListStatRequest};

    let path = get_random_path();
    let fileobject_client = service.fileobject_v1beta_client(Interface::Grpc).await;
    let filefolder_client = service.filefolder_v1beta_client(Interface::Grpc).await;

    // Create an object under the folder.
    fileobject_client
        .write_random_object(
            service
                .url_builder(backend)
                .path(format!("{}/object", path))
                .to_string(),
            32,
        )
        .await
        .expect("object write failed");

    // ListStat the folder.
    filefolder_client
        .list_stat(ListStatRequest {
            folder: Some(FolderAddress {
                uri: service.url_builder(backend).path(path).to_string(),
            }),
        })
        .first_error()
        .await
        .expect("list failed");
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_filefolder_list_stat_unknown(service: &Service) {
    use common::clients::filefolder::types::v1beta::{FolderAddress, ListStatRequest};

    let filefolder_client = service.filefolder_v1beta_client(Interface::Grpc).await;

    let status = filefolder_client
        .list_stat(ListStatRequest {
            folder: Some(FolderAddress {
                uri: "https://unknown".into(),
            }),
        })
        .first_error()
        .await
        .expect_err("list is expected to fail");

    assert_unsupported_address_status(status);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_capabilities_list_top_level_addresses(service: &Service) {
    use common::clients::capabilities::types::v1beta::TopLevelAddressEntry;

    let capabilities_client = service.capabilities_v1beta_client(Interface::Grpc).await;

    // List top-level addresses. Both the s3 and the azure blob root addresses, without the trailing slash, must appear
    // in the response.
    let response = capabilities_client
        .list_top_level_addresses(Default::default())
        .await
        .expect("top level addresses list failed");

    assert!(response.items.contains(&TopLevelAddressEntry {
        top_level_address: {
            let mut address = service.az_url_builder().to_string();
            address.pop();

            address
        }
    }));
    assert!(response.items.contains(&TopLevelAddressEntry {
        top_level_address: {
            let mut address = service.s3_url_builder().to_string();
            address.pop();

            address
        }
    }));
}

#[test_log::test(tokio::test)]
async fn test_top_level_addresses_hide_unlisted_endpoints() {
    use common::clients::capabilities::types::v1beta::TopLevelAddressEntry;

    let service = start_visibility_service();
    let client = service.capabilities_v1beta_client(Interface::Grpc).await;
    let tla = |address: &str| TopLevelAddressEntry {
        top_level_address: address.into(),
    };

    let response = client
        .list_top_level_addresses(Default::default())
        .await
        .expect("top level addresses list failed");

    assert!(response.items.contains(&tla(VISIBLE_ENDPOINT)));
    assert!(!response.items.contains(&tla(UNLISTED_ENDPOINT)));
    assert!(!response.items.contains(&tla(HIDDEN_ENDPOINT)));
}

#[test_log::test(tokio::test)]
async fn test_endpoint_hidden_from_routes_is_still_served() {
    use common::env::get_env;
    use serde_json::json;

    let env = get_env().unwrap_s3();
    let hidden_endpoint = format!("http://{}:{}/{}", env.host, env.port, env.bucket);
    let service = Service::start_with_config(json!({
        "storage": {
            "azure_blob": { "enabled": false },
            "s3": {
                "endpoints": {
                    &hidden_endpoint: {
                        "credentials": {
                            "access_key": {
                                "access_key_id": &env.access_key_id,
                                "secret_access_key": &env.secret_access_key,
                            },
                        },
                        "endpoint_url": env.endpoint_url(),
                        "addressing_style": "path",
                        "list_in_routes": false,
                    },
                },
            },
        },
        "rest": { "addr": "0.0.0.0", "port": 0 },
        "grpc": { "addr": "0.0.0.0", "port": 0 },
    }));
    let client = service.fileobject_v1beta_client(Interface::Grpc).await;
    let hidden_address = format!("{hidden_endpoint}/{}", get_random_path());

    client
        .write_random_object(&hidden_address, 32)
        .await
        .expect("object write failed");
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_versioning_enumerate_versions(
    service: &Service,
    #[values(Backend::Azure, Backend::S3)] backend: Backend,
) {
    use common::clients::versioning::types::v1beta::EnumerateVersionsRequest;

    let path = get_random_path();
    let fileobject_client = service.fileobject_v1beta_client(Interface::Grpc).await;
    let versioning_client = service.versioning_v1beta_client(Interface::Grpc).await;

    // Create an object with a single version.
    let address = service.url_builder(backend).path(path).to_string();
    fileobject_client
        .write_random_object(&address, 32)
        .await
        .expect("object write failed");

    // Enumerate versions of the object. The request must succeed.
    versioning_client
        .enumerate_versions(EnumerateVersionsRequest {
            resource_address: address,
        })
        .first_error()
        .await
        .expect("versions enumerate failed");
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_versioning_enumerate_versions_unknown(service: &Service) {
    use common::clients::versioning::types::v1beta::EnumerateVersionsRequest;

    let versioning_client = service.versioning_v1beta_client(Interface::Grpc).await;

    let status = versioning_client
        .enumerate_versions(EnumerateVersionsRequest {
            resource_address: "https://unexpected".into(),
        })
        .first_error()
        .await
        .expect_err("enumerate versions is expected to fail");

    assert_unsupported_address_status(status);
}

#[track_caller]
fn assert_identity_belongs_to_address(identity: impl AsRef<str>, address: impl AsRef<str>) {
    let identity = identity.as_ref();
    let address = address.as_ref();

    let (prefix, _) = identity
        .split_once("?")
        .expect("query parameters are missing from identity");
    assert_eq!(prefix, address);
}

#[track_caller]
fn assert_unsupported_address_status(status: Status) {
    assert_eq!(status.code(), Code::InvalidArgument);
    assert!(status.message().contains("resource address is not supported"));
}

#[track_caller]
fn assert_unsupported_identity_status(status: Status) {
    assert_eq!(status.code(), Code::InvalidArgument);
    assert!(status.message().contains("resource identity is not supported"));
}

impl Service {
    pub fn url_builder(&self, backend: Backend) -> UrlBuilder {
        match backend {
            Backend::Azure => self.az_url_builder(),
            Backend::S3 => self.s3_url_builder(),
        }
    }
}

trait StreamExt {
    fn first_error(self) -> impl Future<Output = Result<(), Status>>;
}

impl<V, T: Stream<Item = Result<V, Status>> + Unpin> StreamExt for T {
    async fn first_error(mut self) -> Result<(), Status> {
        while self.try_next().await?.is_some() {}

        Ok(())
    }
}
