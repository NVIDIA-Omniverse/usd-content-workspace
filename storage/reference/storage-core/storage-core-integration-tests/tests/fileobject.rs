// SPDX-FileCopyrightText: Copyright (c) 2024 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

#![allow(clippy::too_many_arguments)]

use std::collections::HashMap;

use assert_matches::assert_matches;
use bytes::Bytes;
// Native-metadata round-trip seeding (writes are rejected under coherent-false, so seed via the SDK).
use aws_sdk_s3 as seed_s3;
use azure_storage_blobs::prelude::ClientBuilder as AzureClientBuilder;
use bytesize::MIB;
use futures::stream::StreamExt;
use hex_literal::hex;
use percent_encoding::{percent_decode_str, utf8_percent_encode, NON_ALPHANUMERIC};
use reqwest::header::{HeaderMap, HeaderName};
use rstest::*;
use sha2::{Digest, Sha256};
use tonic::Code;
use tracing::{debug, trace};

use storage_core::traits::{
    filefolder::v1alpha::{FolderAddress, ListStatRequest},
    fileobject::v1alpha::*,
    versioning::v1alpha::EnumerateVersionsRequest,
};
use storage_core_integration_tests::cdn_storage_fixture;
use storage_core_integration_tests::fixture::*;
use storage_core_integration_tests::utils::*;
use storage_core_integration_tests::{
    assert_method_metric, assert_metric, assume_role_storage_fixture, storage_fixture,
};
use storage_core_test::{
    clients::rest::fileobject::FileObjectRestClient, headers::HeadersConfig,
    identity::assert_identity_has_object_identifier_parameters, service::ApiVersion, service::Interface,
};
use url::Url;
#[rstest]
#[case::case_01_empty_address("")]
#[case::case_02_plain_tonic("Tonic")]
#[case::case_03_invalid_scheme("htp://exampleaccount.blob.core.windows.net/astronaut.usd")]
#[case::case_04_missing_scheme("exampleaccount.blob.core.windows.net/astronaut.usd")]
#[case::case_05_s3_scheme("s3://content-prod.s3.us-west-2.amazonaws.com/astronaut.usd")]
#[case::case_06_garbage("ruj08n2-[erjk-[r-[3")]
#[case::case_07_underscores("________________")]
#[test_log::test(tokio::test)]
async fn invalid_requests(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure, Backend::Minio, Backend::GCPS3Compat)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[values(Cache::List, Cache::Stat, Cache::Disabled)] cache: Cache,
    #[case] address: &str,
) {
    let mut client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;

    // stat
    let request = StatRequest {
        resource_address: address.into(),
    };
    let response = client.stat(request).await.map_err(|s| s.code());
    assert_matches!(response, Err(tonic::Code::InvalidArgument));

    // read from address
    let request = ReadFromAddressRequest {
        resource_address: address.into(),
        download_preference: None,
    };
    let response = client.read_from_address(request).await.map_err(|s| s.code());
    assert_matches!(response, Err(tonic::Code::InvalidArgument));

    // read
    let request = ReadRequest {
        resource_identity: Some(ResourceIdentity {
            encoded_identity: address.into(),
        }),
        download_preference: None,
    };
    // With the read coalescing in the small object cache, it's possible to get the error from the call directly
    // or from the stream, so we need to check both
    let response = client.read(request).await;
    let failed = match response {
        Ok(mut stream) => match stream.next().await {
            Some(response) => match response {
                Ok(_) => false,
                Err(s) => {
                    assert_matches!(s.code(), tonic::Code::InvalidArgument);
                    true
                },
            },
            None => false,
        },
        Err(s) => {
            assert_matches!(s.code(), tonic::Code::InvalidArgument);
            true
        },
    };
    assert!(failed);

    // Validate sending the wrong backend address doesn't work
    let address = match backend {
        Backend::Azure => "https://example-bucket.s3.us-west-2.amazonaws.com/astronaut.usd",
        _ => "https://exampleaccount.blob.core.windows.net/example-container/astronaut.usd",
    };
    let request = StatRequest {
        resource_address: address.into(),
    };
    let response = client.stat(request).await.map_err(|s| s.code());
    assert_matches!(response, Err(tonic::Code::InvalidArgument));

    // The Rest interface doesn't actually issue some of these invalid requests, so we only check the Grpc interface
    if interface == Interface::Grpc {
        // Because tests run simultaneously, we can't know the exact metric value, but it must be greater than 0
        assert_method_metric!(
            storage_fixture,
            backend,
            api_version,
            "storage.requests",
            "stat",
            ["result" => "InvalidArgument"]
        );
        assert_method_metric!(
            storage_fixture,
            backend,
            api_version,
            "storage.requests",
            "read_from_address",
            ["result" => "InvalidArgument"]
        );
    }
}
#[rstest]
#[test_log::test(tokio::test)]
async fn stat_blob_name_too_long(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[values(Cache::Disabled)] cache: Cache,
) {
    let long_name = "a".repeat(1025);
    let Some(address) = storage_fixture.make_url(backend, Access::Public, &long_name) else {
        return;
    };

    let mut client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;

    let request = StatRequest {
        resource_address: address.clone(),
    };
    let response = client.stat(request).await.map_err(|s| s.code());
    assert_matches!(response, Err(tonic::Code::InvalidArgument));
}
#[rstest]
#[test_log::test(tokio::test)]
async fn write_blob_name_too_long(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[values(Cache::Disabled)] cache: Cache,
) {
    let long_name = "a".repeat(1025);
    let Some(address) = storage_fixture.make_url(backend, Access::Public, &long_name) else {
        return;
    };

    let mut client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;

    write_data_body(
        &address,
        b"test data",
        Some(ExpectedFailure::InvalidArgument),
        None,
        &mut client,
    )
    .await;
}
#[rstest]
#[test_log::test(tokio::test)]
async fn copy_blob_name_too_long(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(ApiVersion::V1Alpha)] api_version: ApiVersion,
    #[values(Cache::Disabled)] cache: Cache,
) {
    let mut client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;
    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, Access::PrivateKey, cache, api_version)
        .await
    else {
        return;
    };

    // Write a valid source file
    let src_file = temp_dir.new_file();
    let data = random_data(64);
    let resource_id = write_data_body(&src_file, &data, None, None, &mut client).await;

    // Copy to an invalid (too-long) destination
    let long_name = "a".repeat(1025);
    let Some(dst_address) = storage_fixture.make_url(backend, Access::PrivateKey, &long_name) else {
        temp_dir.cleanup().await;
        return;
    };

    copy_file(
        resource_id.unwrap(),
        &dst_address,
        Some(ExpectedFailure::InvalidArgument),
        None,
        &mut client,
    )
    .await;

    temp_dir.cleanup().await;
}
#[rstest]
#[test_log::test(tokio::test)]
async fn move_blob_name_too_long(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(ApiVersion::V1Alpha)] api_version: ApiVersion,
    #[values(Cache::Disabled)] cache: Cache,
) {
    let mut client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;
    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, Access::PrivateKey, cache, api_version)
        .await
    else {
        return;
    };

    // Write a valid source file
    let src_file = temp_dir.new_file();
    let data = random_data(64);
    write_data_body(&src_file, &data, None, None, &mut client).await;

    // Move to an invalid (too-long) destination
    let long_name = "a".repeat(1025);
    let Some(dst_address) = storage_fixture.make_url(backend, Access::PrivateKey, &long_name) else {
        temp_dir.cleanup().await;
        return;
    };

    move_file(
        &src_file,
        &dst_address,
        Some(ExpectedFailure::InvalidArgument),
        None,
        None,
        &mut client,
    )
    .await;

    temp_dir.cleanup().await;
}
#[rstest]
#[test_log::test(tokio::test)]
async fn stat_blob_name_control_chars(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[values(Cache::Disabled)] cache: Cache,
) {
    let Some(address) = storage_fixture.make_url(backend, Access::Public, "blob%00name") else {
        return;
    };

    let mut client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;

    let request = StatRequest {
        resource_address: address.clone(),
    };
    let response = client.stat(request).await.map_err(|s| s.code());
    assert_matches!(response, Err(tonic::Code::InvalidArgument));
}
#[rstest]
#[test_log::test(tokio::test)]
async fn none_requests(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure, Backend::Minio, Backend::GCPS3Compat)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[values(Cache::List, Cache::Stat, Cache::Disabled)] cache: Cache,
) {
    let mut client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;

    // read where resource identity is none
    let request = ReadRequest {
        resource_identity: None,
        download_preference: None,
    };
    let response = client.read(request).await.map_err(|s| s.code());
    assert_matches!(response, Err(tonic::Code::InvalidArgument));
}

fn resource_address_from_identity(identity: &ResourceIdentity) -> String {
    let mut url = Url::parse(&identity.encoded_identity).expect("resource identity should be a valid URL");
    url.set_query(None);
    url.set_fragment(None);
    url.to_string()
}

fn versioned_resource_address_from_identity(identity: &ResourceIdentity) -> String {
    let mut url = Url::parse(&identity.encoded_identity).expect("resource identity should be a valid URL");
    let version_id = url
        .query_pairs()
        .find(|(name, _)| name == "version-id")
        .map(|(_, value)| value.into_owned())
        .expect("versioned resource identity should contain version-id");
    url.set_query(None);
    url.set_fragment(None);
    url.query_pairs_mut().append_pair("version-id", &version_id);
    url.to_string()
}

fn paths_address_same_object(left: &str, right: &str) -> bool {
    let Ok(left) = Url::parse(left) else {
        return false;
    };
    let Ok(right) = Url::parse(right) else {
        return false;
    };
    left.scheme() == right.scheme()
        && left.host_str() == right.host_str()
        && percent_decode_str(left.path()).eq(percent_decode_str(right.path()))
}

async fn assert_list_stat_returns_exact_identity(
    client: &mut Box<dyn storage_core_test::clients::traits::filefolder::FileFolder>,
    folder: &str,
    expected_identity: &ResourceIdentity,
) {
    let expected_address = resource_address_from_identity(expected_identity);
    let mut stream = client
        .list_stat(ListStatRequest {
            include_native_metadata: false,
            folder: Some(FolderAddress {
                uri: folder.to_string(),
            }),
        })
        .await
        .expect("listStat should return a stream");

    while let Some(response) = stream.next().await {
        for entry in response.expect("listStat stream should succeed").entries {
            if paths_address_same_object(&entry.resource_address, &expected_address) {
                assert_eq!(entry.resource_address, expected_address);
                assert_eq!(
                    entry
                        .resource_info
                        .expect("listStat entry should contain resource info")
                        .resource_identity
                        .expect("listStat resource info should contain an identity")
                        .encoded_identity,
                    expected_identity.encoded_identity
                );
                return;
            }
        }
    }

    panic!("listStat did not return {expected_address}");
}

async fn assert_enumerate_versions_returns_exact_identity(
    client: &mut Box<dyn storage_core_test::clients::traits::versioning::Versioning>,
    resource_address: &str,
    expected_identity: &ResourceIdentity,
) {
    let expected_versioned_address = versioned_resource_address_from_identity(expected_identity);
    let mut stream = client
        .enumerate_versions(EnumerateVersionsRequest {
            resource_address: resource_address.to_string(),
        })
        .await
        .expect("enumerate_versions should return a stream");

    while let Some(response) = stream.next().await {
        for item in response.expect("enumerate_versions stream should succeed").items {
            let Some(item_address) = item.resource_address else {
                continue;
            };
            if paths_address_same_object(&item_address, &expected_versioned_address) {
                assert_eq!(item_address, expected_versioned_address);
                assert_eq!(
                    item.resource_info
                        .expect("version should contain resource info")
                        .resource_identity
                        .expect("version resource info should contain an identity")
                        .encoded_identity,
                    expected_identity.encoded_identity
                );
                return;
            }
        }
    }

    panic!("enumerate_versions did not return {expected_versioned_address}");
}

async fn assert_stat_returns_exact_identity(
    client: &mut Box<dyn storage_core_test::clients::traits::fileobject::FileObject>,
    resource_address: &str,
    expected_identity: &ResourceIdentity,
    expected_size: usize,
) {
    let resource_info = client
        .stat(StatRequest {
            resource_address: resource_address.to_string(),
        })
        .await
        .expect("stat should succeed")
        .resource_info
        .expect("stat should return resource info");
    assert_eq!(
        resource_info
            .metadata
            .expect("stat should return metadata")
            .data_object_size,
        Some(expected_size as u64)
    );
    assert_eq!(
        resource_info
            .resource_identity
            .expect("stat should return an identity")
            .encoded_identity,
        expected_identity.encoded_identity
    );
}

async fn assert_read_from_address_returns_exact_identity(
    client: &mut Box<dyn storage_core_test::clients::traits::fileobject::FileObject>,
    resource_address: &str,
    expected_identity: &ResourceIdentity,
    expected_data: &[u8],
) {
    let mut stream = client
        .read_from_address(ReadFromAddressRequest {
            resource_address: resource_address.to_string(),
            download_preference: Some(DownloadPreference::Body.into()),
        })
        .await
        .expect("read_from_address should return a stream");
    let resource_info = match stream
        .next()
        .await
        .expect("read_from_address should return resource info")
        .expect("read_from_address stream should succeed")
        .reply_type
        .expect("read_from_address response should have a reply type")
    {
        read_from_address_response::ReplyType::ResourceInfo(resource_info) => resource_info,
        other => panic!("expected resource info, got {other:?}"),
    };
    assert_eq!(
        resource_info
            .resource_identity
            .expect("read_from_address should return an identity")
            .encoded_identity,
        expected_identity.encoded_identity
    );

    let mut body = Vec::new();
    while let Some(response) = stream.next().await {
        match response
            .expect("read_from_address stream should succeed")
            .reply_type
            .expect("read_from_address response should have a reply type")
        {
            read_from_address_response::ReplyType::Chunk(chunk) => body.extend_from_slice(&chunk.chunk),
            other => panic!("expected body chunk, got {other:?}"),
        }
    }
    assert_eq!(body, expected_data);
}

async fn assert_enumerate_returns_exact_identity(
    client: &mut Box<dyn storage_core_test::clients::traits::fileobject::FileObject>,
    resource_address: &str,
    expected_identity: &ResourceIdentity,
) {
    let expected_address = resource_address_from_identity(expected_identity);
    let mut stream = client
        .enumerate(EnumerateRequest {
            resource_address: resource_address.to_string(),
        })
        .await
        .expect("enumerate should return a stream");
    while let Some(response) = stream.next().await {
        for item in response.expect("enumerate stream should succeed").items {
            if paths_address_same_object(&item.resource_address, &expected_address) {
                assert_eq!(item.resource_address, expected_address);
                return;
            }
        }
    }
    panic!("enumerate did not return {expected_address}");
}

async fn assert_rest_read_returns_exact_identity_header(
    storage_fixture: &StorageFixture,
    backend: Backend,
    cache: Cache,
    api_version: ApiVersion,
    request_identity: &str,
    expected_identity: &str,
) {
    let version = match api_version {
        ApiVersion::V1Alpha => "v1alpha",
        ApiVersion::V1Beta => "v1beta",
    };
    let encoded_identity = utf8_percent_encode(request_identity, NON_ALPHANUMERIC);
    let path = format!("{version}/fileobject/by-identity/{encoded_identity}");
    let mut client = FileObjectRestClient::new(storage_fixture.rest_port(backend, cache), HeadersConfig::default());
    let response = client
        .read(&path, Some("body"))
        .await
        .expect("REST read-by-identity should succeed");
    assert_eq!(
        response
            .headers()
            .get("x-nvidia-omniverse-storage-resource-identity")
            .expect("REST read-by-identity should return an identity header")
            .to_str()
            .expect("identity header should be valid text"),
        expected_identity
    );
}

fn loose_identity_spelling(identity: &str) -> String {
    let mut url = Url::parse(identity).expect("resource identity should be a valid URL");
    let loose_path = url
        .path()
        .replace("%28", "(")
        .replace("%29", ")")
        .replace("%2B", "+")
        .replace("%5B", "[")
        .replace("%5D", "]");
    url.set_path(&loose_path);
    url.to_string()
}

async fn write_body_returning_identity(
    client: &mut Box<dyn storage_core_test::clients::traits::fileobject::FileObject>,
    resource_address: &str,
    data: &[u8],
    previous_version: Option<ResourceIdentity>,
) -> ResourceIdentity {
    let response = client
        .write(
            WriteParameters {
                native_metadata: Default::default(),
                destination_resource_address: resource_address.to_string(),
                previous_version,
                fail_if_exists: None,
                data_object_size: data.len() as u64,
                upload_preference: Some(UploadPreference::Body.into()),
            },
            Some(Bytes::copy_from_slice(data)),
        )
        .await
        .expect("write should succeed");
    let write_response::WriteResponseType::ResourceInfo(resource_info) = response
        .write_response_type
        .expect("write should return a response type")
    else {
        panic!("body write should return resource info");
    };
    resource_info
        .resource_identity
        .expect("write resource info should contain an identity")
}

