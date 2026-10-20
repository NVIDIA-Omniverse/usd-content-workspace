// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use crate::fixture::*;
use bytes::{Bytes, BytesMut};
use std::collections::HashMap;

use futures::stream::StreamExt;
use storage_core::traits::filefolder::v1alpha::*;
use storage_core::traits::fileobject::v1alpha::*;
use storage_core::traits::versioning::v1alpha::*;
use storage_core_test::{
    clients::traits::filefolder::FileFolder, clients::traits::fileobject::FileObject,
    clients::traits::versioning::Versioning, headers::HeadersConfig,
};
use tonic::{Code, Status};
use tracing::debug;

pub async fn read_direct(client: &mut dyn FileObject, request: ReadRequest) -> Result<(Metadata, Bytes), Status> {
    let mut response_stream = client.read(request).await?;
    let Some(read_response::ReplyType::Metadata(metadata)) = response_stream
        .next()
        .await
        .expect("unexpected end of the response stream")?
        .reply_type
    else {
        panic!("metadata message expected");
    };

    let mut content = BytesMut::new();
    while let Some(message) = response_stream.next().await {
        let Some(read_response::ReplyType::Chunk(chunk)) = message?.reply_type else {
            panic!("chunk message expected");
        };

        content.extend_from_slice(&chunk.chunk);
    }

    Ok((metadata, content.into()))
}

pub fn random_data(len: usize) -> Vec<u8> {
    let mut data = Vec::with_capacity(len);
    data.resize_with(len, rand::random);
    data
}

pub async fn write_data_body(
    file: &String,
    data: &[u8],
    expected_failure: Option<ExpectedFailure>,
    previous_version: Option<ResourceIdentity>,
    client: &mut Box<dyn FileObject>,
) -> Option<ResourceIdentity> {
    write_data_body_with_options(file, data, expected_failure, previous_version, None, client).await
}

pub async fn write_data_body_with_options(
    file: &String,
    data: &[u8],
    expected_failure: Option<ExpectedFailure>,
    previous_version: Option<ResourceIdentity>,
    fail_if_exists: Option<bool>,
    client: &mut Box<dyn FileObject>,
) -> Option<ResourceIdentity> {
    let write_url = file.clone();
    let write_size = data.len();
    let write_data = data.to_vec();

    let write_response = match client
        .write(
            WriteParameters {
                native_metadata: Default::default(),
                destination_resource_address: write_url.clone(),
                previous_version,
                fail_if_exists,
                data_object_size: write_size as u64,
                upload_preference: Some(UploadPreference::Body.into()),
            },
            Some(Bytes::from(write_data)),
        )
        .await
    {
        Ok(s) => {
            assert!(expected_failure.is_none());
            let write_response::WriteResponseType::ResourceInfo(write_response) = s.write_response_type.unwrap() else {
                panic!("Response should be ResourceInfo");
            };
            write_response
        },
        Err(e) => {
            if let Some(c) = expected_failure {
                assert_eq!(e.code(), c.into());
                return None;
            } else {
                panic!("Write returned unexepected error {e:?}");
            }
        },
    };

    assert_eq!(write_response.metadata.unwrap().data_object_size(), write_size as u64);
    let resource_identity = write_response.resource_identity.unwrap();
    assert!(resource_identity.encoded_identity.contains(file));

    Some(resource_identity)
}

