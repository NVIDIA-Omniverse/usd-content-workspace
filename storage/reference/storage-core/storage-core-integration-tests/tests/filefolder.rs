// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

#![allow(clippy::too_many_arguments)]

use std::collections::HashSet;

use fred::interfaces::KeysInterface;
use futures::stream::StreamExt;
use percent_encoding::percent_decode_str;
use rstest::rstest;
use tonic::Code;
use tracing::debug;
use url::Url;

use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};

use storage_core::traits::filefolder::v1alpha::*;
use storage_core::traits::fileobject::v1alpha::{DownloadPreference, ReadFromAddressRequest, StatRequest};
use storage_core_integration_tests::fixture::*;
use storage_core_integration_tests::utils::*;
use storage_core_integration_tests::{assert_method_metric, storage_fixture};
use storage_core_test::{
    clients::traits::{filefolder::FileFolder as TestFileFolder, fileobject::FileObject as TestFileObject},
    service::{ApiVersion, Interface},
};
use storage_core_util::url_path::canonicalize_url_path;

fn canonical_url_spelling(value: &str) -> String {
    let mut url = Url::parse(value).expect("resource address should be a valid URL");
    canonicalize_url_path(&mut url).expect("resource address path should be canonicalizable");
    url.to_string()
}

#[rstest]
#[case::case_01_s3_medium(
    Backend::S3,
    "unittests/list/medium", 
    vec!["unittests/list/medium/folder-0000/", "unittests/list/medium/folder-0001/", "unittests/list/medium/folder-0002/"],
    vec!["unittests/list/medium/file-0000.txt", "unittests/list/medium/file-0001.txt"]
)]
#[case::case_02_azure_medium(
    Backend::Azure,
    "unittests/list/medium", 
    vec!["unittests/list/medium/folder-0000/", "unittests/list/medium/folder-0001/", "unittests/list/medium/folder-0002/"],
    vec!["unittests/list/medium/file-0000.txt", "unittests/list/medium/file-0001.txt"]
)]
#[case::case_03_minio_medium(
    Backend::Minio,
    "unittests/list/medium", 
    vec!["unittests/list/medium/folder-0000/", "unittests/list/medium/folder-0001/", "unittests/list/medium/folder-0002/"],
    vec!["unittests/list/medium/file-0000.txt", "unittests/list/medium/file-0001.txt"]
)]
#[case::case_04_gcp_medium(
    Backend::GCPS3Compat,
    "unittests/list/medium", 
    vec!["unittests/list/medium/folder-0000/", "unittests/list/medium/folder-0001/", "unittests/list/medium/folder-0002/"],
    vec!["unittests/list/medium/file-0000.txt", "unittests/list/medium/file-0001.txt"]
)]
#[case::case_05_s3_medium_trailing_slash(
    Backend::S3,
    "unittests/list/medium/", 
    vec!["unittests/list/medium/folder-0000/", "unittests/list/medium/folder-0001/", "unittests/list/medium/folder-0002/"],
    vec!["unittests/list/medium/file-0000.txt", "unittests/list/medium/file-0001.txt"]
)]
#[case::case_06_azure_medium_trailing_slash(
    Backend::Azure,
    "unittests/list/medium/", 
    vec!["unittests/list/medium/folder-0000/", "unittests/list/medium/folder-0001/", "unittests/list/medium/folder-0002/"],
    vec!["unittests/list/medium/file-0000.txt", "unittests/list/medium/file-0001.txt"]
)]
#[case::case_07_minio_medium_trailing_slash(
    Backend::Minio,
    "unittests/list/medium/", 
    vec!["unittests/list/medium/folder-0000/", "unittests/list/medium/folder-0001/", "unittests/list/medium/folder-0002/"],
    vec!["unittests/list/medium/file-0000.txt", "unittests/list/medium/file-0001.txt"]
)]
#[case::case_08_gcp_medium_trailing_slash(
    Backend::GCPS3Compat,
    "unittests/list/medium/", 
    vec!["unittests/list/medium/folder-0000/", "unittests/list/medium/folder-0001/", "unittests/list/medium/folder-0002/"],
    vec!["unittests/list/medium/file-0000.txt", "unittests/list/medium/file-0001.txt"]
)]
// Minio doesn't support the full special character set that S3 does
#[case::case_09_s3_specialchars_root(
    Backend::S3,
    "unittests/specialchars",
    vec![
        "unittests/specialchars/folder%20%09~!@%23$%^&*()_%2B%601234567890-=[]%7B%7D|:;%22'%3C%3E,%3F%5C/",
    ],
    vec![
        "unittests/specialchars/french_accents_%C3%A2%C3%A0%C3%A4%C3%A7%C3%A9%C3%A8%C3%AA%C3%AB%C3%AE%C3%AF%C3%B4%C3%B9%C3%BB%C3%BC%C5%93.txt",
        "unittests/specialchars/spanish_accents_%C3%A1%C3%A9%C3%AD%C3%B3%C3%BA%C3%BC%C3%B1%C2%BF%C2%A1.txt",
        "unittests/specialchars/test%20with%20plus%20%2B%20for%20s3.txt",
        "unittests/specialchars/test%2Bfile%20with%20spaces.txt",
        // Object name with a trailing space (see Azure case for context).
        "unittests/specialchars/trailing_whitespace_probe%20",
    ],
)]
#[case::case_10_azure_specialchars_root(
    Backend::Azure,
    "unittests/specialchars",
    vec![
        "unittests/specialchars/folder%20~!@%23$%^&*()_%2B%601234567890-=[]%7B%7D|:;%22'%3C%3E,%3F/",
    ],
    vec![
        "unittests/specialchars/french_accents_%C3%A2%C3%A0%C3%A4%C3%A7%C3%A9%C3%A8%C3%AA%C3%AB%C3%AE%C3%AF%C3%B4%C3%B9%C3%BB%C3%BC%C5%93.txt",
        "unittests/specialchars/spanish_accents_%C3%A1%C3%A9%C3%AD%C3%B3%C3%BA%C3%BC%C3%B1%C2%BF%C2%A1.txt",
        "unittests/specialchars/test%20with%20plus%20%2B%20for%20s3.txt",
        "unittests/specialchars/test%2Bfile%20with%20spaces.txt",
        // Blob name with a trailing space — quick-xml's deserializer used to
        // strip it, so list dropped/garbled the entry. The %20 must survive end-to-end.
        "unittests/specialchars/trailing_whitespace_probe%20",
    ],
)]
#[case::case_11_gcp_specialchars_root(
    Backend::GCPS3Compat,
    "unittests/specialchars",
    vec![
        "unittests/specialchars/folder%20%09~!@%23$%^&*()_%2B%601234567890-=[]%7B%7D|:;%22'%3C%3E,%3F%5C/",
    ],
    vec![
        "unittests/specialchars/french_accents_%C3%A2%C3%A0%C3%A4%C3%A7%C3%A9%C3%A8%C3%AA%C3%AB%C3%AE%C3%AF%C3%B4%C3%B9%C3%BB%C3%BC%C5%93.txt",
        "unittests/specialchars/spanish_accents_%C3%A1%C3%A9%C3%AD%C3%B3%C3%BA%C3%BC%C3%B1%C2%BF%C2%A1.txt",
        "unittests/specialchars/test%20with%20plus%20%2B%20for%20s3.txt",
        "unittests/specialchars/test%2Bfile%20with%20spaces.txt",
        // Object name with a trailing space (see Azure case for context).
        "unittests/specialchars/trailing_whitespace_probe%20",
    ],
)]
#[case::case_12_s3_specialchars_folder(
    Backend::S3,
    "unittests/specialchars/folder%20%09~!@%23$%^&*()_%2B%601234567890-=[]%7B%7D|:;%22'%3C%3E,%3F%5C/",
    vec![],
    vec![
        "unittests/specialchars/folder%20%09~!@%23$%^&*()_%2B%601234567890-=[]%7B%7D|:;%22'%3C%3E,%3F%5C/test%20with%20plus%20%2B%20for%20s3.txt",
    ],
)]
#[case::case_13_azure_specialchars_folder(
    Backend::Azure,
    "unittests/specialchars/folder%20~!@%23$%^&*()_%2B%601234567890-=[]%7B%7D|:;%22'%3C%3E,%3F/",
    vec![],
    vec![
        "unittests/specialchars/folder%20~!@%23$%^&*()_%2B%601234567890-=[]%7B%7D|:;%22'%3C%3E,%3F/test%20with%20plus%20%2B%20for%20s3.txt",
    ],
)]
#[case::case_14_gcp_specialchars_folder(
    Backend::GCPS3Compat,
    "unittests/specialchars/folder%20%09~!@%23$%^&*()_%2B%601234567890-=[]%7B%7D|:;%22'%3C%3E,%3F%5C/",
    vec![],
    vec![
        "unittests/specialchars/folder%20%09~!@%23$%^&*()_%2B%601234567890-=[]%7B%7D|:;%22'%3C%3E,%3F%5C/test%20with%20plus%20%2B%20for%20s3.txt",
    ],
)]
// Minio can't list the full specialchars folder (it can't stat the tab/backslash
// siblings — see the stat_full Minio skip), so the trailing-space fixture is exercised via a
// dedicated clean subfolder that contains only the probe.
#[case::case_15_minio_specialchars_whitespace(
    Backend::Minio,
    "unittests/specialchars/whitespace",
    vec![],
    vec![
        "unittests/specialchars/whitespace/trailing_whitespace_probe%20",
    ],
)]
#[test_log::test(tokio::test)]
async fn list(
    storage_fixture: &StorageFixture,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::Public, Access::PrivateKey)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[case] backend: Backend,
    #[case] base_address: &str,
    #[case] expected_subfolders: Vec<&str>,
    #[case] expected_objects: Vec<&str>,
) {
    let Some(base_address) = storage_fixture.make_url(backend, access, base_address) else {
        debug!("Skipping test for {:?} {:?} {:?}", backend, access, base_address);
        return;
    };
    let expected_folders: Vec<_> = expected_subfolders
        .iter()
        .map(|name| storage_fixture.make_url(backend, access, name).unwrap())
        .collect();
    let expected_objects: Vec<_> = expected_objects
        .iter()
        .map(|name| storage_fixture.make_url(backend, access, name).unwrap())
        .collect();

    let mut client = storage_fixture
        .file_folder_client(backend, Cache::Disabled, interface, api_version)
        .await;

    let request = ListRequest {
        folder: Some(FolderAddress {
            uri: base_address.to_string(),
        }),
    };
    debug!(?request);
    let response_stream = client.list(request).await.unwrap();

    let responses: Vec<_> = response_stream
        .filter_map(|response| async { response.ok() })
        .collect()
        .await;
    debug!(?responses);

    let objects: HashSet<_> = responses
        .iter()
        .flat_map(|response| response.sub_resource_addresses.clone())
        .collect();
    debug!(?objects);

    let folders: HashSet<_> = responses
        .iter()
        .flat_map(|response| response.subfolder_addresses.iter())
        .map(|address| address.uri.clone())
        .collect();
    debug!(?folders);

    let missing_objects: Vec<_> = expected_objects
        .into_iter()
        .map(|address| canonical_url_spelling(&address))
        .filter(|address| !objects.contains(address))
        .collect();
    debug!(?missing_objects);
    assert_eq!(missing_objects, Vec::<String>::new());

    let missing_folders: Vec<_> = expected_folders
        .into_iter()
        .map(|address| canonical_url_spelling(&address))
        .filter(|address| !folders.contains(address))
        .collect();
    debug!(?missing_folders);
    assert_eq!(missing_folders, Vec::<String>::new());

    // Ensure we can stat the resource addresses returned by the call.
    let mut file_object_client = storage_fixture
        .file_object_client(backend, Cache::Disabled, interface, api_version)
        .await;
    for resource_address in &objects {
        let resource_identity = file_object_client
            .stat(StatRequest {
                resource_address: resource_address.clone(),
            })
            .await
            .unwrap()
            .resource_info
            .unwrap()
            .resource_identity
            .unwrap()
            .encoded_identity;

        assert!(resource_identity.starts_with(resource_address));
    }

    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.requests",
        "list",
        ["result" => "Ok"]
    );
    assert_method_metric!(storage_fixture, backend, api_version, "storage.sdk.requests", "list", ["result" => "Ok"]);
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.request.duration",
        "list",
        [],
        1
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.sdk.request.duration",
        "list",
        [],
        1
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.enumeration.items",
        "list",
        [],
        1
    );
}