#[rstest]
#[test_log::test(tokio::test)]
async fn canonical_encoded_identity_matches_backend_listing(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
) {
    let api_version = ApiVersion::V1Alpha;
    let interface = Interface::Rest;
    let cache = Cache::Disabled;
    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, Access::Versioned, cache, api_version)
        .await
    else {
        debug!("Skipping test — no versioned bucket fixture for {:?}", backend);
        return;
    };
    let mut file_client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;
    let mut folder_client = storage_fixture
        .file_folder_client(backend, cache, interface, api_version)
        .await;
    let mut versioning_client = storage_fixture
        .versioning_client(backend, cache, interface, api_version)
        .await;
    let data = Bytes::from_static(b"canonical identity test data");

    let write_address = format!("{}/write ( ) + [ ].bin", temp_dir.url());
    let write_response = file_client
        .write(
            WriteParameters {
                native_metadata: Default::default(),
                destination_resource_address: write_address.clone(),
                previous_version: None,
                fail_if_exists: None,
                data_object_size: data.len() as u64,
                upload_preference: Some(UploadPreference::Body.into()),
            },
            Some(data.clone()),
        )
        .await
        .expect("write should succeed");
    let write_response::WriteResponseType::ResourceInfo(write_info) = write_response
        .write_response_type
        .expect("write should return resource info")
    else {
        panic!("write should return resource info");
    };
    let write_identity = write_info
        .resource_identity
        .expect("write resource info should contain an identity");
    assert_list_stat_returns_exact_identity(&mut folder_client, temp_dir.url(), &write_identity).await;
    assert_stat_returns_exact_identity(&mut file_client, &write_address, &write_identity, data.len()).await;
    assert_read_from_address_returns_exact_identity(&mut file_client, &write_address, &write_identity, &data).await;
    assert_rest_read_returns_exact_identity_header(
        storage_fixture,
        backend,
        cache,
        api_version,
        &loose_identity_spelling(&write_identity.encoded_identity),
        &write_identity.encoded_identity,
    )
    .await;

    let copy_address = format!("{}/copy ( ) + [ ].bin", temp_dir.url());
    let copy_identity = file_client
        .copy(CopyRequest {
            source_resource_identity: Some(write_identity),
            destination_resource_address: copy_address.clone(),
            previous_version: None,
            fail_if_exists: None,
        })
        .await
        .expect("copy should succeed")
        .resource_identity
        .expect("copy should return an identity");
    assert_list_stat_returns_exact_identity(&mut folder_client, temp_dir.url(), &copy_identity).await;

    let move_address = format!("{}/move ( ) + [ ].bin", temp_dir.url());
    let move_identity = file_client
        .move_object(MoveRequest {
            source_resource_address: copy_address,
            destination_resource_address: move_address,
            source_previous_version: Some(copy_identity),
            destination_previous_version: None,
            fail_if_exists: None,
        })
        .await
        .expect("move should succeed")
        .resource_identity
        .expect("move should return an identity");
    assert_list_stat_returns_exact_identity(&mut folder_client, temp_dir.url(), &move_identity).await;

    let multipart_address = format!("{}/multipart ( ) + [ ].bin", temp_dir.url());
    let multipart = file_client
        .write(
            WriteParameters {
                native_metadata: Default::default(),
                destination_resource_address: multipart_address.clone(),
                previous_version: None,
                fail_if_exists: None,
                data_object_size: data.len() as u64,
                upload_preference: Some(UploadPreference::Multipart.into()),
            },
            None,
        )
        .await
        .expect("multipart write preparation should succeed");
    let write_response::WriteResponseType::MultipartUpload(multipart) = multipart
        .write_response_type
        .expect("multipart write preparation should return upload information")
    else {
        panic!("multipart write preparation should return upload information");
    };
    let redirect = multipart
        .first_part_write_redirect
        .expect("multipart write should return a first-part redirect");
    let completion_headers = perform_upload(
        &data,
        None,
        redirect.method,
        &redirect.redirect_target_url,
        redirect.additional_headers,
        &redirect.completion_header_names,
    )
    .await
    .expect("multipart part upload should succeed");
    let multipart_identity = file_client
        .complete_multipart_upload(CompleteMultipartUploadRequest {
            upload_id: multipart.upload_id,
            destination_resource_address: multipart_address.clone(),
            parts: vec![CompletedUploadPart {
                part_number: 0,
                headers: completion_headers,
            }],
        })
        .await
        .expect("multipart completion should succeed")
        .resource_info
        .expect("multipart completion should return resource info")
        .resource_identity
        .expect("multipart completion resource info should contain an identity");
    assert_list_stat_returns_exact_identity(&mut folder_client, temp_dir.url(), &multipart_identity).await;
    assert_enumerate_versions_returns_exact_identity(&mut versioning_client, &multipart_address, &multipart_identity)
        .await;

    let redirect_address = format!("{}/redirect ( ) + [ ].bin", temp_dir.url());
    let redirect_response = file_client
        .write(
            WriteParameters {
                native_metadata: Default::default(),
                destination_resource_address: redirect_address.clone(),
                previous_version: None,
                fail_if_exists: None,
                data_object_size: data.len() as u64,
                upload_preference: Some(UploadPreference::Redirect.into()),
            },
            None,
        )
        .await
        .expect("redirect write preparation should succeed");
    let write_response::WriteResponseType::WriteRedirect(redirect) = redirect_response
        .write_response_type
        .expect("redirect write preparation should return upload information")
    else {
        panic!("redirect write preparation should return a redirect");
    };
    let completion_headers = perform_upload(
        &data,
        None,
        redirect.method,
        &redirect.redirect_target_url,
        redirect.additional_headers,
        &redirect.completion_header_names,
    )
    .await
    .expect("redirect upload should succeed");
    let completed_redirect_info = file_client
        .complete_redirect_upload(CompleteRedirectUploadRequest {
            destination_resource_address: redirect_address.clone(),
            additional_headers: completion_headers,
        })
        .await
        .expect("redirect completion should succeed")
        .resource_info
        .expect("redirect completion should return resource info");
    let redirect_identity = match completed_redirect_info.resource_identity {
        Some(identity) => identity,
        None => file_client
            .stat(StatRequest {
                resource_address: redirect_address.clone(),
            })
            .await
            .expect("completed redirect should be stat-able")
            .resource_info
            .expect("stat should return resource info")
            .resource_identity
            .expect("stat should return an identity"),
    };
    assert_list_stat_returns_exact_identity(&mut folder_client, temp_dir.url(), &redirect_identity).await;

    for identity in [&move_identity, &multipart_identity, &redirect_identity] {
        assert_enumerate_returns_exact_identity(&mut file_client, temp_dir.url(), identity).await;
    }

    temp_dir.cleanup().await;
}

// Pairwise cache/API/interface coverage for canonical return values. Each case performs a cold
// lookup, repeats it using the alternate URL spelling to exercise the warm cache, then overwrites
// through the opposite spelling and verifies that the original spelling does not return stale
// metadata. Cache::List also enables the small-object cache, so the paired read-by-identity calls
// exercise both spellings of its body and metadata keys.
#[rstest]
#[case::s3_stat_grpc_v1beta(Backend::S3, Cache::Stat, Interface::Grpc, ApiVersion::V1Beta)]
#[case::azure_stat_rest_v1beta(Backend::Azure, Cache::Stat, Interface::Rest, ApiVersion::V1Beta)]
#[case::s3_list_rest_v1alpha(Backend::S3, Cache::List, Interface::Rest, ApiVersion::V1Alpha)]
#[case::azure_list_grpc_v1beta(Backend::Azure, Cache::List, Interface::Grpc, ApiVersion::V1Beta)]
#[test_log::test(tokio::test)]
async fn canonical_identity_survives_cache_hits_and_cross_spelling_invalidation(
    storage_fixture: &StorageFixture,
    #[case] backend: Backend,
    #[case] cache: Cache,
    #[case] interface: Interface,
    #[case] api_version: ApiVersion,
) {
    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, Access::PrivateKey, cache, api_version)
        .await
    else {
        return;
    };
    let mut file_client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;
    let mut folder_client = storage_fixture
        .file_folder_client(backend, cache, interface, api_version)
        .await;

    let loose_address = format!("{}/cached ( ) + [ ].bin", temp_dir.url());
    let canonical_address = format!("{}/cached%20%28%20%29%20%2B%20%5B%20%5D.bin", temp_dir.url());
    let initial_data = b"cold-cache";
    let initial_identity = write_body_returning_identity(&mut file_client, &loose_address, initial_data, None).await;
    assert_eq!(resource_address_from_identity(&initial_identity), canonical_address);

    // Cold loose lookup followed by repeated strict/loose warm lookups.
    assert_stat_returns_exact_identity(&mut file_client, &loose_address, &initial_identity, initial_data.len()).await;
    for address in [&canonical_address, &loose_address] {
        assert_stat_returns_exact_identity(&mut file_client, address, &initial_identity, initial_data.len()).await;
        assert_read_from_address_returns_exact_identity(&mut file_client, address, &initial_identity, initial_data)
            .await;
    }

    let loose_initial_identity = ResourceIdentity {
        encoded_identity: loose_identity_spelling(&initial_identity.encoded_identity),
    };
    for identity in [&loose_initial_identity, &initial_identity] {
        let (_, body) = read_direct(
            &mut *file_client,
            ReadRequest {
                resource_identity: Some(identity.clone()),
                download_preference: Some(DownloadPreference::Body.into()),
            },
        )
        .await
        .expect("read-by-identity should succeed");
        assert_eq!(body, initial_data.as_slice());
    }
    if interface == Interface::Rest {
        assert_rest_read_returns_exact_identity_header(
            storage_fixture,
            backend,
            cache,
            api_version,
            &loose_initial_identity.encoded_identity,
            &initial_identity.encoded_identity,
        )
        .await;
    }

    // Populate listing/enumeration payloads, then mutate through the strict spelling. Looking up
    // through the original loose spelling must observe the new object, proving cross-spelling
    // invalidation instead of merely asserting that two independent cache entries look valid.
    assert_list_stat_returns_exact_identity(&mut folder_client, temp_dir.url(), &initial_identity).await;
    assert_enumerate_returns_exact_identity(&mut file_client, temp_dir.url(), &initial_identity).await;
    let replacement_data = b"replacement-cache-body";
    let replacement_identity = write_body_returning_identity(
        &mut file_client,
        &canonical_address,
        replacement_data,
        Some(initial_identity),
    )
    .await;
    assert_eq!(resource_address_from_identity(&replacement_identity), canonical_address);
    assert_stat_returns_exact_identity(
        &mut file_client,
        &loose_address,
        &replacement_identity,
        replacement_data.len(),
    )
    .await;
    assert_read_from_address_returns_exact_identity(
        &mut file_client,
        &loose_address,
        &replacement_identity,
        replacement_data,
    )
    .await;
    assert_list_stat_returns_exact_identity(&mut folder_client, temp_dir.url(), &replacement_identity).await;
    assert_enumerate_returns_exact_identity(&mut file_client, temp_dir.url(), &replacement_identity).await;

    temp_dir.cleanup().await;
}

// Small stat test - selected by the nextest small profile (limited test cases for local efficiency)
#[rstest]
#[case::case_01_empty_file("unittests/stat/empty.txt", 0)]
#[case::case_02_lorem_ipsum("unittests/readFile/Lorem ipsum.txt", 3549)]
#[test_log::test(tokio::test)]
async fn stat_basic(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[values((Cache::List, 10), (Cache::Stat, 10), (Cache::Disabled, 1))] cache: (Cache, usize),
    #[case] base_address: &str,
    #[case] size: u64,
) {
    stat_impl(
        storage_fixture,
        backend,
        interface,
        access,
        api_version,
        cache,
        base_address,
        size,
    )
    .await;
}

// Full stat test - selected by the nextest full profile (comprehensive test coverage)
#[rstest]
#[case::case_01_empty_file("unittests/stat/empty.txt", 0)]
#[case::case_02_lorem_ipsum("unittests/readFile/Lorem ipsum.txt", 3549)]
#[case::case_03_splash_jpg("unittests/stat/sub-folder/splash.jpg", 241619)]
#[case::case_04_jellyfish_hevc("unittests/stat/jellyfish - hevc.mkv", 37541091)]
#[case::case_05_french_accents("unittests/specialchars/french_accents_âàäçéèêëîïôùûüœ.txt", 0)]
#[case::case_06_pekannot("unittests/specialchars/pekannöt.txt", 5670)]
#[case::case_07_encoded_plus_spaces("unittests/specialchars/test%2Bfile%20with%20spaces.txt", 30)]
// URLs need to be properly encoded prior to being sent to the gRPC API, otherwise its impossible to distinguish
// between valid S3 keys and urls parts, like url fragments, since '#' is valid in an S3 key
// https://<public-bucket>.s3.us-west-2.amazonaws.com/unittests/specialchars/folder<space><tab>~!@#$%^&*()_+`1234567890-=[]{}|:;\"'<>,?\\/test<space>with<space>plus<space>+<space>for<space>s3.txt
#[case::case_08_encoded_specialchars_folder("unittests/specialchars/folder%20%09%7E%21%40%23%24%25%5E%26%2A%28%29%5F%2B%601234567890%2D%3D%5B%5D%7B%7D%7C%3A%3B%22%27%3C%3E%2C%3F%5C/test%20with%20plus%20%2B%20for%20s3%2Etxt", 48)]
// Object name ending in a space. quick-xml (Azure) used to trim it during list
// deserialization; the %20 must survive a stat round-trip on every backend.
#[case::case_09_trailing_whitespace("unittests/specialchars/trailing_whitespace_probe%20", 53)]
#[test_log::test(tokio::test(flavor = "multi_thread", worker_threads = 4))]
async fn stat_full(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure, Backend::Minio, Backend::GCPS3Compat)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::Public, Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[values((Cache::List, 1), (Cache::List, 10), (Cache::List, 100), (Cache::Stat, 1), (Cache::Stat, 10), (Cache::Stat, 100), (Cache::Disabled, 1))]
    cache: (Cache, usize),
    #[case] base_address: &str,
    #[case] size: u64,
) {
    stat_impl(
        storage_fixture,
        backend,
        interface,
        access,
        api_version,
        cache,
        base_address,
        size,
    )
    .await;
}

// Shared implementation for both stat tests
async fn stat_impl(
    storage_fixture: &StorageFixture,
    backend: Backend,
    interface: Interface,
    access: Access,
    api_version: ApiVersion,
    cache: (Cache, usize),
    base_address: &str,
    size: u64,
) {
    // Minio doesn't support the full special character set that S3 does
    if backend == Backend::Minio && base_address == "unittests/specialchars/folder%20%09%7E%21%40%23%24%25%5E%26%2A%28%29%5F%2B%601234567890%2D%3D%5B%5D%7B%7D%7C%3A%3B%22%27%3C%3E%2C%3F%5C/test%20with%20plus%20%2B%20for%20s3%2Etxt" {
        return;
    }

    // azure sdk does not handle tab characters in the path correctly
    if backend == Backend::Azure && base_address.contains("%09") {
        return;
    }

    debug!(?base_address, %base_address);

    let Some(address) = storage_fixture.make_url(backend, access, base_address) else {
        // It would be nice to indicate that this test is skipped, but there's currently no way to do that
        // https://internals.rust-lang.org/t/pre-rfc-skippable-tests/14611
        debug!("Skipping test for {:?} {:?} {:?}", backend, access, base_address);
        return;
    };

    let mut client = storage_fixture
        .file_object_client(backend, cache.0, interface, api_version)
        .await;

    for _ in 0..cache.1 {
        let request = StatRequest {
            resource_address: address.clone(),
        };
        let stat_response = client.stat(request).await.unwrap();
        let resource_info = stat_response.resource_info.unwrap();
        assert_eq!(resource_info.metadata.as_ref().unwrap().data_object_size.unwrap(), size);

        assert_identity_has_object_identifier_parameters(
            resource_info.resource_identity.unwrap().encoded_identity.as_ref(),
            &address,
            access == Access::Versioned,
        );
    }

    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.requests",
        "stat",
        ["result" => "Ok"]
    );

    if cache.0 == Cache::List {
        assert_metric!(
            storage_fixture,
            "storage.cache.operations",
            ["access_type" => "miss", "name" => "list_cache"],
            "name=list_cache"
        );
        assert_metric!(
            storage_fixture,
            "storage.cache.operations",
            ["access_type" => "access", "name" => "list_cache"],
            "name=list_cache"
        );
    } else if cache.0 == Cache::Stat {
        assert_metric!(
            storage_fixture,
            "storage.cache.operations",
            ["access_type" => "miss", "name" => "stat_cache"],
            "name=stat_cache"
        );

        assert_metric!(
            storage_fixture,
            "storage.cache.operations",
            ["access_type" => "access", "name" => "stat_cache"],
            "name=stat_cache"
        );
    } else if cache.0 == Cache::Disabled {
        // Method metrics are only valid for disabled cache tests because
        // that ensures backend requests are made to the service
        assert_method_metric!(
            storage_fixture,
            backend,
            api_version,
            "storage.sdk.requests",
            "stat",
            ["result" => "Ok"]
        );

        assert_method_metric!(
            storage_fixture,
            backend,
            api_version,
            "storage.sdk.request.duration",
            "stat",
            [],
            1
        );
    }

    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.request.duration",
        "stat",
        [],
        1
    );
}

// Regression test for list_cache normalization of unknown query strings.
//
// list_cache strips query+fragment when computing the parent folder cache key
// (parent_url() in storage-core/src/list_cache/cache.rs), but the per-entry
// lookup key inside that parent's list map is built from the unmodified URL
// (list_cache/v1alpha/fileobject.rs and v1beta/fileobject.rs). Only
// `version-id` is special-cased to bypass list_cache.
//
// A stat against a real object with any other query string therefore returns
// NotFound through list_cache even though the object exists. The
// Cache::Disabled arm proves the URL itself is fine; the Cache::List arm
// exercises the bug.
#[rstest]
#[test_log::test(tokio::test)]
async fn stat_with_unknown_query_string_full(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3)] backend: Backend,
    #[values(Interface::Grpc)] interface: Interface,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[values(Cache::List, Cache::Stat, Cache::Disabled)] cache: Cache,
) {
    let Some(mut address) = storage_fixture.make_url(backend, Access::PrivateKey, "unittests/readFile/Lorem ipsum.txt")
    else {
        return;
    };
    address.push_str("?timestamp=1773055462");

    let mut client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;

    let request = StatRequest {
        resource_address: address.clone(),
    };
    let stat_response = client.stat(request).await.unwrap();
    let resource_info = stat_response.resource_info.unwrap();
    assert_eq!(resource_info.metadata.as_ref().unwrap().data_object_size.unwrap(), 3549);
}

// Small stat_not_found test - selected by the nextest small profile (limited test cases for local efficiency)
#[rstest]
#[case::case_01_missing_jpg("unittests/stat/thisdefinitelydoesnotexist.jpg")]
#[test_log::test(tokio::test)]
async fn stat_not_found_basic(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[values((Cache::List, 10), (Cache::Stat, 10), (Cache::Disabled, 1))] cache: (Cache, usize),
    #[case] address: &str,
) {
    stat_not_found_impl(storage_fixture, backend, interface, access, api_version, cache, address).await;
}

// Full stat_not_found test - selected by the nextest full profile (comprehensive test coverage)
#[rstest]
#[case::case_01_missing_jpg("unittests/stat/thisdefinitelydoesnotexist.jpg")]
#[test_log::test(tokio::test)]
async fn stat_not_found_full(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure, Backend::Minio, Backend::GCPS3Compat)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::Public, Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    // Test repeat requests with cache enabled
    #[values((Cache::List, 1), (Cache::List, 10), (Cache::Stat, 1), (Cache::Stat, 10), (Cache::Disabled, 1))] cache: (
        Cache,
        usize,
    ),
    #[case] address: &str,
) {
    stat_not_found_impl(storage_fixture, backend, interface, access, api_version, cache, address).await;
}

