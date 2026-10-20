// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use bytes::Bytes;
use storage_core::traits::fileobject::v1alpha::{UploadPreference, WriteParameters};
use storage_core::traits::metadata::v1alpha::{
    value, DeleteMetadataRequest, GetMetadataRequest, UpdateMetadataRequest, Value,
};
use storage_core_test::clients::traits::fileobject::FileObject;
use storage_core_test::clients::traits::metadata::Metadata;

use crate::report::TrafficReport;

const PHASE: &str = "Metadata";

pub async fn run(
    client: &mut Box<dyn Metadata>,
    fo_client: &mut Box<dyn FileObject>,
    base_url: &str,
    report: &TrafficReport,
) {
    let object_uri = format!("{base_url}traffic-gen/metadata-test-{}.txt", rand::random::<u32>());
    let data = vec![42u8; 64];

    // Create the object so metadata operations have a valid target
    let write_ok = report
        .track(PHASE, "write(setup)", async {
            fo_client
                .write(
                    WriteParameters {
                        native_metadata: Default::default(),
                        destination_resource_address: object_uri.clone(),
                        previous_version: None,
                        fail_if_exists: None,
                        data_object_size: data.len() as u64,
                        upload_preference: Some(UploadPreference::Body.into()),
                    },
                    Some(Bytes::from(data)),
                )
                .await
        })
        .await;

    if write_ok.is_none() {
        tracing::warn!("Skipping metadata operations: setup write failed");
        return;
    }

    // get_metadata
    report
        .track(PHASE, "get_metadata", async {
            client
                .get_metadata(GetMetadataRequest {
                    uri: object_uri.clone(),
                    user_metadata_keys: vec!["traffic-key".to_string()],
                })
                .await
        })
        .await;

    // update_metadata
    let update_value = Value {
        kind: Some(value::Kind::StringValue("traffic-value".to_string())),
    };
    report
        .track(PHASE, "update_metadata", async {
            client
                .update_metadata(UpdateMetadataRequest {
                    uri: object_uri.clone(),
                    user_metadata_key: "traffic-key".to_string(),
                    user_metadata: Some(update_value),
                    expected_etag: None,
                })
                .await
        })
        .await;

    // get_metadata after update
    report
        .track(PHASE, "get_metadata_after_update", async {
            client
                .get_metadata(GetMetadataRequest {
                    uri: object_uri.clone(),
                    user_metadata_keys: vec!["traffic-key".to_string()],
                })
                .await
        })
        .await;

    // get_metadata with empty keys (all keys)
    report
        .track(PHASE, "get_metadata_all_keys", async {
            client
                .get_metadata(GetMetadataRequest {
                    uri: object_uri.clone(),
                    user_metadata_keys: vec![],
                })
                .await
        })
        .await;

    // delete_metadata
    report
        .track(PHASE, "delete_metadata", async {
            client
                .delete_metadata(DeleteMetadataRequest {
                    uri: object_uri.clone(),
                    user_metadata_key: "traffic-key".to_string(),
                    expected_etag: None,
                })
                .await
        })
        .await;
}
