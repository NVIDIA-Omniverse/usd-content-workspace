// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

#![allow(clippy::too_many_arguments)]

use std::collections::{BTreeMap, BTreeSet};

use bytes::{Bytes, BytesMut};
use futures::stream::StreamExt;
use percent_encoding::percent_decode_str;
use rstest::*;
use serde_json::json;
use sha2::{Digest, Sha256};
use storage_core::traits::filefolder::v1alpha::*;
use storage_core::traits::fileobject::v1alpha::*;
use storage_core::traits::versioning::v1alpha::*;
use storage_core_integration_tests::fixture::*;
use storage_core_integration_tests::storage_fixture;
use storage_core_integration_tests::utils::{
    assert_data_eq, delete_file, move_file, write_data_body, write_data_multipart, write_data_redirect,
};
use storage_core_test::service::{ApiVersion, Interface};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Code, Status};

const SOURCES: &str = "unittests/workspaces/sources";
const OVERLAY: &str = "unittests/workspaces/overlay_v1.wkspc";
const MANIFEST_ONLY: &str = "unittests/workspaces/manifest_only_v1.wkspc";
const ABSENT: &str = "unittests/workspaces/absent_v1.wkspc";
const INVALID: &str = "unittests/workspaces/invalid_v1.wkspc";

#[derive(Debug, Clone, Copy)]
struct EntryCase {
    entry_rel: &'static str,
    source_rel: &'static str,
}

const HELLO: EntryCase = EntryCase {
    entry_rel: "folder_a/hello.txt",
    source_rel: "hello.txt",
};
const GOODBYE: EntryCase = EntryCase {
    entry_rel: "folder_a/goodbye.txt",
    source_rel: "goodbye.txt",
};
const BEE: EntryCase = EntryCase {
    entry_rel: "folder_a/nested/bee.txt",
    source_rel: "nested/bee.txt",
};
const BLOB: EntryCase = EntryCase {
    entry_rel: "folder_b/jellyfish - hevc.mkv",
    source_rel: "blob.bin",
};
const SHADOWED: EntryCase = EntryCase {
    entry_rel: "shadowed.txt",
    source_rel: "shadow_source.txt",
};
const CAFE: EntryCase = EntryCase {
    entry_rel: "café.txt",
    source_rel: "unicode/café.txt",
};

fn workspace_url(
    storage_fixture: &StorageFixture,
    backend: Backend,
    access: Access,
    workspace: &str,
) -> Option<String> {
    storage_fixture.make_url(backend, access, workspace)
}

fn child_url(
    storage_fixture: &StorageFixture,
    backend: Backend,
    access: Access,
    parent: &str,
    rel: &str,
) -> Option<String> {
    let base = storage_fixture.make_url(backend, access, parent)?;
    if rel.is_empty() {
        Some(base)
    } else {
        Some(format!("{}/{}", base.trim_end_matches('/'), rel))
    }
}

fn overlay_url(storage_fixture: &StorageFixture, backend: Backend, access: Access, rel: &str) -> Option<String> {
    child_url(storage_fixture, backend, access, OVERLAY, rel)
}

fn source_url(storage_fixture: &StorageFixture, backend: Backend, access: Access, source_rel: &str) -> Option<String> {
    child_url(storage_fixture, backend, access, SOURCES, source_rel)
}

fn write_workspace_url(
    storage_fixture: &StorageFixture,
    backend: Backend,
    access: Access,
    workspace_name: &str,
    rel: &str,
) -> Option<String> {
    child_url(
        storage_fixture,
        backend,
        access,
        &format!("unittests/writetests/{workspace_name}.wkspc"),
        rel,
    )
}

fn unique_workspace_name(prefix: &str) -> String {
    format!("{prefix}-{:016x}", rand::random::<u64>())
}

fn percent_decode_path(path: &str) -> String {
    percent_decode_str(path)
        .decode_utf8()
        .expect("valid UTF-8 path")
        .to_string()
}

fn relative_to_base(address: &str, base: &str) -> Option<String> {
    let address_url = url::Url::parse(address).ok()?;
    let base_url = url::Url::parse(base).ok()?;
    if address_url.scheme() != base_url.scheme() || address_url.host() != base_url.host() {
        return None;
    }

    let address_path = percent_decode_path(address_url.path())
        .trim_end_matches('/')
        .to_string();
    let base_path = percent_decode_path(base_url.path()).trim_end_matches('/').to_string();
    let relative = address_path.strip_prefix(&base_path)?;
    Some(relative.trim_start_matches('/').to_string())
}

fn relative_folder_to_base(address: &str, base: &str) -> Option<String> {
    let mut rel = relative_to_base(address, base)?;
    if !rel.is_empty() && !rel.ends_with('/') {
        rel.push('/');
    }
    Some(rel)
}

fn source_label(storage_fixture: &StorageFixture, backend: Backend, access: Access, address: &str) -> Option<String> {
    let sources_base = storage_fixture.make_url(backend, access, SOURCES)?;
    relative_to_base(address, &sources_base).map(|rel| format!("source:{rel}"))
}

fn workspace_label(address: &str, workspace_base: &str) -> Option<String> {
    relative_to_base(address, workspace_base).map(|rel| format!("workspace:{rel}"))
}

fn url_multiset(values: impl IntoIterator<Item = String>) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for value in values {
        *counts.entry(value).or_insert(0) += 1;
    }
    counts
}

fn assert_identity_addresses_equivalent(left: &ResourceIdentity, right: &ResourceIdentity) {
    let left_url = url::Url::parse(&left.encoded_identity).expect("left identity URL");
    let right_url = url::Url::parse(&right.encoded_identity).expect("right identity URL");
    assert_eq!(left_url.scheme(), right_url.scheme());
    assert_eq!(left_url.host(), right_url.host());
    assert_eq!(left_url.path(), right_url.path());

    let left_params: BTreeMap<_, _> = left_url.query_pairs().filter(|(key, _)| key != "version-id").collect();
    let right_params: BTreeMap<_, _> = right_url.query_pairs().filter(|(key, _)| key != "version-id").collect();
    assert_eq!(left_params, right_params);
}

fn assert_resource_info_equivalent(left: &ResourceInfo, right: &ResourceInfo) {
    assert_eq!(left.metadata, right.metadata);
    match (&left.resource_identity, &right.resource_identity) {
        (Some(left), Some(right)) => assert_identity_addresses_equivalent(left, right),
        (None, None) => {},
        _ => panic!("resource identity presence differs"),
    }
}

fn public_azure_workspace_fixture(backend: Backend, access: Access) -> bool {
    matches!((backend, access), (Backend::Azure, Access::Public))
}

fn assert_resource_info_same_address_and_size(left: &ResourceInfo, right: &ResourceInfo) {
    assert_eq!(
        left.metadata.as_ref().and_then(|metadata| metadata.data_object_size),
        right.metadata.as_ref().and_then(|metadata| metadata.data_object_size)
    );

    let left_identity = left.resource_identity.as_ref().expect("left identity");
    let right_identity = right.resource_identity.as_ref().expect("right identity");
    let left_url = url::Url::parse(&left_identity.encoded_identity).expect("left identity URL");
    let right_url = url::Url::parse(&right_identity.encoded_identity).expect("right identity URL");
    assert_eq!(left_url.scheme(), right_url.scheme());
    assert_eq!(left_url.host(), right_url.host());
    assert_eq!(left_url.path(), right_url.path());
}

async fn read_from_address_bytes(
    client: &mut Box<dyn storage_core_test::clients::traits::fileobject::FileObject>,
    resource_address: String,
    download_preference: DownloadPreference,
) -> Result<(ResourceInfo, Bytes), Status> {
    let mut stream = client
        .read_from_address(ReadFromAddressRequest {
            resource_address,
            download_preference: Some(download_preference.into()),
        })
        .await?;

    let resource_info = match stream
        .next()
        .await
        .expect("read_from_address stream should produce ResourceInfo")?
        .reply_type
        .expect("reply type")
    {
        read_from_address_response::ReplyType::ResourceInfo(info) => info,
        other => panic!("expected ResourceInfo, got {other:?}"),
    };

    match download_preference {
        DownloadPreference::Body => {
            let mut content = BytesMut::new();
            while let Some(response) = stream.next().await {
                match response?.reply_type.expect("reply type") {
                    read_from_address_response::ReplyType::Chunk(chunk) => content.extend_from_slice(&chunk.chunk),
                    other => panic!("expected Chunk, got {other:?}"),
                }
            }
            Ok((resource_info, content.freeze()))
        },
        DownloadPreference::Redirect => {
            let redirect = match stream
                .next()
                .await
                .expect("read_from_address stream should produce Redirect")?
                .reply_type
                .expect("reply type")
            {
                read_from_address_response::ReplyType::Redirect(redirect) => redirect,
                other => panic!("expected Redirect, got {other:?}"),
            };

            let mut headers = reqwest::header::HeaderMap::new();
            for header in redirect.additional_headers {
                headers.insert(
                    reqwest::header::HeaderName::from_bytes(header.name.as_bytes()).unwrap(),
                    header.value.parse().unwrap(),
                );
            }
            let body = reqwest::Client::new()
                .get(redirect.redirect_target_url)
                .headers(headers)
                .send()
                .await
                .unwrap()
                .bytes()
                .await
                .unwrap();
            Ok((resource_info, body))
        },
        _ => panic!("unsupported download preference: {download_preference:?}"),
    }
}