// Shared implementation for both stat_not_found tests
async fn stat_not_found_impl(
    storage_fixture: &StorageFixture,
    backend: Backend,
    interface: Interface,
    access: Access,
    api_version: ApiVersion,
    cache: (Cache, usize),
    address: &str,
) {
    debug!(?address, %address);

    let Some(address) = storage_fixture.make_url(backend, access, address) else {
        // It would be nice to indicate that this test is skipped, but there's currently no way to do that
        // https://internals.rust-lang.org/t/pre-rfc-skippable-tests/14611
        debug!("Skipping test for {:?} {:?} {:?}", backend, access, address);
        return;
    };

    let mut client = storage_fixture
        .file_object_client(backend, cache.0, interface, api_version)
        .await;

    for _ in 0..cache.1 {
        let request = StatRequest {
            resource_address: address.clone(),
        };
        let response = client.stat(request).await;
        assert!(response.is_err());
        assert_eq!(response.err().unwrap().code(), tonic::Code::NotFound);
    }

    if cache.0 == Cache::List {
        assert_metric!(
            storage_fixture,
            "storage.cache.operations",
            ["access_type" => "miss", "name" => "list_cache"],
            "name=list_cache"
        );
        assert_metric!(
            storage_fixture,
            "storage.cache.operations",
            ["access_type" => "access", "name" => "list_cache"],
            "name=list_cache"
        );
    } else if cache.0 == Cache::Stat {
        assert_metric!(
            storage_fixture,
            "storage.cache.operations",
            ["access_type" => "miss", "name" => "stat_cache"],
            "name=stat_cache"
        );
        assert_metric!(
            storage_fixture,
            "storage.cache.operations",
            ["access_type" => "access", "name" => "stat_cache"],
            "name=stat_cache"
        );
    } else if cache.0 == Cache::Disabled {
        assert_method_metric!(
            storage_fixture,
            backend,
            api_version,
            "storage.sdk.requests",
            "stat",
            ["result" => "NotFound"]
        );
    }
}

async fn stat_permission_denied_impl(
    storage_fixture: &StorageFixture,
    backend: Backend,
    interface: Interface,
    access: Access,
    api_version: ApiVersion,
    cache: (Cache, usize),
    address: &str,
) {
    debug!(?address, %address);

    let Some(address) = storage_fixture.make_url(backend, access, address) else {
        // It would be nice to indicate that this test is skipped, but there's currently no way to do that
        // https://internals.rust-lang.org/t/pre-rfc-skippable-tests/14611
        debug!("Skipping test for {:?} {:?} {:?}", backend, access, address);
        return;
    };

    let mut client = storage_fixture
        .file_object_client(backend, cache.0, interface, api_version)
        .await;

    for _ in 0..cache.1 {
        let request = StatRequest {
            resource_address: address.clone(),
        };
        let response = client.stat(request).await;
        assert!(response.is_err());
        assert_eq!(response.err().unwrap().code(), tonic::Code::PermissionDenied);
    }

    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.requests",
        "stat",
        ["result" => "PermissionDenied"]
    );

    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.sdk.requests",
        "stat",
        ["result" => "PermissionDenied"]
    );
}

// Small read test - selected by the nextest small profile (limited test cases for local efficiency)
#[rstest]
// Combine case information into a single tuple to reduce the number of arguments. Otherwise clippy complains,
// and trying to set #[allow(clippy::too_many_arguments)] on the function doesn't work with the rstest macros.
#[case::case_01_lorem_ipsum(
    ("unittests/readFile/Lorem ipsum.txt",
    3549,
    hex!("78f5b9115f4924f23bb7dbe646e67dd297479f7711bb06c55360377a9927cb7e"))
)]
#[test_log::test(tokio::test)]
async fn read_basic(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[values((Cache::List, 2), (Cache::Stat, 2), (Cache::Disabled, 1))] cache: (Cache, usize),
    #[case] info: (&str, u64, [u8; 32]),
) {
    read_impl(storage_fixture, backend, interface, access, api_version, cache, info).await;
}

// Full read test - selected by the nextest full profile (comprehensive test coverage)
#[rstest]
// Combine case information into a single tuple to reduce the number of arguments. Otherwise clippy complains,
// and trying to set #[allow(clippy::too_many_arguments)] on the function doesn't work with the rstest macros.
#[case::case_01_lorem_ipsum(
    ("unittests/readFile/Lorem ipsum.txt",
    3549,
    hex!("78f5b9115f4924f23bb7dbe646e67dd297479f7711bb06c55360377a9927cb7e"))
)]
#[case::case_02_splash_jpg(
    ("unittests/readFile/splash.jpg",
    241619,
    hex!("73849c680471383c228bda00ef9b390a4fec90b20fe3a27c5c0bfe24139cb8a3"))
)]
#[case::case_03_empty_file(
    ("unittests/stat/empty.txt",
    0,
    hex!("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"))
)]
#[test_log::test(tokio::test)]
async fn read_full(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure, Backend::Minio, Backend::GCPS3Compat)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::Public, Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    // Test repeat requests with cache enabled
    #[values((Cache::List, 1), (Cache::List, 2), (Cache::List, 5), (Cache::Stat, 1), (Cache::Stat, 2), (Cache::Stat, 5), (Cache::Disabled, 1))]
    cache: (Cache, usize),
    #[case] info: (&str, u64, [u8; 32]),
) {
    read_impl(storage_fixture, backend, interface, access, api_version, cache, info).await;
}

// Shared implementation for both read tests
async fn read_impl(
    storage_fixture: &StorageFixture,
    backend: Backend,
    interface: Interface,
    access: Access,
    api_version: ApiVersion,
    cache: (Cache, usize),
    info: (&str, u64, [u8; 32]),
) {
    let address = info.0;
    let size = info.1;
    let sha256 = info.2;
    debug!(?address, %address);

    let Some(address) = storage_fixture.make_url(backend, access, address) else {
        // It would be nice to indicate that this test is skipped, but there's currently no way to do that
        // https://internals.rust-lang.org/t/pre-rfc-skippable-tests/14611
        debug!("Skipping test for {:?} {:?} {:?}", backend, access, address);
        return;
    };

    let mut client = storage_fixture
        .file_object_client(backend, cache.0, interface, api_version)
        .await;
    for _ in 0..cache.1 {
        // read from address
        let request = ReadFromAddressRequest {
            resource_address: address.clone(),
            download_preference: Some(DownloadPreference::Body.into()),
        };

        let mut read_stream = client.read_from_address(request).await.unwrap();
        debug!(?read_stream);

        // metadata first, save the resource info so we can call read() afterward
        let resource_identity = match read_stream.next().await.unwrap() {
            Ok(read_response) => {
                debug!(?read_response);

                match read_response.reply_type {
                    Some(read_from_address_response::ReplyType::ResourceInfo(info)) => {
                        debug!(?info);
                        assert_eq!(info.metadata.as_ref().unwrap().data_object_size.unwrap(), size);
                        // Too hard to track all timestamps for all buckets. Just assert the timestamp is nonzero
                        assert!(
                            info.metadata
                                .as_ref()
                                .unwrap()
                                .last_modified_timestamp
                                .as_ref()
                                .unwrap()
                                .seconds
                                > 0
                        );

                        let resource_identity = info.resource_identity;
                        assert_identity_has_object_identifier_parameters(
                            resource_identity.as_ref().unwrap().encoded_identity.as_str(),
                            &address,
                            access == Access::Versioned,
                        );

                        resource_identity
                    },
                    _ => panic!("Read replied without a resource info as first message!"),
                }
            },
            Err(e) => panic!("Read reply error: {e:?}"),
        };

        // sha256 content
        let mut hasher = Sha256::new();

        while let Some(read_response) = read_stream.next().await {
            match read_response {
                Ok(read_response) => match read_response.reply_type {
                    Some(read_from_address_response::ReplyType::Chunk(chunk)) => {
                        trace!(?chunk);

                        hasher.update(chunk.chunk);
                    },
                    _ => panic!("Read chunk expected!"),
                },
                Err(e) => panic!("Read reply error: {e:?}"),
            }
        }

        let result = hasher.finalize();
        debug!(?result, ?sha256);
        assert_eq!(result[..], sha256);

        // Read with body
        let request = ReadRequest {
            resource_identity: resource_identity.clone(),
            download_preference: Some(DownloadPreference::Body.into()),
        };
        let mut read_stream = client.read(request).await.unwrap();
        debug!(?read_stream);

        // metadata first, save the resource info so we can call read() afterward
        match read_stream.next().await.unwrap() {
            Ok(read_response) => {
                debug!(?read_response);

                match read_response.reply_type {
                    Some(read_response::ReplyType::Metadata(info)) => {
                        debug!(?info);
                        assert_eq!(info.data_object_size.unwrap(), size);
                        // Too hard to track all timestamps for all buckets. Just assert the timestamp is nonzero
                        assert!(info.last_modified_timestamp.unwrap().seconds > 0);
                    },
                    _ => panic!("Read replied without a resource info as first message!"),
                }
            },
            Err(e) => panic!("Read reply error: {e:?}"),
        };

        // sha256 content
        let mut hasher = Sha256::new();

        while let Some(read_response) = read_stream.next().await {
            match read_response {
                Ok(read_response) => {
                    trace!(?read_response);
                    match read_response.reply_type {
                        Some(read_response::ReplyType::Chunk(chunk)) => {
                            trace!(?chunk);
                            hasher.update(chunk.chunk);
                        },
                        _ => panic!("Read chunk expected!"),
                    }
                },
                Err(e) => panic!("Read reply error: {e:?}"),
            }
        }

        let result = hasher.finalize();
        debug!(?result, ?sha256);
        assert_eq!(result[..], sha256);

        // Read from address with redirect download
        let request = ReadFromAddressRequest {
            resource_address: address.clone(),
            download_preference: Some(DownloadPreference::Redirect.into()),
        };
        let mut read_stream = client.read_from_address(request).await.unwrap();
        debug!(?read_stream);

        // metadata first, save the resource info so we can call read() afterward
        match read_stream.next().await.unwrap() {
            Ok(read_response) => {
                debug!(?read_response);

                match read_response.reply_type {
                    Some(read_from_address_response::ReplyType::ResourceInfo(info)) => {
                        debug!(?info);
                        assert_eq!(info.metadata.as_ref().unwrap().data_object_size.unwrap(), size);
                        // Too hard to track all timestamps for all buckets. Just assert the timestamp is nonzero
                        assert!(
                            info.metadata
                                .as_ref()
                                .unwrap()
                                .last_modified_timestamp
                                .as_ref()
                                .unwrap()
                                .seconds
                                > 0
                        );

                        assert_identity_has_object_identifier_parameters(
                            info.resource_identity.unwrap().encoded_identity.as_ref(),
                            &address,
                            access == Access::Versioned,
                        );
                    },
                    _ => panic!("Read replied without a resource info as first message!"),
                }
            },
            Err(e) => panic!("Read reply error: {e:?}"),
        }

        // sha256 content
        let mut hasher = Sha256::new();

        // this should be redirect to HTTP download
        match read_stream.next().await.unwrap() {
            Ok(read_response) => {
                debug!(?read_response);

                match read_response.reply_type {
                    Some(read_from_address_response::ReplyType::Redirect(redirect)) => {
                        // Validate that our supplied headers are returned
                        HeadersConfig::new(HashMap::new(), HashMap::new())
                            .validate_headers(&redirect.additional_headers);
                        // add in any additional headers required
                        let mut headers = HeaderMap::new();
                        for header in redirect.additional_headers {
                            headers.insert(
                                HeaderName::from_bytes(header.name.as_bytes()).unwrap(),
                                header.value.parse().unwrap(),
                            );
                        }

                        // http get the file
                        let response = reqwest::Client::new()
                            .get(&redirect.redirect_target_url)
                            .headers(headers)
                            .send()
                            .await
                            .unwrap();
                        let status = response.status();
                        let body = response.bytes().await.unwrap();
                        if !status.is_success() {
                            let body_str = String::from_utf8_lossy(&body);
                            panic!(
                                "Presigned URL download failed: status={status}, url={}, body={body_str}",
                                redirect.redirect_target_url
                            );
                        }
                        trace!(?body);
                        hasher.update(body);
                    },
                    _ => panic!("Expected redirect response but got {:?}", read_response.reply_type),
                }
            },
            Err(_) => panic!("Expected redirect response but got empty reply!"),
        }

        let result = hasher.finalize();
        debug!(?result, ?sha256);
        assert_eq!(result[..], sha256);

        // Read with redirect download
        let request = ReadRequest {
            resource_identity,
            download_preference: Some(DownloadPreference::Redirect.into()),
        };
        let mut read_stream = client.read(request).await.unwrap();
        debug!(?read_stream);

        // metadata first, save the resource info so we can call read() afterward
        match read_stream.next().await.unwrap() {
            Ok(read_response) => {
                debug!(?read_response);

                match read_response.reply_type {
                    Some(read_response::ReplyType::Metadata(md)) => {
                        debug!(?md);
                        assert_eq!(md.data_object_size.unwrap(), size);
                        // Too hard to track all timestamps for all buckets. Just assert the timestamp is nonzero
                        assert!(md.last_modified_timestamp.unwrap().seconds > 0);
                    },
                    _ => panic!("Read replied without a resource info as first message!"),
                }
            },
            Err(e) => panic!("Read reply error: {e:?}"),
        }

        // sha256 content
        let mut hasher = Sha256::new();

        // this should be redirect to HTTP download
        match read_stream.next().await.unwrap() {
            Ok(read_response) => {
                debug!(?read_response);

                match read_response.reply_type {
                    Some(read_response::ReplyType::Redirect(redirect)) => {
                        // Validate that our supplied headers are returned
                        HeadersConfig::new(HashMap::new(), HashMap::new())
                            .validate_headers(&redirect.additional_headers);
                        // add in any additional headers required
                        let mut headers = HeaderMap::new();
                        for header in redirect.additional_headers {
                            headers.insert(
                                HeaderName::from_bytes(header.name.as_bytes()).unwrap(),
                                header.value.parse().unwrap(),
                            );
                        }

                        // http get the file
                        let response = reqwest::Client::new()
                            .get(&redirect.redirect_target_url)
                            .headers(headers)
                            .send()
                            .await
                            .unwrap();
                        let status = response.status();
                        let body = response.bytes().await.unwrap();
                        if !status.is_success() {
                            let body_str = String::from_utf8_lossy(&body);
                            panic!(
                                "Presigned URL download failed: status={status}, url={}, body={body_str}",
                                redirect.redirect_target_url
                            );
                        }
                        hasher.update(body);
                    },
                    _ => panic!("Expected redirect response but got {:?}", read_response.reply_type),
                }
            },
            Err(_) => panic!("Expected redirect response but got empty reply!"),
        }

        let result = hasher.finalize();
        debug!(?result, ?sha256);
        assert_eq!(result[..], sha256);
    }

    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.requests",
        "read",
        ["result" => "Ok"]
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.requests",
        "read_from_address",
        ["result" => "Ok"]
    );

    if cache.0 == Cache::List {
        assert_metric!(
            storage_fixture,
            "storage.cache.operations",
            ["access_type" => "miss", "name" => "list_cache"],
            "name=list_cache"
        );
        assert_metric!(
            storage_fixture,
            "storage.cache.operations",
            ["access_type" => "access", "name" => "list_cache"],
            "name=list_cache"
        );

        assert_method_metric!(
            storage_fixture,
            backend,
            api_version,
            "storage.sdk.request.duration",
            "list_stat",
            [],
            1
        );
    } else if cache.0 == Cache::Stat {
        assert_metric!(
            storage_fixture,
            "storage.cache.operations",
            ["access_type" => "miss", "name" => "stat_cache"],
            "name=stat_cache"
        );
        assert_metric!(
            storage_fixture,
            "storage.cache.operations",
            ["access_type" => "access", "name" => "stat_cache"],
            "name=stat_cache"
        );

        assert_method_metric!(
            storage_fixture,
            backend,
            api_version,
            "storage.sdk.request.duration",
            "stat",
            [],
            1
        );
    }

    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.request.duration",
        "read",
        [],
        1
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.request.duration",
        "read_from_address",
        [],
        1
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.sdk.requests",
        "read_from_address",
        ["result" => "Ok"]
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.sdk.request.duration",
        "read_from_address",
        [],
        1
    );

    // Depending on timing, if cache is enabled, the read call could use the cache,
    // or go to the backend, so we can only be sure this metric is set if cache is disabled
    if cache.0 == Cache::Disabled {
        assert_method_metric!(
            storage_fixture,
            backend,
            api_version,
            "storage.sdk.requests",
            "read",
            ["result" => "Ok"]
        );
        assert_method_metric!(
            storage_fixture,
            backend,
            api_version,
            "storage.sdk.request.duration",
            "read",
            [],
            1
        );
    }

    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.read.object.size",
        "read",
        [],
        1
    );

    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.read.object.size",
        "read_from_address",
        [],
        1
    );
}
#[rstest]
#[case::case_01_lorem_ipsum(
    ("unittests/readFile/Lorem ipsum.txt",
    3549,
    hex!("78f5b9115f4924f23bb7dbe646e67dd297479f7711bb06c55360377a9927cb7e"))
)]
#[case::case_02_splash_jpg(
    ("unittests/readFile/splash.jpg",
    241619,
    hex!("73849c680471383c228bda00ef9b390a4fec90b20fe3a27c5c0bfe24139cb8a3"))
)]
#[case::case_03_testfile_300kib(
    ("unittests/readFile/testfile_300kib.txt",
    300 * 1024,
    hex!("6f2b30bdc49ca58a58c755c93f4a348d99a0a9188cbf8a0d98b788433517b411"))
)]
// use CDN fixture but force body download and verify the data is returned
#[test_log::test(tokio::test)]
async fn read_cdn_body(
    #[from(cdn_storage_fixture)] storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3)] backend: Backend,
    #[values(Interface::Grpc)] interface: Interface,
    #[values(Access::Public, Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[values((Cache::List, 10), (Cache::Stat, 10), (Cache::Disabled, 1))] cache: (Cache, usize),
    #[case] info: (&str, u64, [u8; 32]),
) {
    debug!(?info);
    read_impl(storage_fixture, backend, interface, access, api_version, cache, info).await;
}
#[rstest]
#[case::case_01_testfile_300kib(
    ("unittests/readFile/testfile_300kib.txt",
    300 * 1024,
    hex!("6f2b30bdc49ca58a58c755c93f4a348d99a0a9188cbf8a0d98b788433517b411"))
)]
// use CDN fixture, verify redirect url is returned
#[test_log::test(tokio::test)]
async fn read_cdn_redirect(
    #[from(cdn_storage_fixture)] storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3)] backend: Backend,
    #[values(Interface::Grpc)] interface: Interface,
    #[values(Access::Public, Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[case] info: (&str, u64, [u8; 32]),
) {
    let address = info.0;
    let size = info.1;
    let sha256 = info.2;
    debug!(?address, %address);

    let Some(address) = storage_fixture.make_url(backend, access, address) else {
        // It would be nice to indicate that this test is skipped, but there's currently no way to do that
        // https://internals.rust-lang.org/t/pre-rfc-skippable-tests/14611
        debug!("Skipping test for {:?} {:?} {:?}", backend, access, address);
        return;
    };

    let mut client = storage_fixture
        .file_object_client(backend, Cache::Disabled, interface, api_version)
        .await;

    let request = ReadFromAddressRequest {
        resource_address: address.clone(),
        download_preference: None,
    };

    let mut read_stream = client.read_from_address(request).await.unwrap();
    debug!(?read_stream);

    // resource info expected first
    match read_stream.next().await.unwrap() {
        Ok(read_response) => {
            debug!(?read_response);

            match read_response.reply_type {
                Some(read_from_address_response::ReplyType::ResourceInfo(info)) => {
                    debug!(?info);
                    assert_eq!(info.metadata.as_ref().unwrap().data_object_size.unwrap(), size);
                    // Too hard to track all timestamps for all buckets. Just assert the timestamp is nonzero
                    assert!(
                        info.metadata
                            .as_ref()
                            .unwrap()
                            .last_modified_timestamp
                            .as_ref()
                            .unwrap()
                            .seconds
                            > 0
                    );

                    let resource_identity = info.resource_identity;
                    assert_identity_has_object_identifier_parameters(
                        resource_identity.as_ref().unwrap().encoded_identity.as_str(),
                        &address,
                        access == Access::Versioned,
                    );
                },
                _ => panic!("Read replied without a resource info as first message!"),
            }
        },
        Err(e) => panic!("Read reply error: {e:?}"),
    }

    // redirect expected
    match read_stream.next().await.unwrap() {
        Ok(read_response) => {
            debug!(?read_response);

            match read_response.reply_type {
                Some(read_from_address_response::ReplyType::Redirect(redirect)) => {
                    debug!(?redirect);
                    let redirect_url = Url::parse(&redirect.redirect_target_url).unwrap();
                    match backend {
                        Backend::Azure => {
                            assert!(redirect_url.host_str().unwrap().ends_with("azurefd.net"));
                        },
                        Backend::S3 => {
                            assert!(redirect_url.host_str().unwrap().ends_with("cloudfront.net"));
                        },
                        _ => panic!("Unexpected backend: {backend:?}"),
                    }

                    // http get the file
                    let body = reqwest::Client::new()
                        .get(redirect.redirect_target_url)
                        .send()
                        .await
                        .unwrap()
                        .bytes()
                        .await
                        .unwrap();
                    let mut hasher = Sha256::new();
                    hasher.update(body);
                    let result = hasher.finalize();
                    debug!(?result, ?sha256);
                    assert_eq!(result[..], sha256);
                },
                _ => panic!("Read replied without redirect as second message!"),
            }
        },
        Err(e) => panic!("Read reply error: {e:?}"),
    }
}