#[rstest]
#[case::case_01_s3_medium(
    Backend::S3,
    "unittests/list/medium",
    vec!["unittests/list/medium/folder-0000/", "unittests/list/medium/folder-0001/", "unittests/list/medium/folder-0002/"],
    vec![
        ("unittests/list/medium/file-0001.txt", 48),
        ("unittests/list/medium/file-0000.txt", 0),
    ],
)]
#[case::case_02_azure_medium(
    Backend::Azure,
    "unittests/list/medium",
    vec!["unittests/list/medium/folder-0000/", "unittests/list/medium/folder-0001/", "unittests/list/medium/folder-0002/"],
    vec![
        ("unittests/list/medium/file-0001.txt", 48),
        ("unittests/list/medium/file-0000.txt", 0),
    ],
)]
#[case::case_03_minio_medium(
    Backend::Minio,
    "unittests/list/medium",
    vec!["unittests/list/medium/folder-0000/", "unittests/list/medium/folder-0001/", "unittests/list/medium/folder-0002/"],
    vec![
        ("unittests/list/medium/file-0001.txt", 48),
        ("unittests/list/medium/file-0000.txt", 0),
    ],
)]
#[case::case_04_gcp_medium(
    Backend::GCPS3Compat,
    "unittests/list/medium",
    vec!["unittests/list/medium/folder-0000/", "unittests/list/medium/folder-0001/", "unittests/list/medium/folder-0002/"],
    vec![
        ("unittests/list/medium/file-0001.txt", 48),
        ("unittests/list/medium/file-0000.txt", 0),
    ],
)]
#[case::case_05_s3_medium_trailing_slash(
    Backend::S3,
    "unittests/list/medium/",
    vec!["unittests/list/medium/folder-0000/", "unittests/list/medium/folder-0001/", "unittests/list/medium/folder-0002/"],
    vec![
        ("unittests/list/medium/file-0001.txt", 48),
        ("unittests/list/medium/file-0000.txt", 0),
    ],
)]
#[case::case_06_azure_medium_trailing_slash(
    Backend::Azure,
    "unittests/list/medium/",
    vec!["unittests/list/medium/folder-0000/", "unittests/list/medium/folder-0001/", "unittests/list/medium/folder-0002/"],
    vec![
        ("unittests/list/medium/file-0001.txt", 48),
        ("unittests/list/medium/file-0000.txt", 0),
    ],
)]
#[case::case_07_minio_medium_trailing_slash(
    Backend::Minio,
    "unittests/list/medium/",
    vec!["unittests/list/medium/folder-0000/", "unittests/list/medium/folder-0001/", "unittests/list/medium/folder-0002/"],
    vec![
        ("unittests/list/medium/file-0001.txt", 48),
        ("unittests/list/medium/file-0000.txt", 0),
    ],
)]
#[case::case_08_gcp_medium_trailing_slash(
    Backend::GCPS3Compat,
    "unittests/list/medium/",
    vec!["unittests/list/medium/folder-0000/", "unittests/list/medium/folder-0001/", "unittests/list/medium/folder-0002/"],
    vec![
        ("unittests/list/medium/file-0001.txt", 48),
        ("unittests/list/medium/file-0000.txt", 0),
    ],
)]
#[case::case_09_s3_specialchars_root(
    Backend::S3,
    "unittests/specialchars",
    vec![
        "unittests/specialchars/folder%20%09~!@%23$%^&*()_%2B%601234567890-=[]%7B%7D|:;%22'%3C%3E,%3F%5C/",
    ],
    vec![
        ("unittests/specialchars/french_accents_%C3%A2%C3%A0%C3%A4%C3%A7%C3%A9%C3%A8%C3%AA%C3%AB%C3%AE%C3%AF%C3%B4%C3%B9%C3%BB%C3%BC%C5%93.txt", 0),
        ("unittests/specialchars/spanish_accents_%C3%A1%C3%A9%C3%AD%C3%B3%C3%BA%C3%BC%C3%B1%C2%BF%C2%A1.txt", 0),
        ("unittests/specialchars/test%20with%20plus%20%2B%20for%20s3.txt", 48),
        ("unittests/specialchars/test%2Bfile%20with%20spaces.txt", 30),
        // Object name with a trailing space (see Azure case for context).
        ("unittests/specialchars/trailing_whitespace_probe%20", 53),
    ],
)]
#[case::case_10_azure_specialchars_root(
    Backend::Azure,
    "unittests/specialchars",
    vec![
        "unittests/specialchars/folder%20~!@%23$%^&*()_%2B%601234567890-=[]%7B%7D|:;%22'%3C%3E,%3F/",
    ],
    vec![
        ("unittests/specialchars/french_accents_%C3%A2%C3%A0%C3%A4%C3%A7%C3%A9%C3%A8%C3%AA%C3%AB%C3%AE%C3%AF%C3%B4%C3%B9%C3%BB%C3%BC%C5%93.txt", 0),
        ("unittests/specialchars/spanish_accents_%C3%A1%C3%A9%C3%AD%C3%B3%C3%BA%C3%BC%C3%B1%C2%BF%C2%A1.txt", 0),
        ("unittests/specialchars/test%20with%20plus%20%2B%20for%20s3.txt", 48),
        ("unittests/specialchars/test%2Bfile%20with%20spaces.txt", 30),
        // Object name with a trailing space (see Azure case for context).
        ("unittests/specialchars/trailing_whitespace_probe%20", 53),
    ],
)]
#[case::case_11_gcp_specialchars_root(
    Backend::GCPS3Compat,
    "unittests/specialchars",
    vec![
        "unittests/specialchars/folder%20%09~!@%23$%^&*()_%2B%601234567890-=[]%7B%7D|:;%22'%3C%3E,%3F%5C/",
    ],
    vec![
        ("unittests/specialchars/french_accents_%C3%A2%C3%A0%C3%A4%C3%A7%C3%A9%C3%A8%C3%AA%C3%AB%C3%AE%C3%AF%C3%B4%C3%B9%C3%BB%C3%BC%C5%93.txt", 0),
        ("unittests/specialchars/spanish_accents_%C3%A1%C3%A9%C3%AD%C3%B3%C3%BA%C3%BC%C3%B1%C2%BF%C2%A1.txt", 0),
        ("unittests/specialchars/test%20with%20plus%20%2B%20for%20s3.txt", 48),
        ("unittests/specialchars/test%2Bfile%20with%20spaces.txt", 30),
        // Object name with a trailing space (see Azure case for context).
        ("unittests/specialchars/trailing_whitespace_probe%20", 53),
    ],
)]
#[case::case_12_s3_specialchars_folder(
    Backend::S3,
    "unittests/specialchars/folder%20%09~!@%23$%^&*()_%2B%601234567890-=[]%7B%7D|:;%22'%3C%3E,%3F%5C/",
    vec![],
    vec![
        ("unittests/specialchars/folder%20%09~!@%23$%^&*()_%2B%601234567890-=[]%7B%7D|:;%22'%3C%3E,%3F%5C/test%20with%20plus%20%2B%20for%20s3.txt", 48),
    ],
)]
#[case::case_13_azure_specialchars_folder(
    Backend::Azure,
    "unittests/specialchars/folder%20~!@%23$%^&*()_%2B%601234567890-=[]%7B%7D|:;%22'%3C%3E,%3F/",
    vec![],
    vec![
        ("unittests/specialchars/folder%20~!@%23$%^&*()_%2B%601234567890-=[]%7B%7D|:;%22'%3C%3E,%3F/test%20with%20plus%20%2B%20for%20s3.txt", 48),
    ],
)]
#[case::case_14_gcp_specialchars_folder(
    Backend::GCPS3Compat,
    "unittests/specialchars/folder%20%09~!@%23$%^&*()_%2B%601234567890-=[]%7B%7D|:;%22'%3C%3E,%3F%5C/",
    vec![],
    vec![
        ("unittests/specialchars/folder%20%09~!@%23$%^&*()_%2B%601234567890-=[]%7B%7D|:;%22'%3C%3E,%3F%5C/test%20with%20plus%20%2B%20for%20s3.txt", 48),
    ],
)]
// Minio-only clean subfolder for the trailing-space fixture (see list test).
#[case::case_15_minio_specialchars_whitespace(
    Backend::Minio,
    "unittests/specialchars/whitespace",
    vec![],
    vec![
        ("unittests/specialchars/whitespace/trailing_whitespace_probe%20", 53),
    ],
)]
#[test_log::test(tokio::test)]
async fn list_stat(
    storage_fixture: &StorageFixture,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::Public, Access::PrivateKey)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[case] backend: Backend,
    #[case] base_address: &str,
    #[case] expected_subfolders: Vec<&str>,
    #[case] expected_objects: Vec<(&str, u64)>,
) {
    let Some(base_address) = storage_fixture.make_url(backend, access, base_address) else {
        debug!("Skipping test for {:?} {:?} {:?}", backend, access, base_address);
        return;
    };
    let expected_folders: Vec<_> = expected_subfolders
        .iter()
        .map(|name| storage_fixture.make_url(backend, access, name).unwrap())
        .collect();
    let expected_objects: Vec<_> = expected_objects
        .iter()
        .map(|(name, size)| (storage_fixture.make_url(backend, access, name).unwrap(), Some(*size)))
        .collect();

    let mut client = storage_fixture
        .file_folder_client(backend, Cache::Disabled, interface, api_version)
        .await;

    let request = ListStatRequest {
        include_native_metadata: false,
        folder: Some(FolderAddress {
            uri: base_address.to_string(),
        }),
    };
    let response_stream = client.list_stat(request).await.unwrap();

    let responses: Vec<_> = response_stream
        .filter_map(|response| async { response.ok() })
        .collect()
        .await;

    let folders: HashSet<_> = responses
        .iter()
        .flat_map(|response| response.subfolder_addresses.iter())
        .map(|address| address.uri.clone())
        .collect();
    debug!(?folders);

    let objects: HashSet<_> = responses
        .iter()
        .flat_map(|response| response.entries.iter())
        .map(|entry| {
            (
                entry.resource_address.clone(),
                entry
                    .resource_info
                    .as_ref()
                    .and_then(|resource_info| resource_info.metadata.as_ref())
                    .and_then(|metadata| metadata.data_object_size),
            )
        })
        .collect();
    debug!(?objects);

    let decoded_folders: HashSet<_> = folders
        .iter()
        .map(|s| percent_decode_str(s).decode_utf8_lossy().into_owned())
        .collect();
    debug!(?decoded_folders);

    let decoded_objects: HashSet<_> = objects
        .iter()
        .map(|(address, size)| (percent_decode_str(address).decode_utf8_lossy().into_owned(), *size))
        .collect();
    debug!(?decoded_objects);

    let missing_objects: Vec<_> = expected_objects
        .into_iter()
        .filter(|address| {
            let decoded_address = (
                percent_decode_str(&address.0).decode_utf8_lossy().into_owned(),
                address.1,
            );
            !decoded_objects.contains(&decoded_address)
        })
        .collect();
    assert_eq!(missing_objects, Vec::<(String, Option<u64>)>::new());

    let missing_folders: Vec<_> = expected_folders
        .into_iter()
        .filter(|address| {
            let decoded_address = percent_decode_str(address).decode_utf8_lossy().into_owned();
            !decoded_folders.contains(&decoded_address)
        })
        .collect();
    assert_eq!(missing_folders, Vec::<String>::new());

    // Ensure we can stat the resource addresses returned by the service.
    let mut file_object_client = storage_fixture
        .file_object_client(backend, Cache::Disabled, interface, api_version)
        .await;
    for (resource_address, size) in &objects {
        let resource_info = file_object_client
            .stat(StatRequest {
                resource_address: resource_address.clone(),
            })
            .await
            .unwrap()
            .resource_info
            .unwrap();

        assert!(resource_info
            .resource_identity
            .unwrap()
            .encoded_identity
            .starts_with(resource_address));
        assert_eq!(resource_info.metadata.unwrap().data_object_size, *size);
    }

    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.requests",
        "list_stat",
        ["result" => "Ok"]
    );
    assert_method_metric!(storage_fixture, backend, api_version, "storage.sdk.requests", "list_stat", ["result" => "Ok"]);
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.request.duration",
        "list_stat",
        [],
        1
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
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.enumeration.items",
        "list_stat",
        [],
        1
    );
}

