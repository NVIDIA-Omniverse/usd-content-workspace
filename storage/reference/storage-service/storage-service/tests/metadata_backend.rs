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

use std::time::Duration;

use crate::common::{
    clients::metadata::types::v1alpha::{
        BatchErrorEntry, DeleteMetadataBatchRequestEntry, DeleteMetadataBatchResponseEntry,
        GetMetadataBatchRequestEntry, GetMetadataBatchResponseEntry, QueryMetadataResponseEntry, ResourceLocation,
        UpdateMetadataBatchRequestEntry, UserMetadataValue,
    },
    env::get_env,
    metrics::PostgresMetadataBackendMetrics,
    random::{get_random_path, get_random_path_item},
    service::Service,
    types::Interface,
};
use assert_matches::assert_matches;
use rstest::{fixture, rstest};
use serial_test::serial;
use storage_service_protos::google::protobuf::Value;

mod common;

const S3_BUCKET_URL_A: &str = "https://bucket-a.s3.us-west-1.amazonaws.com";
const S3_BUCKET_URL_B: &str = "https://bucket-b.s3.us-west-1.amazonaws.com";

struct RouteTestServiceSet {
    pg_aa_service: Service,
    pg_ab_service: Service,
}

struct RedisCacheServiceSet {
    rd_service: Service,
    wr_service: Service,
}

#[fixture]
#[once]
fn postgres_metadata_service() -> Service {
    Service::builder().enable_s3(true).enable_postgres_metadata().start()
}

#[fixture]
#[once]
fn postgres_cache_metadata_service() -> Service {
    Service::builder()
        .enable_s3(true)
        .enable_postgres_metadata()
        .enable_metadata_cache(false)
        .start()
}

#[fixture]
#[once]
fn canon_metadata_service() -> Service {
    Service::builder().enable_s3(true).enable_postgres_metadata().start()
}

#[fixture]
#[once]
fn route_metadata_services() -> RouteTestServiceSet {
    use crate::common::env::get_env;
    use serde_json::json;

    let pg_env = get_env().unwrap_postgres();

    pg_env.create_schema("route_a");
    pg_env.create_schema("route_b");

    let pg_ab_service = Service::start_with_config(json!({
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
                    S3_BUCKET_URL_A: {
                        "credentials": {
                            "anonymous": true
                        },
                        "metadata_backend": "metadata-a",
                    },
                    S3_BUCKET_URL_B: {
                        "credentials": {
                            "anonymous": true
                        },
                        "metadata_backend": "metadata-b",
                    },
                }
            }
        },
        "metadata": {
            "metadata-a": {
                "postgres": {
                    "host": pg_env.host,
                    "port": pg_env.port,
                    "dbname": pg_env.dbname,
                    "schema": "route_a",
                    "credentials": {
                        "password": {
                            "user": pg_env.user,
                            "password": pg_env.password,
                        },
                    },
                },
            },
            "metadata-b": {
                "postgres": {
                    "host": pg_env.host,
                    "port": pg_env.port,
                    "dbname": pg_env.dbname,
                    "schema": "route_b",
                    "credentials": {
                        "password": {
                            "user": pg_env.user,
                            "password": pg_env.password,
                        },
                    },
                },
            },
        },
    }));

    let pg_aa_service = Service::start_with_config(json!({
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
                    S3_BUCKET_URL_A: {
                        "credentials": {
                            "anonymous": true
                        },
                        "metadata_backend": "metadata-a",
                    },
                    S3_BUCKET_URL_B: {
                        "credentials": {
                            "anonymous": true,
                        },
                        "metadata_backend": "metadata-a",
                    },
                },
            },
        },
        "metadata": {
            "metadata-a": {
                "postgres": {
                    "host": pg_env.host,
                    "port": pg_env.port,
                    "dbname": pg_env.dbname,
                    "schema": "route_a",
                    "credentials": {
                        "password": {
                            "user": pg_env.user,
                            "password": pg_env.password,
                        },
                    },
                },
            },
        },
    }));

    RouteTestServiceSet {
        pg_aa_service,
        pg_ab_service,
    }
}

#[fixture]
#[once]
fn postgres_redis_metadata_service() -> RedisCacheServiceSet {
    let create_service = || {
        Service::builder()
            .enable_s3(true)
            .enable_redis()
            .enable_postgres_metadata()
            .enable_metadata_cache(true)
            .start()
    };

    RedisCacheServiceSet {
        rd_service: create_service(),
        wr_service: create_service(),
    }
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_router_set_visible_across_replicas_with_smlr_cfg(route_metadata_services: &RouteTestServiceSet) {
    use common::clients::metadata::types::v1alpha::*;

    let resource_uri = format!("{S3_BUCKET_URL_A}/{}", get_random_path_item());

    let client_pg_aa = route_metadata_services
        .pg_aa_service
        .metadata_v1alpha_client(Interface::Grpc)
        .await;
    let client_pg_ab = route_metadata_services
        .pg_ab_service
        .metadata_v1alpha_client(Interface::Grpc)
        .await;

    // Set metadata for the resource uri through the first service. It must be visible via the second service.
    let value = create_string_value("value-a");
    let update_response = client_pg_aa
        .update_metadata(UpdateMetadataRequest {
            uri: resource_uri.clone(),
            user_metadata_key: "key-a".into(),
            user_metadata: Some(value.clone()),
            ..Default::default()
        })
        .await
        .expect("metadata update failed");

    let get_response = client_pg_ab
        .get_metadata(GetMetadataRequest {
            uri: resource_uri,
            ..Default::default()
        })
        .await
        .expect("metadata query failed");

    assert_eq!(
        get_response,
        GetMetadataResponse {
            user_metadata: [(
                "key-a".into(),
                UserMetadataValue {
                    etag: update_response.etag,
                    value: Some(value),
                }
            )]
            .into()
        }
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_router_set_invisible_across_replicas_with_dfnt_cfg(route_metadata_services: &RouteTestServiceSet) {
    use common::clients::metadata::types::v1alpha::{GetMetadataRequest, GetMetadataResponse, UpdateMetadataRequest};

    let resource_uri = format!("{S3_BUCKET_URL_B}/{}", get_random_path_item());

    let client_pg_aa = route_metadata_services
        .pg_aa_service
        .metadata_v1alpha_client(Interface::Grpc)
        .await;
    let client_pg_ab = route_metadata_services
        .pg_ab_service
        .metadata_v1alpha_client(Interface::Grpc)
        .await;

    // Set metadata for the resource uri through the first service. The second service must not see it.
    let value = create_string_value("value-a");
    client_pg_aa
        .update_metadata(UpdateMetadataRequest {
            uri: resource_uri.clone(),
            user_metadata_key: "key-a".into(),
            user_metadata: Some(value.clone()),
            ..Default::default()
        })
        .await
        .expect("metadata update failed");

    let get_response = client_pg_ab
        .get_metadata(GetMetadataRequest {
            uri: resource_uri,
            ..Default::default()
        })
        .await
        .expect("metadata query failed");

    assert_eq!(get_response, GetMetadataResponse::default());
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_router_delete_visible_across_replicas_with_smlr_cfg(route_metadata_services: &RouteTestServiceSet) {
    use common::clients::metadata::types::v1alpha::*;

    let resource_uri = format!("{S3_BUCKET_URL_A}/{}", get_random_path_item());

    let client_pg_aa = route_metadata_services
        .pg_aa_service
        .metadata_v1alpha_client(Interface::Grpc)
        .await;
    let client_pg_ab = route_metadata_services
        .pg_ab_service
        .metadata_v1alpha_client(Interface::Grpc)
        .await;

    // Set metadata for the resource uri through the first service.
    let value = create_string_value("value-a");
    let update_response = client_pg_aa
        .update_metadata(UpdateMetadataRequest {
            uri: resource_uri.clone(),
            user_metadata_key: "key-a".into(),
            user_metadata: Some(value.clone()),
            ..Default::default()
        })
        .await
        .expect("metadata update failed");

    // Delete the value through the second service.
    client_pg_ab
        .delete_metadata(DeleteMetadataRequest {
            uri: resource_uri.clone(),
            user_metadata_key: "key-a".into(),
            expected_etag: Some(update_response.etag),
        })
        .await
        .expect("metadata delete failed");

    // Read the value through the first service. We must see nothing.
    let get_response = client_pg_aa
        .get_metadata(GetMetadataRequest {
            uri: resource_uri,
            ..Default::default()
        })
        .await
        .expect("metadata query failed");

    assert_eq!(get_response, GetMetadataResponse::default());
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_router_delete_invisible_across_replicas_with_dfnt_cfg(route_metadata_services: &RouteTestServiceSet) {
    use common::clients::metadata::types::v1alpha::*;

    let resource_uri = format!("{S3_BUCKET_URL_B}/{}", get_random_path_item());

    let client_pg_aa = route_metadata_services
        .pg_aa_service
        .metadata_v1alpha_client(Interface::Grpc)
        .await;
    let client_pg_ab = route_metadata_services
        .pg_ab_service
        .metadata_v1alpha_client(Interface::Grpc)
        .await;

    // Set metadata for the resource uri through the first service.
    let value = create_string_value("value-a");
    let update_response = client_pg_aa
        .update_metadata(UpdateMetadataRequest {
            uri: resource_uri.clone(),
            user_metadata_key: "key-a".into(),
            user_metadata: Some(value.clone()),
            ..Default::default()
        })
        .await
        .expect("metadata update failed");

    // Delete the value through the second service.
    client_pg_ab
        .delete_metadata(DeleteMetadataRequest {
            uri: resource_uri.clone(),
            user_metadata_key: "key-a".into(),
            ..Default::default()
        })
        .await
        .expect("metadata delete failed");

    // Read the value through the first service. The delete must have no effect.
    let get_response = client_pg_aa
        .get_metadata(GetMetadataRequest {
            uri: resource_uri,
            ..Default::default()
        })
        .await
        .expect("metadata query failed");

    assert_eq!(
        get_response,
        GetMetadataResponse {
            user_metadata: [(
                "key-a".into(),
                UserMetadataValue {
                    etag: update_response.etag,
                    value: Some(value),
                },
            )]
            .into(),
        }
    );
}

#[rstest]
#[serial(postgres)]
#[test_log::test(tokio::test)]
async fn test_router_case_insensitive_url_components(postgres_metadata_service: &Service) {
    use common::clients::metadata::types::v1alpha::*;
    use common::env::get_env;

    let s3_env = get_env().unwrap_s3();

    let resource_path = get_random_path();
    let s3_host_lower = s3_env.host.to_ascii_lowercase();
    let s3_host_upper = s3_env.host.to_ascii_uppercase();
    let resource_uri_1 = format!(
        "http://{}:{}/{}/{}",
        s3_host_lower, s3_env.port, s3_env.bucket, resource_path,
    );
    let resource_uri_2 = format!(
        "HTTP://{}:{}/{}/{}",
        s3_host_upper, s3_env.port, s3_env.bucket, resource_path,
    );

    let client = postgres_metadata_service.metadata_v1alpha_client(Interface::Grpc).await;

    // Set metadata for resource uri 1.
    let value = create_string_value("value-1");
    let update_response = client
        .update_metadata(UpdateMetadataRequest {
            uri: resource_uri_1,
            user_metadata_key: "key-a".into(),
            user_metadata: Some(value.clone()),
            ..Default::default()
        })
        .await
        .expect("metadata update failed");

    // Get metadata for resource uri 2. It must be the same as we've just set.
    let get_response = client
        .get_metadata(GetMetadataRequest {
            uri: resource_uri_2,
            ..Default::default()
        })
        .await
        .expect("metadata query failed");

    assert_eq!(
        get_response,
        GetMetadataResponse {
            user_metadata: [(
                "key-a".into(),
                UserMetadataValue {
                    etag: update_response.etag,
                    value: Some(value),
                }
            )]
            .into()
        }
    );
}

#[test_log::test(tokio::test)]
async fn test_postgres_supports_access_token_credentials() {
    use serde_json::json;
    use tempfile::TempDir;

    use common::{clients::metadata::types::v1alpha::UpdateMetadataRequest, env::get_env, service::Service};

    let temp_dir = TempDir::new().expect("failed to create a temp dir");
    let access_token_file_path = temp_dir.path().join("token");

    let pg_env = get_env().unwrap_postgres();
    tokio::fs::write(
        &access_token_file_path,
        &json!({
            "access_token": pg_env.password,
            "expires_on": "2999-09-09T00:00:00Z",
        })
        .to_string(),
    )
    .await
    .expect("failed to create the token file");

    let service = Service::start_with_config(json!({
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
                    S3_BUCKET_URL_A: {
                        "credentials": {
                            "anonymous": true,
                        },
                        "metadata_backend": "metadata-a",
                    },
                },
            },
        },
        "metadata": {
            "metadata-a": {
                "postgres": {
                    "host": pg_env.host,
                    "port": pg_env.port,
                    "dbname": pg_env.dbname,
                    "cache_enabled": false,
                    "credentials": {
                        "access_token_file": {
                            "user": pg_env.user,
                            "path": access_token_file_path,
                        },
                    },
                },
            },
        },
    }));

    // Update a key-value. The request must succeed.
    service
        .metadata_v1alpha_client(Interface::Grpc)
        .await
        .update_metadata(UpdateMetadataRequest {
            uri: format!("{S3_BUCKET_URL_A}/{}", get_random_path()),
            user_metadata_key: "key-a".into(),
            user_metadata: Some(create_string_value("value-a")),
            ..Default::default()
        })
        .await
        .expect("metadata update failed");
}

#[rstest]
#[serial(postgres)]
#[test_log::test(tokio::test)]
async fn test_postgres_get_metadata_batch(postgres_metadata_service: &Service) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let resource_uri = create_resource_uri();
    let metrics = postgres_metadata_service.metrics_client();
    let client = postgres_metadata_service.metadata_v1alpha_client(Interface::Grpc).await;

    let (value_1, etag_1) = client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri), "a")
        .await
        .expect("metadata set failed");
    let (value_2, etag_2) = client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri), "b")
        .await
        .expect("metadata set failed");

    // Make a request. We must observe exactly one batch get query.
    let before_read = metrics.get_postgres_metadata_backend_metrics().await;
    let response = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                create_get_metadata_batch_resource_address_request_entry(&resource_uri, "a"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri, "b"),
            ],
        })
        .await
        .expect("get metadata failed");

    assert_eq!(
        response,
        GetMetadataBatchResponse {
            results: vec![
                create_get_metadata_batch_success_response_entry(0, value_1, etag_1),
                create_get_metadata_batch_success_response_entry(1, value_2, etag_2),
            ],
        },
    );

    let after_read = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_read,
        PostgresMetadataBackendMetrics {
            query_metadata_batch_count: before_read.query_metadata_batch_count + 1,
            ..before_read
        }
    );
}

#[rstest]
#[serial(postgres)]
#[test_log::test(tokio::test)]
async fn test_postgres_get_metadata_batch_partial_success(postgres_metadata_service: &Service) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let resource_uri = create_resource_uri();
    let metrics = postgres_metadata_service.metrics_client();
    let client = postgres_metadata_service.metadata_v1alpha_client(Interface::Grpc).await;

    let (value_1, etag_1) = client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri), "a")
        .await
        .expect("metadata set failed");
    let (value_2, etag_2) = client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri), "b")
        .await
        .expect("metadata set failed");

    // Make a request. We must observe exactly one batch get query.
    let before_read = metrics.get_postgres_metadata_backend_metrics().await;
    let response = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                create_get_metadata_batch_resource_address_request_entry(&resource_uri, "a"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri, "b"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri, "c"),
            ],
        })
        .await
        .expect("get metadata failed");

    assert_eq!(
        response,
        GetMetadataBatchResponse {
            results: vec![
                create_get_metadata_batch_success_response_entry(0, value_1, etag_1),
                create_get_metadata_batch_success_response_entry(1, value_2, etag_2),
                create_get_metadata_batch_not_found_response_entry(2),
            ],
        },
    );

    let after_read = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_read,
        PostgresMetadataBackendMetrics {
            query_metadata_batch_count: before_read.query_metadata_batch_count + 1,
            ..before_read
        }
    );
}