#[rstest]
#[test_log::test(tokio::test)]
async fn cdn_redirect_read_after_redirect_overwrite_uses_versioned_cache_key(
    #[from(cdn_storage_fixture)] storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3)] backend: Backend,
    #[values(ApiVersion::V1Alpha)] api_version: ApiVersion,
) {
    let mut client = storage_fixture
        .file_object_client(backend, Cache::Disabled, Interface::Grpc, api_version)
        .await;
    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, Access::Versioned, Cache::Disabled, api_version)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, Access::Versioned);
        return;
    };

    let file = temp_dir.new_file();
    let zeros = vec![b'0'; (20 * MIB) as usize];
    let ones = vec![b'1'; (20 * MIB) as usize];

    cdn_write_data_redirect_for_cdn_cache_repro(&file, &zeros, &mut client).await;
    let (zeros_redirect_url, zeros_body) =
        cdn_download_via_cdn_redirect(backend, &mut client, &file, zeros.len()).await;
    assert_eq!(&zeros_body[..20], &zeros[..20]);

    cdn_write_data_redirect_for_cdn_cache_repro(&file, &ones, &mut client).await;
    let (ones_redirect_url, ones_body) = cdn_download_via_cdn_redirect(backend, &mut client, &file, ones.len()).await;
    assert_eq!(&ones_body[..20], &ones[..20]);

    let zeros_version_id = cdn_object_version_id(&zeros_redirect_url)
        .expect("CDN redirect URL after first write should include an object version id");
    let ones_version_id = cdn_object_version_id(&ones_redirect_url)
        .expect("CDN redirect URL after overwrite should include an object version id");
    assert_ne!(
        zeros_version_id, ones_version_id,
        "CDN redirect URLs must address distinct object versions so cached objects cannot be reused across writes"
    );

    temp_dir.cleanup().await;
}

async fn cdn_write_data_redirect_for_cdn_cache_repro(
    file: &str,
    data: &[u8],
    client: &mut Box<dyn storage_core_test::clients::traits::fileobject::FileObject>,
) -> ResourceIdentity {
    let redirect = client
        .write(
            WriteParameters {
                native_metadata: Default::default(),
                destination_resource_address: file.to_owned(),
                previous_version: None,
                fail_if_exists: None,
                data_object_size: data.len() as u64,
                upload_preference: Some(UploadPreference::Redirect.into()),
            },
            None,
        )
        .await
        .expect("redirect write allocation should succeed");

    let write_response::WriteResponseType::WriteRedirect(redirect) = redirect
        .write_response_type
        .expect("write response should include redirect")
    else {
        panic!("write response should be a redirect");
    };

    let completion_headers = cdn_perform_upload_without_cache_control(
        data,
        redirect.method,
        &redirect.redirect_target_url,
        redirect.additional_headers,
        &redirect.completion_header_names,
    )
    .await;

    client
        .complete_redirect_upload(CompleteRedirectUploadRequest {
            destination_resource_address: file.to_owned(),
            additional_headers: completion_headers,
        })
        .await
        .expect("redirect upload completion should succeed")
        .resource_info
        .expect("completion should return resource info")
        .resource_identity
        .expect("completion should return resource identity")
}

async fn cdn_perform_upload_without_cache_control(
    data: &[u8],
    method: i32,
    url: &str,
    additional_headers: Vec<Header>,
    completion_header_names: &[String],
) -> Vec<Header> {
    let http_client = reqwest::Client::new();
    let mut builder = match method {
        method if method == UploadMethod::Post as i32 => http_client.post(url),
        method if method == UploadMethod::Put as i32 => http_client.put(url),
        _ => panic!("upload method must be specified"),
    };

    for header in additional_headers {
        if header.name.eq_ignore_ascii_case("cache-control") {
            continue;
        }
        builder = builder.header(header.name, header.value);
    }

    let http_response = builder.body(data.to_vec()).send().await.unwrap();
    let status = http_response.status();
    if status != reqwest::StatusCode::CREATED && status != reqwest::StatusCode::OK {
        let body = http_response.text().await.unwrap_or_default();
        panic!("redirect target upload failed: status={status}, body={body}");
    }

    let mut headers = Vec::new();
    for header in completion_header_names {
        if let Some(header_value) = http_response.headers().get(header) {
            headers.push(Header {
                name: header.clone(),
                value: header_value.to_str().unwrap().into(),
            });
        }
    }
    headers
}

async fn cdn_download_via_cdn_redirect(
    backend: Backend,
    client: &mut Box<dyn storage_core_test::clients::traits::fileobject::FileObject>,
    resource_address: &str,
    expected_len: usize,
) -> (Url, Vec<u8>) {
    let mut read_stream = client
        .read_from_address(ReadFromAddressRequest {
            resource_address: resource_address.to_owned(),
            download_preference: Some(DownloadPreference::Redirect.into()),
        })
        .await
        .expect("read_from_address should return a stream");

    match read_stream.next().await {
        Some(Ok(read_response)) => match read_response.reply_type {
            Some(read_from_address_response::ReplyType::ResourceInfo(info)) => {
                let metadata = info.metadata.expect("resource info should include metadata");
                assert_eq!(metadata.data_object_size(), expected_len as u64);
                assert_identity_has_object_identifier_parameters(
                    info.resource_identity
                        .as_ref()
                        .expect("resource info should include resource identity")
                        .encoded_identity
                        .as_ref(),
                    resource_address,
                    false,
                );
            },
            other => panic!("read_from_address should return resource info first, got {other:?}"),
        },
        Some(Err(status)) => panic!("read_from_address returned an unexpected error: {status:?}"),
        None => panic!("read_from_address returned an empty stream"),
    }

    let redirect = match read_stream.next().await {
        Some(Ok(read_response)) => match read_response.reply_type {
            Some(read_from_address_response::ReplyType::Redirect(redirect)) => redirect,
            other => panic!("read_from_address should return redirect second, got {other:?}"),
        },
        Some(Err(status)) => panic!("read_from_address returned an unexpected error: {status:?}"),
        None => panic!("read_from_address did not return a redirect"),
    };

    let redirect_url = Url::parse(&redirect.redirect_target_url).expect("redirect URL should parse");
    let expected_host_suffix = match backend {
        Backend::Azure => "azurefd.net",
        Backend::S3 => "cloudfront.net",
        _ => panic!("unexpected CDN backend: {backend:?}"),
    };
    assert!(
        redirect_url
            .host_str()
            .unwrap_or_default()
            .ends_with(expected_host_suffix),
        "redirect URL should target the configured CDN for {backend:?}: {redirect_url}"
    );

    let mut headers = HeaderMap::new();
    for header in redirect.additional_headers {
        if header.name.eq_ignore_ascii_case("cache-control") {
            continue;
        }
        headers.insert(
            HeaderName::from_bytes(header.name.as_bytes()).unwrap(),
            header.value.parse().unwrap(),
        );
    }

    let response = reqwest::Client::new()
        .get(redirect_url.clone())
        .headers(headers)
        .send()
        .await
        .expect("following CDN redirect URL failed");
    let status = response.status();
    let body = response.bytes().await.expect("CDN response body should read");
    assert!(
        status.is_success(),
        "CDN redirect download failed: status={status}, path={}, body={}",
        redirect_url.path(),
        String::from_utf8_lossy(&body)
    );
    assert_eq!(body.len(), expected_len);

    (redirect_url, body.to_vec())
}

fn cdn_object_version_id(url: &Url) -> Option<String> {
    url.query_pairs()
        .find(|(name, _)| name.eq_ignore_ascii_case("versionid") || name.eq_ignore_ascii_case("version-id"))
        .map(|(_, value)| value.into_owned())
}

// Small read_invalid_version test - selected by the nextest small profile (limited test cases for local efficiency)
#[rstest]
#[case::case_01_lorem_ipsum("unittests/readFile/Lorem ipsum.txt")]
#[test_log::test(tokio::test)]
async fn read_invalid_version_basic(
    storage_fixture: &StorageFixture,
    // Minio and GCPS3Compat don't error on the bogus version ID, so skip this test
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc)] interface: Interface,
    #[values(Access::Versioned)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[case] address: &str,
) {
    read_invalid_version_impl(
        storage_fixture,
        backend,
        interface,
        access,
        api_version,
        Cache::Disabled,
        address,
    )
    .await;
}

// Full read_invalid_version test - selected by the nextest full profile (comprehensive test coverage)
#[rstest]
#[case::case_01_lorem_ipsum("unittests/readFile/Lorem ipsum.txt")]
#[test_log::test(tokio::test)]
async fn read_invalid_version_full(
    storage_fixture: &StorageFixture,
    // Minio and GCPS3Compat don't error on the bogus version ID, so skip this test
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::Versioned)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[values(Cache::Disabled, Cache::Stat, Cache::List)] cache: Cache,
    #[case] address: &str,
) {
    read_invalid_version_impl(storage_fixture, backend, interface, access, api_version, cache, address).await;
}

// Shared implementation for both read_invalid_version tests
async fn read_invalid_version_impl(
    storage_fixture: &StorageFixture,
    backend: Backend,
    interface: Interface,
    access: Access,
    api_version: ApiVersion,
    cache: Cache,
    address: &str,
) {
    use url::Url;

    let Some(address) = storage_fixture.make_url(backend, access, address) else {
        // It would be nice to indicate that this test is skipped, but there's currently no way to do that
        // https://internals.rust-lang.org/t/pre-rfc-skippable-tests/14611
        debug!("Skipping test for {:?} {:?} {:?}", backend, access, address);
        return;
    };

    let mut client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;
    let resource_id = client
        .stat(StatRequest {
            resource_address: address.clone(),
        })
        .await
        .unwrap()
        .resource_info
        .unwrap()
        .resource_identity
        .unwrap();
    debug!(?resource_id);

    // Modify the returned version ID
    let url = Url::parse(&resource_id.encoded_identity).unwrap();
    let new_params: Vec<(String, String)> = url
        .query_pairs()
        .into_iter()
        .map(|(k, v)| match k {
            std::borrow::Cow::Borrowed("version-id") => (k.to_string(), "thisshouldbeabogusversion".to_string()),
            _ => (k.to_string(), v.to_string()),
        })
        .collect();

    let mut modified_url = url.clone();
    modified_url.query_pairs_mut().clear();
    for (k, v) in new_params {
        modified_url.query_pairs_mut().append_pair(&k, &v);
    }
    modified_url
        .query_pairs_mut()
        .append_pair("version-id", "thisisabogusversionid");
    debug!(?modified_url);

    // Read with body
    let request = ReadRequest {
        resource_identity: Some(ResourceIdentity {
            encoded_identity: modified_url.to_string(),
        }),
        download_preference: Some(DownloadPreference::Body.into()),
    };

    match client.read(request).await {
        Err(status) => {
            assert_eq!(status.code(), tonic::Code::InvalidArgument);
        },
        Ok(mut stream) => {
            let response = stream
                .next()
                .await
                .expect("read stream ended before returning invalid-version error")
                .map_err(|status| status.code());
            assert_matches!(response, Err(tonic::Code::InvalidArgument));
        },
    }
}

// The read path no longer validates content-length and last-modified. Etag-if-match is now the sole staleness guard on reads,
// so we need an explicit test that a tampered etag on the identity URL is
// rejected by the backend. The analogous pre-change test
// `read_from_corrupted_resource_identity` was retired because its content-length
// corruption premise no longer applies; this reinstates coverage of the
// staleness guard that still exists.
// Small read_corrupted_etag test - selected by the nextest small profile (limited test cases for local efficiency)
#[rstest]
#[case::case_01_lorem_ipsum("unittests/readFile/Lorem ipsum.txt")]
#[test_log::test(tokio::test)]
async fn read_corrupted_etag_basic(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[case] address: &str,
) {
    read_corrupted_etag_impl(storage_fixture, backend, interface, access, api_version, address).await;
}

// Full read_corrupted_etag test - selected by the nextest medium-and-larger profiles (broader edge-case coverage)
#[rstest]
#[case::case_01_lorem_ipsum("unittests/readFile/Lorem ipsum.txt")]
#[test_log::test(tokio::test)]
async fn read_corrupted_etag_full(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[case] address: &str,
) {
    read_corrupted_etag_impl(storage_fixture, backend, interface, access, api_version, address).await;
}

async fn read_corrupted_etag_impl(
    storage_fixture: &StorageFixture,
    backend: Backend,
    interface: Interface,
    access: Access,
    api_version: ApiVersion,
    address: &str,
) {
    use url::Url;

    let Some(address) = storage_fixture.make_url(backend, access, address) else {
        debug!("Skipping test for {:?} {:?} {:?}", backend, access, address);
        return;
    };

    let mut client = storage_fixture
        .file_object_client(backend, Cache::Disabled, interface, api_version)
        .await;

    // Stat to get a valid identity with the real etag, then tamper with the
    // etag query param. A well-formed identity that doesn't match the object's
    // current etag must be rejected by the backend via the GetObject if-match
    // precondition — NOT silently served as if etag validation were advisory.
    let resource_id = client
        .stat(StatRequest {
            resource_address: address.clone(),
        })
        .await
        .unwrap()
        .resource_info
        .unwrap()
        .resource_identity
        .unwrap();
    debug!(?resource_id);

    let url = Url::parse(&resource_id.encoded_identity).unwrap();
    let mut modified_url = url.clone();
    modified_url.query_pairs_mut().clear();
    for (k, v) in url.query_pairs() {
        if k == "etag" {
            // A syntactically valid but wrong etag. Quoted per RFC 7232.
            modified_url
                .query_pairs_mut()
                .append_pair("etag", "\"00000000000000000000000000000000\"");
        } else {
            modified_url.query_pairs_mut().append_pair(&k, &v);
        }
    }
    debug!(?modified_url);

    let request = ReadRequest {
        resource_identity: Some(ResourceIdentity {
            encoded_identity: modified_url.to_string(),
        }),
        download_preference: Some(DownloadPreference::Body.into()),
    };
    let response = client.read(request).await.map_err(|s| s.code());
    assert_matches!(
        response,
        Err(tonic::Code::FailedPrecondition) | Err(tonic::Code::InvalidArgument),
        "expected body read to reject tampered etag for backend={:?}",
        backend,
    );

    // Redirect reads return a signed URL/SAS that carries the if-match guard.
    // The service call itself succeeds; the backend rejects the tampered etag
    // when that URL is followed.
    let request = ReadRequest {
        resource_identity: Some(ResourceIdentity {
            encoded_identity: modified_url.to_string(),
        }),
        download_preference: Some(DownloadPreference::Redirect.into()),
    };
    let mut read_stream = client
        .read(request)
        .await
        .expect("redirect read should return a stream");

    match read_stream.next().await {
        Some(Ok(read_response)) => match read_response.reply_type {
            Some(read_response::ReplyType::Metadata(_)) => {},
            other => panic!("redirect read should return metadata first, got {other:?}"),
        },
        Some(Err(status)) => panic!("redirect read returned unexpected error: {status:?}"),
        None => panic!("redirect read returned an empty stream"),
    }

    let redirect = match read_stream.next().await {
        Some(Ok(read_response)) => match read_response.reply_type {
            Some(read_response::ReplyType::Redirect(redirect)) => redirect,
            other => panic!("redirect read should return redirect second, got {other:?}"),
        },
        Some(Err(status)) => panic!("redirect read returned unexpected error: {status:?}"),
        None => panic!("redirect read did not return a redirect"),
    };

    let mut headers = HeaderMap::new();
    for header in redirect.additional_headers {
        headers.insert(
            HeaderName::from_bytes(header.name.as_bytes()).unwrap(),
            header.value.parse().unwrap(),
        );
    }

    let response = reqwest::Client::new()
        .get(&redirect.redirect_target_url)
        .headers(headers)
        .send()
        .await
        .expect("following redirect URL failed");
    assert_eq!(
        response.status(),
        reqwest::StatusCode::PRECONDITION_FAILED,
        "expected backend to reject redirect URL with tampered etag for backend={backend:?}"
    );
}

// Small enumerate test - selected by the nextest small profile (limited test cases for local efficiency)
#[rstest]
#[case::case_01_list_root("unittests/list")]
#[test_log::test(tokio::test)]
async fn enumerate_basic(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[case] base_address: &str,
) {
    enumerate_impl(storage_fixture, backend, interface, access, api_version, base_address).await;
}

// Full enumerate test - selected by the nextest full profile (comprehensive test coverage)
#[rstest]
#[case::case_01_list_root("unittests/list")]
#[test_log::test(tokio::test)]
async fn enumerate_full(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure, Backend::Minio, Backend::GCPS3Compat)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::Public, Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[case] base_address: &str,
) {
    enumerate_impl(storage_fixture, backend, interface, access, api_version, base_address).await;
}