async fn assert_read_from_address_status(
    client: &mut Box<dyn storage_core_test::clients::traits::fileobject::FileObject>,
    resource_address: String,
    expected: Code,
) {
    match client
        .read_from_address(ReadFromAddressRequest {
            resource_address,
            download_preference: Some(DownloadPreference::Body.into()),
        })
        .await
    {
        Err(status) => assert_eq!(status.code(), expected),
        Ok(mut stream) => match stream.next().await {
            Some(Err(status)) => assert_eq!(status.code(), expected),
            other => panic!("expected {expected:?} read error, got {other:?}"),
        },
    }
}

async fn assert_enumerate_status(
    client: &mut Box<dyn storage_core_test::clients::traits::fileobject::FileObject>,
    resource_address: String,
    expected: Code,
) {
    match client.enumerate(EnumerateRequest { resource_address }).await {
        Err(status) => assert_eq!(status.code(), expected),
        Ok(mut stream) => match stream.next().await {
            Some(Err(status)) => assert_eq!(status.code(), expected),
            other => panic!("expected {expected:?} enumerate error, got {other:?}"),
        },
    }
}

async fn collect_list(
    stream: &mut ReceiverStream<Result<ListResponse, Status>>,
    workspace_base: &str,
) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut folders = BTreeSet::new();
    let mut files = BTreeSet::new();
    while let Some(response) = stream.next().await {
        let response = response.expect("list response");
        for folder in response.subfolder_addresses {
            folders.insert(relative_folder_to_base(&folder.uri, workspace_base).expect("workspace folder"));
        }
        for file in response.sub_resource_addresses {
            files.insert(relative_to_base(&file, workspace_base).expect("workspace file"));
        }
    }
    (folders, files)
}

async fn collect_list_stat(
    stream: &mut ReceiverStream<Result<ListStatResponse, Status>>,
    workspace_base: &str,
) -> (BTreeSet<String>, BTreeSet<String>, Vec<ListItem>) {
    let mut folders = BTreeSet::new();
    let mut files = BTreeSet::new();
    let mut entries = Vec::new();
    while let Some(response) = stream.next().await {
        let response = response.expect("list_stat response");
        for folder in response.subfolder_addresses {
            folders.insert(relative_folder_to_base(&folder.uri, workspace_base).expect("workspace folder"));
        }
        for entry in response.entries {
            files.insert(relative_to_base(&entry.resource_address, workspace_base).expect("workspace file"));
            entries.push(entry);
        }
    }
    (folders, files, entries)
}

/// Drain a `list` stream into multisets (folders, files), counting every occurrence so that
/// duplicate emissions across pages are visible. Used by the cross-source pagination tests:
/// a `BTreeSet` would silently mask the dup-on-page-boundary bug class.
async fn collect_list_multiset(
    stream: &mut ReceiverStream<Result<ListResponse, Status>>,
    workspace_base: &str,
) -> (BTreeMap<String, usize>, BTreeMap<String, usize>) {
    let mut folders = Vec::new();
    let mut files = Vec::new();
    while let Some(response) = stream.next().await {
        let response = response.expect("list response");
        for folder in response.subfolder_addresses {
            folders.push(relative_folder_to_base(&folder.uri, workspace_base).expect("workspace folder"));
        }
        for file in response.sub_resource_addresses {
            files.push(relative_to_base(&file, workspace_base).expect("workspace file"));
        }
    }
    (url_multiset(folders), url_multiset(files))
}

/// Drain a `list_stat` stream into multisets (folders, files), counting every occurrence.
async fn collect_list_stat_multiset(
    stream: &mut ReceiverStream<Result<ListStatResponse, Status>>,
    workspace_base: &str,
) -> (BTreeMap<String, usize>, BTreeMap<String, usize>) {
    let mut folders = Vec::new();
    let mut files = Vec::new();
    while let Some(response) = stream.next().await {
        let response = response.expect("list_stat response");
        for folder in response.subfolder_addresses {
            folders.push(relative_folder_to_base(&folder.uri, workspace_base).expect("workspace folder"));
        }
        for entry in response.entries {
            files.push(relative_to_base(&entry.resource_address, workspace_base).expect("workspace file"));
        }
    }
    (url_multiset(folders), url_multiset(files))
}

async fn collect_enumerate_labels(
    stream: &mut ReceiverStream<Result<EnumerateResponse, Status>>,
    storage_fixture: &StorageFixture,
    backend: Backend,
    access: Access,
    workspace_base: &str,
) -> BTreeMap<String, usize> {
    let mut labels = Vec::new();
    while let Some(response) = stream.next().await {
        let response = response.expect("enumerate response");
        for item in response.items {
            if let Some(label) = workspace_label(&item.resource_address, workspace_base) {
                labels.push(label);
            } else if let Some(label) = source_label(storage_fixture, backend, access, &item.resource_address) {
                labels.push(label);
            } else {
                panic!("unexpected enumerate address: {}", item.resource_address);
            }
        }
    }
    url_multiset(labels)
}

fn expected_overlay_enumerate() -> BTreeMap<String, usize> {
    url_multiset([
        "workspace:.manifest".to_string(),
        "workspace:passthrough.txt".to_string(),
        "workspace:shared/from_backend.txt".to_string(),
        "workspace:my docs/from_backend.txt".to_string(),
        "workspace:sub/deep/leaf.txt".to_string(),
        "source:hello.txt".to_string(),
        "source:hello.txt".to_string(),
        "source:goodbye.txt".to_string(),
        "source:nested/bee.txt".to_string(),
        "source:blob.bin".to_string(),
        "source:shadow_source.txt".to_string(),
        "source:spaced/from_manifest.txt".to_string(),
        "source:unicode/café.txt".to_string(),
    ])
}

fn expected_source_labels(sources: &[&str]) -> BTreeMap<String, usize> {
    url_multiset(sources.iter().map(|source| format!("source:{source}")))
}

fn manifest_json(entries: &[(&str, &str)]) -> Bytes {
    let files: Vec<_> = entries
        .iter()
        .map(|(relative_path, resource_identity)| {
            json!({
                "relative_path": relative_path,
                "resource_identity": resource_identity,
            })
        })
        .collect();
    Bytes::from(
        serde_json::to_vec(&json!({
            "format_version": { "version_string": "1.0" },
            "storage_api_version": "v1alpha",
            "files": files,
        }))
        .expect("manifest JSON"),
    )
}

fn manifest_json_at_least(entries: &[(&str, &str)], minimum_size: usize) -> Bytes {
    let mut manifest: serde_json::Value = serde_json::from_slice(&manifest_json(entries)).expect("manifest JSON value");
    let current_len = serde_json::to_vec(&manifest).expect("manifest JSON").len();
    if current_len < minimum_size {
        manifest["_padding"] = json!("x".repeat(minimum_size - current_len));
    }
    Bytes::from(serde_json::to_vec(&manifest).expect("padded manifest JSON"))
}

async fn source_identity(
    storage_fixture: &StorageFixture,
    backend: Backend,
    access: Access,
    api_version: ApiVersion,
    source_rel: &str,
) -> Option<String> {
    let mut client = storage_fixture
        .file_object_client(backend, Cache::Disabled, Interface::Grpc, api_version)
        .await;
    let source = source_url(storage_fixture, backend, access, source_rel)?;
    Some(
        client
            .stat(StatRequest {
                resource_address: source,
            })
            .await
            .unwrap()
            .resource_info
            .unwrap()
            .resource_identity
            .unwrap()
            .encoded_identity,
    )
}

fn manifest_identity_address(identity: &ResourceIdentity) -> String {
    let mut url = url::Url::parse(&identity.encoded_identity).expect("resource identity URL");
    url.set_query(None);
    url.set_fragment(None);
    url.to_string()
}

