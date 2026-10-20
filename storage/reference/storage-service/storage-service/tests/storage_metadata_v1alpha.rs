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

#![allow(clippy::too_many_arguments)]

mod common;

use std::collections::HashMap;

use crate::common::clients::metadata::v1alpha::Client;
use crate::common::random::get_random_path;
use crate::common::service::Service as StorageService;
use crate::common::types::Interface;
use crate::common::url::UrlBuilder;
use rstest::{fixture, rstest};
use storage_service_protos::google;
use storage_service_protos::nvidia::omniverse::storage::metadata::v1alpha::*;
use tonic::Code;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum Backend {
    #[cfg(feature = "dynamodb")]
    DynamoDB,
    #[cfg(feature = "cosmosdb")]
    CosmosDB,
    Postgres,
}

struct TestServices {
    #[cfg(feature = "dynamodb")]
    dynamodb_storage_service: StorageService,
    #[cfg(feature = "cosmosdb")]
    cosmosdb_storage_service: StorageService,
    postgres_storage_service: StorageService,
}

impl TestServices {
    async fn metadata_v1alpha_client(&self, backend: Backend, interface: Interface) -> Client {
        match backend {
            #[cfg(feature = "dynamodb")]
            Backend::DynamoDB => self.dynamodb_storage_service.metadata_v1alpha_client(interface).await,
            #[cfg(feature = "cosmosdb")]
            Backend::CosmosDB => self.cosmosdb_storage_service.metadata_v1alpha_client(interface).await,
            Backend::Postgres => self.postgres_storage_service.metadata_v1alpha_client(interface).await,
        }
    }

    fn url_builder(&self, backend: Backend) -> UrlBuilder {
        match backend {
            #[cfg(feature = "dynamodb")]
            Backend::DynamoDB => self.dynamodb_storage_service.az_url_builder(),
            #[cfg(feature = "cosmosdb")]
            Backend::CosmosDB => self.cosmosdb_storage_service.az_url_builder(),
            Backend::Postgres => self.postgres_storage_service.s3_url_builder(),
        }
    }
}

#[fixture]
#[once]
fn test_services() -> TestServices {
    TestServices {
        #[cfg(feature = "dynamodb")]
        dynamodb_storage_service: StorageService::builder()
            .enable_az(true)
            .enable_dynamodb_metadata()
            .start(),
        #[cfg(feature = "cosmosdb")]
        cosmosdb_storage_service: StorageService::builder()
            .enable_az(true)
            .enable_cosmosdb_metadata()
            .start(),
        postgres_storage_service: StorageService::builder()
            .enable_s3(true)
            .enable_postgres_metadata()
            .start(),
    }
}

#[fixture]
#[once]
fn pg_test_service() -> StorageService {
    StorageService::builder()
        .enable_s3(true)
        .enable_postgres_metadata()
        .enable_metadata_cache(false)
        .start()
}

#[rstest]
#[case(Backend::Postgres, Interface::Grpc, vec![], vec!["key-a", "key-b"])]
#[case(Backend::Postgres, Interface::Rest, vec![], vec!["key-a", "key-b"])]
#[case(Backend::Postgres, Interface::Grpc, vec!["key-a"], vec!["key-a"])]
#[case(Backend::Postgres, Interface::Rest, vec!["key-a"], vec!["key-a"])]
#[cfg_attr(feature = "dynamodb", case(Backend::DynamoDB, Interface::Grpc, vec![], vec!["key-a", "key-b"]))]
#[cfg_attr(feature = "dynamodb", case(Backend::DynamoDB, Interface::Grpc, vec!["key-a"], vec!["key-a"]))]
#[cfg_attr(feature = "cosmosdb", case(Backend::CosmosDB, Interface::Grpc, vec![], vec!["key-a", "key-b"]))]
#[cfg_attr(feature = "cosmosdb", case(Backend::CosmosDB, Interface::Grpc, vec!["key-a"], vec!["key-a"]))]
#[test_log::test(tokio::test)]
async fn set_get_metadata(
    test_services: &TestServices,
    #[case] backend: Backend,
    #[case] interface: Interface,
    #[case] key_filter: Vec<&str>,
    #[case] expected_response_keys: Vec<&str>,
) {
    let client = test_services.metadata_v1alpha_client(backend, interface).await;
    let resource_uri = test_services.url_builder(backend).path(get_random_path()).to_string();

    let metadata_key_1 = "key-a";
    let metadata_key_2 = "key-b";
    let metadata_value_1 = create_string_value("value-a");
    let metadata_value_2 = create_string_value("value-b");
    let metadata_entries = [
        (
            metadata_key_1,
            metadata_value_1.clone(),
            update_metadata_entry(&client, &resource_uri, metadata_key_1, metadata_value_1).await,
        ),
        (
            metadata_key_2,
            metadata_value_2.clone(),
            update_metadata_entry(&client, &resource_uri, metadata_key_2, metadata_value_2).await,
        ),
    ];

    let expected_user_metadata: HashMap<String, UserMetadataValue> = metadata_entries
        .into_iter()
        .filter_map(|(key, value, etag)| {
            if expected_response_keys.contains(&key) {
                Some((
                    key.to_string(),
                    UserMetadataValue {
                        value: Some(value),
                        etag,
                    },
                ))
            } else {
                None
            }
        })
        .collect();

    let get_metadata_response = client
        .get_metadata(GetMetadataRequest {
            uri: resource_uri,
            user_metadata_keys: key_filter.into_iter().map(Into::into).collect(),
        })
        .await
        .expect("get metadata failed");
    assert_eq!(
        get_metadata_response,
        GetMetadataResponse {
            user_metadata: expected_user_metadata
        }
    );
}