// Shared implementation for both enumerate tests
async fn enumerate_impl(
    storage_fixture: &StorageFixture,
    backend: Backend,
    interface: Interface,
    access: Access,
    api_version: ApiVersion,
    base_address: &str,
) {
    let Some(base_address) = storage_fixture.make_url(backend, access, base_address) else {
        // It would be nice to indicate that this test is skipped, but there's currently no way to do that
        // https://internals.rust-lang.org/t/pre-rfc-skippable-tests/14611
        debug!("Skipping test for {:?} {:?} {:?}", backend, access, base_address);
        return;
    };

    let mut client = storage_fixture
        .file_object_client(backend, Cache::Disabled, interface, api_version)
        .await;
    // key that does not exist
    let request = EnumerateRequest {
        resource_address: format!("{base_address}/does-not-exist/"),
    };
    let mut enum_stream = client.enumerate(request).await.unwrap();
    trace!(?enum_stream);

    match enum_stream.next().await.unwrap() {
        Ok(response) => panic!("Expected an error, but the method returned a successful response: {response:?}"),
        Err(status) => {
            assert_eq!(status.code(), Code::NotFound, "Expected NOT_FOUND error");
        },
    }

    // key that has no children
    let empty_resource_address = format!("{base_address}/empty/");
    let request = EnumerateRequest {
        resource_address: empty_resource_address.clone(),
    };
    let mut enum_stream = client.enumerate(request).await.unwrap();
    trace!(?enum_stream);

    match enum_stream.next().await.unwrap() {
        Ok(response) => panic!("Expected an error, but the method returned a successful response: {response:?}"),
        Err(status) => {
            assert_eq!(status.code(), Code::NotFound, "Expected NOT_FOUND error");
        },
    }

    // key with spaces and 6 children
    let resource_address = format!("{base_address}/folder%20with spaces2/");
    let request = EnumerateRequest {
        resource_address: resource_address.clone(),
    };
    let enum_stream = client.enumerate(request).await.unwrap();
    trace!(?enum_stream);

    let enum_results = enum_stream
        .filter_map(|x| async move { x.ok() })
        .collect::<Vec<_>>()
        .await;
    trace!(?enum_results);
    assert_eq!(enum_results.len(), 1);
    assert_eq!(enum_results[0].items.len(), 6);

    // large folder
    let resource_address = format!("{base_address}/large/");
    let request = EnumerateRequest {
        resource_address: resource_address.clone(),
    };
    let enum_stream = client.enumerate(request).await.unwrap();
    trace!(?enum_stream);

    let enum_results = enum_stream
        .filter_map(|x| async move { x.ok() })
        .collect::<Vec<_>>()
        .await;
    trace!(?enum_results);
    debug!(len = ?enum_results.len());

    // The Rest client handles pagination, so we don't know how many chunks there are
    if interface == Interface::Grpc {
        // S3 is 1000 results per chunk and Azure is 5000 results per chunk
        let chunk_count = match backend {
            Backend::S3 | Backend::Minio | Backend::GCPS3Compat => 6,
            Backend::Azure => 2,
        };
        assert_eq!(enum_results.len(), chunk_count);
    }

    let items: usize = enum_results.iter().map(|x| x.items.len()).sum();
    assert_eq!(items, 5001);

    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.requests",
        "enumerate",
        ["result" => "Ok"]
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.enumeration.items",
        "enumerate",
        [],
        1
    );
    storage_fixture.metrics_exporter.print_metrics();
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.request.duration",
        "enumerate",
        [],
        1
    );
}
#[rstest]
#[case::case_01_lorem_ipsum(
    ("unittests/readFile/Lorem ipsum.txt",
    3549,
    hex!("78f5b9115f4924f23bb7dbe646e67dd297479f7711bb06c55360377a9927cb7e"))
)]
#[test_log::test(tokio::test)]
async fn read_through_small_object_cache(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::Public)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[case] (address, size, hash): (&str, u64, [u8; 32]),
) {
    let mut client = storage_fixture
        .file_object_client(backend, Cache::List, interface, api_version)
        .await;

    let Some(address) = storage_fixture.make_url(backend, access, address) else {
        // It would be nice to indicate that this test is skipped, but there's currently no way to do that
        // https://internals.rust-lang.org/t/pre-rfc-skippable-tests/14611
        debug!("Skipping test for {:?} {:?} {:?}", backend, access, address);
        return;
    };

    let resource_id = client
        .stat(StatRequest {
            resource_address: address.clone(),
        })
        .await
        .unwrap()
        .resource_info
        .unwrap()
        .resource_identity
        .unwrap();

    let (resource_info_1, content_1) = read_direct(
        client.as_mut(),
        ReadRequest {
            resource_identity: Some(resource_id.clone()),
            download_preference: Some(DownloadPreference::Body.into()),
        },
    )
    .await
    .expect("read failed");

    assert_eq!(resource_info_1.data_object_size, Some(size));
    assert_eq!(content_1.len() as u64, size);
    assert_eq!(Sha256::digest(&content_1).as_ref(), hash);

    let (resource_info_2, content_2) = read_direct(
        client.as_mut(),
        ReadRequest {
            resource_identity: Some(resource_id),
            download_preference: Some(DownloadPreference::Body.into()),
        },
    )
    .await
    .expect("read failed");

    assert_eq!(resource_info_2.data_object_size, Some(size));
    assert_eq!(content_2.len() as u64, size);
    assert_eq!(Sha256::digest(&content_2).as_ref(), hash);

    // small_object_cache intercepts stat() and pre-populates a
    // metadata-only entry keyed by identity, so the first read-by-identity is a
    // cache hit (no miss). We only assert the cache was exercised.
    assert_metric!(
        storage_fixture,
        "storage.cache.operations",
        ["access_type" => "access", "name" => "small_object_cache"],
        "name=small_object_cache"
    );
}
#[rstest]
#[case::case_01_splash_jpg(
    "unittests/readFile/splash.jpg",
    241619,
    hex!("73849c680471383c228bda00ef9b390a4fec90b20fe3a27c5c0bfe24139cb8a3")
)]
#[test_log::test(tokio::test)]
async fn read_through_small_object_cache_ignores_big_objects(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::Public)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[case] address: &str,
    #[case] expected_size: u64,
    #[case] expected_hash: [u8; 32],
) {
    let mut client = storage_fixture
        .file_object_client(backend, Cache::List, interface, api_version)
        .await;

    let Some(address) = storage_fixture.make_url(backend, access, address) else {
        // It would be nice to indicate that this test is skipped, but there's currently no way to do that
        // https://internals.rust-lang.org/t/pre-rfc-skippable-tests/14611
        debug!("Skipping test for {:?} {:?} {:?}", backend, access, address);
        return;
    };

    let resource_id = client
        .stat(StatRequest {
            resource_address: address.clone(),
        })
        .await
        .unwrap()
        .resource_info
        .unwrap()
        .resource_identity
        .unwrap();

    let (resource_info_1, content_1) = read_direct(
        client.as_mut(),
        ReadRequest {
            resource_identity: Some(resource_id.clone()),
            download_preference: Some(DownloadPreference::Body.into()),
        },
    )
    .await
    .expect("read failed");

    assert_eq!(resource_info_1.data_object_size, Some(expected_size));
    assert_eq!(content_1.len() as u64, expected_size);
    assert_eq!(Sha256::digest(&content_1).as_ref(), expected_hash);

    let (resource_info_2, content_2) = read_direct(
        client.as_mut(),
        ReadRequest {
            resource_identity: Some(resource_id),
            download_preference: Some(DownloadPreference::Body.into()),
        },
    )
    .await
    .expect("read failed");

    assert_eq!(resource_info_2.data_object_size, Some(expected_size));
    assert_eq!(content_2.len() as u64, expected_size);
    assert_eq!(Sha256::digest(&content_2).as_ref(), expected_hash);

    // small_object_cache intercepts stat() and pre-populates a
    // metadata-only entry keyed by identity, so the first read-by-identity is a
    // cache hit (no miss). We only assert the cache was exercised.
    assert_metric!(
        storage_fixture,
        "storage.cache.operations",
        ["access_type" => "access", "name" => "small_object_cache"],
        "name=small_object_cache"
    );
}

#[rstest]
#[case::case_01_missing_prefix("unittests/list/nonexistent")]
#[case::case_02_missing_prefix_slash("unittests/list/nonexistent/")]
#[case::case_03_file_as_prefix("unittests/list/medium/file-0000.txt")]
#[test_log::test(tokio::test)]
async fn enumerate_nonexistent_prefix(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure, Backend::Minio, Backend::GCPS3Compat)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::Public, Access::PrivateKey)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[case] base_address: &str,
) {
    let Some(base_address) = storage_fixture.make_url(backend, access, base_address) else {
        debug!("Skipping test for {:?} {:?} {:?}", backend, access, base_address);
        return;
    };

    let mut client = storage_fixture
        .file_object_client(backend, Cache::Stat, interface, api_version)
        .await;

    let request = EnumerateRequest {
        resource_address: base_address.to_string(),
    };
    let mut response_stream = client.enumerate(request).await.unwrap();

    match response_stream.next().await.unwrap() {
        Ok(response) => panic!("Expected an error, but the method returned a successful response: {response:?}"),
        Err(status) => {
            assert_eq!(status.code(), Code::NotFound, "Expected NOT_FOUND error");
        },
    }
}

// Small write_body test - selected by the nextest small profile (limited test cases for local efficiency)
#[rstest]
#[test_log::test(tokio::test)]
async fn write_body_basic(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[values(Cache::List, Cache::Stat)] cache: Cache,
) {
    write_body_impl(storage_fixture, backend, interface, access, api_version, cache).await;
}

// Full write_body test - selected by the nextest full profile (comprehensive test coverage)
#[rstest]
#[test_log::test(tokio::test)]
async fn write_body_full(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure, Backend::Minio, Backend::GCPS3Compat)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[values(Cache::List, Cache::Stat, Cache::Disabled)] cache: Cache,
) {
    write_body_impl(storage_fixture, backend, interface, access, api_version, cache).await;
}

// Shared implementation for both write_body tests
async fn write_body_impl(
    storage_fixture: &StorageFixture,
    backend: Backend,
    interface: Interface,
    access: Access,
    api_version: ApiVersion,
    cache: Cache,
) {
    let mut client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;
    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, api_version)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    // Write a file
    let file = temp_dir.new_file();
    let data = random_data(2048);
    let identity_before = write_data_body(&file, &data, None, None, &mut client).await;
    debug!(?identity_before);

    assert_identity_has_object_identifier_parameters(
        identity_before.unwrap().encoded_identity.as_ref(),
        &file,
        access == Access::Versioned,
    );

    // Validate the written file
    validate_file(&file, &data, &mut client).await;

    // Rewrite the file and reread, testing cache invalidation
    let data = random_data(2048);
    let identity_after = write_data_body(&file, &data, None, None, &mut client).await;
    debug!(?identity_after);

    assert_identity_has_object_identifier_parameters(
        identity_after.unwrap().encoded_identity.as_ref(),
        &file,
        access == Access::Versioned,
    );

    // Validate the new data
    validate_file(&file, &data, &mut client).await;

    // Clean up the directory
    temp_dir.cleanup().await;

    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.requests",
        "write",
        ["result" => "Ok"]
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.sdk.requests",
        "write",
        ["result" => "Ok"]
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.write.operations",
        "write",
        ["upload_preference" => "body"],
        1
    );

    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.request.duration",
        "write",
        [],
        1
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.sdk.request.duration",
        "write",
        [],
        1
    );

    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.read.object.size",
        "write",
        [],
        1
    );
}

// Small write_multipart test - selected by the nextest small profile (limited test cases for local efficiency)
#[rstest]
#[case::case_01_10mib_two_parts(10 * MIB, 2)]
#[test_log::test(tokio::test)]
async fn write_multipart_basic(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[case] data_size: u64,
    #[case] part_count: usize,
    #[values(Cache::List, Cache::Stat)] cache: Cache,
) {
    write_multipart_impl(
        storage_fixture,
        backend,
        interface,
        access,
        api_version,
        data_size,
        part_count,
        cache,
    )
    .await;
}

// Full write_multipart test - selected by the nextest full profile (comprehensive test coverage)
#[rstest]
#[case::case_01_2kib_one_part(2 * bytesize::KIB, 1)]
#[case::case_02_10mib_one_part(10 * MIB, 1)]
#[case::case_03_10mib_two_parts(10 * MIB, 2)]
#[case::case_04_20mib_four_parts(20 * MIB, 4)]
#[test_log::test(tokio::test)]
async fn write_multipart_full(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure, Backend::Minio, Backend::GCPS3Compat)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[case] data_size: u64,
    #[case] part_count: usize,
    #[values(Cache::List, Cache::Stat, Cache::Disabled)] cache: Cache,
) {
    write_multipart_impl(
        storage_fixture,
        backend,
        interface,
        access,
        api_version,
        data_size,
        part_count,
        cache,
    )
    .await;
}

// Shared implementation for both write_multipart tests
async fn write_multipart_impl(
    storage_fixture: &StorageFixture,
    backend: Backend,
    interface: Interface,
    access: Access,
    api_version: ApiVersion,
    data_size: u64,
    part_count: usize,
    cache: Cache,
) {
    let mut client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;
    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, api_version)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    // Write a file
    let file = temp_dir.new_file();
    let data = random_data(data_size as usize);
    let identity_before = write_data_multipart(&file, &data, part_count, None, None, &mut client).await;

    assert_identity_has_object_identifier_parameters(
        identity_before.unwrap().encoded_identity.as_ref(),
        &file,
        access == Access::Versioned,
    );

    // Validate the written file
    validate_file(&file, &data, &mut client).await;

    // Rewrite the file, testing cache invalidation
    let data = random_data(data_size as usize);
    let identity_after = write_data_multipart(&file, &data, part_count, None, None, &mut client).await;

    assert_identity_has_object_identifier_parameters(
        identity_after.unwrap().encoded_identity.as_ref(),
        &file,
        access == Access::Versioned,
    );

    // Validate the updated file
    validate_file(&file, &data, &mut client).await;

    // Clean up the directory
    temp_dir.cleanup().await;

    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.requests",
        "write",
        ["result" => "Ok"]
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.write.operations",
        "write",
        ["upload_preference" => "multipart"],
        1
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.request.duration",
        "write",
        [],
        1
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.read.object.size",
        "write",
        [],
        1
    );
}

// Small write_redirect test - selected by the nextest small profile (limited test cases for local efficiency)
#[rstest]
#[test_log::test(tokio::test)]
async fn write_redirect_basic(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[values(Cache::List, Cache::Stat)] cache: Cache,
) {
    write_redirect_impl(storage_fixture, backend, interface, access, api_version, cache).await;
}

// Full write_redirect test - selected by the nextest full profile (comprehensive test coverage)
#[rstest]
#[test_log::test(tokio::test)]
async fn write_redirect_full(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure, Backend::Minio, Backend::GCPS3Compat)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[values(Cache::List, Cache::Stat, Cache::Disabled)] cache: Cache,
) {
    write_redirect_impl(storage_fixture, backend, interface, access, api_version, cache).await;
}

// Shared implementation for both write_redirect tests
async fn write_redirect_impl(
    storage_fixture: &StorageFixture,
    backend: Backend,
    interface: Interface,
    access: Access,
    api_version: ApiVersion,
    cache: Cache,
) {
    let mut client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;
    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, api_version)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    // Write a file
    let file = temp_dir.new_file();
    let data = random_data(2048);
    let resource_identity_before = write_data_redirect(&file, &data, None, None, &mut client, backend).await;

    assert_identity_has_object_identifier_parameters(
        resource_identity_before.unwrap().encoded_identity.as_ref(),
        &file,
        access == Access::Versioned,
    );

    // Validate the written file
    validate_file(&file, &data, &mut client).await;

    // Rewrite the file, testing cache invalidation
    let data = random_data(2048);
    let resource_identity_after = write_data_redirect(&file, &data, None, None, &mut client, backend).await;

    assert_identity_has_object_identifier_parameters(
        resource_identity_after.unwrap().encoded_identity.as_ref(),
        &file,
        access == Access::Versioned,
    );
    validate_file(&file, &data, &mut client).await;

    // Clean up the directory
    temp_dir.cleanup().await;

    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.requests",
        "write",
        ["result" => "Ok"]
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.write.operations",
        "write",
        ["upload_preference" => "redirect"],
        1
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.request.duration",
        "write",
        [],
        1
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.read.object.size",
        "write",
        [],
        1
    );
}
#[rstest]
#[test_log::test(tokio::test)]
async fn write_body_after_file_change(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure, Backend::Minio)] backend: Backend, // GCP does not support conditional writes.
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[values(Cache::List, Cache::Stat, Cache::Disabled)] cache: Cache,
) {
    let mut client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;
    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, api_version)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    // Write the file once and get the resource ID
    let file = temp_dir.new_file();
    let data = random_data(2048);
    let resource_id = write_data_body(&file, &data, None, None, &mut client).await;

    // Write the file again with new data
    let data = random_data(2048);
    write_data_body(&file, &data, None, None, &mut client).await;

    // Try to write again with the previous version
    write_data_body(
        &file,
        &data,
        Some(ExpectedFailure::PreconditionFailed),
        resource_id,
        &mut client,
    )
    .await;
    // Clean up the directory
    temp_dir.cleanup().await;

    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.requests",
        "write",
        ["result" => "Ok"]
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.sdk.requests",
        "write",
        ["result" => "FailedPrecondition"]
    );
}
#[rstest]
#[test_log::test(tokio::test)]
async fn write_multipart_after_file_change(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure, Backend::Minio)] backend: Backend, // GCP does not support conditional writes.
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[values(Cache::List, Cache::Stat, Cache::Disabled)] cache: Cache,
) {
    let mut client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;
    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, api_version)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    // Write the file once and get the resource ID
    let file = temp_dir.new_file();
    let data = random_data(2048);
    let resource_id = write_data_body(&file, &data, None, None, &mut client).await;

    // Write the file again with new data
    let data = random_data(2048);
    write_data_body(&file, &data, None, None, &mut client).await;

    // Try to write again with the previous version
    write_data_multipart(
        &file,
        &data,
        1,
        Some(ExpectedFailure::PreconditionFailed),
        resource_id,
        &mut client,
    )
    .await;

    // Clean up the directory
    temp_dir.cleanup().await;

    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.requests",
        "write",
        ["result" => "Ok"]
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.sdk.requests",
        "complete_multipart_upload",
        ["result" => "FailedPrecondition"]
    );
}
#[rstest]
#[test_log::test(tokio::test)]
async fn write_redirect_after_file_change(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure, Backend::Minio)] backend: Backend, // GCP does not support conditional writes.
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[values(Cache::List, Cache::Stat, Cache::Disabled)] cache: Cache,
) {
    let mut client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;
    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, api_version)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    // Write the file once and get the resource ID
    let file = temp_dir.new_file();
    let data = random_data(2048);
    let resource_id = write_data_body(&file, &data, None, None, &mut client).await;

    // Write the file again with new data
    let data = random_data(2048);
    write_data_body(&file, &data, None, None, &mut client).await;

    // Try to write again with the previous version
    write_data_redirect(
        &file,
        &data,
        Some(ExpectedFailure::PreconditionFailed),
        resource_id,
        &mut client,
        backend,
    )
    .await;
    // Clean up the directory
    temp_dir.cleanup().await;

    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.requests",
        "write",
        ["result" => "Ok"]
    );
}