fn loose_manifest_identity_spelling(identity: &str) -> String {
    let mut url = url::Url::parse(identity).expect("resource identity URL");
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

// This is deliberately a pairwise matrix rather than a full Cartesian product: together the two
// cases cover both live backends, both public interfaces, and both API versions. Every URL assertion
// is byte-exact. In particular, this test does not use relative_to_base(), whose percent-decoding is
// appropriate for older semantic fixture tests but would mask this spelling regression.
#[rstest]
#[case::s3_grpc_v1beta(Backend::S3, Interface::Grpc, ApiVersion::V1Beta)]
#[case::azure_rest_v1alpha(Backend::Azure, Interface::Rest, ApiVersion::V1Alpha)]
#[test_log::test(tokio::test)]
async fn canonical_manifest_addresses_and_identities(
    storage_fixture: &StorageFixture,
    #[case] backend: Backend,
    #[case] interface: Interface,
    #[case] api_version: ApiVersion,
) {
    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, Access::PrivateKey, Cache::Disabled, api_version)
        .await
    else {
        return;
    };
    let workspace_strict = format!("{}/workspace%20%28%20%29%20%2B%20%5B%20%5D.wkspc", temp_dir.url());
    let workspace_loose = loose_manifest_identity_spelling(&workspace_strict);
    let child_strict = format!("{workspace_strict}/child%20%28%20%29%20%2B%20%5B%20%5D.txt");
    let child_loose = loose_manifest_identity_spelling(&child_strict);
    let manifest_address = format!("{workspace_strict}/.manifest");
    let source_address = format!("{}/source%20%28%20%29%20%2B%20%5B%20%5D.txt", temp_dir.url());
    let source_data = b"canonical manifest source\n";
    let collision_data = b"physical collision must be shadowed\n";

    let mut backend_client = storage_fixture
        .file_object_client(backend, Cache::Disabled, interface, api_version)
        .await;
    let source_identity = write_data_body(&source_address, source_data, None, None, &mut backend_client)
        .await
        .expect("source write should return an identity");
    assert_eq!(manifest_identity_address(&source_identity), source_address);
    write_data_body(&child_strict, collision_data, None, None, &mut backend_client)
        .await
        .expect("collision write should return an identity");

    // Bake the loose spelling into the manifest document itself. All synthesized ResourceInfo
    // values must still expose the strict source identity.
    let loose_source_identity = loose_manifest_identity_spelling(&source_identity.encoded_identity);
    assert_ne!(loose_source_identity, source_identity.encoded_identity);
    let mut manifest_object_client = storage_fixture
        .file_object_client(backend, Cache::Manifest, interface, api_version)
        .await;
    write_data_body(
        &manifest_address,
        &manifest_json(&[("child ( ) + [ ].txt", &loose_source_identity)]),
        None,
        None,
        &mut manifest_object_client,
    )
    .await
    .expect("manifest write should return an identity");

    let stat_info = manifest_object_client
        .stat(StatRequest {
            resource_address: child_loose.clone(),
        })
        .await
        .expect("manifest child stat should succeed")
        .resource_info
        .expect("manifest child stat should return resource info");
    assert_eq!(
        stat_info
            .resource_identity
            .expect("manifest child stat should return an identity")
            .encoded_identity,
        source_identity.encoded_identity
    );

    let (read_info, body) = read_from_address_bytes(
        &mut manifest_object_client,
        child_loose.clone(),
        DownloadPreference::Body,
    )
    .await
    .expect("manifest child read_from_address should succeed");
    assert_eq!(
        read_info
            .resource_identity
            .expect("read_from_address should return an identity")
            .encoded_identity,
        source_identity.encoded_identity
    );
    assert_eq!(body, source_data.as_slice());

    let mut manifest_folder_client = storage_fixture
        .file_folder_client(backend, Cache::Manifest, interface, api_version)
        .await;
    let mut list_stream = manifest_folder_client
        .list(ListRequest {
            folder: Some(FolderAddress {
                uri: workspace_loose.clone(),
            }),
        })
        .await
        .expect("manifest workspace list should return a stream");
    let mut listed_addresses = Vec::new();
    while let Some(response) = list_stream.next().await {
        listed_addresses.extend(
            response
                .expect("manifest list stream should succeed")
                .sub_resource_addresses,
        );
    }
    assert_eq!(
        listed_addresses
            .iter()
            .filter(|address| *address == &child_strict)
            .count(),
        1,
        "manifest child and physical collision should merge to one strict list address: {listed_addresses:?}"
    );

    let mut list_stat_stream = manifest_folder_client
        .list_stat(ListStatRequest {
            include_native_metadata: false,
            folder: Some(FolderAddress {
                uri: workspace_loose.clone(),
            }),
        })
        .await
        .expect("manifest workspace list_stat should return a stream");
    let mut matching_list_stat = Vec::new();
    while let Some(response) = list_stat_stream.next().await {
        matching_list_stat.extend(
            response
                .expect("manifest list_stat stream should succeed")
                .entries
                .into_iter()
                .filter(|entry| entry.resource_address == child_strict),
        );
    }
    assert_eq!(matching_list_stat.len(), 1, "list_stat should return one strict child");
    assert_eq!(
        matching_list_stat
            .pop()
            .expect("matching list_stat entry")
            .resource_info
            .expect("list_stat child should have resource info")
            .resource_identity
            .expect("list_stat child should have an identity")
            .encoded_identity,
        source_identity.encoded_identity
    );

    let source_identity_address = manifest_identity_address(&source_identity);
    let mut enumerate_stream = manifest_object_client
        .enumerate(EnumerateRequest {
            resource_address: workspace_loose,
        })
        .await
        .expect("manifest workspace enumerate should return a stream");
    let mut source_items = 0;
    while let Some(response) = enumerate_stream.next().await {
        for item in response.expect("manifest enumerate stream should succeed").items {
            assert_ne!(
                item.resource_address, child_strict,
                "recursive enumerate must shadow the physical collision"
            );
            if item.resource_address == source_identity_address {
                source_items += 1;
            }
        }
    }
    assert_eq!(
        source_items, 1,
        "enumerate should contain exactly one canonical manifest source"
    );

    temp_dir.cleanup().await;
}

// Small stat test - selected by the nextest small profile.
#[rstest]
#[case::case_01_hello(HELLO)]
#[test_log::test(tokio::test)]
async fn stat_basic(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3)] backend: Backend,
    #[values(Interface::Grpc)] interface: Interface,
    #[values(Access::Public)] access: Access,
    #[values(ApiVersion::V1Alpha)] api_version: ApiVersion,
    #[case] entry: EntryCase,
) {
    stat_impl(storage_fixture, backend, interface, access, api_version, entry).await;
}

// Full stat test - selected by the nextest full profile.
#[rstest]
#[case::case_01_hello(HELLO)]
#[case::case_02_goodbye(GOODBYE)]
#[case::case_03_jellyfish_hevc(BLOB)]
#[case::case_04_shadowed(SHADOWED)]
#[case::case_05_cafe(CAFE)]
#[test_log::test(tokio::test)]
async fn stat_full(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::Public, Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[case] entry: EntryCase,
) {
    stat_impl(storage_fixture, backend, interface, access, api_version, entry).await;
}

async fn stat_impl(
    storage_fixture: &StorageFixture,
    backend: Backend,
    interface: Interface,
    access: Access,
    api_version: ApiVersion,
    entry: EntryCase,
) {
    let Some(manifest_url) = overlay_url(storage_fixture, backend, access, entry.entry_rel) else {
        return;
    };
    let Some(source_url) = source_url(storage_fixture, backend, access, entry.source_rel) else {
        return;
    };
    let mut manifest_client = storage_fixture
        .file_object_client(backend, Cache::Manifest, interface, api_version)
        .await;
    let mut regular_client = storage_fixture
        .file_object_client(backend, Cache::Disabled, interface, api_version)
        .await;

    let manifest_response = manifest_client
        .stat(StatRequest {
            resource_address: manifest_url,
        })
        .await
        .unwrap();
    let regular_response = regular_client
        .stat(StatRequest {
            resource_address: source_url,
        })
        .await
        .unwrap();

    let manifest_info = manifest_response.resource_info.unwrap();
    let regular_info = regular_response.resource_info.unwrap();
    if public_azure_workspace_fixture(backend, access) {
        assert_resource_info_same_address_and_size(&manifest_info, &regular_info);
    } else {
        assert_resource_info_equivalent(&manifest_info, &regular_info);
    }
}

// Small read_from_address test - selected by the nextest small profile.
#[rstest]
#[case::case_01_hello_body(HELLO, DownloadPreference::Body)]
#[test_log::test(tokio::test)]
async fn read_from_address_basic(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3)] backend: Backend,
    #[values(Interface::Grpc)] interface: Interface,
    #[values(Access::Public)] access: Access,
    #[values(ApiVersion::V1Alpha)] api_version: ApiVersion,
    #[case] entry: EntryCase,
    #[case] download_preference: DownloadPreference,
) {
    read_from_address_impl(
        storage_fixture,
        backend,
        interface,
        access,
        api_version,
        entry,
        download_preference,
    )
    .await;
}

// Full read_from_address test - selected by the nextest full profile.
#[rstest]
#[case::case_01_hello_body(HELLO, DownloadPreference::Body)]
#[case::case_02_goodbye_body(GOODBYE, DownloadPreference::Body)]
#[case::case_03_jellyfish_redirect(BLOB, DownloadPreference::Redirect)]
#[case::case_04_shadowed_manifest_wins(SHADOWED, DownloadPreference::Body)]
#[case::case_05_cafe_manifest_wins(CAFE, DownloadPreference::Body)]
#[test_log::test(tokio::test)]
async fn read_from_address_full(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::Public, Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[case] entry: EntryCase,
    #[case] download_preference: DownloadPreference,
) {
    read_from_address_impl(
        storage_fixture,
        backend,
        interface,
        access,
        api_version,
        entry,
        download_preference,
    )
    .await;
}

