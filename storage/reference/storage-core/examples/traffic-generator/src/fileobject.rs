// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use bytes::Bytes;
use futures::stream::StreamExt;
use storage_core::traits::fileobject::v1alpha::{
    write_response, AbortMultipartUploadRequest, CompleteMultipartUploadRequest, CompleteRedirectUploadRequest,
    CompletedUploadPart, CopyRequest, DeleteRequest, DownloadPreference, EnumerateRequest, FetchWriteTypeInfoRequest,
    Header, MoveRequest, ReadFromAddressRequest, ReadRequest, StatRequest, UploadMethod, UploadPartRequest,
    UploadPreference, WriteParameters,
};
use storage_core_test::clients::traits::fileobject::FileObject;
use storage_core_util::{debug_limited, error_limited, warn_limited};
use tonic::Status;

use crate::report::TrafficReport;
use crate::runner::ApiVersionArg;

const PHASE_BODY: &str = "FileObject/Body";
const PHASE_MULTIPART: &str = "FileObject/Multipart";
const PHASE_REDIRECT: &str = "FileObject/Redirect";

/// Generate random test data of the given size.
fn random_data(len: usize) -> Vec<u8> {
    let mut data = Vec::with_capacity(len);
    data.resize_with(len, rand::random);
    data
}

/// Perform an HTTP upload to a redirect URL (used by multipart and redirect flows).
async fn perform_redirect_upload(
    data: &[u8],
    method: i32,
    url: &str,
    additional_headers: Vec<Header>,
    completion_header_names: &[String],
) -> Result<Vec<Header>, String> {
    let http_client = reqwest::Client::new();
    let mut builder = match method {
        method if method == UploadMethod::Post as i32 => http_client.post(url),
        method if method == UploadMethod::Put as i32 => http_client.put(url),
        _ => return Err("Upload method must be Post or Put".to_string()),
    };

    for header in additional_headers {
        builder = builder.header(header.name, header.value);
    }

    builder = builder.body(data.to_vec());
    let http_response = builder.send().await.map_err(|e| format!("HTTP upload failed: {e}"))?;

    if !http_response.status().is_success() {
        return Err(format!("HTTP upload returned status {}", http_response.status()));
    }

    let mut headers = Vec::new();
    for header_name in completion_header_names {
        if let Some(header_value) = http_response.headers().get(header_name) {
            headers.push(Header {
                name: header_name.clone(),
                value: header_value.to_str().unwrap_or_default().into(),
            });
        }
    }
    Ok(headers)
}

// ─── Phase 2: Body Upload Lifecycle ──────────────────────────────────────────