pub async fn write_data_multipart(
    file: &String,
    data: &[u8],
    part_count: usize,
    expected_failure: Option<ExpectedFailure>,
    previous_version: Option<ResourceIdentity>,
    client: &mut Box<dyn FileObject>,
) -> Option<ResourceIdentity> {
    let write_url = file.clone();
    let write_size = data.len();

    // Partition data into exactly part_count chunks
    // Calculate base chunk size and how many bytes are left over
    let base_chunk_size = write_size / part_count;
    let remainder = write_size % part_count;

    // Create exactly part_count chunks
    // The first 'remainder' chunks will have size (base_chunk_size + 1)
    // The remaining chunks will have size base_chunk_size
    let write_chunks: Vec<&[u8]> = (0..part_count)
        .map(|i| {
            let start = i * base_chunk_size + i.min(remainder);
            let chunk_size = if i < remainder {
                base_chunk_size + 1
            } else {
                base_chunk_size
            };
            &data[start..start + chunk_size]
        })
        .collect();
    let mut write_chunks = write_chunks.into_iter();

    let response = match client
        .write(
            WriteParameters {
                native_metadata: Default::default(),
                destination_resource_address: write_url.clone(),
                previous_version: previous_version.clone(),
                fail_if_exists: None,
                data_object_size: write_size as u64,
                upload_preference: Some(UploadPreference::Multipart.into()),
            },
            None,
        )
        .await
    {
        Ok(s) => {
            let write_response::WriteResponseType::MultipartUpload(write_response) = s.write_response_type.unwrap()
            else {
                panic!("Response should be MultipartUpload");
            };
            write_response
        },
        Err(e) => {
            if let Some(c) = expected_failure {
                assert_eq!(e.code(), c.into());
                return None;
            } else {
                panic!("Write returned unexepected error {e:?}");
            }
        },
    };
    let redirect = response.first_part_write_redirect.unwrap();

    // Validate that our supplied headers are returned
    HeadersConfig::new(HashMap::new(), HashMap::new()).validate_headers(&redirect.additional_headers);

    let mut all_headers = Vec::new();

    // Perform the first upload
    all_headers.push(
        perform_upload(
            write_chunks.next().unwrap(),
            None,
            redirect.method,
            &redirect.redirect_target_url,
            redirect.additional_headers,
            &redirect.completion_header_names,
        )
        .await
        .unwrap(),
    );

    // Perform any additional uploads
    if part_count > 1 {
        let part_response = client
            .upload_part(UploadPartRequest {
                upload_id: response.upload_id.clone(),
                destination_resource_address: file.clone(),
                part_number: 1,
                part_count: Some((part_count - 1) as u32),
            })
            .await
            .unwrap();

        assert_eq!(part_response.part_write_redirects.len(), part_count - 1);
        for part in part_response.part_write_redirects {
            // Validate that our supplied headers are returned
            HeadersConfig::new(HashMap::new(), HashMap::new()).validate_headers(&part.additional_headers);
            all_headers.push(
                perform_upload(
                    write_chunks.next().unwrap(),
                    None,
                    redirect.method,
                    &part.redirect_target_url,
                    part.additional_headers,
                    &part.completion_header_names,
                )
                .await
                .unwrap(),
            );
        }
    }

    assert!(write_chunks.next().is_none());

    let complete_status = client
        .complete_multipart_upload(CompleteMultipartUploadRequest {
            upload_id: response.upload_id,
            destination_resource_address: file.clone(),
            parts: all_headers
                .into_iter()
                .enumerate()
                .map(|(n, h)| CompletedUploadPart {
                    part_number: n as u32,
                    headers: h,
                })
                .collect(),
        })
        .await;

    if let Some(c) = expected_failure {
        assert_eq!(complete_status.err().unwrap().code(), c.into());
        None
    } else {
        let complete_status = complete_status.unwrap();
        let resource_info = complete_status.resource_info.unwrap();
        let resource_identity = resource_info.resource_identity.expect("resource identity is missing");

        // Backends don't return size or last modified
        assert!(resource_identity.encoded_identity.contains(file));
        assert!(resource_identity.encoded_identity.contains("etag"));

        Some(resource_identity)
    }
}

