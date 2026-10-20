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

use assert_matches::assert_matches;
use rstest::{fixture, rstest};
use storage_service_protos::google;

use crate::common::clients::metadata::types::v1beta::*;
use crate::common::clients::metadata::v1beta::Client;
use crate::common::random::get_random_path;
use crate::common::service::Service as StorageService;
use crate::common::types::Interface;
use crate::common::url::UrlBuilder;

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
    async fn metadata_v1beta_client(&self, backend: Backend, interface: Interface) -> Client {
        match backend {
            #[cfg(feature = "dynamodb")]
            Backend::DynamoDB => self.dynamodb_storage_service.metadata_v1beta_client(interface).await,
            #[cfg(feature = "cosmosdb")]
            Backend::CosmosDB => self.cosmosdb_storage_service.metadata_v1beta_client(interface).await,
            Backend::Postgres => self.postgres_storage_service.metadata_v1beta_client(interface).await,
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

#[rstest]
#[case(Backend::Postgres, Interface::Grpc)]
#[case(Backend::Postgres, Interface::Rest)]
#[cfg_attr(feature = "dynamodb", case(Backend::DynamoDB, Interface::Grpc))]
#[cfg_attr(feature = "cosmosdb", case(Backend::CosmosDB, Interface::Grpc))]
#[test_log::test(tokio::test)]
async fn test_set_get_metadata(test_services: &TestServices, #[case] backend: Backend, #[case] interface: Interface) {
    use get_metadata_batch_response_entry::Outcome;

    let client = test_services.metadata_v1beta_client(backend, interface).await;
    let uri = test_services.url_builder(backend).path(get_random_path()).to_string();

    let value_a = string_value("value-a");
    let value_b = string_value("value-b");
    let etag_a = client
        .set_metadata_value(location(&uri), "key-a", value_a.clone())
        .await
        .expect("set key-a failed");
    let etag_b = client
        .set_metadata_value(location(&uri), "key-b", value_b.clone())
        .await
        .expect("set key-b failed");

    let response = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![get_entry(&uri, "key-a"), get_entry(&uri, "key-b")],
        })
        .await
        .expect("get metadata batch failed");

    assert_matches!(
        &response.results[0].outcome,
        Some(Outcome::Success(entry)) if entry.value == Some(value_a) && entry.etag == etag_a
    );
    assert_matches!(
        &response.results[1].outcome,
        Some(Outcome::Success(entry)) if entry.value == Some(value_b) && entry.etag == etag_b
    );
}

#[rstest]
#[case(Backend::Postgres, Interface::Grpc)]
#[case(Backend::Postgres, Interface::Rest)]
#[cfg_attr(feature = "dynamodb", case(Backend::DynamoDB, Interface::Grpc))]
#[cfg_attr(feature = "cosmosdb", case(Backend::CosmosDB, Interface::Grpc))]
#[test_log::test(tokio::test)]
async fn test_query_metadata_returns_all_entries(
    test_services: &TestServices,
    #[case] backend: Backend,
    #[case] interface: Interface,
) {
    let client = test_services.metadata_v1beta_client(backend, interface).await;
    let uri = test_services.url_builder(backend).path(get_random_path()).to_string();

    client
        .set_metadata_value(location(&uri), "key-a", string_value("value-a"))
        .await
        .expect("set key-a failed");
    client
        .set_metadata_value(location(&uri), "key-b", string_value("value-b"))
        .await
        .expect("set key-b failed");

    let response = client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(location(&uri)),
            namespace: None,
        })
        .await
        .expect("query metadata failed");

    let keys: Vec<&str> = response
        .results
        .iter()
        .map(|entry| entry.user_metadata_key.as_str())
        .collect();
    assert_eq!(keys, ["key-a", "key-b"]);
}