// Coverage for list_stat on a versioned bucket.
//
// The implementation switched S3's list_stat from `ListObjectsV2` to
// `ListObjectVersions` so that version-id is populated on the returned
// identities. These tests exercise behavior that only differs on versioned
// buckets:
//   1. Every returned identity carries a non-empty, non-"null" version-id.
//   2. Historical (is_latest == false) versions of the same key are filtered
//      out server-side — list_stat returns one entry per key, not one per
//      version. This is the load-bearing invariant of the Phase 2 rewrite.
//   3. Delete markers do not surface as entries. A key whose latest entry is
//      a delete marker must be absent from list_stat; a key that was deleted
//      and then rewritten must appear (at the new live version).
//
// Azure doesn't return historical versions from its list API by default, so
// item 2 is S3-specific in practice; Azure is still exercised for items 1
// and 3 to confirm parity.
#[rstest]
#[test_log::test(tokio::test)]
async fn list_stat_versioned_filters_historical_and_delete_markers_basic(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc)] interface: Interface,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
) {
    list_stat_versioned_impl(storage_fixture, backend, interface, api_version).await;
}

#[rstest]
#[test_log::test(tokio::test)]
async fn list_stat_versioned_filters_historical_and_delete_markers_full(
    storage_fixture: &StorageFixture,
    // GCPS3Compat must fall back to ListObjectsV2 — it returns `version_id: None`
    // and `is_latest: true` synthetically on every item. We include it here to
    // verify the fallback path stays wired up end-to-end (not just via the
    // OnceLock pre-seed).
    #[values(Backend::S3, Backend::Azure, Backend::GCPS3Compat)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
) {
    list_stat_versioned_impl(storage_fixture, backend, interface, api_version).await;
}