#[rstest]
#[serial(postgres)]
#[test_log::test(tokio::test)]
async fn test_postgres_get_metadata_batch_partial_success_multiple_queries(postgres_metadata_service: &Service) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let resource_uri = create_resource_uri();
    let metrics = postgres_metadata_service.metrics_client();
    let client = postgres_metadata_service.metadata_v1alpha_client(Interface::Grpc).await;

    let (value_1, etag_1) = client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri), "a")
        .await
        .expect("metadata set failed");
    let (value_2, etag_2) = client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri), "b")
        .await
        .expect("metadata set failed");

    // Make a request. We must observe two batch queries.
    let before_read = metrics.get_postgres_metadata_backend_metrics().await;
    let response = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                create_get_metadata_batch_resource_address_request_entry(&resource_uri, "0"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri, "a"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri, "1"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri, "b"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri, "2"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri, "c"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri, "3"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri, "d"),
            ],
        })
        .await
        .expect("get metadata failed");

    assert_eq!(
        response,
        GetMetadataBatchResponse {
            results: vec![
                create_get_metadata_batch_not_found_response_entry(0,),
                create_get_metadata_batch_success_response_entry(1, value_1, etag_1),
                create_get_metadata_batch_not_found_response_entry(2),
                create_get_metadata_batch_success_response_entry(3, value_2, etag_2),
                create_get_metadata_batch_not_found_response_entry(4),
                create_get_metadata_batch_not_found_response_entry(5),
                create_get_metadata_batch_not_found_response_entry(6),
                create_get_metadata_batch_not_found_response_entry(7),
            ],
        },
    );

    let after_read = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_read,
        PostgresMetadataBackendMetrics {
            query_metadata_batch_count: before_read.query_metadata_batch_count + 2,
            ..before_read
        }
    );
}

#[rstest]
#[case(vec!["a", "b", "c", "d"], 1)]
#[case(vec!["a", "b", "c", "d", "e"], 1)]
#[case(vec!["a", "b", "c", "d", "e", "f"], 2)]
#[serial(postgres)]
#[test_log::test(tokio::test)]
async fn test_postgres_query_metadata(
    postgres_metadata_service: &Service,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[case] keys: Vec<&str>,
    #[case] expected_queries: u64,
) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let resource_uri = create_resource_uri();
    let metrics = postgres_metadata_service.metrics_client();
    let client = postgres_metadata_service.metadata_v1alpha_client(interface).await;

    let values_etags = populate_metadata(
        &client,
        ResourceLocation::resource_address(&resource_uri),
        keys.iter().copied(),
    )
    .await;

    let before_query = metrics.get_postgres_metadata_backend_metrics().await;
    let response = client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
            ..Default::default()
        })
        .await
        .expect("metadata query failed");

    assert_eq!(
        response,
        QueryMetadataResponse {
            results: keys
                .into_iter()
                .zip(values_etags.into_iter())
                .map(|(key, (value, etag))| create_query_metadata_response_entry(key, value, etag))
                .collect(),
        }
    );

    let after_query = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_query,
        PostgresMetadataBackendMetrics {
            query_all_metadata_for_resource_count: before_query.query_all_metadata_for_resource_count
                + expected_queries,
            ..before_query
        }
    );
}

#[rstest]
#[case(vec!["a", "b", "c", "d"], 1)]
#[serial(postgres)]
#[test_log::test(tokio::test)]
async fn test_postgres_update_metadata_batch(
    postgres_metadata_service: &Service,
    #[case] keys: Vec<&str>,
    #[case] expected_queries: u64,
) {
    // TODO: add more test scenarios once we've added batch chunking.

    use crate::common::clients::metadata::types::v1alpha::{update_metadata_batch_response_entry::Outcome, *};

    let resource_uri = create_resource_uri();
    let metrics = postgres_metadata_service.metrics_client();
    let client = postgres_metadata_service.metadata_v1alpha_client(Interface::Grpc).await;

    let before_update = metrics.get_postgres_metadata_backend_metrics().await;
    let update_response = client
        .update_metadata_batch_sorted(UpdateMetadataBatchRequest {
            requests: keys
                .iter()
                .map(|key| {
                    create_update_metadata_batch_resource_address_request_entry(
                        &resource_uri,
                        *key,
                        create_string_value(*key),
                    )
                })
                .collect(),
        })
        .await
        .expect("metadata update failed");

    assert_eq!(update_response.results.len(), keys.len());
    assert!(update_response
        .results
        .iter()
        .all(|e| matches!(e.outcome, Some(Outcome::Success(ref e)) if !e.etag.is_empty())));

    let after_update = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_update,
        PostgresMetadataBackendMetrics {
            upsert_metadata_batch_count: before_update.upsert_metadata_batch_count + expected_queries,
            ..before_update
        }
    );

    let get_response = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: keys
                .iter()
                .map(|key| create_get_metadata_batch_resource_address_request_entry(&resource_uri, *key))
                .collect(),
        })
        .await
        .unwrap();

    assert_eq!(
        get_response,
        GetMetadataBatchResponse {
            results: keys
                .into_iter()
                .zip(update_response.results.into_iter())
                .map(|(key, response)| create_get_metadata_batch_success_response_entry(
                    response.request_index,
                    create_string_value(key),
                    &response.as_success().etag
                ))
                .collect()
        }
    );
}

#[rstest]
#[serial(postgres)]
#[test_log::test(tokio::test)]
async fn test_postgres_update_metadata_batch_reports_precondition_failure_errors(
    postgres_metadata_service: &Service,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::{
        batch_error_entry::Code, update_metadata_batch_response_entry::Outcome, *,
    };

    let client = postgres_metadata_service.metadata_v1alpha_client(interface).await;
    let resource_uri_1 = create_resource_uri();
    let resource_uri_2 = create_resource_uri();

    let (value_1, etag_1) = client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri_1), "b")
        .await
        .expect("metadata set failed");
    let (_value_2, etag_2) = client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri_2), "c")
        .await
        .expect("metadata set failed");

    let response = client
        .update_metadata_batch_sorted(UpdateMetadataBatchRequest {
            requests: vec![
                UpdateMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri_1)),
                    user_metadata_key: "a".into(),
                    user_metadata: Some(create_string_value("a")),
                    ..Default::default()
                },
                UpdateMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri_1)),
                    user_metadata_key: "b".into(),
                    user_metadata: Some(create_string_value("b")),
                    expected_etag: Some("etag".into()),
                    ..Default::default()
                },
                UpdateMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri_2)),
                    user_metadata_key: "c".into(),
                    user_metadata: Some(create_string_value("c")),
                    expected_etag: Some(etag_2),
                    ..Default::default()
                },
            ],
        })
        .await
        .expect("metadata update failed");

    assert_eq!(response.results.len(), 3);
    assert_matches!(response.results[0].outcome, Some(Outcome::Success(ref e)) if !e.etag.is_empty());
    assert_matches!(response.results[1].outcome, Some(Outcome::Error(ref e)) if e.code == Code::FailedPrecondition as i32);
    assert_matches!(response.results[2].outcome, Some(Outcome::Success(ref e)) if !e.etag.is_empty());

    // Read metadata back. The second entry must remain unchanged.
    let get_response = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_1, "a"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_1, "b"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_2, "c"),
            ],
        })
        .await
        .expect("metadata fetch faled");

    assert_eq!(
        get_response,
        GetMetadataBatchResponse {
            results: vec![
                create_get_metadata_batch_success_response_entry(
                    0,
                    create_string_value("a"),
                    &response.results[0].as_success().etag
                ),
                create_get_metadata_batch_success_response_entry(1, value_1, etag_1),
                create_get_metadata_batch_success_response_entry(
                    2,
                    create_string_value("c"),
                    &response.results[2].as_success().etag
                ),
            ],
        },
    );
}

#[rstest]
#[serial(postgres)]
#[test_log::test(tokio::test)]
async fn test_postgres_update_metadata_batch_rejects_duplicate_writes(
    postgres_metadata_service: &Service,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::{batch_error_entry::Code, *};

    let client = postgres_metadata_service.metadata_v1alpha_client(interface).await;
    let resource_uri_1 = create_resource_uri();
    let resource_uri_2 = create_resource_uri();

    let response = client
        .update_metadata_batch_sorted(UpdateMetadataBatchRequest {
            requests: vec![
                create_update_metadata_batch_resource_address_request_entry(
                    &resource_uri_1,
                    "a",
                    create_string_value("0"),
                ),
                create_update_metadata_batch_resource_address_request_entry(
                    &resource_uri_2,
                    "b",
                    create_string_value("b"),
                ),
                create_update_metadata_batch_resource_address_request_entry(
                    &resource_uri_1,
                    "a",
                    create_string_value("1"),
                ),
                create_update_metadata_batch_resource_address_request_entry(
                    &resource_uri_1,
                    "b",
                    create_string_value("x"),
                ),
            ],
        })
        .await
        .expect("metadata update failed");

    assert_eq!(response.results.len(), 4);
    assert_eq!(response.results[0].as_error().code, Code::InvalidArgument as i32);
    assert!(!response.results[1].as_success().etag.is_empty());
    assert_eq!(response.results[2].as_error().code, Code::InvalidArgument as i32);
    assert!(!response.results[3].as_success().etag.is_empty());

    // Read metadata back. The first entry must be nonexistent as its write has been rejected.
    let get_response = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_1, "a"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_2, "b"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_1, "b"),
            ],
        })
        .await
        .expect("metadata fetch faled");

    assert_eq!(
        get_response,
        GetMetadataBatchResponse {
            results: vec![
                create_get_metadata_batch_not_found_response_entry(0),
                create_get_metadata_batch_success_response_entry(
                    1,
                    create_string_value("b"),
                    &response.results[1].as_success().etag
                ),
                create_get_metadata_batch_success_response_entry(
                    2,
                    create_string_value("x"),
                    &response.results[3].as_success().etag
                ),
            ],
        },
    );
}

#[rstest]
#[serial(postgres)]
#[test_log::test(tokio::test)]
async fn test_postgres_delete_metadata_batch(
    postgres_metadata_service: &Service,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    // TODO: add more test scenarios once we've added batch chunking.

    use crate::common::clients::metadata::types::v1alpha::*;

    let client = postgres_metadata_service.metadata_v1alpha_client(interface).await;
    let metrics = postgres_metadata_service.metrics_client();
    let resource_uri_1 = create_resource_uri();
    let resource_uri_2 = create_resource_uri();

    let (_value_1a, _etag_1a) = client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri_1), "a")
        .await
        .unwrap();
    let (value_1b, etag_1b) = client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri_1), "b")
        .await
        .unwrap();
    let (value_2a, etag_2a) = client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri_2), "a")
        .await
        .unwrap();
    let (_value_2b, _etag_2b) = client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri_2), "b")
        .await
        .unwrap();

    // Delete metadata. One delete query must be observed.
    let before_delete = metrics.get_postgres_metadata_backend_metrics().await;
    let response = client
        .delete_metadata_batch_sorted(DeleteMetadataBatchRequest {
            requests: vec![
                create_delete_metadata_batch_resource_address_request_entry(&resource_uri_1, "a"),
                create_delete_metadata_batch_resource_address_request_entry(&resource_uri_1, "c"),
                create_delete_metadata_batch_resource_address_request_entry(&resource_uri_2, "b"),
                create_delete_metadata_batch_resource_address_request_entry(&resource_uri_2, "c"),
            ],
        })
        .await
        .expect("metadata delete failed");

    assert_eq!(
        response,
        DeleteMetadataBatchResponse {
            results: vec![
                create_delete_metadata_batch_success_response_entry(0),
                create_delete_metadata_batch_success_response_entry(1),
                create_delete_metadata_batch_success_response_entry(2),
                create_delete_metadata_batch_success_response_entry(3),
            ],
        }
    );

    let after_delete = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_delete,
        PostgresMetadataBackendMetrics {
            delete_metadata_batch_count: before_delete.delete_metadata_batch_count + 1,
            ..before_delete
        }
    );

    // Read metadata back. The affected entries must be deleted.
    let get_response = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_1, "a"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_1, "b"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_2, "a"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_2, "b"),
            ],
        })
        .await
        .expect("metadata fetch failed");

    assert_eq!(
        get_response,
        GetMetadataBatchResponse {
            results: vec![
                create_get_metadata_batch_not_found_response_entry(0),
                create_get_metadata_batch_success_response_entry(1, value_1b, etag_1b),
                create_get_metadata_batch_success_response_entry(2, value_2a, etag_2a),
                create_get_metadata_batch_not_found_response_entry(3),
            ],
        }
    );
}

#[rstest]
#[serial(postgres)]
#[test_log::test(tokio::test)]
async fn test_postgres_delete_metadata_batch_reports_precondition_failure_errors(
    postgres_metadata_service: &Service,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let client = postgres_metadata_service.metadata_v1alpha_client(interface).await;
    let resource_uri_1 = create_resource_uri();
    let resource_uri_2 = create_resource_uri();

    let (_value_1a, etag_1a) = client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri_1), "a")
        .await
        .unwrap();
    let (value_1b, etag_1b) = client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri_1), "b")
        .await
        .unwrap();
    let (_value_2a, _etag_2a) = client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri_2), "a")
        .await
        .unwrap();

    let response = client
        .delete_metadata_batch_sorted(DeleteMetadataBatchRequest {
            requests: vec![
                DeleteMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri_1)),
                    user_metadata_key: "a".into(),
                    expected_etag: Some(etag_1a),
                    ..Default::default()
                },
                DeleteMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri_1)),
                    user_metadata_key: "b".into(),
                    expected_etag: Some("etag".into()),
                    ..Default::default()
                },
                DeleteMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri_2)),
                    user_metadata_key: "a".into(),
                    ..Default::default()
                },
            ],
        })
        .await
        .unwrap();

    assert_eq!(
        response,
        DeleteMetadataBatchResponse {
            results: vec![
                create_delete_metadata_batch_success_response_entry(0),
                create_delete_metadata_metadata_modified_response_entry(1),
                create_delete_metadata_batch_success_response_entry(2),
            ]
        }
    );

    // Read metadata back. Only the second entry must remain.
    let get_response = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_1, "a"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_1, "b"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_2, "a"),
            ],
        })
        .await
        .unwrap();

    assert_eq!(
        get_response,
        GetMetadataBatchResponse {
            results: vec![
                create_get_metadata_batch_not_found_response_entry(0),
                create_get_metadata_batch_success_response_entry(1, value_1b, etag_1b),
                create_get_metadata_batch_not_found_response_entry(2),
            ],
        }
    );
}

#[rstest]
#[serial(postgres)]
#[test_log::test(tokio::test)]
async fn test_postgres_delete_metadata_batch_rejects_duplicate_writes(
    postgres_metadata_service: &Service,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let client = postgres_metadata_service.metadata_v1alpha_client(interface).await;
    let resource_uri_1 = create_resource_uri();
    let resource_uri_2 = create_resource_uri();

    let (value_1, etag_1) = client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri_1), "a")
        .await
        .unwrap();
    let (_value_2, _etag_2) = client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri_2), "b")
        .await
        .unwrap();

    let response = client
        .delete_metadata_batch_sorted(DeleteMetadataBatchRequest {
            requests: vec![
                create_delete_metadata_batch_resource_address_request_entry(&resource_uri_1, "a"),
                create_delete_metadata_batch_resource_address_request_entry(&resource_uri_2, "b"),
                create_delete_metadata_batch_resource_address_request_entry(&resource_uri_1, "a"),
                create_delete_metadata_batch_resource_address_request_entry(&resource_uri_1, "b"),
            ],
        })
        .await
        .expect("metadata delete failed");

    assert_eq!(
        response,
        DeleteMetadataBatchResponse {
            results: vec![
                create_delete_metadata_duplicate_batch_entry_response_entry(0),
                create_delete_metadata_batch_success_response_entry(1),
                create_delete_metadata_duplicate_batch_entry_response_entry(2),
                create_delete_metadata_batch_success_response_entry(3),
            ]
        }
    );

    // Read metadata back. Rejected operations must not have been executed.
    let get_response = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_1, "a"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_2, "b"),
            ],
        })
        .await
        .expect("metadata fetch faled");

    assert_eq!(
        get_response,
        GetMetadataBatchResponse {
            results: vec![
                create_get_metadata_batch_success_response_entry(0, value_1, etag_1),
                create_get_metadata_batch_not_found_response_entry(1),
            ],
        },
    );
}

