// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Names of the cloud buckets, storage accounts and containers the integration tests run against.
//!
//! The suite talks to real S3, GCS (S3-compatible API), MinIO and Azure Blob storage. Which buckets
//! and accounts those are is deployment-specific, so every name is read from the environment rather
//! than compiled in. Every variable below is required: if any is unset or empty, the first call to
//! [`test_names`] panics with a message naming the missing variables. There is deliberately no
//! fallback name, because S3/GCS bucket names and Azure account names are global and any valid
//! placeholder could be owned by a third party, which would then receive the suite's signed and
//! anonymous requests. Set the variables from `tools/vault/local_env.sh` for local runs.
//!
//! | Variable                                     | Meaning                                       |
//! |----------------------------------------------|-----------------------------------------------|
//! | `OMNI_STORAGE_SERVICE_S3_BUCKET`             | private S3 bucket (also the GCS and MinIO one) |
//! | `OMNI_STORAGE_SERVICE_S3_PUBLIC_BUCKET`      | anonymously readable S3 / MinIO bucket         |
//! | `OMNI_STORAGE_SERVICE_S3_VERSIONED_BUCKET`   | S3 / MinIO bucket with versioning enabled      |
//! | `OMNI_STORAGE_SERVICE_S3_EMPTY_BUCKET`       | empty S3 / GCS / MinIO bucket                  |
//! | `OMNI_STORAGE_SERVICE_AZURE_PRIVATE_ACCOUNT` | Azure account holding the private container    |
//! | `OMNI_STORAGE_SERVICE_AZURE_PUBLIC_ACCOUNT`  | Azure account holding the public/empty ones    |
//! | `OMNI_STORAGE_SERVICE_AZURE_VERSIONED_ACCOUNT` | Azure account with blob versioning enabled  |
//! | `OMNI_STORAGE_SERVICE_AZURE_PUBLIC_CONTAINER` | anonymously readable container (public account) |
//! | `OMNI_STORAGE_SERVICE_AZURE_EMPTY_CONTAINER` | empty container (public account)               |
//!
//! The private and versioned Azure accounts use a container named [`AZURE_CONTAINER`]. The same
//! S3 bucket names are used for GCS and for the local MinIO server.

use std::sync::OnceLock;

/// S3 region every S3 test bucket lives in.
pub const S3_REGION: &str = "us-west-2";

/// Container used in the private and versioned Azure accounts.
pub const AZURE_CONTAINER: &str = "unittests";

/// Bucket, account and container names for the integration test backends.
#[derive(Debug, Clone)]
pub struct TestNames {
    pub s3_bucket: String,
    pub s3_public_bucket: String,
    pub s3_versioned_bucket: String,
    pub s3_empty_bucket: String,
    pub azure_private_account: String,
    pub azure_public_account: String,
    pub azure_versioned_account: String,
    pub azure_public_container: String,
    pub azure_empty_container: String,
}

impl TestNames {
    fn load() -> Self {
        let mut missing = Vec::new();
        let mut var = |name: &'static str| match std::env::var(name) {
            Ok(value) if !value.is_empty() => value,
            _ => {
                missing.push(name);
                String::new()
            },
        };
        let names = Self {
            s3_bucket: var("OMNI_STORAGE_SERVICE_S3_BUCKET"),
            s3_public_bucket: var("OMNI_STORAGE_SERVICE_S3_PUBLIC_BUCKET"),
            s3_versioned_bucket: var("OMNI_STORAGE_SERVICE_S3_VERSIONED_BUCKET"),
            s3_empty_bucket: var("OMNI_STORAGE_SERVICE_S3_EMPTY_BUCKET"),
            azure_private_account: var("OMNI_STORAGE_SERVICE_AZURE_PRIVATE_ACCOUNT"),
            azure_public_account: var("OMNI_STORAGE_SERVICE_AZURE_PUBLIC_ACCOUNT"),
            azure_versioned_account: var("OMNI_STORAGE_SERVICE_AZURE_VERSIONED_ACCOUNT"),
            azure_public_container: var("OMNI_STORAGE_SERVICE_AZURE_PUBLIC_CONTAINER"),
            azure_empty_container: var("OMNI_STORAGE_SERVICE_AZURE_EMPTY_CONTAINER"),
        };
        assert!(
            missing.is_empty(),
            "integration test storage names not configured; set {} (e.g. `source tools/vault/local_env.sh`)",
            missing.join(", ")
        );
        names
    }

    /// Virtual-hosted S3 host name of `bucket`, e.g. `bucket.s3.us-west-2.amazonaws.com`.
    pub fn s3_host(bucket: &str) -> String {
        format!("{bucket}.s3.{S3_REGION}.amazonaws.com")
    }

    /// `https://` URL of an S3 bucket, without a trailing slash.
    pub fn s3_url(bucket: &str) -> String {
        format!("https://{}", Self::s3_host(bucket))
    }

    /// GCS (S3-compatible API) address of `bucket`, without scheme or trailing slash.
    pub fn gcs_host_path(bucket: &str) -> String {
        format!("storage.cloud.google.com/{bucket}")
    }

    /// `https://` URL of a GCS bucket, without a trailing slash.
    pub fn gcs_url(bucket: &str) -> String {
        format!("https://{}", Self::gcs_host_path(bucket))
    }

    /// Blob endpoint host of an Azure storage account, e.g. `account.blob.core.windows.net`.
    pub fn azure_host(account: &str) -> String {
        format!("{account}.blob.core.windows.net")
    }

    /// `https://` URL of an Azure storage account, without a trailing slash.
    pub fn azure_url(account: &str) -> String {
        format!("https://{}", Self::azure_host(account))
    }
}

/// The test names, read from the environment once per process.
///
/// # Panics
///
/// Panics if any of the variables listed in the module documentation is unset or empty.
pub fn test_names() -> &'static TestNames {
    static NAMES: OnceLock<TestNames> = OnceLock::new();
    NAMES.get_or_init(TestNames::load)
}
