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

use std::collections::BTreeSet;

use common::{
    clients::filefolder::types::v1beta::{FolderAddress, ListRequest, ListStatRequest},
    random::get_random_path_item,
    service::{Builder, Service as StorageService},
    types::Interface,
};
use rstest::{fixture, rstest};
use serde_json::{json, Value};

const CHILD_COUNT: usize = 4;

struct TestService {
    service: StorageService,
    parent_mapped: String,
    child_mapped_urls: Vec<String>,
}

fn make_service() -> TestService {
    let tag = format!("syn-{}", get_random_path_item());
    let env = common::env::get_env().unwrap_s3();
    let parent_mapped = format!("omniverse://myhost.mydomain/{tag}/parent/");
    let parent_original = env.resource_url(format!("{tag}-parent/"));
    let mut rules: Vec<(String, String)> = vec![
        (
            "omniverse://myhost.mydomain/".to_string(),
            env.resource_url(format!("{tag}-mydomain-root/")),
        ),
        (
            format!("omniverse://myhost.mydomain/{tag}/"),
            env.resource_url(format!("{tag}-root/")),
        ),
        (parent_mapped.clone(), parent_original),
    ];

    let mut child_mapped_urls = Vec::with_capacity(CHILD_COUNT);
    for idx in 0..CHILD_COUNT {
        let child_mapped = format!("omniverse://myhost.mydomain/{tag}/parent/mount-{idx}/");
        rules.push((child_mapped.clone(), env.resource_url(format!("{tag}-child-{idx}/"))));
        child_mapped_urls.push(child_mapped);
    }

    let service = with_url_remapper_multi(Builder::new().enable_s3(false), rules, true).start();
    TestService {
        service,
        parent_mapped,
        child_mapped_urls,
    }
}

fn with_url_remapper_multi(
    builder: Builder,
    rewrite_rules: impl IntoIterator<Item = (String, String)>,
    expose_child_rules_as_folders: bool,
) -> Builder {
    let rules: serde_json::Map<String, Value> = rewrite_rules
        .into_iter()
        .map(|(m, o)| (m, json!({ "target": o })))
        .collect();
    builder.with_url_remapper_config(json!({
        "enabled": true,
        "expose_child_rules_as_folders": expose_child_rules_as_folders,
        "rewrite_rules": rules,
    }))
}

#[fixture]
#[once]
fn storage_service() -> TestService {
    make_service()
}

async fn ensure_parent_has_a_placeholder(service: &StorageService, parent_mapped: &str) {
    let client = service.fileobject_v1beta_client(Interface::Rest).await;
    let placeholder_uri = format!("{}.synthetic-test-placeholder", parent_mapped);
    let _ = client.write_random_object(placeholder_uri, 1).await;
}

#[derive(Debug, PartialEq, Eq)]
struct ListPageSnapshot {
    subfolders: Vec<String>,
}

async fn drain_list_v1beta(
    service: &StorageService,
    parent_mapped: &str,
    max_page_size: Option<u32>,
) -> Vec<ListPageSnapshot> {
    let client = service.filefolder_v1beta_rest_client();
    let request = ListRequest {
        folder: Some(FolderAddress {
            uri: parent_mapped.to_owned(),
        }),
    };

    let mut continuation_handle: Option<String> = None;
    let mut out = Vec::new();
    loop {
        let (response, next) = client
            .list_once(&request, continuation_handle.as_deref(), max_page_size)
            .await
            .expect("list ok");
        let idx = out.len();
        let size = response.subfolder_addresses.len() + response.sub_resource_addresses.len();
        if let Some(cap) = max_page_size {
            assert!(size <= cap as usize, "page #{idx} ({size} entries) violates cap {cap}");
        }
        out.push(ListPageSnapshot {
            subfolders: response.subfolder_addresses.into_iter().map(|a| a.uri).collect(),
        });
        match next {
            None => break,
            Some(token) => continuation_handle = Some(token),
        }
    }
    out
}

#[derive(Debug, PartialEq, Eq)]
struct ListStatPageSnapshot {
    subfolders: Vec<String>,
    entries: Vec<String>,
}

async fn drain_list_stat_v1beta(
    service: &StorageService,
    parent_mapped: &str,
    max_page_size: Option<u32>,
) -> Vec<ListStatPageSnapshot> {
    let client = service.filefolder_v1beta_rest_client();
    let request = ListStatRequest {
        folder: Some(FolderAddress {
            uri: parent_mapped.to_owned(),
        }),
    };

    let mut continuation_handle: Option<String> = None;
    let mut out = Vec::new();
    loop {
        let (response, next) = client
            .list_stat_once(&request, continuation_handle.as_deref(), max_page_size)
            .await
            .expect("list_stat ok");
        let idx = out.len();
        let size = response.subfolder_addresses.len() + response.entries.len();
        if let Some(cap) = max_page_size {
            assert!(size <= cap as usize, "page #{idx} ({size} entries) violates cap {cap}");
        }
        out.push(ListStatPageSnapshot {
            subfolders: response.subfolder_addresses.into_iter().map(|a| a.uri).collect(),
            entries: response.entries.into_iter().map(|e| e.resource_address).collect(),
        });
        match next {
            None => break,
            Some(token) => continuation_handle = Some(token),
        }
    }
    out
}

