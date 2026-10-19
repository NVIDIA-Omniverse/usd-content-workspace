// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

pub mod v1alpha;
pub mod v1beta;

use futures::StreamExt;
use get_size2::GetSize;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use storage_core_util::cache::CacheControl;
use storage_core_util::trace_limited;
use unicode_normalization::UnicodeNormalization;

use super::{
    error::Error,
    format::{manifest_cache_key, workspace_suffix_after_prefix, ManifestEntryValue, WorkspaceKey},
    AddressClass, ManifestService, ManifestState,
};
use crate::traits::{BoxStream, Extensions, MetadataMap, StreamingOptions};

pub enum WorkspaceMutationAction {
    Delegate,
    ManifestFile { workspace_path: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum WorkspaceEnumerateToken {
    Backend { cursor: Option<String> },
    Manifest { index: usize },
}

impl WorkspaceEnumerateToken {
    pub fn from_continuation_handle(continuation_handle: Option<String>) -> crate::traits::error::Result<Self> {
        match continuation_handle {
            None => Ok(Self::Backend { cursor: None }),
            Some(token) => serde_json::from_str(&token)
                .map_err(|e| Error::invalid_argument(format!("Invalid workspace enumerate token: {}", e)).into()),
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
    /// Common implementation for stat operations across API versions.
    ///
    /// This helper handles the repeated pattern of:
    /// 1. Classifying the path as workspace / manifest file / ordinary backend path
    /// 2. If delegation is needed, calling the `on_delegate` callback
    /// 3. If a manifest is found, looking up the object in the manifest
    /// 4. Converting the manifest entry value to `TReturn`
    ///
    /// # Parameters
    /// - `resource_address`: The path to look up in the manifest
    /// - `request`: The request object to pass through
    /// - `read_manifest_fn`: Callback to read the manifest for a given `WorkspaceKey`
    /// - `on_delegate`: Callback invoked when delegation to the inner service is needed
    ///
    /// # Returns
    /// - `Ok(TReturn)` with the converted result (either from delegation or from parsing the manifest)
    /// - `Err(..)` if there was an error reading the manifest, the object path was not found,
    ///   or parsing the resource identity failed
    ///
    /// # Type Parameters
    /// - `TReturn` must implement `TryFrom<&ManifestEntryValue>` for automatic conversion
    pub async fn stat_impl<T, F, Fut, FOnDelegate, FutOnDelegate, TReturn>(
        &self,
        resource_address: &str,
        request: crate::traits::Request<T>,
        read_manifest_fn: F,
        on_delegate: FOnDelegate,
    ) -> crate::traits::error::Result<TReturn>
    where
        T: Clone,
        F: FnOnce(WorkspaceKey, MetadataMap, Extensions) -> Fut,
        Fut: std::future::Future<Output = crate::traits::error::Result<ManifestState>> + Send,
        FOnDelegate: FnOnce(crate::traits::Request<T>) -> FutOnDelegate,
        FutOnDelegate: std::future::Future<Output = crate::traits::error::Result<TReturn>> + Send,
        TReturn: for<'a> TryFrom<&'a ManifestEntryValue, Error = Error>,
    {
        let key = match self.classify_address(resource_address)? {
            AddressClass::NotWorkspace => return on_delegate(request).await,
            AddressClass::ManifestFile { .. } => {
                // The reserved manifest object is forwarded to the backend, but the manifest layer is
                // its authoritative cache — mark it NoCache so the stat/object caches below don't keep
                // a second, stale-able copy (§11.1).
                let mut request = request;
                request.extensions_mut().insert(CacheControl::NoCache);
                return on_delegate(request).await;
            },
            AddressClass::WorkspacePath { key } => key,
        };

        let request_metadata = request.metadata().clone();
        let request_extensions = request.extensions().clone();
        let object_path = key.object_path.clone();
        let state = self
            .get_manifest_state(&key, |key| read_manifest_fn(key, request_metadata, request_extensions))
            .await?;

        match state {
            ManifestState::Loaded(manifest) => {
                let Some(object_path) = object_path else {
                    return on_delegate(request).await;
                };

                let Some(entry_value) = manifest.get(&object_path) else {
                    return on_delegate(request).await;
                };

                trace_limited!("manifest resource_identity: {}", entry_value.resource_identity());

                Ok(TReturn::try_from(entry_value)?)
            },
            ManifestState::Absent => on_delegate(request).await,
            ManifestState::Invalid => Err(ManifestService::<S>::invalid_manifest_error().into()),
        }
    }

    pub async fn mutation_action_for_path<F, Fut>(
        &self,
        resource_address: &str,
        metadata: MetadataMap,
        extensions: Extensions,
        read_manifest_fn: F,
    ) -> crate::traits::error::Result<WorkspaceMutationAction>
    where
        F: Fn(WorkspaceKey, MetadataMap, Extensions) -> Fut,
        Fut: std::future::Future<Output = crate::traits::error::Result<ManifestState>> + Send,
    {
        let key = match self.classify_address(resource_address)? {
            AddressClass::NotWorkspace => return Ok(WorkspaceMutationAction::Delegate),
            AddressClass::ManifestFile { key } => {
                return Ok(WorkspaceMutationAction::ManifestFile {
                    workspace_path: key.workspace_path,
                })
            },
            AddressClass::WorkspacePath { key } => key,
        };

        let object_path = key.object_path.clone();

        // Preliminary manifest state from the (single-flight coalesced, and for `Loaded`/`Invalid`
        // cached) manifest cache.
        let state = self
            .get_manifest_state(&key, |key| read_manifest_fn(key, metadata.clone(), extensions.clone()))
            .await?;

        // A mutation may only be *permitted* on the strength of an authoritative read. Coalescing
        // means this preflight can join an in-flight fill that predates a concurrent `.manifest`
        // activation or rewrite and observe a state that permits a now-forbidden mutation — `Absent`,
        // or a `Loaded` snapshot that does not yet contain the target entry. Both would `Delegate`, so
        // any would-delegate decision is re-checked with a direct, uncoalesced `.manifest` read and
        // acted on authoritatively. A decision to *reject* (target is an entry, or `Invalid`) is safe
        // to take from the coalesced read: a stale rejection only over-restricts (a retryable false
        // read-only / fail-closed error), it never permits a bad write.
        //
        // This closes the stale-cohort window, not every race: a `.manifest` activated *after* the
        // confirmation but before the backend mutation is an inherent check-then-act gap across two
        // objects and is out of scope here.
        //
        // Cost: mutation preflight is a control-plane RPC (payload bytes never traverse this service).
        // The preliminary read is coalesced/cached; only the confirmation is a direct read. So a
        // would-delegate mutation into a `.wkspc` path costs its (often cached) preliminary read plus
        // one direct `.manifest` read (a 404 when absent, plus L2 read/lease traffic under Redis) —
        // per RPC, not per payload byte, and only for `.wkspc`-classified paths. Non-workspace writes
        // never reach here. Hoist to a per-session preflight if it becomes hot.
        let would_delegate = match &state {
            ManifestState::Absent => true,
            ManifestState::Loaded(manifest) => !object_path
                .as_ref()
                .is_some_and(|object_path| manifest.contains_key(object_path)),
            ManifestState::Invalid => false,
        };
        let state = if would_delegate {
            read_manifest_fn(key.clone(), metadata, extensions).await?
        } else {
            state
        };

        match state {
            ManifestState::Loaded(manifest) => {
                if object_path
                    .as_ref()
                    .is_some_and(|object_path| manifest.contains_key(object_path))
                {
                    Err(Error::method_not_allowed("Manifest entries are read-only").into())
                } else {
                    Ok(WorkspaceMutationAction::Delegate)
                }
            },
            ManifestState::Absent => Ok(WorkspaceMutationAction::Delegate),
            ManifestState::Invalid => Err(ManifestService::<S>::invalid_manifest_error().into()),
        }
    }

    pub fn invalidate_manifest_on_stream_item<T, F>(
        &self,
        workspace_path: String,
        stream: BoxStream<T>,
        is_terminal_success: F,
    ) -> BoxStream<T>
    where
        T: Send + 'static,
        F: Fn(&T) -> bool + Send + Sync + 'static,
    {
        let manifests = self.manifests.clone();
        // Normalize once to the cache key so the invalidate matches the keying in `get_manifest_state`.
        let workspace_path = manifest_cache_key(&workspace_path);
        let is_terminal_success = Arc::new(is_terminal_success);
        Box::pin(stream.then(move |response| {
            let manifests = manifests.clone();
            let workspace_path = workspace_path.clone();
            let is_terminal_success = is_terminal_success.clone();
            async move {
                if response.as_ref().is_ok_and(|response| is_terminal_success(response)) {
                    manifests.invalidate(&workspace_path).await;
                }
                response
            }
        }))
    }

    pub fn workspace_relative_path(canonical_workspace_path: &str, resource_address: &str) -> Option<String> {
        // Fast path: backend listing addresses sit directly under the request's constant workspace
        // prefix and are percent-encoded, so a prefix-strip + decode + NFC avoids a full `Url::parse`
        // per entry on the listing hot path (#10). Fall back to the parser for forms it can't match
        // (a different workspace, or a query/fragment-bearing address).
        if let Some(suffix) = workspace_suffix_after_prefix(canonical_workspace_path, resource_address) {
            return Some(normalize_workspace_path(&suffix));
        }
        let key = WorkspaceKey::parse(resource_address).ok().flatten()?;
        if key.workspace_path != canonical_workspace_path {
            return None;
        }
        key.object_path.as_deref().map(normalize_workspace_path)
    }

    /// Common streaming implementation for enumerate operations.
    ///
    /// This function handles both streaming and paginated responses:
    /// - For `StreamingOptions::Streamed`: Batches items by size until page_size_limit is reached
    /// - For `StreamingOptions::Paginated`: Uses skip/take based on continuation_handle and max_page_size
    ///
    /// # Parameters
    /// - `items`: The items to stream (must implement GetSize)
    /// - `streaming`: The streaming options (Streamed or Paginated)
    /// - `response_function`: Creates the response from items and continue_token
    pub fn stream_enumerate_response<T, V, FResponse>(
        &self,
        items: Vec<V>,
        streaming: StreamingOptions,
        response_function: FResponse,
    ) -> BoxStream<T>
    where
        T: Send + 'static,
        V: GetSize + Send + Sync + Clone + 'static,
        FResponse: Fn(Vec<V>, Option<String>) -> T + Send + Sync + 'static,
    {
        let page_size_limit = self.page_size_limit.as_u64();

        match streaming {
            StreamingOptions::Streamed => Box::pin(async_stream::stream! {
                let mut response_items = Vec::new();
                let mut response_size = 0u64;

                for item in &items {
                    let size = item.get_size() as u64;

                    // Check if adding this item would exceed the page size limit
                    if response_size > 0 && response_size + size >= page_size_limit {
                        trace_limited!(response_size, items = response_items.len(), "page size limit reached");
                        yield Ok(response_function(response_items, None));
                        response_size = 0;
                        response_items = Vec::new();
                    }

                    response_size += size;
                    response_items.push(item.clone());
                }

                // Send final response
                trace_limited!(response_size, items = response_items.len(), "final page response");
                yield Ok(response_function(response_items, None));
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
                    let total_items = items.len();
                    let mut response_items = Vec::new();
                    let mut continue_index = None;

                    for (index, item) in items.iter().enumerate().skip(skip).take(take) {
                        response_items.push(item.clone());
                        continue_index = Some(index + 1);
                    }

                    // If we reached the end, no continue token
                    let continue_token = continue_index
                        .filter(|index| *index < total_items)
                        .map(|index| index.to_string());

                    trace_limited!(?continue_token, items = response_items.len());

                    yield Ok(response_function(response_items, continue_token));
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

    use crate::manifest::test_utils::{self, TestItem};
    use crate::traits::{Pagination, StreamingOptions};

    /// A simple response type for testing enumerate operations
    #[derive(Debug, Clone, PartialEq)]
    struct TestEnumerateResponse {
        items: Vec<TestItem>,
        continue_token: Option<String>,
    }

    fn test_response_function(items: Vec<TestItem>, continue_token: Option<String>) -> TestEnumerateResponse {
        TestEnumerateResponse { items, continue_token }
    }

    #[test]
    fn test_workspace_relative_path_decodes_backend_address() {
        let relative = super::ManifestService::<()>::workspace_relative_path(
            "s3://bucket/workspace.wkspc",
            "s3://bucket/workspace.wkspc/my%20docs/file.usd",
        );

        assert_eq!(relative.as_deref(), Some("my docs/file.usd"));
    }

    #[test]
    fn test_workspace_relative_path_normalizes_unicode() {
        let relative = super::ManifestService::<()>::workspace_relative_path(
            "s3://bucket/workspace.wkspc",
            "s3://bucket/workspace.wkspc/caf%C3%A9/file.usd",
        );

        assert_eq!(relative.as_deref(), Some("café/file.usd"));
        assert_eq!(super::normalize_workspace_path("cafe\u{301}/file.usd"), "café/file.usd");
    }

    #[test]
    fn test_workspace_relative_path_matches_loose_request_to_strict_backend_address() {
        let workspace_key = crate::manifest::format::WorkspaceKey::parse("https://host/work().wkspc")
            .unwrap()
            .unwrap();
        let relative = super::ManifestService::<()>::workspace_relative_path(
            &workspace_key.workspace_path,
            "https://host/work%28%29.wkspc/pkg/file%281%29.usd",
        );

        assert_eq!(relative.as_deref(), Some("pkg/file(1).usd"));
    }

    #[tokio::test]
    async fn test_stream_enumerate_response_single_page() {
        let service = test_utils::create_test_service(ByteSize::kib(10)).await;
        let items = TestItem::create_many(5);

        let stream = service.stream_enumerate_response(items, StreamingOptions::Streamed, test_response_function);

        let responses: Vec<_> = stream.collect().await;

        assert_eq!(responses.len(), 1);
        let response = responses[0].as_ref().unwrap();
        assert_eq!(response.items.len(), 5);
        assert!(response.continue_token.is_none());
    }

    #[tokio::test]
    async fn test_stream_enumerate_response_multiple_pages() {
        // Each item is 100 bytes, page size is 250 bytes
        // So we can fit 2 items per page before exceeding
        let service = test_utils::create_test_service(ByteSize::b(250)).await;
        let items = TestItem::create_many(5);

        let stream = service.stream_enumerate_response(items, StreamingOptions::Streamed, test_response_function);

        let responses: Vec<_> = stream.collect().await;

        // Should have multiple pages
        assert!(responses.len() > 1, "Expected multiple pages, got {}", responses.len());

        // All items should be present
        let total_items: usize = responses.iter().map(|r| r.as_ref().unwrap().items.len()).sum();
        assert_eq!(total_items, 5);
    }

    #[tokio::test]
    async fn test_stream_enumerate_response_empty() {
        let service = test_utils::create_test_service(ByteSize::kib(10)).await;
        let items: Vec<TestItem> = vec![];

        let stream = service.stream_enumerate_response(items, StreamingOptions::Streamed, test_response_function);

        let responses: Vec<_> = stream.collect().await;

        assert_eq!(responses.len(), 1);
        let response = responses[0].as_ref().unwrap();
        assert!(response.items.is_empty());
    }

    #[tokio::test]
    async fn test_paginated_enumerate_first_page() {
        let service = test_utils::create_test_service(ByteSize::kib(10)).await;
        let items = TestItem::create_many(10);

        let stream = service.stream_enumerate_response(
            items,
            StreamingOptions::Paginated(Pagination {
                continuation_handle: None,
                max_page_size: Some(3),
            }),
            test_response_function,
        );

        let responses: Vec<_> = stream.collect().await;

        assert_eq!(responses.len(), 1);
        let response = responses[0].as_ref().unwrap();
        assert_eq!(response.items.len(), 3);
        assert_eq!(response.continue_token, Some("3".to_string()));

        // Verify we got the first 3 items
        assert_eq!(response.items[0].id, 0);
        assert_eq!(response.items[1].id, 1);
        assert_eq!(response.items[2].id, 2);
    }

    #[tokio::test]
    async fn test_paginated_enumerate_middle_page() {
        let service = test_utils::create_test_service(ByteSize::kib(10)).await;
        let items = TestItem::create_many(10);

        let stream = service.stream_enumerate_response(
            items,
            StreamingOptions::Paginated(Pagination {
                continuation_handle: Some("3".to_string()),
                max_page_size: Some(3),
            }),
            test_response_function,
        );

        let responses: Vec<_> = stream.collect().await;

        assert_eq!(responses.len(), 1);
        let response = responses[0].as_ref().unwrap();
        assert_eq!(response.items.len(), 3);
        assert_eq!(response.continue_token, Some("6".to_string()));

        // Verify we got items 3, 4, 5
        assert_eq!(response.items[0].id, 3);
        assert_eq!(response.items[1].id, 4);
        assert_eq!(response.items[2].id, 5);
    }

    #[tokio::test]
    async fn test_paginated_enumerate_last_page() {
        let service = test_utils::create_test_service(ByteSize::kib(10)).await;
        let items = TestItem::create_many(10);

        let stream = service.stream_enumerate_response(
            items,
            StreamingOptions::Paginated(Pagination {
                continuation_handle: Some("9".to_string()),
                max_page_size: Some(5),
            }),
            test_response_function,
        );

        let responses: Vec<_> = stream.collect().await;

        assert_eq!(responses.len(), 1);
        let response = responses[0].as_ref().unwrap();
        assert_eq!(response.items.len(), 1);
        assert!(response.continue_token.is_none()); // No more items

        // Verify we got the last item
        assert_eq!(response.items[0].id, 9);
    }
}