#[rstest]
#[serial(postgres)]
#[test_log::test(tokio::test)]
async fn test_postgres_update_get_metadata_batch_respect_namespace(postgres_metadata_service: &Service) {
    use crate::common::clients::metadata::types::v1alpha::{
        batch_error_entry::Code, get_metadata_batch_response_entry::Outcome, *,
    };

    let client = postgres_metadata_service.metadata_v1alpha_client(Interface::Grpc).await;
    let resource_uri = create_resource_uri();

    let update_response = client
        .update_metadata_batch_sorted(UpdateMetadataBatchRequest {
            requests: vec![
                UpdateMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "dupe".into(),
                    user_metadata: Some(create_string_value("dupe-value")),
                    ..Default::default()
                },
                UpdateMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "dupe".into(),
                    user_metadata: Some(create_string_value("dupe-value")),
                    namespace: Some("default".into()),
                    ..Default::default()
                },
                UpdateMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "a".into(),
                    user_metadata: Some(create_string_value("a-default")),
                    ..Default::default()
                },
                UpdateMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "a".into(),
                    user_metadata: Some(create_string_value("a-custom")),
                    namespace: Some("custom".into()),
                    ..Default::default()
                },
            ],
        })
        .await
        .unwrap();

    assert_eq!(update_response.results.len(), 4);
    assert_eq!(update_response.results[0].as_error().code, Code::InvalidArgument as i32);
    assert_eq!(update_response.results[1].as_error().code, Code::InvalidArgument as i32);
    assert!(!update_response.results[2].as_success().etag.is_empty());
    assert!(!update_response.results[3].as_success().etag.is_empty());

    let get_response = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                GetMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "dupe".into(),
                    ..Default::default()
                },
                GetMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "dupe".into(),
                    namespace: Some("default".into()),
                },
                GetMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "a".into(),
                    ..Default::default()
                },
                GetMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "a".into(),
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
                create_get_metadata_batch_not_found_response_entry(0),
                create_get_metadata_batch_not_found_response_entry(1),
                GetMetadataBatchResponseEntry {
                    request_index: 2,
                    outcome: Some(Outcome::Success(create_user_metadata_value(
                        create_string_value("a-default"),
                        &update_response.results[2].as_success().etag
                    )))
                },
                GetMetadataBatchResponseEntry {
                    request_index: 3,
                    outcome: Some(Outcome::Success(create_user_metadata_value(
                        create_string_value("a-custom"),
                        &update_response.results[3].as_success().etag
                    )))
                },
            ]
        }
    );
}

#[rstest]
#[serial(postgres)]
#[test_log::test(tokio::test)]
async fn test_postgres_query_metadata_respects_namespace(postgres_metadata_service: &Service) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let client = postgres_metadata_service.metadata_v1alpha_client(Interface::Grpc).await;
    let resource_uri = create_resource_uri();

    let update_response = client
        .update_metadata_batch_sorted(UpdateMetadataBatchRequest {
            requests: vec![
                UpdateMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "a".into(),
                    user_metadata: Some(create_string_value("a-default")),
                    ..Default::default()
                },
                UpdateMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "a".into(),
                    user_metadata: Some(create_string_value("a-custom")),
                    namespace: Some("custom".into()),
                    ..Default::default()
                },
            ],
        })
        .await
        .unwrap();

    assert_eq!(update_response.results.len(), 2);
    assert!(!update_response.results[0].as_success().etag.is_empty());
    assert!(!update_response.results[1].as_success().etag.is_empty());

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
            results: vec![QueryMetadataResponseEntry {
                user_metadata_key: "a".into(),
                user_metadata: Some(create_user_metadata_value(
                    create_string_value("a-default"),
                    &update_response.results[0].as_success().etag
                )),
            }]
        }
    );

    let query_response_2 = client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
            namespace: Some("default".into()),
        })
        .await
        .unwrap();

    assert_eq!(query_response_2, query_response_1);

    let query_response_3 = client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
            namespace: Some("custom".into()),
        })
        .await
        .unwrap();

    assert_eq!(
        query_response_3,
        QueryMetadataResponse {
            results: vec![QueryMetadataResponseEntry {
                user_metadata_key: "a".into(),
                user_metadata: Some(create_user_metadata_value(
                    create_string_value("a-custom"),
                    &update_response.results[1].as_success().etag
                )),
            }]
        }
    );
}

#[rstest]
#[serial(postgres)]
#[test_log::test(tokio::test)]
async fn test_postgres_update_metadata_batch_through_update_respects_namespace(postgres_metadata_service: &Service) {
    use crate::common::clients::metadata::types::v1alpha::{batch_error_entry::Code, *};

    let client = postgres_metadata_service.metadata_v1alpha_client(Interface::Grpc).await;
    let resource_uri = create_resource_uri();

    let update_response_1 = client
        .update_metadata_batch_sorted(UpdateMetadataBatchRequest {
            requests: vec![
                UpdateMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "a".into(),
                    user_metadata: Some(create_string_value("a-default")),
                    ..Default::default()
                },
                UpdateMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "a".into(),
                    user_metadata: Some(create_string_value("a-custom")),
                    namespace: Some("custom".into()),
                    ..Default::default()
                },
                UpdateMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "b".into(),
                    user_metadata: Some(create_string_value("b-custom")),
                    namespace: Some("custom".into()),
                    ..Default::default()
                },
            ],
        })
        .await
        .unwrap();

    assert_eq!(update_response_1.results.len(), 3);
    assert!(!update_response_1.results[0].as_success().etag.is_empty());
    assert!(!update_response_1.results[1].as_success().etag.is_empty());
    assert!(!update_response_1.results[2].as_success().etag.is_empty());

    let update_response_2 = client
        .update_metadata_batch_sorted(UpdateMetadataBatchRequest {
            requests: vec![
                UpdateMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "a".into(),
                    user_metadata: Some(create_string_value("a-default-new")),
                    expected_etag: Some(update_response_1.results[0].as_success().etag.clone()),
                    ..Default::default()
                },
                UpdateMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "a".into(),
                    user_metadata: Some(create_string_value("a-custom-new")),
                    namespace: Some("custom".into()),
                    expected_etag: Some(update_response_1.results[1].as_success().etag.clone()),
                },
                UpdateMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "b".into(),
                    user_metadata: Some(create_string_value("b-custom-new")),
                    expected_etag: Some(update_response_1.results[2].as_success().etag.clone()),
                    ..Default::default()
                },
            ],
        })
        .await
        .unwrap();

    assert_eq!(update_response_2.results.len(), 3);
    assert!(!update_response_2.results[0].as_success().etag.is_empty());
    assert!(!update_response_2.results[1].as_success().etag.is_empty());
    assert_eq!(
        update_response_2.results[2].as_error().code,
        Code::FailedPrecondition as i32
    );

    let get_response = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                GetMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "a".into(),
                    ..Default::default()
                },
                GetMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "a".into(),
                    namespace: Some("custom".into()),
                },
                GetMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "b".into(),
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
                create_get_metadata_batch_success_response_entry(
                    0,
                    create_string_value("a-default-new"),
                    &update_response_2.results[0].as_success().etag
                ),
                create_get_metadata_batch_success_response_entry(
                    1,
                    create_string_value("a-custom-new"),
                    &update_response_2.results[1].as_success().etag
                ),
                create_get_metadata_batch_success_response_entry(
                    2,
                    create_string_value("b-custom"),
                    &update_response_1.results[2].as_success().etag
                ),
            ],
        }
    );
}

#[rstest]
#[serial(postgres)]
#[test_log::test(tokio::test)]
async fn test_postgres_delete_metadata_batch_respects_namespace(postgres_metadata_service: &Service) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let client = postgres_metadata_service.metadata_v1alpha_client(Interface::Grpc).await;
    let resource_uri = create_resource_uri();

    let update_response = client
        .update_metadata_batch_sorted(UpdateMetadataBatchRequest {
            requests: vec![
                UpdateMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "a".into(),
                    user_metadata: Some(create_string_value("a-default")),
                    ..Default::default()
                },
                UpdateMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "b".into(),
                    user_metadata: Some(create_string_value("b-default")),
                    ..Default::default()
                },
                UpdateMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "a".into(),
                    user_metadata: Some(create_string_value("a-custom")),
                    namespace: Some("custom".into()),
                    ..Default::default()
                },
            ],
        })
        .await
        .unwrap();

    assert_eq!(update_response.results.len(), 3);
    assert!(!update_response.results[0].as_success().etag.is_empty());
    assert!(!update_response.results[1].as_success().etag.is_empty());
    assert!(!update_response.results[2].as_success().etag.is_empty());

    let delete_response = client
        .delete_metadata_batch_sorted(DeleteMetadataBatchRequest {
            requests: vec![
                DeleteMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "a".into(),
                    expected_etag: Some(update_response.results[0].as_success().etag.clone()),
                    ..Default::default()
                },
                DeleteMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "b".into(),
                    expected_etag: Some(update_response.results[1].as_success().etag.clone()),
                    namespace: Some("default".into()),
                },
                DeleteMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "a".into(),
                    expected_etag: Some(update_response.results[2].as_success().etag.clone()),
                    namespace: Some("custom".into()),
                },
                DeleteMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "c".into(),
                    ..Default::default()
                },
                DeleteMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "c".into(),
                    namespace: Some("default".into()),
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
                create_delete_metadata_batch_success_response_entry(0),
                create_delete_metadata_batch_success_response_entry(1),
                create_delete_metadata_batch_success_response_entry(2),
                create_delete_metadata_duplicate_batch_entry_response_entry(3),
                create_delete_metadata_duplicate_batch_entry_response_entry(4),
            ],
        }
    );

    let get_response = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                GetMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "a".into(),
                    ..Default::default()
                },
                GetMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "b".into(),
                    ..Default::default()
                },
                GetMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "a".into(),
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
                create_get_metadata_batch_not_found_response_entry(0),
                create_get_metadata_batch_not_found_response_entry(1),
                create_get_metadata_batch_not_found_response_entry(2),
            ],
        }
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_postgres_limits_write_item_size(#[values(Interface::Grpc, Interface::Rest)] interface: Interface) {
    use crate::common::clients::metadata::types::v1alpha::{batch_error_entry::Code, *};
    use serde_json::json;

    const MAX_METADATA_VALUE_SIZE: usize = 32;

    let postgres_env = get_env().unwrap_postgres();
    let s3_env = get_env().unwrap_s3();
    let service = Service::start_with_config(json!({
        "storage": {
            "s3": {
                "endpoints": {
                    format!("http://{}:{}/{}", s3_env.host, s3_env.port, s3_env.bucket): {
                        "credentials": {
                            "access_key": {
                                "access_key_id": &s3_env.access_key_id,
                                "secret_access_key": &s3_env.secret_access_key,
                            },
                        },
                        "endpoint_url": s3_env.endpoint_url(),
                        "addressing_style": "path",
                        "metadata_backend": "default",
                    },
                },
            },
        },
        "metadata": {
            "default": {
                "postgres": {
                    "host": &postgres_env.host,
                    "port": postgres_env.port,
                    "credentials": {
                        "password": {
                            "user": &postgres_env.user,
                            "password": &postgres_env.password,
                        },
                    },
                    "dbname": &postgres_env.dbname,
                    "connection_pool_size": 1,
                    "max_metadata_value_size": format!("{MAX_METADATA_VALUE_SIZE}B"),
                },
            },
        },
        "rest": {
            "addr": "0.0.0.0",
            "port": 0,
        },
        "grpc": {
            "addr": "0.0.0.0",
            "port": 0,
        },
    }));
    let client = service.metadata_v1alpha_client(interface).await;
    let resource_uri = create_resource_uri();
    let short_value = create_string_value("reasonable value");
    let long_value = create_string_value("x".repeat(MAX_METADATA_VALUE_SIZE + 1));

    let update_response = client
        .update_metadata_batch_sorted(UpdateMetadataBatchRequest {
            requests: vec![
                create_update_metadata_batch_resource_address_request_entry(
                    &resource_uri,
                    "short",
                    short_value.clone(),
                ),
                create_update_metadata_batch_resource_address_request_entry(&resource_uri, "long", long_value),
            ],
        })
        .await
        .expect("metadata update failed");

    assert_eq!(update_response.results.len(), 2);
    assert!(!update_response.results[0].as_success().etag.is_empty());
    assert_eq!(
        update_response.results[1].as_error(),
        &BatchErrorEntry {
            code: Code::InvalidArgument as i32,
            message: format!(
                "metadata value is too large (actual {}, limit {MAX_METADATA_VALUE_SIZE})",
                MAX_METADATA_VALUE_SIZE + 3
            ),
        }
    );

    let get_response = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                create_get_metadata_batch_resource_address_request_entry(&resource_uri, "short"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri, "long"),
            ],
        })
        .await
        .expect("metadata fetch failed");

    assert_eq!(
        get_response,
        GetMetadataBatchResponse {
            results: vec![
                create_get_metadata_batch_success_response_entry(
                    0,
                    short_value,
                    &update_response.results[0].as_success().etag
                ),
                create_get_metadata_batch_not_found_response_entry(1),
            ],
        }
    );
}

