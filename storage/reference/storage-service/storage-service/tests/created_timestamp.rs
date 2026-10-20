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

//! The service records a `created_timestamp` extended-metadata entry (seconds since the Unix
//! epoch) when a resource is first created via write / copy / move / create_folder, once, and
//! preserves it across later overwrites. This is what the client-library reads as an
//! object's/folder's creation time.

mod common;

use std::time::{SystemTime, UNIX_EPOCH};

use assert_matches::assert_matches;
use bytes::Bytes;
use rstest::{fixture, rstest};
use storage_service_protos::google::protobuf::value::Kind;
use storage_service_protos::nvidia::omniverse::storage::filefolder::v1alpha::{CreateFolderRequest, FolderAddress};
use storage_service_protos::nvidia::omniverse::storage::filefolder::v1beta as filefolder_v1beta;
use storage_service_protos::nvidia::omniverse::storage::fileobject::{v1alpha, v1beta};

use crate::common::clients::fileobject::types::v1alpha::UnwrapIdentityExt;
use crate::common::clients::metadata::types::v1alpha::get_metadata_batch_response_entry::Outcome;
use crate::common::clients::metadata::types::v1alpha::{
    GetMetadataBatchRequest, GetMetadataBatchRequestEntry, ResourceLocation, ResourceLocationExt,
};
use crate::common::random::get_random_path;
use crate::common::service::Service as StorageService;
use crate::common::types::Interface;

const CREATED_TIMESTAMP_KEY: &str = "created_timestamp";

#[fixture]
#[once]
fn service() -> StorageService {
    StorageService::builder()
        .enable_s3(true)
        .enable_postgres_metadata()
        .start()
}

fn now_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_secs_f64()
}

/// The `created_timestamp` value (seconds) recorded for `uri`, or `None` if it is not set. Folder
/// URIs (trailing slash) are keyed under a folder address, objects under a resource address.
async fn read_created_timestamp(service: &StorageService, uri: &str) -> Option<f64> {
    let resource_location = if uri.ends_with('/') {
        ResourceLocation::folder_address(uri)
    } else {
        ResourceLocation::resource_address(uri)
    };
    let response = service
        .metadata_v1alpha_client(Interface::Grpc)
        .await
        .get_metadata_batch_sorted(GetMetadataBatchRequest {
            requests: vec![GetMetadataBatchRequestEntry {
                resource_location: Some(resource_location),
                user_metadata_key: CREATED_TIMESTAMP_KEY.to_owned(),
                ..Default::default()
            }],
        })
        .await
        .expect("get_metadata_batch failed");
    match response.results.into_iter().next().expect("one result entry").outcome {
        Some(Outcome::Success(entry)) => match entry.value.and_then(|value| value.kind) {
            Some(Kind::NumberValue(seconds)) => Some(seconds),
            _ => None,
        },
        _ => None,
    }
}

fn assert_recent(timestamp: f64, not_before: f64) {
    let lower = not_before - 5.0;
    let upper = now_seconds() + 5.0;
    assert!(
        timestamp >= lower && timestamp <= upper,
        "created_timestamp {timestamp} is not within the write window [{lower}, {upper}]"
    );
}

async fn write_v1alpha(service: &StorageService, uri: &str, content: &'static [u8]) {
    service
        .fileobject_v1alpha_client(Interface::Grpc)
        .await
        .write_object(
            v1alpha::WriteParameters {
                destination_resource_address: uri.to_owned(),
                data_object_size: content.len() as u64,
                ..Default::default()
            },
            Bytes::from_static(content),
        )
        .await
        .expect("write failed");
}

async fn write_v1beta(service: &StorageService, uri: &str, content: &'static [u8]) {
    service
        .fileobject_v1beta_client(Interface::Grpc)
        .await
        .write_object(
            v1beta::WriteParameters {
                destination_resource_address: uri.to_owned(),
                data_object_size: content.len() as u64,
                ..Default::default()
            },
            Bytes::from_static(content),
        )
        .await
        .expect("v1beta write failed");
}

#[rstest]
#[test_log::test(tokio::test)]
async fn write_sets_created_timestamp(service: &StorageService) {
    let uri = service
        .s3_url_builder()
        .path(format!("{}.usda", get_random_path()))
        .to_string();
    let before = now_seconds();

    write_v1alpha(service, &uri, b"abc").await;

    let timestamp = read_created_timestamp(service, &uri)
        .await
        .expect("created_timestamp must be set on write");
    assert_recent(timestamp, before);
}