async fn list_stat_versioned_impl(
    storage_fixture: &StorageFixture,
    backend: Backend,
    interface: Interface,
    api_version: ApiVersion,
) {
    use storage_core::traits::fileobject::v1alpha::{DownloadPreference, ReadRequest};
    use storage_core_test::identity::assert_identity_has_object_identifier_parameters;

    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, Access::Versioned, Cache::Disabled, api_version)
        .await
    else {
        debug!("Skipping test — no versioned bucket fixture for {:?}", backend);
        return;
    };

    let mut file_client = storage_fixture
        .file_object_client(backend, Cache::Disabled, interface, api_version)
        .await;
    let mut folder_client = storage_fixture
        .file_folder_client(backend, Cache::Disabled, interface, api_version)
        .await;

    // Establish a test layout inside the temp dir:
    //   live_a — written twice (should appear exactly once, as the latest version)
    //   live_b — written once
    //   deleted — written, then deleted (delete marker should NOT surface)
    //   resurrected — written, deleted, rewritten (latest live version should surface)
    //
    // The temp dir is itself a fresh UUID-keyed subfolder, so no prior listing
    // state can pollute assertions.
    let live_a = temp_dir.new_file();
    let live_b = temp_dir.new_file();
    let deleted = temp_dir.new_file();
    let resurrected = temp_dir.new_file();

    // `live_a`: two writes produces two versions; only the latest must be
    // returned by list_stat. This is the `is_latest == true` filter's actual
    // business requirement.
    write_data_body(&live_a, &random_data(16), None, None, &mut file_client).await;
    write_data_body(&live_a, &random_data(16), None, None, &mut file_client).await;

    write_data_body(&live_b, &random_data(16), None, None, &mut file_client).await;

    write_data_body(&deleted, &random_data(16), None, None, &mut file_client).await;
    storage_core_integration_tests::utils::delete_file(&deleted, None, None, &mut file_client).await;

    write_data_body(&resurrected, &random_data(16), None, None, &mut file_client).await;
    storage_core_integration_tests::utils::delete_file(&resurrected, None, None, &mut file_client).await;
    write_data_body(&resurrected, &random_data(16), None, None, &mut file_client).await;

    // list_stat the temp dir.
    let folder_uri = temp_dir.url().to_string();
    let request = ListStatRequest {
        include_native_metadata: false,
        folder: Some(FolderAddress {
            uri: folder_uri.clone(),
        }),
    };
    let response_stream = folder_client.list_stat(request).await.unwrap();
    let responses: Vec<_> = response_stream.filter_map(|r| async { r.ok() }).collect().await;

    let mut entries: HashSet<String> = HashSet::new();
    for response in &responses {
        for entry in &response.entries {
            entries.insert(entry.resource_address.clone());

            let identity = entry
                .resource_info
                .as_ref()
                .unwrap()
                .resource_identity
                .as_ref()
                .unwrap()
                .encoded_identity
                .clone();

            // GCS's fallback path intentionally lacks version-id (the S3
            // interop endpoint doesn't expose it). Everywhere else we require
            // version-id to be present and well-formed.
            let expect_version_id = backend != Backend::GCPS3Compat;
            assert_identity_has_object_identifier_parameters(&identity, &entry.resource_address, expect_version_id);
        }
    }

    assert!(
        entries.contains(&live_a),
        "list_stat did not return live_a (written twice, should appear exactly once)"
    );
    assert!(entries.contains(&live_b), "list_stat did not return live_b");
    assert!(
        entries.contains(&resurrected),
        "list_stat did not return resurrected (live latest version after delete+rewrite)"
    );
    assert!(
        !entries.contains(&deleted),
        "list_stat unexpectedly returned {} — delete marker leaked through the is_latest filter",
        deleted,
    );
    assert_eq!(
        entries.len(),
        3,
        "list_stat returned unexpected entries: got {:?}, expected exactly [live_a, live_b, resurrected]",
        entries,
    );

    // The identity returned for `live_a` must resolve to the LATEST version's
    // content: read by it and drain the stream to confirm the backend honors
    // the if-match + versionId for the latest write. Pre-Phase-2 a non-is_latest
    // entry could creep in if `ListObjectsV2` semantics were relied on; this
    // guards against regression.
    let live_a_identity = responses
        .iter()
        .flat_map(|r| r.entries.iter())
        .find(|e| e.resource_address == live_a)
        .and_then(|e| e.resource_info.as_ref())
        .and_then(|ri| ri.resource_identity.clone())
        .expect("live_a entry must carry a resource_identity");

    let mut stream = file_client
        .read(ReadRequest {
            resource_identity: Some(live_a_identity),
            download_preference: Some(DownloadPreference::Body.into()),
        })
        .await
        .expect("read-by-identity for live_a (latest version) should succeed");
    while let Some(chunk) = stream.next().await {
        chunk.expect("read stream for live_a (latest version) produced an error");
    }

    temp_dir.cleanup().await;
}

#[rstest]
#[test_log::test(tokio::test)]
async fn list_stat_versioned_deleted_only_folder_returns_not_found(
    storage_fixture: &StorageFixture,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
) {
    let backend = Backend::S3;
    let Some(temp_dir) = storage_fixture
        .temp_directory(backend, Access::Versioned, Cache::Disabled, api_version)
        .await
    else {
        debug!("Skipping test — no versioned bucket fixture for {:?}", backend);
        return;
    };

    let mut file_client = storage_fixture
        .file_object_client(backend, Cache::Disabled, interface, api_version)
        .await;
    let mut folder_client = storage_fixture
        .file_folder_client(backend, Cache::Disabled, interface, api_version)
        .await;

    let deleted = temp_dir.new_file();
    write_data_body(&deleted, &random_data(16), None, None, &mut file_client).await;
    delete_file(&deleted, None, None, &mut file_client).await;

    let request = ListStatRequest {
        include_native_metadata: false,
        folder: Some(FolderAddress {
            uri: temp_dir.url().to_string(),
        }),
    };
    let response = folder_client.list_stat(request).await;

    match response {
        Ok(_) => panic!("Expected deleted-only versioned folder to be NotFound"),
        Err(status) => assert_eq!(status.code(), Code::NotFound, "Expected NOT_FOUND error"),
    }
}