#[rstest]
#[case(1, vec![vec!["a"], vec!["b"], vec!["c"]])]
#[case(150, vec![vec!["a", "b"], vec!["c"]])]
#[test_log::test(tokio::test)]
async fn test_postgres_respects_query_budget_limit(
    #[case] query_byte_size_budget: usize,
    #[case] expected_page_keys: Vec<Vec<&str>>,
) {
    use crate::common::clients::metadata::types::v1alpha::*;
    use futures::TryStreamExt;
    use serde_json::json;

    let postgres_env = get_env().unwrap_postgres();
    let s3_env = get_env().unwrap_s3();
    let service = Service::start_with_config(json!({
        "storage": {
            "s3": {
                "endpoints": {
                    format!("http://{}:{}/{}", s3_env.host, s3_env.port, s3_env.bucket): {
                        "credentials": {
                            "access_key": {
                                "access_key_id": &s3_env.access_key_id,
                                "secret_access_key": &s3_env.secret_access_key,
                            },
                        },
                        "endpoint_url": s3_env.endpoint_url(),
                        "addressing_style": "path",
                        "metadata_backend": "default",
                    },
                },
            },
        },
        "metadata": {
            "default": {
                "postgres": {
                    "host": &postgres_env.host,
                    "port": postgres_env.port,
                    "credentials": {
                        "password": {
                            "user": &postgres_env.user,
                            "password": &postgres_env.password,
                        },
                    },
                    "dbname": &postgres_env.dbname,
                    "connection_pool_size": 1,
                    "query_byte_size_budget": format!("{query_byte_size_budget}B"),
                },
            },
        },
        "rest": {
            "addr": "0.0.0.0",
            "port": 0,
        },
        "grpc": {
            "addr": "0.0.0.0",
            "port": 0,
        },
    }));
    let client = service.metadata_v1alpha_client(Interface::Rest).await;
    let resource_uri = create_resource_uri();
    populate_metadata(
        &client,
        ResourceLocation::resource_address(&resource_uri),
        ["a", "b", "c"],
    )
    .await;

    let responses: Vec<_> = client
        .query_metadata_with_page_size_hint(
            QueryMetadataRequest {
                resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                ..Default::default()
            },
            100,
        )
        .try_collect()
        .await
        .unwrap();

    let actual_page_keys = responses
        .iter()
        .map(|response| {
            response
                .results
                .iter()
                .map(|entry| entry.user_metadata_key.as_str())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert_eq!(actual_page_keys, expected_page_keys);
}

#[rstest]
#[serial(postgres_cache)]
#[test_log::test(tokio::test)]
async fn test_metadata_cache_get_metadata_populates_key_cache(
    postgres_cache_metadata_service: &Service,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let client = postgres_cache_metadata_service.metadata_v1alpha_client(interface).await;
    let metrics = postgres_cache_metadata_service.metrics_client();
    let resource_uri_1 = create_resource_uri();
    let resource_uri_2 = create_resource_uri();

    let (value_1, etag_1) = client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri_1), "a")
        .await
        .unwrap();
    let (value_2, etag_2) = client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri_1), "b")
        .await
        .unwrap();
    let (_value_3, _etag_3) = client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri_2), "a")
        .await
        .unwrap();

    // Populate cache. We must observe a query.
    let before_read = metrics.get_postgres_metadata_backend_metrics().await;
    let get_response_1 = client
        .get_metadata(GetMetadataRequest {
            uri: resource_uri_1.clone(),
            user_metadata_keys: vec!["a".into(), "b".into(), "c".into()],
        })
        .await
        .unwrap();

    assert_eq!(
        get_response_1,
        GetMetadataResponse {
            user_metadata: [
                create_user_metadata_kv("a", value_1.clone(), &etag_1),
                create_user_metadata_kv("b", value_2.clone(), &etag_2),
            ]
            .into(),
        }
    );

    let after_read_1 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_read_1,
        PostgresMetadataBackendMetrics {
            query_metadata_batch_count: before_read.query_metadata_batch_count + 1,
            ..before_read
        }
    );

    // Query the same keys, including the non-existent one. We must observe no new queries.
    let get_response_2 = client
        .get_metadata(GetMetadataRequest {
            uri: resource_uri_1.clone(),
            user_metadata_keys: vec!["a".into(), "c".into()],
        })
        .await
        .unwrap();

    assert_eq!(
        get_response_2,
        GetMetadataResponse {
            user_metadata: [create_user_metadata_kv("a", value_1.clone(), &etag_1)].into(),
        }
    );

    let after_read_2 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(after_read_2, after_read_1);

    // Query a not-yet-fetched key. We must observe a query.
    let get_response_3 = client
        .get_metadata(GetMetadataRequest {
            uri: resource_uri_1.clone(),
            user_metadata_keys: vec!["a".into(), "1".into()],
        })
        .await
        .unwrap();

    assert_eq!(
        get_response_3,
        GetMetadataResponse {
            user_metadata: [create_user_metadata_kv("a", value_1, etag_1)].into(),
        }
    );

    let after_read_3 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_read_3,
        PostgresMetadataBackendMetrics {
            query_metadata_batch_count: after_read_2.query_metadata_batch_count + 1,
            ..after_read_2
        }
    );
}

#[rstest]
#[serial(postgres_cache)]
#[test_log::test(tokio::test)]
async fn test_metadata_cache_get_all_metadata_populates_query_cache(
    postgres_cache_metadata_service: &Service,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let client = postgres_cache_metadata_service.metadata_v1alpha_client(interface).await;
    let metrics = postgres_cache_metadata_service.metrics_client();
    let resource_uri = create_resource_uri();

    let (value_1, etag_1) = client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri), "a")
        .await
        .unwrap();
    let (value_2, etag_2) = client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri), "b")
        .await
        .unwrap();

    // Populate the cache. We must observe a query.
    let before_read = metrics.get_postgres_metadata_backend_metrics().await;
    let get_response_1 = client
        .get_metadata(GetMetadataRequest {
            uri: resource_uri.clone(),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(
        get_response_1,
        GetMetadataResponse {
            user_metadata: [
                create_user_metadata_kv("a", value_1.clone(), &etag_1),
                create_user_metadata_kv("b", value_2.clone(), &etag_2),
            ]
            .into()
        }
    );

    let after_read_1 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_read_1,
        PostgresMetadataBackendMetrics {
            query_all_metadata_for_resource_count: before_read.query_all_metadata_for_resource_count + 1,
            ..before_read
        }
    );

    // Get all metadata for the resource. We must observe no new queries.
    let get_response_2 = client
        .get_metadata(GetMetadataRequest {
            uri: resource_uri.clone(),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(get_response_2, get_response_1);

    let after_read_2 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(after_read_2, after_read_1);

    // Get arbitrary metadata keys for the resource. We must observe no queries.
    let get_response_3 = client
        .get_metadata(GetMetadataRequest {
            uri: resource_uri.clone(),
            user_metadata_keys: vec!["a".into(), "1".into()],
        })
        .await
        .unwrap();

    assert_eq!(
        get_response_3,
        GetMetadataResponse {
            user_metadata: [create_user_metadata_kv("a", value_1, etag_1)].into()
        }
    );

    let after_read_3 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(after_read_3, after_read_2);
}

#[rstest]
#[serial(postgres_cache)]
#[test_log::test(tokio::test)]
async fn test_metadata_cache_update_metadata_invalidates_key_cache(
    postgres_cache_metadata_service: &Service,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let client = postgres_cache_metadata_service.metadata_v1alpha_client(interface).await;
    let metrics = postgres_cache_metadata_service.metrics_client();
    let resource_uri = create_resource_uri();

    // Populate metadata cache.
    let before_read = metrics.get_postgres_metadata_backend_metrics().await;
    let get_response_1 = client
        .get_metadata(GetMetadataRequest {
            uri: resource_uri.clone(),
            user_metadata_keys: vec!["a".into(), "b".into()],
        })
        .await
        .unwrap();

    assert_eq!(get_response_1, GetMetadataResponse::default());

    let after_read_1 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_read_1,
        PostgresMetadataBackendMetrics {
            query_metadata_batch_count: before_read.query_metadata_batch_count + 1,
            ..before_read
        }
    );

    // Update a key value. Only the affected key cache entry must be invalidated.
    let update_response = client
        .update_metadata(UpdateMetadataRequest {
            uri: resource_uri.clone(),
            user_metadata_key: "a".into(),
            user_metadata: Some(create_string_value("a-new")),
            ..Default::default()
        })
        .await
        .unwrap();

    assert!(!update_response.etag.is_empty());

    let after_write_2 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_write_2,
        PostgresMetadataBackendMetrics {
            upsert_metadata_batch_count: after_read_1.upsert_metadata_batch_count + 1,
            ..after_read_1
        }
    );

    // Query the unaffected key value. We must observe no queries.
    let get_response_2 = client
        .get_metadata(GetMetadataRequest {
            uri: resource_uri.clone(),
            user_metadata_keys: vec!["b".into()],
        })
        .await
        .unwrap();

    assert_eq!(get_response_2, GetMetadataResponse::default());

    let after_read_3 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(after_read_3, after_write_2);

    // Query the affected key value. We must observe a query.
    let get_response_3 = client
        .get_metadata(GetMetadataRequest {
            uri: resource_uri,
            user_metadata_keys: vec!["a".into()],
        })
        .await
        .unwrap();

    assert_eq!(
        get_response_3,
        GetMetadataResponse {
            user_metadata: [create_user_metadata_kv(
                "a",
                create_string_value("a-new"),
                update_response.etag
            )]
            .into()
        }
    );

    let after_read_4 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_read_4,
        PostgresMetadataBackendMetrics {
            query_metadata_batch_count: after_read_3.query_metadata_batch_count + 1,
            ..after_read_3
        }
    );
}

#[rstest]
#[serial(postgres_cache)]
#[test_log::test(tokio::test)]
async fn test_metadata_cache_update_metadata_invalidates_query_cache(
    postgres_cache_metadata_service: &Service,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let client = postgres_cache_metadata_service.metadata_v1alpha_client(interface).await;
    let metrics = postgres_cache_metadata_service.metrics_client();
    let resource_uri = create_resource_uri();

    // Populate query cache.
    let before_read = metrics.get_postgres_metadata_backend_metrics().await;
    let get_response_1 = client
        .get_metadata(GetMetadataRequest {
            uri: resource_uri.clone(),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(get_response_1, GetMetadataResponse::default());

    let after_read_1 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_read_1,
        PostgresMetadataBackendMetrics {
            query_all_metadata_for_resource_count: before_read.query_all_metadata_for_resource_count + 1,
            ..before_read
        }
    );

    // Write a metadata key. We must observe a new upsert query.
    let update_response = client
        .update_metadata(UpdateMetadataRequest {
            uri: resource_uri.clone(),
            user_metadata_key: "a".into(),
            user_metadata: Some(create_string_value("new-a")),
            ..Default::default()
        })
        .await
        .unwrap();

    assert!(!update_response.etag.is_empty());

    let after_write_2 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_write_2,
        PostgresMetadataBackendMetrics {
            upsert_metadata_batch_count: after_read_1.upsert_metadata_batch_count + 1,
            ..after_read_1
        }
    );

    // Query metadata again. We must observe a new query.
    let get_response_2 = client
        .get_metadata(GetMetadataRequest {
            uri: resource_uri.clone(),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(
        get_response_2,
        GetMetadataResponse {
            user_metadata: [create_user_metadata_kv(
                "a",
                create_string_value("new-a"),
                update_response.etag
            )]
            .into()
        }
    );

    let after_read_3 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_read_3,
        PostgresMetadataBackendMetrics {
            query_all_metadata_for_resource_count: after_write_2.query_all_metadata_for_resource_count + 1,
            ..after_write_2
        }
    );
}

#[rstest]
#[serial(postgres_cache)]
#[test_log::test(tokio::test)]
async fn test_metadata_cache_delete_metadata_invalidates_key_cache(
    postgres_cache_metadata_service: &Service,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let client = postgres_cache_metadata_service.metadata_v1alpha_client(interface).await;
    let metrics = postgres_cache_metadata_service.metrics_client();
    let resource_uri = create_resource_uri();

    // Populate metadata cache.
    let before_read = metrics.get_postgres_metadata_backend_metrics().await;
    let get_response_1 = client
        .get_metadata(GetMetadataRequest {
            uri: resource_uri.clone(),
            user_metadata_keys: vec!["a".into(), "b".into()],
        })
        .await
        .unwrap();

    assert_eq!(get_response_1, GetMetadataResponse::default());

    let after_read_1 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_read_1,
        PostgresMetadataBackendMetrics {
            query_metadata_batch_count: before_read.query_metadata_batch_count + 1,
            ..before_read
        }
    );

    // Delete an arbitrary key. We must observe a delete query.
    client
        .delete_metadata(DeleteMetadataRequest {
            uri: resource_uri.clone(),
            user_metadata_key: "a".into(),
            ..Default::default()
        })
        .await
        .unwrap();

    let after_write_2 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_write_2,
        PostgresMetadataBackendMetrics {
            delete_metadata_batch_count: after_read_1.delete_metadata_batch_count + 1,
            ..after_read_1
        }
    );

    // Query the unaffected key value. We must observe no queries.
    let get_response_2 = client
        .get_metadata(GetMetadataRequest {
            uri: resource_uri.clone(),
            user_metadata_keys: vec!["b".into()],
        })
        .await
        .unwrap();

    assert_eq!(get_response_2, GetMetadataResponse::default());

    let after_read_3 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(after_read_3, after_write_2);

    // Query the affected key value. We must observe a query.
    let get_response_3 = client
        .get_metadata(GetMetadataRequest {
            uri: resource_uri,
            user_metadata_keys: vec!["a".into()],
        })
        .await
        .unwrap();

    assert_eq!(get_response_3, GetMetadataResponse::default());

    let after_read_4 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_read_4,
        PostgresMetadataBackendMetrics {
            query_metadata_batch_count: after_read_3.query_metadata_batch_count + 1,
            ..after_read_3
        }
    );
}

#[rstest]
#[serial(postgres_cache)]
#[test_log::test(tokio::test)]
async fn test_metadata_cache_delete_metadata_invalidates_query_cache(
    postgres_cache_metadata_service: &Service,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let client = postgres_cache_metadata_service.metadata_v1alpha_client(interface).await;
    let metrics = postgres_cache_metadata_service.metrics_client();
    let resource_uri = create_resource_uri();

    // Populate query cache.
    let before_read = metrics.get_postgres_metadata_backend_metrics().await;
    let get_response_1 = client
        .get_metadata(GetMetadataRequest {
            uri: resource_uri.clone(),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(get_response_1, GetMetadataResponse::default());

    let after_read_1 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_read_1,
        PostgresMetadataBackendMetrics {
            query_all_metadata_for_resource_count: before_read.query_all_metadata_for_resource_count + 1,
            ..before_read
        }
    );

    // Delete a metadata key. We must observe a new delete query.
    client
        .delete_metadata(DeleteMetadataRequest {
            uri: resource_uri.clone(),
            user_metadata_key: "a".into(),
            ..Default::default()
        })
        .await
        .unwrap();

    let after_write_2 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_write_2,
        PostgresMetadataBackendMetrics {
            delete_metadata_batch_count: after_read_1.delete_metadata_batch_count + 1,
            ..after_read_1
        }
    );

    // Query metadata again. We must observe a new query.
    let get_response_2 = client
        .get_metadata(GetMetadataRequest {
            uri: resource_uri.clone(),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(get_response_2, GetMetadataResponse::default());

    let after_read_3 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_read_3,
        PostgresMetadataBackendMetrics {
            query_all_metadata_for_resource_count: after_write_2.query_all_metadata_for_resource_count + 1,
            ..after_write_2
        }
    );
}

#[rstest]
#[serial(postgres_cache)]
#[test_log::test(tokio::test)]
async fn test_metadata_cache_get_metadata_batch_populates_key_cache(
    postgres_cache_metadata_service: &Service,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let client = postgres_cache_metadata_service.metadata_v1alpha_client(interface).await;
    let metrics = postgres_cache_metadata_service.metrics_client();
    let resource_uri_1 = create_resource_uri();
    let resource_uri_2 = create_resource_uri();

    let (value_1a, etag_1a) = client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri_1), "a")
        .await
        .unwrap();
    let (value_1b, etag_1b) = client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri_1), "b")
        .await
        .unwrap();
    let (value_2a, etag_2a) = client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri_2), "a")
        .await
        .unwrap();

    // Make a batch read. We should see a new read query.
    let before_read = metrics.get_postgres_metadata_backend_metrics().await;
    let response_1 = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_1, "a"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_1, "b"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_1, "c"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_2, "a"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_2, "b"),
            ],
        })
        .await
        .unwrap();

    assert_eq!(
        response_1,
        GetMetadataBatchResponse {
            results: vec![
                create_get_metadata_batch_success_response_entry(0, value_1a.clone(), &etag_1a),
                create_get_metadata_batch_success_response_entry(1, value_1b.clone(), &etag_1b),
                create_get_metadata_batch_not_found_response_entry(2),
                create_get_metadata_batch_success_response_entry(3, value_2a, etag_2a),
                create_get_metadata_batch_not_found_response_entry(4),
            ],
        },
    );

    let after_read_1 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_read_1,
        PostgresMetadataBackendMetrics {
            query_metadata_batch_count: before_read.query_metadata_batch_count + 1,
            ..before_read
        }
    );

    // Make the same query again. Cached values and not found errors must be cached.
    // No new queries must be observed.
    let response_2 = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_1, "a"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_1, "b"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_1, "c"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_2, "a"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_2, "b"),
            ],
        })
        .await
        .unwrap();
    assert_eq!(response_2, response_1);

    let after_read_2 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(after_read_1, after_read_2);

    // Query a mixed set of cached/non-cached values. We must observe an additional query.
    let response_3 = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_1, "a"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_1, "0"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_1, "1"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_1, "2"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_1, "a"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_1, "3"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_1, "4"),
            ],
        })
        .await
        .unwrap();

    assert_eq!(
        response_3,
        GetMetadataBatchResponse {
            results: vec![
                create_get_metadata_batch_success_response_entry(0, value_1a.clone(), &etag_1a),
                create_get_metadata_batch_not_found_response_entry(1),
                create_get_metadata_batch_not_found_response_entry(2),
                create_get_metadata_batch_not_found_response_entry(3),
                create_get_metadata_batch_success_response_entry(4, value_1a, etag_1a),
                create_get_metadata_batch_not_found_response_entry(5),
                create_get_metadata_batch_not_found_response_entry(6),
            ]
        }
    );

    let after_read_3 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_read_3,
        PostgresMetadataBackendMetrics {
            query_metadata_batch_count: after_read_2.query_metadata_batch_count + 1,
            ..after_read_2
        }
    );
}

