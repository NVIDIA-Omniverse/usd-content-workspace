// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

pub mod v1alpha;
pub mod v1beta;

use storage_core_util::trace_limited;

use super::{
    format::{encode_relative_path, workspace_suffix_after_prefix, Manifest, ManifestEntryValue, WorkspaceKey},
    ManifestService,
};

use crate::traits::{error::Result, BoxStream, StreamingOptions};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use unicode_normalization::UnicodeNormalization;

pub struct ListItem {
    // The resource address of the list item relative to the manifest path
    pub resource_address: String,
    // The manifest entry value (either minimal or full variant)
    pub entry_value: ManifestEntryValue,
}

/// Helper data for list operations that separates files from subfolders
pub struct ListData {
    /// Direct child file URLs with metadata
    pub file_urls: Vec<ListItem>,
    /// Subfolder addresses
    pub subfolder_addresses: Vec<String>,
}

impl ListData {
    pub fn len(&self) -> usize {
        self.file_urls.len() + self.subfolder_addresses.len()
    }

    pub fn is_empty(&self) -> bool {
        self.file_urls.is_empty() && self.subfolder_addresses.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum WorkspaceListToken {
    Backend { cursor: Option<String> },
    Manifest { index: usize },
}

impl WorkspaceListToken {
    pub fn from_continuation_handle(continuation_handle: Option<String>) -> Result<Self> {
        match continuation_handle {
            None => Ok(Self::Backend { cursor: None }),
            Some(token) => serde_json::from_str(&token).map_err(|e| {
                super::error::Error::invalid_argument(format!("Invalid workspace list token: {}", e)).into()
            }),
        }
    }

    // These build the same internally-tagged JSON that `from_continuation_handle` parses, but via
    // `serde_json::Value::to_string`, which is infallible (unlike `to_string(&struct)`, which returns
    // a `Result` we'd have to `.expect()` — forbidden on this long-running hot path). Key order is
    // irrelevant: deserialization keys off the `phase` tag regardless of order.
    pub fn backend_continue(cursor: Option<String>) -> Option<String> {
        cursor.map(|cursor| serde_json::json!({ "phase": "backend", "cursor": cursor }).to_string())
    }

    pub fn manifest_continue(index: usize) -> String {
        serde_json::json!({ "phase": "manifest", "index": index }).to_string()
    }
}

impl<S, R> ManifestService<S, R> {
    /// Single-pass projection of the cached manifest for a `list`/`list_stat` of `workspace_key`'s
    /// folder.
    ///
    /// Produces both the direct-child [`ListData`] (files + synthesized subfolders) **and** the
    /// shadow-name set (direct-child names used to drop colliding backend entries) in one pass, and
    /// **consumes** the manifest so each entry value is moved into the result rather than cloned per
    /// page (#9). Child addresses are percent-encoded so they round-trip and match the
    /// percent-encoded backend-sourced siblings in the merged listing (#4).
    pub fn list_projection(&self, workspace_key: &WorkspaceKey, manifest: Manifest) -> (ListData, HashSet<String>) {
        // Normalize the object_path to end with "/" for directory listing.
        let prefix = match workspace_key.object_path.as_deref() {
            None | Some("") => String::new(),
            Some(p) if p.ends_with('/') => p.to_string(),
            Some(p) => format!("{}/", p),
        };

        let mut file_urls = Vec::new();
        let mut subfolder_set = HashSet::new();
        let mut shadow_names = HashSet::new();

        for (relative_path, entry_value) in manifest.into_inner() {
            // Only entries under our prefix.
            if !prefix.is_empty() && !relative_path.starts_with(&prefix) {
                continue;
            }
            let relative_to_prefix = &relative_path[prefix.len()..];
            if relative_to_prefix.is_empty() {
                continue;
            }

            match relative_to_prefix.find('/') {
                // In a subfolder: record the synthesized subfolder address + its name in the shadow set.
                Some(slash_pos) => {
                    let subfolder_name = &relative_to_prefix[..slash_pos];
                    shadow_names.insert(subfolder_name.to_string());
                    subfolder_set.insert(format!(
                        "{}/{}",
                        workspace_key.workspace_path,
                        encode_relative_path(&format!("{prefix}{subfolder_name}/"))
                    ));
                },
                // A direct child file: move the entry value into the result (no clone).
                None => {
                    shadow_names.insert(relative_to_prefix.to_string());
                    file_urls.push(ListItem {
                        resource_address: format!(
                            "{}/{}",
                            workspace_key.workspace_path,
                            encode_relative_path(&relative_path)
                        ),
                        entry_value,
                    });
                },
            }
        }

        file_urls.sort_by(|a, b| a.resource_address.cmp(&b.resource_address));
        let mut subfolder_addresses: Vec<_> = subfolder_set.into_iter().collect();
        subfolder_addresses.sort();

        (
            ListData {
                file_urls,
                subfolder_addresses,
            },
            shadow_names,
        )
    }

    pub fn direct_child_name_from_address(parent_object_path: Option<&str>, child_address: &str) -> Option<String> {
        let child_key = WorkspaceKey::parse(child_address).ok().flatten()?;
        Self::direct_child_name_from_object_path(parent_object_path, child_key.object_path.as_deref())
    }

    /// Like [`direct_child_name_from_address`](Self::direct_child_name_from_address) but takes the
    /// request's constant `workspace_path` so the child name can be derived with a cheap
    /// prefix-strip instead of a full `Url::parse` per backend entry on the listing hot path (#10).
    /// Falls back to the parser for addresses that don't sit under the workspace prefix (or carry a
    /// query/fragment) so dedup stays correct.
    pub fn direct_child_name_from_workspace_address(
        workspace_path: &str,
        parent_object_path: Option<&str>,
        child_address: &str,
    ) -> Option<String> {
        if let Some(child_object_path) = workspace_suffix_after_prefix(workspace_path, child_address) {
            return Self::direct_child_name_from_object_path(parent_object_path, Some(&child_object_path));
        }
        Self::direct_child_name_from_address(parent_object_path, child_address)
    }

    pub fn direct_child_name_from_object_path(
        parent_object_path: Option<&str>,
        child_object_path: Option<&str>,
    ) -> Option<String> {
        let child_path = normalize_workspace_path(child_object_path.unwrap_or(""));
        let parent_path = normalize_workspace_path(parent_object_path.unwrap_or(""));
        let parent_prefix = if parent_path.is_empty() {
            String::new()
        } else if parent_path.ends_with('/') {
            parent_path
        } else {
            format!("{parent_path}/")
        };

        let relative = if parent_prefix.is_empty() {
            child_path.as_str()
        } else {
            child_path.strip_prefix(&parent_prefix)?
        };

        relative
            .trim_end_matches('/')
            .split('/')
            .next()
            .filter(|child| !child.is_empty())
            .map(ToString::to_string)
    }

    /// Common streaming implementation for list and list_stat operations.
    ///
    /// This function handles both streaming and paginated responses:
    /// - For `StreamingOptions::Streamed`: Batches items by size until page_size_limit is reached
    /// - For `StreamingOptions::Paginated`: Uses skip/take based on continuation_handle and max_page_size
    ///
    /// # Parameters
    /// - `list_data`: The list data containing folders and file items
    /// - `streaming`: The streaming options (Streamed or Paginated)
    /// - `item_function`: Converts a ListItem to (Either<Folder, Entry>, size)
    /// - `folder_function`: Converts a subfolder address to (Folder, size)
    /// - `response_function`: Creates the response from folders, entries, and continue_token
    pub fn stream_list_response<T, U, V, FItem, FFolder, FResponse>(
        &self,
        list_data: ListData,
        streaming: StreamingOptions,
        item_function: FItem,
        folder_function: FFolder,
        response_function: FResponse,
    ) -> BoxStream<T>
    where
        T: Send + 'static,
        U: Send + 'static,
        V: Send + 'static,
        FItem: Fn(&ListItem) -> (V, u64) + Send + Sync + 'static,
        FFolder: Fn(&str) -> (U, u64) + Send + Sync + 'static,
        FResponse: Fn(Vec<U>, Vec<V>, Option<String>) -> T + Send + Sync + 'static,
    {
        let page_size_limit = self.page_size_limit.as_u64();

        match streaming {
            StreamingOptions::Streamed => Box::pin(async_stream::stream! {
                let mut folders = Vec::new();
                let mut entries = Vec::new();
                let mut response_size = 0u64;

                // Process subfolders first
                for subfolder in &list_data.subfolder_addresses {
                    let (folder, size) = folder_function(subfolder);

                    // Check if adding this item would exceed the page size limit
                    if response_size > 0 && response_size + size >= page_size_limit {
                        trace_limited!(
                            response_size,
                            folders = folders.len(),
                            entries = entries.len(),
                            "page size limit reached"
                        );
                        yield Ok(response_function(folders, entries, None));
                        response_size = 0;
                        folders = Vec::new();
                        entries = Vec::new();
                    }

                    response_size += size;
                    folders.push(folder);
                }

                // Process file items
                for item in &list_data.file_urls {
                    let (entry, size) = item_function(item);

                    // Check if adding this item would exceed the page size limit
                    if response_size > 0 && response_size + size >= page_size_limit {
                        trace_limited!(
                            response_size,
                            folders = folders.len(),
                            entries = entries.len(),
                            "page size limit reached"
                        );
                        yield Ok(response_function(folders, entries, None));
                        response_size = 0;
                        folders = Vec::new();
                        entries = Vec::new();
                    }

                    response_size += size;
                    entries.push(entry);
                }

                // Send final response
                trace_limited!(
                    response_size,
                    folders = folders.len(),
                    entries = entries.len(),
                    "final page response"
                );
                yield Ok(response_function(folders, entries, None));
            }),
            StreamingOptions::Paginated(pagination) => {
                let skip = pagination
                    .continuation_handle
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(0);
                let take = pagination
                    .max_page_size
                    .map(|page_size| page_size as usize)
                    .unwrap_or(2000)
                    .min(2000);
                Box::pin(async_stream::stream! {
                    // Combine all items into a single iterator (folders first, then entries)
                    let total_items = list_data.subfolder_addresses.len() + list_data.file_urls.len();
                    let folder_count = list_data.subfolder_addresses.len();

                    let mut folders = Vec::new();
                    let mut entries = Vec::new();
                    let mut continue_index = None;

                    for (index, _) in (0..total_items).enumerate().skip(skip).take(take) {
                        if index < folder_count {
                            let (folder, _) = folder_function(&list_data.subfolder_addresses[index]);
                            folders.push(folder);
                        } else {
                            let (entry, _) = item_function(&list_data.file_urls[index - folder_count]);
                            entries.push(entry);
                        }
                        continue_index = Some(index + 1);
                    }

                    // If we reached the end, no continue token
                    let continue_token = continue_index
                        .filter(|index| *index < total_items)
                        .map(|index| index.to_string());

                    trace_limited!(?continue_token, folders = folders.len(), entries = entries.len());

                    yield Ok(response_function(folders, entries, continue_token));
                })
            },
        }
    }
}

pub fn normalize_workspace_path(path: &str) -> String {
    path.nfc().collect()
}

pub fn is_not_found_error(error: &crate::traits::StorageError) -> bool {
    error.tonic_status().code() == tonic::Code::NotFound
}

#[cfg(test)]
mod tests {
    use bytesize::ByteSize;
    use futures::StreamExt;

    use super::*;
    use crate::manifest::format::WorkspaceKey;
    use crate::manifest::test_utils::{self, create_test_list_data};
    use crate::traits::{Pagination, StreamingOptions};

    /// A simple response type for testing list operations
    #[derive(Debug, Clone, PartialEq)]
    struct TestListResponse {
        folders: Vec<String>,
        entries: Vec<String>,
        continue_token: Option<String>,
    }

    /// Converts test utilities list data to the module's ListData type
    fn make_list_data(num_folders: usize, num_files: usize, file_prefix: &str) -> ListData {
        let (subfolder_addresses, test_items) = create_test_list_data(num_folders, num_files, file_prefix);
        let file_urls = test_items
            .into_iter()
            .map(|item| ListItem {
                resource_address: item.resource_address,
                entry_value: item.entry_value,
            })
            .collect();
        ListData {
            file_urls,
            subfolder_addresses,
        }
    }

    /// Simple item/folder functions for testing that return predictable sizes
    fn test_item_function(item: &ListItem) -> (String, u64) {
        (item.resource_address.clone(), 100) // Fixed size of 100 bytes per item
    }

    fn test_folder_function(folder: &str) -> (String, u64) {
        (folder.to_string(), 100) // Fixed size of 100 bytes per folder
    }

    fn test_response_function(
        folders: Vec<String>,
        entries: Vec<String>,
        continue_token: Option<String>,
    ) -> TestListResponse {
        TestListResponse {
            folders,
            entries,
            continue_token,
        }
    }

    #[test]
    fn test_direct_child_name_decodes_backend_addresses() {
        let child =
            ManifestService::<()>::direct_child_name_from_address(None, "s3://bucket/workspace.wkspc/my%20docs/");

        assert_eq!(child.as_deref(), Some("my docs"));
    }

    #[test]
    fn test_direct_child_name_uses_parent_object_path() {
        let child = ManifestService::<()>::direct_child_name_from_address(
            Some("pkg"),
            "s3://bucket/workspace.wkspc/pkg/my%20docs/file.usd",
        );

        assert_eq!(child.as_deref(), Some("my docs"));
    }

    #[tokio::test]
    async fn test_list_projection_shadow_names_match_encoded_backend_names() {
        let service = test_utils::create_test_service(ByteSize::kib(10)).await;
        let key = WorkspaceKey {
            workspace_path: "s3://bucket/workspace.wkspc".to_string(),
            manifest_path: "s3://bucket/workspace.wkspc/.manifest".to_string(),
            object_path: None,
        };
        let manifest = Manifest::from_iter([(
            "my docs/file.usd".to_string(),
            ManifestEntryValue::ResourceIdentity("s3://bucket/object".to_string()),
        )]);

        let (_, shadow_names) = service.list_projection(&key, manifest);
        let backend_name =
            ManifestService::<()>::direct_child_name_from_address(None, "s3://bucket/workspace.wkspc/my%20docs/");

        assert!(backend_name
            .as_ref()
            .is_some_and(|backend_name| shadow_names.contains(backend_name)));
    }

    #[tokio::test]
    async fn test_list_projection_shadow_names_normalize_unicode() {
        let service = test_utils::create_test_service(ByteSize::kib(10)).await;
        let key = WorkspaceKey {
            workspace_path: "s3://bucket/workspace.wkspc".to_string(),
            manifest_path: "s3://bucket/workspace.wkspc/.manifest".to_string(),
            object_path: None,
        };
        // Manifest keys reach `list_projection` already NFC-normalized (`ManifestFile::into_manifest`
        // does this for every entry), so the shadow set is NFC by construction; the NFC of an
        // NFD-authored name is what `into_manifest` would store.
        let manifest = Manifest::from_iter([(
            "cafe\u{301}/file.usd".nfc().collect::<String>(),
            ManifestEntryValue::ResourceIdentity("s3://bucket/object".to_string()),
        )]);

        let (_, shadow_names) = service.list_projection(&key, manifest);
        let backend_name =
            ManifestService::<()>::direct_child_name_from_address(None, "s3://bucket/workspace.wkspc/caf%C3%A9/");

        assert!(backend_name
            .as_ref()
            .is_some_and(|backend_name| shadow_names.contains(backend_name)));
    }

    #[tokio::test]
    async fn test_list_projection_percent_encodes_child_addresses() {
        // Manifest keys are stored decoded; the merged listing must emit percent-encoded,
        // round-trippable addresses consistent with the backend-sourced siblings (#4).
        let service = test_utils::create_test_service(ByteSize::kib(10)).await;
        let key = WorkspaceKey {
            workspace_path: "s3://bucket/workspace.wkspc".to_string(),
            manifest_path: "s3://bucket/workspace.wkspc/.manifest".to_string(),
            object_path: None,
        };
        let manifest = Manifest::from_iter([
            (
                "a#b.usd".to_string(),
                ManifestEntryValue::ResourceIdentity("s3://bucket/object".to_string()),
            ),
            (
                "my docs/file.usd".to_string(),
                ManifestEntryValue::ResourceIdentity("s3://bucket/object2".to_string()),
            ),
        ]);

        let (list_data, _) = service.list_projection(&key, manifest);
        assert!(list_data
            .file_urls
            .iter()
            .any(|item| item.resource_address == "s3://bucket/workspace.wkspc/a%23b.usd"));
        assert!(list_data
            .subfolder_addresses
            .iter()
            .any(|address| address == "s3://bucket/workspace.wkspc/my%20docs/"));
    }

    #[tokio::test]
    async fn test_stream_list_response_single_page() {
        // With a large page size, all items should fit in a single response
        let service = test_utils::create_test_service(ByteSize::kib(10)).await;
        let list_data = make_list_data(2, 3, "file_");

        let stream = service.stream_list_response(
            list_data,
            StreamingOptions::Streamed,
            test_item_function,
            test_folder_function,
            test_response_function,
        );

        let responses: Vec<_> = stream.collect().await;

        assert_eq!(responses.len(), 1);
        let response = responses[0].as_ref().unwrap();
        assert_eq!(response.folders.len(), 2);
        assert_eq!(response.entries.len(), 3);
        assert!(response.continue_token.is_none());
    }

    #[tokio::test]
    async fn unit_stream_list_response_multiple_pages() {
        // With a small page size (250 bytes), we should get multiple responses
        // Each item is 100 bytes, so we can fit 2 items per page (200 bytes)
        // before the 3rd item would exceed 250 bytes
        let service = test_utils::create_test_service(ByteSize::b(250)).await;
        let list_data = make_list_data(0, 5, "file_");

        let stream = service.stream_list_response(
            list_data,
            StreamingOptions::Streamed,
            test_item_function,
            test_folder_function,
            test_response_function,
        );

        let responses: Vec<_> = stream.collect().await;

        // Should have multiple responses
        assert!(
            responses.len() > 1,
            "Expected multiple responses, got {}",
            responses.len()
        );

        // All items should be present across all responses
        let total_entries: usize = responses.iter().map(|r| r.as_ref().unwrap().entries.len()).sum();
        assert_eq!(total_entries, 5);

        // No continue tokens in streaming mode
        for response in &responses {
            assert!(response.as_ref().unwrap().continue_token.is_none());
        }
    }

    #[tokio::test]
    async fn test_stream_list_response_empty() {
        let service = test_utils::create_test_service(ByteSize::kib(10)).await;
        let list_data = make_list_data(0, 0, "");

        let stream = service.stream_list_response(
            list_data,
            StreamingOptions::Streamed,
            test_item_function,
            test_folder_function,
            test_response_function,
        );

        let responses: Vec<_> = stream.collect().await;

        // Should still get one response with empty data
        assert_eq!(responses.len(), 1);
        let response = responses[0].as_ref().unwrap();
        assert!(response.folders.is_empty());
        assert!(response.entries.is_empty());
    }

    #[tokio::test]
    async fn test_stream_list_response_folders_and_entries_split() {
        // Test that folders and entries can be split across pages
        // Page size of 250 bytes, 2 folders + 3 entries = 5 items at 100 bytes each = 500 bytes
        let service = test_utils::create_test_service(ByteSize::b(250)).await;
        let list_data = make_list_data(2, 3, "file_");

        let stream = service.stream_list_response(
            list_data,
            StreamingOptions::Streamed,
            test_item_function,
            test_folder_function,
            test_response_function,
        );

        let responses: Vec<_> = stream.collect().await;

        // Should have multiple responses
        assert!(responses.len() > 1);

        // Verify all items are present
        let total_folders: usize = responses.iter().map(|r| r.as_ref().unwrap().folders.len()).sum();
        let total_entries: usize = responses.iter().map(|r| r.as_ref().unwrap().entries.len()).sum();
        assert_eq!(total_folders, 2);
        assert_eq!(total_entries, 3);
    }

    #[tokio::test]
    async fn test_paginated_first_page() {
        let service = test_utils::create_test_service(ByteSize::kib(10)).await;
        let list_data = make_list_data(2, 5, "file_");

        let stream = service.stream_list_response(
            list_data,
            StreamingOptions::Paginated(Pagination {
                continuation_handle: None,
                max_page_size: Some(3),
            }),
            test_item_function,
            test_folder_function,
            test_response_function,
        );

        let responses: Vec<_> = stream.collect().await;

        assert_eq!(responses.len(), 1);
        let response = responses[0].as_ref().unwrap();

        // Should get first 3 items (folders come first)
        assert_eq!(response.folders.len(), 2);
        assert_eq!(response.entries.len(), 1);

        // Should have a continue token since there are more items
        assert_eq!(response.continue_token, Some("3".to_string()));
    }

    #[tokio::test]
    async fn test_paginated_second_page() {
        let service = test_utils::create_test_service(ByteSize::kib(10)).await;
        let list_data = make_list_data(2, 5, "file_");

        let stream = service.stream_list_response(
            list_data,
            StreamingOptions::Paginated(Pagination {
                continuation_handle: Some("3".to_string()),
                max_page_size: Some(3),
            }),
            test_item_function,
            test_folder_function,
            test_response_function,
        );

        let responses: Vec<_> = stream.collect().await;

        assert_eq!(responses.len(), 1);
        let response = responses[0].as_ref().unwrap();

        // Should get next 3 items (all entries since folders were in first page)
        assert_eq!(response.folders.len(), 0);
        assert_eq!(response.entries.len(), 3);

        // Should have a continue token since there's 1 more item
        assert_eq!(response.continue_token, Some("6".to_string()));
    }

    #[tokio::test]
    async fn test_paginated_last_page() {
        let service = test_utils::create_test_service(ByteSize::kib(10)).await;
        let list_data = make_list_data(2, 5, "file_");

        let stream = service.stream_list_response(
            list_data,
            StreamingOptions::Paginated(Pagination {
                continuation_handle: Some("6".to_string()),
                max_page_size: Some(3),
            }),
            test_item_function,
            test_folder_function,
            test_response_function,
        );

        let responses: Vec<_> = stream.collect().await;

        assert_eq!(responses.len(), 1);
        let response = responses[0].as_ref().unwrap();

        // Should get the last item
        assert_eq!(response.folders.len(), 0);
        assert_eq!(response.entries.len(), 1);

        // No continue token since this is the last page
        assert!(response.continue_token.is_none());
    }

    #[tokio::test]
    async fn test_paginated_all_items_in_one_page() {
        let service = test_utils::create_test_service(ByteSize::kib(10)).await;
        let list_data = make_list_data(2, 3, "file_");

        let stream = service.stream_list_response(
            list_data,
            StreamingOptions::Paginated(Pagination {
                continuation_handle: None,
                max_page_size: Some(100), // Large enough for all items
            }),
            test_item_function,
            test_folder_function,
            test_response_function,
        );

        let responses: Vec<_> = stream.collect().await;

        assert_eq!(responses.len(), 1);
        let response = responses[0].as_ref().unwrap();

        assert_eq!(response.folders.len(), 2);
        assert_eq!(response.entries.len(), 3);
        assert!(response.continue_token.is_none());
    }

    #[tokio::test]
    async fn test_paginated_max_page_size_capped_at_2000() {
        let service = test_utils::create_test_service(ByteSize::kib(10)).await;

        // Create more than 2000 items
        let list_data = make_list_data(0, 2500, "file_");

        let stream = service.stream_list_response(
            list_data,
            StreamingOptions::Paginated(Pagination {
                continuation_handle: None,
                max_page_size: Some(5000), // Request more than 2000
            }),
            test_item_function,
            test_folder_function,
            test_response_function,
        );

        let responses: Vec<_> = stream.collect().await;

        assert_eq!(responses.len(), 1);
        let response = responses[0].as_ref().unwrap();

        // Should be capped at 2000 items
        assert_eq!(response.entries.len(), 2000);
        assert_eq!(response.continue_token, Some("2000".to_string()));
    }

    #[tokio::test]
    async fn test_paginated_default_page_size() {
        let service = test_utils::create_test_service(ByteSize::kib(10)).await;

        // Create more than 2000 items
        let list_data = make_list_data(0, 2500, "file_");

        let stream = service.stream_list_response(
            list_data,
            StreamingOptions::Paginated(Pagination {
                continuation_handle: None,
                max_page_size: None, // Use default
            }),
            test_item_function,
            test_folder_function,
            test_response_function,
        );

        let responses: Vec<_> = stream.collect().await;

        assert_eq!(responses.len(), 1);
        let response = responses[0].as_ref().unwrap();

        // Default should be 2000 items
        assert_eq!(response.entries.len(), 2000);
    }
}