#[rstest]
#[test_log::test(tokio::test)]
async fn list_stat_versioned_paginated_continues_past_stale_only_pages(
    storage_fixture: &StorageFixture,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
) {
    let backend = Backend::S3;
    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, Access::Versioned, Cache::Disabled, api_version)
        .await
    else {
        debug!("Skipping test — no versioned bucket fixture for {:?}", backend);
        return;
    };

    let mut file_client = storage_fixture
        .file_object_client(backend, Cache::Disabled, Interface::Rest, api_version)
        .await;

    let deleted = format!("{}/000-deleted", temp_dir.url());
    let live = format!("{}/999-live", temp_dir.url());
    write_data_body(&deleted, &random_data(16), None, None, &mut file_client).await;
    delete_file(&deleted, None, None, &mut file_client).await;
    write_data_body(&live, &random_data(16), None, None, &mut file_client).await;

    let first_page = rest_list_stat_json_page(storage_fixture, api_version, temp_dir.url(), None, 1).await;
    assert!(
        rest_list_stat_entries(&first_page).is_empty(),
        "first page should only contain stale S3 version records"
    );
    assert!(
        rest_list_stat_subfolders(&first_page).is_empty(),
        "first page should not contain subfolders"
    );

    let mut entries = Vec::new();
    let mut continuation_handle = rest_list_stat_next_handle(&first_page);
    assert!(
        continuation_handle.is_some(),
        "stale-only first page should still return a continuation handle"
    );

    let mut pages_seen = 1;
    while let Some(handle) = continuation_handle {
        pages_seen += 1;
        assert!(pages_seen < 10, "list_stat pagination did not terminate");

        let page = rest_list_stat_json_page(storage_fixture, api_version, temp_dir.url(), Some(&handle), 1).await;
        entries.extend(rest_list_stat_entries(&page));
        continuation_handle = rest_list_stat_next_handle(&page);
    }

    assert!(entries.contains(&live), "paginated list_stat did not return live key");
    assert!(
        !entries.contains(&deleted),
        "paginated list_stat returned a key whose latest version is a delete marker"
    );

    temp_dir.cleanup().await;
}

#[rstest]
#[test_log::test(tokio::test)]
async fn list_stat_versioned_common_prefixes_use_live_namespace_paginated_rest(
    storage_fixture: &StorageFixture,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
) {
    let backend = Backend::S3;
    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, Access::Versioned, Cache::Disabled, api_version)
        .await
    else {
        debug!("Skipping test — no versioned bucket fixture for {:?}", backend);
        return;
    };

    let mut file_client = storage_fixture
        .file_object_client(backend, Cache::Disabled, Interface::Rest, api_version)
        .await;

    let stale_child_file = format!("{}/stale-child/file.txt", temp_dir.url());
    let stale_child_folder = format!("{}/stale-child/", temp_dir.url());
    let live_child_file = format!("{}/live-child/file.txt", temp_dir.url());
    let live_child_folder = format!("{}/live-child/", temp_dir.url());
    let live_file = format!("{}/zz-live.txt", temp_dir.url());

    write_data_body(&stale_child_file, &random_data(16), None, None, &mut file_client).await;
    delete_file(&stale_child_file, None, None, &mut file_client).await;
    write_data_body(&live_child_file, &random_data(16), None, None, &mut file_client).await;
    write_data_body(&live_file, &random_data(16), None, None, &mut file_client).await;

    let mut entries = Vec::new();
    let mut subfolders = Vec::new();
    let mut continuation_handle = None;
    let mut pages_seen = 0;
    loop {
        pages_seen += 1;
        assert!(pages_seen < 20, "list_stat pagination did not terminate");

        let page = rest_list_stat_json_page(
            storage_fixture,
            api_version,
            temp_dir.url(),
            continuation_handle.as_deref(),
            1,
        )
        .await;
        entries.extend(rest_list_stat_entries(&page));
        subfolders.extend(rest_list_stat_subfolders(&page));

        continuation_handle = rest_list_stat_next_handle(&page);
        if continuation_handle.is_none() {
            break;
        }
    }

    assert!(
        subfolders.contains(&live_child_folder),
        "paginated list_stat did not return live child folder; folders={subfolders:?}"
    );
    assert!(
        !subfolders.contains(&stale_child_folder),
        "paginated list_stat returned stale-only child folder; folders={subfolders:?}"
    );
    assert!(
        entries.contains(&live_file),
        "paginated list_stat did not return live direct child file; entries={entries:?}"
    );

    temp_dir.cleanup().await;
}

async fn rest_list_stat_json_page(
    storage_fixture: &StorageFixture,
    api_version: ApiVersion,
    folder_uri: &str,
    continuation_handle: Option<&str>,
    max_page_size: u32,
) -> serde_json::Value {
    let version_path = match api_version {
        ApiVersion::V1Alpha => "v1alpha",
        ApiVersion::V1Beta => "v1beta",
    };
    let encoded_folder = utf8_percent_encode(folder_uri, NON_ALPHANUMERIC).to_string();
    let mut url = format!(
        "http://localhost:{}/{version_path}/filefolder/liststat/{encoded_folder}?max_page_size={max_page_size}",
        storage_fixture.rest_port(Backend::S3, Cache::Disabled),
    );
    if let Some(continuation_handle) = continuation_handle {
        let encoded_handle = utf8_percent_encode(continuation_handle, NON_ALPHANUMERIC);
        url.push_str("&continuation_handle=");
        url.push_str(&encoded_handle.to_string());
    }

    let response = reqwest::Client::new()
        .get(url)
        .send()
        .await
        .expect("REST list_stat request failed");
    let status = response.status();
    let body = response.bytes().await.expect("REST list_stat response body failed");
    assert!(
        status.is_success(),
        "REST list_stat expected success, got {status}: {}",
        String::from_utf8_lossy(&body),
    );

    serde_json::from_slice(&body).expect("REST list_stat response was not JSON")
}

fn rest_list_stat_entries(page: &serde_json::Value) -> Vec<String> {
    page["entries"]
        .as_array()
        .expect("REST list_stat response missing entries array")
        .iter()
        .map(|entry| {
            entry["resource_address"]
                .as_str()
                .expect("REST list_stat entry missing resource_address")
                .to_string()
        })
        .collect()
}

fn rest_list_stat_subfolders(page: &serde_json::Value) -> Vec<String> {
    page["subfolder_addresses"]
        .as_array()
        .expect("REST list_stat response missing subfolder_addresses array")
        .iter()
        .map(|folder| {
            folder
                .as_str()
                .expect("REST list_stat subfolder address was not a string")
                .to_string()
        })
        .collect()
}

fn rest_list_stat_next_handle(page: &serde_json::Value) -> Option<String> {
    page["next_continuation_handle"].as_str().map(String::from)
}

#[rstest]
#[test_log::test(tokio::test)]
async fn get_folder_mode(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    let mut client = storage_fixture
        .file_folder_client(backend, Cache::Disabled, interface, ApiVersion::V1Alpha)
        .await;

    // The backends don't actually look at the folder address, so we can use any address.
    let request = GetFolderModeRequest {
        folder: Some(FolderAddress {
            uri: "unittests/get_folder_mode".to_string(),
        }),
    };
    let response = client.get_folder_mode(request).await;
    assert_eq!(response.unwrap().folder_mode, FolderMode::Hybrid as i32);
}

// Small create_folder test - selected by the nextest small profile (limited test cases for local efficiency)
#[rstest]
#[test_log::test(tokio::test)]
async fn create_folder_basic(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(Cache::List, Cache::Stat)] cache: Cache,
) {
    create_folder_impl(storage_fixture, backend, interface, access, cache).await;
}