// Small copy_new_file test - selected by the nextest small profile (limited test cases for local efficiency)
#[rstest]
#[test_log::test(tokio::test)]
async fn copy_new_file_basic(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(ApiVersion::V1Alpha)] api_version: ApiVersion,
    #[values(Cache::List, Cache::Stat)] cache: Cache,
) {
    copy_new_file_impl(storage_fixture, backend, interface, access, api_version, cache).await;
}

// Full copy_new_file test - selected by the nextest full profile (comprehensive test coverage)
#[rstest]
#[test_log::test(tokio::test)]
async fn copy_new_file_full(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure, Backend::Minio, Backend::GCPS3Compat)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(ApiVersion::V1Alpha)] api_version: ApiVersion,
    #[values(Cache::List, Cache::Stat, Cache::Disabled)] cache: Cache,
) {
    copy_new_file_impl(storage_fixture, backend, interface, access, api_version, cache).await;
}

// Shared implementation for both copy_new_file tests
async fn copy_new_file_impl(
    storage_fixture: &StorageFixture,
    backend: Backend,
    interface: Interface,
    access: Access,
    api_version: ApiVersion,
    cache: Cache,
) {
    // Just use V1Alpha for test setup
    let mut client = storage_fixture
        .file_object_client(backend, cache, interface, ApiVersion::V1Alpha)
        .await;
    // Test copy with different versions
    let mut copy_client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;
    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, ApiVersion::V1Alpha)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    // Write the source file
    let src_file = temp_dir.new_file();
    let data = random_data(2048);
    let resource_id = write_data_body(&src_file, &data, None, None, &mut client).await;

    let dst_file = temp_dir.new_file();

    // Perform the copy
    let resource_identity = copy_file(resource_id.unwrap(), &dst_file, None, None, &mut copy_client).await;

    assert_identity_has_object_identifier_parameters(
        resource_identity.unwrap().encoded_identity.as_ref(),
        &dst_file,
        access == Access::Versioned,
    );

    // Validate the copied file
    validate_file(&dst_file, &data, &mut client).await;

    // Clean up the directory
    temp_dir.cleanup().await;

    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.requests",
        "copy",
        ["result" => "Ok"]
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.sdk.requests",
        "copy",
        ["result" => "Ok"]
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.request.duration",
        "copy",
        [],
        1
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.sdk.request.duration",
        "copy",
        [],
        1
    );
}
#[rstest]
#[test_log::test(tokio::test)]
async fn copy_overwrite(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure, Backend::Minio, Backend::GCPS3Compat)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(ApiVersion::V1Alpha)] api_version: ApiVersion,
    #[values(Cache::List, Cache::Stat, Cache::Disabled)] cache: Cache,
) {
    // Just use V1Alpha for test setup
    let mut client = storage_fixture
        .file_object_client(backend, cache, interface, ApiVersion::V1Alpha)
        .await;
    // Test copy with different versions
    let mut copy_client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;
    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, ApiVersion::V1Alpha)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    // Write the source file
    let src_file = temp_dir.new_file();
    let src_data = random_data(2048);
    let resource_id = write_data_body(&src_file, &src_data, None, None, &mut client).await;

    assert_identity_has_object_identifier_parameters(
        resource_id.as_ref().unwrap().encoded_identity.as_ref(),
        &src_file,
        access == Access::Versioned,
    );

    // Write the dst file
    let dst_file = temp_dir.new_file();
    let dst_data = random_data(2048);
    let dst_resource_id = write_data_body(&dst_file, &dst_data, None, None, &mut client).await;

    assert_identity_has_object_identifier_parameters(
        dst_resource_id.as_ref().unwrap().encoded_identity.as_ref(),
        &dst_file,
        access == Access::Versioned,
    );

    // Validate the dest file to populate the cache
    validate_file(&dst_file, &dst_data, &mut client).await;

    // Perform the copy
    copy_file(resource_id.unwrap(), &dst_file, None, None, &mut copy_client).await;

    // Validate the copied file
    validate_file(&dst_file, &src_data, &mut client).await;

    // Clean up the directory
    temp_dir.cleanup().await;

    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.requests",
        "copy",
        ["result" => "Ok"]
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.sdk.requests",
        "copy",
        ["result" => "Ok"]
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.request.duration",
        "copy",
        [],
        1
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.sdk.request.duration",
        "copy",
        [],
        1
    );
}
#[rstest]
#[test_log::test(tokio::test)]
async fn copy_after_source_change(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure, Backend::Minio)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(ApiVersion::V1Alpha)] api_version: ApiVersion,
    #[values(Cache::List, Cache::Stat, Cache::Disabled)] cache: Cache,
) {
    // Just use V1Alpha for test setup
    let mut client = storage_fixture
        .file_object_client(backend, cache, interface, ApiVersion::V1Alpha)
        .await;
    // Test copy with different versions
    let mut copy_client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;
    let mut versioning_client = storage_fixture
        .versioning_client(backend, cache, interface, ApiVersion::V1Alpha)
        .await;
    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, ApiVersion::V1Alpha)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    // Write the source file
    let src_file = temp_dir.new_file();
    let data = random_data(2048);
    write_data_body(&src_file, &data, None, None, &mut client).await;
    if backend == Backend::Minio {
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }

    let resource_id = client
        .stat(StatRequest {
            resource_address: src_file.clone(),
        })
        .await
        .unwrap()
        .resource_info
        .unwrap()
        .resource_identity
        .unwrap();
    trace!(?resource_id);

    // Capture the identity of the original object up front, before the source is rewritten.
    //
    // Versioning state is operator-configured only (no runtime detection); the fixtures
    // leave it unset, so enumerate_versions always reaches the backend. On a genuinely
    // versioned bucket (Access::Versioned) we resolve the newest version id and later
    // expect the copy to succeed against that pinned version. On a non-versioned
    // PrivateKey bucket we deliberately use the stat-derived, etag-only `resource_id`:
    // get_latest_version_id would panic there because the listing carries no usable
    // version ids.
    let source_identity = if access == Access::Versioned {
        let resource_version_id = ResourceIdentity {
            encoded_identity: get_latest_version_id(&src_file, &mut versioning_client).await,
        };
        trace!(?resource_version_id);
        resource_version_id
    } else {
        // PrivateKey: etag-only identity derived from stat.
        resource_id
    };

    // Write the source again
    let data2 = random_data(2048);
    write_data_body(&src_file, &data2, None, None, &mut client).await;

    let dst_file = temp_dir.new_file();

    if backend == Backend::Minio {
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }

    if access == Access::Versioned {
        // With versioning, the copy succeeds and copies the original (pinned) version.
        copy_file(source_identity, &dst_file, None, None, &mut copy_client).await;
        // Validate the copied file
        validate_file(&dst_file, &data, &mut client).await;
    } else {
        // PrivateKey (non-versioned): the etag-only identity no longer matches after the
        // source was rewritten, so the etag-conditional copy must fail with
        // FAILED_PRECONDITION. This keeps regression coverage of the etag-conditional copy
        // path on S3/Minio (mirroring Azure).
        copy_file(
            source_identity,
            &dst_file,
            Some(ExpectedFailure::PreconditionFailed),
            None,
            &mut copy_client,
        )
        .await;
    }

    // Clean up the directory
    temp_dir.cleanup().await;

    let copy_result = if access == Access::Versioned {
        "Ok"
    } else {
        "FailedPrecondition"
    };
    assert_method_metric!(storage_fixture, backend, api_version, "storage.requests", "copy", ["result" => copy_result]);
    assert_method_metric!(storage_fixture, backend, api_version, "storage.sdk.requests", "copy", ["result" => copy_result]);
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.request.duration",
        "copy",
        [],
        1
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.sdk.request.duration",
        "copy",
        [],
        1
    );
}
#[rstest]
#[test_log::test(tokio::test)]
async fn copy_after_dest_change(
    storage_fixture: &StorageFixture,
    // Minio and GCPS3Compat don't support conditional copy
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(ApiVersion::V1Alpha)] api_version: ApiVersion,
    #[values(Cache::List, Cache::Stat, Cache::Disabled)] cache: Cache,
) {
    // Just use V1Alpha for test setup
    let mut client = storage_fixture
        .file_object_client(backend, cache, interface, ApiVersion::V1Alpha)
        .await;
    // Test copy with different versions
    let mut copy_client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;
    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, ApiVersion::V1Alpha)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    // Write the source file
    let src_file = temp_dir.new_file();
    let data = random_data(2048);
    let resource_id = write_data_body(&src_file, &data, None, None, &mut client).await;

    // Write the dst file
    let dst_file = temp_dir.new_file();
    let dst_data = random_data(2048);
    let dst_resource_id = write_data_body(&dst_file, &dst_data, None, None, &mut client).await;

    // Write the dst again
    let dst_data = random_data(2048);
    write_data_body(&dst_file, &dst_data, None, None, &mut client).await;

    // Perform the copy expecting the failure
    copy_file(
        resource_id.unwrap(),
        &dst_file,
        Some(ExpectedFailure::PreconditionFailed),
        dst_resource_id,
        &mut copy_client,
    )
    .await;

    // Clean up the directory
    temp_dir.cleanup().await;

    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.requests",
        "copy",
        ["result" => "FailedPrecondition"]
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.sdk.requests",
        "copy",
        ["result" => "FailedPrecondition"]
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.request.duration",
        "copy",
        [],
        1
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.sdk.request.duration",
        "copy",
        [],
        1
    );
}

// Small delete test - selected by the nextest small profile (limited test cases for local efficiency)
#[rstest]
#[test_log::test(tokio::test)]
async fn delete_basic(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[values(Cache::List, Cache::Stat)] cache: Cache,
) {
    delete_impl(storage_fixture, backend, interface, access, api_version, cache).await;
}

// Full delete test - selected by the nextest full profile (comprehensive test coverage)
#[rstest]
#[test_log::test(tokio::test)]
async fn delete_full(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure, Backend::Minio, Backend::GCPS3Compat)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[values(Cache::List, Cache::Stat, Cache::Disabled)] cache: Cache,
) {
    delete_impl(storage_fixture, backend, interface, access, api_version, cache).await;
}

// Shared implementation for both delete tests
async fn delete_impl(
    storage_fixture: &StorageFixture,
    backend: Backend,
    interface: Interface,
    access: Access,
    api_version: ApiVersion,
    cache: Cache,
) {
    let mut client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;
    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, api_version)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    // Write a file
    let file = temp_dir.new_file();
    let data = random_data(2048);
    write_data_body(&file, &data, None, None, &mut client).await;

    // Validate the file to populate the stat cache if enabled
    validate_file(&file, &data, &mut client).await;

    // Delete the file
    delete_file(&file, None, None, &mut client).await;

    // File shouldn't exist
    assert_eq!(
        client
            .stat(StatRequest { resource_address: file })
            .await
            .err()
            .unwrap()
            .code(),
        tonic::Code::NotFound
    );

    // Clean up the directory
    temp_dir.cleanup().await;

    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.requests",
        "delete",
        ["result" => "Ok"]
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.sdk.requests",
        "delete",
        ["result" => "Ok"]
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.request.duration",
        "delete",
        [],
        1
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.sdk.request.duration",
        "delete",
        [],
        1
    );
}
#[rstest]
#[test_log::test(tokio::test)]
async fn delete_after_file_change(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend, // Minio and GCP do not support conditional deletes.
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[values(Cache::List, Cache::Stat, Cache::Disabled)] cache: Cache,
) {
    let mut client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;
    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, api_version)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    // Write a file
    let file = temp_dir.new_file();
    let data = random_data(2048);
    let resource_id = write_data_body(&file, &data, None, None, &mut client).await;

    // Write the file again
    let data = random_data(2048);
    write_data_body(&file, &data, None, None, &mut client).await;

    // Delete the file
    delete_file(
        &file,
        Some(ExpectedFailure::PreconditionFailed),
        resource_id,
        &mut client,
    )
    .await;

    // File should still exist
    validate_file(&file, &data, &mut client).await;

    // Clean up the directory
    temp_dir.cleanup().await;

    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.requests",
        "delete",
        ["result" => "FailedPrecondition"]
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.sdk.requests",
        "delete",
        ["result" => "FailedPrecondition"]
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.request.duration",
        "delete",
        [],
        1
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.sdk.request.duration",
        "delete",
        [],
        1
    );
}
#[rstest]
#[test_log::test(tokio::test)]
async fn write_between_stat_and_read(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure, Backend::Minio)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[values(Cache::List, Cache::Stat, Cache::Disabled)] cache: Cache,
) {
    let mut client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;
    let mut versioning_client = storage_fixture
        .versioning_client(backend, cache, interface, ApiVersion::V1Alpha)
        .await;
    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, api_version)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    // Write a file
    let file = temp_dir.new_file();
    let data = random_data(2048);
    write_data_body(&file, &data, None, None, &mut client).await;
    if backend == Backend::Minio {
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }

    // Capture the identity to read back after the file is rewritten.
    //
    // Versioning state is operator-configured only (no runtime detection); the fixtures
    // leave it unset, so enumerate_versions always reaches the backend. On a genuinely
    // versioned bucket (Access::Versioned) we resolve the newest version id and later
    // expect the versioned read to return the original data. On a non-versioned
    // PrivateKey bucket we use the stat-derived, etag-only identity for S3/Minio —
    // get_latest_version_id would panic there because the listing carries no usable
    // version ids. (Azure PrivateKey keeps using its enumerate-derived etag-only
    // identity — its listing still yields an identity.)
    let resource_id = if access == Access::PrivateKey && backend != Backend::Azure {
        client
            .stat(StatRequest {
                resource_address: file.clone(),
            })
            .await
            .unwrap()
            .resource_info
            .unwrap()
            .resource_identity
            .unwrap()
    } else {
        ResourceIdentity {
            encoded_identity: get_latest_version_id(&file, &mut versioning_client).await,
        }
    };
    trace!(?resource_id);

    if backend == Backend::Minio {
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }
    // Write the file again
    let new_data = random_data(3072);
    write_data_body(&file, &new_data, None, None, &mut client).await;
    if backend == Backend::Minio {
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }
    match access {
        Access::Versioned => {
            // Validate the versioned read gets back the original data
            validate_resource_id(&resource_id, &data, &mut client).await;
        },
        Access::PrivateKey => {
            // PrivateKey buckets are non-versioned: the identity is etag-only, so reading it
            // after the file was rewritten must fail with FAILED_PRECONDITION (the etag no
            // longer matches). This preserves regression coverage of the etag-conditional
            // read path on S3/Minio as well as Azure.
            let error_code = match client
                .read(ReadRequest {
                    resource_identity: Some(resource_id),
                    download_preference: Some(DownloadPreference::Body.into()),
                })
                .await
            {
                Ok(mut s) => s.next().await.unwrap().err().unwrap().code(),
                Err(e) => e.code(),
            };
            assert_eq!(error_code, tonic::Code::FailedPrecondition);
        },
        other => unreachable!(
            "write_between_stat_and_read is only parametrized over PrivateKey and Versioned, got {other:?}"
        ),
    };

    // Clean up the directory
    temp_dir.cleanup().await;
}

// Small move_new_file test - selected by the nextest small profile (limited test cases for local efficiency)
#[rstest]
#[test_log::test(tokio::test)]
async fn move_new_file_basic(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(Cache::List, Cache::Stat)] cache: Cache,
) {
    move_new_file_impl(storage_fixture, backend, interface, access, cache).await;
}

// Full move_new_file test - selected by the nextest full profile (comprehensive test coverage)
#[rstest]
#[test_log::test(tokio::test)]
async fn move_new_file_full(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure, Backend::Minio, Backend::GCPS3Compat)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(Cache::List, Cache::Stat, Cache::Disabled)] cache: Cache,
) {
    move_new_file_impl(storage_fixture, backend, interface, access, cache).await;
}

