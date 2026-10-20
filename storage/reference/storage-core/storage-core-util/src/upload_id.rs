// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

const SEPARATOR: &str = "::__SPLIT__::";
const FAIL_IF_EXISTS: &str = "fail_if_exists=true";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodedUploadId<'a> {
    pub upload_id: &'a str,
    pub resource_identity: Option<&'a str>,
    pub fail_if_exists: bool,
}

/// Encodes an upload ID and optional resource identity into a single string.
/// The legacy format is: <upload_id>::__SPLIT__::<resource_identity>.
/// When fail_if_exists is true, the format is:
/// <upload_id>::__SPLIT__::<resource_identity>::__SPLIT__::fail_if_exists=true
///
/// In the FileObject API, the previous version is provided to the Write API and needs
/// to be encoded into the upload ID to later be passed to the CompleteMultipartUpload
/// API.
pub fn encode(upload_id: &str, resource_identity: Option<&str>, fail_if_exists: bool) -> String {
    if fail_if_exists {
        let identity = resource_identity.unwrap_or_default();
        format!("{upload_id}{SEPARATOR}{identity}{SEPARATOR}{FAIL_IF_EXISTS}")
    } else if let Some(identity) = resource_identity {
        format!("{upload_id}{SEPARATOR}{identity}")
    } else {
        upload_id.to_string()
    }
}

pub fn decode_with_options(encoded: &str) -> DecodedUploadId<'_> {
    // Resource identities are URLs whose path segments (object keys) may legally
    // contain the literal substring `::__SPLIT__::`. A naive splitn(3) would treat
    // such an embedded occurrence as the marker boundary and truncate the identity.
    // Strip the fail_if_exists suffix first (when present as an actual trailing
    // marker), then split on the FIRST separator to recover the boundary between
    // upload_id and resource_identity.
    let (body, fail_if_exists) = match encoded
        .strip_suffix(FAIL_IF_EXISTS)
        .and_then(|s| s.strip_suffix(SEPARATOR))
    {
        Some(prefix) => (prefix, true),
        None => (encoded, false),
    };

    let (upload_id, identity) = match body.split_once(SEPARATOR) {
        Some((upload_id, identity)) => (upload_id, Some(identity)),
        None => (body, None),
    };

    DecodedUploadId {
        upload_id,
        resource_identity: identity.filter(|s| !s.is_empty()),
        fail_if_exists,
    }
}

/// Decodes a string into an upload ID and optional resource identity.
/// Returns a tuple of (&str, Option<&str>).
pub fn decode(encoded: &str) -> (&str, Option<&str>) {
    let decoded = decode_with_options(encoded);
    (decoded.upload_id, decoded.resource_identity)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_with_identity() {
        let upload_id = "test-upload-123";
        let identity = "https://example.com/file?etag=abc123";

        let encoded = encode(upload_id, Some(identity), false);
        assert_eq!(
            encoded,
            format!("test-upload-123{SEPARATOR}https://example.com/file?etag=abc123")
        );
    }

    #[test]
    fn test_encode_without_identity() {
        let upload_id = "test-upload-123";
        let encoded = encode(upload_id, None, false);
        assert_eq!(encoded, "test-upload-123");
    }

    #[test]
    fn test_decode_with_identity() {
        let encoded = format!("test-upload-123{SEPARATOR}https://example.com/file?etag=abc123");
        let (upload_id, identity) = decode(&encoded);
        assert_eq!(upload_id, "test-upload-123");
        assert_eq!(identity.unwrap(), "https://example.com/file?etag=abc123");
    }

    #[test]
    fn test_decode_without_identity() {
        let encoded = "test-upload-123";
        let (upload_id, identity) = decode(encoded);
        assert_eq!(upload_id, "test-upload-123");
        assert!(identity.is_none());
    }

    #[test]
    fn test_decode_empty_string() {
        let result = decode("");
        assert!(result.0.is_empty());
    }

    #[test]
    fn test_encode_with_fail_if_exists() {
        let upload_id = "test-upload-123";
        let identity = "https://example.com/file?etag=abc123";

        let encoded = encode(upload_id, Some(identity), true);
        assert_eq!(
            encoded,
            format!("test-upload-123{SEPARATOR}https://example.com/file?etag=abc123{SEPARATOR}{FAIL_IF_EXISTS}")
        );

        let decoded = decode_with_options(&encoded);
        assert_eq!(decoded.upload_id, upload_id);
        assert_eq!(decoded.resource_identity, Some(identity));
        assert!(decoded.fail_if_exists);
    }

    #[test]
    fn test_encode_fail_if_exists_without_identity() {
        let upload_id = "test-upload-123";
        let encoded = encode(upload_id, None, true);
        assert_eq!(
            encoded,
            format!("test-upload-123{SEPARATOR}{SEPARATOR}{FAIL_IF_EXISTS}")
        );

        let decoded = decode_with_options(&encoded);
        assert_eq!(decoded.upload_id, upload_id);
        assert_eq!(decoded.resource_identity, None);
        assert!(decoded.fail_if_exists);
    }

    #[test]
    fn test_upload_id_with_colon() {
        let upload_id = "test:upload:123";
        let identity = "https://example.com/file?etag=abc123";

        let encoded = encode(upload_id, Some(identity), false);
        let (decoded_id, decoded_identity) = decode(&encoded);

        assert_eq!(decoded_id, "test:upload:123");
        assert_eq!(decoded_identity.unwrap(), "https://example.com/file?etag=abc123");
    }

    #[test]
    fn test_decode_identity_with_embedded_separator() {
        let upload_id = "test-upload-123";
        let identity = "https://bucket.s3.us-west-2.amazonaws.com/foo::__SPLIT__::bar?etag=abc123";

        let encoded = encode(upload_id, Some(identity), false);
        let decoded = decode_with_options(&encoded);
        assert_eq!(decoded.upload_id, upload_id);
        assert_eq!(decoded.resource_identity, Some(identity));
        assert!(!decoded.fail_if_exists);
    }

    #[test]
    fn test_decode_identity_with_embedded_separator_and_fail_if_exists() {
        let upload_id = "test-upload-123";
        let identity = "https://bucket.s3.us-west-2.amazonaws.com/foo::__SPLIT__::bar?etag=abc123";

        let encoded = encode(upload_id, Some(identity), true);
        let decoded = decode_with_options(&encoded);
        assert_eq!(decoded.upload_id, upload_id);
        assert_eq!(decoded.resource_identity, Some(identity));
        assert!(decoded.fail_if_exists);
    }
}