// Full create_folder test - selected by the nextest full profile (comprehensive test coverage)
#[rstest]
#[test_log::test(tokio::test)]
async fn create_folder_full(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure, Backend::Minio, Backend::GCPS3Compat)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(Cache::List, Cache::Stat, Cache::Disabled)] cache: Cache,
) {
    create_folder_impl(storage_fixture, backend, interface, access, cache).await;
}

// Shared implementation for both create_folder tests
async fn create_folder_impl(
    storage_fixture: &StorageFixture,
    backend: Backend,
    interface: Interface,
    access: Access,
    cache: Cache,
) {
    let mut client = storage_fixture
        .file_folder_client(backend, Cache::Disabled, interface, ApiVersion::V1Alpha)
        .await;
    let mut file_object_client = storage_fixture
        .file_object_client(backend, Cache::Disabled, interface, ApiVersion::V1Alpha)
        .await;

    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, ApiVersion::V1Alpha)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    let folder = temp_dir.new_folder();

    let request = CreateFolderRequest {
        folder: Some(FolderAddress { uri: folder.clone() }),
    };

    // Create the folder
    client.create_folder(request).await.unwrap();

    // List the folder, should be empty and ok
    list_folder(&folder, None, Vec::new(), Vec::new(), &mut client).await;

    // List the parent folder, should contain the folder
    list_folder(temp_dir.url(), None, vec![&folder], Vec::new(), &mut client).await;

    // Stat the placeholder, should be an InvalidArgument
    let placeholder = StorageFixture::get_folder_placeholder(&folder, backend);
    assert_eq!(
        file_object_client
            .stat(StatRequest {
                resource_address: placeholder,
            })
            .await
            .err()
            .unwrap()
            .code(),
        Code::InvalidArgument
    );

    // Write a file to the folder, should be ok
    let file = temp_dir.new_file_in_folder(&folder);
    let data = random_data(2048);
    write_data_body(&file, &data, None, None, &mut file_object_client).await;

    // List again, should contain the file
    list_folder(&folder, None, Vec::new(), vec![&file], &mut client).await;

    // Delete the file
    delete_file(&file, None, None, &mut file_object_client).await;

    // List the parent folder, should contain the folder
    list_folder(temp_dir.url(), None, vec![&folder], Vec::new(), &mut client).await;

    // List again, folder should still exist
    list_folder(&folder, None, Vec::new(), Vec::new(), &mut client).await;

    // Delete the folder
    client
        .delete_folder(DeleteFolderRequest {
            folder: Some(FolderAddress { uri: folder.clone() }),
        })
        .await
        .unwrap();

    // List again, folder should be gone
    list_folder(&folder, Some(Code::NotFound), Vec::new(), Vec::new(), &mut client).await;

    temp_dir.cleanup().await;

    assert_method_metric!(
        storage_fixture,
        backend,
        ApiVersion::V1Alpha,
        "storage.requests",
        "create_folder",
        ["result" => "Ok"]
    );
    assert_method_metric!(storage_fixture, backend, ApiVersion::V1Alpha, "storage.sdk.requests", "create_folder", ["result" => "Ok"]);
    assert_method_metric!(
        storage_fixture,
        backend,
        ApiVersion::V1Alpha,
        "storage.request.duration",
        "create_folder",
        [],
        1
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        ApiVersion::V1Alpha,
        "storage.sdk.request.duration",
        "create_folder",
        [],
        1
    );
}

#[rstest]
#[case::case_01_missing_folder("unittests/list/nonexistent")]
#[case::case_02_missing_folder_slash("unittests/list/nonexistent/")]
#[case::case_03_file_as_folder("unittests/list/medium/file-0000.txt")]
#[test_log::test(tokio::test)]
async fn list_nonexistent_folder(
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
        .file_folder_client(backend, Cache::Disabled, interface, api_version)
        .await;

    let request = ListRequest {
        folder: Some(FolderAddress {
            uri: base_address.to_string(),
        }),
    };
    let response_stream = client.list(request).await;

    match response_stream {
        Ok(_) => panic!("Expected an error, but the method returned a successful response"),
        Err(status) => {
            assert_eq!(status.code(), Code::NotFound, "Expected NOT_FOUND error");
        },
    }

    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.requests",
        "list",
        ["result" => "NotFound"]
    );
}

#[rstest]
#[case::case_01_missing_folder("unittests/list/nonexistent")]
#[case::case_02_missing_folder_slash("unittests/list/nonexistent/")]
#[case::case_03_file_as_folder("unittests/list/medium/file-0000.txt")]
#[test_log::test(tokio::test)]
async fn list_stat_nonexistent_folder(
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
        .file_folder_client(backend, Cache::Disabled, interface, api_version)
        .await;

    let request = ListStatRequest {
        include_native_metadata: false,
        folder: Some(FolderAddress {
            uri: base_address.to_string(),
        }),
    };
    let response = client.list_stat(request).await;

    match response {
        Ok(_) => panic!("Expected an error, but the method returned a successful response"),
        Err(status) => {
            assert_eq!(status.code(), Code::NotFound, "Expected NOT_FOUND error");
        },
    }

    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.requests",
        "list_stat",
        ["result" => "NotFound"]
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn list_empty_top_level(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure, Backend::Minio, Backend::GCPS3Compat)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
) {
    let Some(base_address) = storage_fixture.make_url(backend, Access::Empty, "") else {
        debug!("Skipping test for {:?}", backend);
        return;
    };

    let mut client = storage_fixture
        .file_folder_client(backend, Cache::Disabled, interface, api_version)
        .await;

    let request = ListRequest {
        folder: Some(FolderAddress {
            uri: base_address.to_string(),
        }),
    };
    let response_stream = client.list(request).await.unwrap();

    let responses: Vec<_> = response_stream
        .filter_map(|response| async { response.ok() })
        .collect()
        .await;

    assert!(responses.len() == 1);
    assert!(responses[0].subfolder_addresses.is_empty());
    assert!(responses[0].sub_resource_addresses.is_empty());
}

/// Test that writing a deeply nested object invalidates all ancestor list cache entries.
///
/// Sequence:
/// 1. list({temp_dir}/a/) -> NotFound (populates cache with not-found error)
/// 2. write({temp_dir}/a/b/c/image.png) -> triggers ancestor invalidation
/// 3. list({temp_dir}/a/) -> sees subfolder a/b/ (proves /a/ cache was invalidated)
/// 4. list({temp_dir}/a/b/) -> sees subfolder a/b/c/
/// 5. list({temp_dir}/a/b/c/) -> sees image.png
#[rstest]
#[test_log::test(tokio::test)]
async fn list_cache_ancestor_invalidation(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc)] interface: Interface,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
) {
    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, Access::PrivateKey, Cache::List, api_version)
        .await
    else {
        debug!("Skipping test for {:?}", backend);
        return;
    };

    // Use Cache::List for both clients so the list cache is active and writes trigger invalidation
    let mut folder_client = storage_fixture
        .file_folder_client(backend, Cache::List, interface, api_version)
        .await;
    let mut file_client = storage_fixture
        .file_object_client(backend, Cache::List, interface, api_version)
        .await;

    // Build nested paths under the temp directory
    let base = temp_dir.url().to_string();
    let folder_a = format!("{base}/a/");
    let folder_ab = format!("{base}/a/b/");
    let folder_abc = format!("{base}/a/b/c/");
    let file_path = format!("{base}/a/b/c/image.png");

    // Step 1: List /a/ -> should be NotFound (this populates the cache with a not-found error)
    list_folder(
        &folder_a,
        Some(Code::NotFound),
        Vec::new(),
        Vec::new(),
        &mut folder_client,
    )
    .await;

    // Step 2: Write a file to /a/b/c/image.png -> should trigger ancestor invalidation
    let data = random_data(2048);
    write_data_body(&file_path, &data, None, None, &mut file_client).await;

    // Step 3: List /a/ -> should now see subfolder /a/b/ (proves /a/ cache was invalidated)
    list_folder(&folder_a, None, vec![&folder_ab], Vec::new(), &mut folder_client).await;

    // Step 4: List /a/b/ -> should see subfolder /a/b/c/
    list_folder(&folder_ab, None, vec![&folder_abc], Vec::new(), &mut folder_client).await;

    // Step 5: List /a/b/c/ -> should see image.png
    list_folder(&folder_abc, None, Vec::new(), vec![&file_path], &mut folder_client).await;

    temp_dir.cleanup().await;
}