pub async fn write_data_redirect(
    file: &str,
    data: &[u8],
    expected_failure: Option<ExpectedFailure>,
    previous_version: Option<ResourceIdentity>,
    client: &mut Box<dyn FileObject>,
    backend: Backend,
) -> Option<ResourceIdentity> {
    let write_url = file.to_string();
    let write_size = data.len();
    let write_data = data.to_vec();
    let redirect = match client
        .write(
            WriteParameters {
                native_metadata: Default::default(),
                destination_resource_address: write_url.clone(),
                previous_version: previous_version.clone(),
                fail_if_exists: None,
                data_object_size: write_size as u64,
                upload_preference: Some(UploadPreference::Redirect.into()),
            },
            None,
        )
        .await
    {
        Ok(s) => {
            debug!(?s);
            let write_response::WriteResponseType::WriteRedirect(write_response) = s.write_response_type.unwrap()
            else {
                panic!("Response should be WriteRedirect");
            };
            write_response
        },
        Err(e) => {
            if let Some(c) = expected_failure {
                assert_eq!(e.code(), c.into());
                return None;
            } else {
                panic!("Write returned unexepected error {e:?}");
            }
        },
    };

    // Validate that our supplied headers are returned
    HeadersConfig::new(HashMap::new(), HashMap::new()).validate_headers(&redirect.additional_headers);

    // Perform the upload
    if let Some(headers) = perform_upload(
        &write_data,
        expected_failure,
        redirect.method,
        &redirect.redirect_target_url,
        redirect.additional_headers,
        &redirect.completion_header_names,
    )
    .await
    {
        let complete_status = client
            .complete_redirect_upload(CompleteRedirectUploadRequest {
                destination_resource_address: file.to_string(),
                additional_headers: headers,
            })
            .await
            .unwrap();

        let resource_info = complete_status.resource_info.unwrap();

        // Resource identity only given back by azure
        if backend == Backend::Azure {
            assert!(resource_info
                .resource_identity
                .as_ref()
                .unwrap()
                .encoded_identity
                .contains(file));
            assert!(resource_info
                .resource_identity
                .as_ref()
                .unwrap()
                .encoded_identity
                .contains("etag"));
        }

        return resource_info.resource_identity;
    }

    None
}

pub async fn perform_upload(
    write_data: &[u8],
    expected_failure: Option<ExpectedFailure>,
    method: i32,
    url: &str,
    additional_headers: Vec<Header>,
    completion_header_names: &Vec<String>,
) -> Option<Vec<Header>> {
    let http_client = reqwest::Client::new();
    let mut builder = match method {
        method if method == UploadMethod::Post as i32 => http_client.post(url),
        method if method == UploadMethod::Put as i32 => http_client.put(url),
        _ => panic!("Upload Method Must be specified"),
    };

    // Add headers
    for header in additional_headers {
        builder = builder.header(header.name, header.value);
    }

    builder = builder.body(write_data.to_vec());
    let http_response = builder.send().await.unwrap();

    if let Some(c) = expected_failure {
        assert_eq!(http_response.status(), reqwest::StatusCode::from(c));
        None
    } else {
        assert!(
            http_response.status() == reqwest::StatusCode::CREATED || http_response.status() == reqwest::StatusCode::OK
        );

        // Gather response headers
        let mut headers = Vec::new();
        for header in completion_header_names {
            if let Some(header_value) = http_response.headers().get(header) {
                headers.push(Header {
                    name: header.clone(),
                    value: header_value.to_str().unwrap().into(),
                });
            }
        }
        Some(headers)
    }
}

// Validates that a file exists and that its data matches the provided data
pub async fn validate_file(file: &String, expected_data: &[u8], client: &mut Box<dyn FileObject>) {
    let response = client
        .stat(StatRequest {
            resource_address: file.clone(),
        })
        .await
        .unwrap();
    let metadata = response.resource_info.unwrap().metadata.unwrap();
    debug!(?metadata);
    assert_eq!(metadata.data_object_size.unwrap(), expected_data.len() as u64);
    assert!(metadata.last_modified_timestamp.unwrap().seconds > 0);

    let mut read_stream = client
        .read_from_address(ReadFromAddressRequest {
            resource_address: file.clone(),
            download_preference: Some(DownloadPreference::Body.into()),
        })
        .await
        .unwrap();

    let first_response = read_stream.next().await;

    // First response should be metadata
    let read_from_address_response::ReplyType::ResourceInfo(m) = first_response.unwrap().unwrap().reply_type.unwrap()
    else {
        panic!("Response should be statresult");
    };
    assert_eq!(
        m.metadata.unwrap().data_object_size.unwrap(),
        expected_data.len() as u64
    );
    assert!(m.resource_identity.unwrap().encoded_identity.contains(file));

    // Now data
    let mut read_data: Vec<u8> = Vec::with_capacity(expected_data.len());
    while let Some(Ok(response)) = read_stream.next().await {
        let read_from_address_response::ReplyType::Chunk(c) = response.reply_type.unwrap() else {
            panic!("Response should be chunk");
        };
        read_data.extend_from_slice(&c.chunk);
    }
    assert_data_eq(expected_data, &read_data);
}