// The folder case is the one the client-library could not populate on its own.
#[rstest]
#[test_log::test(tokio::test)]
async fn create_folder_sets_created_timestamp(service: &StorageService) {
    let uri = service
        .s3_url_builder()
        .path(format!("{}/", get_random_path()))
        .to_string();
    let before = now_seconds();

    service
        .filefolder_v1alpha_client(Interface::Grpc)
        .await
        .create_folder(CreateFolderRequest {
            folder: Some(FolderAddress { uri: uri.clone() }),
        })
        .await
        .expect("create_folder failed");

    let timestamp = read_created_timestamp(service, &uri)
        .await
        .expect("created_timestamp must be set on create_folder");
    assert_recent(timestamp, before);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn copy_sets_created_timestamp_on_destination(service: &StorageService) {
    let source = service
        .s3_url_builder()
        .path(format!("{}.usda", get_random_path()))
        .to_string();
    let destination = service
        .s3_url_builder()
        .path(format!("{}.usda", get_random_path()))
        .to_string();
    let client = service.fileobject_v1alpha_client(Interface::Grpc).await;

    let (source_info, _) = client
        .write_object(
            v1alpha::WriteParameters {
                destination_resource_address: source.clone(),
                data_object_size: 3,
                ..Default::default()
            },
            Bytes::from_static(b"abc"),
        )
        .await
        .expect("source write failed");
    let before = now_seconds();

    client
        .copy(v1alpha::CopyRequest {
            source_resource_identity: Some(v1alpha::ResourceIdentity {
                encoded_identity: source_info.unwrap_identity(),
            }),
            destination_resource_address: destination.clone(),
            ..Default::default()
        })
        .await
        .expect("copy failed");

    let timestamp = read_created_timestamp(service, &destination)
        .await
        .expect("created_timestamp must be set on copy");
    assert_recent(timestamp, before);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn move_sets_created_timestamp_on_destination(service: &StorageService) {
    let source = service
        .s3_url_builder()
        .path(format!("{}.usda", get_random_path()))
        .to_string();
    let destination = service
        .s3_url_builder()
        .path(format!("{}.usda", get_random_path()))
        .to_string();
    let client = service.fileobject_v1alpha_client(Interface::Grpc).await;
    write_v1alpha(service, &source, b"abc").await;
    let before = now_seconds();

    client
        .move_object(v1alpha::MoveRequest {
            source_resource_address: source.clone(),
            destination_resource_address: destination.clone(),
            ..Default::default()
        })
        .await
        .expect("move failed");

    let timestamp = read_created_timestamp(service, &destination)
        .await
        .expect("created_timestamp must be set on move");
    assert_recent(timestamp, before);
}

// Set-once: an overwrite must keep the original creation time, not reset it.
#[rstest]
#[test_log::test(tokio::test)]
async fn overwrite_preserves_created_timestamp(service: &StorageService) {
    let uri = service
        .s3_url_builder()
        .path(format!("{}.usda", get_random_path()))
        .to_string();

    write_v1alpha(service, &uri, b"abc").await;
    let first = read_created_timestamp(service, &uri)
        .await
        .expect("created_timestamp must be set on first write");

    write_v1alpha(service, &uri, b"defghi").await;
    let second = read_created_timestamp(service, &uri)
        .await
        .expect("created_timestamp must remain set after overwrite");

    assert_eq!(first, second, "overwrite must preserve the original created_timestamp");
}

// Deleting a resource best-effort removes its recorded `created_timestamp`.
#[rstest]
#[test_log::test(tokio::test)]
async fn delete_removes_created_timestamp(service: &StorageService) {
    let uri = service
        .s3_url_builder()
        .path(format!("{}.usda", get_random_path()))
        .to_string();

    write_v1alpha(service, &uri, b"abc").await;
    assert_matches!(read_created_timestamp(service, &uri).await, Some(_));

    service
        .fileobject_v1alpha_client(Interface::Grpc)
        .await
        .delete(v1alpha::DeleteRequest {
            resource_address: uri.clone(),
            ..Default::default()
        })
        .await
        .expect("delete failed");

    assert_matches!(read_created_timestamp(service, &uri).await, None);
}

// The v1beta trait has its own write / delete implementations, distinct from v1alpha; exercise each
// of those control-flow paths.
#[rstest]
#[test_log::test(tokio::test)]
async fn v1beta_write_sets_created_timestamp(service: &StorageService) {
    let uri = service
        .s3_url_builder()
        .path(format!("{}.usda", get_random_path()))
        .to_string();
    let before = now_seconds();

    write_v1beta(service, &uri, b"abc").await;

    let timestamp = read_created_timestamp(service, &uri)
        .await
        .expect("created_timestamp must be set on a v1beta write");
    assert_recent(timestamp, before);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn v1beta_delete_removes_created_timestamp(service: &StorageService) {
    let uri = service
        .s3_url_builder()
        .path(format!("{}.usda", get_random_path()))
        .to_string();

    write_v1beta(service, &uri, b"abc").await;
    assert_matches!(read_created_timestamp(service, &uri).await, Some(_));

    service
        .fileobject_v1beta_client(Interface::Grpc)
        .await
        .delete(v1beta::DeleteRequest {
            resource_address: uri.clone(),
            ..Default::default()
        })
        .await
        .expect("v1beta delete failed");

    assert_matches!(read_created_timestamp(service, &uri).await, None);
}

// The v1beta filefolder trait has no `create_folder`, so the folder is created through v1alpha and
// then removed through the v1beta `delete_folder` path under test.
#[rstest]
#[test_log::test(tokio::test)]
async fn v1beta_delete_folder_removes_created_timestamp(service: &StorageService) {
    let uri = service
        .s3_url_builder()
        .path(format!("{}/", get_random_path()))
        .to_string();

    service
        .filefolder_v1alpha_client(Interface::Grpc)
        .await
        .create_folder(CreateFolderRequest {
            folder: Some(FolderAddress { uri: uri.clone() }),
        })
        .await
        .expect("create_folder failed");
    assert_matches!(read_created_timestamp(service, &uri).await, Some(_));

    service
        .filefolder_v1beta_client(Interface::Grpc)
        .await
        .delete_folder(filefolder_v1beta::DeleteFolderRequest {
            folder: Some(filefolder_v1beta::FolderAddress { uri: uri.clone() }),
        })
        .await
        .expect("v1beta delete_folder failed");

    assert_matches!(read_created_timestamp(service, &uri).await, None);
}
