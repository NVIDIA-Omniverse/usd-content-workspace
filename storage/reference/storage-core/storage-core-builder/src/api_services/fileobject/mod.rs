// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use storage_core_util::url_path::canonicalize_url_path;
use url::Url;

use crate::api_services::error::StorageApiServiceError;

pub mod v1alpha;
pub mod v1beta;

mod utils;

/// Canonicalize the path portion of a resource identity for a REST response header.
pub(crate) fn canonicalize_identity(value: &str) -> Result<String, StorageApiServiceError> {
    let mut url = Url::parse(value)
        .map_err(|e| StorageApiServiceError::invalid_argument(format!("invalid resource identity '{value}': {e}")))?;
    canonicalize_url_path(&mut url).map_err(|e| {
        StorageApiServiceError::invalid_argument(format!("URL path canonicalization failed for '{url}': {e}"))
    })?;
    Ok(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_identity_path_is_strict_and_idempotent() {
        let loose = "https://host/container/!$%&'()*+,:;=@[]^|?etag=%22abc%22";
        let strict = "https://host/container/%21%24%25%26%27%28%29%2A%2B%2C%3A%3B%3D%40%5B%5D%5E%7C?etag=%22abc%22";
        assert_eq!(canonicalize_identity(loose).unwrap(), strict);
        assert_eq!(canonicalize_identity(strict).unwrap(), strict);
    }

    #[test]
    fn canonical_identity_rejects_dot_segment_created_by_encoded_slash() {
        let error = canonicalize_identity("https://host/container/..%2Fkey?etag=%22abc%22").unwrap_err();
        assert!(error.to_string().contains("prohibited dot segment"), "{error}");
    }

    #[test]
    fn canonical_identity_preserves_query_parameters() {
        let value = "https://host/container/blob?download=1&ETAG=%22abc%22&custom=a%2Bb&Version-Id=v1";
        assert_eq!(canonicalize_identity(value).unwrap(), value);
    }
}
