// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

pub mod error;
pub mod filefolder;
pub mod fileobject;
pub mod format;
pub mod versioning;

pub mod service;
pub use service::*;

#[cfg(test)]
mod overlay_tests;

#[cfg(test)]
#[allow(dead_code)]
pub(crate) mod test_utils {
    use std::sync::Arc;

    use bytesize::ByteSize;
    use get_size2::GetSize;

    use super::format::ManifestEntryValue;
    use super::service::Config;
    use super::ManifestService;
    use storage_core_util::cache::{LayerCache, LayerCacheArgs};

    /// Creates a ManifestService for testing with the specified page_size_limit.
    /// Uses `()` as the inner service since we're testing the streaming helpers directly.
    pub async fn create_test_service(page_size_limit: ByteSize) -> ManifestService<()> {
        let config = Config {
            page_size_limit,
            ..Default::default()
        };
        let args: LayerCacheArgs = config.clone().into();
        let cache = LayerCache::new(&args, "test_manifest", "test_manifest", None, None)
            .await
            .expect("LayerCache build for test_manifest");
        ManifestService::new((), (), config, Arc::new(cache), page_size_limit)
    }

    /// A test item that implements GetSize with a predictable fixed size.
    /// Used for testing enumerate streaming.
    #[derive(Debug, Clone, PartialEq)]
    pub struct TestItem {
        pub id: usize,
        pub data: String,
    }

    impl GetSize for TestItem {
        fn get_size(&self) -> usize {
            100 // Fixed size for predictable testing
        }
    }

    impl TestItem {
        pub fn create_many(count: usize) -> Vec<Self> {
            (0..count)
                .map(|i| TestItem {
                    id: i,
                    data: format!("item_{}", i),
                })
                .collect()
        }
    }

    /// A test version item that implements GetSize with a predictable fixed size.
    /// Used for testing enumerate_versions streaming.
    #[derive(Debug, Clone, PartialEq)]
    pub struct TestVersionItem {
        pub version_id: usize,
        pub sorting_key: String,
    }

    impl GetSize for TestVersionItem {
        fn get_size(&self) -> usize {
            100 // Fixed size for predictable testing
        }
    }

    impl TestVersionItem {
        pub fn create_many(count: usize) -> Vec<Self> {
            (0..count)
                .map(|i| TestVersionItem {
                    version_id: i,
                    sorting_key: format!("2024-01-{:02}T00:00:00Z", i + 1),
                })
                .collect()
        }
    }

    /// Helper struct representing a list item for testing
    pub struct TestListItem {
        pub resource_address: String,
        pub entry_value: ManifestEntryValue,
    }

    /// Helper to create test list data (folders and files)
    pub fn create_test_list_data(
        num_folders: usize,
        num_files: usize,
        file_prefix: &str,
    ) -> (Vec<String>, Vec<TestListItem>) {
        let subfolder_addresses: Vec<String> = (0..num_folders)
            .map(|i| format!("s3://bucket/workspace.wkspc/folder_{}", i))
            .collect();

        let file_urls: Vec<TestListItem> = (0..num_files)
            .map(|i| TestListItem {
                resource_address: format!("s3://bucket/workspace.wkspc/{}{}.txt", file_prefix, i),
                entry_value: ManifestEntryValue::ResourceIdentity(format!("identity_{}", i)),
            })
            .collect();

        (subfolder_addresses, file_urls)
    }
}
