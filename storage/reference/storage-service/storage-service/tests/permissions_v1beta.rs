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

use std::ops::Deref;

use crate::common::clients::fileobject::types::v1beta::{ResourceInfo, UnwrapIdentityExt, UploadPreference};
use crate::common::clients::utils::{WithInvalidAuthorization, WithoutAuthorization};
use crate::common::env::get_env;
use crate::common::random::{get_random_bytes, get_random_path};
use crate::common::service::Service as StorageService;
use crate::common::url::get_stat_query_pairs;
use bytes::Bytes;
use common::types::Interface;
use futures::TryStreamExt;
use rstest::{fixture, rstest};
use storage_service_test::mocks::permissions::Decision;
use storage_service_test::mocks::permissions::{
    grpc::Service as GrpcPermissionService, rest::Service as RestPermissionService, Context,
};

use tonic::Code;

#[derive(Debug, Copy, Clone)]
enum PermissionBackend {
    Rest,
    Grpc,
}

#[derive(Debug, Copy, Clone)]
struct ServiceSet<'a> {
    storage_service: &'a StorageService,
    permission_context: &'a Context,
}

impl ServiceSet<'_> {
    async fn create_random_object(&self, address: impl Into<String>) -> (ResourceInfo, Bytes) {
        let address = address.into();

        // Temporarily override rules for the object.
        let rule_set = self.permission_context.new_rule_set();
        rule_set.allow_object_write(&address);

        let client = self.fileobject_v1beta_client(Interface::Grpc).await;
        let (resource_info, content) = client
            .write_random_object(address, 32)
            .await
            .expect("object write failed");

        (resource_info, content)
    }

    async fn create_random_objects(
        &self,
        addresses: impl IntoIterator<Item = impl Into<String>>,
    ) -> Vec<(ResourceInfo, Bytes)> {
        let client = self.fileobject_v1beta_client(Interface::Grpc).await;
        let rule_set = self.permission_context.new_rule_set();

        let mut results = Vec::new();
        for address in addresses.into_iter() {
            let address = address.into();
            rule_set.allow_object_write(&address);

            results.push(
                client
                    .write_random_object(address, 32)
                    .await
                    .expect("object write failed"),
            );
        }

        results
    }

    async fn get_resource_identities(&self, addresses: impl IntoIterator<Item = impl Into<String>>) -> Vec<String> {
        use crate::common::clients::fileobject::types::v1beta::StatRequest;

        let client = self.fileobject_v1beta_client(Interface::Grpc).await;
        let rule_set = self.permission_context.new_rule_set();

        let mut results = Vec::new();
        for address in addresses.into_iter() {
            let address = address.into();
            rule_set.allow_object_read(&address);

            results.push(
                client
                    .stat(StatRequest {
                        resource_address: address,
                    })
                    .await
                    .expect("stat failed")
                    .resource_info
                    .expect("resource info is missing from the response")
                    .unwrap_identity(),
            );
        }

        results
    }
}

impl Deref for ServiceSet<'_> {
    type Target = StorageService;

    fn deref(&self) -> &Self::Target {
        self.storage_service
    }
}

struct StorageFixture {
    rest_storage_service: StorageService,
    grpc_storage_service: StorageService,
    rest_permission_context: Context,
    grpc_permission_context: Context,
    _rest_permission_service: RestPermissionService,
    _grpc_permission_service: GrpcPermissionService,
}

impl StorageFixture {
    fn new() -> Self {
        Self::with_az_permissions(true)
    }

    /// Build a fixture whose Azure endpoint either enforces the permission layer
    /// or opts out of it (`enable_az_permissions = false` for a public bucket).
    fn with_az_permissions(enable_az_permissions: bool) -> Self {
        let rest_permission_context = Context::new();
        let rest_permission_service = RestPermissionService::new(rest_permission_context.clone());

        let grpc_permission_context = Context::new();
        let grpc_permission_service = GrpcPermissionService::new(grpc_permission_context.clone());

        Self {
            rest_storage_service: StorageService::builder()
                .enable_az_with_permissions(true, enable_az_permissions)
                .enable_postgres_metadata()
                .enable_permissions("rest", rest_permission_service.endpoint_url(), false)
                .start(),
            grpc_storage_service: StorageService::builder()
                .enable_az_with_permissions(true, enable_az_permissions)
                .enable_postgres_metadata()
                .enable_permissions("grpc", grpc_permission_service.endpoint_url(), false)
                .start(),
            _grpc_permission_service: grpc_permission_service,
            _rest_permission_service: rest_permission_service,
            rest_permission_context,
            grpc_permission_context,
        }
    }

    fn service_set<'a>(&'a self, backend: PermissionBackend) -> ServiceSet<'a> {
        match backend {
            PermissionBackend::Grpc => ServiceSet {
                storage_service: &self.grpc_storage_service,
                permission_context: &self.grpc_permission_context,
            },
            PermissionBackend::Rest => ServiceSet {
                storage_service: &self.rest_storage_service,
                permission_context: &self.rest_permission_context,
            },
        }
    }
}

#[fixture]
#[once]
fn storage_fixture() -> StorageFixture {
    StorageFixture::new()
}