#[rstest]
#[case(Backend::Postgres, Interface::Grpc)]
#[case(Backend::Postgres, Interface::Rest)]
#[cfg_attr(feature = "dynamodb", case(Backend::DynamoDB, Interface::Grpc))]
#[cfg_attr(feature = "cosmosdb", case(Backend::CosmosDB, Interface::Grpc))]
#[test_log::test(tokio::test)]
async fn get_nonexistent_metadata(
    test_services: &TestServices,
    #[case] backend: Backend,
    #[case] interface: Interface,
) {
    let client = test_services.metadata_v1alpha_client(backend, interface).await;
    let resource_uri = test_services.url_builder(backend).path(get_random_path()).to_string();

    let get_metadata_response = client
        .get_metadata(GetMetadataRequest {
            uri: resource_uri,
            ..Default::default()
        })
        .await
        .expect("get metadata failed");
    assert_eq!(get_metadata_response, GetMetadataResponse::default())
}

#[rstest]
#[case(Backend::Postgres, Interface::Grpc)]
#[case(Backend::Postgres, Interface::Rest)]
#[cfg_attr(feature = "dynamodb", case(Backend::DynamoDB, Interface::Grpc))]
#[cfg_attr(feature = "cosmosdb", case(Backend::CosmosDB, Interface::Grpc))]
#[test_log::test(tokio::test)]
async fn get_metadata_after_set_retrieves_the_latest_value(
    test_services: &TestServices,
    #[case] backend: Backend,
    #[case] interface: Interface,
) {
    let client = test_services.metadata_v1alpha_client(backend, interface).await;
    let resource_uri = test_services.url_builder(backend).path(get_random_path()).to_string();
    let metadata_key = "key";
    let metadata_value = create_string_value("value-2");

    update_metadata_entry(&client, &resource_uri, metadata_key, create_string_value("value-1")).await;
    let etag = update_metadata_entry(&client, &resource_uri, metadata_key, metadata_value.clone()).await;
    let expected_get_metadata_response = GetMetadataResponse {
        user_metadata: [(
            metadata_key.to_string(),
            UserMetadataValue {
                etag,
                value: Some(metadata_value),
            },
        )]
        .into_iter()
        .collect(),
    };

    let get_metadata_response = client
        .get_metadata(GetMetadataRequest {
            uri: resource_uri,
            ..Default::default()
        })
        .await
        .expect("get metadata failed");
    assert_eq!(get_metadata_response, expected_get_metadata_response);
}