// Validates that a file exists and that its data matches the provided data
pub async fn validate_resource_id(
    resource_id: &ResourceIdentity,
    expected_data: &[u8],
    client: &mut Box<dyn FileObject>,
) {
    tracing::error!("validate_resource_id {:?}", resource_id);
    // Read with body
    let mut read_stream = client
        .read(ReadRequest {
            resource_identity: Some(resource_id.clone()),
            download_preference: Some(DownloadPreference::Body.into()),
        })
        .await
        .unwrap();

    let first_response = read_stream.next().await;

    // First response should be metadata
    let read_response::ReplyType::Metadata(m) = first_response.unwrap().unwrap().reply_type.unwrap() else {
        panic!("Response should be statresult");
    };
    assert_eq!(m.data_object_size.unwrap(), expected_data.len() as u64);
    // Now data
    let mut read_data: Vec<u8> = Vec::with_capacity(expected_data.len());
    while let Some(Ok(response)) = read_stream.next().await {
        let read_response::ReplyType::Chunk(c) = response.reply_type.unwrap() else {
            panic!("Response should be chunk");
        };
        read_data.extend_from_slice(&c.chunk);
    }
    assert_data_eq(expected_data, &read_data);

    // Read with redirect
    let request = ReadRequest {
        resource_identity: Some(resource_id.clone()),
        download_preference: Some(DownloadPreference::Redirect.into()),
    };
    let mut read_stream = client.read(request).await.unwrap();
    debug!(?read_stream);

    // metadata first, save the resource info so we can call read() afterward
    match read_stream.next().await {
        Some(Ok(read_response)) => {
            debug!(?read_response);

            match read_response.reply_type {
                Some(read_response::ReplyType::Metadata(md)) => {
                    debug!(?md);
                    assert_eq!(md.data_object_size.unwrap(), expected_data.len() as u64);
                    // Too hard to track all timestamps for all buckets. Just assert the timestamp is nonzero
                    assert!(md.last_modified_timestamp.unwrap().seconds > 0);
                },
                _ => panic!("Read replied without a resource info as first message!"),
            }
        },
        _ => panic!("Read reply empty!"),
    }

    // this should be redirect to HTTP download
    match read_stream.next().await {
        Some(Ok(read_response)) => {
            debug!(?read_response);

            match read_response.reply_type {
                Some(read_response::ReplyType::Redirect(redirect)) => {
                    // add in any additional headers required
                    let mut headers = reqwest::header::HeaderMap::new();
                    for header in redirect.additional_headers {
                        headers.insert(
                            reqwest::header::HeaderName::from_bytes(header.name.as_bytes()).unwrap(),
                            header.value.parse().unwrap(),
                        );
                    }

                    // http get the file
                    let body = reqwest::Client::new()
                        .get(redirect.redirect_target_url)
                        .headers(headers)
                        .send()
                        .await
                        .unwrap()
                        .bytes()
                        .await
                        .unwrap();
                    assert_data_eq(expected_data, &body);
                },
                _ => panic!("Expected redirect response but got {:?}", read_response.reply_type),
            }
        },
        _ => panic!("Expected redirect response but got empty reply!"),
    }
}

pub async fn copy_file(
    source: ResourceIdentity,
    destination: &String,
    expected_failure: Option<ExpectedFailure>,
    previous_version: Option<ResourceIdentity>,
    write_client: &mut Box<dyn FileObject>,
) -> Option<ResourceIdentity> {
    copy_file_with_options(
        source,
        destination,
        expected_failure,
        previous_version,
        None,
        write_client,
    )
    .await
}