pub async fn run_body_lifecycle(
    client: &mut Box<dyn FileObject>,
    base_url: &str,
    api_version: ApiVersionArg,
    report: &TrafficReport,
) {
    let run_id = rand::random::<u32>();
    let enum_prefix = format!("{base_url}traffic-gen/enum-{run_id}/");
    let file_path = format!("{enum_prefix}body-test");
    let file_path_2 = format!("{enum_prefix}body-test-2");
    let copy_path = format!("{base_url}traffic-gen/body-copy-{run_id}");
    let move_path = format!("{base_url}traffic-gen/body-move-{run_id}");
    let data = random_data(1024); // 1 KiB test file

    // fetch_write_type_info
    report
        .track(PHASE_BODY, "fetch_write_type_info", async {
            client
                .fetch_write_type_info(FetchWriteTypeInfoRequest {
                    destination_resource_address: file_path.clone(),
                })
                .await
        })
        .await;

    // write (body)
    let write_response = report
        .track(PHASE_BODY, "write", async {
            client
                .write(
                    WriteParameters {
                        native_metadata: Default::default(),
                        destination_resource_address: file_path.clone(),
                        previous_version: None,
                        fail_if_exists: None,
                        data_object_size: data.len() as u64,
                        upload_preference: Some(UploadPreference::Body.into()),
                    },
                    Some(Bytes::from(data.clone())),
                )
                .await
        })
        .await;

    // Extract resource identity from write response for later operations
    let resource_identity = write_response.and_then(|resp| {
        if let Some(write_response::WriteResponseType::ResourceInfo(info)) = resp.write_response_type {
            info.resource_identity
        } else {
            None
        }
    });

    // stat
    report
        .track(PHASE_BODY, "stat", async {
            client
                .stat(StatRequest {
                    resource_address: file_path.clone(),
                })
                .await
        })
        .await;

    // read_from_address (body)
    report
        .track(PHASE_BODY, "read_from_address(body)", async {
            let stream = client
                .read_from_address(ReadFromAddressRequest {
                    resource_address: file_path.clone(),
                    download_preference: Some(DownloadPreference::Body.into()),
                })
                .await?;
            // Consume the stream
            let mut stream = stream;
            let mut byte_count = 0u64;
            while let Some(msg) = stream.next().await {
                msg?;
                byte_count += 1;
            }
            debug_limited!(messages = byte_count, "read_from_address(body) consumed");
            Ok::<_, Status>(byte_count)
        })
        .await;

    // read_from_address (redirect)
    report
        .track(PHASE_BODY, "read_from_address(redirect)", async {
            let stream = client
                .read_from_address(ReadFromAddressRequest {
                    resource_address: file_path.clone(),
                    download_preference: Some(DownloadPreference::Redirect.into()),
                })
                .await?;
            let mut stream = stream;
            while let Some(msg) = stream.next().await {
                msg?;
            }
            Ok::<_, Status>(())
        })
        .await;

    // read (by identity, body) - only if we have a resource identity
    if let Some(ref identity) = resource_identity {
        report
            .track(PHASE_BODY, "read(by_identity,body)", async {
                let stream = client
                    .read(ReadRequest {
                        resource_identity: Some(identity.clone()),
                        download_preference: Some(DownloadPreference::Body.into()),
                    })
                    .await?;
                let mut stream = stream;
                while let Some(msg) = stream.next().await {
                    msg?;
                }
                Ok::<_, Status>(())
            })
            .await;
    }

    // Write a second file under the same prefix so enumerate has multiple objects to list.
    // Enumerate works like list (prefix-based), not on a single object.
    report
        .track(PHASE_BODY, "write(enum-setup)", async {
            client
                .write(
                    WriteParameters {
                        native_metadata: Default::default(),
                        destination_resource_address: file_path_2.clone(),
                        previous_version: None,
                        fail_if_exists: None,
                        data_object_size: data.len() as u64,
                        upload_preference: Some(UploadPreference::Body.into()),
                    },
                    Some(Bytes::from(data.clone())),
                )
                .await
        })
        .await;

    // enumerate - called on the shared prefix, not a single file
    report
        .track(PHASE_BODY, "enumerate", async {
            let stream = client
                .enumerate(EnumerateRequest {
                    resource_address: enum_prefix.clone(),
                })
                .await?;
            let mut stream = stream;
            let mut item_count = 0u64;
            while let Some(msg) = stream.next().await {
                match msg {
                    Ok(resp) => item_count += resp.items.len() as u64,
                    Err(e) => return Err(e),
                }
            }
            debug_limited!(item_count, "enumerate complete");
            Ok::<_, Status>(item_count)
        })
        .await;

    // Clean up the second file
    cleanup_file(client, &file_path_2, report).await;

    // copy (v1alpha only)
    let copy_identity = if api_version == ApiVersionArg::V1Alpha {
        if let Some(ref identity) = resource_identity {
            report
                .track(PHASE_BODY, "copy", async {
                    client
                        .copy(CopyRequest {
                            source_resource_identity: Some(identity.clone()),
                            destination_resource_address: copy_path.clone(),
                            previous_version: None,
                            fail_if_exists: None,
                        })
                        .await
                })
                .await
                .and_then(|r| r.resource_identity)
        } else {
            None
        }
    } else {
        None
    };

    // move_object (v1alpha only)
    if api_version == ApiVersionArg::V1Alpha {
        if let Some(ref identity) = copy_identity {
            report
                .track(PHASE_BODY, "move_object", async {
                    client
                        .move_object(MoveRequest {
                            source_resource_address: copy_path.clone(),
                            destination_resource_address: move_path.clone(),
                            source_previous_version: Some(identity.clone()),
                            destination_previous_version: None,
                            fail_if_exists: None,
                        })
                        .await
                })
                .await;

            // Clean up the moved file
            cleanup_file(client, &move_path, report).await;
        }
    }

    // delete (clean up the original file)
    cleanup_file(client, &file_path, report).await;
}

// ─── Phase 3: Multipart Upload Lifecycle ─────────────────────────────────────