#[rstest]
#[case(Backend::Postgres, Interface::Grpc)]
#[case(Backend::Postgres, Interface::Rest)]
#[cfg_attr(feature = "dynamodb", case(Backend::DynamoDB, Interface::Grpc))]
#[cfg_attr(feature = "cosmosdb", case(Backend::CosmosDB, Interface::Grpc))]
#[test_log::test(tokio::test)]
async fn set_metadata_conditionally(
    test_services: &TestServices,
    #[case] backend: Backend,
    #[case] interface: Interface,
) {
    let client = test_services.metadata_v1alpha_client(backend, interface).await;
    let resource_uri = test_services.url_builder(backend).path(get_random_path()).to_string();
    let metadata_key = "key";
    let metadata_value = create_string_value("value-3");

    let etag_1 = update_metadata_entry(&client, &resource_uri, metadata_key, create_string_value("a")).await;
    let etag_2 = update_metadata_entry(&client, &resource_uri, metadata_key, create_string_value("b")).await;
    assert_ne!(etag_1, etag_2);

    let update_metadata_response = client
        .update_metadata(UpdateMetadataRequest {
            uri: resource_uri.clone(),
            user_metadata_key: metadata_key.into(),
            user_metadata: Some(metadata_value.clone()),
            expected_etag: Some(etag_2),
        })
        .await
        .expect("update metadata failed");
    let expected_get_metadata_response = GetMetadataResponse {
        user_metadata: [(
            metadata_key.to_string(),
            UserMetadataValue {
                value: Some(metadata_value),
                etag: update_metadata_response.etag,
            },
        )]
        .into_iter()
        .collect(),
    };

    let get_metadata_response = client
        .get_metadata(GetMetadataRequest {
            uri: resource_uri,
            ..Default::default()
        })
        .await
        .expect("get metadata failed");
    assert_eq!(get_metadata_response, expected_get_metadata_response);
}

#[rstest]
#[case(Backend::Postgres, Interface::Grpc)]
#[case(Backend::Postgres, Interface::Rest)]
#[cfg_attr(feature = "dynamodb", case(Backend::DynamoDB, Interface::Grpc))]
#[cfg_attr(feature = "cosmosdb", case(Backend::CosmosDB, Interface::Grpc))]
#[test_log::test(tokio::test)]
async fn set_metadata_conditionally_returns_failed_precondition_on_invalid_etag(
    test_services: &TestServices,
    #[case] backend: Backend,
    #[case] interface: Interface,
) {
    let client = test_services.metadata_v1alpha_client(backend, interface).await;
    let resource_uri = test_services.url_builder(backend).path(get_random_path()).to_string();
    let metadata_key = "key";

    let etag_1 = update_metadata_entry(&client, &resource_uri, metadata_key, create_string_value("a")).await;
    let etag_2 = update_metadata_entry(&client, &resource_uri, metadata_key, create_string_value("b")).await;
    assert_ne!(etag_1, etag_2);

    let update_metadata_status = client
        .update_metadata(UpdateMetadataRequest {
            uri: resource_uri,
            user_metadata_key: metadata_key.into(),
            user_metadata: Some(create_string_value("c")),
            expected_etag: Some(etag_1),
        })
        .await
        .expect_err("update metadata did not fail");
    assert_eq!(update_metadata_status.code(), Code::FailedPrecondition);
}

#[rstest]
#[case(Backend::Postgres, Interface::Grpc)]
#[case(Backend::Postgres, Interface::Rest)]
#[cfg_attr(feature = "dynamodb", case(Backend::DynamoDB, Interface::Grpc))]
#[cfg_attr(feature = "cosmosdb", case(Backend::CosmosDB, Interface::Grpc))]
#[test_log::test(tokio::test)]
async fn delete_metadata(test_services: &TestServices, #[case] backend: Backend, #[case] interface: Interface) {
    let client = test_services.metadata_v1alpha_client(backend, interface).await;
    let resource_uri = test_services.url_builder(backend).path(get_random_path()).to_string();
    let metadata_key = "key-a";

    update_metadata_entry(&client, &resource_uri, metadata_key, create_string_value("some-value")).await;

    let get_metadata_response = client
        .get_metadata(GetMetadataRequest {
            uri: resource_uri.clone(),
            ..Default::default()
        })
        .await
        .expect("get metadata failed");
    assert_eq!(get_metadata_response.user_metadata.len(), 1);

    client
        .delete_metadata(DeleteMetadataRequest {
            uri: resource_uri.to_string(),
            user_metadata_key: metadata_key.to_string(),
            ..Default::default()
        })
        .await
        .expect("delete metadata failed");

    let get_metadata_response = client
        .get_metadata(GetMetadataRequest {
            uri: resource_uri,
            ..Default::default()
        })
        .await
        .expect("get metadata failed");
    assert_eq!(get_metadata_response, GetMetadataResponse::default());
}

