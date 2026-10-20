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
use std::assert_matches;
use tonic::Code;

use common::{service::Service, types::Interface};

#[fixture]
#[once]
fn storage_service() -> Service {
    Service::builder().enable_az(true).enable_postgres_metadata().start()
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_get_metadata_batch_mixed_invalid_blob_path_is_rejected(storage_service: &Service) {
    use common::clients::metadata::types::v1alpha::{
        batch_error_entry::Code as BatchCode, get_metadata_batch_response_entry::Outcome, GetMetadataBatchRequest,
        GetMetadataBatchRequestEntry, ResourceLocation, ResourceLocationExt,
    };
    use common::random::get_random_path;

    let client = storage_service.metadata_v1alpha_client(Interface::Grpc).await;
    let valid_uri = storage_service.az_url_builder().path(get_random_path()).to_string();
    let bad_uri = storage_service.az_url_builder().path("blob.").to_string();
    let first_etag = client
        .set_metadata_value(
            ResourceLocation::resource_address(&valid_uri),
            "first",
            string_value("first value"),
        )
        .await
        .unwrap();
    let last_etag = client
        .set_metadata_value(
            ResourceLocation::resource_address(&valid_uri),
            "last",
            string_value("last value"),
        )
        .await
        .unwrap();

    let response = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                GetMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&valid_uri)),
                    user_metadata_key: "first".into(),
                    ..Default::default()
                },
                GetMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&bad_uri)),
                    user_metadata_key: "bad".into(),
                    ..Default::default()
                },
                GetMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&valid_uri)),
                    user_metadata_key: "last".into(),
                    ..Default::default()
                },
            ],
        })
        .await
        .unwrap();

    assert_eq!(response.results.len(), 3);
    assert_eq!(response.results[0].request_index, 0);
    assert_eq!(response.results[1].request_index, 1);
    assert_eq!(response.results[2].request_index, 2);
    assert_matches!(&response.results[0].outcome, Some(Outcome::Success(entry)) if entry.value.as_ref() == Some(&string_value("first value")) && entry.etag == first_etag);
    assert_matches!(&response.results[2].outcome, Some(Outcome::Success(entry)) if entry.value.as_ref() == Some(&string_value("last value")) && entry.etag == last_etag);
    assert_matches!(&response.results[1].outcome, Some(Outcome::Error(error)) if error.code == BatchCode::InvalidArgument as i32);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_update_metadata_batch_mixed_invalid_blob_path_is_rejected(storage_service: &Service) {
    use common::clients::metadata::types::v1alpha::{
        batch_error_entry::Code as BatchCode, update_metadata_batch_response_entry::Outcome, ResourceLocation,
        ResourceLocationExt, UpdateMetadataBatchRequest, UpdateMetadataBatchRequestEntry,
    };
    use common::random::get_random_path;

    let client = storage_service.metadata_v1alpha_client(Interface::Grpc).await;
    let valid_uri = storage_service.az_url_builder().path(get_random_path()).to_string();
    let bad_uri = storage_service.az_url_builder().path("blob.").to_string();

    let response = client
        .update_metadata_batch_sorted(UpdateMetadataBatchRequest {
            requests: vec![
                UpdateMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&valid_uri)),
                    user_metadata_key: "first".into(),
                    user_metadata: Some(string_value("first value")),
                    ..Default::default()
                },
                UpdateMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&bad_uri)),
                    user_metadata_key: "bad".into(),
                    user_metadata: Some(string_value("bad value")),
                    ..Default::default()
                },
                UpdateMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&valid_uri)),
                    user_metadata_key: "last".into(),
                    user_metadata: Some(string_value("last value")),
                    ..Default::default()
                },
            ],
        })
        .await
        .unwrap();

    assert_eq!(response.results.len(), 3);
    assert_eq!(response.results[0].request_index, 0);
    assert_eq!(response.results[1].request_index, 1);
    assert_eq!(response.results[2].request_index, 2);
    assert_matches!(&response.results[0].outcome, Some(Outcome::Success(_)));
    assert_matches!(&response.results[2].outcome, Some(Outcome::Success(_)));
    assert_matches!(&response.results[1].outcome, Some(Outcome::Error(error)) if error.code == BatchCode::InvalidArgument as i32);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_delete_metadata_batch_mixed_invalid_blob_path_is_rejected(storage_service: &Service) {
    use common::clients::metadata::types::v1alpha::{
        batch_error_entry::Code as BatchCode, delete_metadata_batch_response_entry::Outcome,
        DeleteMetadataBatchRequest, DeleteMetadataBatchRequestEntry, ResourceLocation, ResourceLocationExt,
    };
    use common::random::get_random_path;

    let client = storage_service.metadata_v1alpha_client(Interface::Grpc).await;
    let valid_uri = storage_service.az_url_builder().path(get_random_path()).to_string();
    let bad_uri = storage_service.az_url_builder().path("blob.").to_string();
    let first_etag = client
        .set_metadata_value(
            ResourceLocation::resource_address(&valid_uri),
            "first",
            string_value("first value"),
        )
        .await
        .unwrap();
    let last_etag = client
        .set_metadata_value(
            ResourceLocation::resource_address(&valid_uri),
            "last",
            string_value("last value"),
        )
        .await
        .unwrap();

    let response = client
        .delete_metadata_batch_sorted(DeleteMetadataBatchRequest {
            requests: vec![
                DeleteMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&valid_uri)),
                    user_metadata_key: "first".into(),
                    expected_etag: Some(first_etag),
                    ..Default::default()
                },
                DeleteMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&bad_uri)),
                    user_metadata_key: "bad".into(),
                    ..Default::default()
                },
                DeleteMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&valid_uri)),
                    user_metadata_key: "last".into(),
                    expected_etag: Some(last_etag),
                    ..Default::default()
                },
            ],
        })
        .await
        .unwrap();

    assert_eq!(response.results.len(), 3);
    assert_eq!(response.results[0].request_index, 0);
    assert_eq!(response.results[1].request_index, 1);
    assert_eq!(response.results[2].request_index, 2);
    assert_matches!(&response.results[0].outcome, Some(Outcome::Success(_)));
    assert_matches!(&response.results[2].outcome, Some(Outcome::Success(_)));
    assert_matches!(&response.results[1].outcome, Some(Outcome::Error(error)) if error.code == BatchCode::InvalidArgument as i32);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_get_metadata_batch_all_invalid_resource_address_invalid_blob_path_is_rejected(storage_service: &Service) {
    use common::clients::metadata::types::v1alpha::{
        batch_error_entry::Code as BatchCode, get_metadata_batch_response_entry::Outcome, GetMetadataBatchRequest,
        GetMetadataBatchRequestEntry, ResourceLocation, ResourceLocationExt,
    };

    let client = storage_service.metadata_v1alpha_client(Interface::Grpc).await;
    let uri = storage_service.az_url_builder().path("blob.").to_string();

    let response = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                GetMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&uri)),
                    user_metadata_key: "first".into(),
                    ..Default::default()
                },
                GetMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&uri)),
                    user_metadata_key: "middle".into(),
                    ..Default::default()
                },
                GetMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_address(&uri)),
                    user_metadata_key: "last".into(),
                    ..Default::default()
                },
            ],
        })
        .await
        .unwrap();

    assert_eq!(response.results.len(), 3);
    assert_eq!(response.results[0].request_index, 0);
    assert_matches!(&response.results[0].outcome, Some(Outcome::Error(error)) if error.code == BatchCode::InvalidArgument as i32);
    assert_eq!(response.results[1].request_index, 1);
    assert_matches!(&response.results[1].outcome, Some(Outcome::Error(error)) if error.code == BatchCode::InvalidArgument as i32);
    assert_eq!(response.results[2].request_index, 2);
    assert_matches!(&response.results[2].outcome, Some(Outcome::Error(error)) if error.code == BatchCode::InvalidArgument as i32);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_get_metadata_batch_all_invalid_resource_identity_invalid_blob_path_is_rejected(
    storage_service: &Service,
) {
    use common::clients::metadata::types::v1alpha::{
        batch_error_entry::Code as BatchCode, get_metadata_batch_response_entry::Outcome, GetMetadataBatchRequest,
        GetMetadataBatchRequestEntry, ResourceLocation, ResourceLocationExt,
    };

    let client = storage_service.metadata_v1alpha_client(Interface::Grpc).await;
    let uri = storage_service.az_url_builder().path("blob.").to_string();

    let response = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                GetMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_identity(&uri)),
                    user_metadata_key: "first".into(),
                    ..Default::default()
                },
                GetMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_identity(&uri)),
                    user_metadata_key: "middle".into(),
                    ..Default::default()
                },
                GetMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::resource_identity(&uri)),
                    user_metadata_key: "last".into(),
                    ..Default::default()
                },
            ],
        })
        .await
        .unwrap();

    assert_eq!(response.results.len(), 3);
    assert_eq!(response.results[0].request_index, 0);
    assert_matches!(&response.results[0].outcome, Some(Outcome::Error(error)) if error.code == BatchCode::InvalidArgument as i32);
    assert_eq!(response.results[1].request_index, 1);
    assert_matches!(&response.results[1].outcome, Some(Outcome::Error(error)) if error.code == BatchCode::InvalidArgument as i32);
    assert_eq!(response.results[2].request_index, 2);
    assert_matches!(&response.results[2].outcome, Some(Outcome::Error(error)) if error.code == BatchCode::InvalidArgument as i32);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_get_metadata_batch_all_invalid_folder_address_invalid_blob_path_is_rejected(storage_service: &Service) {
    use common::clients::metadata::types::v1alpha::{
        batch_error_entry::Code as BatchCode, get_metadata_batch_response_entry::Outcome, GetMetadataBatchRequest,
        GetMetadataBatchRequestEntry, ResourceLocation, ResourceLocationExt,
    };

    let client = storage_service.metadata_v1alpha_client(Interface::Grpc).await;
    let uri = storage_service.az_url_builder().path("blob.").to_string();

    let response = client
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![
                GetMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::folder_address(&uri)),
                    user_metadata_key: "first".into(),
                    ..Default::default()
                },
                GetMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::folder_address(&uri)),
                    user_metadata_key: "middle".into(),
                    ..Default::default()
                },
                GetMetadataBatchRequestEntry {
                    resource_location: Some(ResourceLocation::folder_address(&uri)),
                    user_metadata_key: "last".into(),
                    ..Default::default()
                },
            ],
        })
        .await
        .unwrap();

    assert_eq!(response.results.len(), 3);
    assert_eq!(response.results[0].request_index, 0);
    assert_matches!(&response.results[0].outcome, Some(Outcome::Error(error)) if error.code == BatchCode::InvalidArgument as i32);
    assert_eq!(response.results[1].request_index, 1);
    assert_matches!(&response.results[1].outcome, Some(Outcome::Error(error)) if error.code == BatchCode::InvalidArgument as i32);
    assert_eq!(response.results[2].request_index, 2);
    assert_matches!(&response.results[2].outcome, Some(Outcome::Error(error)) if error.code == BatchCode::InvalidArgument as i32);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_query_metadata_invalid_blob_path_is_rejected(storage_service: &Service) {
    use common::clients::metadata::types::v1alpha::{QueryMetadataRequest, ResourceLocation, ResourceLocationExt};

    let client = storage_service.metadata_v1alpha_client(Interface::Grpc).await;
    let uri = storage_service.az_url_builder().path("blob.").to_string();

    let error = client
        .query_metadata_sorted(QueryMetadataRequest {
            resource_location: Some(ResourceLocation::resource_address(uri)),
            ..Default::default()
        })
        .await
        .unwrap_err();

    assert_eq!(error.code(), Code::InvalidArgument);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_get_metadata_invalid_blob_path_is_rejected(storage_service: &Service) {
    use common::clients::metadata::types::v1alpha::GetMetadataRequest;

    let client = storage_service.metadata_v1alpha_client(Interface::Grpc).await;
    let uri = storage_service.az_url_builder().path("blob.").to_string();

    let error = client
        .get_metadata(GetMetadataRequest {
            uri,
            ..Default::default()
        })
        .await
        .unwrap_err();

    assert_eq!(error.code(), Code::InvalidArgument);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_update_metadata_invalid_blob_path_is_rejected(storage_service: &Service) {
    use common::clients::metadata::types::v1alpha::UpdateMetadataRequest;

    let client = storage_service.metadata_v1alpha_client(Interface::Grpc).await;
    let uri = storage_service.az_url_builder().path("blob.").to_string();

    let error = client
        .update_metadata(UpdateMetadataRequest {
            uri,
            user_metadata_key: "key".into(),
            user_metadata: Some(string_value("value")),
            ..Default::default()
        })
        .await
        .unwrap_err();

    assert_eq!(error.code(), Code::InvalidArgument);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_delete_metadata_invalid_blob_path_is_rejected(storage_service: &Service) {
    use common::clients::metadata::types::v1alpha::DeleteMetadataRequest;

    let client = storage_service.metadata_v1alpha_client(Interface::Grpc).await;
    let uri = storage_service.az_url_builder().path("blob.").to_string();

    let error = client
        .delete_metadata(DeleteMetadataRequest {
            uri,
            user_metadata_key: "key".into(),
            ..Default::default()
        })
        .await
        .unwrap_err();

    assert_eq!(error.code(), Code::InvalidArgument);
}

fn string_value(value: impl Into<String>) -> storage_service_protos::google::protobuf::Value {
    use storage_service_protos::google::protobuf::{value::Kind, Value};

    Value {
        kind: Some(Kind::StringValue(value.into())),
    }
}