pub async fn run_multipart_lifecycle(client: &mut Box<dyn FileObject>, base_url: &str, report: &TrafficReport) {
    let file_path = format!("{base_url}traffic-gen/multipart-test-{}", rand::random::<u32>());
    let part_count = 2usize;
    let data = random_data(2 * 1024); // 2 KiB, split into 2 parts

    // write (multipart preference) -> initiate multipart upload
    let multipart_response = report
        .track(PHASE_MULTIPART, "write(initiate)", async {
            client
                .write(
                    WriteParameters {
                        native_metadata: Default::default(),
                        destination_resource_address: file_path.clone(),
                        previous_version: None,
                        fail_if_exists: None,
                        data_object_size: data.len() as u64,
                        upload_preference: Some(UploadPreference::Multipart.into()),
                    },
                    None,
                )
                .await
        })
        .await;

    let multipart_info = multipart_response.and_then(|resp| {
        if let Some(write_response::WriteResponseType::MultipartUpload(info)) = resp.write_response_type {
            Some(info)
        } else {
            None
        }
    });

    let Some(multipart_info) = multipart_info else {
        warn_limited!("Multipart initiation did not return expected response, skipping rest of multipart phase");
        return;
    };

    let upload_id = multipart_info.upload_id.clone();
    let first_redirect = match multipart_info.first_part_write_redirect {
        Some(r) => r,
        None => {
            warn_limited!("No first part redirect in multipart response, aborting multipart");
            abort_multipart(client, &file_path, &upload_id, report).await;
            return;
        },
    };

    // Split data into parts
    let chunk_size = data.len() / part_count;
    let first_chunk = &data[..chunk_size];
    let second_chunk = &data[chunk_size..];

    // Upload first part via redirect
    let first_headers = match perform_redirect_upload(
        first_chunk,
        first_redirect.method,
        &first_redirect.redirect_target_url,
        first_redirect.additional_headers,
        &first_redirect.completion_header_names,
    )
    .await
    {
        Ok(h) => h,
        Err(e) => {
            error_limited!(error = %e, "First part upload failed");
            abort_multipart(client, &file_path, &upload_id, report).await;
            return;
        },
    };

    // upload_part -> get redirect for second part
    let upload_part_response = report
        .track(PHASE_MULTIPART, "upload_part", async {
            client
                .upload_part(UploadPartRequest {
                    upload_id: upload_id.clone(),
                    destination_resource_address: file_path.clone(),
                    part_number: 1,
                    part_count: Some(1),
                })
                .await
        })
        .await;

    let mut all_headers = vec![first_headers];

    if let Some(part_response) = upload_part_response {
        for part in part_response.part_write_redirects {
            match perform_redirect_upload(
                second_chunk,
                first_redirect.method,
                &part.redirect_target_url,
                part.additional_headers,
                &part.completion_header_names,
            )
            .await
            {
                Ok(h) => all_headers.push(h),
                Err(e) => {
                    error_limited!(error = %e, "Part upload failed");
                    abort_multipart(client, &file_path, &upload_id, report).await;
                    return;
                },
            }
        }
    } else {
        abort_multipart(client, &file_path, &upload_id, report).await;
        return;
    }

    // complete_multipart_upload
    report
        .track(PHASE_MULTIPART, "complete_multipart_upload", async {
            client
                .complete_multipart_upload(CompleteMultipartUploadRequest {
                    upload_id: upload_id.clone(),
                    destination_resource_address: file_path.clone(),
                    parts: all_headers
                        .into_iter()
                        .enumerate()
                        .map(|(n, h)| CompletedUploadPart {
                            part_number: n as u32,
                            headers: h,
                        })
                        .collect(),
                })
                .await
        })
        .await;

    // delete (clean up)
    cleanup_file(client, &file_path, report).await;
}

// ─── Phase 4: Redirect Upload Lifecycle ──────────────────────────────────────

pub async fn run_redirect_lifecycle(client: &mut Box<dyn FileObject>, base_url: &str, report: &TrafficReport) {
    let file_path = format!("{base_url}traffic-gen/redirect-test-{}", rand::random::<u32>());
    let data = random_data(1024); // 1 KiB test file

    // write (redirect preference)
    let redirect_response = report
        .track(PHASE_REDIRECT, "write(redirect)", async {
            client
                .write(
                    WriteParameters {
                        native_metadata: Default::default(),
                        destination_resource_address: file_path.clone(),
                        previous_version: None,
                        fail_if_exists: None,
                        data_object_size: data.len() as u64,
                        upload_preference: Some(UploadPreference::Redirect.into()),
                    },
                    None,
                )
                .await
        })
        .await;

    let redirect_info = redirect_response.and_then(|resp| {
        if let Some(write_response::WriteResponseType::WriteRedirect(info)) = resp.write_response_type {
            Some(info)
        } else {
            None
        }
    });

    let Some(redirect_info) = redirect_info else {
        warn_limited!("Redirect initiation did not return expected response, skipping rest of redirect phase");
        return;
    };

    // Perform HTTP upload to redirect URL
    let completion_headers = match perform_redirect_upload(
        &data,
        redirect_info.method,
        &redirect_info.redirect_target_url,
        redirect_info.additional_headers,
        &redirect_info.completion_header_names,
    )
    .await
    {
        Ok(h) => h,
        Err(e) => {
            error_limited!(error = %e, "Redirect upload failed");
            return;
        },
    };

    // complete_redirect_upload
    report
        .track(PHASE_REDIRECT, "complete_redirect_upload", async {
            client
                .complete_redirect_upload(CompleteRedirectUploadRequest {
                    destination_resource_address: file_path.clone(),
                    additional_headers: completion_headers,
                })
                .await
        })
        .await;

    // delete (clean up)
    cleanup_file(client, &file_path, report).await;
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

async fn cleanup_file(client: &mut Box<dyn FileObject>, path: &str, report: &TrafficReport) {
    report
        .track("Cleanup", "delete", async {
            client
                .delete(DeleteRequest {
                    resource_address: path.to_string(),
                    previous_version: None,
                })
                .await
        })
        .await;
}

async fn abort_multipart(client: &mut Box<dyn FileObject>, path: &str, upload_id: &str, report: &TrafficReport) {
    report
        .track(PHASE_MULTIPART, "abort_multipart_upload", async {
            client
                .abort_multipart_upload(AbortMultipartUploadRequest {
                    upload_id: upload_id.to_string(),
                    destination_resource_address: path.to_string(),
                })
                .await
        })
        .await;
}