#[rstest]
#[case(Backend::Postgres, Interface::Grpc)]
#[case(Backend::Postgres, Interface::Rest)]
#[cfg_attr(feature = "dynamodb", case(Backend::DynamoDB, Interface::Grpc))]
#[cfg_attr(feature = "cosmosdb", case(Backend::CosmosDB, Interface::Grpc))]
#[test_log::test(tokio::test)]
async fn delete_nonexistent_metadata(
    test_services: &TestServices,
    #[case] backend: Backend,
    #[case] interface: Interface,
) {
    let client = test_services.metadata_v1alpha_client(backend, interface).await;
    let resource_uri = test_services.url_builder(backend).path(get_random_path()).to_string();

    client
        .delete_metadata(DeleteMetadataRequest {
            uri: resource_uri,
            user_metadata_key: "key".into(),
            ..Default::default()
        })
        .await
        .expect("delete metadata failed");
}

#[rstest]
#[case(Backend::Postgres, Interface::Grpc)]
#[case(Backend::Postgres, Interface::Rest)]
#[cfg_attr(feature = "dynamodb", case(Backend::DynamoDB, Interface::Grpc))]
#[cfg_attr(feature = "cosmosdb", case(Backend::CosmosDB, Interface::Grpc))]
#[test_log::test(tokio::test)]
async fn delete_metadata_conditionally(
    test_services: &TestServices,
    #[case] backend: Backend,
    #[case] interface: Interface,
) {
    let client = test_services.metadata_v1alpha_client(backend, interface).await;
    let resource_uri = test_services.url_builder(backend).path(get_random_path()).to_string();
    let metadata_key = "key";

    let etag_1 = update_metadata_entry(&client, &resource_uri, metadata_key, create_string_value("a")).await;
    let etag_2 = update_metadata_entry(&client, &resource_uri, metadata_key, create_string_value("b")).await;
    assert_ne!(etag_1, etag_2);

    let get_metadata_response = client
        .get_metadata(GetMetadataRequest {
            uri: resource_uri.clone(),
            ..Default::default()
        })
        .await
        .expect("get metadata failed");
    assert_eq!(get_metadata_response.user_metadata.len(), 1);

    client
        .delete_metadata(DeleteMetadataRequest {
            uri: resource_uri.clone(),
            user_metadata_key: metadata_key.into(),
            expected_etag: Some(etag_2),
        })
        .await
        .expect("delete metadata failed");

    let get_metadata_response = client
        .get_metadata(GetMetadataRequest {
            uri: resource_uri,
            ..Default::default()
        })
        .await
        .expect("get metadata failed");
    assert_eq!(get_metadata_response, GetMetadataResponse::default());
}

#[rstest]
#[case(Backend::Postgres, Interface::Grpc)]
#[case(Backend::Postgres, Interface::Rest)]
#[cfg_attr(feature = "dynamodb", case(Backend::DynamoDB, Interface::Grpc))]
#[cfg_attr(feature = "cosmosdb", case(Backend::CosmosDB, Interface::Grpc))]
#[test_log::test(tokio::test)]
async fn delete_metadata_conditionally_returns_failed_precondition_on_invalid_etag(
    test_services: &TestServices,
    #[case] backend: Backend,
    #[case] interface: Interface,
) {
    let client = test_services.metadata_v1alpha_client(backend, interface).await;
    let resource_uri = test_services.url_builder(backend).path(get_random_path()).to_string();
    let metadata_key = "key";

    let etag_1 = update_metadata_entry(&client, &resource_uri, metadata_key, create_string_value("a")).await;
    let etag_2 = update_metadata_entry(&client, &resource_uri, metadata_key, create_string_value("b")).await;
    assert_ne!(etag_1, etag_2);

    let delete_metadata_status = client
        .delete_metadata(DeleteMetadataRequest {
            uri: resource_uri,
            user_metadata_key: metadata_key.into(),
            expected_etag: Some(etag_1),
        })
        .await
        .expect_err("delete metadata did not fail");
    assert_eq!(delete_metadata_status.code(), Code::FailedPrecondition);
}