pub async fn copy_file_with_options(
    source: ResourceIdentity,
    destination: &String,
    expected_failure: Option<ExpectedFailure>,
    previous_version: Option<ResourceIdentity>,
    fail_if_exists: Option<bool>,
    write_client: &mut Box<dyn FileObject>,
) -> Option<ResourceIdentity> {
    let copy_status = write_client
        .copy(CopyRequest {
            source_resource_identity: Some(source.clone()),
            destination_resource_address: destination.clone(),
            previous_version: previous_version.clone(),
            fail_if_exists,
        })
        .await;

    if let Some(c) = expected_failure {
        match copy_status {
            Ok(_) => {
                panic!(
                    "Copy did not fail. Source: {:?}; Destination: {:?}; Previous Version: {:?}",
                    source.encoded_identity,
                    destination,
                    previous_version.map(|v| v.encoded_identity)
                );
            },
            Err(e) => {
                assert_eq!(e.code(), c.into());
                None
            },
        }
    } else {
        let resource_identity = match copy_status {
            Ok(copy_status) => copy_status.resource_identity.unwrap(),
            Err(e) => {
                panic!(
                    "Copy failed unexpectedly. Error: {:?}; Source: {:?}; Destination: {:?}; Previous Version: {:?}",
                    e,
                    source.encoded_identity,
                    destination,
                    previous_version.map(|v| v.encoded_identity)
                );
            },
        };
        assert!(resource_identity.encoded_identity.contains(destination));

        Some(resource_identity)
    }
}

pub async fn delete_file(
    file: &str,
    expected_failure: Option<ExpectedFailure>,
    previous_version: Option<ResourceIdentity>,
    file_object_client: &mut Box<dyn FileObject>,
) {
    let delete_status = file_object_client
        .delete(DeleteRequest {
            resource_address: file.to_owned(),
            previous_version,
        })
        .await;

    tracing::error!(?delete_status);
    if let Some(c) = expected_failure {
        assert_eq!(delete_status.err().unwrap().code(), c.into());
    } else {
        delete_status.unwrap();
    }
}

pub async fn move_file(
    source_resource_address: &String,
    destination_resource_address: &String,
    expected_failure: Option<ExpectedFailure>,
    source_previous_version: Option<ResourceIdentity>,
    destination_previous_version: Option<ResourceIdentity>,
    write_client: &mut Box<dyn FileObject>,
) -> Option<ResourceIdentity> {
    move_file_with_options(
        source_resource_address,
        destination_resource_address,
        expected_failure,
        source_previous_version,
        destination_previous_version,
        None,
        write_client,
    )
    .await
}

pub async fn move_file_with_options(
    source_resource_address: &String,
    destination_resource_address: &String,
    expected_failure: Option<ExpectedFailure>,
    source_previous_version: Option<ResourceIdentity>,
    destination_previous_version: Option<ResourceIdentity>,
    fail_if_exists: Option<bool>,
    write_client: &mut Box<dyn FileObject>,
) -> Option<ResourceIdentity> {
    let move_status = write_client
        .move_object(MoveRequest {
            source_resource_address: source_resource_address.clone(),
            destination_resource_address: destination_resource_address.clone(),
            source_previous_version: source_previous_version.clone(),
            destination_previous_version: destination_previous_version.clone(),
            fail_if_exists,
        })
        .await;

    if let Some(c) = expected_failure {
        match move_status {
            Ok(_) => {
                panic!(
                    "Move did not fail. Source: {:?}; Destination: {:?}; Source Previous Version: {:?}; Destination Previous Version: {:?}",
                    source_resource_address,
                    destination_resource_address,
                    source_previous_version.map(|v| v.encoded_identity),
                    destination_previous_version.map(|v| v.encoded_identity)
                );
            },
            Err(e) => {
                assert_eq!(e.code(), c.into());
                None
            },
        }
    } else {
        let resource_identity = match move_status {
            Ok(move_status) => move_status.resource_identity.unwrap(),
            Err(e) => {
                panic!(
                    "Move failed unexpectedly. Error: {:?}; Source: {:?}; Destination: {:?}; Source Previous Version: {:?}; Destination Previous Version: {:?}",
                    e,
                    source_resource_address,
                    destination_resource_address,
                    source_previous_version.map(|v| v.encoded_identity),
                    destination_previous_version.map(|v| v.encoded_identity)
                );
            },
        };
        assert!(resource_identity
            .encoded_identity
            .contains(destination_resource_address));

        Some(resource_identity)
    }
}