/// Compute the decoded user key the list cache uses for a folder URL,
/// mirroring `ListCacheLayer::key()`.
fn list_cache_user_key(folder_url: &str) -> String {
    let url = Url::parse(folder_url).expect("parse folder url");
    percent_decode_str(url.as_str())
        .decode_utf8()
        .expect("decode folder url utf8")
        .into_owned()
}

/// Compute the L2 (Redis) data key the list cache uses for a folder URL,
/// mirroring `ListCacheLayer::key()` + `LowLevelCache::data_key()`. The fixture
/// writes a unique `[redis] namespace = ...` per test process so parallel
/// `cargo nextest` workers (and prior CI runs) don't share keyspace; the
/// resulting key is `{prefix}:list_cache:d:<url>`.
fn list_cache_l2_data_key(prefix: &str, folder_url: &str) -> String {
    format!("{prefix}:list_cache:d:{}", list_cache_user_key(folder_url))
}

/// Compute the L2 tombstone key planted when a list-cache entry is invalidated.
fn list_cache_l2_tombstone_key(prefix: &str, folder_url: &str) -> String {
    format!("{prefix}:list_cache:t:{}", list_cache_user_key(folder_url))
}

/// Like `list_cache_ancestor_invalidation`, but additionally proves the L2
/// (Redis) entry for an ancestor folder is deleted by the write — not just
/// the in-process L1. Soft-skips when L2 isn't configured.
///
/// Without this assertion, a regression that left the L2 entry intact would
/// be invisible: the same-process L1 invalidation in
/// `list_cache_ancestor_invalidation` would still pass because moka clears
/// the L1 entry, masking the stale L2 entry until L1 expired or evicted it.
///
/// Sequence:
/// 1. list({temp_dir}/a/) -> NotFound (populates L1 + L2 negative cache)
/// 2. probe Redis: L2 key for /a/ exists
/// 3. write({temp_dir}/a/b/c/image.png) -> triggers ancestor invalidation
/// 4. probe Redis: L2 key for /a/ is gone, tombstone is set
/// 5. list({temp_dir}/a/) -> sees /a/b/ (behavioral confirmation)
#[rstest]
#[test_log::test(tokio::test)]
async fn list_cache_l2_ancestor_invalidation(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc)] interface: Interface,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
) {
    let Some(redis) = storage_fixture.redis_client() else {
        debug!("Skipping L2 test: OMNI_STORAGE_SERVICE_REDIS_ADDRESS not set");
        return;
    };
    let prefix = storage_fixture
        .redis_namespace()
        .expect("redis client present implies redis_namespace is set");

    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, Access::PrivateKey, Cache::List, api_version)
        .await
    else {
        debug!("Skipping test for {:?}", backend);
        return;
    };

    let mut folder_client = storage_fixture
        .file_folder_client(backend, Cache::List, interface, api_version)
        .await;
    let mut file_client = storage_fixture
        .file_object_client(backend, Cache::List, interface, api_version)
        .await;

    let base = temp_dir.url().to_string();
    let folder_a = format!("{base}/a/");
    let folder_ab = format!("{base}/a/b/");
    let file_path = format!("{base}/a/b/c/image.png");

    let key_a = list_cache_l2_data_key(prefix, &folder_a);
    let tomb_a = list_cache_l2_tombstone_key(prefix, &folder_a);

    // Step 1: List /a/ -> NotFound. Populates L1 + L2 with the cached error.
    list_folder(
        &folder_a,
        Some(Code::NotFound),
        Vec::new(),
        Vec::new(),
        &mut folder_client,
    )
    .await;

    // Step 2: L2 must hold the negative cache entry now. The L2 envelope is a
    // binary postcard payload (not UTF-8), so read it as raw bytes rather than
    // a `String`.
    let before: Option<Vec<u8>> = redis.get(&key_a).await.expect("redis GET before write");
    assert!(
        before.is_some(),
        "expected L2 key {key_a} to be populated after initial list, was nil"
    );

    // Step 3: Write the deeply-nested file. Triggers
    // `invalidate_cache_entry_by_object_url` -> `invalidate_ancestors`.
    let data = random_data(2048);
    write_data_body(&file_path, &data, None, None, &mut file_client).await;

    // Step 4a: L2 entry for /a/ must be gone (proves L2 deletion, not just L1).
    let after: Option<Vec<u8>> = redis.get(&key_a).await.expect("redis GET after write");
    assert!(
        after.is_none(),
        "expected L2 key {key_a} to be deleted by invalidate_ancestors, still present"
    );

    // Step 4b: tombstone for /a/ should be set. Distinguishes "Lua
    // invalidate_key script ran" from "key happened to be absent". The
    // tombstone TTL is short (10s default), so this assertion has to come
    // promptly after the write.
    let tomb: Option<String> = redis.get(&tomb_a).await.expect("redis GET tombstone");
    assert!(
        tomb.is_some(),
        "expected tombstone {tomb_a} after invalidate_ancestors, was nil"
    );

    // Step 5: behavioral check — re-list /a/ now sees the new subfolder.
    list_folder(&folder_a, None, vec![&folder_ab], Vec::new(), &mut folder_client).await;

    temp_dir.cleanup().await;
}