async fn read_from_address_impl(
    storage_fixture: &StorageFixture,
    backend: Backend,
    interface: Interface,
    access: Access,
    api_version: ApiVersion,
    entry: EntryCase,
    download_preference: DownloadPreference,
) {
    if public_azure_workspace_fixture(backend, access) {
        return;
    }

    let Some(manifest_url) = overlay_url(storage_fixture, backend, access, entry.entry_rel) else {
        return;
    };
    let Some(source_url) = source_url(storage_fixture, backend, access, entry.source_rel) else {
        return;
    };
    let mut manifest_client = storage_fixture
        .file_object_client(backend, Cache::Manifest, interface, api_version)
        .await;
    let mut regular_client = storage_fixture
        .file_object_client(backend, Cache::Disabled, interface, api_version)
        .await;

    let (manifest_info, manifest_body) =
        read_from_address_bytes(&mut manifest_client, manifest_url, download_preference)
            .await
            .unwrap();
    let (regular_info, regular_body) = read_from_address_bytes(&mut regular_client, source_url, download_preference)
        .await
        .unwrap();

    assert_eq!(manifest_info.metadata, regular_info.metadata);
    assert_eq!(Sha256::digest(&manifest_body), Sha256::digest(&regular_body));
}

#[rstest]
#[test_log::test(tokio::test)]
async fn read_fallthrough_manifest_wins_missing_and_manifest_file_basic(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3)] backend: Backend,
    #[values(Interface::Grpc)] interface: Interface,
    #[values(Access::Public)] access: Access,
    #[values(ApiVersion::V1Alpha)] api_version: ApiVersion,
) {
    read_overlay_behavior_impl(storage_fixture, backend, interface, access, api_version).await;
}

#[rstest]
#[test_log::test(tokio::test)]
async fn read_fallthrough_manifest_wins_missing_and_manifest_file_full(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::Public, Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
) {
    read_overlay_behavior_impl(storage_fixture, backend, interface, access, api_version).await;
}

async fn read_overlay_behavior_impl(
    storage_fixture: &StorageFixture,
    backend: Backend,
    interface: Interface,
    access: Access,
    api_version: ApiVersion,
) {
    let mut manifest_client = storage_fixture
        .file_object_client(backend, Cache::Manifest, interface, api_version)
        .await;
    let mut regular_client = storage_fixture
        .file_object_client(backend, Cache::Disabled, interface, api_version)
        .await;

    let Some(passthrough) = overlay_url(storage_fixture, backend, access, "passthrough.txt") else {
        return;
    };
    let (_, manifest_passthrough) =
        read_from_address_bytes(&mut manifest_client, passthrough.clone(), DownloadPreference::Body)
            .await
            .unwrap();
    let (_, regular_passthrough) = read_from_address_bytes(&mut regular_client, passthrough, DownloadPreference::Body)
        .await
        .unwrap();
    assert_data_eq(&regular_passthrough, &manifest_passthrough);

    if !public_azure_workspace_fixture(backend, access) {
        let Some(shadowed) = overlay_url(storage_fixture, backend, access, SHADOWED.entry_rel) else {
            return;
        };
        let (_, body) = read_from_address_bytes(&mut manifest_client, shadowed, DownloadPreference::Body)
            .await
            .unwrap();
        assert_data_eq(b"FROM-MANIFEST\n", &body);
    }

    let Some(missing) = overlay_url(storage_fixture, backend, access, "nope.txt") else {
        return;
    };
    assert_read_from_address_status(&mut manifest_client, missing, Code::NotFound).await;

    let Some(manifest_file) = overlay_url(storage_fixture, backend, access, ".manifest") else {
        return;
    };
    let (_, manifest_bytes) =
        read_from_address_bytes(&mut manifest_client, manifest_file.clone(), DownloadPreference::Body)
            .await
            .unwrap();
    let (_, regular_manifest_bytes) =
        read_from_address_bytes(&mut regular_client, manifest_file, DownloadPreference::Body)
            .await
            .unwrap();
    assert_data_eq(&regular_manifest_bytes, &manifest_bytes);

    // plan #6: the forwarded `.manifest` bytes are the real manifest document, not e.g. a
    // synthesized/empty payload. Parse them and assert the expected JSON shape (a `files` array).
    let manifest_value: serde_json::Value =
        serde_json::from_slice(&manifest_bytes).expect(".manifest bytes should be valid JSON");
    assert!(
        manifest_value.get("files").and_then(|files| files.as_array()).is_some(),
        ".manifest JSON should contain a `files` array, got: {manifest_value}"
    );
}

// Small list test - selected by the nextest small profile.
#[rstest]
#[case::case_01_folder_a(OVERLAY, "folder_a", &["nested/"], &["goodbye.txt", "hello.txt"])]
#[test_log::test(tokio::test)]
async fn list_basic(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3)] backend: Backend,
    #[values(Interface::Grpc)] interface: Interface,
    #[values(Access::Public)] access: Access,
    #[values(ApiVersion::V1Alpha)] api_version: ApiVersion,
    #[case] workspace: &str,
    #[case] folder_rel: &str,
    #[case] expected_folders: &[&str],
    #[case] expected_files: &[&str],
) {
    list_impl(
        storage_fixture,
        backend,
        interface,
        access,
        api_version,
        workspace,
        folder_rel,
        expected_folders,
        expected_files,
    )
    .await;
}

// Full list test - selected by the nextest full profile.
#[rstest]
#[case::case_01_folder_a(OVERLAY, "folder_a", &["nested/"], &["goodbye.txt", "hello.txt"])]
#[case::case_02_overlay_root(
    OVERLAY,
    "",
    &["folder_a/", "folder_b/", "my docs/", "shared/", "sub/"],
    &[".manifest", "café.txt", "passthrough.txt", "shadowed.txt"]
)]
#[case::case_03_backend_only_sub(OVERLAY, "sub", &["deep/"], &[])]
#[case::case_04_shared_merge(OVERLAY, "shared", &[], &["from_backend.txt", "in_manifest.txt"])]
#[case::case_05_spaced_merge(OVERLAY, "my docs", &[], &["from_backend.txt", "from_manifest.txt"])]
#[case::case_06_manifest_only_root(MANIFEST_ONLY, "", &["pkg/"], &[".manifest"])]
#[case::case_07_manifest_only_pkg(MANIFEST_ONLY, "pkg", &["sub/"], &["a.usd", "b.usd"])]
#[case::case_08_absent_root(ABSENT, "", &["sub/"], &["plain_a.txt", "plain_b.txt"])]
#[test_log::test(tokio::test)]
async fn list_full(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::Public, Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[case] workspace: &str,
    #[case] folder_rel: &str,
    #[case] expected_folders: &[&str],
    #[case] expected_files: &[&str],
) {
    list_impl(
        storage_fixture,
        backend,
        interface,
        access,
        api_version,
        workspace,
        folder_rel,
        expected_folders,
        expected_files,
    )
    .await;
}

async fn list_impl(
    storage_fixture: &StorageFixture,
    backend: Backend,
    interface: Interface,
    access: Access,
    api_version: ApiVersion,
    workspace: &str,
    folder_rel: &str,
    expected_folders: &[&str],
    expected_files: &[&str],
) {
    let Some(workspace_base) = workspace_url(storage_fixture, backend, access, workspace) else {
        return;
    };
    let Some(folder_url) = child_url(storage_fixture, backend, access, workspace, folder_rel) else {
        return;
    };
    let mut manifest_client = storage_fixture
        .file_folder_client(backend, Cache::Manifest, interface, api_version)
        .await;
    let mut stream = manifest_client
        .list(ListRequest {
            folder: Some(FolderAddress { uri: folder_url }),
        })
        .await
        .unwrap();
    let (folders, files) = collect_list(
        &mut stream,
        format!("{}/{}", workspace_base, folder_rel).trim_end_matches('/'),
    )
    .await;

    assert_eq!(
        folders,
        expected_folders.iter().map(|value| (*value).to_string()).collect()
    );
    assert_eq!(files, expected_files.iter().map(|value| (*value).to_string()).collect());
}

// Small list_stat test - selected by the nextest small profile.
#[rstest]
#[case::case_01_folder_a(OVERLAY, "folder_a", &["nested/"], &["goodbye.txt", "hello.txt"])]
#[test_log::test(tokio::test)]
async fn list_stat_basic(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3)] backend: Backend,
    #[values(Interface::Grpc)] interface: Interface,
    #[values(Access::Public)] access: Access,
    #[values(ApiVersion::V1Alpha)] api_version: ApiVersion,
    #[case] workspace: &str,
    #[case] folder_rel: &str,
    #[case] expected_folders: &[&str],
    #[case] expected_files: &[&str],
) {
    list_stat_impl(
        storage_fixture,
        backend,
        interface,
        access,
        api_version,
        workspace,
        folder_rel,
        expected_folders,
        expected_files,
    )
    .await;
}