#[rstest]
#[serial(postgres_cache)]
#[test_log::test(tokio::test)]
async fn test_metadata_cache_query_metadata_populates_caches(
    postgres_cache_metadata_service: &Service,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let client = postgres_cache_metadata_service.metadata_v1alpha_client(interface).await;
    let metrics = postgres_cache_metadata_service.metrics_client();
    let resource_uri = create_resource_uri();

    let (value_1, etag_1) = client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri), "1")
        .await
        .unwrap();
    let (value_2, etag_2) = client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri), "2")
        .await
        .unwrap();

    // Make a query. We should observe a query operation.
    let before_read = metrics.get_postgres_metadata_backend_metrics().await;
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
            results: vec![
                create_query_metadata_response_entry("1", value_1.clone(), &etag_1),
                create_query_metadata_response_entry("2", value_2.clone(), &etag_2),
            ]
        }
    );

    let after_read_1 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_read_1,
        PostgresMetadataBackendMetrics {
            query_all_metadata_for_resource_count: before_read.query_all_metadata_for_resource_count + 1,
            ..before_read
        }
    );

    // Make another query against the same address. We must observe no new queries.
    let query_response_2 = client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(query_response_2, query_response_1);

    let after_read_2 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(after_read_2, after_read_1);

    // Get metadata keys in a batch. No new queries must be observed.
    let get_response = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                create_get_metadata_batch_resource_address_request_entry(&resource_uri, "1"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri, "x"),
            ],
        })
        .await
        .unwrap();

    assert_eq!(
        get_response,
        GetMetadataBatchResponse {
            results: vec![
                create_get_metadata_batch_success_response_entry(0, value_1, etag_1),
                create_get_metadata_batch_not_found_response_entry(1),
            ]
        }
    );

    let after_read_3 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(after_read_3, after_read_2);
}

#[rstest]
#[serial(postgres_cache)]
#[test_log::test(tokio::test)]
async fn test_metadata_cache_update_metadata_batch_invalidates_key_cache(
    postgres_cache_metadata_service: &Service,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::{batch_error_entry::Code, *};

    let client = postgres_cache_metadata_service.metadata_v1alpha_client(interface).await;
    let metrics = postgres_cache_metadata_service.metrics_client();
    let resource_uri = create_resource_uri();

    // Populate cache.
    let before_read = metrics.get_postgres_metadata_backend_metrics().await;
    let get_response_1 = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                create_get_metadata_batch_resource_address_request_entry(&resource_uri, "a"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri, "b"),
            ],
        })
        .await
        .unwrap();

    assert_eq!(
        get_response_1,
        GetMetadataBatchResponse {
            results: vec![
                create_get_metadata_batch_not_found_response_entry(0),
                create_get_metadata_batch_not_found_response_entry(1),
            ],
        }
    );

    let after_read_1 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_read_1,
        PostgresMetadataBackendMetrics {
            query_metadata_batch_count: before_read.query_metadata_batch_count + 1,
            ..before_read
        }
    );

    // Update key values. The operation must not invalidate cache entries for operations failed
    // with the failed precondition error.
    let update_response = client
        .update_metadata_batch_sorted(UpdateMetadataBatchRequest {
            requests: vec![
                UpdateMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "a".into(),
                    user_metadata: Some(create_string_value("a-new")),
                    ..Default::default()
                },
                UpdateMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "b".into(),
                    user_metadata: Some(create_string_value("b-new")),
                    expected_etag: Some("etag".into()),
                    ..Default::default()
                },
            ],
        })
        .await
        .unwrap();

    assert_eq!(update_response.results.len(), 2);
    assert!(!update_response.results[0].as_success().etag.is_empty());
    assert_eq!(
        update_response.results[1].as_error().code,
        Code::FailedPrecondition as i32
    );

    let after_write_2 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_write_2,
        PostgresMetadataBackendMetrics {
            update_metadata_batch_count: after_read_1.update_metadata_batch_count + 1,
            upsert_metadata_batch_count: after_read_1.upsert_metadata_batch_count + 1,
            ..after_read_1
        }
    );

    // Query the unaffected key value. We must observe no new queries.
    let get_response_2 = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![create_get_metadata_batch_resource_address_request_entry(
                &resource_uri,
                "b",
            )],
        })
        .await
        .unwrap();

    assert_eq!(
        get_response_2,
        GetMetadataBatchResponse {
            results: vec![create_get_metadata_batch_not_found_response_entry(0)]
        }
    );

    let after_read_3 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(after_read_3, after_write_2);

    // Query the affected key value. We must observe a query.
    let get_response_3 = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![create_get_metadata_batch_resource_address_request_entry(
                &resource_uri,
                "a",
            )],
        })
        .await
        .unwrap();

    assert_eq!(
        get_response_3,
        GetMetadataBatchResponse {
            results: vec![create_get_metadata_batch_success_response_entry(
                0,
                create_string_value("a-new"),
                &update_response.results[0].as_success().etag
            )]
        }
    );

    let after_read_4 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_read_4,
        PostgresMetadataBackendMetrics {
            query_metadata_batch_count: after_read_3.query_metadata_batch_count + 1,
            ..after_read_3
        }
    );
}

#[rstest]
#[serial(postgres_cache)]
#[test_log::test(tokio::test)]
async fn test_metadata_cache_update_metadata_batch_invalidates_query_cache(
    postgres_cache_metadata_service: &Service,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::{batch_error_entry::Code, *};

    let client = postgres_cache_metadata_service.metadata_v1alpha_client(interface).await;
    let metrics = postgres_cache_metadata_service.metrics_client();
    let resource_uri_1 = create_resource_uri();
    let resource_uri_2 = create_resource_uri();

    // Query metdata for both of the resource addresses. We must observe two queries.
    let before_query = metrics.get_postgres_metadata_backend_metrics().await;
    let query_result_1 = client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::resource_address(&resource_uri_1)),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(query_result_1, QueryMetadataResponse::default());

    let query_result_2 = client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::resource_address(&resource_uri_2)),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(query_result_2, QueryMetadataResponse::default());

    let after_query_1 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_query_1,
        PostgresMetadataBackendMetrics {
            query_all_metadata_for_resource_count: before_query.query_all_metadata_for_resource_count + 2,
            ..before_query
        }
    );

    // Update arbitrary metadata keys for both of the resources. Cache values must be invalidated for
    // the successfully completed operations only.
    let update_result = client
        .update_metadata_batch_sorted(UpdateMetadataBatchRequest {
            requests: vec![
                UpdateMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri_1)),
                    user_metadata_key: "a".into(),
                    user_metadata: Some(create_string_value("a")),
                    ..Default::default()
                },
                UpdateMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri_1)),
                    user_metadata_key: "b".into(),
                    user_metadata: Some(create_string_value("b")),
                    expected_etag: Some("etag".into()),
                    ..Default::default()
                },
            ],
        })
        .await
        .unwrap();

    assert_eq!(update_result.results.len(), 2);
    assert!(!update_result.results[0].as_success().etag.is_empty());
    assert_eq!(
        update_result.results[1].as_error().code,
        Code::FailedPrecondition as i32
    );

    let after_update_2 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_update_2,
        PostgresMetadataBackendMetrics {
            update_metadata_batch_count: after_query_1.update_metadata_batch_count + 1,
            upsert_metadata_batch_count: after_query_1.upsert_metadata_batch_count + 1,
            ..after_query_1
        }
    );

    // Query metadata for the unaffected resource. We must observe no new queries.
    let query_result_3 = client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::resource_address(&resource_uri_2)),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(query_result_3, QueryMetadataResponse::default());

    let after_query_3 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(after_query_3, after_update_2);

    // Query metadata for the affected resource. We must observe a query.
    let query_result_4 = client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::resource_address(&resource_uri_1)),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(
        query_result_4,
        QueryMetadataResponse {
            results: vec![create_query_metadata_response_entry(
                "a",
                create_string_value("a"),
                &update_result.results[0].as_success().etag
            ),]
        }
    );

    let after_query_4 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_query_4,
        PostgresMetadataBackendMetrics {
            query_all_metadata_for_resource_count: after_query_3.query_all_metadata_for_resource_count + 1,
            ..after_query_3
        }
    );
}

#[rstest]
#[serial(postgres_cache)]
#[test_log::test(tokio::test)]
async fn test_metadata_cache_delete_metadata_batch_invalidates_key_cache(
    postgres_cache_metadata_service: &Service,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let client = postgres_cache_metadata_service.metadata_v1alpha_client(interface).await;
    let metrics = postgres_cache_metadata_service.metrics_client();
    let resource_uri_1 = create_resource_uri();
    let resource_uri_2 = create_resource_uri();

    let (value_1, etag_1) = client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri_1), "a")
        .await
        .unwrap();
    let (value_2, etag_2) = client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri_2), "b")
        .await
        .unwrap();

    // Populate cache keys. We must observe a query.
    let before_read = metrics.get_postgres_metadata_backend_metrics().await;
    let get_response_1 = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_1, "a"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_2, "b"),
            ],
        })
        .await
        .unwrap();

    assert_eq!(
        get_response_1,
        GetMetadataBatchResponse {
            results: vec![
                create_get_metadata_batch_success_response_entry(0, value_1.clone(), &etag_1),
                create_get_metadata_batch_success_response_entry(1, value_2.clone(), &etag_2),
            ]
        }
    );

    let after_read_1 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_read_1,
        PostgresMetadataBackendMetrics {
            query_metadata_batch_count: before_read.query_metadata_batch_count + 1,
            ..before_read
        }
    );

    // Delete key values. We must observe a delete query.
    let delete_response = client
        .delete_metadata_batch_sorted(DeleteMetadataBatchRequest {
            requests: vec![
                DeleteMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri_1)),
                    user_metadata_key: "a".into(),
                    expected_etag: Some("etag".into()),
                    ..Default::default()
                },
                DeleteMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri_2)),
                    user_metadata_key: "b".into(),
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
                create_delete_metadata_metadata_modified_response_entry(0),
                create_delete_metadata_batch_success_response_entry(1),
            ]
        }
    );

    let after_write_2 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_write_2,
        PostgresMetadataBackendMetrics {
            delete_metadata_batch_count: after_read_1.delete_metadata_batch_count + 1,
            ..after_read_1
        }
    );

    // Fetch metadata for the unaffected key. We must observe no new queries.
    let get_response_2 = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![create_get_metadata_batch_resource_address_request_entry(
                &resource_uri_1,
                "a",
            )],
        })
        .await
        .unwrap();

    assert_eq!(
        get_response_2,
        GetMetadataBatchResponse {
            results: vec![create_get_metadata_batch_success_response_entry(
                0,
                value_1.clone(),
                &etag_1
            ),]
        }
    );

    let after_read_3 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(after_read_3, after_write_2);

    // Fetch metadata for the affected key. We must observe a query.
    let get_response_3 = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![create_get_metadata_batch_resource_address_request_entry(
                &resource_uri_2,
                "b",
            )],
        })
        .await
        .unwrap();

    assert_eq!(
        get_response_3,
        GetMetadataBatchResponse {
            results: vec![create_get_metadata_batch_not_found_response_entry(0)]
        }
    );

    let after_read_4 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_read_4,
        PostgresMetadataBackendMetrics {
            query_metadata_batch_count: after_read_3.query_metadata_batch_count + 1,
            ..after_read_3
        }
    );
}

#[rstest]
#[serial(postgres_cache)]
#[test_log::test(tokio::test)]
async fn test_metadata_cache_delete_metadata_batch_invalidates_query_cache(
    postgres_cache_metadata_service: &Service,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let client = postgres_cache_metadata_service.metadata_v1alpha_client(interface).await;
    let metrics = postgres_cache_metadata_service.metrics_client();
    let resource_uri_1 = create_resource_uri();
    let resource_uri_2 = create_resource_uri();

    let (value_1, etag_1) = client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri_1), "a")
        .await
        .unwrap();
    let (value_2, etag_2) = client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri_2), "b")
        .await
        .unwrap();

    // Populate caches. We must observe a query per request.
    let before_read = metrics.get_postgres_metadata_backend_metrics().await;
    let query_response_1 = client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::resource_address(&resource_uri_1)),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(
        query_response_1,
        QueryMetadataResponse {
            results: vec![create_query_metadata_response_entry("a", value_1.clone(), &etag_1)]
        }
    );

    let query_response_2 = client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::resource_address(&resource_uri_2)),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(
        query_response_2,
        QueryMetadataResponse {
            results: vec![create_query_metadata_response_entry("b", value_2.clone(), &etag_2)]
        }
    );

    let after_read_1 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_read_1,
        PostgresMetadataBackendMetrics {
            query_all_metadata_for_resource_count: before_read.query_all_metadata_for_resource_count + 2,
            ..before_read
        }
    );

    // Delete metadata keys. Caches must be invalidated for successfully updated keys only.
    let delete_response = client
        .delete_metadata_batch_sorted(DeleteMetadataBatchRequest {
            requests: vec![
                DeleteMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri_1)),
                    user_metadata_key: "a".into(),
                    ..Default::default()
                },
                DeleteMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri_2)),
                    user_metadata_key: "b".into(),
                    expected_etag: Some("etag".into()),
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
                create_delete_metadata_batch_success_response_entry(0),
                create_delete_metadata_metadata_modified_response_entry(1),
            ]
        }
    );

    let after_write_2 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_write_2,
        PostgresMetadataBackendMetrics {
            delete_metadata_batch_count: after_read_1.delete_metadata_batch_count + 1,
            ..after_read_1
        }
    );

    // Query metadata for the unaffected resource. We must observe no new queries.
    let query_response_3 = client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::resource_address(&resource_uri_2)),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(
        query_response_3,
        QueryMetadataResponse {
            results: vec![create_query_metadata_response_entry("b", value_2.clone(), &etag_2)]
        }
    );

    let after_read_3 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(after_read_3, after_write_2);

    // Query metadata for the affected resource. We must observe a query.
    let query_response_3 = client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::resource_address(&resource_uri_1)),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(query_response_3, QueryMetadataResponse::default());

    let after_read_4 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_read_4,
        PostgresMetadataBackendMetrics {
            query_all_metadata_for_resource_count: after_read_3.query_all_metadata_for_resource_count + 1,
            ..after_read_3
        }
    );
}