#[rstest]
#[case(Backend::Postgres, Interface::Grpc)]
#[case(Backend::Postgres, Interface::Rest)]
#[cfg_attr(feature = "dynamodb", case(Backend::DynamoDB, Interface::Grpc))]
#[cfg_attr(feature = "cosmosdb", case(Backend::CosmosDB, Interface::Grpc))]
#[test_log::test(tokio::test)]
async fn test_get_nonexistent_metadata(
    test_services: &TestServices,
    #[case] backend: Backend,
    #[case] interface: Interface,
) {
    use get_metadata_batch_response_entry::Outcome;

    let client = test_services.metadata_v1beta_client(backend, interface).await;
    let uri = test_services.url_builder(backend).path(get_random_path()).to_string();

    let response = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![get_entry(&uri, "key")],
        })
        .await
        .expect("get metadata batch failed");

    assert_matches!(
        &response.results[0].outcome,
        Some(Outcome::Error(error)) if error.code == batch_error_entry::Code::NotFound as i32
    );
}

#[rstest]
#[case(Backend::Postgres, Interface::Grpc)]
#[case(Backend::Postgres, Interface::Rest)]
#[cfg_attr(feature = "dynamodb", case(Backend::DynamoDB, Interface::Grpc))]
#[cfg_attr(feature = "cosmosdb", case(Backend::CosmosDB, Interface::Grpc))]
#[test_log::test(tokio::test)]
async fn test_get_metadata_after_set_retrieves_the_latest_value(
    test_services: &TestServices,
    #[case] backend: Backend,
    #[case] interface: Interface,
) {
    use get_metadata_batch_response_entry::Outcome;

    let client = test_services.metadata_v1beta_client(backend, interface).await;
    let uri = test_services.url_builder(backend).path(get_random_path()).to_string();
    let latest = string_value("value-2");

    client
        .set_metadata_value(location(&uri), "key", string_value("value-1"))
        .await
        .expect("first set failed");
    let etag = client
        .set_metadata_value(location(&uri), "key", latest.clone())
        .await
        .expect("second set failed");

    let response = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![get_entry(&uri, "key")],
        })
        .await
        .expect("get metadata batch failed");

    assert_matches!(
        &response.results[0].outcome,
        Some(Outcome::Success(entry)) if entry.value == Some(latest) && entry.etag == etag
    );
}

#[rstest]
#[case(Backend::Postgres, Interface::Grpc)]
#[case(Backend::Postgres, Interface::Rest)]
#[cfg_attr(feature = "dynamodb", case(Backend::DynamoDB, Interface::Grpc))]
#[cfg_attr(feature = "cosmosdb", case(Backend::CosmosDB, Interface::Grpc))]
#[test_log::test(tokio::test)]
async fn test_set_metadata_conditionally(
    test_services: &TestServices,
    #[case] backend: Backend,
    #[case] interface: Interface,
) {
    use get_metadata_batch_response_entry::Outcome;

    let client = test_services.metadata_v1beta_client(backend, interface).await;
    let uri = test_services.url_builder(backend).path(get_random_path()).to_string();
    let final_value = string_value("value-3");

    let etag_1 = client
        .set_metadata_value(location(&uri), "key", string_value("a"))
        .await
        .expect("first set failed");
    let etag_2 = client
        .set_metadata_value(location(&uri), "key", string_value("b"))
        .await
        .expect("second set failed");
    assert_ne!(etag_1, etag_2);

    let response = client
        .update_metadata_batch_sorted(UpdateMetadataBatchRequest {
            requests: vec![UpdateMetadataBatchRequestEntry {
                resource_location: Some(location(&uri)),
                user_metadata_key: "key".into(),
                user_metadata: Some(final_value.clone()),
                expected_etag: Some(etag_2),
                namespace: None,
            }],
        })
        .await
        .expect("conditional update failed");

    let final_etag = assert_matches!(
        &response.results[0].outcome,
        Some(update_metadata_batch_response_entry::Outcome::Success(success)) => success.etag.clone()
    );

    let get = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![get_entry(&uri, "key")],
        })
        .await
        .expect("get metadata batch failed");

    assert_matches!(
        &get.results[0].outcome,
        Some(Outcome::Success(entry)) if entry.value == Some(final_value) && entry.etag == final_etag
    );
}

