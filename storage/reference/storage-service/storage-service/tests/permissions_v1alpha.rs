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

use crate::common::clients::metadata::types::v1alpha::ResourceLocationExt;
use crate::common::clients::utils::{WithInvalidAuthorization, WithoutAuthorization};
use crate::common::clients::{
    fileobject::types::v1alpha::{ResourceInfo, UnwrapIdentityExt, UploadPreference},
    metadata::types::v1alpha::{
        batch_error_entry, delete_metadata_batch_response_entry, get_metadata_batch_response_entry, BatchErrorEntry,
        DeleteMetadataBatchRequestEntry, DeleteMetadataBatchResponseEntry, GetMetadataBatchRequestEntry,
        GetMetadataBatchResponseEntry, QueryMetadataResponseEntry, ResourceLocation, UpdateMetadataBatchRequestEntry,
        UserMetadataValue,
    },
};
use crate::common::env::get_env;
use crate::common::random::{get_random_alphanumeric_string, get_random_bytes, get_random_path};
use crate::common::service::Service as StorageService;
use crate::common::url::get_stat_query_pairs;

use assert_matches::assert_matches;
use bytes::Bytes;
use common::types::Interface;
use futures::TryStreamExt;
use rstest::{fixture, rstest};
use storage_service_protos::google::protobuf::Value;
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

        let client = self.fileobject_v1alpha_client(Interface::Grpc).await;
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
        let client = self.fileobject_v1alpha_client(Interface::Grpc).await;
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
        use crate::common::clients::fileobject::types::v1alpha::StatRequest;

        let client = self.fileobject_v1alpha_client(Interface::Grpc).await;
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

    async fn create_random_metadata(&self, uri: impl Into<String>, key: impl Into<String>) -> (Value, String) {
        use common::clients::metadata::types::v1alpha::UpdateMetadataRequest;
        use storage_service_protos::google::protobuf::value::Kind;

        let uri = uri.into();

        // We don't know want to know whether the specified uri belongs to an object or a folder. Hence, we allow
        // writes for both.
        let rule_set = self.permission_context.new_rule_set();
        rule_set.allow_object_write(&uri);
        rule_set.allow_folder_write(&uri);

        let value = Value {
            kind: Some(Kind::StringValue(get_random_alphanumeric_string(32))),
        };
        let client = self.metadata_v1alpha_client(Interface::Grpc).await;

        let response = client
            .update_metadata(UpdateMetadataRequest {
                uri,
                user_metadata_key: key.into(),
                user_metadata: Some(value.clone()),
                ..Default::default()
            })
            .await
            .expect("metadata update failed");

        (value, response.etag)
    }

    async fn create_random_metadata_with_ns(
        &self,
        resource_location: impl Into<ResourceLocation>,
        key: impl Into<String>,
        namespace: Option<impl Into<String>>,
    ) -> (Value, String) {
        use common::clients::metadata::types::v1alpha::*;
        use storage_service_protos::google::protobuf::value::Kind;

        let resource_location = resource_location.into();

        let rule_set = self.permission_context.new_rule_set();
        rule_set.allow_object_write(resource_location.uri());
        rule_set.allow_folder_write(resource_location.uri());

        let value = Value {
            kind: Some(Kind::StringValue(get_random_alphanumeric_string(32))),
        };
        let client = self.metadata_v1alpha_client(Interface::Grpc).await;

        let response = client
            .update_metadata_batch_sorted(UpdateMetadataBatchRequest {
                requests: vec![UpdateMetadataBatchRequestEntry {
                    resource_location: Some(resource_location),
                    namespace: namespace.map(Into::into),
                    user_metadata_key: key.into(),
                    user_metadata: Some(value.clone()),
                    ..Default::default()
                }],
            })
            .await
            .expect("metadata update failed");

        (value, response.results[0].as_success().etag.clone())
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
    fn new(include_acl_in_all_keys: bool) -> Self {
        Self::with_az_permissions(include_acl_in_all_keys, true)
    }

    /// Build a fixture whose Azure endpoint either enforces the permission layer
    /// or opts out of it (`enable_az_permissions = false` for a public bucket).
    fn with_az_permissions(include_acl_in_all_keys: bool, enable_az_permissions: bool) -> Self {
        let rest_permission_context = Context::new();
        let rest_permission_service = RestPermissionService::new(rest_permission_context.clone());

        let grpc_permission_context = Context::new();
        let grpc_permission_service = GrpcPermissionService::new(grpc_permission_context.clone());

        Self {
            rest_storage_service: StorageService::builder()
                .enable_az_with_permissions(true, enable_az_permissions)
                .enable_postgres_metadata()
                .enable_permissions("rest", rest_permission_service.endpoint_url(), include_acl_in_all_keys)
                .start(),
            grpc_storage_service: StorageService::builder()
                .enable_az_with_permissions(true, enable_az_permissions)
                .enable_postgres_metadata()
                .enable_permissions("grpc", grpc_permission_service.endpoint_url(), include_acl_in_all_keys)
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
    StorageFixture::new(false)
}

#[fixture]
#[once]
fn storage_fixture_with_injected_metadata_acls() -> StorageFixture {
    StorageFixture::new(true)
}

// Fixtures whose Azure endpoint opted out of permission enforcement. Requests
// against it must succeed even when the configured rules deny access, because
// the permission service is never consulted.
#[fixture]
#[once]
fn bypassed_storage_fixture() -> StorageFixture {
    StorageFixture::with_az_permissions(false, false)
}

#[fixture]
#[once]
fn bypassed_storage_fixture_with_injected_metadata_acls() -> StorageFixture {
    StorageFixture::with_az_permissions(true, false)
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_stat(storage_fixture: &StorageFixture, #[case] backend: PermissionBackend, #[case] interface: Interface) {
    use common::clients::fileobject::types::v1alpha::StatRequest;

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let path = get_random_path();
    let client = service_set.fileobject_v1alpha_client(interface).await;

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
    use common::clients::fileobject::types::v1alpha::StatRequest;

    let service_set = storage_fixture.service_set(PermissionBackend::Grpc);
    let rule_set = service_set.permission_context.new_rule_set();
    let folder_path = get_random_path();
    let client = service_set.fileobject_v1alpha_client(Interface::Rest).await;

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
    use common::clients::fileobject::types::v1alpha::StatRequest;

    let service_set = storage_fixture.service_set(backend);
    let path = get_random_path();
    let client = service_set.fileobject_v1alpha_client(interface).await;

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
    let client = service_set.fileobject_v1alpha_client(interface).await;

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
    let client = service_set.fileobject_v1alpha_client(interface).await;

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
    let client = service_set.fileobject_v1alpha_client(interface).await;

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
    let client = service_set.fileobject_v1alpha_client(interface).await;

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
    use crate::common::clients::fileobject::types::v1alpha::EnumerateRequest;

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.fileobject_v1alpha_client(interface).await;
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
    use crate::common::clients::fileobject::types::v1alpha::EnumerateRequest;

    let service_set = storage_fixture.service_set(PermissionBackend::Grpc);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.fileobject_v1alpha_client(Interface::Grpc).await;
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
    use crate::common::clients::fileobject::types::v1alpha::EnumerateRequest;

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.fileobject_v1alpha_client(interface).await;
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
    use crate::common::clients::fileobject::types::v1alpha::EnumerateRequest;

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.fileobject_v1alpha_client(interface).await;
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
    use crate::common::clients::fileobject::types::v1alpha::{
        FetchWriteTypeInfoRequest, FetchWriteTypeInfoResponse, UploadPreference, WriteTypeForSizeInterval,
    };

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.fileobject_v1alpha_client(interface).await;
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
    use crate::common::clients::fileobject::types::v1alpha::FetchWriteTypeInfoRequest;

    let service_set = storage_fixture.service_set(backend);
    let client = service_set.fileobject_v1alpha_client(interface).await;
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
    use crate::common::clients::fileobject::types::v1alpha::WriteParameters;

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.fileobject_v1alpha_client(interface).await;
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
    use crate::common::clients::fileobject::types::v1alpha::{UploadPreference, WriteParameters};

    let service_set = storage_fixture.service_set(backend);
    let client = service_set.fileobject_v1alpha_client(interface).await;
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
    use crate::common::clients::fileobject::types::v1alpha::CompleteRedirectUploadRequest;

    let service_set = storage_fixture.service_set(backend);
    let client = service_set.fileobject_v1alpha_client(interface).await;
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
    use crate::common::clients::fileobject::types::v1alpha::UploadPartRequest;

    let service_set = storage_fixture.service_set(backend);
    let client = service_set.fileobject_v1alpha_client(interface).await;
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
    use crate::common::clients::fileobject::types::v1alpha::CompleteMultipartUploadRequest;

    let service_set = storage_fixture.service_set(backend);
    let client = service_set.fileobject_v1alpha_client(interface).await;
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
    use crate::common::clients::fileobject::types::v1alpha::AbortMultipartUploadRequest;

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.fileobject_v1alpha_client(interface).await;
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
    use crate::common::clients::fileobject::types::v1alpha::AbortMultipartUploadRequest;

    let service_set = storage_fixture.service_set(backend);
    let client = service_set.fileobject_v1alpha_client(interface).await;
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
async fn test_copy(storage_fixture: &StorageFixture, #[case] backend: PermissionBackend, #[case] interface: Interface) {
    use crate::common::clients::fileobject::types::v1alpha::{CopyRequest, ResourceIdentity};

    let service_set = storage_fixture.service_set(backend);
    let client = service_set.fileobject_v1alpha_client(interface).await;
    let rule_set = service_set.permission_context.new_rule_set();
    let source_path = get_random_path();
    let target_path = get_random_path();

    // Create the source object.
    let source_address = service_set.az_url_builder().path(source_path).to_string();
    let (resource_info, _) = service_set.create_random_object(&source_address).await;

    // Configure permissions for the source and the target.
    let target_address = service_set.az_url_builder().path(target_path).to_string();
    rule_set.allow_object_write(&target_address);
    rule_set.allow_object_read(&source_address);

    // Copy the source object to the target address. The operation must succeed.
    client
        .copy(CopyRequest {
            source_resource_identity: Some(ResourceIdentity {
                encoded_identity: resource_info.unwrap_identity(),
            }),
            destination_resource_address: target_address,
            ..Default::default()
        })
        .await
        .expect("object copy failed");
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc, true, false)]
#[case(PermissionBackend::Grpc, Interface::Grpc, false, true)]
#[case(PermissionBackend::Rest, Interface::Rest, true, false)]
#[case(PermissionBackend::Rest, Interface::Rest, false, true)]
#[test_log::test(tokio::test)]
async fn test_copy_returns_permission_denied_on_forbidden_access(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
    #[case] allow_source_read: bool,
    #[case] allow_target_write: bool,
) {
    use crate::common::clients::fileobject::types::v1alpha::{CopyRequest, ResourceIdentity};

    let service_set = storage_fixture.service_set(backend);
    let client = service_set.fileobject_v1alpha_client(interface).await;
    let rule_set = service_set.permission_context.new_rule_set();
    let source_path = get_random_path();
    let target_path = get_random_path();

    // Create the source object.
    let source_address = service_set.az_url_builder().path(source_path).to_string();
    let (resource_info, _) = service_set.create_random_object(&source_address).await;

    // Configure permissions for the source and the target.
    let target_address = service_set.az_url_builder().path(target_path).to_string();
    if allow_target_write {
        rule_set.allow_object_write(&target_address);
    }

    if allow_source_read {
        rule_set.allow_object_read(&source_address);
    }

    // Copy the source object to the target address. The operation must succeed.
    let status = client
        .copy(CopyRequest {
            source_resource_identity: Some(ResourceIdentity {
                encoded_identity: resource_info.unwrap_identity(),
            }),
            destination_resource_address: target_address,
            ..Default::default()
        })
        .await
        .expect_err("object copy is expected to fail");

    assert_eq!(status.code(), Code::PermissionDenied);
}

#[rstest]
#[test_log::test(tokio::test)]
#[ignore = "not yet fixed"]
async fn test_copy_returns_unauthorized_on_missing_header() {
    todo!();
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_move(storage_fixture: &StorageFixture, #[case] backend: PermissionBackend, #[case] interface: Interface) {
    use crate::common::clients::fileobject::types::v1alpha::MoveRequest;

    let service_set = storage_fixture.service_set(backend);
    let client = service_set.fileobject_v1alpha_client(interface).await;
    let rule_set = service_set.permission_context.new_rule_set();
    let source_path = get_random_path();
    let target_path = get_random_path();

    // Create the source object.
    let source_address = service_set.az_url_builder().path(source_path).to_string();
    service_set.create_random_object(&source_address).await;

    // Configure permissions for the source and the target.
    let target_address = service_set.az_url_builder().path(target_path).to_string();
    rule_set.allow_object_write(&target_address);
    rule_set.allow_object_write(&source_address);

    // Move the source object to the target address. The operation must succeed.
    client
        .move_object(MoveRequest {
            source_resource_address: source_address,
            destination_resource_address: target_address,
            ..Default::default()
        })
        .await
        .expect("object move failed");
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc, true, false)]
#[case(PermissionBackend::Grpc, Interface::Grpc, false, true)]
#[case(PermissionBackend::Rest, Interface::Rest, true, false)]
#[case(PermissionBackend::Rest, Interface::Rest, false, true)]
#[test_log::test(tokio::test)]
async fn test_move_returns_permission_denied_on_forbidden_access(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
    #[case] allow_source_write: bool,
    #[case] allow_target_write: bool,
) {
    use crate::common::clients::fileobject::types::v1alpha::MoveRequest;

    let service_set = storage_fixture.service_set(backend);
    let client = service_set.fileobject_v1alpha_client(interface).await;
    let rule_set = service_set.permission_context.new_rule_set();
    let source_path = get_random_path();
    let target_path = get_random_path();

    // Create the source object.
    let source_address = service_set.az_url_builder().path(source_path).to_string();
    service_set.create_random_object(&source_address).await;

    // Configure permissions for the source and the target.
    let target_address = service_set.az_url_builder().path(target_path).to_string();
    if allow_target_write {
        rule_set.allow_object_write(&target_address);
    }

    if allow_source_write {
        rule_set.allow_object_write(&source_address);
    }

    // Copy the source object to the target address. The operation must succeed.
    let status = client
        .move_object(MoveRequest {
            source_resource_address: source_address,
            destination_resource_address: target_address,
            ..Default::default()
        })
        .await
        .expect_err("object move is expected to fail");

    assert_eq!(status.code(), Code::PermissionDenied);
}

#[rstest]
#[test_log::test(tokio::test)]
#[ignore = "not yet fixed"]
async fn test_move_returns_unauthorized_on_missing_header() {
    todo!();
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_create_folder(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::filefolder::types::v1alpha::{CreateFolderRequest, FolderAddress};

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.filefolder_v1alpha_client(interface).await;
    let path = format!("{}/", get_random_path());

    // Configure permissions for the folder address.
    let folder_address = service_set.az_url_builder().path(path).to_string();
    rule_set.allow_folder_write(&folder_address);

    // Create a folder. The operation must succeed.
    client
        .create_folder(CreateFolderRequest {
            folder: Some(FolderAddress { uri: folder_address }),
        })
        .await
        .expect("folder create failed");
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_create_folder_normalizes_resource_uri_if_needed(
    storage_fixture: &StorageFixture,
    #[values("", "/")] maybe_trailing_slash: &str,
) {
    use crate::common::clients::filefolder::types::v1alpha::{CreateFolderRequest, FolderAddress};

    let service_set = storage_fixture.service_set(PermissionBackend::Grpc);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.filefolder_v1alpha_client(Interface::Grpc).await;
    let path = get_random_path();

    // Configure permissions for the folder address.
    rule_set.allow_folder_write(service_set.az_url_builder().path(format!("{}/", path)));

    // Create a folder. The operation must succeed.
    let folder_address = service_set
        .az_url_builder()
        .path(format!("{}{}", path, maybe_trailing_slash))
        .to_string();
    client
        .create_folder(CreateFolderRequest {
            folder: Some(FolderAddress { uri: folder_address }),
        })
        .await
        .expect("folder create failed");
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_create_folder_returns_permission_denied_on_forbidden_action(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::filefolder::types::v1alpha::{CreateFolderRequest, FolderAddress};

    let service_set = storage_fixture.service_set(backend);
    let client = service_set.filefolder_v1alpha_client(interface).await;
    let path = get_random_path();

    // Create a folder. The operation must fail.
    let folder_address = service_set.az_url_builder().path(path).to_string();
    let status = client
        .create_folder(CreateFolderRequest {
            folder: Some(FolderAddress { uri: folder_address }),
        })
        .await
        .expect_err("folder create is expected to fail");

    assert_eq!(status.code(), Code::PermissionDenied);
}

#[rstest]
#[test_log::test(tokio::test)]
#[ignore = "not yet fixed"]
async fn test_create_folder_returns_unauthorized_on_missing_header() {
    todo!();
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_delete_folder(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::filefolder::types::v1alpha::{CreateFolderRequest, DeleteFolderRequest, FolderAddress};

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.filefolder_v1alpha_client(interface).await;
    let path = format!("{}/", get_random_path());

    // Allow folder writes and create the folder.
    let folder_address = service_set.az_url_builder().path(path).to_string();
    rule_set.allow_folder_write(&folder_address);

    client
        .create_folder(CreateFolderRequest {
            folder: Some(FolderAddress {
                uri: folder_address.clone(),
            }),
        })
        .await
        .expect("folder create failed");

    // Delete the folder. The operation must succeed.
    client
        .delete_folder(DeleteFolderRequest {
            folder: Some(FolderAddress { uri: folder_address }),
        })
        .await
        .expect("folder delete failed");
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_delete_folder_normalizes_folder_uri_if_needed(
    storage_fixture: &StorageFixture,
    #[values("", "/")] maybe_trailing_slash: &str,
) {
    use crate::common::clients::filefolder::types::v1alpha::{CreateFolderRequest, DeleteFolderRequest, FolderAddress};

    let service_set = storage_fixture.service_set(PermissionBackend::Grpc);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.filefolder_v1alpha_client(Interface::Grpc).await;
    let path = get_random_path();

    // Allow folder writes and create the folder.
    let create_folder_address = service_set.az_url_builder().path(format!("{}/", path)).to_string();
    rule_set.allow_folder_write(&create_folder_address);

    client
        .create_folder(CreateFolderRequest {
            folder: Some(FolderAddress {
                uri: create_folder_address,
            }),
        })
        .await
        .expect("folder create failed");

    // Delete the folder. The operation must succeed.
    let delete_folder_address = service_set
        .az_url_builder()
        .path(format!("{}{}", path, maybe_trailing_slash))
        .to_string();
    client
        .delete_folder(DeleteFolderRequest {
            folder: Some(FolderAddress {
                uri: delete_folder_address,
            }),
        })
        .await
        .expect("folder delete failed");
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_delete_folder_returns_permission_denied_on_forbidden_action(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::filefolder::types::v1alpha::{DeleteFolderRequest, FolderAddress};

    let service_set = storage_fixture.service_set(backend);
    let client = service_set.filefolder_v1alpha_client(interface).await;
    let path = get_random_path();

    // Delete a folder. The operation must fail.
    let folder_address = service_set.az_url_builder().path(path).to_string();
    let status = client
        .delete_folder(DeleteFolderRequest {
            folder: Some(FolderAddress { uri: folder_address }),
        })
        .await
        .expect_err("folder delete is expected to fail");

    assert_eq!(status.code(), Code::PermissionDenied);
}

#[rstest]
#[test_log::test(tokio::test)]
#[ignore = "not yet fixed"]
async fn test_delete_folder_returns_unauthorized_on_missing_header() {
    todo!();
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_get_folder_mode(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::filefolder::types::v1alpha::{
        FolderAddress, FolderMode, GetFolderModeRequest, GetFolderModeResponse,
    };

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.filefolder_v1alpha_client(interface).await;
    let path = format!("{}/", get_random_path());

    // Allow folder reads.
    let folder_address = service_set.az_url_builder().path(path).to_string();
    rule_set.allow_folder_read(&folder_address);

    // Fetch folder mode. The operation must succeed.
    let response = client
        .get_folder_mode(GetFolderModeRequest {
            folder: Some(FolderAddress {
                uri: folder_address.clone(),
            }),
        })
        .await
        .expect("folder mode fetch failed");

    assert_eq!(
        response,
        GetFolderModeResponse {
            folder_mode: FolderMode::Hybrid as i32,
        },
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_get_folder_mode_normalizes_folder_uri_if_needed(
    storage_fixture: &StorageFixture,
    #[values("", "/")] maybe_trailing_slash: &str,
) {
    use crate::common::clients::filefolder::types::v1alpha::{
        FolderAddress, FolderMode, GetFolderModeRequest, GetFolderModeResponse,
    };

    let service_set = storage_fixture.service_set(PermissionBackend::Grpc);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.filefolder_v1alpha_client(Interface::Grpc).await;
    let path = get_random_path();

    // Allow folder reads.
    rule_set.allow_folder_read(service_set.az_url_builder().path(format!("{}/", path)));

    // Fetch folder mode. The operation must succeed.
    let folder_address = service_set
        .az_url_builder()
        .path(format!("{}{}", path, maybe_trailing_slash))
        .to_string();
    let response = client
        .get_folder_mode(GetFolderModeRequest {
            folder: Some(FolderAddress {
                uri: folder_address.clone(),
            }),
        })
        .await
        .expect("folder mode fetch failed");

    assert_eq!(
        response,
        GetFolderModeResponse {
            folder_mode: FolderMode::Hybrid as i32,
        },
    );
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_get_folder_mode_returns_permission_denied_on_forbidden_action(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::filefolder::types::v1alpha::{FolderAddress, GetFolderModeRequest};

    let service_set = storage_fixture.service_set(backend);
    let client = service_set.filefolder_v1alpha_client(interface).await;
    let path = format!("{}/", get_random_path());

    // Fetch folder mode. The operation must fail.
    let folder_address = service_set.az_url_builder().path(path).to_string();
    let status = client
        .get_folder_mode(GetFolderModeRequest {
            folder: Some(FolderAddress {
                uri: folder_address.clone(),
            }),
        })
        .await
        .expect_err("folder mode fetch is expected to fail");

    assert_eq!(status.code(), Code::PermissionDenied);
}

#[rstest]
#[test_log::test(tokio::test)]
#[ignore = "not yet fixed"]
async fn test_get_folder_mode_returns_unauthorized_on_missing_header() {
    todo!();
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_list(storage_fixture: &StorageFixture, #[case] backend: PermissionBackend, #[case] interface: Interface) {
    use crate::common::clients::filefolder::types::v1alpha::{FolderAddress, ListRequest, ListResponse};

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.filefolder_v1alpha_client(interface).await;
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
    use crate::common::clients::filefolder::types::v1alpha::{FolderAddress, ListRequest, ListResponse};

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.filefolder_v1alpha_client(interface).await;
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
    use crate::common::clients::filefolder::types::v1alpha::{FolderAddress, ListRequest, ListResponse};

    let service_set = storage_fixture.service_set(PermissionBackend::Grpc);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.filefolder_v1alpha_client(Interface::Grpc).await;
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
    use crate::common::clients::filefolder::types::v1alpha::{FolderAddress, ListRequest};

    let service_set = storage_fixture.service_set(backend);
    let client = service_set.filefolder_v1alpha_client(interface).await;
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
    use crate::common::clients::filefolder::types::v1alpha::{FolderAddress, ListStatRequest};

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.filefolder_v1alpha_client(interface).await;
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
            include_native_metadata: false,
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
    use crate::common::clients::filefolder::types::v1alpha::{FolderAddress, ListStatRequest};

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.filefolder_v1alpha_client(interface).await;
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
            include_native_metadata: false,
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
    use crate::common::clients::filefolder::types::v1alpha::{FolderAddress, ListStatRequest};

    let service_set = storage_fixture.service_set(PermissionBackend::Grpc);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.filefolder_v1alpha_client(Interface::Grpc).await;
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
            include_native_metadata: false,
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
    use crate::common::clients::filefolder::types::v1alpha::{FolderAddress, ListStatRequest};

    let service_set = storage_fixture.service_set(backend);
    let client = service_set.filefolder_v1alpha_client(interface).await;
    let folder_path = get_random_path();

    // ListStat the folder. The operation must fail.
    let folder_address = service_set.az_url_builder().path(folder_path).to_string();
    let status = client
        .list_stat_collect(ListStatRequest {
            folder: Some(FolderAddress { uri: folder_address }),
            include_native_metadata: false,
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
#[case(PermissionBackend::Grpc, Interface::Grpc, "", false)]
#[case(PermissionBackend::Grpc, Interface::Grpc, "/", true)]
#[case(PermissionBackend::Rest, Interface::Rest, "", false)]
#[case(PermissionBackend::Rest, Interface::Rest, "/", true)]
#[test_log::test(tokio::test)]
async fn test_get_metadata(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
    #[case] path_suffix: &str,
    #[case] is_folder: bool,
) {
    use crate::common::clients::metadata::types::v1alpha::{
        GetMetadataRequest, GetMetadataResponse, UserMetadataValue,
    };

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.metadata_v1alpha_client(interface).await;
    let path = format!("{}{}", get_random_path(), path_suffix);

    // Write some metadata.
    let resource_uri = service_set.az_url_builder().path(path).to_string();
    let (value_1, etag_1) = service_set.create_random_metadata(&resource_uri, "a").await;
    let (value_2, etag_2) = service_set.create_random_metadata(&resource_uri, "b").await;

    // Enable reads for the address.
    if is_folder {
        rule_set.allow_folder_read(&resource_uri);
    } else {
        rule_set.allow_object_read(&resource_uri);
    }

    // Read metadata for the specified address. The operation must succeed.
    let response = client
        .get_metadata(GetMetadataRequest {
            uri: resource_uri,
            ..Default::default()
        })
        .await
        .expect("metadata get failed");

    let expected_response = GetMetadataResponse {
        user_metadata: [
            (
                "a".into(),
                UserMetadataValue {
                    etag: etag_1,
                    value: Some(value_1),
                },
            ),
            (
                "b".into(),
                UserMetadataValue {
                    etag: etag_2,
                    value: Some(value_2),
                },
            ),
        ]
        .into(),
    };
    assert_eq!(response, expected_response);
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_get_metadata_with_filter(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::{
        GetMetadataRequest, GetMetadataResponse, UserMetadataValue,
    };

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.metadata_v1alpha_client(interface).await;
    let path = get_random_path();

    // Write some metadata.
    let resource_uri = service_set.az_url_builder().path(path).to_string();
    service_set.create_random_metadata(&resource_uri, "a").await;
    let (value, etag) = service_set.create_random_metadata(&resource_uri, "b").await;

    // Enable reads for the address.
    rule_set.allow_object_read(&resource_uri);

    // Read metadata for the specified address. The operation must succeed.
    let response = client
        .get_metadata(GetMetadataRequest {
            uri: resource_uri,
            user_metadata_keys: vec!["b".into()],
        })
        .await
        .expect("metadata get failed");

    let expected_response = GetMetadataResponse {
        user_metadata: [(
            "b".into(),
            UserMetadataValue {
                etag,
                value: Some(value),
            },
        )]
        .into(),
    };
    assert_eq!(response, expected_response);
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_get_metadata_returns_acls_when_asked(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::{
        GetMetadataRequest, GetMetadataResponse, UserMetadataValue,
    };
    use storage_service_protos::google::protobuf::{value::Kind, ListValue, Value};

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.metadata_v1alpha_client(interface).await;
    let path = get_random_path();

    // Write some metadata.
    let resource_uri = service_set.az_url_builder().path(path).to_string();
    service_set.create_random_metadata(&resource_uri, "a").await;
    let (value, etag) = service_set.create_random_metadata(&resource_uri, "b").await;

    // Enable reads for the address.
    rule_set.allow_object_read(&resource_uri);

    // Read metadata for the specified address. The operation must succeed.
    let response = client
        .get_metadata(GetMetadataRequest {
            uri: resource_uri,
            user_metadata_keys: vec!["b".into(), "acl".into()],
        })
        .await
        .expect("metadata get failed");

    let expected_response = GetMetadataResponse {
        user_metadata: [
            (
                "b".into(),
                UserMetadataValue {
                    etag,
                    value: Some(value),
                },
            ),
            (
                "acl".into(),
                UserMetadataValue {
                    value: Some(Value {
                        kind: Some(Kind::ListValue(ListValue {
                            values: vec![Value {
                                kind: Some(Kind::StringValue("read".into())),
                            }],
                        })),
                    }),
                    ..Default::default()
                },
            ),
        ]
        .into(),
    };
    assert_eq!(response, expected_response);
}

#[rstest]
#[case(vec![], vec!["a", "acl"])]
#[case(vec!["acl"], vec!["acl"])]
#[case(vec!["a"], vec!["a"])]
#[case(vec!["b"], vec![])]
#[test_log::test(tokio::test)]
async fn test_get_metadata_include_acl_in_all_keys(#[case] filter: Vec<&str>, #[case] expected_keys: Vec<&str>) {
    use crate::common::clients::metadata::types::v1alpha::{
        GetMetadataRequest, GetMetadataResponse, UserMetadataValue,
    };
    use crate::common::service::Service as StorageService;
    use storage_service_protos::google::protobuf::{value::Kind, ListValue};
    use storage_service_test::mocks::permissions::{grpc::Service as PermissionService, Context};

    let permission_context = Context::new();
    let permission_service = PermissionService::new(permission_context.clone());
    let storage_service = StorageService::builder()
        .enable_az(true)
        .enable_postgres_metadata()
        .enable_permissions("grpc", permission_service.endpoint_url(), true)
        .start();

    let rule_set = permission_context.new_rule_set();
    let client = storage_service.metadata_v1alpha_client(Interface::Grpc).await;
    let path = get_random_path();

    // Write some metadata.
    let resource_uri = storage_service.az_url_builder().path(path).to_string();
    let (value, etag) = {
        let rule_set = permission_context.new_rule_set();
        rule_set.allow_object_write(&resource_uri);

        let value = Value {
            kind: Some(Kind::StringValue("value-a".into())),
        };
        let etag = client
            .update_metadata(
                crate::common::clients::metadata::types::v1alpha::UpdateMetadataRequest {
                    uri: resource_uri.clone(),
                    user_metadata_key: "a".into(),
                    user_metadata: Some(value.clone()),
                    ..Default::default()
                },
            )
            .await
            .expect("metadata update failed")
            .etag;

        (value, etag)
    };

    // Enable reads for the address.
    rule_set.allow_object_read(&resource_uri);

    // Read metadata with empty keys list (get all keys).
    let response = client
        .get_metadata(GetMetadataRequest {
            uri: resource_uri.clone(),
            user_metadata_keys: filter.into_iter().map(Into::into).collect(),
        })
        .await
        .expect("metadata get failed");

    let expected_response = GetMetadataResponse {
        user_metadata: [
            (
                "a".to_string(),
                UserMetadataValue {
                    etag,
                    value: Some(value),
                },
            ),
            (
                "acl".to_string(),
                UserMetadataValue {
                    value: Some(Value {
                        kind: Some(Kind::ListValue(ListValue {
                            values: vec![Value {
                                kind: Some(Kind::StringValue("read".into())),
                            }],
                        })),
                    }),
                    ..Default::default()
                },
            ),
        ]
        .into_iter()
        .filter(|(key, _)| expected_keys.contains(&key.as_str()))
        .collect(),
    };
    assert_eq!(response, expected_response);
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_get_metadata_returns_permission_denied_on_forbidden_action(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::GetMetadataRequest;

    let service_set = storage_fixture.service_set(backend);
    let client = service_set.metadata_v1alpha_client(interface).await;
    let path = get_random_path();

    // Read metadata at the specified address. The operation must fail.
    let resource_uri = service_set.az_url_builder().path(path).to_string();
    let status = client
        .get_metadata(GetMetadataRequest {
            uri: resource_uri,
            ..Default::default()
        })
        .await
        .expect_err("get metadata is expected to fail");

    assert_eq!(status.code(), Code::PermissionDenied);
}

#[rstest]
#[test_log::test(tokio::test)]
#[ignore = "not yet fixed"]
async fn test_get_metadata_returns_unauthorized_on_missing_header() {
    todo!();
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc, "", false)]
#[case(PermissionBackend::Grpc, Interface::Grpc, "/", true)]
#[case(PermissionBackend::Rest, Interface::Rest, "", false)]
#[case(PermissionBackend::Rest, Interface::Rest, "/", true)]
#[test_log::test(tokio::test)]
async fn test_update_metadata(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
    #[case] path_suffix: &str,
    #[case] is_folder: bool,
) {
    use crate::common::clients::metadata::types::v1alpha::UpdateMetadataRequest;
    use storage_service_protos::google::protobuf::value::Kind;

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.metadata_v1alpha_client(interface).await;
    let path = format!("{}{}", get_random_path(), path_suffix);

    // Configure write permission for the address.
    let resource_uri = service_set.az_url_builder().path(path).to_string();
    if is_folder {
        rule_set.allow_folder_write(&resource_uri);
    } else {
        rule_set.allow_object_write(&resource_uri);
    }

    // Write metadata at the specified address. The operation must succeed.
    client
        .update_metadata(UpdateMetadataRequest {
            uri: resource_uri,
            user_metadata_key: "some-key".into(),
            user_metadata: Some(Value {
                kind: Some(Kind::BoolValue(true)),
            }),
            ..Default::default()
        })
        .await
        .expect("metadata update failed");
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_update_metadata_returns_invalid_argument_on_acl_update(storage_fixture: &StorageFixture) {
    use crate::common::clients::metadata::types::v1alpha::UpdateMetadataRequest;
    use storage_service_protos::google::protobuf::value::Kind;

    let service_set = storage_fixture.service_set(PermissionBackend::Grpc);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.metadata_v1alpha_client(Interface::Grpc).await;
    let path = get_random_path();

    // Configure write permission for the address.
    let resource_uri = service_set.az_url_builder().path(path).to_string();
    rule_set.allow_object_write(&resource_uri);

    // Write metadata at the specified address. The operation must succeed.
    let status = client
        .update_metadata(UpdateMetadataRequest {
            uri: resource_uri,
            user_metadata_key: "acl".into(),
            user_metadata: Some(Value {
                kind: Some(Kind::BoolValue(true)),
            }),
            ..Default::default()
        })
        .await
        .expect_err("user metadata update is expected to fail");

    assert_eq!(status.code(), Code::InvalidArgument);
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_update_metadata_returns_permission_denied_on_forbidden_action(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::UpdateMetadataRequest;
    use storage_service_protos::google::protobuf::value::Kind;

    let service_set = storage_fixture.service_set(backend);
    let client = service_set.metadata_v1alpha_client(interface).await;
    let path = get_random_path();

    // Write metadata at the specified address. The operation must fail.
    let resource_uri = service_set.az_url_builder().path(path).to_string();
    let status = client
        .update_metadata(UpdateMetadataRequest {
            uri: resource_uri,
            user_metadata_key: "some-key".into(),
            user_metadata: Some(Value {
                kind: Some(Kind::BoolValue(true)),
            }),
            ..Default::default()
        })
        .await
        .expect_err("user metadata update is expected to fail");

    assert_eq!(status.code(), Code::PermissionDenied);
}

#[rstest]
#[test_log::test(tokio::test)]
#[ignore = "not yet fixed"]
async fn test_update_metadata_returns_unauthorized_on_missing_header() {
    todo!();
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_get_metadata_batch_all_permitted(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::{GetMetadataBatchRequest, GetMetadataBatchResponse};

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.metadata_v1alpha_client(interface).await;

    let resource_uri_1 = service_set.az_url_builder().path(get_random_path()).to_string();
    let resource_uri_2 = service_set.az_url_builder().path(get_random_path()).to_string();
    let resource_uri_3 = service_set.az_url_builder().path(get_random_path()).to_string();
    rule_set.allow_object_read(&resource_uri_1);
    rule_set.allow_object_read(&resource_uri_2);
    rule_set.allow_object_read(&resource_uri_3);

    let (value_1, etag_1) = service_set.create_random_metadata(&resource_uri_1, "key-1").await;
    let (value_2, etag_2) = service_set.create_random_metadata(&resource_uri_2, "key-2").await;
    let (value_3, etag_3) = service_set.create_random_metadata(&resource_uri_3, "key-3").await;

    let response = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_1, "key-1"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_2, "key-2"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_3, "key-3"),
            ],
        })
        .await
        .expect("failed to fetch metadata");

    assert_eq!(
        response,
        GetMetadataBatchResponse {
            results: vec![
                create_get_metadata_batch_success_response_entry(0, value_1, etag_1),
                create_get_metadata_batch_success_response_entry(1, value_2, etag_2),
                create_get_metadata_batch_success_response_entry(2, value_3, etag_3),
            ],
        }
    );
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_get_metadata_batch_rejects_forbidden_writes(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::{GetMetadataBatchRequest, GetMetadataBatchResponse};

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.metadata_v1alpha_client(interface).await;

    let resource_uri_1 = service_set.az_url_builder().path(get_random_path()).to_string();
    let resource_uri_2 = service_set.az_url_builder().path(get_random_path()).to_string();
    let resource_uri_3 = service_set.az_url_builder().path(get_random_path()).to_string();
    rule_set.allow_object_read(&resource_uri_2);

    let (value, etag) = service_set.create_random_metadata(&resource_uri_2, "key-2").await;
    service_set.create_random_metadata(&resource_uri_1, "key-1").await;
    service_set.create_random_metadata(&resource_uri_3, "key-3").await;

    let response = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_1, "key-1"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_2, "key-2"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_3, "key-3"),
            ],
        })
        .await
        .expect("failed to fetch metadata");

    assert_eq!(
        response,
        GetMetadataBatchResponse {
            results: vec![
                create_get_metadata_batch_forbidden_response_entry(0),
                create_get_metadata_batch_success_response_entry(1, value, etag),
                create_get_metadata_batch_forbidden_response_entry(2),
            ],
        }
    );
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_get_metadata_batch_returns_acls(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::{GetMetadataBatchRequest, GetMetadataBatchResponse};

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.metadata_v1alpha_client(interface).await;

    let resource_uri_1 = service_set.az_url_builder().path(get_random_path()).to_string();
    let resource_uri_2 = service_set.az_url_builder().path(get_random_path()).to_string();
    let resource_uri_3 = service_set.az_url_builder().path(get_random_path()).to_string();
    rule_set.allow_object_read(&resource_uri_2);
    rule_set.allow_object_read(&resource_uri_3);
    rule_set.allow_object_write(&resource_uri_3);

    let (value_2, etag_2) = service_set.create_random_metadata(&resource_uri_2, "key-2").await;
    let (value_3, etag_3) = service_set.create_random_metadata(&resource_uri_3, "key-3").await;

    let response = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_1, "acl"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_2, "key-2"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_3, "key-3"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_2, "acl"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_3, "acl"),
            ],
        })
        .await
        .expect("failed to fetch metadata");

    assert_eq!(
        response,
        GetMetadataBatchResponse {
            results: vec![
                create_get_metadata_batch_forbidden_response_entry(0),
                create_get_metadata_batch_success_response_entry(1, value_2, etag_2),
                create_get_metadata_batch_success_response_entry(2, value_3, etag_3),
                create_get_metadata_batch_success_response_entry(3, create_acls_value(["read"]), ""),
                create_get_metadata_batch_success_response_entry(4, create_acls_value(["read", "write"]), ""),
            ],
        }
    );
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_get_metadata_batch_rejects_invalid_uris(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::{GetMetadataBatchRequest, GetMetadataBatchResponse};

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.metadata_v1alpha_client(interface).await;

    let resource_uri = service_set.az_url_builder().path(get_random_path()).to_string();
    rule_set.allow_object_read(&resource_uri);

    let (value, etag) = service_set.create_random_metadata(&resource_uri, "key-1").await;

    let response = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                create_get_metadata_batch_resource_address_request_entry("text", "key-1"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri, "key-1"),
                create_get_metadata_batch_resource_address_request_entry("text", "key-3"),
            ],
        })
        .await
        .expect("failed to fetch metadata");

    assert_eq!(
        response,
        GetMetadataBatchResponse {
            results: vec![
                create_get_metadata_batch_invalid_resource_uri_response_entry(0),
                create_get_metadata_batch_success_response_entry(1, value, etag),
                create_get_metadata_batch_invalid_resource_uri_response_entry(2),
            ],
        }
    );
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_get_metadata_batch_mixed_rejections(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::{GetMetadataBatchRequest, GetMetadataBatchResponse};

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.metadata_v1alpha_client(interface).await;

    let resource_uri_1 = service_set.az_url_builder().path(get_random_path()).to_string();
    let resource_uri_2 = service_set.az_url_builder().path(get_random_path()).to_string();
    rule_set.allow_object_read(&resource_uri_2);

    let (value_2, etag_2) = service_set.create_random_metadata(&resource_uri_2, "key-2").await;

    let response = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_1, "key-1"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_2, "key-2"),
                create_get_metadata_batch_resource_address_request_entry("text", "key-3"),
            ],
        })
        .await
        .expect("failed to fetch metadata");

    assert_eq!(
        response,
        GetMetadataBatchResponse {
            results: vec![
                create_get_metadata_batch_forbidden_response_entry(0),
                create_get_metadata_batch_success_response_entry(1, value_2, etag_2),
                create_get_metadata_batch_invalid_resource_uri_response_entry(2),
            ],
        }
    );
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_get_metadata_batch_all_invalid_uris(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::{GetMetadataBatchRequest, GetMetadataBatchResponse};

    let service_set = storage_fixture.service_set(backend);
    let client = service_set.metadata_v1alpha_client(interface).await;

    let response = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                create_get_metadata_batch_resource_address_request_entry("text-a", "key-a"),
                create_get_metadata_batch_resource_address_request_entry("text-b", "key-b"),
            ],
        })
        .await
        .expect("failed to fetch metadata");

    assert_eq!(
        response,
        GetMetadataBatchResponse {
            results: vec![
                create_get_metadata_batch_invalid_resource_uri_response_entry(0),
                create_get_metadata_batch_invalid_resource_uri_response_entry(1),
            ],
        },
    );
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_get_metadata_batch_applies_different_policies_to_objects_and_folders(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::{GetMetadataBatchRequest, GetMetadataBatchResponse};

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.metadata_v1alpha_client(interface).await;

    let path_1 = format!("{}/", get_random_path());
    let path_2 = format!("{}/", get_random_path());
    let path_3 = format!("{}/", get_random_path());
    let resource_uri_1 = service_set.az_url_builder().path(path_1).to_string();
    let resource_uri_2 = service_set.az_url_builder().path(path_2).query("etag=abcd").to_string();
    let resource_uri_3 = service_set.az_url_builder().path(path_3).to_string();

    // Since folders and objects have the same name, access to them must be explicitly configured.
    rule_set.allow_object_read(&resource_uri_1);
    rule_set.allow_object_read(&resource_uri_2);
    rule_set.allow_folder_read(&resource_uri_3);
    rule_set.forbid_folder_read(&resource_uri_1);
    rule_set.forbid_folder_read(&resource_uri_2);
    rule_set.forbid_object_read(&resource_uri_3);

    let (value_1, etag_1) = service_set.create_random_metadata(&resource_uri_1, "key-1").await;
    let (value_2, etag_2) = service_set.create_random_metadata(&resource_uri_2, "key-2").await;
    let (value_3, etag_3) = service_set.create_random_metadata(&resource_uri_3, "key-3").await;

    let response = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_1, "key-1"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_2, "key-2"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_3, "key-3"),
                create_get_metadata_batch_resource_identity_request_entry(&resource_uri_1, "key-1"),
                create_get_metadata_batch_resource_identity_request_entry(&resource_uri_2, "key-2"),
                create_get_metadata_batch_resource_identity_request_entry(&resource_uri_3, "key-3"),
                create_get_metadata_batch_folder_address_request_entry(&resource_uri_1, "key-1"),
                create_get_metadata_batch_folder_address_request_entry(&resource_uri_2, "key-2"),
                create_get_metadata_batch_folder_address_request_entry(&resource_uri_3, "key-3"),
            ],
        })
        .await
        .expect("failed to fetch metadata");

    assert_eq!(
        response,
        GetMetadataBatchResponse {
            results: vec![
                create_get_metadata_batch_success_response_entry(0, value_1, etag_1),
                create_get_metadata_batch_not_found_response_entry(1),
                create_get_metadata_batch_forbidden_response_entry(2),
                create_get_metadata_batch_invalid_resource_uri_response_entry(3),
                create_get_metadata_batch_success_response_entry(4, value_2, etag_2),
                create_get_metadata_batch_forbidden_response_entry(5),
                create_get_metadata_batch_forbidden_response_entry(6),
                create_get_metadata_batch_forbidden_response_entry(7),
                create_get_metadata_batch_success_response_entry(8, value_3, etag_3),
            ],
        }
    );
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc, 0)]
#[case(PermissionBackend::Rest, Interface::Rest, 1)]
#[case(PermissionBackend::Rest, Interface::Rest, 2)]
#[case(PermissionBackend::Rest, Interface::Rest, 4)]
#[test_log::test(tokio::test)]
async fn test_query_metadata_returns_acl_injected_acls(
    storage_fixture_with_injected_metadata_acls: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
    #[case] page_size_hint: usize,
) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let service_set = storage_fixture_with_injected_metadata_acls.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.metadata_v1alpha_client(interface).await;
    let resource_uri = service_set.az_url_builder().path(get_random_path()).to_string();

    let (value_1, etag_1) = service_set.create_random_metadata(&resource_uri, "0").await;
    let (value_2, etag_2) = service_set.create_random_metadata(&resource_uri, "1").await;

    rule_set.allow_object_read(&resource_uri);

    let query_response = client
        .query_metadata_with_page_size_hint_sorted(
            QueryMetadataRequest {
                resource_location: Some(ResourceLocation::resource_address(resource_uri)),
                ..Default::default()
            },
            page_size_hint,
        )
        .await
        .unwrap();

    assert_eq!(
        query_response,
        QueryMetadataResponse {
            results: vec![
                create_query_metadata_response_entry("0", value_1, etag_1),
                create_query_metadata_response_entry("1", value_2, etag_2),
                create_query_metadata_response_entry("acl", create_acls_value(["read"]), ""),
            ]
        }
    );
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_query_metadata_returns_acl_injected_acls_empty_downstream_metadata(
    storage_fixture_with_injected_metadata_acls: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let service_set = storage_fixture_with_injected_metadata_acls.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.metadata_v1alpha_client(interface).await;
    let resource_uri = service_set.az_url_builder().path(get_random_path()).to_string();

    rule_set.allow_object_read(&resource_uri);

    let query_response = client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::resource_address(resource_uri)),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(
        query_response,
        QueryMetadataResponse {
            results: vec![create_query_metadata_response_entry(
                "acl",
                create_acls_value(["read"]),
                ""
            ),]
        }
    );
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_query_metadata_returns_forbidden_on_lacking_permissions(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.metadata_v1alpha_client(interface).await;
    let resource_uri = service_set.az_url_builder().path(get_random_path()).to_string();

    rule_set.forbid_object_read(&resource_uri);

    let query_error = client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::resource_address(resource_uri)),
            ..Default::default()
        })
        .await
        .unwrap_err();

    assert_eq!(query_error.code(), Code::PermissionDenied);
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_query_metadata_returns_forbidden_on_lacking_permissions_injected_acls(
    storage_fixture_with_injected_metadata_acls: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let service_set = storage_fixture_with_injected_metadata_acls.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.metadata_v1alpha_client(interface).await;
    let resource_uri = service_set.az_url_builder().path(get_random_path()).to_string();

    rule_set.forbid_object_read(&resource_uri);

    let query_error = client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::resource_address(resource_uri)),
            ..Default::default()
        })
        .await
        .unwrap_err();

    assert_eq!(query_error.code(), Code::PermissionDenied);
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_query_metadata_rejects_invalid_uris(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let service_set = storage_fixture.service_set(backend);
    let client = service_set.metadata_v1alpha_client(interface).await;

    let error = client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::resource_address("text")),
            ..Default::default()
        })
        .await
        .unwrap_err();

    assert_eq!(error.code(), Code::InvalidArgument);
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_query_metadata_applies_different_policies_to_objects_and_folders(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.metadata_v1alpha_client(interface).await;

    let resource_uri = service_set
        .az_url_builder()
        .path(format!("{}/", get_random_path()))
        .to_string();
    let (value, etag) = service_set.create_random_metadata(&resource_uri, "a").await;

    // Allow reads for the object, but not for the folder. Query metadata for both. We must
    // be able to query metadata using the resource address, but not the folder address.
    rule_set.allow_object_read(&resource_uri);
    rule_set.forbid_folder_read(&resource_uri);

    let query_response_1 = client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(
        query_response_1,
        QueryMetadataResponse {
            results: vec![create_query_metadata_response_entry("a", value.clone(), &etag)],
        }
    );

    let query_error_1 = client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::folder_address(&resource_uri)),
            ..Default::default()
        })
        .await
        .unwrap_err();

    assert_eq!(query_error_1.code(), Code::PermissionDenied);

    // Flip access rule. The error pattern must also flip.
    rule_set.forbid_object_read(&resource_uri);
    rule_set.allow_folder_read(&resource_uri);

    let query_response_2 = client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::folder_address(&resource_uri)),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(
        query_response_2,
        QueryMetadataResponse {
            results: vec![create_query_metadata_response_entry("a", value, etag)],
        }
    );

    let query_error_2 = client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
            ..Default::default()
        })
        .await
        .unwrap_err();

    assert_eq!(query_error_2.code(), Code::PermissionDenied);
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_update_metadata_batch_all_permitted(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::{
        update_metadata_batch_response_entry::Outcome, UpdateMetadataBatchRequest,
    };

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.metadata_v1alpha_client(interface).await;

    let resource_uri_1 = service_set.az_url_builder().path(get_random_path()).to_string();
    let resource_uri_2 = service_set
        .az_url_builder()
        .path(get_random_path())
        .query("etag=abcd")
        .to_string();
    let resource_uri_3 = service_set
        .az_url_builder()
        .path(format!("{}/", get_random_path()))
        .to_string();
    rule_set.allow_object_write(&resource_uri_1);
    rule_set.allow_object_write(&resource_uri_2);
    rule_set.allow_folder_write(&resource_uri_3);

    let response = client
        .update_metadata_batch_sorted(UpdateMetadataBatchRequest {
            requests: vec![
                create_update_metadata_batch_resource_address_request_entry(&resource_uri_1, "key-a"),
                create_update_metadata_batch_resource_identity_request_entry(&resource_uri_2, "key-b"),
                create_update_metadata_batch_folder_address_request_entry(&resource_uri_3, "key-c"),
            ],
        })
        .await
        .expect("update metadata failed");

    assert_eq!(response.results.len(), 3);
    assert_eq!(response.results[0].request_index, 0);
    assert_eq!(response.results[1].request_index, 1);
    assert_eq!(response.results[2].request_index, 2);
    assert_matches!(&response.results[0].outcome, Some(Outcome::Success(outcome)) if !outcome.etag.is_empty());
    assert_matches!(&response.results[1].outcome, Some(Outcome::Success(outcome)) if !outcome.etag.is_empty());
    assert_matches!(&response.results[2].outcome, Some(Outcome::Success(outcome)) if !outcome.etag.is_empty());
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_update_metadata_batch_all_rejected(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::{
        batch_error_entry::Code, update_metadata_batch_response_entry::Outcome, UpdateMetadataBatchRequest,
    };

    let service_set = storage_fixture.service_set(backend);
    let client = service_set.metadata_v1alpha_client(interface).await;

    let resource_uri_1 = service_set.az_url_builder().path(get_random_path()).to_string();
    let resource_uri_2 = service_set.az_url_builder().path(get_random_path()).to_string();
    let resource_uri_3 = service_set.az_url_builder().path(get_random_path()).to_string();

    let response = client
        .update_metadata_batch_sorted(UpdateMetadataBatchRequest {
            requests: vec![
                create_update_metadata_batch_resource_address_request_entry(&resource_uri_1, "key"),
                create_update_metadata_batch_resource_identity_request_entry(&resource_uri_2, "key"),
                create_update_metadata_batch_folder_address_request_entry(&resource_uri_3, "key"),
            ],
        })
        .await
        .expect("update metadata failed");

    assert_eq!(response.results.len(), 3);
    assert_eq!(response.results[0].request_index, 0);
    assert_eq!(response.results[1].request_index, 1);
    assert_eq!(response.results[2].request_index, 2);
    assert_matches!(&response.results[0].outcome, Some(Outcome::Error(e)) if e.code == Code::PermissionDenied as i32);
    assert_matches!(&response.results[1].outcome, Some(Outcome::Error(e)) if e.code == Code::PermissionDenied as i32);
    assert_matches!(&response.results[2].outcome, Some(Outcome::Error(e)) if e.code == Code::PermissionDenied as i32);
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_update_metadata_batch_rejects_forbidden_writes(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::{
        batch_error_entry::Code, update_metadata_batch_response_entry::Outcome, UpdateMetadataBatchRequest,
    };

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.metadata_v1alpha_client(interface).await;

    let resource_uri_1 = service_set.az_url_builder().path(get_random_path()).to_string();
    let resource_uri_2 = service_set
        .az_url_builder()
        .path(get_random_path())
        .query("etag=abcd")
        .to_string();
    let resource_uri_3 = service_set
        .az_url_builder()
        .path(format!("{}/", get_random_path()))
        .to_string();
    rule_set.allow_object_write(&resource_uri_1);
    rule_set.allow_object_write(&resource_uri_2);
    rule_set.allow_folder_write(&resource_uri_3);
    rule_set.forbid_folder_write(&resource_uri_1);
    rule_set.forbid_folder_write(&resource_uri_2);
    rule_set.forbid_object_write(&resource_uri_3);

    let response = client
        .update_metadata_batch_sorted(UpdateMetadataBatchRequest {
            requests: vec![
                create_update_metadata_batch_resource_address_request_entry(&resource_uri_1, "1"),
                create_update_metadata_batch_resource_address_request_entry(&resource_uri_2, "2"),
                create_update_metadata_batch_resource_address_request_entry(&resource_uri_3, "3"),
                create_update_metadata_batch_resource_identity_request_entry(&resource_uri_1, "4"),
                create_update_metadata_batch_resource_identity_request_entry(&resource_uri_2, "5"),
                create_update_metadata_batch_resource_identity_request_entry(&resource_uri_3, "6"),
                create_update_metadata_batch_folder_address_request_entry(&resource_uri_1, "7"),
                create_update_metadata_batch_folder_address_request_entry(&resource_uri_2, "8"),
                create_update_metadata_batch_folder_address_request_entry(&resource_uri_3, "9"),
            ],
        })
        .await
        .expect("update metadata failed");

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
    assert_matches!(&response.results[0].outcome, Some(Outcome::Success(_)));
    assert_matches!(&response.results[1].outcome, Some(Outcome::Success(_)));
    assert_matches!(&response.results[2].outcome, Some(Outcome::Error(e)) if e.code == Code::PermissionDenied as i32);
    assert_matches!(&response.results[3].outcome, Some(Outcome::Error(e)) if e.code == Code::InvalidArgument as i32);
    assert_matches!(&response.results[4].outcome, Some(Outcome::Success(_)));
    assert_matches!(&response.results[5].outcome, Some(Outcome::Error(e)) if e.code == Code::PermissionDenied as i32);
    assert_matches!(&response.results[6].outcome, Some(Outcome::Error(e)) if e.code == Code::PermissionDenied as i32);
    assert_matches!(&response.results[7].outcome, Some(Outcome::Error(e)) if e.code == Code::PermissionDenied as i32);
    assert_matches!(&response.results[8].outcome, Some(Outcome::Success(_)));
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_update_metadata_batch_rejects_acl_writes(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::{
        batch_error_entry::Code, update_metadata_batch_response_entry::Outcome, UpdateMetadataBatchRequest,
    };

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.metadata_v1alpha_client(interface).await;

    let resource_uri_1 = service_set.az_url_builder().path(get_random_path()).to_string();
    let resource_uri_2 = service_set.az_url_builder().path(get_random_path()).to_string();
    let resource_uri_3 = service_set.az_url_builder().path(get_random_path()).to_string();
    rule_set.allow_object_write(&resource_uri_1);
    rule_set.allow_object_write(&resource_uri_2);
    rule_set.allow_object_write(&resource_uri_3);

    let response = client
        .update_metadata_batch_sorted(UpdateMetadataBatchRequest {
            requests: vec![
                create_update_metadata_batch_resource_address_request_entry(&resource_uri_1, "key"),
                create_update_metadata_batch_resource_address_request_entry(&resource_uri_2, "acl"),
                create_update_metadata_batch_resource_address_request_entry(&resource_uri_3, "key"),
            ],
        })
        .await
        .expect("update metadata failed");

    assert_eq!(response.results.len(), 3);
    assert_eq!(response.results[0].request_index, 0);
    assert_eq!(response.results[1].request_index, 1);
    assert_eq!(response.results[2].request_index, 2);
    assert_matches!(&response.results[0].outcome, Some(Outcome::Success(_)));
    assert_matches!(&response.results[1].outcome, Some(Outcome::Error(e)) if e.code == Code::InvalidArgument as i32);
    assert_matches!(&response.results[2].outcome, Some(Outcome::Success(_)));
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_update_metadata_batch_rejects_invalid_uris(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::{
        batch_error_entry::Code, update_metadata_batch_response_entry::Outcome, UpdateMetadataBatchRequest,
    };

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.metadata_v1alpha_client(interface).await;

    let resource_uri = service_set.az_url_builder().path(get_random_path()).to_string();
    rule_set.allow_object_write(&resource_uri);

    let response = client
        .update_metadata_batch_sorted(UpdateMetadataBatchRequest {
            requests: vec![
                create_update_metadata_batch_resource_address_request_entry("text-1", "key"),
                create_update_metadata_batch_resource_address_request_entry(&resource_uri, "key"),
                create_update_metadata_batch_resource_address_request_entry("text-2", "key"),
            ],
        })
        .await
        .expect("update metadata failed");

    assert_eq!(response.results.len(), 3);
    assert_eq!(response.results[0].request_index, 0);
    assert_eq!(response.results[1].request_index, 1);
    assert_eq!(response.results[2].request_index, 2);
    assert_matches!(&response.results[0].outcome, Some(Outcome::Error(e)) if e.code == Code::InvalidArgument as i32);
    assert_matches!(&response.results[1].outcome, Some(Outcome::Success(_)));
    assert_matches!(&response.results[2].outcome, Some(Outcome::Error(e)) if e.code == Code::InvalidArgument as i32);
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_delete_metadata_batch_all_permitted(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::{DeleteMetadataBatchRequest, DeleteMetadataBatchResponse};

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.metadata_v1alpha_client(interface).await;

    let resource_uri_1 = service_set.az_url_builder().path(get_random_path()).to_string();
    let resource_uri_2 = service_set.az_url_builder().path(get_random_path()).to_string();
    let resource_uri_3 = service_set.az_url_builder().path(get_random_path()).to_string();
    rule_set.allow_object_write(&resource_uri_1);
    rule_set.allow_object_write(&resource_uri_2);
    rule_set.allow_object_write(&resource_uri_3);

    let response = client
        .delete_metadata_batch_sorted(DeleteMetadataBatchRequest {
            requests: vec![
                create_delete_metadata_batch_resource_address_request_entry(&resource_uri_1, "1"),
                create_delete_metadata_batch_resource_address_request_entry(&resource_uri_2, "2"),
                create_delete_metadata_batch_resource_address_request_entry(&resource_uri_3, "3"),
            ],
        })
        .await
        .expect("delete metadata failed");

    assert_eq!(
        response,
        DeleteMetadataBatchResponse {
            results: vec![
                create_delete_metadata_batch_success_response_entry(0),
                create_delete_metadata_batch_success_response_entry(1),
                create_delete_metadata_batch_success_response_entry(2),
            ],
        }
    );
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_delete_metadata_batch_rejects_forbidden_writes(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::{DeleteMetadataBatchRequest, DeleteMetadataBatchResponse};

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.metadata_v1alpha_client(interface).await;

    let path_1 = format!("{}/", get_random_path());
    let path_2 = format!("{}/", get_random_path());
    let path_3 = format!("{}/", get_random_path());
    let resource_uri_1 = service_set.az_url_builder().path(path_1).to_string();
    let resource_uri_2 = service_set.az_url_builder().path(path_2).query("etag=abcd").to_string();
    let resource_uri_3 = service_set.az_url_builder().path(path_3).to_string();
    rule_set.allow_object_write(&resource_uri_1);
    rule_set.allow_object_write(&resource_uri_2);
    rule_set.allow_folder_write(&resource_uri_3);
    rule_set.forbid_folder_write(&resource_uri_1);
    rule_set.forbid_folder_write(&resource_uri_2);
    rule_set.forbid_object_write(&resource_uri_3);

    let response = client
        .delete_metadata_batch_sorted(DeleteMetadataBatchRequest {
            requests: vec![
                create_delete_metadata_batch_resource_address_request_entry(&resource_uri_1, "1"),
                create_delete_metadata_batch_resource_address_request_entry(&resource_uri_2, "2"),
                create_delete_metadata_batch_resource_address_request_entry(&resource_uri_3, "3"),
                create_delete_metadata_batch_resource_identity_request_entry(&resource_uri_1, "4"),
                create_delete_metadata_batch_resource_identity_request_entry(&resource_uri_2, "5"),
                create_delete_metadata_batch_resource_identity_request_entry(&resource_uri_3, "6"),
                create_delete_metadata_batch_folder_address_request_entry(&resource_uri_1, "7"),
                create_delete_metadata_batch_folder_address_request_entry(&resource_uri_2, "8"),
                create_delete_metadata_batch_folder_address_request_entry(&resource_uri_3, "9"),
            ],
        })
        .await
        .expect("delete metadata failed");

    assert_eq!(
        response,
        DeleteMetadataBatchResponse {
            results: vec![
                create_delete_metadata_batch_success_response_entry(0),
                create_delete_metadata_batch_success_response_entry(1),
                create_delete_metadata_batch_forbidden_response_entry(2),
                create_delete_metadata_batch_invalid_resource_uri_response_entry(3),
                create_delete_metadata_batch_success_response_entry(4),
                create_delete_metadata_batch_forbidden_response_entry(5),
                create_delete_metadata_batch_forbidden_response_entry(6),
                create_delete_metadata_batch_forbidden_response_entry(7),
                create_delete_metadata_batch_success_response_entry(8),
            ],
        }
    );
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_delete_metadata_batch_rejects_acl_writes(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::{DeleteMetadataBatchRequest, DeleteMetadataBatchResponse};

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.metadata_v1alpha_client(interface).await;

    let resource_uri_1 = service_set.az_url_builder().path(get_random_path()).to_string();
    let resource_uri_2 = service_set.az_url_builder().path(get_random_path()).to_string();
    let resource_uri_3 = service_set.az_url_builder().path(get_random_path()).to_string();
    rule_set.allow_object_write(&resource_uri_1);
    rule_set.allow_object_write(&resource_uri_2);
    rule_set.allow_object_write(&resource_uri_3);

    let response = client
        .delete_metadata_batch_sorted(DeleteMetadataBatchRequest {
            requests: vec![
                create_delete_metadata_batch_resource_address_request_entry(&resource_uri_1, "acl"),
                create_delete_metadata_batch_resource_address_request_entry(&resource_uri_2, "key"),
                create_delete_metadata_batch_resource_address_request_entry(&resource_uri_3, "acl"),
            ],
        })
        .await
        .expect("delete metadata failed");

    assert_eq!(
        response,
        DeleteMetadataBatchResponse {
            results: vec![
                create_delete_metadata_batch_metadata_key_not_editable_response_entry(0),
                create_delete_metadata_batch_success_response_entry(1),
                create_delete_metadata_batch_metadata_key_not_editable_response_entry(2)
            ]
        }
    );
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_delete_metadata_batch_rejects_invalid_uris(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::{DeleteMetadataBatchRequest, DeleteMetadataBatchResponse};

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.metadata_v1alpha_client(interface).await;

    let resource_uri = service_set.az_url_builder().path(get_random_path()).to_string();
    rule_set.allow_object_write(&resource_uri);

    let response = client
        .delete_metadata_batch_sorted(DeleteMetadataBatchRequest {
            requests: vec![
                create_delete_metadata_batch_resource_address_request_entry("text-1", "key"),
                create_delete_metadata_batch_resource_address_request_entry(&resource_uri, "acl"),
                create_delete_metadata_batch_resource_address_request_entry("text-2", "key"),
                create_delete_metadata_batch_resource_address_request_entry(&resource_uri, "key"),
            ],
        })
        .await
        .expect("delete metadata failed");

    assert_eq!(
        response,
        DeleteMetadataBatchResponse {
            results: vec![
                create_delete_metadata_batch_invalid_resource_uri_response_entry(0),
                create_delete_metadata_batch_metadata_key_not_editable_response_entry(1),
                create_delete_metadata_batch_invalid_resource_uri_response_entry(2),
                create_delete_metadata_batch_success_response_entry(3),
            ]
        }
    );
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_delete_metadata_batch_all_rejected(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::{DeleteMetadataBatchRequest, DeleteMetadataBatchResponse};

    let service_set = storage_fixture.service_set(backend);
    let client = service_set.metadata_v1alpha_client(interface).await;

    let response = client
        .delete_metadata_batch_sorted(DeleteMetadataBatchRequest {
            requests: vec![
                create_delete_metadata_batch_resource_address_request_entry("text-1", "key"),
                create_delete_metadata_batch_resource_address_request_entry("text-2", "key"),
            ],
        })
        .await
        .expect("delete metadata failed");

    assert_eq!(
        response,
        DeleteMetadataBatchResponse {
            results: vec![
                create_delete_metadata_batch_invalid_resource_uri_response_entry(0),
                create_delete_metadata_batch_invalid_resource_uri_response_entry(1),
            ]
        }
    );
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc, "", false)]
#[case(PermissionBackend::Grpc, Interface::Grpc, "/", true)]
#[case(PermissionBackend::Rest, Interface::Rest, "", false)]
#[case(PermissionBackend::Rest, Interface::Rest, "/", true)]
#[test_log::test(tokio::test)]
async fn test_delete_metadata(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
    #[case] path_suffix: &str,
    #[case] is_folder: bool,
) {
    use crate::common::clients::metadata::types::v1alpha::DeleteMetadataRequest;

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.metadata_v1alpha_client(interface).await;
    let path = format!("{}{}", get_random_path(), path_suffix);

    // Enable write operations for the uri.
    let resource_uri = service_set.az_url_builder().path(path).to_string();
    if is_folder {
        rule_set.allow_folder_write(&resource_uri);
    } else {
        rule_set.allow_object_write(&resource_uri);
    }

    // Delete a piece of metadata. The operation must succeed.
    client
        .delete_metadata(DeleteMetadataRequest {
            uri: resource_uri,
            user_metadata_key: "some-key".into(),
            ..Default::default()
        })
        .await
        .expect("metadata delete failed");
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_delete_metadata_returns_invalid_argument_on_acl_update(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::DeleteMetadataRequest;

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.metadata_v1alpha_client(interface).await;
    let path = get_random_path();

    // Enable write operations for the object.
    let resource_uri = service_set.az_url_builder().path(path).to_string();
    rule_set.allow_folder_write(&resource_uri);

    // Delete a piece of metadata. The operation must succeed.
    let status = client
        .delete_metadata(DeleteMetadataRequest {
            uri: resource_uri,
            user_metadata_key: "acl".into(),
            ..Default::default()
        })
        .await
        .expect_err("metadata delete is expected to fail");

    assert_eq!(status.code(), Code::InvalidArgument);
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_delete_metadata_returns_permission_denied_on_forbidden_action(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::DeleteMetadataRequest;

    let service_set = storage_fixture.service_set(backend);
    let client = service_set.metadata_v1alpha_client(interface).await;
    let path = get_random_path();

    // Delete a piece of metadata. The operation must fail.
    let resource_uri = service_set.az_url_builder().path(path).to_string();
    let status = client
        .delete_metadata(DeleteMetadataRequest {
            uri: resource_uri,
            user_metadata_key: "some-key".into(),
            ..Default::default()
        })
        .await
        .expect_err("metadata delete is expected to fail");

    assert_eq!(status.code(), Code::PermissionDenied);
}

#[rstest]
#[test_log::test(tokio::test)]
#[ignore = "not yet fixed"]
async fn test_delete_metadata_returns_unauthorized_on_missing_header() {
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
    use crate::common::clients::versioning::types::v1alpha::{EnumerateVersionsRequest, VersionsOrder};

    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.versioning_v1alpha_client(interface).await;
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
    use crate::common::clients::versioning::types::v1alpha::EnumerateVersionsRequest;

    let service_set = storage_fixture.service_set(backend);
    let client = service_set.versioning_v1alpha_client(interface).await;
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
    use crate::common::clients::capabilities::types::v1alpha::{ListServicesResponse, ServiceEntry};

    let service_set = storage_fixture.service_set(backend);
    let client = service_set.capabilities_v1alpha_client(interface).await;

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
async fn test_list_routes(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::capabilities::types::v1alpha::{route::RouteType, ListRoutesResponse, Route};

    let service_set = storage_fixture.service_set(backend);
    let client = service_set.capabilities_v1alpha_client(interface).await;

    // Without an explicit allow, only the wildcard routes are listed: they
    // cannot name a resource, so they are never evaluated.
    let response = client
        .list_routes(Default::default())
        .await
        .expect("routes list failed");

    assert_eq!(
        response,
        ListRoutesResponse {
            items: vec![
                Route {
                    route_type: Some(RouteType::WildcardPattern("https://*.s3.*.amazonaws.com/**".into()))
                },
                Route {
                    route_type: Some(RouteType::WildcardPattern("https://*.s3-*.amazonaws.com/**".into()))
                },
                Route {
                    route_type: Some(RouteType::WildcardPattern("https://*.blob.core.windows.net/**".into()))
                },
            ],
        }
    );
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_advertised_wildcard_route_denies_specific_address_per_request(
    storage_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::capabilities::types::v1alpha::{route::RouteType, ListRoutesResponse, Route};
    use common::clients::fileobject::types::v1alpha::StatRequest;

    let service_set = storage_fixture.service_set(backend);
    let capabilities_client = service_set.capabilities_v1alpha_client(interface).await;
    let fileobject_client = service_set.fileobject_v1alpha_client(interface).await;

    // The caller has no allow rules, so only the inner-wildcard patterns survive
    // filtering: they cannot name a resource and are advertised without evaluation.
    let response = capabilities_client
        .list_routes(Default::default())
        .await
        .expect("routes list failed");
    assert_eq!(
        response,
        ListRoutesResponse {
            items: vec![
                Route {
                    route_type: Some(RouteType::WildcardPattern("https://*.s3.*.amazonaws.com/**".into()))
                },
                Route {
                    route_type: Some(RouteType::WildcardPattern("https://*.s3-*.amazonaws.com/**".into()))
                },
                Route {
                    route_type: Some(RouteType::WildcardPattern("https://*.blob.core.windows.net/**".into()))
                },
            ],
        }
    );

    // A specific address covered by the wildcard is still evaluated per
    // request: without an allow rule the operation is denied.
    let status = fileobject_client
        .stat(StatRequest {
            resource_address: "https://someprivatebucket.s3.us-west-2.amazonaws.com/some/object".into(),
        })
        .await
        .expect_err("object stat is expected to fail");

    assert_eq!(status.code(), Code::PermissionDenied);
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_list_routes_lists_explicitly_allowed_routes(
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    use crate::common::clients::capabilities::types::v1alpha::{route::RouteType, ListRoutesResponse, Route};

    let az_config = get_env().unwrap_azure();
    let storage_fixture = StorageFixture::new(false);
    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.capabilities_v1alpha_client(interface).await;
    rule_set.allow_folder_read(az_config.storage_account_url());

    let response = client
        .list_routes(Default::default())
        .await
        .expect("routes list failed");

    assert_eq!(
        response,
        ListRoutesResponse {
            items: vec![
                Route {
                    route_type: Some(RouteType::WildcardPattern("https://*.s3.*.amazonaws.com/**".into()))
                },
                Route {
                    route_type: Some(RouteType::WildcardPattern("https://*.s3-*.amazonaws.com/**".into()))
                },
                Route {
                    route_type: Some(RouteType::WildcardPattern("https://*.blob.core.windows.net/**".into()))
                },
                Route {
                    route_type: Some(RouteType::WildcardPattern(format!(
                        "https://{}.blob.core.windows.net/**",
                        az_config.storage_account
                    ))),
                },
            ],
        }
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_bypassed_endpoint_route_listed_despite_deny_rule(bypassed_storage_fixture: &StorageFixture) {
    use crate::common::clients::capabilities::types::v1alpha::{route::RouteType, Route};

    let az_env = get_env().unwrap_azure();
    let service_set = bypassed_storage_fixture.service_set(PermissionBackend::Grpc);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.capabilities_v1alpha_client(Interface::Grpc).await;
    rule_set.forbid_folder_read(az_env.storage_account_url());

    // The endpoint opted out of permission enforcement; its route is listed
    // without consulting the permission service.
    let response = client
        .list_routes(Default::default())
        .await
        .expect("routes list failed");

    assert!(response.items.contains(&Route {
        route_type: Some(RouteType::WildcardPattern(format!(
            "https://{}.blob.core.windows.net/**",
            az_env.storage_account
        ))),
    }));
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_list_top_level_addresses(#[case] backend: PermissionBackend, #[case] interface: Interface) {
    use crate::common::clients::capabilities::types::v1alpha::TopLevelAddressEntry;

    let storage_fixture = StorageFixture::new(false);
    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.capabilities_v1alpha_client(interface).await;

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
    use crate::common::clients::capabilities::types::v1alpha::TopLevelAddressEntry;

    let storage_fixture = StorageFixture::new(false);
    let service_set = storage_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.capabilities_v1alpha_client(interface).await;

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
    use common::clients::fileobject::types::v1alpha::StatRequest;

    let service_set = storage_fixture.service_set(backend);
    let path = get_random_path();
    let client = service_set.fileobject_v1alpha_client(interface).await;

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
    use common::clients::fileobject::types::v1alpha::StatRequest;

    let service_set = storage_fixture.service_set(backend);
    let path = get_random_path();
    let client = service_set.fileobject_v1alpha_client(interface).await;

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
    use common::clients::fileobject::types::v1alpha::StatRequest;

    let service_set = bypassed_storage_fixture.service_set(backend);
    let client = service_set.fileobject_v1alpha_client(interface).await;
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
    use common::clients::fileobject::types::v1alpha::StatRequest;

    let service_set = bypassed_storage_fixture.service_set(PermissionBackend::Grpc);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.fileobject_v1alpha_client(Interface::Grpc).await;

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
    use crate::common::clients::filefolder::types::v1alpha::{FolderAddress, ListRequest};

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

    let object_client = service_set.fileobject_v1alpha_client(Interface::Grpc).await;
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

    let filefolder_client = service_set.filefolder_v1alpha_client(Interface::Grpc).await;
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
    use crate::common::clients::fileobject::types::v1alpha::EnumerateRequest;

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

    let object_client = service_set.fileobject_v1alpha_client(Interface::Grpc).await;
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
async fn test_bypassed_endpoint_copy_succeeds_despite_deny_rules(bypassed_storage_fixture: &StorageFixture) {
    use crate::common::clients::fileobject::types::v1alpha::{CopyRequest, ResourceIdentity};

    let service_set = bypassed_storage_fixture.service_set(PermissionBackend::Grpc);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.fileobject_v1alpha_client(Interface::Grpc).await;
    let source_address = service_set.az_url_builder().path(get_random_path()).to_string();
    let (source_info, _) = client
        .write_random_object(&source_address, 32)
        .await
        .expect("object write failed");
    let copy_target = service_set.az_url_builder().path(get_random_path()).to_string();

    // Rules that would reject the copy on an enforced endpoint.
    rule_set.forbid_object_read(&source_address);
    rule_set.forbid_object_write(&copy_target);

    client
        .copy(CopyRequest {
            source_resource_identity: Some(ResourceIdentity {
                encoded_identity: source_info.unwrap_identity(),
            }),
            destination_resource_address: copy_target,
            ..Default::default()
        })
        .await
        .expect("object copy failed");
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_bypassed_endpoint_move_succeeds_despite_deny_rules(bypassed_storage_fixture: &StorageFixture) {
    use crate::common::clients::fileobject::types::v1alpha::MoveRequest;

    let service_set = bypassed_storage_fixture.service_set(PermissionBackend::Grpc);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.fileobject_v1alpha_client(Interface::Grpc).await;
    let source_address = service_set.az_url_builder().path(get_random_path()).to_string();
    client
        .write_random_object(&source_address, 32)
        .await
        .expect("object write failed");
    let move_target = service_set.az_url_builder().path(get_random_path()).to_string();

    // Rules that would reject the move on an enforced endpoint.
    rule_set.forbid_object_read(&source_address);
    rule_set.forbid_object_write(&source_address);
    rule_set.forbid_object_write(&move_target);

    client
        .move_object(MoveRequest {
            source_resource_address: source_address,
            destination_resource_address: move_target,
            ..Default::default()
        })
        .await
        .expect("object move failed");
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_bypassed_endpoint_get_metadata_batch_succeeds_despite_deny_rules(
    bypassed_storage_fixture: &StorageFixture,
) {
    use crate::common::clients::metadata::types::v1alpha::GetMetadataBatchRequest;

    let service_set = bypassed_storage_fixture.service_set(PermissionBackend::Grpc);
    let rule_set = service_set.permission_context.new_rule_set();
    let metadata_client = service_set.metadata_v1alpha_client(Interface::Grpc).await;

    let resource_uri_1 = service_set.az_url_builder().path(get_random_path()).to_string();
    let resource_uri_2 = service_set.az_url_builder().path(get_random_path()).to_string();
    let (value_1, etag_1) = service_set.create_random_metadata(&resource_uri_1, "key-1").await;
    let (value_2, etag_2) = service_set.create_random_metadata(&resource_uri_2, "key-2").await;

    // Rules that would reject both entries on an enforced endpoint.
    rule_set.forbid_object_read(&resource_uri_1);
    rule_set.forbid_object_read(&resource_uri_2);

    let response = metadata_client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_1, "key-1"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_2, "key-2"),
            ],
        })
        .await
        .expect("failed to fetch metadata");

    assert_eq!(
        response.results,
        vec![
            create_get_metadata_batch_success_response_entry(0, value_1, etag_1),
            create_get_metadata_batch_success_response_entry(1, value_2, etag_2),
        ],
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_get_metadata_batch_doesnt_include_acl_in_nondefault_ns(
    storage_fixture_with_injected_metadata_acls: &StorageFixture,
) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let service_set = storage_fixture_with_injected_metadata_acls.service_set(PermissionBackend::Grpc);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.metadata_v1alpha_client(Interface::Grpc).await;
    let resource_uri = service_set.az_url_builder().path(get_random_path()).to_string();
    rule_set.allow_object_read(&resource_uri);

    let (value, etag) = service_set
        .create_random_metadata_with_ns(ResourceLocation::resource_address(&resource_uri), "a", Some("custom"))
        .await;

    let get_response = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                GetMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "a".into(),
                    namespace: Some("custom".into()),
                },
                GetMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "acl".into(),
                    namespace: Some("custom".into()),
                },
            ],
        })
        .await
        .unwrap();

    assert_eq!(
        get_response,
        GetMetadataBatchResponse {
            results: vec![
                create_get_metadata_batch_success_response_entry(0, value, etag),
                create_get_metadata_batch_not_found_response_entry(1),
            ],
        }
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_bypassed_endpoint_get_metadata_batch_returns_all_acls(
    bypassed_storage_fixture_with_injected_metadata_acls: &StorageFixture,
) {
    use crate::common::clients::metadata::types::v1alpha::GetMetadataBatchRequest;

    let service_set = bypassed_storage_fixture_with_injected_metadata_acls.service_set(PermissionBackend::Grpc);
    let rule_set = service_set.permission_context.new_rule_set();
    let metadata_client = service_set.metadata_v1alpha_client(Interface::Grpc).await;

    let resource_uri = service_set.az_url_builder().path(get_random_path()).to_string();
    service_set.create_random_metadata(&resource_uri, "key-1").await;

    // A rule that would reject the ACL query on an enforced endpoint.
    rule_set.forbid_object_read(&resource_uri);

    // A bypassed ACL query returns the full ACL set without consulting the rules.
    let response = metadata_client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![create_get_metadata_batch_resource_address_request_entry(
                &resource_uri,
                "acl",
            )],
        })
        .await
        .expect("failed to fetch metadata");

    assert_matches!(
        &response.results[..],
        [entry] if entry.request_index == 0
            && matches!(
                &entry.outcome,
                Some(get_metadata_batch_response_entry::Outcome::Success(_))
            )
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_bypassed_endpoint_query_metadata_returns_all_acls(
    bypassed_storage_fixture_with_injected_metadata_acls: &StorageFixture,
) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let service_set = bypassed_storage_fixture_with_injected_metadata_acls.service_set(PermissionBackend::Grpc);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.metadata_v1alpha_client(Interface::Grpc).await;
    let resource_uri = service_set.az_url_builder().path(get_random_path()).to_string();
    let (value, etag) = service_set.create_random_metadata(&resource_uri, "0").await;

    // A rule that would reject the injected ACL query on an enforced endpoint.
    rule_set.forbid_object_read(&resource_uri);

    let query_response = client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::resource_address(resource_uri)),
            ..Default::default()
        })
        .await
        .expect("failed to query metadata");

    assert_eq!(
        query_response,
        QueryMetadataResponse {
            results: vec![
                create_query_metadata_response_entry("0", value, etag),
                create_query_metadata_response_entry("acl", create_acls_value(["read", "write"]), ""),
            ]
        }
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_bypassed_endpoint_query_metadata_doesnt_include_acl(bypassed_storage_fixture: &StorageFixture) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let service_set = bypassed_storage_fixture.service_set(PermissionBackend::Grpc);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.metadata_v1alpha_client(Interface::Grpc).await;
    let resource_uri = service_set.az_url_builder().path(get_random_path()).to_string();
    let (value, etag) = service_set.create_random_metadata(&resource_uri, "0").await;

    // A rule that would reject the query on an enforced endpoint.
    rule_set.forbid_object_read(&resource_uri);

    let query_response = client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::resource_address(resource_uri)),
            ..Default::default()
        })
        .await
        .expect("failed to query metadata");

    assert_eq!(
        query_response,
        QueryMetadataResponse {
            results: vec![create_query_metadata_response_entry("0", value, etag)]
        }
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_bypassed_endpoint_lists_top_level_address_for_principal_without_acls(
    bypassed_storage_fixture: &StorageFixture,
) {
    use crate::common::clients::capabilities::types::v1alpha::TopLevelAddressEntry;

    let service_set = bypassed_storage_fixture.service_set(PermissionBackend::Grpc);
    let client = service_set.capabilities_v1alpha_client(Interface::Grpc).await;
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

#[rstest]
#[test_log::test(tokio::test)]
async fn test_query_metadata_doesnt_include_acl_in_nondefault_ns(
    storage_fixture_with_injected_metadata_acls: &StorageFixture,
) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let service_set = storage_fixture_with_injected_metadata_acls.service_set(PermissionBackend::Grpc);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.metadata_v1alpha_client(Interface::Grpc).await;
    let resource_uri = service_set.az_url_builder().path(get_random_path()).to_string();
    rule_set.allow_object_read(&resource_uri);

    let (value, etag) = service_set
        .create_random_metadata_with_ns(ResourceLocation::resource_address(&resource_uri), "a", Some("custom"))
        .await;

    let query_response = client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
            namespace: Some("custom".into()),
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

fn create_acls_value(permissions: impl IntoIterator<Item = impl Into<String>>) -> Value {
    use storage_service_protos::google::protobuf::{value::Kind, ListValue};

    Value {
        kind: Some(Kind::ListValue(ListValue {
            values: permissions
                .into_iter()
                .map(|value| Value {
                    kind: Some(Kind::StringValue(value.into())),
                })
                .collect(),
        })),
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

fn create_forbidden_batch_error_entry() -> BatchErrorEntry {
    BatchErrorEntry {
        code: batch_error_entry::Code::PermissionDenied as i32,
        message: "access to the resource is forbidden".into(),
    }
}

fn create_not_found_batch_error_entry() -> BatchErrorEntry {
    BatchErrorEntry {
        code: batch_error_entry::Code::NotFound as i32,
        message: "metadata key not found".into(),
    }
}

fn create_invalid_resource_uri_batch_error_entry() -> BatchErrorEntry {
    BatchErrorEntry {
        code: batch_error_entry::Code::InvalidArgument as i32,
        message: "resource uri is invalid".into(),
    }
}

fn create_metadata_key_not_editable_error_entry() -> BatchErrorEntry {
    BatchErrorEntry {
        code: batch_error_entry::Code::InvalidArgument as i32,
        message: "metadata key is not editable".into(),
    }
}

fn create_get_metadata_batch_success_response_entry(
    request_index: u32,
    value: Value,
    etag: impl Into<String>,
) -> GetMetadataBatchResponseEntry {
    GetMetadataBatchResponseEntry {
        request_index,
        outcome: Some(get_metadata_batch_response_entry::Outcome::Success(UserMetadataValue {
            value: Some(value),
            etag: etag.into(),
        })),
    }
}

fn create_get_metadata_batch_forbidden_response_entry(request_index: u32) -> GetMetadataBatchResponseEntry {
    GetMetadataBatchResponseEntry {
        request_index,
        outcome: Some(get_metadata_batch_response_entry::Outcome::Error(
            create_forbidden_batch_error_entry(),
        )),
    }
}

fn create_get_metadata_batch_invalid_resource_uri_response_entry(request_index: u32) -> GetMetadataBatchResponseEntry {
    GetMetadataBatchResponseEntry {
        request_index,
        outcome: Some(get_metadata_batch_response_entry::Outcome::Error(
            create_invalid_resource_uri_batch_error_entry(),
        )),
    }
}

fn create_get_metadata_batch_not_found_response_entry(request_index: u32) -> GetMetadataBatchResponseEntry {
    GetMetadataBatchResponseEntry {
        request_index,
        outcome: Some(get_metadata_batch_response_entry::Outcome::Error(
            create_not_found_batch_error_entry(),
        )),
    }
}

fn create_delete_metadata_batch_success_response_entry(request_index: u32) -> DeleteMetadataBatchResponseEntry {
    DeleteMetadataBatchResponseEntry {
        request_index,
        outcome: Some(delete_metadata_batch_response_entry::Outcome::Success(
            Default::default(),
        )),
    }
}

fn create_delete_metadata_batch_invalid_resource_uri_response_entry(
    request_index: u32,
) -> DeleteMetadataBatchResponseEntry {
    DeleteMetadataBatchResponseEntry {
        request_index,
        outcome: Some(delete_metadata_batch_response_entry::Outcome::Error(
            create_invalid_resource_uri_batch_error_entry(),
        )),
    }
}

fn create_delete_metadata_batch_forbidden_response_entry(request_index: u32) -> DeleteMetadataBatchResponseEntry {
    DeleteMetadataBatchResponseEntry {
        request_index,
        outcome: Some(delete_metadata_batch_response_entry::Outcome::Error(
            create_forbidden_batch_error_entry(),
        )),
    }
}

fn create_delete_metadata_batch_metadata_key_not_editable_response_entry(
    request_index: u32,
) -> DeleteMetadataBatchResponseEntry {
    DeleteMetadataBatchResponseEntry {
        request_index,
        outcome: Some(delete_metadata_batch_response_entry::Outcome::Error(
            create_metadata_key_not_editable_error_entry(),
        )),
    }
}

fn create_query_metadata_response_entry(
    key: impl Into<String>,
    value: impl Into<Value>,
    etag: impl Into<String>,
) -> QueryMetadataResponseEntry {
    QueryMetadataResponseEntry {
        user_metadata_key: key.into(),
        user_metadata: Some(UserMetadataValue {
            value: Some(value.into()),
            etag: etag.into(),
        }),
    }
}

fn create_random_value() -> Value {
    use storage_service_protos::google::protobuf::value::Kind;

    Value {
        kind: Some(Kind::StringValue(get_random_alphanumeric_string(32))),
    }
}