#[rstest]
#[case("suffix", None, "su%66%66ix", None, "s%75ffix", None, "k/e/y")]
#[case("su%66%66ix", Some("etags"), "suffix", None, "s%75ffix", None, "k@e@y")]
#[case("su%66%66ix", None, "suffix", Some("query"), "s%75ffix", None, "k-e-y")]
#[case("suffix", None, "su%66%66ix", None, "s%75ffix", Some("query"), "k e y")]
#[case(
    "suffix",
    Some("etag=a"),
    "su%66%66ix",
    Some("etag=%61"),
    "s%75ffix",
    Some("query&etag=a"),
    "k?e?y"
)]
#[case(
    "suffix",
    Some("etag=a&query"),
    "su%66%66ix",
    Some("etag=%61&query"),
    "s%75ffix",
    Some("etag=a"),
    "k%20e%20y"
)]
#[test_log::test(tokio::test)]
async fn test_set_get_delete_metadata_uses_consistent_parameter_encoding(
    test_services: &TestServices,
    #[values(Backend::Postgres)] backend: Backend,
    #[case] update_path_suffix: &str,
    #[case] update_query: Option<&str>,
    #[case] get_path_suffix: &str,
    #[case] get_query: Option<&str>,
    #[case] delete_path_suffix: &str,
    #[case] delete_query: Option<&str>,
    #[case] user_metadata_key: &str,
) {
    let client = test_services.metadata_v1alpha_client(backend, Interface::Grpc).await;
    let folder_path = get_random_path();
    let update_resource_uri = test_services
        .url_builder(backend)
        .raw_path(format!("{}/{}", folder_path, update_path_suffix))
        .maybe_raw_query(update_query)
        .to_string();
    let get_resource_uri = test_services
        .url_builder(backend)
        .raw_path(format!("{}/{}", folder_path, get_path_suffix))
        .maybe_raw_query(get_query)
        .to_string();
    let delete_resource_uri = test_services
        .url_builder(backend)
        .raw_path(format!("{}/{}", folder_path, delete_path_suffix))
        .maybe_raw_query(delete_query)
        .to_string();
    let user_metadata_value = create_string_value("some-value");

    // Create a piece of metadata.
    let update_response = client
        .update_metadata(UpdateMetadataRequest {
            uri: update_resource_uri,
            user_metadata_key: user_metadata_key.into(),
            user_metadata: Some(user_metadata_value.clone()),
            ..Default::default()
        })
        .await
        .expect("metadata update failed");

    // Fetch the piece of metadata. The operation must succeed.
    let get_response = client
        .get_metadata(GetMetadataRequest {
            uri: get_resource_uri.clone(),
            ..Default::default()
        })
        .await
        .expect("metadata fetch failed");

    assert_eq!(
        get_response,
        GetMetadataResponse {
            user_metadata: HashMap::from([(
                user_metadata_key.into(),
                UserMetadataValue {
                    etag: update_response.etag,
                    value: Some(user_metadata_value),
                },
            )]),
        }
    );

    // Delete the piece of metadata. Fetching metadata after that must return an empty set.
    client
        .delete_metadata(DeleteMetadataRequest {
            uri: delete_resource_uri,
            user_metadata_key: user_metadata_key.into(),
            ..Default::default()
        })
        .await
        .expect("metadata delete failed");

    let get_response = client
        .get_metadata(GetMetadataRequest {
            uri: get_resource_uri,
            ..Default::default()
        })
        .await
        .expect("metadata fetch failed");

    assert_eq!(get_response, GetMetadataResponse::default());
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_get_metadata_returns_invalid_argument_on_unsupported_uri(
    pg_test_service: &StorageService,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    let client = pg_test_service.metadata_v1alpha_client(interface).await;

    let error = client
        .get_metadata(GetMetadataRequest {
            uri: "https://myhost.mydomain/object".into(),
            ..Default::default()
        })
        .await
        .expect_err("get metadata is expected to fail");

    assert_eq!(error.code(), Code::InvalidArgument);
}

fn create_string_value(value: impl Into<String>) -> google::protobuf::Value {
    google::protobuf::Value {
        kind: Some(google::protobuf::value::Kind::StringValue(value.into())),
    }
}

async fn update_metadata_entry(
    client: &Client,
    uri: impl Into<String>,
    key: impl Into<String>,
    value: impl Into<google::protobuf::Value>,
) -> String {
    client
        .update_metadata(UpdateMetadataRequest {
            uri: uri.into(),
            user_metadata_key: key.into(),
            user_metadata: Some(value.into()),
            expected_etag: None,
        })
        .await
        .expect("update metadata failed")
        .etag
}