// Full list_stat test - selected by the nextest full profile.
#[rstest]
#[case::case_01_folder_a(OVERLAY, "folder_a", &["nested/"], &["goodbye.txt", "hello.txt"])]
#[case::case_02_overlay_root(
    OVERLAY,
    "",
    &["folder_a/", "folder_b/", "my docs/", "shared/", "sub/"],
    &[".manifest", "café.txt", "passthrough.txt", "shadowed.txt"]
)]
#[case::case_03_backend_only_sub(OVERLAY, "sub", &["deep/"], &[])]
#[case::case_04_shared_merge(OVERLAY, "shared", &[], &["from_backend.txt", "in_manifest.txt"])]
#[case::case_05_spaced_merge(OVERLAY, "my docs", &[], &["from_backend.txt", "from_manifest.txt"])]
#[case::case_06_manifest_only_pkg(MANIFEST_ONLY, "pkg", &["sub/"], &["a.usd", "b.usd"])]
#[case::case_07_absent_root(ABSENT, "", &["sub/"], &["plain_a.txt", "plain_b.txt"])]
#[test_log::test(tokio::test)]
async fn list_stat_full(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::Public, Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[case] workspace: &str,
    #[case] folder_rel: &str,
    #[case] expected_folders: &[&str],
    #[case] expected_files: &[&str],
) {
    list_stat_impl(
        storage_fixture,
        backend,
        interface,
        access,
        api_version,
        workspace,
        folder_rel,
        expected_folders,
        expected_files,
    )
    .await;
}

async fn list_stat_impl(
    storage_fixture: &StorageFixture,
    backend: Backend,
    interface: Interface,
    access: Access,
    api_version: ApiVersion,
    workspace: &str,
    folder_rel: &str,
    expected_folders: &[&str],
    expected_files: &[&str],
) {
    let Some(workspace_base) = workspace_url(storage_fixture, backend, access, workspace) else {
        return;
    };
    let Some(folder_url) = child_url(storage_fixture, backend, access, workspace, folder_rel) else {
        return;
    };
    let mut manifest_client = storage_fixture
        .file_folder_client(backend, Cache::Manifest, interface, api_version)
        .await;
    let mut stream = manifest_client
        .list_stat(ListStatRequest {
            include_native_metadata: false,
            folder: Some(FolderAddress { uri: folder_url }),
        })
        .await
        .unwrap();
    let (folders, files, entries) = collect_list_stat(
        &mut stream,
        format!("{}/{}", workspace_base, folder_rel).trim_end_matches('/'),
    )
    .await;

    assert_eq!(
        folders,
        expected_folders.iter().map(|value| (*value).to_string()).collect()
    );
    assert_eq!(files, expected_files.iter().map(|value| (*value).to_string()).collect());
    for entry in entries {
        assert!(entry.resource_info.and_then(|info| info.metadata).is_some());
    }
}

/// Run `list_stat` against `folder_rel` of `workspace` (manifest runner) and return its entries.
async fn list_stat_entries(
    storage_fixture: &StorageFixture,
    backend: Backend,
    interface: Interface,
    access: Access,
    api_version: ApiVersion,
    workspace: &str,
    folder_rel: &str,
) -> Option<Vec<ListItem>> {
    let workspace_base = workspace_url(storage_fixture, backend, access, workspace)?;
    let folder_url = child_url(storage_fixture, backend, access, workspace, folder_rel)?;
    let mut manifest_client = storage_fixture
        .file_folder_client(backend, Cache::Manifest, interface, api_version)
        .await;
    let mut stream = manifest_client
        .list_stat(ListStatRequest {
            include_native_metadata: false,
            folder: Some(FolderAddress { uri: folder_url }),
        })
        .await
        .unwrap();
    let (_, _, entries) = collect_list_stat(
        &mut stream,
        format!("{}/{}", workspace_base, folder_rel).trim_end_matches('/'),
    )
    .await;
    Some(entries)
}

/// Pick the `list_stat` entry whose `resource_address` ends with `/rel` (the workspace-relative
/// name we listed under). Panics if it is absent — these are known fixture entries.
fn entry_with_rel(entries: &[ListItem], rel: &str) -> ListItem {
    entries
        .iter()
        .find(|entry| {
            percent_decode_path(&entry.resource_address)
                .trim_end_matches('/')
                .ends_with(&format!("/{rel}"))
        })
        .unwrap_or_else(|| panic!("list_stat entries missing {rel:?}"))
        .clone()
}

// list_stat metadata fidelity (plan #12). The shape tests above only assert
// `metadata.is_some()`; this asserts the *source* of that metadata so a backend/manifest
// metadata-source swap regression is caught:
//   - `folder_a/hello.txt` is a manifest entry -> its ResourceInfo (size + baked identity) must
//     match the manifest's source object `sources/hello.txt`.
//   - `passthrough.txt` is a backend fall-through entry -> its ResourceInfo must match a direct
//     backend stat of the same overlay address.
// Public-Azure is skipped (front-door rewrites metadata/identity, exactly as the read/stat
// tests skip it via `public_azure_workspace_fixture`).
#[rstest]
#[test_log::test(tokio::test)]
async fn list_stat_metadata_fidelity_basic(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3)] backend: Backend,
    #[values(Interface::Grpc)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(ApiVersion::V1Alpha)] api_version: ApiVersion,
) {
    list_stat_metadata_fidelity_impl(storage_fixture, backend, interface, access, api_version).await;
}

#[rstest]
#[test_log::test(tokio::test)]
async fn list_stat_metadata_fidelity_full(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
) {
    list_stat_metadata_fidelity_impl(storage_fixture, backend, interface, access, api_version).await;
}

async fn list_stat_metadata_fidelity_impl(
    storage_fixture: &StorageFixture,
    backend: Backend,
    interface: Interface,
    access: Access,
    api_version: ApiVersion,
) {
    if public_azure_workspace_fixture(backend, access) {
        return;
    }

    let mut disabled_client = storage_fixture
        .file_object_client(backend, Cache::Disabled, interface, api_version)
        .await;

    // --- manifest entry: folder_a/hello.txt vs its source sources/hello.txt ---
    let Some(folder_a_entries) = list_stat_entries(
        storage_fixture,
        backend,
        interface,
        access,
        api_version,
        OVERLAY,
        "folder_a",
    )
    .await
    else {
        return;
    };
    let hello_entry = entry_with_rel(&folder_a_entries, HELLO.entry_rel.rsplit('/').next().unwrap());
    let hello_info = hello_entry.resource_info.expect("hello.txt list_stat resource_info");

    let Some(source_addr) = source_url(storage_fixture, backend, access, HELLO.source_rel) else {
        return;
    };
    let source_info = disabled_client
        .stat(StatRequest {
            resource_address: source_addr,
        })
        .await
        .unwrap()
        .resource_info
        .expect("source stat resource_info");
    // Manifest entry metadata must come from the manifest's source object (size + baked identity,
    // ignoring version-id), not from a backend stat of the (nonexistent) overlay child object.
    assert_resource_info_equivalent(&hello_info, &source_info);

    // --- backend fall-through entry: passthrough.txt vs a direct backend stat ---
    let Some(root_entries) =
        list_stat_entries(storage_fixture, backend, interface, access, api_version, OVERLAY, "").await
    else {
        return;
    };
    let passthrough_entry = entry_with_rel(&root_entries, "passthrough.txt");
    let passthrough_info = passthrough_entry
        .resource_info
        .expect("passthrough.txt list_stat resource_info");

    let Some(passthrough_addr) = overlay_url(storage_fixture, backend, access, "passthrough.txt") else {
        return;
    };
    let backend_info = disabled_client
        .stat(StatRequest {
            resource_address: passthrough_addr,
        })
        .await
        .unwrap()
        .resource_info
        .expect("backend stat resource_info");
    // Backend entry metadata must come from the backend object itself.
    assert_resource_info_equivalent(&passthrough_info, &backend_info);
}

// Small enumerate test - selected by the nextest small profile.
#[rstest]
#[case::case_01_folder_a(OVERLAY, "folder_a", &["hello.txt", "goodbye.txt", "nested/bee.txt"])]
#[test_log::test(tokio::test)]
async fn enumerate_basic(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3)] backend: Backend,
    #[values(Interface::Grpc)] interface: Interface,
    #[values(Access::Public)] access: Access,
    #[values(ApiVersion::V1Alpha)] api_version: ApiVersion,
    #[case] workspace: &str,
    #[case] prefix: &str,
    #[case] expected_sources: &[&str],
) {
    enumerate_sources_impl(
        storage_fixture,
        backend,
        interface,
        access,
        api_version,
        workspace,
        prefix,
        expected_sources,
    )
    .await;
}