#[rstest]
#[serial(postgres_cache)]
#[test_log::test(tokio::test)]
async fn test_metadata_cache_get_metadata_batch_respects_namespace(postgres_cache_metadata_service: &Service) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let client = postgres_cache_metadata_service
        .metadata_v1alpha_client(Interface::Grpc)
        .await;
    let metrics = postgres_cache_metadata_service.metrics_client();
    let resource_uri = create_resource_uri();

    let update_response = client
        .update_metadata_batch_sorted(UpdateMetadataBatchRequest {
            requests: vec![
                UpdateMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "a".into(),
                    user_metadata: Some(create_string_value("a-default")),
                    ..Default::default()
                },
                UpdateMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "a".into(),
                    user_metadata: Some(create_string_value("a-custom")),
                    namespace: Some("custom".into()),
                    ..Default::default()
                },
            ],
        })
        .await
        .unwrap();

    assert_eq!(update_response.results.len(), 2);
    assert!(!update_response.results[0].as_success().etag.is_empty());
    assert!(!update_response.results[1].as_success().etag.is_empty());

    let before_read = metrics.get_postgres_metadata_backend_metrics().await;
    let get_response_1 = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                GetMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "a".into(),
                    ..Default::default()
                },
                GetMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "a".into(),
                    namespace: Some("custom".into()),
                },
            ],
        })
        .await
        .unwrap();

    assert_eq!(
        get_response_1,
        GetMetadataBatchResponse {
            results: vec![
                create_get_metadata_batch_success_response_entry(
                    0,
                    create_string_value("a-default"),
                    &update_response.results[0].as_success().etag
                ),
                create_get_metadata_batch_success_response_entry(
                    1,
                    create_string_value("a-custom"),
                    &update_response.results[1].as_success().etag
                ),
            ]
        }
    );

    let after_read_1 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_read_1,
        PostgresMetadataBackendMetrics {
            query_metadata_batch_count: before_read.query_metadata_batch_count + 1,
            ..before_read
        }
    );

    let get_response_2 = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                GetMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "a".into(),
                    namespace: Some("default".into()),
                },
                GetMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "a".into(),
                    namespace: Some("custom".into()),
                },
            ],
        })
        .await
        .unwrap();

    assert_eq!(get_response_2, get_response_1);

    let after_read_2 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(after_read_2, after_read_1);
}

#[rstest]
#[case(0, None, Some("default"), "a-default")]
#[case(0, Some("default"), None, "a-default")]
#[case(1, Some("custom"), Some("custom"), "a-custom")]
#[serial(postgres_cache)]
#[test_log::test(tokio::test)]
async fn test_metadata_cache_query_metadata_respects_namespace(
    postgres_cache_metadata_service: &Service,
    #[case] update_response_idx: usize,
    #[case] namespace_1: Option<&str>,
    #[case] namespace_2: Option<&str>,
    #[case] expected_value: &str,
) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let client = postgres_cache_metadata_service
        .metadata_v1alpha_client(Interface::Grpc)
        .await;
    let metrics = postgres_cache_metadata_service.metrics_client();
    let resource_uri = create_resource_uri();

    let update_response = client
        .update_metadata_batch_sorted(UpdateMetadataBatchRequest {
            requests: vec![
                UpdateMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "a".into(),
                    user_metadata: Some(create_string_value("a-default")),
                    ..Default::default()
                },
                UpdateMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "a".into(),
                    user_metadata: Some(create_string_value("a-custom")),
                    namespace: Some("custom".into()),
                    ..Default::default()
                },
            ],
        })
        .await
        .unwrap();

    assert_eq!(update_response.results.len(), 2);
    assert!(!update_response.results[0].as_success().etag.is_empty());
    assert!(!update_response.results[1].as_success().etag.is_empty());

    let before_query = metrics.get_postgres_metadata_backend_metrics().await;
    let query_response_1 = client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
            namespace: namespace_1.map(Into::into),
        })
        .await
        .unwrap();

    assert_eq!(
        query_response_1,
        QueryMetadataResponse {
            results: vec![create_query_metadata_response_entry(
                "a",
                create_string_value(expected_value),
                &update_response.results[update_response_idx].as_success().etag,
            ),]
        }
    );

    let after_query_1 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_query_1,
        PostgresMetadataBackendMetrics {
            query_all_metadata_for_resource_count: before_query.query_all_metadata_for_resource_count + 1,
            ..before_query
        }
    );

    let query_response_2 = client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
            namespace: namespace_2.map(Into::into),
        })
        .await
        .unwrap();

    assert_eq!(query_response_1, query_response_2);

    let after_query_2 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(after_query_2, after_query_1);
}

#[rstest]
#[case(0, None, "custom")]
#[case(0, Some("default"), "custom")]
#[case(0, Some("custom"), "default")]
#[serial(postgres_cache)]
#[test_log::test(tokio::test)]
async fn test_metadata_cache_update_metadata_batch_respects_namespace(
    postgres_cache_metadata_service: &Service,
    #[case] update_request_idx: usize,
    #[case] update_namespace: Option<&str>,
    #[case] get_namespace: &str,
) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let client = postgres_cache_metadata_service
        .metadata_v1alpha_client(Interface::Grpc)
        .await;
    let metrics = postgres_cache_metadata_service.metrics_client();
    let resource_uri = create_resource_uri();

    let get_response_1 = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                GetMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "a".into(),
                    ..Default::default()
                },
                GetMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "a".into(),
                    namespace: Some("custom".into()),
                },
            ],
        })
        .await
        .unwrap();

    assert_eq!(
        get_response_1,
        GetMetadataBatchResponse {
            results: vec![
                create_get_metadata_batch_not_found_response_entry(0),
                create_get_metadata_batch_not_found_response_entry(1),
            ]
        }
    );

    let before_write = metrics.get_postgres_metadata_backend_metrics().await;
    let update_response = client
        .update_metadata_batch_sorted(UpdateMetadataBatchRequest {
            requests: vec![UpdateMetadataBatchRequestEntry {
                resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                user_metadata_key: "a".into(),
                user_metadata: Some(create_string_value("a")),
                namespace: update_namespace.map(Into::into),
                ..Default::default()
            }],
        })
        .await
        .unwrap();

    assert_eq!(update_response.results.len(), 1);
    assert!(!update_response.results[0].as_success().etag.is_empty());

    let after_write_1 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_write_1,
        PostgresMetadataBackendMetrics {
            upsert_metadata_batch_count: before_write.upsert_metadata_batch_count + 1,
            ..before_write
        }
    );

    let get_response_2 = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![GetMetadataBatchRequestEntry {
                resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                namespace: Some(get_namespace.into()),
                user_metadata_key: "a".into(),
            }],
        })
        .await
        .unwrap();

    assert_eq!(
        get_response_2,
        GetMetadataBatchResponse {
            results: vec![create_get_metadata_batch_not_found_response_entry(0)]
        }
    );

    let after_read_2 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(after_read_2, after_write_1);

    let get_response_3 = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![GetMetadataBatchRequestEntry {
                resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                namespace: update_namespace.map(Into::into),
                user_metadata_key: "a".into(),
            }],
        })
        .await
        .unwrap();

    assert_eq!(
        get_response_3,
        GetMetadataBatchResponse {
            results: vec![create_get_metadata_batch_success_response_entry(
                0,
                create_string_value("a"),
                &update_response.results[update_request_idx].as_success().etag,
            )]
        }
    );

    let after_read_3 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_read_3,
        PostgresMetadataBackendMetrics {
            query_metadata_batch_count: after_read_2.query_metadata_batch_count + 1,
            ..after_read_2
        }
    );
}

#[rstest]
#[case(None, "custom")]
#[case(Some("default"), "custom")]
#[case(Some("custom"), "default")]
#[serial(postgres_cache)]
#[test_log::test(tokio::test)]
async fn test_metadata_cache_delete_metadata_batch_respects_namespace(
    postgres_cache_metadata_service: &Service,
    #[case] delete_namespace: Option<&str>,
    #[case] get_namespace: &str,
) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let client = postgres_cache_metadata_service
        .metadata_v1alpha_client(Interface::Grpc)
        .await;
    let metrics = postgres_cache_metadata_service.metrics_client();
    let resource_uri = create_resource_uri();

    let get_response_1 = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                GetMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "a".into(),
                    ..Default::default()
                },
                GetMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                    user_metadata_key: "a".into(),
                    namespace: Some("custom".into()),
                },
            ],
        })
        .await
        .unwrap();

    assert_eq!(
        get_response_1,
        GetMetadataBatchResponse {
            results: vec![
                create_get_metadata_batch_not_found_response_entry(0),
                create_get_metadata_batch_not_found_response_entry(1),
            ]
        }
    );

    let before_write = metrics.get_postgres_metadata_backend_metrics().await;
    let delete_response = client
        .delete_metadata_batch_sorted(DeleteMetadataBatchRequest {
            requests: vec![DeleteMetadataBatchRequestEntry {
                resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                user_metadata_key: "a".into(),
                namespace: delete_namespace.map(Into::into),
                ..Default::default()
            }],
        })
        .await
        .unwrap();

    assert_eq!(
        delete_response,
        DeleteMetadataBatchResponse {
            results: vec![create_delete_metadata_batch_success_response_entry(0)],
        }
    );

    let after_write_1 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_write_1,
        PostgresMetadataBackendMetrics {
            delete_metadata_batch_count: before_write.delete_metadata_batch_count + 1,
            ..before_write
        }
    );

    let get_response_2 = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![GetMetadataBatchRequestEntry {
                resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                namespace: Some(get_namespace.into()),
                user_metadata_key: "a".into(),
            }],
        })
        .await
        .unwrap();

    assert_eq!(
        get_response_2,
        GetMetadataBatchResponse {
            results: vec![create_get_metadata_batch_not_found_response_entry(0)]
        }
    );

    let after_read_2 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(after_read_2, after_write_1);

    let get_response_3 = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![GetMetadataBatchRequestEntry {
                resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
                namespace: delete_namespace.map(Into::into),
                user_metadata_key: "a".into(),
            }],
        })
        .await
        .unwrap();

    assert_eq!(
        get_response_3,
        GetMetadataBatchResponse {
            results: vec![create_get_metadata_batch_not_found_response_entry(0),]
        }
    );

    let after_read_3 = metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_read_3,
        PostgresMetadataBackendMetrics {
            query_metadata_batch_count: after_read_2.query_metadata_batch_count + 1,
            ..after_read_2
        }
    );
}