/// v1beta mirror of `list_emits_every_synthetic_child_exactly_once`.
#[rstest]
#[case::no_cap(None, vec![4])]
#[case::cap_5(Some(5), vec![4])]
#[case::cap_3(Some(3), vec![2, 2])]
#[case::cap_2(Some(2), vec![1, 2, 1])]
#[case::cap_1(Some(1), vec![0, 1, 1, 1, 1])]
#[test_log::test(tokio::test)]
async fn list_emits_every_synthetic_child_exactly_once(
    storage_service: &TestService,
    #[case] max_page_size: Option<u32>,
    #[case] expected_pages: Vec<usize>,
) {
    let TestService {
        service,
        parent_mapped,
        child_mapped_urls: expected_children,
    } = storage_service;
    ensure_parent_has_a_placeholder(service, parent_mapped).await;

    let pages = drain_list_v1beta(service, parent_mapped, max_page_size).await;

    assert_eq!(pages.len(), expected_pages.len());
    for (page, expected_subfolders) in pages.iter().zip(&expected_pages) {
        assert_eq!(page.subfolders.len(), *expected_subfolders);
    }
    let all_subfolders: BTreeSet<String> = pages.iter().flat_map(|p| p.subfolders.iter().cloned()).collect();
    let expected_set: BTreeSet<String> = expected_children.iter().cloned().collect();
    assert_eq!(all_subfolders, expected_set);
}

/// `list_stat` counterpart of `list_emits_every_synthetic_child_exactly_once`.
/// Uses Streamed mode (no max_page_size) because S3 list_stat uses
/// ListObjectVersions, which does not support the paginated path on rustfs.
/// Pagination with synthetic injection is already covered by the `list` test.
#[rstest]
#[test_log::test(tokio::test)]
async fn list_stat_emits_every_synthetic_child_exactly_once(storage_service: &TestService) {
    let TestService {
        service,
        parent_mapped,
        child_mapped_urls: expected_children,
    } = storage_service;
    ensure_parent_has_a_placeholder(service, parent_mapped).await;
    let placeholder_uri = format!("{}.synthetic-test-placeholder", parent_mapped);

    let pages = drain_list_stat_v1beta(service, parent_mapped, None).await;

    let all_subfolders: BTreeSet<String> = pages.iter().flat_map(|p| p.subfolders.iter().cloned()).collect();
    let expected_set: BTreeSet<String> = expected_children.iter().cloned().collect();
    assert_eq!(all_subfolders, expected_set);
    assert_eq!(
        pages
            .iter()
            .flat_map(|p| p.entries.iter())
            .filter(|f| **f == placeholder_uri)
            .count(),
        1
    );
}

/// v1beta mirror of `list_with_and_without_trailing_slash_matches`.
#[test_log::test(tokio::test)]
async fn list_with_and_without_trailing_slash_matches() {
    let TestService {
        service,
        parent_mapped,
        child_mapped_urls: expected_children,
    } = make_service();
    ensure_parent_has_a_placeholder(&service, &parent_mapped).await;

    let without_trailing = parent_mapped.trim_end_matches('/').to_owned();

    let pages_with = drain_list_v1beta(&service, &parent_mapped, None).await;
    let pages_without = drain_list_v1beta(&service, &without_trailing, None).await;

    let subfolders_with: BTreeSet<String> = pages_with.iter().flat_map(|p| p.subfolders.iter().cloned()).collect();
    let subfolders_without: BTreeSet<String> = pages_without
        .iter()
        .flat_map(|p| p.subfolders.iter().cloned())
        .collect();
    assert_eq!(subfolders_with, subfolders_without);
    let expected: BTreeSet<String> = expected_children.iter().cloned().collect();
    assert_eq!(subfolders_with, expected);
}

#[test_log::test(tokio::test)]
async fn list_does_not_inject_when_feature_is_off() {
    let tag = format!("syn-{}", get_random_path_item());
    let env = common::env::get_env().unwrap_s3();
    let parent_mapped = format!("omniverse://myhost.mydomain/{tag}/parent/");
    let parent_original = env.resource_url(format!("{tag}-parent/"));
    let rules = vec![
        (parent_mapped.clone(), parent_original),
        (
            format!("omniverse://myhost.mydomain/{tag}/parent/mount-0/"),
            env.resource_url(format!("{tag}-child-0/")),
        ),
    ];
    let service = with_url_remapper_multi(Builder::new().enable_s3(false), rules, false).start();
    ensure_parent_has_a_placeholder(&service, &parent_mapped).await;
    let client = service.filefolder_v1beta_rest_client();
    let request = ListRequest {
        folder: Some(FolderAddress {
            uri: parent_mapped.clone(),
        }),
    };
    let synthetic = format!("omniverse://myhost.mydomain/{tag}/parent/mount-0/");

    let (response, _) = client.list_once(&request, None, None).await.expect("list ok");

    assert!(response.subfolder_addresses.iter().all(|a| a.uri != synthetic));
}