#[rstest]
#[case(Backend::Postgres, Interface::Grpc)]
#[case(Backend::Postgres, Interface::Rest)]
#[cfg_attr(feature = "dynamodb", case(Backend::DynamoDB, Interface::Grpc))]
#[cfg_attr(feature = "cosmosdb", case(Backend::CosmosDB, Interface::Grpc))]
#[test_log::test(tokio::test)]
async fn test_set_metadata_conditionally_returns_failed_precondition_on_invalid_etag(
    test_services: &TestServices,
    #[case] backend: Backend,
    #[case] interface: Interface,
) {
    let client = test_services.metadata_v1beta_client(backend, interface).await;
    let uri = test_services.url_builder(backend).path(get_random_path()).to_string();

    let etag_1 = client
        .set_metadata_value(location(&uri), "key", string_value("a"))
        .await
        .expect("first set failed");
    let etag_2 = client
        .set_metadata_value(location(&uri), "key", string_value("b"))
        .await
        .expect("second set failed");
    assert_ne!(etag_1, etag_2);

    let response = client
        .update_metadata_batch_sorted(UpdateMetadataBatchRequest {
            requests: vec![UpdateMetadataBatchRequestEntry {
                resource_location: Some(location(&uri)),
                user_metadata_key: "key".into(),
                user_metadata: Some(string_value("c")),
                expected_etag: Some(etag_1),
                namespace: None,
            }],
        })
        .await
        .expect("conditional update failed");

    assert_matches!(
        &response.results[0].outcome,
        Some(update_metadata_batch_response_entry::Outcome::Error(error))
            if error.code == batch_error_entry::Code::FailedPrecondition as i32
    );
}

#[rstest]
#[case(Backend::Postgres, Interface::Grpc)]
#[case(Backend::Postgres, Interface::Rest)]
#[cfg_attr(feature = "dynamodb", case(Backend::DynamoDB, Interface::Grpc))]
#[cfg_attr(feature = "cosmosdb", case(Backend::CosmosDB, Interface::Grpc))]
#[test_log::test(tokio::test)]
async fn test_delete_metadata(test_services: &TestServices, #[case] backend: Backend, #[case] interface: Interface) {
    let client = test_services.metadata_v1beta_client(backend, interface).await;
    let uri = test_services.url_builder(backend).path(get_random_path()).to_string();

    client
        .set_metadata_value(location(&uri), "key", string_value("some-value"))
        .await
        .expect("set failed");

    let delete = client
        .delete_metadata_batch_sorted(DeleteMetadataBatchRequest {
            requests: vec![delete_entry(&uri, "key", None)],
        })
        .await
        .expect("delete metadata batch failed");
    assert_matches!(
        delete.results[0].outcome,
        Some(delete_metadata_batch_response_entry::Outcome::Success(_))
    );

    let get = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![get_entry(&uri, "key")],
        })
        .await
        .expect("get metadata batch failed");
    assert_matches!(
        &get.results[0].outcome,
        Some(get_metadata_batch_response_entry::Outcome::Error(error))
            if error.code == batch_error_entry::Code::NotFound as i32
    );
}

#[rstest]
#[case(Backend::Postgres, Interface::Grpc)]
#[case(Backend::Postgres, Interface::Rest)]
#[cfg_attr(feature = "dynamodb", case(Backend::DynamoDB, Interface::Grpc))]
#[cfg_attr(feature = "cosmosdb", case(Backend::CosmosDB, Interface::Grpc))]
#[test_log::test(tokio::test)]
async fn test_delete_nonexistent_metadata(
    test_services: &TestServices,
    #[case] backend: Backend,
    #[case] interface: Interface,
) {
    let client = test_services.metadata_v1beta_client(backend, interface).await;
    let uri = test_services.url_builder(backend).path(get_random_path()).to_string();

    let response = client
        .delete_metadata_batch_sorted(DeleteMetadataBatchRequest {
            requests: vec![delete_entry(&uri, "key", None)],
        })
        .await
        .expect("delete metadata batch failed");

    assert_matches!(
        response.results[0].outcome,
        Some(delete_metadata_batch_response_entry::Outcome::Success(_))
    );
}