#[rstest]
#[case("", "?a=b&c=d", "?etag=abcd")]
#[case("?etag=abcd", "?a=b&etag=abcd&c=d", "")]
#[case("?etag=abcd", "?a=b&etag=%61%62%63%64&c=d", "")]
#[test_log::test(tokio::test)]
async fn test_get_metadata_canonicalizes_uris(
    canon_metadata_service: &Service,
    #[case] suffix_canon: &str,
    #[case] suffix_match: &str,
    #[case] suffix_mismatch: &str,
) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let client = canon_metadata_service.metadata_v1alpha_client(Interface::Grpc).await;
    let resource_uri_base = create_resource_uri();
    let resource_uri_canon = format!("{resource_uri_base}{suffix_canon}");
    let resource_uri_match = format!("{resource_uri_base}{suffix_match}");
    let resource_uri_mismatch = format!("{resource_uri_base}{suffix_mismatch}");

    // Update metadata for the canonical resource uri.
    let update_response = client
        .update_metadata(UpdateMetadataRequest {
            uri: resource_uri_canon,
            user_metadata_key: "a".into(),
            user_metadata: Some(create_string_value("a")),
            ..Default::default()
        })
        .await
        .unwrap();

    // Fetch metadata for a uri that resolves to the canonical resource uri. We must fetch the
    // same value as we've just written.
    let get_response_match = client
        .get_metadata(GetMetadataRequest {
            uri: resource_uri_match,
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(
        get_response_match,
        GetMetadataResponse {
            user_metadata: [create_user_metadata_kv(
                "a",
                create_string_value("a"),
                update_response.etag
            )]
            .into()
        }
    );

    // Fetch metadata for a uri that resolves to a different canonical form. We must fetch nothing.
    let get_response_mismatch = client
        .get_metadata(GetMetadataRequest {
            uri: resource_uri_mismatch,
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(get_response_mismatch, GetMetadataResponse::default());
}

#[rstest]
#[case("", "?a=b&c=d", "?etag=abcd")]
#[case("?etag=abcd", "?a=b&etag=abcd&c=d", "")]
#[case("?etag=abcd", "?a=b&etag=%61%62%63%64&c=d", "")]
#[test_log::test(tokio::test)]
async fn test_update_metadata_canonicalizes_uris(
    canon_metadata_service: &Service,
    #[case] suffix_canon: &str,
    #[case] suffix_match: &str,
    #[case] suffix_mismatch: &str,
) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let client = canon_metadata_service.metadata_v1alpha_client(Interface::Grpc).await;
    let resource_uri_base = create_resource_uri();
    let resource_uri_canon = format!("{resource_uri_base}{suffix_canon}");
    let resource_uri_match = format!("{resource_uri_base}{suffix_match}");
    let resource_uri_mismatch = format!("{resource_uri_base}{suffix_mismatch}");

    // Update metadata for a uri that resolves to the canonical form.
    let update_response_match = client
        .update_metadata(UpdateMetadataRequest {
            uri: resource_uri_match,
            user_metadata_key: "a".into(),
            user_metadata: Some(create_string_value("a-match")),
            ..Default::default()
        })
        .await
        .unwrap();

    // Update metadata for a uri that resolves to a different canonical form.
    client
        .update_metadata(UpdateMetadataRequest {
            uri: resource_uri_mismatch,
            user_metadata_key: "a".into(),
            user_metadata: Some(create_string_value("a-mismatch")),
            ..Default::default()
        })
        .await
        .unwrap();

    // Query metadata for the canonical resource uri. We must receive values from the first
    // update call.
    let get_response_canon = client
        .get_metadata(GetMetadataRequest {
            uri: resource_uri_canon,
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(
        get_response_canon,
        GetMetadataResponse {
            user_metadata: [create_user_metadata_kv(
                "a",
                create_string_value("a-match"),
                update_response_match.etag
            )]
            .into()
        }
    );
}

#[rstest]
#[case("", "?a=b&c=d", "?etag=abcd")]
#[case("?etag=abcd", "?a=b&etag=abcd&c=d", "")]
#[case("?etag=abcd", "?a=b&etag=%61%62%63%64&c=d", "")]
#[test_log::test(tokio::test)]
async fn test_delete_metadata_canonicalizes_uris(
    canon_metadata_service: &Service,
    #[case] suffix_canon: &str,
    #[case] suffix_match: &str,
    #[case] suffix_mismatch: &str,
) {
    use crate::common::clients::metadata::types::v1alpha::*;
    use tonic::Code;

    let client = canon_metadata_service.metadata_v1alpha_client(Interface::Grpc).await;
    let resource_uri_base = create_resource_uri();
    let resource_uri_canon = format!("{resource_uri_base}{suffix_canon}");
    let resource_uri_match = format!("{resource_uri_base}{suffix_match}");
    let resource_uri_mismatch = format!("{resource_uri_base}{suffix_mismatch}");

    // Update metadata for the canonical form.
    let update_response_match = client
        .update_metadata(UpdateMetadataRequest {
            uri: resource_uri_canon,
            user_metadata_key: "a".into(),
            user_metadata: Some(create_string_value("a")),
            ..Default::default()
        })
        .await
        .unwrap();

    // Conditionally delete metadata for a uri that resolves to a different canonical form.
    // The request must fail because the value does not exist.
    let delete_error_mismatch = client
        .delete_metadata(DeleteMetadataRequest {
            uri: resource_uri_mismatch,
            user_metadata_key: "a".into(),
            expected_etag: Some(update_response_match.etag.clone()),
        })
        .await
        .unwrap_err();
    assert_eq!(delete_error_mismatch.code(), Code::FailedPrecondition);

    // Conditionally delete metadata for a uri that resolves to the canonical form. The request
    // must succeed.
    client
        .delete_metadata(DeleteMetadataRequest {
            uri: resource_uri_match,
            user_metadata_key: "a".into(),
            expected_etag: Some(update_response_match.etag),
        })
        .await
        .unwrap();
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_batch_get_metadata_canonicalizes_uris(canon_metadata_service: &Service) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let client = canon_metadata_service.metadata_v1alpha_client(Interface::Grpc).await;
    let resource_address_canon = create_resource_uri();
    let resource_identity_canon = format!("{resource_address_canon}?etag=abcd");
    let folder_address_canon = format!("{resource_address_canon}/");

    let (value_ra, etag_ra) = client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_address_canon), "a")
        .await
        .unwrap();
    let (value_ri, etag_ri) = client
        .set_random_metadata_value(ResourceLocation::resource_identity(&resource_identity_canon), "a")
        .await
        .unwrap();
    let (value_fa, etag_fa) = client
        .set_random_metadata_value(ResourceLocation::folder_address(&folder_address_canon), "a")
        .await
        .unwrap();

    let response = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                create_get_metadata_batch_resource_address_request_entry(&resource_address_canon, "a"),
                create_get_metadata_batch_resource_identity_request_entry(&resource_address_canon, "a"),
                create_get_metadata_batch_folder_address_request_entry(&resource_address_canon, "a"),
                create_get_metadata_batch_resource_address_request_entry(&resource_identity_canon, "a"),
                create_get_metadata_batch_resource_identity_request_entry(&resource_identity_canon, "a"),
                create_get_metadata_batch_folder_address_request_entry(&resource_identity_canon, "a"),
                create_get_metadata_batch_resource_address_request_entry(&folder_address_canon, "a"),
                create_get_metadata_batch_resource_identity_request_entry(&folder_address_canon, "a"),
                create_get_metadata_batch_folder_address_request_entry(&folder_address_canon, "a"),
                create_get_metadata_batch_folder_address_request_entry(
                    format!("{folder_address_canon}?etag=abcd"),
                    "a",
                ),
                create_get_metadata_batch_folder_address_request_entry(
                    format!("{resource_address_canon}?etag=abcd"),
                    "a",
                ),
                create_get_metadata_batch_resource_identity_request_entry(
                    format!("{resource_address_canon}?a=b&etag=%61%62%63%64&c=d"),
                    "a",
                ),
            ],
        })
        .await
        .unwrap();

    assert_eq!(
        response,
        GetMetadataBatchResponse {
            results: vec![
                create_get_metadata_batch_success_response_entry(0, value_ra.clone(), &etag_ra),
                create_get_metadata_batch_resource_uri_invalid_response_entry(1),
                create_get_metadata_batch_success_response_entry(2, value_fa.clone(), &etag_fa),
                create_get_metadata_batch_success_response_entry(3, value_ra.clone(), &etag_ra),
                create_get_metadata_batch_success_response_entry(4, value_ri.clone(), &etag_ri),
                create_get_metadata_batch_success_response_entry(5, value_fa.clone(), &etag_fa),
                create_get_metadata_batch_success_response_entry(6, value_fa.clone(), &etag_fa),
                create_get_metadata_batch_resource_uri_invalid_response_entry(7),
                create_get_metadata_batch_success_response_entry(8, value_fa.clone(), &etag_fa),
                create_get_metadata_batch_success_response_entry(9, value_fa.clone(), &etag_fa),
                create_get_metadata_batch_success_response_entry(10, value_fa.clone(), &etag_fa),
                create_get_metadata_batch_success_response_entry(11, value_ri.clone(), &etag_ri),
            ]
        },
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_query_metadata_canonicalizes_uris(canon_metadata_service: &Service) {
    use crate::common::clients::metadata::types::v1alpha::*;

    // Parameters in the operation undergo the same normalization rules as in the test scenario for the
    // get metadata batch operation. Hence, we reduce test scenarios to just one.
    let client = canon_metadata_service.metadata_v1alpha_client(Interface::Grpc).await;
    let resource_address = create_resource_uri();
    let resource_identity_canon = format!("{resource_address}?etag=abcd");
    let resource_identity_match = format!("{resource_address}?etag=%61%62%63%64&xq=d");

    let (value_1, etag_1) = client
        .set_random_metadata_value(ResourceLocation::resource_identity(&resource_identity_canon), "a")
        .await
        .unwrap();
    let (value_2, etag_2) = client
        .set_random_metadata_value(ResourceLocation::resource_identity(&resource_identity_canon), "b")
        .await
        .unwrap();

    let query_response_1 = client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::resource_identity(&resource_identity_match)),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(
        query_response_1,
        QueryMetadataResponse {
            results: vec![
                create_query_metadata_response_entry("a", value_1, etag_1),
                create_query_metadata_response_entry("b", value_2, etag_2),
            ]
        }
    );

    let query_response_2 = client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::resource_address(&resource_identity_match)),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(query_response_2, QueryMetadataResponse::default());
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_query_metadata_errors_on_invalid_resource_uri(canon_metadata_service: &Service) {
    use crate::common::clients::metadata::types::v1alpha::*;
    use tonic::Code;

    let client = canon_metadata_service.metadata_v1alpha_client(Interface::Grpc).await;

    let query_error = client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::resource_identity(create_resource_uri())),
            ..Default::default()
        })
        .await
        .unwrap_err();

    assert_eq!(query_error.code(), Code::InvalidArgument);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_batch_update_metadata_canonicalizes_uris(canon_metadata_service: &Service) {
    use crate::common::clients::metadata::types::v1alpha::{batch_error_entry::Code, *};

    // Parameters in the operation undergo the same normalization rules as in the test scenario for the
    // get metadata batch operation. Hence, we reduce test scenarios to just one.
    let client = canon_metadata_service.metadata_v1alpha_client(Interface::Grpc).await;
    let resource_uri_1 = create_resource_uri();
    let resource_uri_2 = create_resource_uri();
    let resource_uri_3 = create_resource_uri();

    let update_response = client
        .update_metadata_batch_sorted(UpdateMetadataBatchRequest {
            requests: vec![
                create_update_metadata_batch_resource_identity_request_entry(
                    &resource_uri_1,
                    "a",
                    create_string_value("a"),
                ),
                create_update_metadata_batch_resource_identity_request_entry(
                    format!("{resource_uri_2}?a=b&etag=1234"),
                    "b",
                    create_string_value("b"),
                ),
                create_update_metadata_batch_resource_identity_request_entry(
                    format!("{resource_uri_3}?etag=%61%62%63%64"),
                    "c",
                    create_string_value("c"),
                ),
                create_update_metadata_batch_resource_identity_request_entry(
                    &resource_uri_2,
                    "d",
                    create_string_value("d"),
                ),
            ],
        })
        .await
        .unwrap();

    assert_eq!(update_response.results.len(), 4);
    assert_eq!(update_response.results[0].as_error().code, Code::InvalidArgument as i32);
    assert!(!update_response.results[1].as_success().etag.is_empty());
    assert!(!update_response.results[2].as_success().etag.is_empty());
    assert_eq!(update_response.results[3].as_error().code, Code::InvalidArgument as i32);

    let get_response = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_1, "a"),
                create_get_metadata_batch_resource_identity_request_entry(format!("{resource_uri_2}?etag=1234"), "b"),
                create_get_metadata_batch_resource_identity_request_entry(format!("{resource_uri_3}?etag=abcd"), "c"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_2, "d"),
            ],
        })
        .await
        .unwrap();

    assert_eq!(
        get_response,
        GetMetadataBatchResponse {
            results: vec![
                create_get_metadata_batch_not_found_response_entry(0),
                create_get_metadata_batch_success_response_entry(
                    1,
                    create_string_value("b"),
                    &update_response.results[1].as_success().etag
                ),
                create_get_metadata_batch_success_response_entry(
                    2,
                    create_string_value("c"),
                    &update_response.results[2].as_success().etag
                ),
                create_get_metadata_batch_not_found_response_entry(3),
            ],
        }
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_batch_delete_metadata_canonicalizes_uris(canon_metadata_service: &Service) {
    use crate::common::clients::metadata::types::v1alpha::*;

    // Parameters in the operation undergo the same normalization rules as in the test scenario for the
    // get metadata batch operation. Hence, we reduce test scenarios to just one.
    let client = canon_metadata_service.metadata_v1alpha_client(Interface::Grpc).await;
    let resource_uri_1 = create_resource_uri();
    let resource_uri_2 = create_resource_uri();

    let (_value_1, etag_1) = client
        .set_random_metadata_value(
            ResourceLocation::resource_identity(format!("{resource_uri_1}?etag=1234")),
            "a",
        )
        .await
        .unwrap();
    let (_value_2, etag_2) = client
        .set_random_metadata_value(
            ResourceLocation::resource_identity(format!("{resource_uri_2}?etag=abcd")),
            "b",
        )
        .await
        .unwrap();

    let delete_response = client
        .delete_metadata_batch_sorted(DeleteMetadataBatchRequest {
            requests: vec![
                DeleteMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_identity(&resource_uri_1)),
                    user_metadata_key: "a".into(),
                    ..Default::default()
                },
                DeleteMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_identity(format!(
                        "{resource_uri_1}?a=b&etag=1234"
                    ))),
                    user_metadata_key: "a".into(),
                    expected_etag: Some(etag_1),
                    ..Default::default()
                },
                DeleteMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_identity(format!(
                        "{resource_uri_2}?etag=%61%62%63%64"
                    ))),
                    user_metadata_key: "b".into(),
                    expected_etag: Some(etag_2),
                    ..Default::default()
                },
                DeleteMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_identity(resource_uri_2)),
                    user_metadata_key: "c".into(),
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
                create_delete_metadata_resource_uri_invalid_response_entry(0),
                create_delete_metadata_batch_success_response_entry(1),
                create_delete_metadata_batch_success_response_entry(2),
                create_delete_metadata_resource_uri_invalid_response_entry(3),
            ],
        },
    );
}

#[rstest]
#[serial(redis_postgres)]
#[test_log::test(tokio::test)]
async fn test_redis_metadata_cache_get_persists_get_results(postgres_redis_metadata_service: &RedisCacheServiceSet) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let wr_client = postgres_redis_metadata_service
        .wr_service
        .metadata_v1alpha_client(Interface::Grpc)
        .await;
    let rd_client = postgres_redis_metadata_service
        .rd_service
        .metadata_v1alpha_client(Interface::Grpc)
        .await;
    let rd_metrics = postgres_redis_metadata_service.rd_service.metrics_client();

    let resource_uri_1 = create_resource_uri();
    let resource_uri_2 = create_resource_uri();
    let resource_uri_3 = create_resource_uri();
    let (value_1, etag_1) = wr_client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri_1), "a")
        .await
        .unwrap();
    let (value_2, etag_2) = wr_client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri_2), "a")
        .await
        .unwrap();
    let (value_3, etag_3) = wr_client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri_3), "b")
        .await
        .unwrap();

    // Warm-up the redis cache.
    let before_write = rd_metrics.get_postgres_metadata_backend_metrics().await;
    let get_response_1 = wr_client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_1, "a"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_1, "b"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_2, "a"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_2, "b"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_3, "a"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_3, "b"),
            ],
        })
        .await
        .unwrap();

    assert_eq!(
        get_response_1,
        GetMetadataBatchResponse {
            results: vec![
                create_get_metadata_batch_success_response_entry(0, value_1.clone(), &etag_1),
                create_get_metadata_batch_not_found_response_entry(1),
                create_get_metadata_batch_success_response_entry(2, value_2.clone(), &etag_2),
                create_get_metadata_batch_not_found_response_entry(3),
                create_get_metadata_batch_not_found_response_entry(4),
                create_get_metadata_batch_success_response_entry(5, value_3.clone(), &etag_3),
            ]
        }
    );

    let after_read_1 = rd_metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(after_read_1, before_write);

    // Read metadata. Results must be resolved from the cache.
    let get_response_2 = rd_client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_1, "a"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_1, "b"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_2, "a"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_2, "b"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_3, "a"),
                create_get_metadata_batch_resource_address_request_entry(&resource_uri_3, "b"),
            ],
        })
        .await
        .unwrap();

    assert_eq!(get_response_2, get_response_1);

    let after_read_2 = rd_metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(after_read_2, after_read_1);

    // Read a non-cached value. We must observe a query.
    let get_response_3 = rd_client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![create_get_metadata_batch_resource_address_request_entry(
                &resource_uri_1,
                "c",
            )],
        })
        .await
        .unwrap();

    assert_eq!(
        get_response_3,
        GetMetadataBatchResponse {
            results: vec![create_get_metadata_batch_not_found_response_entry(0)]
        }
    );

    let after_read_3 = rd_metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_read_3,
        PostgresMetadataBackendMetrics {
            query_metadata_batch_count: after_read_2.query_metadata_batch_count + 1,
            ..after_read_2
        }
    );
}

#[rstest]
#[serial(redis_postgres)]
#[test_log::test(tokio::test)]
async fn test_redis_metadata_cache_query_persists_query_results(
    postgres_redis_metadata_service: &RedisCacheServiceSet,
) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let wr_client = postgres_redis_metadata_service
        .wr_service
        .metadata_v1alpha_client(Interface::Grpc)
        .await;
    let rd_client = postgres_redis_metadata_service
        .rd_service
        .metadata_v1alpha_client(Interface::Grpc)
        .await;
    let rd_metrics = postgres_redis_metadata_service.rd_service.metrics_client();

    let resource_uri = create_resource_uri();
    let (value_1, etag_1) = wr_client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri), "a")
        .await
        .unwrap();
    let (value_2, etag_2) = wr_client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri), "b")
        .await
        .unwrap();

    // Warm-up the L2 cache.
    let before_read = rd_metrics.get_postgres_metadata_backend_metrics().await;
    let query_response_1 = wr_client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(
        query_response_1,
        QueryMetadataResponse {
            results: vec![
                create_query_metadata_response_entry("a", value_1, etag_1),
                create_query_metadata_response_entry("b", value_2, etag_2),
            ]
        }
    );

    let after_read_1 = rd_metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(after_read_1, before_read);

    // Query metadata for the resource. The results must be resolved from the cache.
    let query_response_2 = rd_client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(query_response_2, query_response_1);

    let after_read_2 = rd_metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(after_read_2, after_read_1);
}

#[rstest]
#[serial(redis_postgres)]
#[test_log::test(tokio::test)]
async fn test_redis_metadata_cache_get_ignores_persisted_query_results(
    postgres_redis_metadata_service: &RedisCacheServiceSet,
) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let wr_client = postgres_redis_metadata_service
        .wr_service
        .metadata_v1alpha_client(Interface::Grpc)
        .await;
    let rd_client = postgres_redis_metadata_service
        .rd_service
        .metadata_v1alpha_client(Interface::Grpc)
        .await;
    let rd_metrics = postgres_redis_metadata_service.rd_service.metrics_client();

    let resource_uri = create_resource_uri();
    let (value, etag) = wr_client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri), "a")
        .await
        .unwrap();

    // Warm-up the L2 cache.
    let before_read = rd_metrics.get_postgres_metadata_backend_metrics().await;
    let query_response = wr_client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_eq!(
        query_response,
        QueryMetadataResponse {
            results: vec![create_query_metadata_response_entry("a", value.clone(), &etag),]
        }
    );

    let after_read_1 = rd_metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(after_read_1, before_read);

    // Fetch the key value. We must observe a query - L2 query results aren't fetched in get calls.
    let get_response = rd_client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![create_get_metadata_batch_resource_address_request_entry(
                resource_uri,
                "a",
            )],
        })
        .await
        .unwrap();

    assert_eq!(
        get_response,
        GetMetadataBatchResponse {
            results: vec![create_get_metadata_batch_success_response_entry(0, value, etag)]
        }
    );

    let after_read_2 = rd_metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_read_2,
        PostgresMetadataBackendMetrics {
            query_metadata_batch_count: after_read_1.query_metadata_batch_count + 1,
            ..after_read_1
        }
    );
}

