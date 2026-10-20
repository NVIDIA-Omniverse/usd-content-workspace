// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use futures::stream::StreamExt;
use storage_core::traits::filefolder::v1alpha::{
    CreateFolderRequest, DeleteFolderRequest, FolderAddress, GetFolderModeRequest, ListRequest, ListStatRequest,
};
use storage_core_test::clients::traits::filefolder::FileFolder;
use storage_core_util::debug_limited;
use tonic::Status;

use crate::report::TrafficReport;
use crate::runner::ApiVersionArg;

const PHASE: &str = "FileFolder";

pub async fn run(
    client: &mut Box<dyn FileFolder>,
    folder_address: &str,
    api_version: ApiVersionArg,
    report: &TrafficReport,
) {
    let folder = FolderAddress {
        uri: folder_address.to_string(),
    };

    // create_folder (v1alpha only)
    if api_version == ApiVersionArg::V1Alpha {
        report
            .track(PHASE, "create_folder", async {
                client
                    .create_folder(CreateFolderRequest {
                        folder: Some(folder.clone()),
                    })
                    .await
            })
            .await;
    }

    // list
    report
        .track(PHASE, "list", async {
            let stream = client
                .list(ListRequest {
                    folder: Some(folder.clone()),
                })
                .await?;
            let mut stream = stream;
            let mut subfolder_count = 0u64;
            let mut object_count = 0u64;
            while let Some(response) = stream.next().await {
                match response {
                    Ok(resp) => {
                        subfolder_count += resp.subfolder_addresses.len() as u64;
                        object_count += resp.sub_resource_addresses.len() as u64;
                    },
                    Err(e) => return Err(e),
                }
            }
            debug_limited!(subfolder_count, object_count, "list complete");
            Ok::<_, Status>((subfolder_count, object_count))
        })
        .await;

    // list_stat
    report
        .track(PHASE, "list_stat", async {
            let stream = client
                .list_stat(ListStatRequest {
                    include_native_metadata: false,
                    folder: Some(folder.clone()),
                })
                .await?;
            let mut stream = stream;
            let mut entry_count = 0u64;
            while let Some(response) = stream.next().await {
                match response {
                    Ok(resp) => {
                        entry_count += resp.entries.len() as u64;
                    },
                    Err(e) => return Err(e),
                }
            }
            debug_limited!(entry_count, "list_stat complete");
            Ok::<_, Status>(entry_count)
        })
        .await;

    // get_folder_mode (v1alpha only)
    if api_version == ApiVersionArg::V1Alpha {
        report
            .track(PHASE, "get_folder_mode", async {
                client
                    .get_folder_mode(GetFolderModeRequest {
                        folder: Some(folder.clone()),
                    })
                    .await
            })
            .await;
    }

    // delete_folder
    report
        .track(PHASE, "delete_folder", async {
            client
                .delete_folder(DeleteFolderRequest {
                    folder: Some(folder.clone()),
                })
                .await
        })
        .await;
}