// Shared implementation for both copy_new_file tests
async fn move_new_file_impl(
    storage_fixture: &StorageFixture,
    backend: Backend,
    interface: Interface,
    access: Access,
    cache: Cache,
) {
    let api_version = ApiVersion::V1Alpha;
    let mut client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;

    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, api_version)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    // Write the source file
    let src_file = temp_dir.new_file();
    let data = random_data(2048);
    write_data_body(&src_file, &data, None, None, &mut client).await;

    let dst_file = temp_dir.new_file();

    // Perform the move
    let resource_identity = move_file(&src_file, &dst_file, None, None, None, &mut client).await;

    assert_identity_has_object_identifier_parameters(
        resource_identity.unwrap().encoded_identity.as_ref(),
        &dst_file,
        access == Access::Versioned,
    );

    // Validate the moved file
    validate_file(&dst_file, &data, &mut client).await;

    // Validate the source file is deleted
    assert_eq!(
        client
            .stat(StatRequest {
                resource_address: src_file
            })
            .await
            .err()
            .unwrap()
            .code(),
        tonic::Code::NotFound
    );

    // Clean up the directory
    temp_dir.cleanup().await;

    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.requests",
        "move",
        ["result" => "Ok"]
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.sdk.requests",
        "move",
        ["result" => "Ok"]
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.request.duration",
        "move",
        [],
        1
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.sdk.request.duration",
        "move",
        [],
        1
    );
}
#[rstest]
#[test_log::test(tokio::test)]
async fn move_overwrite(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure, Backend::Minio, Backend::GCPS3Compat)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(Cache::List, Cache::Stat, Cache::Disabled)] cache: Cache,
) {
    let api_version = ApiVersion::V1Alpha;
    let mut client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;

    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, api_version)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    // Write the source file
    let src_file = temp_dir.new_file();
    let src_data = random_data(2048);
    write_data_body(&src_file, &src_data, None, None, &mut client).await;

    // Write the destination file
    let dst_file = temp_dir.new_file();
    let dst_data = random_data(2048);
    write_data_body(&dst_file, &dst_data, None, None, &mut client).await;

    // Perform the move
    let resource_identity = move_file(&src_file, &dst_file, None, None, None, &mut client).await;

    assert_identity_has_object_identifier_parameters(
        resource_identity.unwrap().encoded_identity.as_ref(),
        &dst_file,
        access == Access::Versioned,
    );

    // Validate the moved file
    validate_file(&dst_file, &src_data, &mut client).await;

    // Validate the source file is deleted
    assert_eq!(
        client
            .stat(StatRequest {
                resource_address: src_file
            })
            .await
            .err()
            .unwrap()
            .code(),
        tonic::Code::NotFound
    );

    // Clean up the directory
    temp_dir.cleanup().await;

    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.requests",
        "move",
        ["result" => "Ok"]
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.sdk.requests",
        "move",
        ["result" => "Ok"]
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.request.duration",
        "move",
        [],
        1
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.sdk.request.duration",
        "move",
        [],
        1
    );
}
#[rstest]
#[test_log::test(tokio::test)]
async fn move_after_source_change(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure, Backend::Minio, Backend::GCPS3Compat)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(Cache::List, Cache::Stat, Cache::Disabled)] cache: Cache,
) {
    let api_version = ApiVersion::V1Alpha;
    let mut client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;

    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, api_version)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    // Write the source file
    let src_file = temp_dir.new_file();
    let src_data_original = random_data(2048);
    let src_resource_id_original = write_data_body(&src_file, &src_data_original, None, None, &mut client).await;

    // Write the source again
    let src_data_new = random_data(2048);
    write_data_body(&src_file, &src_data_new, None, None, &mut client).await;

    let dst_file = temp_dir.new_file();

    // Trying to move the source will fail because the source has changed since the original version.
    move_file(
        &src_file,
        &dst_file,
        Some(ExpectedFailure::PreconditionFailed),
        src_resource_id_original,
        None,
        &mut client,
    )
    .await;

    // Clean up the directory
    temp_dir.cleanup().await;

    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.requests",
        "move",
        ["result" => "FailedPrecondition"]
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.sdk.requests",
        "move",
        ["result" => "FailedPrecondition"]
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.request.duration",
        "move",
        [],
        1
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.sdk.request.duration",
        "move",
        [],
        1
    );
}
#[rstest]
#[test_log::test(tokio::test)]
async fn move_after_destination_change(
    storage_fixture: &StorageFixture,
    // Minio and GCPS3Compat don't support conditional copy
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(Cache::List, Cache::Stat, Cache::Disabled)] cache: Cache,
) {
    let api_version = ApiVersion::V1Alpha;
    let mut client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;

    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, ApiVersion::V1Alpha)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    // Write the source file
    let src_file = temp_dir.new_file();
    let src_data = random_data(2048);
    let src_resource_id = write_data_body(&src_file, &src_data, None, None, &mut client).await;

    // Write the destination file
    let dst_file = temp_dir.new_file();
    let dst_data = random_data(2048);
    let dst_resource_id = write_data_body(&dst_file, &dst_data, None, None, &mut client).await;

    // Write the destination again
    let dst_data = random_data(2048);
    write_data_body(&dst_file, &dst_data, None, None, &mut client).await;

    // Perform the move
    move_file(
        &src_file,
        &dst_file,
        Some(ExpectedFailure::PreconditionFailed),
        None,
        dst_resource_id,
        &mut client,
    )
    .await;

    // Validate the source file is still there by comparing etags
    assert_eq!(
        get_etag_from_encoded_identity(
            &client
                .stat(StatRequest {
                    resource_address: src_file,
                })
                .await
                .unwrap()
                .resource_info
                .unwrap()
                .resource_identity
                .unwrap()
        )
        .unwrap(),
        get_etag_from_encoded_identity(src_resource_id.as_ref().unwrap()).unwrap(),
    );

    // Clean up the directory
    temp_dir.cleanup().await;

    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.requests",
        "move",
        ["result" => "FailedPrecondition"]
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.sdk.requests",
        "move",
        ["result" => "FailedPrecondition"]
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.request.duration",
        "move",
        [],
        1
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.sdk.request.duration",
        "move",
        [],
        1
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn get_optimistic_locking_support(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure, Backend::Minio, Backend::GCPS3Compat)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(Cache::Disabled)] cache: Cache,
) {
    let api_version = ApiVersion::V1Alpha;
    let mut client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;

    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, api_version)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    let response = client
        .get_optimistic_locking_support(GetOptimisticLockingSupportRequest {
            resource_address: temp_dir.new_file(),
        })
        .await
        .unwrap();

    // Capability response varies per backend variant for both the previous_version
    // conditional ops and the create-only fail_if_exists ops.
    match backend {
        Backend::S3 | Backend::Azure => {
            assert!(response.supports_write);
            assert!(response.supports_delete);
            assert!(response.supports_copy);
            assert!(response.supports_move);
            assert!(response.supports_write_fail_if_exists);
            assert!(response.supports_copy_fail_if_exists);
            assert!(response.supports_move_fail_if_exists);
        },
        Backend::Minio => {
            // MinIO supports conditional writes and write fail_if_exists, but
            // its CopyObject/move/DeleteObject do not honor x-amz-copy-source-if-*
            // / If-Match conditions reliably.
            assert!(response.supports_write);
            assert!(!response.supports_delete);
            assert!(!response.supports_copy);
            assert!(!response.supports_move);
            assert!(response.supports_write_fail_if_exists);
            assert!(!response.supports_copy_fail_if_exists);
            assert!(!response.supports_move_fail_if_exists);
        },
        Backend::GCPS3Compat => {
            // GCP's S3 interop endpoint doesn't support conditional operations
            // of any kind.
            assert!(!response.supports_write);
            assert!(!response.supports_delete);
            assert!(!response.supports_copy);
            assert!(!response.supports_move);
            assert!(!response.supports_write_fail_if_exists);
            assert!(!response.supports_copy_fail_if_exists);
            assert!(!response.supports_move_fail_if_exists);
        },
    }

    temp_dir.cleanup().await;
}

#[rstest]
#[test_log::test(tokio::test)]
async fn write_fail_if_exists_rejects_existing_file(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[values(Cache::Disabled)] cache: Cache,
) {
    let mut client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;

    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, api_version)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    let file = temp_dir.new_file();
    let original_data = random_data(512);
    write_data_body(&file, &original_data, None, None, &mut client).await;

    let replacement_data = random_data(512);
    write_data_body_with_options(
        &file,
        &replacement_data,
        Some(ExpectedFailure::AlreadyExists),
        None,
        Some(true),
        &mut client,
    )
    .await;
    validate_file(&file, &original_data, &mut client).await;

    temp_dir.cleanup().await;
}

#[rstest]
#[test_log::test(tokio::test)]
async fn write_fail_if_exists_creates_new_file(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[values(Cache::Disabled, Cache::Stat)] cache: Cache,
) {
    let mut client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;

    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, api_version)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    let file = temp_dir.new_file();
    let data = random_data(512);
    let identity = write_data_body_with_options(&file, &data, None, None, Some(true), &mut client)
        .await
        .expect("write with fail_if_exists=true to a fresh address should succeed");
    assert!(identity.encoded_identity.contains(&file));
    validate_file(&file, &data, &mut client).await;

    temp_dir.cleanup().await;
}

#[rstest]
#[test_log::test(tokio::test)]
async fn write_fail_if_exists_with_previous_version_returns_invalid_argument_full(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(ApiVersion::V1Alpha)] api_version: ApiVersion,
    #[values(Cache::Disabled)] cache: Cache,
) {
    let mut client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;

    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, api_version)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    // Need an existing object so we have a real previous_version to send.
    let file = temp_dir.new_file();
    let data = random_data(512);
    let identity = write_data_body(&file, &data, None, None, &mut client).await.unwrap();

    // fail_if_exists=true combined with previous_version is mutually exclusive
    // per the proto and must return INVALID_ARGUMENT, not ALREADY_EXISTS.
    let replacement_data = random_data(512);
    write_data_body_with_options(
        &file,
        &replacement_data,
        Some(ExpectedFailure::InvalidArgument),
        Some(identity),
        Some(true),
        &mut client,
    )
    .await;

    temp_dir.cleanup().await;
}

#[rstest]
#[test_log::test(tokio::test)]
async fn copy_fail_if_exists_rejects_existing_destination(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(Cache::Disabled)] cache: Cache,
) {
    let api_version = ApiVersion::V1Alpha;
    let mut client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;

    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, api_version)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    let src_file = temp_dir.new_file();
    let src_data = random_data(512);
    let src_identity = write_data_body(&src_file, &src_data, None, None, &mut client)
        .await
        .unwrap();

    let dst_file = temp_dir.new_file();
    let dst_data = random_data(512);
    write_data_body(&dst_file, &dst_data, None, None, &mut client).await;

    copy_file_with_options(
        src_identity,
        &dst_file,
        Some(ExpectedFailure::AlreadyExists),
        None,
        Some(true),
        &mut client,
    )
    .await;
    validate_file(&dst_file, &dst_data, &mut client).await;

    temp_dir.cleanup().await;
}

#[rstest]
#[test_log::test(tokio::test)]
async fn copy_fail_if_exists_creates_new_destination_full(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(Cache::Disabled, Cache::Stat)] cache: Cache,
) {
    let api_version = ApiVersion::V1Alpha;
    let mut client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;

    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, api_version)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    let src_file = temp_dir.new_file();
    let src_data = random_data(512);
    let src_identity = write_data_body(&src_file, &src_data, None, None, &mut client)
        .await
        .unwrap();

    let dst_file = temp_dir.new_file();
    let identity = copy_file_with_options(src_identity, &dst_file, None, None, Some(true), &mut client)
        .await
        .expect("copy with fail_if_exists=true to a fresh address should succeed");
    assert!(identity.encoded_identity.contains(&dst_file));
    validate_file(&dst_file, &src_data, &mut client).await;

    temp_dir.cleanup().await;
}

#[rstest]
#[test_log::test(tokio::test)]
async fn copy_fail_if_exists_with_previous_version_returns_invalid_argument_full(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(Cache::Disabled)] cache: Cache,
) {
    let api_version = ApiVersion::V1Alpha;
    let mut client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;

    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, api_version)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    let src_file = temp_dir.new_file();
    let src_data = random_data(512);
    let src_identity = write_data_body(&src_file, &src_data, None, None, &mut client)
        .await
        .unwrap();

    let dst_file = temp_dir.new_file();
    let dst_identity = write_data_body(&dst_file, &random_data(512), None, None, &mut client)
        .await
        .unwrap();

    // fail_if_exists + previous_version is mutually exclusive per the proto.
    copy_file_with_options(
        src_identity,
        &dst_file,
        Some(ExpectedFailure::InvalidArgument),
        Some(dst_identity),
        Some(true),
        &mut client,
    )
    .await;

    temp_dir.cleanup().await;
}

#[rstest]
#[test_log::test(tokio::test)]
async fn copy_fail_if_exists_with_stale_source_returns_failed_precondition_full(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(Cache::Disabled)] cache: Cache,
) {
    let api_version = ApiVersion::V1Alpha;
    let mut client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;

    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, api_version)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    let src_file = temp_dir.new_file();
    let stale_src_identity = write_data_body(&src_file, &random_data(512), None, None, &mut client)
        .await
        .unwrap();
    // Overwrite the source so the stale identity's etag no longer matches.
    write_data_body(&src_file, &random_data(512), None, None, &mut client).await;

    // Destination is fresh — the only failing precondition is the source
    // staleness check (copy_source_if_match). Must surface as
    // FAILED_PRECONDITION, NOT ALREADY_EXISTS.
    let dst_file = temp_dir.new_file();
    copy_file_with_options(
        stale_src_identity,
        &dst_file,
        Some(ExpectedFailure::PreconditionFailed),
        None,
        Some(true),
        &mut client,
    )
    .await;

    temp_dir.cleanup().await;
}

#[rstest]
#[test_log::test(tokio::test)]
async fn move_fail_if_exists_rejects_existing_destination(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(Cache::Disabled)] cache: Cache,
) {
    let api_version = ApiVersion::V1Alpha;
    let mut client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;

    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, api_version)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    let src_file = temp_dir.new_file();
    let src_data = random_data(512);
    write_data_body(&src_file, &src_data, None, None, &mut client).await;

    let dst_file = temp_dir.new_file();
    let dst_data = random_data(512);
    write_data_body(&dst_file, &dst_data, None, None, &mut client).await;

    move_file_with_options(
        &src_file,
        &dst_file,
        Some(ExpectedFailure::AlreadyExists),
        None,
        None,
        Some(true),
        &mut client,
    )
    .await;
    validate_file(&src_file, &src_data, &mut client).await;
    validate_file(&dst_file, &dst_data, &mut client).await;

    temp_dir.cleanup().await;
}

#[rstest]
#[test_log::test(tokio::test)]
async fn move_fail_if_exists_creates_new_destination_full(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(Cache::Disabled, Cache::Stat)] cache: Cache,
) {
    let api_version = ApiVersion::V1Alpha;
    let mut client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;

    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, api_version)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    let src_file = temp_dir.new_file();
    let src_data = random_data(512);
    write_data_body(&src_file, &src_data, None, None, &mut client).await;

    let dst_file = temp_dir.new_file();
    let identity = move_file_with_options(&src_file, &dst_file, None, None, None, Some(true), &mut client)
        .await
        .expect("move with fail_if_exists=true to a fresh address should succeed");
    assert!(identity.encoded_identity.contains(&dst_file));
    // Destination has the original source content; source is gone.
    validate_file(&dst_file, &src_data, &mut client).await;
    let stat_err = client
        .stat(StatRequest {
            resource_address: src_file.clone(),
        })
        .await;
    assert_eq!(stat_err.unwrap_err().code(), Code::NotFound);

    temp_dir.cleanup().await;
}

#[rstest]
#[test_log::test(tokio::test)]
async fn move_fail_if_exists_with_destination_previous_version_returns_invalid_argument_full(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(Cache::Disabled)] cache: Cache,
) {
    let api_version = ApiVersion::V1Alpha;
    let mut client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;

    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, api_version)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    let src_file = temp_dir.new_file();
    write_data_body(&src_file, &random_data(512), None, None, &mut client).await;

    let dst_file = temp_dir.new_file();
    let dst_identity = write_data_body(&dst_file, &random_data(512), None, None, &mut client)
        .await
        .unwrap();

    // fail_if_exists + destination_previous_version is mutually exclusive
    // per the proto. (source_previous_version is unaffected — see below.)
    move_file_with_options(
        &src_file,
        &dst_file,
        Some(ExpectedFailure::InvalidArgument),
        None,
        Some(dst_identity),
        Some(true),
        &mut client,
    )
    .await;

    temp_dir.cleanup().await;
}

#[rstest]
#[test_log::test(tokio::test)]
async fn move_fail_if_exists_with_source_previous_version_is_legal_full(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(Cache::Disabled)] cache: Cache,
) {
    let api_version = ApiVersion::V1Alpha;
    let mut client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;

    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, api_version)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    let src_file = temp_dir.new_file();
    let src_data = random_data(512);
    let src_identity = write_data_body(&src_file, &src_data, None, None, &mut client)
        .await
        .unwrap();

    // Per the proto, fail_if_exists is mutually exclusive only with
    // destination_previous_version; combining it with source_previous_version
    // is legal and the move should succeed.
    let dst_file = temp_dir.new_file();
    let identity = move_file_with_options(
        &src_file,
        &dst_file,
        None,
        Some(src_identity),
        None,
        Some(true),
        &mut client,
    )
    .await
    .expect("move with source_previous_version + fail_if_exists is legal and should succeed");
    assert!(identity.encoded_identity.contains(&dst_file));
    validate_file(&dst_file, &src_data, &mut client).await;

    temp_dir.cleanup().await;
}