/// Extract the etag query parameter from a ResourceIdentity
pub fn get_etag_from_encoded_identity(resource_identity: &ResourceIdentity) -> Option<String> {
    use url::Url;

    let url = Url::parse(&resource_identity.encoded_identity).ok()?;
    url.query_pairs()
        .find(|(key, _)| key == "etag")
        .map(|(_, value)| value.to_string())
}

pub async fn list_folder(
    folder: &str,
    expected_failure: Option<Code>,
    expected_subfolders: Vec<&str>,
    expected_objects: Vec<&str>,
    client: &mut Box<dyn FileFolder>,
) {
    let request = ListRequest {
        folder: Some(FolderAddress {
            uri: folder.to_string(),
        }),
    };
    let response = client.list(request).await;

    // This call could error, see if it's expected
    if let Err(e) = response {
        if let Some(c) = expected_failure {
            assert_eq!(e.code(), c);
            return;
        } else {
            panic!("List folder failed unexpectedly: {:?}", e);
        }
    }

    let mut responses_stream = response.unwrap();
    let mut responses = Vec::new();

    while let Some(response) = responses_stream.next().await {
        // The error could also come back in the stream
        if let Err(e) = response {
            if let Some(c) = expected_failure {
                assert_eq!(e.code(), c);
                return;
            } else {
                panic!("List folder failed unexpectedly: {:?}", e);
            }
        }
        responses.push(response.unwrap());
    }

    let mut subfolders = Vec::new();
    let mut objects = Vec::new();
    for response in responses {
        subfolders.extend(response.subfolder_addresses);
        objects.extend(response.sub_resource_addresses);
    }

    debug!(?subfolders, ?objects);

    assert_eq!(subfolders.len(), expected_subfolders.len());
    assert_eq!(objects.len(), expected_objects.len());
    for subfolder in expected_subfolders {
        assert!(subfolders.contains(&FolderAddress {
            uri: subfolder.to_string()
        }));
    }
    for object in expected_objects {
        assert!(objects.contains(&object.to_string()));
    }
}

pub async fn get_latest_version_id(resource_address: impl Into<String>, client: &mut Box<dyn Versioning>) -> String {
    let request = EnumerateVersionsRequest {
        resource_address: resource_address.into(),
    };
    let mut stream = client.enumerate_versions(request).await.unwrap();
    let mut responses = Vec::new();
    let mut versions_order = None;

    while let Some(response) = stream.next().await {
        let response = response.unwrap();

        // Assert that the versions order is the same for all responses ¯\_(ツ)_/¯
        match versions_order {
            Some(order) => assert_eq!(order, response.versions_order),
            None => versions_order = Some(response.versions_order),
        }

        for item in response.items {
            responses.push(item.resource_info.unwrap().resource_identity.unwrap().encoded_identity);
        }
    }

    match versions_order.map(|order| VersionsOrder::try_from(order).unwrap()) {
        Some(VersionsOrder::NewestFirst) => responses.first().unwrap().clone(),
        Some(VersionsOrder::OldestFirst) => responses.last().unwrap().clone(),
        _ => panic!("Unsupported versions order: {:?}", versions_order),
    }
}

/// Asserts that two byte slices are equal, providing useful diagnostics on failure
/// without printing the entire buffer contents.
pub fn assert_data_eq(expected: &[u8], actual: &[u8]) {
    if expected == actual {
        return;
    }

    if expected.len() != actual.len() {
        panic!(
            "Data length mismatch: expected {} bytes, got {} bytes",
            expected.len(),
            actual.len()
        );
    }

    if let Some((idx, (exp, act))) = expected
        .iter()
        .zip(actual.iter())
        .enumerate()
        .find(|(_, (a, b))| a != b)
    {
        panic!(
            "Data mismatch at byte {} (of {}): expected 0x{:02x}, got 0x{:02x}",
            idx,
            expected.len(),
            exp,
            act
        );
    }
}