#[rstest]
#[case(Backend::Postgres, Interface::Grpc)]
#[case(Backend::Postgres, Interface::Rest)]
#[cfg_attr(feature = "dynamodb", case(Backend::DynamoDB, Interface::Grpc))]
#[cfg_attr(feature = "cosmosdb", case(Backend::CosmosDB, Interface::Grpc))]
#[test_log::test(tokio::test)]
async fn test_delete_metadata_conditionally(
    test_services: &TestServices,
    #[case] backend: Backend,
    #[case] interface: Interface,
) {
    let client = test_services.metadata_v1beta_client(backend, interface).await;
    let uri = test_services.url_builder(backend).path(get_random_path()).to_string();

    let etag = client
        .set_metadata_value(location(&uri), "key", string_value("a"))
        .await
        .expect("set failed");

    let delete = client
        .delete_metadata_batch_sorted(DeleteMetadataBatchRequest {
            requests: vec![delete_entry(&uri, "key", Some(etag))],
        })
        .await
        .expect("delete metadata batch failed");
    assert_matches!(
        delete.results[0].outcome,
        Some(delete_metadata_batch_response_entry::Outcome::Success(_))
    );

    let get = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![get_entry(&uri, "key")],
        })
        .await
        .expect("get metadata batch failed");
    assert_matches!(
        &get.results[0].outcome,
        Some(get_metadata_batch_response_entry::Outcome::Error(error))
            if error.code == batch_error_entry::Code::NotFound as i32
    );
}

#[rstest]
#[case(Backend::Postgres, Interface::Grpc)]
#[case(Backend::Postgres, Interface::Rest)]
#[cfg_attr(feature = "dynamodb", case(Backend::DynamoDB, Interface::Grpc))]
#[cfg_attr(feature = "cosmosdb", case(Backend::CosmosDB, Interface::Grpc))]
#[test_log::test(tokio::test)]
async fn test_delete_metadata_conditionally_returns_failed_precondition_on_invalid_etag(
    test_services: &TestServices,
    #[case] backend: Backend,
    #[case] interface: Interface,
) {
    let client = test_services.metadata_v1beta_client(backend, interface).await;
    let uri = test_services.url_builder(backend).path(get_random_path()).to_string();

    let etag_1 = client
        .set_metadata_value(location(&uri), "key", string_value("a"))
        .await
        .expect("first set failed");
    let etag_2 = client
        .set_metadata_value(location(&uri), "key", string_value("b"))
        .await
        .expect("second set failed");
    assert_ne!(etag_1, etag_2);

    let response = client
        .delete_metadata_batch_sorted(DeleteMetadataBatchRequest {
            requests: vec![delete_entry(&uri, "key", Some(etag_1))],
        })
        .await
        .expect("delete metadata batch failed");

    assert_matches!(
        &response.results[0].outcome,
        Some(delete_metadata_batch_response_entry::Outcome::Error(error))
            if error.code == batch_error_entry::Code::FailedPrecondition as i32
    );
}

#[rstest]
#[case::grpc(Interface::Grpc)]
#[case::rest(Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_set_then_get_round_trip(test_services: &TestServices, #[case] interface: Interface) {
    use get_metadata_batch_response_entry::Outcome;

    let client = test_services.metadata_v1beta_client(Backend::Postgres, interface).await;
    let uri = test_services
        .url_builder(Backend::Postgres)
        .path(get_random_path())
        .to_string();
    let value = string_value("round-trip");

    let etag = client
        .set_metadata_value(location(&uri), "key", value.clone())
        .await
        .expect("set failed");

    let get = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![get_entry(&uri, "key")],
        })
        .await
        .expect("get metadata batch failed");

    assert_matches!(
        &get.results[0].outcome,
        Some(Outcome::Success(entry)) if entry.value == Some(value) && entry.etag == etag
    );
}

