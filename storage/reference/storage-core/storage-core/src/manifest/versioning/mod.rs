// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

pub mod v1alpha;
pub mod v1beta;

use get_size2::GetSize;
use storage_core_util::trace_limited;

use super::ManifestService;
use crate::traits::{BoxStream, StreamingOptions};

impl<S, R> ManifestService<S, R> {
    /// Common streaming implementation for enumerate_versions operations.
    ///
    /// This function handles both streaming and paginated responses:
    /// - For `StreamingOptions::Streamed`: Sends items in batches by size until page_size_limit is reached
    /// - For `StreamingOptions::Paginated`: Uses skip/take based on continuation_handle and max_page_size
    ///
    /// # Parameters
    /// - `items`: The version items to stream
    /// - `versions_order`: The order of versions in the response
    /// - `version_id_token`: Optional version ID token for the response
    /// - `streaming`: The streaming options (Streamed or Paginated)
    /// - `response_function`: Creates the response from items, versions_order, continue_token, and version_id_token
    pub fn stream_enumerate_versions_response<T, V, FResponse>(
        &self,
        items: Vec<V>,
        versions_order: i32,
        version_id_token: Option<String>,
        streaming: StreamingOptions,
        response_function: FResponse,
    ) -> BoxStream<T>
    where
        T: Send + 'static,
        V: GetSize + Send + Sync + Clone + 'static,
        FResponse: Fn(Vec<V>, i32, Option<String>, Option<String>) -> T + Send + Sync + 'static,
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
                        yield Ok(response_function(
                            response_items,
                            versions_order,
                            None,
                            version_id_token.clone(),
                        ));
                        response_size = 0;
                        response_items = Vec::new();
                    }

                    response_size += size;
                    response_items.push(item.clone());
                }

                // Send final response
                trace_limited!(response_size, items = response_items.len(), "final page response");
                yield Ok(response_function(
                    response_items,
                    versions_order,
                    None,
                    version_id_token,
                ));
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

                    yield Ok(response_function(
                        response_items,
                        versions_order,
                        continue_token,
                        version_id_token,
                    ));
                })
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use bytesize::ByteSize;
    use futures::StreamExt;

    use crate::manifest::test_utils::{self, TestVersionItem};
    use crate::traits::{Pagination, StreamingOptions};

    /// A simple response type for testing version enumeration
    #[derive(Debug, Clone, PartialEq)]
    struct TestVersionResponse {
        items: Vec<TestVersionItem>,
        versions_order: i32,
        continue_token: Option<String>,
        version_id_token: Option<String>,
    }

    fn test_response_function(
        items: Vec<TestVersionItem>,
        versions_order: i32,
        continue_token: Option<String>,
        version_id_token: Option<String>,
    ) -> TestVersionResponse {
        TestVersionResponse {
            items,
            versions_order,
            continue_token,
            version_id_token,
        }
    }

    #[tokio::test]
    async fn test_stream_versions_single_page() {
        let service = test_utils::create_test_service(ByteSize::kib(10)).await;
        let items = TestVersionItem::create_many(5);

        let stream = service.stream_enumerate_versions_response(
            items,
            1, // NewestFirst
            Some("version_token".to_string()),
            StreamingOptions::Streamed,
            test_response_function,
        );

        let responses: Vec<_> = stream.collect().await;

        assert_eq!(responses.len(), 1);
        let response = responses[0].as_ref().unwrap();
        assert_eq!(response.items.len(), 5);
        assert_eq!(response.versions_order, 1);
        assert!(response.continue_token.is_none());
        assert_eq!(response.version_id_token, Some("version_token".to_string()));
    }

    #[tokio::test]
    async fn test_stream_versions_multiple_pages() {
        // Each item is 100 bytes, page size is 250 bytes
        let service = test_utils::create_test_service(ByteSize::b(250)).await;
        let items = TestVersionItem::create_many(5);

        let stream = service.stream_enumerate_versions_response(
            items,
            1,
            None,
            StreamingOptions::Streamed,
            test_response_function,
        );

        let responses: Vec<_> = stream.collect().await;

        // Should have multiple pages
        assert!(responses.len() > 1, "Expected multiple pages, got {}", responses.len());

        // All items should be present
        let total_items: usize = responses.iter().map(|r| r.as_ref().unwrap().items.len()).sum();
        assert_eq!(total_items, 5);

        // All responses should have the same versions_order
        for response in &responses {
            assert_eq!(response.as_ref().unwrap().versions_order, 1);
        }
    }

    #[tokio::test]
    async fn test_stream_versions_empty() {
        let service = test_utils::create_test_service(ByteSize::kib(10)).await;
        let items: Vec<TestVersionItem> = vec![];

        let stream = service.stream_enumerate_versions_response(
            items,
            0, // OldestFirst
            None,
            StreamingOptions::Streamed,
            test_response_function,
        );

        let responses: Vec<_> = stream.collect().await;

        assert_eq!(responses.len(), 1);
        let response = responses[0].as_ref().unwrap();
        assert!(response.items.is_empty());
        assert_eq!(response.versions_order, 0);
    }

    #[tokio::test]
    async fn test_paginated_versions_first_page() {
        let service = test_utils::create_test_service(ByteSize::kib(10)).await;
        let items = TestVersionItem::create_many(10);

        let stream = service.stream_enumerate_versions_response(
            items,
            1,
            Some("token".to_string()),
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
        assert_eq!(response.version_id_token, Some("token".to_string()));

        // Verify we got the first 3 items
        assert_eq!(response.items[0].version_id, 0);
        assert_eq!(response.items[1].version_id, 1);
        assert_eq!(response.items[2].version_id, 2);
    }

    #[tokio::test]
    async fn test_paginated_versions_continuation() {
        let service = test_utils::create_test_service(ByteSize::kib(10)).await;
        let items = TestVersionItem::create_many(10);

        let stream = service.stream_enumerate_versions_response(
            items,
            1,
            None,
            StreamingOptions::Paginated(Pagination {
                continuation_handle: Some("5".to_string()),
                max_page_size: Some(3),
            }),
            test_response_function,
        );

        let responses: Vec<_> = stream.collect().await;

        assert_eq!(responses.len(), 1);
        let response = responses[0].as_ref().unwrap();
        assert_eq!(response.items.len(), 3);
        assert_eq!(response.continue_token, Some("8".to_string()));

        // Verify we got items 5, 6, 7
        assert_eq!(response.items[0].version_id, 5);
        assert_eq!(response.items[1].version_id, 6);
        assert_eq!(response.items[2].version_id, 7);
    }

    #[tokio::test]
    async fn test_paginated_versions_last_page() {
        let service = test_utils::create_test_service(ByteSize::kib(10)).await;
        let items = TestVersionItem::create_many(10);

        let stream = service.stream_enumerate_versions_response(
            items,
            1,
            None,
            StreamingOptions::Paginated(Pagination {
                continuation_handle: Some("8".to_string()),
                max_page_size: Some(5),
            }),
            test_response_function,
        );

        let responses: Vec<_> = stream.collect().await;

        assert_eq!(responses.len(), 1);
        let response = responses[0].as_ref().unwrap();
        assert_eq!(response.items.len(), 2);
        assert!(response.continue_token.is_none()); // No more items

        // Verify we got the last 2 items
        assert_eq!(response.items[0].version_id, 8);
        assert_eq!(response.items[1].version_id, 9);
    }
}