#[rstest]
#[test_log::test(tokio::test)]
async fn write_fail_if_exists_at_multipart_completion_rejects_existing_file_full(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(Cache::Disabled)] cache: Cache,
) {
    // fail_if_exists is encoded into the multipart upload_id at start and
    // re-applied at complete time. Validate the end-to-end behavior on a
    // multi-part write where the destination already exists.
    let api_version = ApiVersion::V1Alpha;
    let mut client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;

    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, api_version)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    let file = temp_dir.new_file();
    let original = random_data(1024);
    write_data_body(&file, &original, None, None, &mut client).await;

    // Issue a multipart write with fail_if_exists=true against the existing
    // destination. The Body path can short-circuit; multipart must instead
    // start the upload, upload parts, then fail at complete.
    // S3 requires each part except the last to be at least 5 MiB, so size
    // the data at 10 MiB / 2 parts.
    let replacement = random_data(10 * (MIB as usize));
    let part_count: usize = 2;
    let base_chunk_size = replacement.len() / part_count;
    let remainder = replacement.len() % part_count;
    let write_chunks: Vec<&[u8]> = (0..part_count)
        .map(|i| {
            let start = i * base_chunk_size + i.min(remainder);
            let chunk_size = if i < remainder {
                base_chunk_size + 1
            } else {
                base_chunk_size
            };
            &replacement[start..start + chunk_size]
        })
        .collect();

    let prepare = client
        .write(
            WriteParameters {
                native_metadata: Default::default(),
                destination_resource_address: file.clone(),
                previous_version: None,
                fail_if_exists: Some(true),
                data_object_size: replacement.len() as u64,
                upload_preference: Some(UploadPreference::Multipart.into()),
            },
            None,
        )
        .await
        .unwrap();
    let write_response::WriteResponseType::MultipartUpload(prepare) = prepare.write_response_type.unwrap() else {
        panic!("Response should be MultipartUpload");
    };
    let redirect = prepare.first_part_write_redirect.unwrap();

    let mut all_headers = Vec::new();
    all_headers.push(
        perform_upload(
            write_chunks[0],
            None,
            redirect.method,
            &redirect.redirect_target_url,
            redirect.additional_headers.clone(),
            &redirect.completion_header_names,
        )
        .await
        .unwrap(),
    );
    let part_response = client
        .upload_part(UploadPartRequest {
            upload_id: prepare.upload_id.clone(),
            destination_resource_address: file.clone(),
            part_number: 1,
            part_count: Some((part_count - 1) as u32),
        })
        .await
        .unwrap();
    for (idx, part) in part_response.part_write_redirects.into_iter().enumerate() {
        all_headers.push(
            perform_upload(
                write_chunks[idx + 1],
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

    let complete = client
        .complete_multipart_upload(CompleteMultipartUploadRequest {
            upload_id: prepare.upload_id,
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

    assert_eq!(complete.unwrap_err().code(), Code::AlreadyExists);
    validate_file(&file, &original, &mut client).await;

    temp_dir.cleanup().await;
}

// Basic assume role test
#[rstest]
#[case::case_01_empty_file("unittests/stat/empty.txt", 0)]
#[case::case_02_lorem_ipsum("unittests/readFile/Lorem ipsum.txt", 3549)]
#[test_log::test(tokio::test)]
async fn assume_role(
    #[from(assume_role_storage_fixture)] storage_fixture: &StorageFixture,
    #[values(Backend::S3)] backend: Backend,
    #[values(Interface::Grpc)] interface: Interface,
    #[values(Access::Public, Access::PrivateKey)] access: Access,
    #[values(ApiVersion::V1Alpha)] api_version: ApiVersion,
    #[case] base_address: &str,
    #[case] size: u64,
) {
    stat_impl(
        storage_fixture,
        backend,
        interface,
        access,
        api_version,
        (Cache::Disabled, 1),
        base_address,
        size,
    )
    .await;
}

#[rstest]
#[case::case_01_empty_file("unittests/stat/empty.txt")]
#[case::case_02_lorem_ipsum("unittests/readFile/Lorem ipsum.txt")]
#[test_log::test(tokio::test)]
async fn assume_role_permission_denied(
    #[from(assume_role_storage_fixture)] storage_fixture: &StorageFixture,
    #[values(Backend::S3)] backend: Backend,
    #[values(Interface::Grpc)] interface: Interface,
    #[values(Access::Versioned)] access: Access,
    #[values(ApiVersion::V1Alpha)] api_version: ApiVersion,
    #[case] base_address: &str,
) {
    // versioned bucket should return a permission denied error
    // as the role does not have access to the versioned bucket
    stat_permission_denied_impl(
        storage_fixture,
        backend,
        interface,
        access,
        api_version,
        (Cache::Disabled, 1),
        base_address,
    )
    .await;
}

#[rstest]
#[test_log::test(tokio::test)]
async fn get_native_metadata_support(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure, Backend::Minio, Backend::GCPS3Compat)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(Cache::Disabled)] cache: Cache,
) {
    let api_version = ApiVersion::V1Alpha;
    let mut client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;

    let Some(temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, api_version)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    // The RPC must reach the backend through the full layer stack over both gRPC and REST
    // (not return UNIMPLEMENTED or 404). The test fixtures do not enable native metadata, so
    // every backend reports it unsupported by default.
    let response = client
        .get_native_metadata_support(GetNativeMetadataSupportRequest {
            resource_address: temp_dir.new_file(),
        })
        .await
        .unwrap();

    assert!(!response.supports_native_metadata);
}

// ===================== native-metadata round-trip =====================
// Native-metadata *writes* are rejected under coherent-false, so seed backend metadata out of band via
// the backend SDK, then assert it surfaces through the native_metadata_enabled runners on
// stat / read_from_address / read-by-identity / list_stat. S3 uses the local minio bucket; Azure the
// versioned test account (whose storage key the harness provides).

/// Asserts every (key -> string value) in `expected` is present in `native` as a StringValue.
fn assert_native_metadata_contains(
    native: &HashMap<String, storage_core_protos::google::protobuf::Value>,
    expected: &[(&str, &str)],
) {
    use storage_core_protos::google::protobuf::value::Kind;
    for (k, v) in expected {
        let got = native
            .get(*k)
            .unwrap_or_else(|| panic!("native_metadata missing key {k}; got {native:?}"));
        assert_matches!(&got.kind, Some(Kind::StringValue(s)) if s == v, "native_metadata[{k}] mismatch");
    }
}

/// Seed `address` with backend metadata out of band. Returns false (soft-skip) when the backend's seed
/// credentials are not present in the environment.
async fn seed_native_metadata(backend: Backend, address: &str, metadata: &[(&str, &str)], body: &[u8]) -> bool {
    let url = url::Url::parse(address).expect("seed address must parse");
    match backend {
        Backend::Minio | Backend::S3 | Backend::GCPS3Compat => {
            let (Ok(access_key), Ok(secret_key)) = (
                std::env::var("OMNI_STORAGE_SERVICE_MINIO_ACCESS_KEY_ID"),
                std::env::var("OMNI_STORAGE_SERVICE_MINIO_SECRET_ACCESS_KEY"),
            ) else {
                return false;
            };
            let region = std::env::var("OMNI_STORAGE_SERVICE_MINIO_REGION").unwrap_or_else(|_| "us-east-1".to_string());
            let mut endpoint = format!("{}://{}", url.scheme(), url.host_str().unwrap());
            if let Some(port) = url.port() {
                endpoint = format!("{endpoint}:{port}");
            }
            // Path-style: the first path segment is the bucket, the rest is the key.
            let mut segments = url.path_segments().expect("seed url has a path");
            let bucket = segments.next().expect("seed url has a bucket").to_string();
            let key = segments.collect::<Vec<_>>().join("/");
            let creds = seed_s3::config::Credentials::new(access_key, secret_key, None, None, "native-metadata-seed");
            let conf = seed_s3::Config::builder()
                .behavior_version(seed_s3::config::BehaviorVersion::latest())
                .region(seed_s3::config::Region::new(region))
                .endpoint_url(endpoint)
                .credentials_provider(creds)
                .force_path_style(true)
                .build();
            let client = seed_s3::Client::from_conf(conf);
            let mut put = client.put_object().bucket(bucket).key(key).body(body.to_vec().into());
            for (k, v) in metadata {
                put = put.metadata(*k, *v);
            }
            put.send().await.expect("seed put_object");
            true
        },
        Backend::Azure => {
            let Ok(key) = std::env::var("OMNI_STORAGE_SERVICE_AZURE_STORAGE_KEY_VERSIONED") else {
                return false;
            };
            let host = url.host_str().expect("azure seed url host"); // <account>.blob.core.windows.net
            let account = host.split('.').next().expect("azure account").to_string();
            let mut segments = url.path_segments().expect("seed url has a path");
            let container = segments.next().expect("azure container").to_string();
            let blob = segments.collect::<Vec<_>>().join("/");
            let creds = azure_storage::StorageCredentials::access_key(account.clone(), key);
            let blob_client = AzureClientBuilder::new(account, creds)
                .blob_service_client()
                .container_client(container)
                .blob_client(blob);
            let mut md = azure_core::request_options::Metadata::new();
            for (k, v) in metadata {
                md.insert(k.to_string(), v.to_string());
            }
            blob_client
                .put_block_blob(body.to_vec())
                .metadata(md)
                .await
                .expect("seed put_block_blob");
            true
        },
    }
}

#[rstest]
#[test_log::test(tokio::test)]
async fn native_metadata_round_trip(
    storage_fixture: &StorageFixture,
    #[values(Backend::Minio, Backend::Azure)] backend: Backend,
) {
    let interface = Interface::Grpc;
    let api_version = ApiVersion::V1Alpha;
    // Azure seeds the Versioned account (whose storage key the harness holds); S3 the public minio bucket.
    let access = match backend {
        Backend::Azure => Access::Versioned,
        _ => Access::Public,
    };
    let suffix: String = random_data(8).iter().map(|b| format!("{b:02x}")).collect();
    let path = format!("native-metadata-roundtrip/{suffix}.bin");
    let Some(address) = storage_fixture.make_url(backend, access, &path) else {
        return; // backend not configured in this environment
    };

    let seed = [("assetid", "abc-123"), ("owner", "roundtrip")];
    if !seed_native_metadata(backend, &address, &seed, b"hello native metadata").await {
        return; // seed credentials unavailable; soft-skip
    }

    let mut client = storage_fixture
        .native_metadata_file_object_client(backend, interface, api_version)
        .await;

    // (1) Stat surfaces native metadata.
    let stat = client
        .stat(StatRequest {
            resource_address: address.clone(),
        })
        .await
        .expect("stat");
    let resource_info = stat.resource_info.expect("stat resource_info");
    let identity = resource_info.resource_identity.clone().expect("stat identity");
    assert_native_metadata_contains(&resource_info.metadata.expect("stat metadata").native_metadata, &seed);

    // (2) ReadFromAddress surfaces native metadata (first message is the ResourceInfo).
    let mut rfa = client
        .read_from_address(ReadFromAddressRequest {
            resource_address: address.clone(),
            download_preference: None,
        })
        .await
        .expect("read_from_address");
    let first = rfa.next().await.expect("rfa message").expect("rfa ok");
    if let Some(read_from_address_response::ReplyType::ResourceInfo(ri)) = first.reply_type {
        assert_native_metadata_contains(&ri.metadata.expect("rfa metadata").native_metadata, &seed);
    } else {
        panic!("expected ResourceInfo first from read_from_address, got {first:?}");
    }

    // (3) Read (by identity) surfaces native metadata (first message is the Metadata).
    let mut read = client
        .read(ReadRequest {
            resource_identity: Some(identity.clone()),
            download_preference: None,
        })
        .await
        .expect("read");
    let first = read.next().await.expect("read message").expect("read ok");
    if let Some(read_response::ReplyType::Metadata(metadata)) = first.reply_type {
        assert_native_metadata_contains(&metadata.native_metadata, &seed);
    } else {
        panic!("expected Metadata first from read, got {first:?}");
    }

    // (4) ListStat of the parent folder surfaces native metadata on the seeded entry.
    let mut folder = storage_fixture
        .native_metadata_file_folder_client(backend, interface, api_version)
        .await;
    let folder_uri = address
        .rsplit_once('/')
        .map(|(base, _)| format!("{base}/"))
        .expect("folder uri");
    let mut list = folder
        .list_stat(ListStatRequest {
            folder: Some(FolderAddress { uri: folder_uri }),
            include_native_metadata: true,
        })
        .await
        .expect("list_stat");
    let mut found = false;
    while let Some(page) = list.next().await {
        let page = page.expect("list_stat page");
        for entry in page.entries {
            if entry.resource_address == address {
                let native = entry
                    .resource_info
                    .expect("entry resource_info")
                    .metadata
                    .expect("entry metadata")
                    .native_metadata;
                assert_native_metadata_contains(&native, &seed);
                found = true;
            }
        }
    }
    assert!(found, "seeded object not found in list_stat of its parent folder");

    // Cleanup.
    let _ = client
        .delete(DeleteRequest {
            resource_address: address,
            ..Default::default()
        })
        .await;
}

// Native metadata must survive the read-side caches. A ListStat of the parent folder primes the
// list_cache (the cached listing that later answers Stat/ReadFromAddress) and the small_object_cache
// (the per-object hint that later answers Read). Each of those cache-served paths must still surface
// the seeded backend metadata rather than an empty map.
#[rstest]
#[test_log::test(tokio::test)]
async fn native_metadata_cache_round_trip(
    storage_fixture: &StorageFixture,
    #[values(Backend::Minio, Backend::Azure)] backend: Backend,
    // The priming ListStat is run both with and without `include_native_metadata`: the caches must
    // publish complete records either way, since a Read served from a cache-published hint is answered
    // from it without a further HEAD/GetProperties.
    #[values(true, false)] prime_include_native_metadata: bool,
) {
    let interface = Interface::Grpc;
    let api_version = ApiVersion::V1Alpha;
    let access = match backend {
        Backend::Azure => Access::Versioned,
        _ => Access::Public,
    };
    let suffix: String = random_data(8).iter().map(|b| format!("{b:02x}")).collect();
    let path = format!("native-metadata-cache/{suffix}.bin");
    let Some(address) = storage_fixture.make_url(backend, access, &path) else {
        return; // backend not configured in this environment
    };

    let seed = [("assetid", "abc-123"), ("owner", "roundtrip")];
    if !seed_native_metadata(backend, &address, &seed, b"hello native metadata").await {
        return; // seed credentials unavailable; soft-skip
    }

    let folder_uri = address
        .rsplit_once('/')
        .map(|(base, _)| format!("{base}/"))
        .expect("folder uri");

    // Prime the caches: a ListStat of the parent folder populates both the list_cache (the listing that
    // later answers Stat) and the small_object_cache (the per-object hint that later answers Read). The
    // listing itself must already carry native metadata, and the cached copy must preserve it.
    let mut folder = storage_fixture
        .native_metadata_cache_file_folder_client(backend, interface, api_version)
        .await;
    let mut list = folder
        .list_stat(ListStatRequest {
            folder: Some(FolderAddress { uri: folder_uri }),
            include_native_metadata: prime_include_native_metadata,
        })
        .await
        .expect("list_stat prime");
    let mut primed = false;
    while let Some(page) = list.next().await {
        let page = page.expect("list_stat page");
        for entry in page.entries {
            if entry.resource_address == address {
                let native = entry
                    .resource_info
                    .expect("entry resource_info")
                    .metadata
                    .expect("entry metadata")
                    .native_metadata;
                assert_native_metadata_contains(&native, &seed);
                primed = true;
            }
        }
    }
    assert!(primed, "seeded object not found while priming the caches");

    let mut client = storage_fixture
        .native_metadata_cache_file_object_client(backend, interface, api_version)
        .await;

    // (1) Stat is answered from the cached parent-folder listing (list_cache).
    let stat = client
        .stat(StatRequest {
            resource_address: address.clone(),
        })
        .await
        .expect("stat");
    let resource_info = stat.resource_info.expect("stat resource_info");
    let identity = resource_info.resource_identity.clone().expect("stat identity");
    assert_native_metadata_contains(&resource_info.metadata.expect("stat metadata").native_metadata, &seed);

    // (2) ReadFromAddress reuses the cached ResourceInfo the list_cache injects into the request.
    let mut rfa = client
        .read_from_address(ReadFromAddressRequest {
            resource_address: address.clone(),
            download_preference: None,
        })
        .await
        .expect("read_from_address");
    let first = rfa.next().await.expect("rfa message").expect("rfa ok");
    if let Some(read_from_address_response::ReplyType::ResourceInfo(ri)) = first.reply_type {
        assert_native_metadata_contains(&ri.metadata.expect("rfa metadata").native_metadata, &seed);
    } else {
        panic!("expected ResourceInfo first from read_from_address, got {first:?}");
    }

    // (3) Read-by-identity is answered from the small_object_cache hint the ListStat published.
    let mut read = client
        .read(ReadRequest {
            resource_identity: Some(identity),
            download_preference: None,
        })
        .await
        .expect("read");
    let first = read.next().await.expect("read message").expect("read ok");
    if let Some(read_response::ReplyType::Metadata(metadata)) = first.reply_type {
        assert_native_metadata_contains(&metadata.native_metadata, &seed);
    } else {
        panic!("expected Metadata first from read, got {first:?}");
    }

    // Cleanup.
    let _ = client
        .delete(DeleteRequest {
            resource_address: address,
            ..Default::default()
        })
        .await;
}

// The small_object_cache publishes a per-object hint for every entry its own `list_stat` returns, and
// v1alpha `read` answers from that hint without a further HEAD. S3 gates its native-metadata fan-out on
// the request's `include_native_metadata`, so a caller that leaves the flag unset (the proto default)
// must not be able to publish an incomplete hint: a Read of a listed object has to return the same
// metadata a direct Read returns. Runs against a runner with `list_cache` disabled, so the hint comes
// from the SOC's own listing rather than from the list_cache above it (which always asks for metadata).
#[rstest]
#[test_log::test(tokio::test)]
async fn native_metadata_small_object_cache_hint_ignores_caller_flag(storage_fixture: &StorageFixture) {
    let interface = Interface::Grpc;
    let api_version = ApiVersion::V1Alpha;
    let backend = Backend::Minio;
    let suffix: String = random_data(8).iter().map(|b| format!("{b:02x}")).collect();
    let path = format!("native-metadata-soc/{suffix}.bin");
    let Some(address) = storage_fixture.make_url(backend, Access::Public, &path) else {
        return; // backend not configured in this environment
    };

    let seed = [("assetid", "abc-123"), ("owner", "roundtrip")];
    if !seed_native_metadata(backend, &address, &seed, b"hello native metadata").await {
        return; // seed credentials unavailable; soft-skip
    }

    let folder_uri = address
        .rsplit_once('/')
        .map(|(base, _)| format!("{base}/"))
        .expect("folder uri");

    // Prime the small_object_cache with `include_native_metadata` unset, exactly as a default client
    // would. The identity is taken from the listing itself: a Stat would go to the backend and publish a
    // complete hint of its own, masking the one this test is about.
    let mut folder = storage_fixture
        .native_metadata_soc_only_file_folder_client(interface, api_version)
        .await;
    let mut list = folder
        .list_stat(ListStatRequest {
            folder: Some(FolderAddress { uri: folder_uri }),
            include_native_metadata: false,
        })
        .await
        .expect("list_stat prime");
    let mut identity = None;
    while let Some(page) = list.next().await {
        let page = page.expect("list_stat page");
        for entry in page.entries {
            if entry.resource_address == address {
                identity = entry.resource_info.expect("entry resource_info").resource_identity;
            }
        }
    }
    let identity = identity.expect("seeded object not found while priming the small_object_cache");

    // The hint is published off the listing stream on a spawned task; give it a moment to land so the
    // Read below is actually answered from it.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let mut client = storage_fixture
        .native_metadata_soc_only_file_object_client(interface, api_version)
        .await;
    let mut read = client
        .read(ReadRequest {
            resource_identity: Some(identity),
            download_preference: None,
        })
        .await
        .expect("read");
    let first = read.next().await.expect("read message").expect("read ok");
    if let Some(read_response::ReplyType::Metadata(metadata)) = first.reply_type {
        assert_native_metadata_contains(&metadata.native_metadata, &seed);
    } else {
        panic!("expected Metadata first from read, got {first:?}");
    }

    // Cleanup.
    let _ = client
        .delete(DeleteRequest {
            resource_address: address,
            ..Default::default()
        })
        .await;
}

#[rstest]
#[test_log::test(tokio::test)]
async fn write_rejects_too_short_payloads(
    storage_fixture: &StorageFixture,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    let backend = Backend::Minio;
    let cache = Cache::Disabled;
    let api_version = ApiVersion::V1Alpha;
    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, Access::PrivateKey, cache, api_version)
        .await
    else {
        debug!("Skipping test: MinIO private-key bucket unavailable");
        return;
    };

    let mut client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;
    let file = temp_dir.new_file();

    let error = client
        .write(
            WriteParameters {
                native_metadata: Default::default(),
                destination_resource_address: file,
                previous_version: None,
                fail_if_exists: None,
                data_object_size: 8,
                upload_preference: Some(UploadPreference::Body.into()),
            },
            Some(Bytes::from_static(b"abcd")),
        )
        .await
        .expect_err("a body shorter than the declared size should be rejected");
    assert_eq!(error.code(), Code::InvalidArgument);

    temp_dir.cleanup().await;
}

#[rstest]
#[test_log::test(tokio::test)]
async fn write_truncates_too_long_payloads(
    storage_fixture: &StorageFixture,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    let backend = Backend::Minio;
    let cache = Cache::Disabled;
    let api_version = ApiVersion::V1Alpha;
    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, Access::PrivateKey, cache, api_version)
        .await
    else {
        debug!("Skipping test: MinIO private-key bucket unavailable");
        return;
    };

    let mut client = storage_fixture
        .file_object_client(backend, cache, interface, api_version)
        .await;
    let file = temp_dir.new_file();

    client
        .write(
            WriteParameters {
                native_metadata: Default::default(),
                destination_resource_address: file.clone(),
                previous_version: None,
                fail_if_exists: None,
                data_object_size: 4,
                upload_preference: Some(UploadPreference::Body.into()),
            },
            Some(Bytes::from_static(b"abcdefgh")),
        )
        .await
        .expect("a body longer than the declared size should be truncated");

    validate_file(&file, b"abcd", &mut client).await;
    temp_dir.cleanup().await;
}