// Full enumerate test - selected by the nextest full profile.
#[rstest]
#[case::case_01_folder_a(OVERLAY, "folder_a", &["hello.txt", "goodbye.txt", "nested/bee.txt"])]
#[case::case_02_manifest_only_pkg(MANIFEST_ONLY, "pkg", &["hello.txt", "goodbye.txt", "nested/bee.txt"])]
#[test_log::test(tokio::test)]
async fn enumerate_full(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::Public, Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[case] workspace: &str,
    #[case] prefix: &str,
    #[case] expected_sources: &[&str],
) {
    enumerate_sources_impl(
        storage_fixture,
        backend,
        interface,
        access,
        api_version,
        workspace,
        prefix,
        expected_sources,
    )
    .await;
}

async fn enumerate_sources_impl(
    storage_fixture: &StorageFixture,
    backend: Backend,
    interface: Interface,
    access: Access,
    api_version: ApiVersion,
    workspace: &str,
    prefix: &str,
    expected_sources: &[&str],
) {
    let Some(workspace_base) = workspace_url(storage_fixture, backend, access, workspace) else {
        return;
    };
    let Some(prefix_url) = child_url(storage_fixture, backend, access, workspace, prefix) else {
        return;
    };
    let mut manifest_client = storage_fixture
        .file_object_client(backend, Cache::Manifest, interface, api_version)
        .await;
    let mut stream = manifest_client
        .enumerate(EnumerateRequest {
            resource_address: prefix_url,
        })
        .await
        .unwrap();
    let labels = collect_enumerate_labels(&mut stream, storage_fixture, backend, access, &workspace_base).await;
    assert_eq!(labels, expected_source_labels(expected_sources));
}

#[rstest]
#[test_log::test(tokio::test)]
async fn enumerate_root_full(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::Public, Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
) {
    let Some(workspace_base) = workspace_url(storage_fixture, backend, access, OVERLAY) else {
        return;
    };
    let mut manifest_client = storage_fixture
        .file_object_client(backend, Cache::Manifest, interface, api_version)
        .await;
    let mut stream = manifest_client
        .enumerate(EnumerateRequest {
            resource_address: workspace_base.clone(),
        })
        .await
        .unwrap();
    let labels = collect_enumerate_labels(&mut stream, storage_fixture, backend, access, &workspace_base).await;
    assert_eq!(labels, expected_overlay_enumerate());
}

// Cross-source pagination (plan #13). With a tiny `[manifest] page_size_limit`, the manifest
// runner splits list / list_stat / enumerate over many pages that straddle the
// backend->manifest boundary. The union of all those pages (following the composite
// continuation token, which both the gRPC and REST test clients drain transparently) must
// equal the single-page result produced by the normal-page manifest runner — with no
// duplicates (every count == 1) and no skips (identical key sets).
//
// Small profile: gRPC, public, v1alpha (one cheap case).
#[rstest]
#[test_log::test(tokio::test)]
async fn cross_source_pagination_basic(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3)] backend: Backend,
    #[values(Interface::Grpc)] interface: Interface,
    #[values(Access::Public)] access: Access,
    #[values(ApiVersion::V1Alpha)] api_version: ApiVersion,
) {
    cross_source_pagination_impl(storage_fixture, backend, interface, access, api_version).await;
}

// Full profile: gRPC + REST, all read-only access levels, both API versions.
#[rstest]
#[test_log::test(tokio::test)]
async fn cross_source_pagination_full(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::Public, Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
) {
    cross_source_pagination_impl(storage_fixture, backend, interface, access, api_version).await;
}

fn assert_no_duplicates(kind: &str, multiset: &BTreeMap<String, usize>) {
    for (value, count) in multiset {
        assert_eq!(
            *count, 1,
            "paginated {kind} emitted {value:?} {count} times (duplicate across pages)"
        );
    }
}

async fn cross_source_pagination_impl(
    storage_fixture: &StorageFixture,
    backend: Backend,
    interface: Interface,
    access: Access,
    api_version: ApiVersion,
) {
    // List the overlay root: it has both backend objects and manifest entries, so a tiny page
    // size guarantees the boundary is crossed mid-stream.
    let Some(workspace_base) = workspace_url(storage_fixture, backend, access, OVERLAY) else {
        return;
    };
    let Some(folder_url) = child_url(storage_fixture, backend, access, OVERLAY, "") else {
        return;
    };

    // --- list ---
    let mut small_page_folder_client = storage_fixture
        .manifest_small_page_file_folder_client(backend, interface, api_version)
        .await;
    let mut single_page_folder_client = storage_fixture
        .file_folder_client(backend, Cache::Manifest, interface, api_version)
        .await;

    let mut paged_stream = small_page_folder_client
        .list(ListRequest {
            folder: Some(FolderAddress {
                uri: folder_url.clone(),
            }),
        })
        .await
        .unwrap();
    let (paged_folders, paged_files) = collect_list_multiset(&mut paged_stream, &workspace_base).await;

    let mut single_stream = single_page_folder_client
        .list(ListRequest {
            folder: Some(FolderAddress {
                uri: folder_url.clone(),
            }),
        })
        .await
        .unwrap();
    let (single_folders, single_files) = collect_list_multiset(&mut single_stream, &workspace_base).await;

    assert_no_duplicates("list folders", &paged_folders);
    assert_no_duplicates("list files", &paged_files);
    assert_eq!(
        paged_folders, single_folders,
        "paginated list folders != single-page list folders"
    );
    assert_eq!(
        paged_files, single_files,
        "paginated list files != single-page list files"
    );

    // --- list_stat ---
    let mut paged_stream = small_page_folder_client
        .list_stat(ListStatRequest {
            include_native_metadata: false,
            folder: Some(FolderAddress {
                uri: folder_url.clone(),
            }),
        })
        .await
        .unwrap();
    let (paged_folders, paged_files) = collect_list_stat_multiset(&mut paged_stream, &workspace_base).await;

    let mut single_stream = single_page_folder_client
        .list_stat(ListStatRequest {
            include_native_metadata: false,
            folder: Some(FolderAddress {
                uri: folder_url.clone(),
            }),
        })
        .await
        .unwrap();
    let (single_folders, single_files) = collect_list_stat_multiset(&mut single_stream, &workspace_base).await;

    assert_no_duplicates("list_stat folders", &paged_folders);
    assert_no_duplicates("list_stat files", &paged_files);
    assert_eq!(
        paged_folders, single_folders,
        "paginated list_stat folders != single-page list_stat folders"
    );
    assert_eq!(
        paged_files, single_files,
        "paginated list_stat files != single-page list_stat files"
    );

    // --- enumerate (recursive) ---
    let mut small_page_object_client = storage_fixture
        .manifest_small_page_file_object_client(backend, interface, api_version)
        .await;
    let mut single_page_object_client = storage_fixture
        .file_object_client(backend, Cache::Manifest, interface, api_version)
        .await;

    let mut paged_stream = small_page_object_client
        .enumerate(EnumerateRequest {
            resource_address: workspace_base.clone(),
        })
        .await
        .unwrap();
    let paged_labels =
        collect_enumerate_labels(&mut paged_stream, storage_fixture, backend, access, &workspace_base).await;

    let mut single_stream = single_page_object_client
        .enumerate(EnumerateRequest {
            resource_address: workspace_base.clone(),
        })
        .await
        .unwrap();
    let single_labels =
        collect_enumerate_labels(&mut single_stream, storage_fixture, backend, access, &workspace_base).await;

    // enumerate's expected union (see `expected_overlay_enumerate`) legitimately repeats
    // `source:hello.txt` (folder_a/hello.txt and shared/in_manifest.txt both resolve to it), so
    // here we assert the paged multiset equals the single-page multiset rather than count == 1.
    assert_eq!(
        paged_labels, single_labels,
        "paginated enumerate != single-page enumerate (dupes or skips at the source boundary)"
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn absent_workspace_basic(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3)] backend: Backend,
    #[values(Interface::Grpc)] interface: Interface,
    #[values(Access::Public)] access: Access,
    #[values(ApiVersion::V1Alpha)] api_version: ApiVersion,
) {
    absent_workspace_impl(storage_fixture, backend, interface, access, api_version).await;
}

#[rstest]
#[test_log::test(tokio::test)]
async fn absent_workspace_full(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::Public, Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
) {
    absent_workspace_impl(storage_fixture, backend, interface, access, api_version).await;
}