#[rstest]
#[case::grpc(Interface::Grpc)]
#[case::rest(Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_update_batch_all_success(test_services: &TestServices, #[case] interface: Interface) {
    use update_metadata_batch_response_entry::Outcome;

    let client = test_services.metadata_v1beta_client(Backend::Postgres, interface).await;
    let uri = test_services
        .url_builder(Backend::Postgres)
        .path(get_random_path())
        .to_string();

    let response = client
        .update_metadata_batch_sorted(UpdateMetadataBatchRequest {
            requests: vec![
                update_entry(&uri, "a", string_value("1"), None),
                update_entry(&uri, "b", string_value("2"), None),
            ],
        })
        .await
        .expect("update metadata batch failed");

    assert_matches!(response.results[0].outcome, Some(Outcome::Success(_)));
    assert_matches!(response.results[1].outcome, Some(Outcome::Success(_)));
}

#[rstest]
#[case::grpc(Interface::Grpc)]
#[case::rest(Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_update_batch_all_error(test_services: &TestServices, #[case] interface: Interface) {
    use update_metadata_batch_response_entry::Outcome;

    let client = test_services.metadata_v1beta_client(Backend::Postgres, interface).await;
    let uri = test_services
        .url_builder(Backend::Postgres)
        .path(get_random_path())
        .to_string();

    let response = client
        .update_metadata_batch_sorted(UpdateMetadataBatchRequest {
            requests: vec![
                update_entry(&uri, "a", string_value("1"), Some("does-not-match".into())),
                update_entry(&uri, "b", string_value("2"), Some("does-not-match".into())),
            ],
        })
        .await
        .expect("update metadata batch failed");

    assert_matches!(&response.results[0].outcome, Some(Outcome::Error(_)));
    assert_matches!(&response.results[1].outcome, Some(Outcome::Error(_)));
}

#[rstest]
#[case::grpc(Interface::Grpc)]
#[case::rest(Interface::Rest)]
#[test_log::test(tokio::test)]
async fn test_update_batch_mixed(test_services: &TestServices, #[case] interface: Interface) {
    use update_metadata_batch_response_entry::Outcome;

    let client = test_services.metadata_v1beta_client(Backend::Postgres, interface).await;
    let uri = test_services
        .url_builder(Backend::Postgres)
        .path(get_random_path())
        .to_string();

    let response = client
        .update_metadata_batch_sorted(UpdateMetadataBatchRequest {
            requests: vec![
                update_entry(&uri, "ok", string_value("1"), None),
                update_entry(&uri, "bad", string_value("2"), Some("does-not-match".into())),
            ],
        })
        .await
        .expect("update metadata batch failed");

    assert_matches!(response.results[0].outcome, Some(Outcome::Success(_)));
    assert_matches!(&response.results[1].outcome, Some(Outcome::Error(_)));
}

fn location(uri: impl Into<String>) -> ResourceLocation {
    ResourceLocation::resource_address(uri)
}

fn string_value(value: impl Into<String>) -> google::protobuf::Value {
    google::protobuf::Value {
        kind: Some(google::protobuf::value::Kind::StringValue(value.into())),
    }
}

fn get_entry(uri: &str, key: impl Into<String>) -> GetMetadataBatchRequestEntry {
    GetMetadataBatchRequestEntry {
        resource_location: Some(location(uri)),
        user_metadata_key: key.into(),
        namespace: None,
    }
}

fn update_entry(
    uri: &str,
    key: impl Into<String>,
    value: google::protobuf::Value,
    expected_etag: Option<String>,
) -> UpdateMetadataBatchRequestEntry {
    UpdateMetadataBatchRequestEntry {
        resource_location: Some(location(uri)),
        user_metadata_key: key.into(),
        user_metadata: Some(value),
        expected_etag,
        namespace: None,
    }
}

fn delete_entry(uri: &str, key: impl Into<String>, expected_etag: Option<String>) -> DeleteMetadataBatchRequestEntry {
    DeleteMetadataBatchRequestEntry {
        resource_location: Some(location(uri)),
        user_metadata_key: key.into(),
        expected_etag,
        namespace: None,
    }
}
