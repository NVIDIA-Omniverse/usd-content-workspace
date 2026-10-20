// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use base64::prelude::*;
use prost::Message;
use storage_core_protos::google::protobuf::{value, Value};

use crate::error::Error;

// ---------------------------------------------------------------------------
// Metadata key normalization and encoding
// ---------------------------------------------------------------------------

/// Prefix for hex-encoded metadata keys. Keys starting with this prefix are
/// treated as encoded — the remainder is hex-decoded to recover the original
/// key. The prefix uses only characters valid in both Azure metadata keys
/// (C# identifiers) and HTTP headers (tchar), so encoded keys are portable
/// across backends.
pub const ENCODED_KEY_PREFIX: &str = "nv_key_";

/// Normalize a metadata key to lowercase.
///
/// S3 stores metadata keys in lowercase. Azure Blob metadata keys are
/// case-insensitive. This function normalizes keys so that lookups and
/// comparisons work regardless of the casing used in the original request.
pub fn normalize_metadata_key(key: String) -> String {
    key.to_lowercase()
}

/// Encode a metadata key for backend storage.
///
/// Keys that pass `is_valid` (and don't collide with the encoding prefix)
/// are lowercased and passed through unchanged. All other keys are lowercased,
/// hex-encoded, and prefixed with [`ENCODED_KEY_PREFIX`].
///
/// The `is_valid` function is backend-specific: Azure requires C# identifier
/// rules, S3 requires HTTP tchar rules.
pub fn encode_metadata_key(key: &str, is_valid: fn(&str) -> bool) -> String {
    let normalized = key.to_lowercase();
    if !normalized.is_empty() && !normalized.starts_with(ENCODED_KEY_PREFIX) && is_valid(&normalized) {
        normalized
    } else {
        format!("{ENCODED_KEY_PREFIX}{}", hex::encode(&normalized))
    }
}

/// Decode a metadata key back to its original form.
///
/// Keys prefixed with [`ENCODED_KEY_PREFIX`] have the prefix stripped and
/// the remainder hex-decoded. If hex decoding fails (e.g. the key was written
/// by another tool and happens to start with the prefix), the key is returned
/// as-is (lowercased). Keys without the prefix are returned lowercased.
pub fn decode_metadata_key(key: &str) -> String {
    let normalized = key.to_lowercase();
    match normalized.strip_prefix(ENCODED_KEY_PREFIX) {
        Some(hex_part) => hex::decode(hex_part)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .unwrap_or(normalized),
        None => normalized,
    }
}

// ---------------------------------------------------------------------------
// Metadata value encoding
// ---------------------------------------------------------------------------

/// Sentinel prefix prepended to base64-encoded protobuf values before storage.
/// Disambiguates structured values from plain strings so that a user string
/// that happens to be valid base64+protobuf is never misinterpreted.
/// Uses only ASCII-printable characters so it is safe in HTTP headers (S3/Azure
/// metadata values are transported as header values).
const PB_SENTINEL: &str = "$pb$";

/// Encode a protobuf `Value` for storage as a string metadata entry.
///
/// Plain strings are stored as-is for readability in cloud consoles.
/// All other kinds (null, number, bool, struct, list) are stored with a
/// `"$pb$"` sentinel prefix followed by base64-encoded protobuf bytes.
pub fn encode_metadata_value(value: &Value) -> String {
    match value.kind {
        Some(value::Kind::StringValue(ref s)) => s.clone(),
        _ => format!("{PB_SENTINEL}{}", BASE64_STANDARD.encode(value.encode_to_vec())),
    }
}

/// Decode a string metadata entry back into a protobuf `Value`.
///
/// Values prefixed with `"$pb$"` are decoded as base64 protobuf.
/// Everything else is treated as a plain string.
///
/// Returns an error if the sentinel prefix is present but the payload
/// is corrupted (invalid base64 or invalid protobuf).
pub fn decode_metadata_value(raw: String) -> Result<Value, Error> {
    if let Some(encoded) = raw.strip_prefix(PB_SENTINEL) {
        let bytes = BASE64_STANDARD.decode(encoded).map_err(|e| {
            Error::internal(format!(
                "metadata value has sentinel prefix but base64 decode failed: {e}"
            ))
        })?;
        let value = Value::decode(bytes.as_slice()).map_err(|e| {
            Error::internal(format!(
                "metadata value has sentinel prefix but protobuf decode failed: {e}"
            ))
        })?;
        return Ok(value);
    }
    Ok(Value {
        kind: Some(value::Kind::StringValue(raw)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // A permissive validator (like S3 tchar) that allows most printable ASCII
    fn is_valid_permissive(key: &str) -> bool {
        key.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~'))
    }

    // A strict validator (like Azure C# identifier) that allows only alphanum + underscore
    fn is_valid_strict(key: &str) -> bool {
        let first = key.as_bytes()[0];
        (first.is_ascii_alphabetic() || first == b'_') && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    }

    #[test]
    fn valid_key_passthrough_strict() {
        assert_eq!(encode_metadata_key("my_tag", is_valid_strict), "my_tag");
    }

    #[test]
    fn valid_key_passthrough_permissive() {
        assert_eq!(encode_metadata_key("content-type", is_valid_permissive), "content-type");
    }

    #[test]
    fn invalid_key_encoded_strict() {
        let encoded = encode_metadata_key("content-type", is_valid_strict);
        assert!(encoded.starts_with(ENCODED_KEY_PREFIX));
        assert_eq!(decode_metadata_key(&encoded), "content-type");
    }

    #[test]
    fn invalid_key_encoded_permissive() {
        let encoded = encode_metadata_key("my key", is_valid_permissive);
        assert!(encoded.starts_with(ENCODED_KEY_PREFIX));
        assert_eq!(decode_metadata_key(&encoded), "my key");
    }

    #[test]
    fn case_normalization() {
        assert_eq!(encode_metadata_key("My_Tag", is_valid_strict), "my_tag");
        assert_eq!(encode_metadata_key("MY_TAG", is_valid_strict), "my_tag");
    }

    #[test]
    fn prefix_collision_encoded() {
        let encoded = encode_metadata_key("nv_key_foo", is_valid_strict);
        assert!(encoded.starts_with(ENCODED_KEY_PREFIX));
        assert_eq!(decode_metadata_key(&encoded), "nv_key_foo");
    }

    #[test]
    fn empty_key_encoded() {
        let encoded = encode_metadata_key("", is_valid_strict);
        assert!(encoded.starts_with(ENCODED_KEY_PREFIX));
        assert_eq!(decode_metadata_key(&encoded), "");
    }

    #[test]
    fn roundtrip_various_keys() {
        let keys = [
            "content-type",
            "last.modified",
            "1st_author",
            "my_tag",
            "nv_key_custom",
            "x-amz-meta-custom",
            "hello world",
            "_leading_underscore",
            "my:key",
            "my/key",
        ];
        for key in keys {
            let lowered = key.to_lowercase();
            let encoded = encode_metadata_key(key, is_valid_strict);
            let decoded = decode_metadata_key(&encoded);
            assert_eq!(decoded, lowered, "roundtrip failed for key: {key}");
        }
    }

    #[test]
    fn decode_non_prefixed_key() {
        assert_eq!(decode_metadata_key("simple"), "simple");
        assert_eq!(decode_metadata_key("MIXED_Case"), "mixed_case");
    }

    #[test]
    fn decode_invalid_hex_passthrough() {
        assert_eq!(decode_metadata_key("nv_key_zzzz"), "nv_key_zzzz");
        assert_eq!(decode_metadata_key("nv_key_abc"), "nv_key_abc"); // odd length
    }
}
