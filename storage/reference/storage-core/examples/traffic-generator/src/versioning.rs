// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use bytes::Bytes;
use futures::stream::StreamExt;
use storage_core::traits::fileobject::v1alpha::{DeleteRequest, UploadPreference, WriteParameters};
use storage_core::traits::versioning::v1alpha::EnumerateVersionsRequest;
use storage_core_test::clients::traits::fileobject::FileObject;
use storage_core_test::clients::traits::versioning::Versioning;
use storage_core_util::{debug_limited, warn_limited};
use tonic::Status;

use crate::report::TrafficReport;

const PHASE: &str = "Versioning";

pub async fn run(
    ver_client: &mut Box<dyn Versioning>,
    fo_client: &mut Box<dyn FileObject>,
    base_url: &str,
    report: &TrafficReport,
) {
    let file_path = format!("{base_url}traffic-gen/version-test-{}", rand::random::<u32>());
    let data = vec![42u8; 256];

    // Write a file so there is at least one version to enumerate
    let write_ok = report
        .track(PHASE, "write(setup)", async {
            fo_client
                .write(
                    WriteParameters {
                        native_metadata: Default::default(),
                        destination_resource_address: file_path.clone(),
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
        warn_limited!("Skipping enumerate_versions: setup write failed");
        return;
    }

    // enumerate_versions
    report
        .track(PHASE, "enumerate_versions", async {
            let stream = ver_client
                .enumerate_versions(EnumerateVersionsRequest {
                    resource_address: file_path.clone(),
                })
                .await?;

            // Consume the stream to generate full traffic
            let mut stream = stream;
            let mut count = 0u64;
            while let Some(response) = stream.next().await {
                match response {
                    Ok(resp) => count += resp.items.len() as u64,
                    Err(e) => return Err(e),
                }
            }
            debug_limited!(versions_found = count, "enumerate_versions complete");
            Ok::<_, Status>(count)
        })
        .await;

    // Clean up
    report
        .track("Cleanup", "delete", async {
            fo_client
                .delete(DeleteRequest {
                    resource_address: file_path,
                    previous_version: None,
                })
                .await
        })
        .await;
}
