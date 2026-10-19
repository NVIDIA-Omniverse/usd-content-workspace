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

use axum::http::{header::AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use common::types::Interface;
use futures::TryStreamExt;
use rstest::{fixture, rstest};
use storage_service_test::mocks::permissions::{
    grpc::Service as GrpcPermissionService, rest::Service as RestPermissionService, Context,
};

use crate::common::clients::fileobject::types::v1beta::{EnumerateRequest, StatRequest};
use crate::common::clients::utils::Request;
use crate::common::random::get_random_path;
use crate::common::service::Service as StorageService;

const USER_ID: HeaderName = HeaderName::from_static("user-id");
const X_REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");
const X_OMNIVERSE_REQUEST_ID: HeaderName = HeaderName::from_static("x-omniverse-request-id");

#[derive(Debug, Copy, Clone)]
enum PermissionBackend {
    Rest,
    Grpc,
}

struct ServiceSet<'a> {
    storage_service: &'a StorageService,
    permission_context: &'a Context,
}

impl ServiceSet<'_> {
    /// Create an object and discard the permission requests the write produced for it.
    async fn create_random_object(&self, address: &str) {
        let rule_set = self.permission_context.new_rule_set();
        rule_set.allow_object_write(address);

        self.storage_service
            .fileobject_v1beta_client(Interface::Grpc)
            .await
            .write_random_object(address, 32)
            .await
            .expect("object write failed");

        self.permission_context.take_recorded_headers(address);
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
    fn new(forward_headers: Option<&[&str]>) -> Self {
        let rest_permission_context = Context::new();
        let rest_permission_service = RestPermissionService::new(rest_permission_context.clone());

        let grpc_permission_context = Context::new();
        let grpc_permission_service = GrpcPermissionService::new(grpc_permission_context.clone());

        let builder = |backend: &str, endpoint: String| {
            let builder = StorageService::builder()
                .enable_az(true)
                .enable_postgres_metadata()
                .enable_permissions(backend, endpoint, false);
            match forward_headers {
                Some(names) => builder.forward_headers(names.iter().copied()),
                None => builder,
            }
        };

        Self {
            rest_storage_service: builder("rest", rest_permission_service.endpoint_url()).start(),
            grpc_storage_service: builder("grpc", grpc_permission_service.endpoint_url()).start(),
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
fn forwarding_fixture() -> StorageFixture {
    StorageFixture::new(Some(&["X-Request-Id", "user-id"]))
}

#[fixture]
#[once]
fn default_fixture() -> StorageFixture {
    StorageFixture::new(None)
}

/// Build the headers of a request carrying identity headers, one of them multi-valued.
fn identity_headers(request_id: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer valid.token"));
    headers.insert(X_REQUEST_ID, HeaderValue::from_str(request_id).unwrap());
    headers.insert(X_OMNIVERSE_REQUEST_ID, HeaderValue::from_static("not-forwarded"));
    headers.append(USER_ID, HeaderValue::from_static("alice"));
    headers.append(USER_ID, HeaderValue::from_static("bob"));
    headers
}

fn header_values<'a>(headers: &'a HeaderMap, name: &HeaderName) -> Vec<&'a [u8]> {
    headers.get_all(name).iter().map(HeaderValue::as_bytes).collect()
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Grpc, Interface::Rest)]
#[case(PermissionBackend::Rest, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_forward_configured_headers(
    forwarding_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    let service_set = forwarding_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.storage_service.fileobject_v1beta_client(interface).await;

    let resource_address = service_set
        .storage_service
        .az_url_builder()
        .path(get_random_path())
        .to_string();
    service_set.create_random_object(&resource_address).await;
    rule_set.allow_object_read(&resource_address);
    let request_id = format!("forwarded-{}", get_random_path());

    client
        .stat(Request {
            message: StatRequest {
                resource_address: resource_address.clone(),
            },
            headers: identity_headers(&request_id),
        })
        .await
        .expect("object stat failed");

    let recorded = service_set.permission_context.take_recorded_headers(&resource_address);
    assert_eq!(recorded.len(), 1);
    let headers = &recorded[0];
    assert_eq!(header_values(headers, &X_REQUEST_ID), [request_id.as_bytes()]);
    assert_eq!(
        header_values(headers, &USER_ID),
        [b"alice".as_slice(), b"bob".as_slice()]
    );
    assert!(!headers.contains_key(&X_OMNIVERSE_REQUEST_ID));
    assert_eq!(
        header_values(headers, &AUTHORIZATION),
        [b"Bearer valid.token".as_slice()]
    );
}

#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_forward_headers_disabled_by_default(
    default_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    let service_set = default_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.storage_service.fileobject_v1beta_client(interface).await;

    let resource_address = service_set
        .storage_service
        .az_url_builder()
        .path(get_random_path())
        .to_string();
    service_set.create_random_object(&resource_address).await;
    rule_set.allow_object_read(&resource_address);

    client
        .stat(Request {
            message: StatRequest {
                resource_address: resource_address.clone(),
            },
            headers: identity_headers("not-forwarded"),
        })
        .await
        .expect("object stat failed");

    let recorded = service_set.permission_context.take_recorded_headers(&resource_address);
    assert_eq!(recorded.len(), 1);
    let headers = &recorded[0];
    assert!(!headers.contains_key(&X_REQUEST_ID));
    assert!(!headers.contains_key(&USER_ID));
    assert_eq!(
        header_values(headers, &AUTHORIZATION),
        [b"Bearer valid.token".as_slice()]
    );
}

// Enumeration filters the response stream with a copy of the request principal, so the forwarded
// headers must survive that path as well.
#[rstest]
#[case(PermissionBackend::Grpc, Interface::Grpc)]
#[case(PermissionBackend::Rest, Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_forward_headers_on_enumerate(
    forwarding_fixture: &StorageFixture,
    #[case] backend: PermissionBackend,
    #[case] interface: Interface,
) {
    let service_set = forwarding_fixture.service_set(backend);
    let rule_set = service_set.permission_context.new_rule_set();
    let client = service_set.storage_service.fileobject_v1beta_client(interface).await;

    let folder_path = format!("{}/", get_random_path());
    let folder_address = service_set
        .storage_service
        .az_url_builder()
        .path(&folder_path)
        .to_string();
    let object_address = service_set
        .storage_service
        .az_url_builder()
        .path(format!("{folder_path}a"))
        .to_string();
    service_set.create_random_object(&object_address).await;
    rule_set.allow_folder_read(&folder_address);
    rule_set.allow_object_read(&object_address);

    // The test client attaches the default `x-request-id: test-N` header to the enumerate request.
    let responses: Vec<_> = client
        .enumerate(EnumerateRequest {
            resource_address: folder_address.clone(),
        })
        .try_collect()
        .await
        .expect("enumerate failed");

    assert!(responses.iter().any(|response| !response.items.is_empty()));
    // The folder is evaluated twice: once for the enumerate request itself and once as the parent of the
    // single item in the filtered response.
    let recorded = service_set.permission_context.take_recorded_headers(&folder_address);
    assert_eq!(recorded.len(), 2);
    assert!(forwarded_request_id(&recorded[0]).starts_with("test-"));
    assert!(forwarded_request_id(&recorded[1]).starts_with("test-"));
    assert!(!recorded[0].contains_key(&X_OMNIVERSE_REQUEST_ID));
    assert!(!recorded[1].contains_key(&X_OMNIVERSE_REQUEST_ID));
}

fn forwarded_request_id(headers: &HeaderMap) -> &str {
    headers
        .get(&X_REQUEST_ID)
        .expect("x-request-id was not forwarded")
        .to_str()
        .unwrap()
}