#[rstest]
#[test_log::test(tokio::test)]
async fn folder_move_by_copy_delete_updates_list_cache(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc)] interface: Interface,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] folder_api_version: ApiVersion,
    #[values(Cache::Disabled, Cache::Stat, Cache::List)] cache: Cache,
) {
    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, Access::PrivateKey, cache, ApiVersion::V1Alpha)
        .await
    else {
        debug!("Skipping test for {:?}", backend);
        return;
    };

    let mut folder_mutation_client = storage_fixture
        .file_folder_client(backend, cache, interface, ApiVersion::V1Alpha)
        .await;
    let mut folder_read_client = storage_fixture
        .file_folder_client(backend, cache, interface, folder_api_version)
        .await;
    let mut file_client = storage_fixture
        .file_object_client(backend, cache, interface, ApiVersion::V1Alpha)
        .await;

    let base = temp_dir.url().to_string();
    let source_folder = format!("{base}/source/");
    let destination_folder = format!("{base}/destination/");
    let source_a = format!("{source_folder}a.bin");
    let source_b = format!("{source_folder}b.bin");
    let destination_a = format!("{destination_folder}a.bin");
    let destination_b = format!("{destination_folder}b.bin");

    folder_mutation_client
        .create_folder(CreateFolderRequest {
            folder: Some(FolderAddress {
                uri: source_folder.clone(),
            }),
        })
        .await
        .unwrap();

    let data_a = random_data(512);
    let data_b = random_data(1024);
    let source_a_identity = write_data_body(&source_a, &data_a, None, None, &mut file_client)
        .await
        .unwrap();
    let source_b_identity = write_data_body(&source_b, &data_b, None, None, &mut file_client)
        .await
        .unwrap();

    // Prime the parent and source folder cache entries before the simulated move.
    list_folder(
        temp_dir.url(),
        None,
        vec![&source_folder],
        Vec::new(),
        &mut folder_read_client,
    )
    .await;
    list_folder(
        &source_folder,
        None,
        Vec::new(),
        vec![&source_a, &source_b],
        &mut folder_read_client,
    )
    .await;
    list_stat_folder(
        &source_folder,
        None,
        Vec::new(),
        vec![(&source_a, data_a.len() as u64), (&source_b, data_b.len() as u64)],
        &mut folder_read_client,
    )
    .await;
    validate_file(&source_a, &data_a, &mut file_client).await;
    validate_file(&source_b, &data_b, &mut file_client).await;

    // Simulate a folder move with API operations the storage service actually supports.
    folder_mutation_client
        .create_folder(CreateFolderRequest {
            folder: Some(FolderAddress {
                uri: destination_folder.clone(),
            }),
        })
        .await
        .unwrap();
    copy_file(source_a_identity.clone(), &destination_a, None, None, &mut file_client)
        .await
        .unwrap();
    copy_file(source_b_identity.clone(), &destination_b, None, None, &mut file_client)
        .await
        .unwrap();

    // Both source and destination should be readable after copy and before source deletion.
    list_folder(
        &source_folder,
        None,
        Vec::new(),
        vec![&source_a, &source_b],
        &mut folder_read_client,
    )
    .await;
    list_stat_folder(
        &source_folder,
        None,
        Vec::new(),
        vec![(&source_a, data_a.len() as u64), (&source_b, data_b.len() as u64)],
        &mut folder_read_client,
    )
    .await;
    validate_file(&source_a, &data_a, &mut file_client).await;
    validate_file(&source_b, &data_b, &mut file_client).await;

    list_folder(
        &destination_folder,
        None,
        Vec::new(),
        vec![&destination_a, &destination_b],
        &mut folder_read_client,
    )
    .await;
    list_stat_folder(
        &destination_folder,
        None,
        Vec::new(),
        vec![
            (&destination_a, data_a.len() as u64),
            (&destination_b, data_b.len() as u64),
        ],
        &mut folder_read_client,
    )
    .await;
    validate_file(&destination_a, &data_a, &mut file_client).await;
    validate_file(&destination_b, &data_b, &mut file_client).await;

    delete_file(&source_a, None, None, &mut file_client).await;
    delete_file(&source_b, None, None, &mut file_client).await;
    folder_mutation_client
        .delete_folder(DeleteFolderRequest {
            folder: Some(FolderAddress {
                uri: source_folder.clone(),
            }),
        })
        .await
        .unwrap();

    assert_list_folder_not_found(&source_folder, &mut folder_read_client).await;
    list_stat_folder(
        &source_folder,
        Some(Code::NotFound),
        Vec::new(),
        Vec::new(),
        &mut folder_read_client,
    )
    .await;
    assert_file_read_operations_not_found(&source_a, &mut file_client).await;
    assert_file_read_operations_not_found(&source_b, &mut file_client).await;

    list_folder(
        temp_dir.url(),
        None,
        vec![&destination_folder],
        Vec::new(),
        &mut folder_read_client,
    )
    .await;
    list_folder(
        &destination_folder,
        None,
        Vec::new(),
        vec![&destination_a, &destination_b],
        &mut folder_read_client,
    )
    .await;
    list_stat_folder(
        &destination_folder,
        None,
        Vec::new(),
        vec![
            (&destination_a, data_a.len() as u64),
            (&destination_b, data_b.len() as u64),
        ],
        &mut folder_read_client,
    )
    .await;
    validate_file(&destination_a, &data_a, &mut file_client).await;
    validate_file(&destination_b, &data_b, &mut file_client).await;

    temp_dir.cleanup().await;
}

async fn assert_list_folder_not_found(folder: &str, client: &mut Box<dyn TestFileFolder>) {
    let response = client
        .list(ListRequest {
            folder: Some(FolderAddress {
                uri: folder.to_string(),
            }),
        })
        .await;

    let mut response_stream = match response {
        Ok(stream) => stream,
        Err(status) => {
            assert_eq!(status.code(), Code::NotFound);
            return;
        },
    };

    match response_stream.next().await {
        Some(Err(status)) => assert_eq!(status.code(), Code::NotFound),
        Some(Ok(response)) => panic!("List folder unexpectedly succeeded for {folder}: {response:?}"),
        None => panic!("List folder unexpectedly returned an empty stream for {folder}"),
    }
}

async fn list_stat_folder(
    folder: &str,
    expected_failure: Option<Code>,
    expected_subfolders: Vec<&str>,
    expected_objects: Vec<(&str, u64)>,
    client: &mut Box<dyn TestFileFolder>,
) {
    let response = client
        .list_stat(ListStatRequest {
            include_native_metadata: false,
            folder: Some(FolderAddress {
                uri: folder.to_string(),
            }),
        })
        .await;

    let mut response_stream = match response {
        Ok(stream) => stream,
        Err(status) => {
            if let Some(code) = expected_failure {
                assert_eq!(status.code(), code);
                return;
            }
            panic!("ListStat folder failed unexpectedly for {folder}: {status:?}");
        },
    };

    let mut responses = Vec::new();
    while let Some(response) = response_stream.next().await {
        match response {
            Ok(response) => responses.push(response),
            Err(status) => {
                if let Some(code) = expected_failure {
                    assert_eq!(status.code(), code);
                    return;
                }
                panic!("ListStat folder stream failed unexpectedly for {folder}: {status:?}");
            },
        }
    }

    if let Some(code) = expected_failure {
        panic!("ListStat folder unexpectedly succeeded for {folder}; expected {code:?}");
    }

    let subfolders: Vec<_> = responses
        .iter()
        .flat_map(|response| response.subfolder_addresses.iter())
        .map(|address| address.uri.clone())
        .collect();
    let entries: Vec<_> = responses
        .iter()
        .flat_map(|response| response.entries.iter())
        .map(|entry| {
            (
                entry.resource_address.clone(),
                entry
                    .resource_info
                    .as_ref()
                    .and_then(|resource_info| resource_info.metadata.as_ref())
                    .and_then(|metadata| metadata.data_object_size),
            )
        })
        .collect();

    assert_eq!(subfolders.len(), expected_subfolders.len());
    for subfolder in expected_subfolders {
        assert!(
            subfolders.contains(&subfolder.to_string()),
            "ListStat folder {folder} missing expected subfolder {subfolder}; got {subfolders:?}"
        );
    }

    assert_eq!(entries.len(), expected_objects.len());
    for (object, size) in expected_objects {
        let expected = (object.to_string(), Some(size));
        assert!(
            entries.contains(&expected),
            "ListStat folder {folder} missing expected object {expected:?}; got {entries:?}"
        );
    }
}

async fn assert_file_read_operations_not_found(file: &str, client: &mut Box<dyn TestFileObject>) {
    let stat_status = client
        .stat(StatRequest {
            resource_address: file.to_string(),
        })
        .await
        .err()
        .unwrap_or_else(|| panic!("Stat unexpectedly succeeded for deleted source file {file}"));
    assert_eq!(stat_status.code(), Code::NotFound);

    let response = client
        .read_from_address(ReadFromAddressRequest {
            resource_address: file.to_string(),
            download_preference: Some(DownloadPreference::Body.into()),
        })
        .await;

    let mut response_stream = match response {
        Ok(stream) => stream,
        Err(status) => {
            assert_eq!(status.code(), Code::NotFound);
            return;
        },
    };

    match response_stream.next().await {
        Some(Err(status)) => assert_eq!(status.code(), Code::NotFound),
        Some(Ok(response)) => {
            panic!("ReadFromAddress unexpectedly succeeded for deleted source file {file}: {response:?}")
        },
        None => panic!("ReadFromAddress unexpectedly returned an empty stream for deleted source file {file}"),
    }
}

#[rstest]
#[test_log::test(tokio::test)]
async fn list_stat_empty_top_level(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure, Backend::Minio, Backend::GCPS3Compat)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
) {
    let Some(base_address) = storage_fixture.make_url(backend, Access::Empty, "") else {
        debug!("Skipping test for {:?}", backend);
        return;
    };

    let mut client = storage_fixture
        .file_folder_client(backend, Cache::Disabled, interface, api_version)
        .await;

    let request = ListStatRequest {
        include_native_metadata: false,
        folder: Some(FolderAddress {
            uri: base_address.to_string(),
        }),
    };

    let response_stream = client.list_stat(request).await.unwrap();
    let responses: Vec<_> = response_stream
        .filter_map(|response| async { response.ok() })
        .collect()
        .await;

    assert!(responses.len() == 1);
    assert!(responses[0].subfolder_addresses.is_empty());
    assert!(responses[0].entries.is_empty());
}