async fn absent_workspace_impl(
    storage_fixture: &StorageFixture,
    backend: Backend,
    interface: Interface,
    access: Access,
    api_version: ApiVersion,
) {
    let Some(plain_a) = child_url(storage_fixture, backend, access, ABSENT, "plain_a.txt") else {
        return;
    };
    let mut manifest_client = storage_fixture
        .file_object_client(backend, Cache::Manifest, interface, api_version)
        .await;
    let (_, body) = read_from_address_bytes(&mut manifest_client, plain_a, DownloadPreference::Body)
        .await
        .unwrap();
    assert_data_eq(b"absent-a\n", &body);

    let Some(missing) = child_url(storage_fixture, backend, access, ABSENT, "missing.txt") else {
        return;
    };
    assert_read_from_address_status(&mut manifest_client, missing, Code::NotFound).await;

    // Enumerate of an absent workspace (plan #16): no `.manifest`, so it is a pure backend
    // recursive listing. All addresses live under the workspace base, so every label is a
    // `workspace:` label.
    let Some(absent_base) = workspace_url(storage_fixture, backend, access, ABSENT) else {
        return;
    };
    let mut stream = manifest_client
        .enumerate(EnumerateRequest {
            resource_address: absent_base.clone(),
        })
        .await
        .unwrap();
    let labels = collect_enumerate_labels(&mut stream, storage_fixture, backend, access, &absent_base).await;
    assert_eq!(
        labels,
        url_multiset([
            "workspace:plain_a.txt".to_string(),
            "workspace:plain_b.txt".to_string(),
            "workspace:sub/plain_c.txt".to_string(),
        ])
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn invalid_workspace_fail_closed_basic(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3)] backend: Backend,
    #[values(Interface::Grpc)] interface: Interface,
    #[values(Access::Public)] access: Access,
    #[values(ApiVersion::V1Alpha)] api_version: ApiVersion,
) {
    invalid_workspace_impl(storage_fixture, backend, interface, access, api_version).await;
}

#[rstest]
#[test_log::test(tokio::test)]
async fn invalid_workspace_fail_closed_full(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::Public, Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
) {
    invalid_workspace_impl(storage_fixture, backend, interface, access, api_version).await;
}

async fn invalid_workspace_impl(
    storage_fixture: &StorageFixture,
    backend: Backend,
    interface: Interface,
    access: Access,
    api_version: ApiVersion,
) {
    let Some(real_file) = child_url(storage_fixture, backend, access, INVALID, "real_file.txt") else {
        return;
    };
    let Some(folder) = workspace_url(storage_fixture, backend, access, INVALID) else {
        return;
    };
    let Some(manifest_file) = child_url(storage_fixture, backend, access, INVALID, ".manifest") else {
        return;
    };

    let mut file_client = storage_fixture
        .file_object_client(backend, Cache::Manifest, interface, api_version)
        .await;
    let mut folder_client = storage_fixture
        .file_folder_client(backend, Cache::Manifest, interface, api_version)
        .await;

    assert_read_from_address_status(&mut file_client, real_file.clone(), Code::InvalidArgument).await;
    assert_eq!(
        file_client
            .stat(StatRequest {
                resource_address: real_file,
            })
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
    assert_eq!(
        folder_client
            .list(ListRequest {
                folder: Some(FolderAddress { uri: folder.clone() }),
            })
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
    assert_enumerate_status(&mut file_client, folder, Code::InvalidArgument).await;

    let (_, manifest_bytes) = read_from_address_bytes(&mut file_client, manifest_file, DownloadPreference::Body)
        .await
        .unwrap();
    assert!(!manifest_bytes.is_empty());
}

// Small enumerate_versions test - selected by the nextest small profile.
#[rstest]
#[case::case_01_nested_bee(BEE)]
#[test_log::test(tokio::test)]
async fn enumerate_versions_basic(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3)] backend: Backend,
    #[values(Interface::Grpc)] interface: Interface,
    #[values(Access::Versioned)] access: Access,
    #[values(ApiVersion::V1Alpha)] api_version: ApiVersion,
    #[case] entry: EntryCase,
) {
    enumerate_versions_impl(storage_fixture, backend, interface, access, api_version, entry).await;
}

// Full enumerate_versions test - selected by the nextest full profile.
#[rstest]
#[case::case_01_nested_bee(BEE)]
#[test_log::test(tokio::test)]
async fn enumerate_versions_full(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::Versioned)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[case] entry: EntryCase,
) {
    enumerate_versions_impl(storage_fixture, backend, interface, access, api_version, entry).await;
}

async fn enumerate_versions_impl(
    storage_fixture: &StorageFixture,
    backend: Backend,
    interface: Interface,
    access: Access,
    api_version: ApiVersion,
    entry: EntryCase,
) {
    // This test requires native backend versioning; the PrivateKey S3 bucket is not
    // versioned, so its enumerations return no usable version ids. Skip.
    if access == Access::PrivateKey && backend == Backend::S3 {
        return;
    }

    let Some(manifest_url) = overlay_url(storage_fixture, backend, access, entry.entry_rel) else {
        return;
    };
    let Some(source_url) = source_url(storage_fixture, backend, access, entry.source_rel) else {
        return;
    };
    let mut manifest_client = storage_fixture
        .versioning_client(backend, Cache::Manifest, interface, api_version)
        .await;
    let mut regular_client = storage_fixture
        .versioning_client(backend, Cache::Disabled, interface, api_version)
        .await;

    let mut manifest_stream = manifest_client
        .enumerate_versions(EnumerateVersionsRequest {
            resource_address: manifest_url,
        })
        .await
        .unwrap();

    // When the operator hard-disables versioning (`versioning_enabled = false`) the call
    // short-circuits with FAILED_PRECONDITION; verify only the manifest results then (the
    // manifest layer provides versioning even without native backend support — that is
    // its purpose). The test fixtures leave the config unset, so the stream path is the
    // one normally taken.
    let regular_stream = match regular_client
        .enumerate_versions(EnumerateVersionsRequest {
            resource_address: source_url,
        })
        .await
    {
        Ok(stream) => Some(stream),
        Err(e) if e.code() == tonic::Code::FailedPrecondition => None,
        Err(e) => panic!("unexpected error from regular enumerate_versions: {e}"),
    };

    let mut manifest_items = Vec::new();
    while let Some(response) = manifest_stream.next().await {
        manifest_items.extend(response.unwrap().items);
    }

    let mut regular_items = Vec::new();
    if let Some(mut stream) = regular_stream {
        while let Some(response) = stream.next().await {
            regular_items.extend(response.unwrap().items);
        }
    }

    tracing::debug!(?manifest_items, ?regular_items);

    // When the backend has versioning disabled the regular client returns FAILED_PRECONDITION
    // and regular_items is empty. In that case just verify the manifest returned items.
    if regular_items.is_empty() {
        assert!(
            !manifest_items.is_empty(),
            "manifest should return items even on non-versioned backends"
        );
        return;
    }

    assert_eq!(manifest_items.len(), regular_items.len());
    for (manifest_item, regular_item) in manifest_items.iter().zip(regular_items.iter()) {
        assert_resource_info_equivalent(
            manifest_item.resource_info.as_ref().unwrap(),
            regular_item.resource_info.as_ref().unwrap(),
        );
    }
}

#[rstest]
#[test_log::test(tokio::test)]
async fn mutation_activation_basic(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3)] backend: Backend,
    #[values(Interface::Grpc)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(ApiVersion::V1Alpha)] api_version: ApiVersion,
) {
    mutation_activation_impl(storage_fixture, backend, interface, access, api_version).await;
}

#[rstest]
#[test_log::test(tokio::test)]
async fn mutation_activation_full(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
) {
    mutation_activation_impl(storage_fixture, backend, interface, access, api_version).await;
}