// A fixture whose Azure endpoint opted out of permission enforcement. Requests
// against it must succeed even when the configured rules deny access, because
// the permission service is never consulted.
#[fixture]
#[once]
fn bypassed_storage_fixture() -> StorageFixture {
    StorageFixture::with_az_permissions(false)
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_stat(storage_fixture: &StorageFixture, #[case] backend: PermissionBackend, #[case] interface: Interface) {
    use common::clients::fileobject::types::v1beta::StatRequest;

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let path = get_random_path();
    let client = service_set.fileobject_v1beta_client(interface).await;

    // Create an object.
    let resource_address = service_set.az_url_builder().path(&path).to_string();
    let (write_resource_info, _) = service_set.create_random_object(&resource_address).await;

    // Configure the read permission for the object and stat it. The operation must succeed.
    rule_set.allow_object_read(&resource_address);
    let stat_response = client
        .stat(StatRequest { resource_address })
        .await
        .expect("object stat failed");

    let write_resource_identity = write_resource_info.unwrap_identity();
    let expected_identity = service_set
        .az_url_builder()
        .path(path)
        .raw_query_params(get_stat_query_pairs(&write_resource_identity))
        .to_string();
    let actual_identity = stat_response
        .resource_info
        .expect("resource info is missing from the response")
        .unwrap_identity();
    assert_eq!(actual_identity, expected_identity);
}

#[rstest]
#[case(
    vec![
        ("a/", "folder", Decision::Deny),
        ("a/b/", "folder", Decision::Allow),
    ],
    Code::Ok
)]
#[case(
    vec![
        ("a/", "folder", Decision::Deny),
        ("a/b/", "folder", Decision::Deny),
        ("a/b/c", "object", Decision::Allow),
    ],
    Code::Ok,
)]
#[case(
    vec![
        ("a/", "folder", Decision::Allow),
        ("a/b/", "folder", Decision::Deny),
    ],
    Code::PermissionDenied,
)]
#[case(
    vec![
        ("a/", "folder", Decision::Allow),
        ("a/b/c", "object", Decision::Deny),
    ],
    Code::PermissionDenied,
)]
#[test_log::test(tokio::test)]
async fn test_stat_evaluates_permissions_hierarchically(
    storage_fixture: &StorageFixture,
    #[case] rules: Vec<(&'static str, &'static str, Decision)>,
    #[case] expected_code: Code,
) {
    use common::clients::fileobject::types::v1beta::StatRequest;

    let service_set = storage_fixture.service_set(PermissionBackend::Grpc);
    let rule_set = service_set.permission_context.new_rule_set();
    let folder_path = get_random_path();
    let client = service_set.fileobject_v1beta_client(Interface::Rest).await;

    // Create an object.
    let resource_address = service_set
        .az_url_builder()
        .path(format!("{}/a/b/c", folder_path))
        .to_string();
    service_set.create_random_object(&resource_address).await;

    // Configure rules.
    for (path, kind, decision) in rules.into_iter() {
        let resource_address = service_set
            .az_url_builder()
            .path(format!("{}/{}", folder_path, path))
            .to_string();
        rule_set.set_decision(kind, resource_address, "read", decision);
    }

    // Stat the object and compare the result with the expectation.
    let code = client
        .stat(StatRequest { resource_address })
        .await
        .map_or_else(|status| status.code(), |_| Code::Ok);

    assert_eq!(code, expected_code);
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_stat_returns_permission_denied_on_forbidden_action(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use common::clients::fileobject::types::v1beta::StatRequest;

    let service_set = storage_fixture.service_set(backend);
    let path = get_random_path();
    let client = service_set.fileobject_v1beta_client(interface).await;

    // Create an object.
    let resource_address = service_set.az_url_builder().path(path).to_string();
    service_set.create_random_object(&resource_address).await;

    // Configure the read permission for the object and stat it. The operation must fail.
    let status = client
        .stat(StatRequest { resource_address })
        .await
        .expect_err("object stat is expected to fail");

    assert_eq!(status.code(), Code::PermissionDenied);
}

#[rstest]
#[test_log::test(tokio::test)]
#[ignore = "not yet fixed"]
async fn test_stat_returns_unauthorized_on_missing_header() {
    todo!();
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_read(storage_fixture: &StorageFixture, #[case] backend: PermissionBackend, #[case] interface: Interface) {
    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let path = get_random_path();
    let client = service_set.fileobject_v1beta_client(interface).await;

    // Create an object.
    let resource_address = service_set.az_url_builder().path(path).to_string();
    let (resource_info, content) = service_set.create_random_object(&resource_address).await;
    let resource_identity = resource_info.unwrap_identity();

    // Configure the read permission for the object and read it. The operation must succeed.
    rule_set.allow_object_read(&resource_address);
    let (read_metadata, read_content) = client.read_object(resource_identity).await.expect("object read failed");

    assert_eq!(read_content, content);
    assert_eq!(read_metadata.data_object_size, Some(content.len() as u64));
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_read_returns_permission_denied_on_forbidden_action(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    let service_set = storage_fixture.service_set(backend);
    let path = get_random_path();
    let client = service_set.fileobject_v1beta_client(interface).await;

    // Create an object.
    let resource_address = service_set.az_url_builder().path(path).to_string();
    service_set.create_random_object(&resource_address).await;

    // Configure the read permission for the object and read it. The operation must fail.
    let status = client
        .read_object(resource_address)
        .await
        .expect_err("read is expected to fail");

    assert_eq!(status.code(), Code::PermissionDenied);
}

#[rstest]
#[test_log::test(tokio::test)]
#[ignore = "not yet fixed"]
async fn test_read_returns_unauthorized_on_missing_header() {
    todo!();
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_read_from_address(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let path = get_random_path();
    let client = service_set.fileobject_v1beta_client(interface).await;

    // Create an object.
    let resource_address = service_set.az_url_builder().path(&path).to_string();
    let (write_resource_info, content) = service_set.create_random_object(&resource_address).await;

    // Configure the read permission for the object and read it. The operation must succeed.
    rule_set.allow_object_read(&resource_address);
    let (read_resource_info, read_content) = client
        .read_object_from_address(resource_address)
        .await
        .expect("object read failed");

    assert_eq!(read_content, content);

    let write_identity = write_resource_info.unwrap_identity();
    let expected_identity = service_set
        .az_url_builder()
        .path(path)
        .raw_query_params(get_stat_query_pairs(&write_identity))
        .to_string();
    assert_eq!(read_resource_info.unwrap_identity(), expected_identity);
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_read_from_address_returns_permission_denied_on_forbidden_action(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    let service_set = storage_fixture.service_set(backend);
    let path = get_random_path();
    let client = service_set.fileobject_v1beta_client(interface).await;

    // Create an object.
    let resource_address = service_set.az_url_builder().path(path).to_string();
    service_set.create_random_object(&resource_address).await;

    // Configure the read permission for the object and read it. The operation must fail.
    let status = client
        .read_object_from_address(resource_address)
        .await
        .expect_err("read is expected to fail");

    assert_eq!(status.code(), Code::PermissionDenied);
}

#[rstest]
#[test_log::test(tokio::test)]
#[ignore = "not yet fixed"]
async fn test_read_from_address_returns_unauthorized_on_missing_header() {
    todo!();
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_enumerate(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::fileobject::types::v1beta::EnumerateRequest;

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.fileobject_v1beta_client(interface).await;
    let folder_path = format!("{}/", get_random_path());
    let subfolder_1_path = format!("{}b/", folder_path);
    let subfolder_2_path = format!("{}c/", folder_path);
    let object_1_path = format!("{}a", folder_path);
    let object_2_path = format!("{}d", subfolder_1_path);
    let object_3_path = format!("{}e", subfolder_2_path);

    // Create objects.
    let expected_addresses: Vec<_> = [object_1_path, object_2_path, object_3_path]
        .into_iter()
        .map(|path| service_set.az_url_builder().path(path).to_string())
        .collect();
    service_set
        .create_random_objects(expected_addresses.iter().cloned())
        .await;

    // Allow folder reads.
    let folder_address = service_set
        .storage_service
        .az_url_builder()
        .path(&folder_path)
        .to_string();
    rule_set.allow_folder_read(&folder_address);

    // Enumerate objects in the folder. The operation must return everything.
    let responses: Vec<_> = client
        .enumerate(EnumerateRequest {
            resource_address: folder_address,
        })
        .try_collect()
        .await
        .expect("enumerate failed");
    let addresses: Vec<_> = responses
        .into_iter()
        .flat_map(|response| response.items)
        .map(|i| i.resource_address)
        .collect();

    assert_eq!(addresses, expected_addresses);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_enumerate_normalizes_folder_uri_if_needed(
    storage_fixture: &StorageFixture,
    #[values("", "/")] maybe_trailing_slash: &str,
) {
    use crate::common::clients::fileobject::types::v1beta::EnumerateRequest;

    let service_set = storage_fixture.service_set(PermissionBackend::Grpc);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.fileobject_v1beta_client(Interface::Grpc).await;
    let folder_path = get_random_path();
    let object_path = format!("{}/object", folder_path);

    // Create the object.
    let object_address = service_set.az_url_builder().path(object_path).to_string();
    service_set.create_random_object(&object_address).await;

    // Allow folder reads.
    rule_set.allow_folder_read(service_set.az_url_builder().path(format!("{}/", folder_path)));

    // Enumerate objects in the folder. The operation must return everything.
    let folder_address = service_set
        .az_url_builder()
        .path(format!("{}{}", folder_path, maybe_trailing_slash))
        .to_string();
    let responses: Vec<_> = client
        .enumerate(EnumerateRequest {
            resource_address: folder_address,
        })
        .try_collect()
        .await
        .expect("enumerate failed");

    let addresses: Vec<_> = responses
        .into_iter()
        .flat_map(|response| response.items)
        .map(|i| i.resource_address)
        .collect();
    assert_eq!(addresses, vec![object_address]);
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_enumerate_returns_permission_denied_on_forbidden_action(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::fileobject::types::v1beta::EnumerateRequest;

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.fileobject_v1beta_client(interface).await;
    let folder_path = get_random_path();

    // Prohibit writing the folder.
    let folder_address = service_set
        .storage_service
        .az_url_builder()
        .path(format!("{}/", folder_path))
        .to_string();
    rule_set.forbid_folder_read(&folder_address);

    // Enumerate objects in the folder. The operation must fail.
    let status = client
        .enumerate(EnumerateRequest {
            resource_address: folder_address,
        })
        .try_next()
        .await
        .expect_err("enumerate is expected to fail");

    assert_eq!(status.code(), Code::PermissionDenied);
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_enumerate_filters_entities_without_access(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::fileobject::types::v1beta::EnumerateRequest;

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.fileobject_v1beta_client(interface).await;
    let folder_path = format!("{}/", get_random_path());
    let subfolder_1_path = format!("{}b/", folder_path);
    let subfolder_2_path = format!("{}c/", folder_path);
    let object_1_path = format!("{}a", folder_path);
    let object_2_path = format!("{}d", subfolder_1_path);
    let object_3_path = format!("{}e", subfolder_2_path);

    // Create objects.
    service_set
        .create_random_objects(
            [&object_1_path, &object_2_path, &object_3_path]
                .into_iter()
                .map(|path| service_set.az_url_builder().path(path)),
        )
        .await;

    // Configure read permission for the root folder and some objects there.
    let folder_address = service_set
        .storage_service
        .az_url_builder()
        .path(&folder_path)
        .to_string();
    rule_set.allow_folder_read(&folder_address);
    rule_set.forbid_folder_read(service_set.az_url_builder().path(subfolder_1_path));
    rule_set.forbid_object_read(service_set.az_url_builder().path(object_3_path));

    // Enumerate objects in the folder. The operation must return everything.
    let responses: Vec<_> = client
        .enumerate(EnumerateRequest {
            resource_address: folder_address,
        })
        .try_collect()
        .await
        .expect("enumerate failed");
    let addresses: Vec<_> = responses
        .into_iter()
        .flat_map(|response| response.items)
        .map(|i| i.resource_address)
        .collect();

    let expected_addresses = vec![service_set.az_url_builder().path(object_1_path).to_string()];
    assert_eq!(addresses, expected_addresses);
}

#[rstest]
#[test_log::test(tokio::test)]
#[ignore = "not yet fixed"]
async fn test_enumerate_returns_unauthorized_on_missing_header() {
    todo!();
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_fetch_write_type_info(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::fileobject::types::v1beta::{
        FetchWriteTypeInfoRequest, FetchWriteTypeInfoResponse, UploadPreference, WriteTypeForSizeInterval,
    };

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.fileobject_v1beta_client(interface).await;
    let path = get_random_path();

    // Enable object read. (????)
    let resource_address = service_set.az_url_builder().path(path).to_string();
    rule_set.allow_object_read(&resource_address);

    // Call FetchWriteTypeInfo for the specified path. The operation must succeed.
    let response = client
        .fetch_write_type_info(FetchWriteTypeInfoRequest {
            destination_resource_address: resource_address,
        })
        .await
        .expect("write type info fetch failed");

    assert_eq!(
        response,
        FetchWriteTypeInfoResponse {
            write_type_intervals: vec![
                WriteTypeForSizeInterval {
                    minimum_data_object_size: 0,
                    maximum_data_object_size: 2000000,
                    preferred_upload_method: UploadPreference::Body as i32,
                },
                WriteTypeForSizeInterval {
                    minimum_data_object_size: 2000000,
                    maximum_data_object_size: 64000000,
                    preferred_upload_method: UploadPreference::Redirect as i32,
                },
                WriteTypeForSizeInterval {
                    minimum_data_object_size: 64000000,
                    maximum_data_object_size: 209715200000,
                    preferred_upload_method: UploadPreference::Multipart as i32,
                },
            ],
        }
    );
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_fetch_write_type_info_returns_permission_denied_on_forbidden_action(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::fileobject::types::v1beta::FetchWriteTypeInfoRequest;

    let service_set = storage_fixture.service_set(backend);
    let client = service_set.fileobject_v1beta_client(interface).await;
    let path = get_random_path();

    // Call FetchWriteTypeInfo for the specified address. The operation must fail.
    let resource_address = service_set.az_url_builder().path(path).to_string();
    let status = client
        .fetch_write_type_info(FetchWriteTypeInfoRequest {
            destination_resource_address: resource_address,
        })
        .await
        .expect_err("fetch write type info is expected to fail");

    assert_eq!(status.code(), Code::PermissionDenied);
}

#[rstest]
#[test_log::test(tokio::test)]
#[ignore = "not yet fixed"]
async fn test_fetch_write_type_info_returns_unauthorized_on_missing_header() {
    todo!();
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc, 32, UploadPreference::Body)]
#[case(PermissionBackend::Grpc, Interface::Grpc, 3200, UploadPreference::Redirect)]
#[case(PermissionBackend::Grpc, Interface::Grpc, 320000, UploadPreference::Multipart)]
#[case(PermissionBackend::Rest, Interface::Rest, 32, UploadPreference::Body)]
#[case(PermissionBackend::Rest, Interface::Rest, 3200, UploadPreference::Redirect)]
#[case(PermissionBackend::Rest, Interface::Rest, 320000, UploadPreference::Multipart)]
#[test_log::test(tokio::test)]
async fn test_write(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
    #[case] size: usize,
    #[case] upload_preference: UploadPreference,
) {
    use crate::common::clients::fileobject::types::v1beta::WriteParameters;

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.fileobject_v1beta_client(interface).await;
    let path = get_random_path();

    // Allow write for the address.
    let resource_address = service_set.az_url_builder().path(path).to_string();
    rule_set.allow_object_write(&resource_address);

    // Call write for the specified address, with the specified upload preference. The operation must succeed.
    // This will ensure that all intermediate calls such as write, upload_part, complete_redirect_upload, and
    // complete_multipart_upload have succeeded.
    let (_, actual_upload_preference) = client
        .write_object(
            WriteParameters {
                destination_resource_address: resource_address,
                upload_preference: Some(upload_preference as i32),
                ..Default::default()
            },
            get_random_bytes(size),
        )
        .await
        .expect("object write failed");

    assert_eq!(actual_upload_preference, upload_preference);
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_write_returns_permission_denied_on_forbidden_action(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::fileobject::types::v1beta::{UploadPreference, WriteParameters};

    let service_set = storage_fixture.service_set(backend);
    let client = service_set.fileobject_v1beta_client(interface).await;
    let path = get_random_path();

    // Call write for the specified address, with the body upload preference. The operation must fail.
    let resource_address = service_set.az_url_builder().path(path).to_string();
    let status = client
        .write_object(
            WriteParameters {
                destination_resource_address: resource_address,
                upload_preference: Some(UploadPreference::Body as i32),
                ..Default::default()
            },
            get_random_bytes(32),
        )
        .await
        .expect_err("object write is expected to fail");

    assert_eq!(status.code(), Code::PermissionDenied);
}

#[rstest]
#[test_log::test(tokio::test)]
#[ignore = "not yet fixed"]
async fn test_write_returns_unauthorized_on_missing_header() {
    todo!();
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_complete_redirect_upload_returns_permission_denied_on_forbidden_action(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::fileobject::types::v1beta::CompleteRedirectUploadRequest;

    let service_set = storage_fixture.service_set(backend);
    let client = service_set.fileobject_v1beta_client(interface).await;
    let path = get_random_path();

    // Attempt to complete a redirect upload. The operation must fail.
    let resource_address = service_set.az_url_builder().path(path).to_string();
    let status = client
        .complete_redirect_upload(CompleteRedirectUploadRequest {
            destination_resource_address: resource_address,
            ..Default::default()
        })
        .await
        .expect_err("complete redirect upload is expected to fail");

    assert_eq!(status.code(), Code::PermissionDenied);
}

#[rstest]
#[test_log::test(tokio::test)]
#[ignore = "not yet fixed"]
async fn test_complete_redirect_upload_returns_unauthorized_on_missing_header() {
    todo!();
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_upload_part_returns_permission_denied_on_forbidden_action(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::fileobject::types::v1beta::UploadPartRequest;

    let service_set = storage_fixture.service_set(backend);
    let client = service_set.fileobject_v1beta_client(interface).await;
    let path = get_random_path();

    // Attempt to upload a part. The operation must fail.
    let resource_address = service_set.az_url_builder().path(path).to_string();
    let status = client
        .upload_part(UploadPartRequest {
            destination_resource_address: resource_address,
            ..Default::default()
        })
        .await
        .expect_err("upload part is expected to fail");

    assert_eq!(status.code(), Code::PermissionDenied);
}

#[rstest]
#[test_log::test(tokio::test)]
#[ignore = "not yet fixed"]
async fn test_upload_part_returns_unauthorized_on_missing_header() {
    todo!();
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_complete_multipart_upload_returns_permission_denied_on_forbidden_action(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::fileobject::types::v1beta::CompleteMultipartUploadRequest;

    let service_set = storage_fixture.service_set(backend);
    let client = service_set.fileobject_v1beta_client(interface).await;
    let path = get_random_path();

    // Attempt to complete a multipart upload. The operation must fail.
    let resource_address = service_set.az_url_builder().path(path).to_string();
    let status = client
        .complete_multipart_upload(CompleteMultipartUploadRequest {
            destination_resource_address: resource_address,
            ..Default::default()
        })
        .await
        .expect_err("complete multipart upload is expected to fail");

    assert_eq!(status.code(), Code::PermissionDenied);
}

#[rstest]
#[test_log::test(tokio::test)]
#[ignore = "not yet fixed"]
async fn test_complete_multipart_upload_returns_unauthorized_on_missing_header() {
    todo!();
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_abort_multipart_upload(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::fileobject::types::v1beta::AbortMultipartUploadRequest;

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.fileobject_v1beta_client(interface).await;
    let path = get_random_path();

    // Allow write operations against the resource address.
    let resource_address = service_set.az_url_builder().path(path).to_string();
    rule_set.allow_object_write(&resource_address);

    // Abort an upload. The operation must succeed.
    client
        .abort_multipart_upload(AbortMultipartUploadRequest {
            destination_resource_address: resource_address,
            ..Default::default()
        })
        .await
        .expect("multipart upload abort failed");
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_abort_multipart_upload_returns_permission_denied_on_forbidden_action(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::fileobject::types::v1beta::AbortMultipartUploadRequest;

    let service_set = storage_fixture.service_set(backend);
    let client = service_set.fileobject_v1beta_client(interface).await;
    let path = get_random_path();

    // Abort an upload. The operation must fail.
    let resource_address = service_set.az_url_builder().path(path).to_string();
    let status = client
        .abort_multipart_upload(AbortMultipartUploadRequest {
            destination_resource_address: resource_address,
            ..Default::default()
        })
        .await
        .expect_err("multipart upload abort is expected to fail");

    assert_eq!(status.code(), Code::PermissionDenied);
}

#[rstest]
#[test_log::test(tokio::test)]
#[ignore = "not yet fixed"]
async fn test_abort_multipart_upload_returns_unauthorized_on_missing_header() {
    todo!();
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_list(storage_fixture: &StorageFixture, #[case] backend: PermissionBackend, #[case] interface: Interface) {
    use crate::common::clients::filefolder::types::v1beta::{FolderAddress, ListRequest, ListResponse};

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.filefolder_v1beta_client(interface).await;
    let folder_path = format!("{}/", get_random_path());
    let subfolder_1_path = format!("{}folder-1/", folder_path);
    let subfolder_2_path = format!("{}folder-2/", folder_path);
    let object_1_path = format!("{}object-1", folder_path);
    let object_2_path = format!("{}object-2", folder_path);
    let object_3_path = format!("{}object-3", subfolder_1_path);
    let object_4_path = format!("{}object-4", subfolder_2_path);

    // Create objects under the specified paths.
    service_set
        .create_random_objects(
            [&object_1_path, &object_2_path, &object_3_path, &object_4_path]
                .into_iter()
                .map(|path| service_set.az_url_builder().path(path)),
        )
        .await;

    // Allow reads for the root folder.
    let folder_address = service_set.az_url_builder().path(folder_path).to_string();
    rule_set.allow_folder_read(&folder_address);

    // List the folder. The operation must succeed.
    let response = client
        .list_collect(ListRequest {
            folder: Some(FolderAddress { uri: folder_address }),
        })
        .await
        .expect("list failed");

    assert_eq!(
        response,
        ListResponse {
            sub_resource_addresses: [object_1_path, object_2_path]
                .into_iter()
                .map(|path| service_set.az_url_builder().path(path).to_string())
                .collect(),
            subfolder_addresses: [subfolder_1_path, subfolder_2_path]
                .into_iter()
                .map(|path| FolderAddress {
                    uri: service_set.az_url_builder().path(path).to_string(),
                })
                .collect(),
        }
    );
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_list_filters_entities_without_access(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::filefolder::types::v1beta::{FolderAddress, ListRequest, ListResponse};

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.filefolder_v1beta_client(interface).await;
    let folder_path = format!("{}/", get_random_path());
    let subfolder_1_path = format!("{}folder-1/", folder_path);
    let subfolder_2_path = format!("{}folder-2/", folder_path);
    let object_1_path = format!("{}object-1", folder_path);
    let object_2_path = format!("{}object-2", folder_path);
    let object_3_path = format!("{}object-3", subfolder_1_path);
    let object_4_path = format!("{}object-4", subfolder_2_path);

    // Create objects under the specified paths.
    service_set
        .create_random_objects(
            [&object_1_path, &object_2_path, &object_3_path, &object_4_path]
                .into_iter()
                .map(|path| service_set.az_url_builder().path(path)),
        )
        .await;

    // Configure access for entities.
    let folder_address = service_set.az_url_builder().path(folder_path).to_string();
    let subfolder_1_address = service_set.az_url_builder().path(subfolder_1_path).to_string();
    let subfolder_2_address = service_set.az_url_builder().path(subfolder_2_path).to_string();
    let object_1_address = service_set.az_url_builder().path(object_1_path).to_string();
    let object_2_address = service_set.az_url_builder().path(object_2_path).to_string();
    rule_set.allow_folder_read(&folder_address);
    rule_set.forbid_folder_read(subfolder_1_address);
    rule_set.forbid_object_read(object_1_address);

    // List the folder. The operation must succeed.
    let response = client
        .list_collect(ListRequest {
            folder: Some(FolderAddress { uri: folder_address }),
        })
        .await
        .expect("list failed");

    assert_eq!(
        response,
        ListResponse {
            sub_resource_addresses: vec![object_2_address],
            subfolder_addresses: vec![FolderAddress {
                uri: subfolder_2_address
            }],
        }
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_list_normalizes_folder_uri_if_needed(
    storage_fixture: &StorageFixture,
    #[values("", "/")] maybe_trailing_slash: &str,
) {
    use crate::common::clients::filefolder::types::v1beta::{FolderAddress, ListRequest, ListResponse};

    let service_set = storage_fixture.service_set(PermissionBackend::Grpc);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.filefolder_v1beta_client(Interface::Grpc).await;
    let folder_path = get_random_path();
    let object_path = format!("{}/object", folder_path);

    // Create the object.
    let object_address = service_set.az_url_builder().path(object_path).to_string();
    service_set.create_random_object(&object_address).await;

    // Permit folder read operations.
    rule_set.allow_folder_read(service_set.az_url_builder().path(format!("{}/", folder_path)));

    // List the folder. The operation must succeed.
    let folder_address = service_set
        .az_url_builder()
        .path(format!("{}{}", folder_path, maybe_trailing_slash))
        .to_string();
    let response = client
        .list_collect(ListRequest {
            folder: Some(FolderAddress { uri: folder_address }),
        })
        .await
        .expect("list failed");

    assert_eq!(
        response,
        ListResponse {
            sub_resource_addresses: vec![object_address],
            ..Default::default()
        }
    );
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_list_returns_permission_denied_on_forbidden_action(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::filefolder::types::v1beta::{FolderAddress, ListRequest};

    let service_set = storage_fixture.service_set(backend);
    let client = service_set.filefolder_v1beta_client(interface).await;
    let folder_path = get_random_path();

    // List the folder. The operation must fail.
    let folder_address = service_set.az_url_builder().path(folder_path).to_string();
    let status = client
        .list_collect(ListRequest {
            folder: Some(FolderAddress { uri: folder_address }),
        })
        .await
        .expect_err("list is expected to fail");

    assert_eq!(status.code(), Code::PermissionDenied);
}

#[rstest]
#[test_log::test(tokio::test)]
#[ignore = "not yet fixed"]
async fn test_list_returns_unauthorized_on_missing_header() {
    todo!();
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_list_stat(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::filefolder::types::v1beta::{FolderAddress, ListStatRequest};

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.filefolder_v1beta_client(interface).await;
    let folder_path = format!("{}/", get_random_path());
    let subfolder_1_path = format!("{}folder-1/", folder_path);
    let subfolder_2_path = format!("{}folder-2/", folder_path);
    let object_1_path = format!("{}object-1", folder_path);
    let object_2_path = format!("{}object-2", folder_path);
    let object_3_path = format!("{}object-3", subfolder_1_path);
    let object_4_path = format!("{}object-4", subfolder_2_path);

    // Create objects under the specified paths.
    let object_1_address = service_set.az_url_builder().path(object_1_path).to_string();
    let object_2_address = service_set.az_url_builder().path(object_2_path).to_string();
    let object_3_address = service_set.az_url_builder().path(object_3_path).to_string();
    let object_4_address = service_set.az_url_builder().path(object_4_path).to_string();
    service_set
        .create_random_objects([
            &object_1_address,
            &object_2_address,
            &object_3_address,
            &object_4_address,
        ])
        .await;

    // Allow reads for the root folder.
    let folder_address = service_set.az_url_builder().path(folder_path).to_string();
    rule_set.allow_folder_read(&folder_address);

    // ListStat the folder. The operation must succeed.
    let response = client
        .list_stat_collect(ListStatRequest {
            folder: Some(FolderAddress { uri: folder_address }),
        })
        .await
        .expect("liststat failed");

    let expected_subfolder_addresses: Vec<_> = [subfolder_1_path, subfolder_2_path]
        .into_iter()
        .map(|path| FolderAddress {
            uri: service_set.az_url_builder().path(path).to_string(),
        })
        .collect();
    assert_eq!(response.subfolder_addresses, expected_subfolder_addresses);

    let expected_object_identities = service_set
        .get_resource_identities([object_1_address, object_2_address])
        .await;
    let object_identities: Vec<_> = response
        .entries
        .into_iter()
        .map(|e| {
            e.resource_info
                .expect("resource info is missing from the response")
                .unwrap_identity()
        })
        .collect();
    assert_eq!(object_identities, expected_object_identities);
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_list_stat_filters_entities_without_access(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::filefolder::types::v1beta::{FolderAddress, ListStatRequest};

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.filefolder_v1beta_client(interface).await;
    let folder_path = format!("{}/", get_random_path());
    let subfolder_1_path = format!("{}folder-1/", folder_path);
    let subfolder_2_path = format!("{}folder-2/", folder_path);
    let object_1_path = format!("{}object-1", folder_path);
    let object_2_path = format!("{}object-2", folder_path);
    let object_3_path = format!("{}object-3", subfolder_1_path);
    let object_4_path = format!("{}object-4", subfolder_2_path);

    // Create objects under the specified paths.
    let object_1_address = service_set.az_url_builder().path(object_1_path).to_string();
    let object_2_address = service_set.az_url_builder().path(object_2_path).to_string();
    let object_3_address = service_set.az_url_builder().path(object_3_path).to_string();
    let object_4_address = service_set.az_url_builder().path(object_4_path).to_string();
    service_set
        .create_random_objects([
            &object_1_address,
            &object_2_address,
            &object_3_address,
            &object_4_address,
        ])
        .await;

    // Configure access for the paths.
    let folder_address = service_set.az_url_builder().path(folder_path).to_string();
    let subfolder_1_address = service_set.az_url_builder().path(subfolder_1_path).to_string();
    let subfolder_2_address = service_set.az_url_builder().path(subfolder_2_path).to_string();
    rule_set.allow_folder_read(&folder_address);
    rule_set.forbid_folder_read(subfolder_1_address);
    rule_set.forbid_object_read(object_1_address);

    // ListStat the folder. The operation must succeed.
    let response = client
        .list_stat_collect(ListStatRequest {
            folder: Some(FolderAddress { uri: folder_address }),
        })
        .await
        .expect("liststat failed");

    let expected_subfolder_addresses = vec![FolderAddress {
        uri: subfolder_2_address,
    }];
    assert_eq!(response.subfolder_addresses, expected_subfolder_addresses);

    let expected_object_identities = service_set.get_resource_identities([object_2_address]).await;
    let object_identities: Vec<_> = response
        .entries
        .into_iter()
        .map(|e| {
            e.resource_info
                .expect("resource info is missing from the response")
                .unwrap_identity()
        })
        .collect();
    assert_eq!(object_identities, expected_object_identities);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_list_stat_normalizes_folder_uri_if_needed(
    storage_fixture: &StorageFixture,
    #[values("", "/")] maybe_trailing_slash: &str,
) {
    use crate::common::clients::filefolder::types::v1beta::{FolderAddress, ListStatRequest};

    let service_set = storage_fixture.service_set(PermissionBackend::Grpc);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.filefolder_v1beta_client(Interface::Grpc).await;
    let folder_path = get_random_path();
    let object_path = format!("{}/object", folder_path);

    // Create the object.
    let object_address = service_set.az_url_builder().path(object_path).to_string();
    service_set.create_random_object(&object_address).await;

    // Configure folder access.
    rule_set.allow_folder_read(service_set.az_url_builder().path(format!("{}/", folder_path)));

    // ListStat the folder. The operation must succeed.
    let folder_address = service_set
        .az_url_builder()
        .path(format!("{}{}", folder_path, maybe_trailing_slash))
        .to_string();
    let response = client
        .list_stat_collect(ListStatRequest {
            folder: Some(FolderAddress { uri: folder_address }),
        })
        .await
        .expect("liststat failed");

    let expected_object_identities = service_set.get_resource_identities([object_address]).await;
    let object_identities: Vec<_> = response
        .entries
        .into_iter()
        .map(|e| {
            e.resource_info
                .expect("resource info is missing from the response")
                .unwrap_identity()
        })
        .collect();
    assert_eq!(object_identities, expected_object_identities);
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_list_stat_returns_permission_denied_on_forbidden_action(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::filefolder::types::v1beta::{FolderAddress, ListStatRequest};

    let service_set = storage_fixture.service_set(backend);
    let client = service_set.filefolder_v1beta_client(interface).await;
    let folder_path = get_random_path();

    // ListStat the folder. The operation must fail.
    let folder_address = service_set.az_url_builder().path(folder_path).to_string();
    let status = client
        .list_stat_collect(ListStatRequest {
            folder: Some(FolderAddress { uri: folder_address }),
        })
        .await
        .expect_err("liststat is expected to fail");

    assert_eq!(status.code(), Code::PermissionDenied);
}

#[rstest]
#[test_log::test(tokio::test)]
#[ignore = "not yet fixed"]
async fn test_list_stat_returns_unauthorized_on_missing_header() {
    todo!();
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_enumerate_versions(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::versioning::types::v1beta::{EnumerateVersionsRequest, VersionsOrder};

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.versioning_v1beta_client(interface).await;
    let path = get_random_path();

    // Create multiple versions of the same object.
    let resource_address = service_set.az_url_builder().path(path).to_string();
    service_set
        .create_random_objects([&resource_address, &resource_address, &resource_address])
        .await;

    // Enable object reads for the address.
    rule_set.allow_object_read(&resource_address);

    // Enumerate versions of the object. The operation must succeed.
    let response = client
        .enumerate_versions_collect(EnumerateVersionsRequest { resource_address })
        .await
        .expect("versions enumerate failed");

    assert_eq!(response.items.len(), 3);
    assert_eq!(response.versions_order, VersionsOrder::OldestFirst as i32);
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_enumerate_versions_returns_permission_denied_on_forbidden_action(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::versioning::types::v1beta::EnumerateVersionsRequest;

    let service_set = storage_fixture.service_set(backend);
    let client = service_set.versioning_v1beta_client(interface).await;
    let path = get_random_path();

    // Enumerate versions of the object. The operation must fail.
    let resource_address = service_set.az_url_builder().path(path).to_string();
    let status = client
        .enumerate_versions_collect(EnumerateVersionsRequest { resource_address })
        .await
        .expect_err("versions enumerate is expected to fail");

    assert_eq!(status.code(), Code::PermissionDenied);
}

#[rstest]
#[test_log::test(tokio::test)]
#[ignore = "not yet fixed"]
async fn test_enumerate_versions_returns_unauthorized_on_missing_header() {
    todo!();
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_list_services(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::capabilities::types::v1beta::{ListServicesResponse, ServiceEntry};

    let service_set = storage_fixture.service_set(backend);
    let client = service_set.capabilities_v1beta_client(interface).await;

    // List services. The operation must succeed.
    let response = client
        .list_services_alphabetically(Default::default())
        .await
        .expect("services list failed");

    assert_eq!(
        response,
        ListServicesResponse {
            services: vec![
                ServiceEntry {
                    service_name: "capabilities".into(),
                    service_versions: vec!["v1alpha".into(), "v1beta".into()],
                },
                ServiceEntry {
                    service_name: "filefolder".into(),
                    service_versions: vec!["v1alpha".into(), "v1beta".into()],
                },
                ServiceEntry {
                    service_name: "fileobject".into(),
                    service_versions: vec!["v1alpha".into(), "v1beta".into()],
                },
                ServiceEntry {
                    service_name: "metadata".into(),
                    service_versions: vec!["v1alpha".into()],
                },
                ServiceEntry {
                    service_name: "versioning".into(),
                    service_versions: vec!["v1alpha".into(), "v1beta".into()],
                }
            ],
        }
    );
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_list_top_level_addresses(#[case] backend: PermissionBackend, #[case] interface: Interface) {
    use crate::common::clients::capabilities::types::v1beta::TopLevelAddressEntry;

    let storage_fixture = StorageFixture::new();
    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.capabilities_v1beta_client(interface).await;

    // Allow folder reads for the storage container.
    let az_env = get_env().unwrap_azure();
    rule_set.allow_folder_read(az_env.storage_account_url());

    // List top level addresses. The operation must return the root container url.
    let response = client
        .list_top_level_addresses(Default::default())
        .await
        .expect("top level address list failed");

    assert!(response.items.contains(&TopLevelAddressEntry {
        top_level_address: format!(
            "https://{}.blob.core.windows.net/{}",
            az_env.storage_account, az_env.storage_container,
        ),
    }));
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_list_top_level_addresses_filters_out_inacessible_roots(
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::capabilities::types::v1beta::TopLevelAddressEntry;

    let storage_fixture = StorageFixture::new();
    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.capabilities_v1beta_client(interface).await;

    // Allow folder reads for the storage account, but not the storage container.
    let az_env = get_env().unwrap_azure();
    rule_set.allow_folder_read(az_env.storage_account_url());
    rule_set.forbid_folder_read(az_env.storage_container_url());

    // List top level addresses. The operation must not return the top container url.
    let response = client
        .list_top_level_addresses(Default::default())
        .await
        .expect("top level address list failed");

    assert!(!response.items.contains(&TopLevelAddressEntry {
        top_level_address: format!(
            "https://{}.blob.core.windows.net/{}",
            az_env.storage_account, az_env.storage_container,
        ),
    }));
}

#[rstest]
#[test_log::test(tokio::test)]
#[ignore = "not yet fixed"]
async fn test_list_top_level_addresses_returns_unauthorized_on_missing_header() {
    todo!();
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_stat_returns_unauthenticated_on_missing_authorization_header(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use common::clients::fileobject::types::v1beta::StatRequest;

    let service_set = storage_fixture.service_set(backend);
    let path = get_random_path();
    let client = service_set.fileobject_v1beta_client(interface).await;

    // Make a request with missing authorization metadata. The request must fail.
    let resource_address = service_set.az_url_builder().path(path).to_string();
    let status = client
        .stat(WithoutAuthorization(StatRequest { resource_address }))
        .await
        .expect_err("object stat is expected to fail");

    assert_eq!(status.code(), Code::Unauthenticated);
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_stat_returns_unauthenticated_on_rejected_principal(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use common::clients::fileobject::types::v1beta::StatRequest;

    let service_set = storage_fixture.service_set(backend);
    let path = get_random_path();
    let client = service_set.fileobject_v1beta_client(interface).await;

    // Make a request with invalid authorization metadata. The request must fail.
    let resource_address = service_set.az_url_builder().path(path).to_string();
    let status = client
        .stat(WithInvalidAuthorization(StatRequest { resource_address }))
        .await
        .expect_err("object stat is expected to fail");

    assert_eq!(status.code(), Code::Unauthenticated);
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_excluded_endpoint_encoded_uri_missing_blob_returns_not_found(
    bypassed_storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use common::clients::fileobject::types::v1beta::StatRequest;

    let service_set = bypassed_storage_fixture.service_set(backend);
    let client = service_set.fileobject_v1beta_client(interface).await;
    let azure = get_env().unwrap_azure();
    let path = get_random_path();
    let resource_address = azure.resource_url(&path);
    let encoded_path = format!("{}/{}", azure.storage_container, path);
    let encoded_address = format!(
        "HTTPS://{}.BLOB.CORE.WINDOWS.NET/{}",
        azure.storage_account.to_ascii_uppercase(),
        percent_encoding::utf8_percent_encode(&encoded_path, percent_encoding::NON_ALPHANUMERIC),
    );
    let rule_set = service_set.permission_context.new_rule_set();
    rule_set.forbid_object_read(&resource_address);
    rule_set.forbid_object_read(&encoded_address);

    let status = client
        .stat(StatRequest {
            resource_address: encoded_address,
        })
        .await
        .unwrap_err();

    assert_eq!(status.code(), Code::NotFound);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_bypassed_endpoint_stat_read_write_succeed_despite_deny_rules(bypassed_storage_fixture: &StorageFixture) {
    use common::clients::fileobject::types::v1beta::StatRequest;

    let service_set = bypassed_storage_fixture.service_set(PermissionBackend::Grpc);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.fileobject_v1beta_client(Interface::Grpc).await;

    // Rules that would reject these operations on an enforced endpoint.
    let resource_address = service_set.az_url_builder().path(get_random_path()).to_string();
    rule_set.forbid_object_write(&resource_address);
    rule_set.forbid_object_read(&resource_address);

    let (resource_info, content) = client
        .write_random_object(&resource_address, 32)
        .await
        .expect("object write failed");
    let resource_identity = resource_info.unwrap_identity();

    client
        .stat(StatRequest {
            resource_address: resource_address.clone(),
        })
        .await
        .expect("object stat failed");

    let (_, read_content) = client.read_object(resource_identity).await.expect("object read failed");
    assert_eq!(read_content, content);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_bypassed_endpoint_list_succeeds_despite_deny_rules(bypassed_storage_fixture: &StorageFixture) {
    use crate::common::clients::filefolder::types::v1beta::{FolderAddress, ListRequest};

    let service_set = bypassed_storage_fixture.service_set(PermissionBackend::Grpc);
    let rule_set = service_set.permission_context.new_rule_set();
    let folder_path = format!("{}/", get_random_path());
    let expected_addresses: Vec<_> = ["object-1", "object-2"]
        .into_iter()
        .map(|name| {
            service_set
                .az_url_builder()
                .path(format!("{folder_path}{name}"))
                .to_string()
        })
        .collect();

    let object_client = service_set.fileobject_v1beta_client(Interface::Grpc).await;
    for address in expected_addresses.iter() {
        object_client
            .write_random_object(address, 32)
            .await
            .expect("object write failed");
    }

    // Rules that would reject the listing and hide its entries on an enforced endpoint.
    let folder_address = service_set.az_url_builder().path(&folder_path).to_string();
    rule_set.forbid_folder_read(&folder_address);
    for address in expected_addresses.iter() {
        rule_set.forbid_object_read(address);
    }

    let filefolder_client = service_set.filefolder_v1beta_client(Interface::Grpc).await;
    let list_response = filefolder_client
        .list_collect(ListRequest {
            folder: Some(FolderAddress { uri: folder_address }),
        })
        .await
        .expect("list failed");

    assert_eq!(list_response.sub_resource_addresses, expected_addresses);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_bypassed_endpoint_enumerate_succeeds_despite_deny_rules(bypassed_storage_fixture: &StorageFixture) {
    use crate::common::clients::fileobject::types::v1beta::EnumerateRequest;

    let service_set = bypassed_storage_fixture.service_set(PermissionBackend::Grpc);
    let rule_set = service_set.permission_context.new_rule_set();
    let folder_path = format!("{}/", get_random_path());
    let expected_addresses: Vec<_> = ["object-1", "object-2"]
        .into_iter()
        .map(|name| {
            service_set
                .az_url_builder()
                .path(format!("{folder_path}{name}"))
                .to_string()
        })
        .collect();

    let object_client = service_set.fileobject_v1beta_client(Interface::Grpc).await;
    for address in expected_addresses.iter() {
        object_client
            .write_random_object(address, 32)
            .await
            .expect("object write failed");
    }

    // Rules that would reject the enumeration and hide its entries on an enforced endpoint.
    let folder_address = service_set.az_url_builder().path(&folder_path).to_string();
    rule_set.forbid_folder_read(&folder_address);
    for address in expected_addresses.iter() {
        rule_set.forbid_object_read(address);
    }

    let enumerated: Vec<_> = object_client
        .enumerate(EnumerateRequest {
            resource_address: folder_address,
        })
        .try_collect()
        .await
        .expect("enumerate failed");

    let addresses: Vec<_> = enumerated
        .into_iter()
        .flat_map(|response| response.items)
        .map(|item| item.resource_address)
        .collect();
    assert_eq!(addresses, expected_addresses);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_bypassed_endpoint_lists_top_level_address_for_principal_without_acls(
    bypassed_storage_fixture: &StorageFixture,
) {
    use crate::common::clients::capabilities::types::v1beta::TopLevelAddressEntry;

    let service_set = bypassed_storage_fixture.service_set(PermissionBackend::Grpc);
    let client = service_set.capabilities_v1beta_client(Interface::Grpc).await;
    let az_env = get_env().unwrap_azure();

    // No ACLs are configured; the bypassed endpoint's top-level address is listed anyway.
    let response = client
        .list_top_level_addresses(Default::default())
        .await
        .expect("top level address list failed");

    assert!(response.items.contains(&TopLevelAddressEntry {
        top_level_address: format!(
            "https://{}.blob.core.windows.net/{}",
            az_env.storage_account, az_env.storage_container,
        ),
    }));
}
