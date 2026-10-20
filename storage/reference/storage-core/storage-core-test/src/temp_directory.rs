// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use futures::StreamExt;
use rand::distr::{Alphanumeric, SampleString};
use storage_core::traits::fileobject::v1alpha::DeleteRequest;
use storage_core::traits::fileobject::v1alpha::EnumerateRequest;

/// A temporary directory for testing.
///
/// This is used for writing and deleting files in the temporary directory. The directory is created with a random name and deleted after the test.
pub struct TempDirectory {
    /// The file object client to use for writing and deleting files.
    client: Box<dyn super::clients::traits::fileobject::FileObject>,
    /// The URL of the temporary directory.
    url: String,
    /// The path of the temporary directory.
    path: String,
}

impl TempDirectory {
    /// Create a new temporary directory.
    pub fn new(
        client: Box<dyn super::clients::traits::fileobject::FileObject>,
        base_url: &str,
        base_path: &str,
    ) -> Self {
        let path = Alphanumeric.sample_string(&mut rand::rng(), 20);
        Self {
            client,
            url: format!("{base_url}{base_path}/{path}"),
            path: format!("{base_path}/{path}"),
        }
    }

    /// Create a new file in the temporary directory.
    pub fn new_file(&self) -> String {
        format!("{}/{}", self.url, Alphanumeric.sample_string(&mut rand::rng(), 20))
    }

    /// Create a new file in the temporary directory with a relative path.
    pub fn new_file_in_folder(&self, folder: &str) -> String {
        format!("{}{}", folder, Alphanumeric.sample_string(&mut rand::rng(), 20))
    }

    /// Create a new folder in the temporary directory.
    pub fn new_folder(&self) -> String {
        format!("{}/{}/", self.url, Alphanumeric.sample_string(&mut rand::rng(), 20))
    }

    /// Create a new file in the temporary directory with a relative path.
    pub fn new_file_with_rel_path(&self) -> (String, String) {
        let suffix = Alphanumeric.sample_string(&mut rand::rng(), 20);
        (format!("{}/{}", self.url, suffix), format!("{}/{}", self.path, suffix))
    }

    /// Get the path of the temporary directory.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Get the URL of the temporary directory.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Clean up the temporary directory.
    ///
    /// We can't impl the Drop trait because this function is async, which doesn't work with Drop.
    pub async fn cleanup(&mut self) {
        // For versioned buckets, there might be multiple versions we need to delete, so loop until we get them all.
        loop {
            // Gather all files
            let files: Vec<String> = self
                .client
                .enumerate(EnumerateRequest {
                    resource_address: self.url.clone(),
                })
                .await
                .unwrap()
                .flat_map(|x| {
                    futures::stream::iter(
                        x.unwrap_or_default()
                            .items
                            .into_iter()
                            .map(|a| a.resource_address)
                            .collect::<Vec<String>>(),
                    )
                })
                .collect()
                .await;

            if files.is_empty() {
                break;
            }

            // Delete all files in the directory
            for file in files {
                self.client
                    .delete(DeleteRequest {
                        resource_address: file,
                        previous_version: None,
                    })
                    .await
                    .unwrap();
            }
        }
        // In Azure, the directory itself doesn't actually exist, so no need to delete
    }
}