async fn mutation_activation_impl(
    storage_fixture: &StorageFixture,
    backend: Backend,
    interface: Interface,
    access: Access,
    api_version: ApiVersion,
) {
    let workspace = unique_workspace_name("activation");
    let Some(manifest_file) = write_workspace_url(storage_fixture, backend, access, &workspace, ".manifest") else {
        return;
    };
    let Some(entry_file) = write_workspace_url(storage_fixture, backend, access, &workspace, "entry.txt") else {
        return;
    };
    let Some(free_file) = write_workspace_url(storage_fixture, backend, access, &workspace, "free.txt") else {
        return;
    };
    let Some(old_file) = write_workspace_url(storage_fixture, backend, access, &workspace, "old.txt") else {
        return;
    };
    let Some(new_file) = write_workspace_url(storage_fixture, backend, access, &workspace, "new.txt") else {
        return;
    };
    let hello_identity = source_identity(storage_fixture, backend, access, api_version, HELLO.source_rel)
        .await
        .unwrap();
    let goodbye_identity = source_identity(storage_fixture, backend, access, api_version, GOODBYE.source_rel)
        .await
        .unwrap();

    let mut manifest_client = storage_fixture
        .file_object_client(backend, Cache::Manifest, interface, api_version)
        .await;

    assert_read_from_address_status(&mut manifest_client, entry_file.clone(), Code::NotFound).await;
    write_data_body(
        &manifest_file,
        &manifest_json(&[("entry.txt", &hello_identity)]),
        None,
        None,
        &mut manifest_client,
    )
    .await;
    let (_, body) = read_from_address_bytes(&mut manifest_client, entry_file.clone(), DownloadPreference::Body)
        .await
        .unwrap();
    assert_data_eq(b"manifest-hello\n", &body);

    assert_eq!(
        manifest_client
            .write(
                WriteParameters {
                    native_metadata: Default::default(),
                    destination_resource_address: entry_file.clone(),
                    previous_version: None,
                    data_object_size: 5,
                    upload_preference: Some(UploadPreference::Body.into()),
                    fail_if_exists: None,
                },
                Some(Bytes::from_static(b"nope\n")),
            )
            .await
            .unwrap_err()
            .code(),
        Code::PermissionDenied
    );
    // plan #24: the rejected write must not have touched the backend object — re-reading the
    // in-manifest path still serves the manifest content (`manifest-hello`), not `nope`.
    let (_, body) = read_from_address_bytes(&mut manifest_client, entry_file.clone(), DownloadPreference::Body)
        .await
        .unwrap();
    assert_data_eq(b"manifest-hello\n", &body);

    write_data_body(&free_file, b"fallthrough\n", None, None, &mut manifest_client).await;
    let (_, body) = read_from_address_bytes(&mut manifest_client, free_file.clone(), DownloadPreference::Body)
        .await
        .unwrap();
    assert_data_eq(b"fallthrough\n", &body);

    // plan #25: a written non-manifest (fall-through) file appears in a listing of the workspace
    // alongside the visible `.manifest` and the manifest-synthesized entry.
    let workspace_base = write_workspace_url(storage_fixture, backend, access, &workspace, "").unwrap();
    let mut folder_client = storage_fixture
        .file_folder_client(backend, Cache::Manifest, interface, api_version)
        .await;
    let mut list_stream = folder_client
        .list(ListRequest {
            folder: Some(FolderAddress {
                uri: workspace_base.clone(),
            }),
        })
        .await
        .unwrap();
    let (_, listed_files) = collect_list(&mut list_stream, workspace_base.trim_end_matches('/')).await;
    assert!(
        listed_files.contains("free.txt"),
        "written fall-through file should be listed, got: {listed_files:?}"
    );

    write_data_body(
        &manifest_file,
        &manifest_json(&[("new.txt", &goodbye_identity)]),
        None,
        None,
        &mut manifest_client,
    )
    .await;
    let (_, body) = read_from_address_bytes(&mut manifest_client, new_file.clone(), DownloadPreference::Body)
        .await
        .unwrap();
    assert_data_eq(b"manifest-goodbye\n", &body);
    assert_read_from_address_status(&mut manifest_client, entry_file.clone(), Code::NotFound).await;

    delete_file(&manifest_file, None, None, &mut manifest_client).await;
    assert_read_from_address_status(&mut manifest_client, new_file.clone(), Code::NotFound).await;
    // plan #26: deleting `.manifest` removes the overlay, but the real `.wkspc` objects written
    // into the workspace (here the fall-through `free.txt`) survive and still read.
    let (_, body) = read_from_address_bytes(&mut manifest_client, free_file.clone(), DownloadPreference::Body)
        .await
        .unwrap();
    assert_data_eq(b"fallthrough\n", &body);
    write_data_body(
        &manifest_file,
        &manifest_json(&[("old.txt", &hello_identity)]),
        None,
        None,
        &mut manifest_client,
    )
    .await;
    assert_eq!(
        manifest_client
            .delete(DeleteRequest {
                resource_address: old_file.clone(),
                previous_version: None,
            })
            .await
            .unwrap_err()
            .code(),
        Code::PermissionDenied
    );
    delete_file(&free_file, None, None, &mut manifest_client).await;
    assert_read_from_address_status(&mut manifest_client, free_file, Code::NotFound).await;
    delete_file(&manifest_file, None, None, &mut manifest_client).await;
}

#[rstest]
#[test_log::test(tokio::test)]
async fn redirect_and_multipart_manifest_writes_activate_full(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
) {
    let hello_identity = source_identity(storage_fixture, backend, access, api_version, HELLO.source_rel)
        .await
        .unwrap();
    let goodbye_identity = source_identity(storage_fixture, backend, access, api_version, GOODBYE.source_rel)
        .await
        .unwrap();
    let workspace = unique_workspace_name("redirect-multipart");
    let Some(manifest_file) = write_workspace_url(storage_fixture, backend, access, &workspace, ".manifest") else {
        return;
    };
    let Some(redirect_entry) = write_workspace_url(storage_fixture, backend, access, &workspace, "redirect.txt") else {
        return;
    };
    let Some(multipart_entry) = write_workspace_url(storage_fixture, backend, access, &workspace, "multipart.txt")
    else {
        return;
    };

    let mut manifest_client = storage_fixture
        .file_object_client(backend, Cache::Manifest, interface, api_version)
        .await;

    write_data_redirect(
        &manifest_file,
        &manifest_json(&[("redirect.txt", &hello_identity)]),
        None,
        None,
        &mut manifest_client,
        backend,
    )
    .await;
    let (_, body) = read_from_address_bytes(&mut manifest_client, redirect_entry, DownloadPreference::Body)
        .await
        .unwrap();
    assert_data_eq(b"manifest-hello\n", &body);

    write_data_multipart(
        &manifest_file,
        &manifest_json_at_least(&[("multipart.txt", &goodbye_identity)], 10 * 1024 * 1024 + 1),
        2,
        None,
        None,
        &mut manifest_client,
    )
    .await;
    let (_, body) = read_from_address_bytes(&mut manifest_client, multipart_entry, DownloadPreference::Body)
        .await
        .unwrap();
    assert_data_eq(b"manifest-goodbye\n", &body);

    delete_file(&manifest_file, None, None, &mut manifest_client).await;
}

#[rstest]
#[test_log::test(tokio::test)]
async fn v1alpha_copy_move_manifest_behaviors_full(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(ApiVersion::V1Alpha)] api_version: ApiVersion,
) {
    let hello_identity = source_identity(storage_fixture, backend, access, api_version, HELLO.source_rel)
        .await
        .unwrap();
    let goodbye_identity = source_identity(storage_fixture, backend, access, api_version, GOODBYE.source_rel)
        .await
        .unwrap();
    let workspace_a = unique_workspace_name("move-a");
    let workspace_b = unique_workspace_name("move-b");
    let Some(manifest_a) = write_workspace_url(storage_fixture, backend, access, &workspace_a, ".manifest") else {
        return;
    };
    let Some(manifest_b) = write_workspace_url(storage_fixture, backend, access, &workspace_b, ".manifest") else {
        return;
    };
    let Some(entry_a) = write_workspace_url(storage_fixture, backend, access, &workspace_a, "entry.txt") else {
        return;
    };
    let Some(entry_b) = write_workspace_url(storage_fixture, backend, access, &workspace_b, "entry.txt") else {
        return;
    };
    let Some(locked_b) = write_workspace_url(storage_fixture, backend, access, &workspace_b, "locked.txt") else {
        return;
    };
    let Some(real_source) = write_workspace_url(storage_fixture, backend, access, &workspace_b, "real-source.txt")
    else {
        return;
    };

    let mut manifest_client = storage_fixture
        .file_object_client(backend, Cache::Manifest, interface, api_version)
        .await;
    write_data_body(
        &manifest_a,
        &manifest_json(&[("entry.txt", &hello_identity)]),
        None,
        None,
        &mut manifest_client,
    )
    .await;
    let (_, body) = read_from_address_bytes(&mut manifest_client, entry_a.clone(), DownloadPreference::Body)
        .await
        .unwrap();
    assert_data_eq(b"manifest-hello\n", &body);

    assert_read_from_address_status(&mut manifest_client, entry_b.clone(), Code::NotFound).await;
    move_file(&manifest_a, &manifest_b, None, None, None, &mut manifest_client).await;
    assert_read_from_address_status(&mut manifest_client, entry_a, Code::NotFound).await;
    let (_, body) = read_from_address_bytes(&mut manifest_client, entry_b, DownloadPreference::Body)
        .await
        .unwrap();
    assert_data_eq(b"manifest-hello\n", &body);

    write_data_body(
        &manifest_b,
        &manifest_json(&[("locked.txt", &goodbye_identity)]),
        None,
        None,
        &mut manifest_client,
    )
    .await;
    let source_identity = ResourceIdentity {
        encoded_identity: hello_identity,
    };
    assert_eq!(
        manifest_client
            .copy(CopyRequest {
                source_resource_identity: Some(source_identity),
                destination_resource_address: locked_b.clone(),
                previous_version: None,
                fail_if_exists: None,
            })
            .await
            .unwrap_err()
            .code(),
        Code::PermissionDenied
    );

    write_data_body(&real_source, b"real-source\n", None, None, &mut manifest_client).await;
    assert_eq!(
        manifest_client
            .move_object(MoveRequest {
                source_resource_address: real_source.clone(),
                source_previous_version: None,
                destination_resource_address: locked_b,
                destination_previous_version: None,
                fail_if_exists: None,
            })
            .await
            .unwrap_err()
            .code(),
        Code::PermissionDenied
    );

    delete_file(&manifest_b, None, None, &mut manifest_client).await;
    delete_file(&real_source, None, None, &mut manifest_client).await;
}