#[rstest]
#[serial(redis_postgres)]
#[test_log::test(tokio::test)]
async fn test_redis_metadata_cache_update_invalidates_get_cache(
    postgres_redis_metadata_service: &RedisCacheServiceSet,
) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let wr_client = postgres_redis_metadata_service
        .wr_service
        .metadata_v1alpha_client(Interface::Grpc)
        .await;
    let rd_client = postgres_redis_metadata_service
        .rd_service
        .metadata_v1alpha_client(Interface::Grpc)
        .await;
    let rd_metrics = postgres_redis_metadata_service.rd_service.metrics_client();

    let resource_uri = create_resource_uri();
    let (value_1, etag_1) = wr_client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri), "a")
        .await
        .unwrap();

    // Warm-up the L2 cache.
    let before_read = rd_metrics.get_postgres_metadata_backend_metrics().await;
    let get_response_1 = rd_client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![create_get_metadata_batch_resource_address_request_entry(
                &resource_uri,
                "a",
            )],
        })
        .await
        .unwrap();

    assert_eq!(
        get_response_1,
        GetMetadataBatchResponse {
            results: vec![create_get_metadata_batch_success_response_entry(0, value_1, etag_1),]
        }
    );

    let after_read_1 = rd_metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_read_1,
        PostgresMetadataBackendMetrics {
            query_metadata_batch_count: before_read.query_metadata_batch_count + 1,
            ..before_read
        }
    );

    // Update the same metadata key via the WR instance.
    let (value_2, etag_2) = wr_client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri), "a")
        .await
        .unwrap();

    // Wait a little for the keyspace notification to arrive and read the metadata key again.
    // We must observe a query.
    tokio::time::sleep(Duration::from_secs(1)).await;

    let get_response_2 = rd_client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![create_get_metadata_batch_resource_address_request_entry(
                &resource_uri,
                "a",
            )],
        })
        .await
        .unwrap();

    assert_eq!(
        get_response_2,
        GetMetadataBatchResponse {
            results: vec![create_get_metadata_batch_success_response_entry(0, value_2, etag_2),]
        }
    );

    let after_read_2 = rd_metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_read_2,
        PostgresMetadataBackendMetrics {
            query_metadata_batch_count: after_read_1.query_metadata_batch_count + 1,
            ..after_read_1
        }
    );
}

#[rstest]
#[serial(redis_postgres)]
#[test_log::test(tokio::test)]
async fn test_redis_metadata_cache_update_invalidates_query_cache(
    postgres_redis_metadata_service: &RedisCacheServiceSet,
) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let wr_client = postgres_redis_metadata_service
        .wr_service
        .metadata_v1alpha_client(Interface::Grpc)
        .await;
    let rd_client = postgres_redis_metadata_service
        .rd_service
        .metadata_v1alpha_client(Interface::Grpc)
        .await;
    let rd_metrics = postgres_redis_metadata_service.rd_service.metrics_client();

    let resource_uri = create_resource_uri();
    let (value_1, etag_1) = wr_client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri), "a")
        .await
        .unwrap();

    // Warm-up the L2 cache.
    let before_read = rd_metrics.get_postgres_metadata_backend_metrics().await;
    let query_response_1 = rd_client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
            namespace: None,
        })
        .await
        .unwrap();

    assert_eq!(
        query_response_1,
        QueryMetadataResponse {
            results: vec![create_query_metadata_response_entry("a", value_1.clone(), &etag_1)]
        }
    );

    let after_read_1 = rd_metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_read_1,
        PostgresMetadataBackendMetrics {
            query_all_metadata_for_resource_count: before_read.query_all_metadata_for_resource_count + 1,
            ..before_read
        }
    );

    // Insert a new metadata key.
    let (value_2, etag_2) = wr_client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri), "b")
        .await
        .unwrap();

    // Wait a little for the keyspace notification to arrive and read the metadata key again.
    // We must observe a query.
    tokio::time::sleep(Duration::from_secs(1)).await;

    let get_response_2 = rd_client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
            namespace: None,
        })
        .await
        .unwrap();

    assert_eq!(
        get_response_2,
        QueryMetadataResponse {
            results: vec![
                create_query_metadata_response_entry("a", value_1, etag_1),
                create_query_metadata_response_entry("b", value_2, etag_2),
            ]
        }
    );

    let after_read_2 = rd_metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_read_2,
        PostgresMetadataBackendMetrics {
            query_all_metadata_for_resource_count: after_read_1.query_all_metadata_for_resource_count + 1,
            ..after_read_1
        }
    );
}

#[rstest]
#[serial(redis_postgres)]
#[test_log::test(tokio::test)]
async fn test_redis_metadata_cache_delete_invalidates_get_cache(
    postgres_redis_metadata_service: &RedisCacheServiceSet,
) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let wr_client = postgres_redis_metadata_service
        .wr_service
        .metadata_v1alpha_client(Interface::Grpc)
        .await;
    let rd_client = postgres_redis_metadata_service
        .rd_service
        .metadata_v1alpha_client(Interface::Grpc)
        .await;
    let rd_metrics = postgres_redis_metadata_service.rd_service.metrics_client();

    let resource_uri = create_resource_uri();
    let (value_1, etag_1) = wr_client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri), "a")
        .await
        .unwrap();

    // Warm-up the L2 cache.
    let before_read = rd_metrics.get_postgres_metadata_backend_metrics().await;
    let query_response_1 = rd_client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![create_get_metadata_batch_resource_address_request_entry(
                &resource_uri,
                "a",
            )],
        })
        .await
        .unwrap();

    assert_eq!(
        query_response_1,
        GetMetadataBatchResponse {
            results: vec![create_get_metadata_batch_success_response_entry(0, value_1, etag_1),]
        }
    );

    let after_read_1 = rd_metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_read_1,
        PostgresMetadataBackendMetrics {
            query_metadata_batch_count: before_read.query_metadata_batch_count + 1,
            ..before_read
        }
    );

    // Update the same metadata key via the WR instance.
    wr_client
        .delete_metadata_batch_sorted(DeleteMetadataBatchRequest {
            requests: vec![create_delete_metadata_batch_resource_address_request_entry(
                &resource_uri,
                "a",
            )],
        })
        .await
        .unwrap();

    // Wait a little for the keyspace notification to arrive and read the metadata key again.
    // We must observe a query.
    tokio::time::sleep(Duration::from_secs(1)).await;

    let query_response_2 = rd_client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![create_get_metadata_batch_resource_address_request_entry(
                &resource_uri,
                "a",
            )],
        })
        .await
        .unwrap();

    assert_eq!(
        query_response_2,
        GetMetadataBatchResponse {
            results: vec![create_get_metadata_batch_not_found_response_entry(0),]
        }
    );

    let after_read_2 = rd_metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_read_2,
        PostgresMetadataBackendMetrics {
            query_metadata_batch_count: after_read_1.query_metadata_batch_count + 1,
            ..after_read_1
        }
    );
}

#[rstest]
#[serial(redis_postgres)]
#[test_log::test(tokio::test)]
async fn test_redis_metadata_cache_delete_invalidates_query_cache(
    postgres_redis_metadata_service: &RedisCacheServiceSet,
) {
    use crate::common::clients::metadata::types::v1alpha::*;

    let wr_client = postgres_redis_metadata_service
        .wr_service
        .metadata_v1alpha_client(Interface::Grpc)
        .await;
    let rd_client = postgres_redis_metadata_service
        .rd_service
        .metadata_v1alpha_client(Interface::Grpc)
        .await;
    let rd_metrics = postgres_redis_metadata_service.rd_service.metrics_client();

    let resource_uri = create_resource_uri();
    let (value_1, etag_1) = wr_client
        .set_random_metadata_value(ResourceLocation::resource_address(&resource_uri), "a")
        .await
        .unwrap();

    // Warm-up the L2 cache.
    let before_read = rd_metrics.get_postgres_metadata_backend_metrics().await;
    let query_response_1 = rd_client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
            namespace: None,
        })
        .await
        .unwrap();

    assert_eq!(
        query_response_1,
        QueryMetadataResponse {
            results: vec![create_query_metadata_response_entry("a", value_1.clone(), &etag_1)]
        }
    );

    let after_read_1 = rd_metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_read_1,
        PostgresMetadataBackendMetrics {
            query_all_metadata_for_resource_count: before_read.query_all_metadata_for_resource_count + 1,
            ..before_read
        }
    );

    // Delete the metadata key.
    wr_client
        .delete_metadata_batch_sorted(DeleteMetadataBatchRequest {
            requests: vec![create_delete_metadata_batch_resource_address_request_entry(
                &resource_uri,
                "a",
            )],
        })
        .await
        .unwrap();

    // Wait a little for the keyspace notification to arrive and read the metadata key again.
    // We must observe a query.
    tokio::time::sleep(Duration::from_secs(1)).await;

    let query_response_2 = rd_client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::resource_address(&resource_uri)),
            namespace: None,
        })
        .await
        .unwrap();

    assert_eq!(query_response_2, QueryMetadataResponse { results: vec![] });

    let after_read_2 = rd_metrics.get_postgres_metadata_backend_metrics().await;
    assert_eq!(
        after_read_2,
        PostgresMetadataBackendMetrics {
            query_all_metadata_for_resource_count: after_read_1.query_all_metadata_for_resource_count + 1,
            ..after_read_1
        }
    );
}

fn create_string_value(value: impl Into<String>) -> Value {
    use storage_service_protos::google::protobuf::value::Kind;

    Value {
        kind: Some(Kind::StringValue(value.into())),
    }
}

fn create_resource_uri() -> String {
    get_env().unwrap_s3().resource_url(get_random_path())
}

fn create_not_found_batch_error_entry() -> BatchErrorEntry {
    use crate::common::clients::metadata::types::v1alpha::batch_error_entry::Code;

    BatchErrorEntry {
        code: Code::NotFound as i32,
        message: "metadata key not found".into(),
    }
}

fn create_metadata_modified_batch_error_entry() -> BatchErrorEntry {
    use crate::common::clients::metadata::types::v1alpha::batch_error_entry::Code;

    BatchErrorEntry {
        code: Code::FailedPrecondition as i32,
        message: "metadata was modified".into(),
    }
}

fn create_duplicate_batch_entry_batch_error_entry() -> BatchErrorEntry {
    use crate::common::clients::metadata::types::v1alpha::batch_error_entry::Code;

    BatchErrorEntry {
        code: Code::InvalidArgument as i32,
        message: "duplicate batch entry".into(),
    }
}

fn create_resource_uri_invalid_batch_error_entry() -> BatchErrorEntry {
    use crate::common::clients::metadata::types::v1alpha::batch_error_entry::Code;

    BatchErrorEntry {
        code: Code::InvalidArgument as i32,
        message: "resource uri is invalid".into(),
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
        ..Default::default()
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
        ..Default::default()
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
        ..Default::default()
    }
}

fn create_get_metadata_batch_success_response_entry(
    request_index: u32,
    value: impl Into<Value>,
    etag: impl Into<String>,
) -> GetMetadataBatchResponseEntry {
    use crate::common::clients::metadata::types::v1alpha::{get_metadata_batch_response_entry::Outcome, *};

    GetMetadataBatchResponseEntry {
        request_index,
        outcome: Some(Outcome::Success(UserMetadataValue {
            value: Some(value.into()),
            etag: etag.into(),
        })),
    }
}

fn create_update_metadata_batch_resource_address_request_entry(
    uri: impl Into<String>,
    key: impl Into<String>,
    value: impl Into<Value>,
) -> UpdateMetadataBatchRequestEntry {
    use crate::common::clients::metadata::types::v1alpha::*;

    UpdateMetadataBatchRequestEntry {
        resource_location: Some(ResourceLocation::resource_address(uri)),
        user_metadata_key: key.into(),
        user_metadata: Some(value.into()),
        ..Default::default()
    }
}

fn create_update_metadata_batch_resource_identity_request_entry(
    uri: impl Into<String>,
    key: impl Into<String>,
    value: impl Into<Value>,
) -> UpdateMetadataBatchRequestEntry {
    use crate::common::clients::metadata::types::v1alpha::*;

    UpdateMetadataBatchRequestEntry {
        resource_location: Some(ResourceLocation::resource_identity(uri)),
        user_metadata_key: key.into(),
        user_metadata: Some(value.into()),
        ..Default::default()
    }
}

fn create_delete_metadata_batch_resource_address_request_entry(
    uri: impl Into<String>,
    key: impl Into<String>,
) -> DeleteMetadataBatchRequestEntry {
    use crate::common::clients::metadata::types::v1alpha::*;

    DeleteMetadataBatchRequestEntry {
        resource_location: Some(ResourceLocation::resource_address(uri)),
        user_metadata_key: key.into(),
        ..Default::default()
    }
}

fn create_get_metadata_batch_not_found_response_entry(request_index: u32) -> GetMetadataBatchResponseEntry {
    use crate::common::clients::metadata::types::v1alpha::get_metadata_batch_response_entry::Outcome;

    GetMetadataBatchResponseEntry {
        request_index,
        outcome: Some(Outcome::Error(create_not_found_batch_error_entry())),
    }
}

fn create_get_metadata_batch_resource_uri_invalid_response_entry(request_index: u32) -> GetMetadataBatchResponseEntry {
    use crate::common::clients::metadata::types::v1alpha::get_metadata_batch_response_entry::Outcome;

    GetMetadataBatchResponseEntry {
        request_index,
        outcome: Some(Outcome::Error(create_resource_uri_invalid_batch_error_entry())),
    }
}

fn create_delete_metadata_batch_success_response_entry(request_index: u32) -> DeleteMetadataBatchResponseEntry {
    use crate::common::clients::metadata::types::v1alpha::delete_metadata_batch_response_entry::Outcome;

    DeleteMetadataBatchResponseEntry {
        request_index,
        outcome: Some(Outcome::Success(Default::default())),
    }
}

fn create_delete_metadata_metadata_modified_response_entry(request_index: u32) -> DeleteMetadataBatchResponseEntry {
    use crate::common::clients::metadata::types::v1alpha::delete_metadata_batch_response_entry::Outcome;

    DeleteMetadataBatchResponseEntry {
        request_index,
        outcome: Some(Outcome::Error(create_metadata_modified_batch_error_entry())),
    }
}

fn create_delete_metadata_duplicate_batch_entry_response_entry(request_index: u32) -> DeleteMetadataBatchResponseEntry {
    use crate::common::clients::metadata::types::v1alpha::delete_metadata_batch_response_entry::Outcome;

    DeleteMetadataBatchResponseEntry {
        request_index,
        outcome: Some(Outcome::Error(create_duplicate_batch_entry_batch_error_entry())),
    }
}

fn create_delete_metadata_resource_uri_invalid_response_entry(request_index: u32) -> DeleteMetadataBatchResponseEntry {
    use crate::common::clients::metadata::types::v1alpha::delete_metadata_batch_response_entry::Outcome;

    DeleteMetadataBatchResponseEntry {
        request_index,
        outcome: Some(Outcome::Error(create_resource_uri_invalid_batch_error_entry())),
    }
}

fn create_query_metadata_response_entry(
    key: impl Into<String>,
    value: impl Into<Value>,
    etag: impl Into<String>,
) -> QueryMetadataResponseEntry {
    use crate::common::clients::metadata::types::v1alpha::UserMetadataValue;

    QueryMetadataResponseEntry {
        user_metadata_key: key.into(),
        user_metadata: Some(UserMetadataValue {
            value: Some(value.into()),
            etag: etag.into(),
        }),
    }
}

fn create_user_metadata_value(value: impl Into<Value>, etag: impl Into<String>) -> UserMetadataValue {
    UserMetadataValue {
        value: Some(value.into()),
        etag: etag.into(),
    }
}

fn create_user_metadata_kv(
    key: impl Into<String>,
    value: impl Into<Value>,
    etag: impl Into<String>,
) -> (String, UserMetadataValue) {
    (key.into(), create_user_metadata_value(value, etag))
}

async fn populate_metadata(
    client: &common::clients::metadata::v1alpha::Client,
    resource_location: impl Into<ResourceLocation>,
    keys: impl IntoIterator<Item = impl Into<String>>,
) -> Vec<(Value, String)> {
    let resource_location = resource_location.into();

    let mut results = Vec::new();
    for key in keys {
        let (value, etag) = client
            .set_random_metadata_value(resource_location.clone(), key)
            .await
            .expect("metadata set failed");
        results.push((value, etag));
    }

    results
}
