// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

pub mod error;
pub mod fileobject;

mod cache;
pub use cache::*;

use storage_core_builder::traits::error::Result;
use storage_core_util::warn_limited;
use url::Url;

// The API-version partition for cache keys lives in storage-core-util so a cache layer built alone
// (`--features small_object_cache` without `stat_cache`) can reference it without a feature dependency.
pub(crate) use storage_core_util::cache::ApiVersion;

pub(crate) fn canonical_cache_key(value: &str) -> Result<String> {
    let mut url = Url::parse(value).map_err(|e| error::Error::invalid_argument(e.to_string()))?;
    storage_core_util::url_path::canonicalize_url_path(&mut url)
        .map_err(|e| error::Error::invalid_argument(e.to_string()))?;
    Ok(url.into())
}

/// Version-scoped read/fill key (`\u{1f}` unit-separator prefix, absent from URLs). Used only by the
/// stat/small-object caches; the metadata cache keeps the un-partitioned `canonical_cache_key`.
pub(crate) fn versioned_cache_key(value: &str, version: ApiVersion) -> Result<String> {
    Ok(format!("{}\u{1f}{}", version.prefix(), canonical_cache_key(value)?))
}

/// All version-partitioned keys for an address. A mutation via one API version must invalidate the
/// value cached under every version, since the object changed for all of them.
pub(crate) fn all_version_keys(value: &str) -> Result<Vec<String>> {
    let url = canonical_cache_key(value)?;
    Ok(ApiVersion::ALL
        .iter()
        .map(|v| format!("{}\u{1f}{}", v.prefix(), url))
        .collect())
}

pub(crate) fn canonical_cache_value(mut value: CachedStatResponse) -> CachedStatResponse {
    if !value.encoded_identity.is_empty() {
        match Url::parse(&value.encoded_identity) {
            Ok(mut url) => match storage_core_util::url_path::canonicalize_url_path(&mut url) {
                Ok(()) => value.encoded_identity = url.into(),
                Err(e) => warn_limited!(%e, "backend returned an uncanonicalizable resource identity path"),
            },
            Err(e) => warn_limited!(%e, "backend returned an invalid resource identity"),
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_key_coalesces_overencoded_container_names() {
        let overencoded = canonical_cache_key("https://account.blob.core.windows.net/my%2Dcontainer/blob").unwrap();
        let canonical = canonical_cache_key("https://account.blob.core.windows.net/my-container/blob").unwrap();
        assert_eq!(overencoded, canonical);
    }

    #[test]
    fn canonical_key_rejects_dot_segment_created_by_encoded_slash() {
        let error = canonical_cache_key("https://account.blob.core.windows.net/container/..%2Fkey").unwrap_err();
        assert_eq!(error.tonic_status().code(), tonic::Code::InvalidArgument);
        assert!(error.to_string().contains("prohibited dot segment"), "{error}");
    }

    #[test]
    fn v1alpha_and_v1beta_keys_do_not_collide() {
        // A v1beta stat (whose proto cannot carry native_metadata) must not share a cache key with a
        // v1alpha stat of the same address, or it would poison the v1alpha read with an empty map.
        let addr = "https://account.blob.core.windows.net/container/blob";
        let a = versioned_cache_key(addr, ApiVersion::V1Alpha).unwrap();
        let b = versioned_cache_key(addr, ApiVersion::V1Beta).unwrap();
        assert_ne!(a, b);
        let all = all_version_keys(addr).unwrap();
        assert!(
            all.contains(&a) && all.contains(&b),
            "invalidation must cover both partitions"
        );
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn cached_size_grows_with_native_metadata() {
        use storage_core_util::cache::CachedSize;
        let base = CachedStatResponse {
            encoded_identity: "https://account.blob.core.windows.net/container/blob".to_string(),
            data_object_size: None,
            last_modified_timestamp: None,
            native_metadata: Default::default(),
        };
        let mut with_meta = base.clone();
        with_meta.native_metadata = storage_core_util::api_types::native_metadata_from_strings([(
            "source".to_string(),
            "a-fairly-long-pipeline-identifier".to_string(),
        )]);
        assert!(
            with_meta.cached_size() > base.cached_size(),
            "native metadata bytes must count against the cache budget"
        );
    }

    #[test]
    fn invalid_backend_identity_is_preserved() {
        let value = canonical_cache_value(CachedStatResponse {
            encoded_identity: "not a URL".to_string(),
            data_object_size: None,
            last_modified_timestamp: None,
            native_metadata: Default::default(),
        });

        assert_eq!(value.encoded_identity, "not a URL");
    }
}
