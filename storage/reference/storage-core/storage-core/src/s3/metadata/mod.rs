// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

pub mod v1alpha;

use std::collections::HashMap;

use super::{
    error,
    error::Error,
    resource::{Address, Identity},
    service::Storage,
};
use crate::traits::error::Result;
use aws_sdk_s3::types::MetadataDirective;
use snafu::prelude::*;
use storage_core_builder::traits::metadata::v1alpha::Value;
use storage_core_util::trace_limited;
use storage_core_util::{
    additional_headers::{AdditionalHeaders, RequestType},
    metadata::{
        decode_metadata_key, decode_metadata_value, encode_metadata_key, encode_metadata_value, normalize_metadata_key,
    },
};
use tracing::{info_span, instrument, Instrument};

/// Resolved metadata URI components extracted by [`resolve_metadata_uri`].
struct ResolvedUri {
    /// The clean resource address (query params stripped for identities).
    address: Address,
    /// Etag extracted from an identity URI. Used as an `If-Match` precondition on
    /// HeadObject to ensure the identity still matches the current object.
    /// `None` for plain and versioned addresses.
    expected_etag: Option<String>,
    /// Version ID extracted from an identity or versioned address. For reads, selects
    /// the specific object version. For mutations, verified against the head version
    /// to ensure the caller's expected version is still current.
    /// `None` for plain addresses.
    version_id: Option<String>,
}

/// Parse a metadata `uri` into its components. The URI may be one of three forms:
///
/// 1. **Plain address** (e.g. `s3://bucket/key`): no preconditions are applied.
/// 2. **Versioned address** (`?version-id=…`, no `etag`): the version ID is extracted.
///    Reads target that specific version; mutations verify it is still the head.
/// 3. **Identity** (`?etag=…` and optionally `?version-id=…`): both the etag and version
///    ID are extracted. The etag is used as an `If-Match` server-side precondition, and
///    the version ID is verified client-side against the head.
///
/// S3's `Identity::parse` requires an etag, so a successful parse guarantees `etag`
/// is `Some`. Plain/versioned URIs will fail `Identity::parse` and fall through to
/// `Address::parse`.
fn resolve_metadata_uri(uri: &str) -> Result<ResolvedUri> {
    // Try identity parse first — requires etag, so plain/versioned addresses will fail here
    if let Ok(identity) = Identity::parse(uri) {
        let etag = identity.etag;
        let version_id = identity.version_id;
        // Build a clean address without identity query params
        let mut clean_url = identity.resource_address.url.clone();
        clean_url.set_query(None);
        let clean_address = Address::parse(clean_url.as_str())?;
        return Ok(ResolvedUri {
            address: clean_address,
            expected_etag: etag,
            version_id,
        });
    }

    // Fall back to address parse
    let address = Address::parse(uri)?;
    let version_id = address.version_id().map(|v| v.to_string());
    Ok(ResolvedUri {
        address,
        expected_etag: None,
        version_id,
    })
}

/// Returns `true` if `key` is a valid S3 metadata key (HTTP token character per RFC 7230).
///
/// Assumes `key` is already lowercased.
pub(crate) fn is_valid_s3_metadata_key(key: &str) -> bool {
    // `Iterator::all` is vacuously true for an empty iterator, so guard the empty key explicitly
    // (a protobuf `map<string, Value>` permits empty keys), matching is_valid_azure_metadata_key.
    !key.is_empty() && key.bytes().all(is_tchar)
}

/// HTTP token character (tchar) per RFC 7230 section 3.2.6.
fn is_tchar(b: u8) -> bool {
    b.is_ascii_alphanumeric()
        || matches!(
            b,
            b'!' | b'#' | b'$' | b'%' | b'&' | b'\'' | b'*' | b'+' | b'-' | b'.' | b'^' | b'_' | b'`' | b'|' | b'~'
        )
}

impl Storage {
    /// Fetch user metadata for a resource.
    ///
    /// Accepts a plain address, versioned address, or identity URI (see `resolve_metadata_uri`).
    ///
    /// - **Plain address**: HeadObject with no preconditions; returns current metadata.
    /// - **Versioned address**: HeadObject targets the specific version via `version_id`.
    /// - **Identity**: HeadObject uses `If-Match` with the identity's etag (server rejects
    ///   if stale) and targets the specific version via `version_id` if present.
    #[instrument(name = "s3::get_metadata_sdk", skip_all)]
    pub async fn get_metadata_sdk(
        &self,
        uri: &str,
        user_metadata_keys: Vec<String>,
        additional_headers: Option<AdditionalHeaders>,
    ) -> Result<HashMap<String, Value>> {
        let ResolvedUri {
            mut address,
            expected_etag,
            version_id,
        } = resolve_metadata_uri(uri)?;

        let client = self.client(&mut address).await?;
        let bucket = &client.bucket;
        let key = address.key()?.into_owned();

        let mut builder = client.head_object().bucket(bucket).key(&key);
        if let Some(ref vid) = version_id {
            builder = builder.version_id(vid.clone());
        }
        if let Some(ref etag) = expected_etag {
            builder = builder.if_match(etag);
        }

        let builder = Self::apply_request_context(
            builder.customize(),
            additional_headers,
            RequestType::Api,
            client.proxy_interceptor.clone(),
            None,
            client.rate_limit_interceptors.head_object.clone(),
        );

        let head = builder
            .send()
            .instrument(info_span!("HeadObject"))
            .await
            .map_err(error::SdkError::from)
            .context(error::Sdk {})?;
        trace_limited!(?head);

        let normalized_keys: Vec<String> = user_metadata_keys.into_iter().map(normalize_metadata_key).collect();

        let metadata = head
            .metadata
            .unwrap_or_default()
            .into_iter()
            .map(|(key, val)| {
                let decoded_key = decode_metadata_key(&key);
                let value = decode_metadata_value(val).map_err(Error::from)?;
                Ok((decoded_key, value))
            })
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .filter(|(key, _)| normalized_keys.is_empty() || normalized_keys.contains(key))
            .collect::<HashMap<String, Value>>();

        Ok(metadata)
    }

    /// Read-modify-write helper for metadata mutations.
    /// HeadObject -> modify -> CopyObject to self with `copy_source_if_match`.
    ///
    /// The `modify` closure mutates the metadata map and returns `true` if a
    /// write-back is needed, or `false` to skip (e.g. deleting a key that
    /// doesn't exist — there's nothing to write).
    ///
    /// Concurrency control depends on the URI type:
    /// - **Plain address** (`expected_etag` and `version_id` both `None`):
    ///   HeadObject has no precondition; CopyObject uses the object's current
    ///   etag as `copy_source_if_match` for optimistic concurrency.
    /// - **Identity** (`expected_etag` is `Some`): HeadObject uses `If-Match`
    ///   with the identity's etag (server rejects if stale); CopyObject still
    ///   uses the current etag from the head response for the guard.
    /// - **Versioned** (`version_id` is `Some`): after HeadObject, the head's
    ///   version ID is compared client-side to ensure the expected version is
    ///   still the head before proceeding with the write.
    async fn modify_metadata(
        &self,
        uri: &str,
        additional_headers: Option<AdditionalHeaders>,
        modify: impl FnOnce(&mut HashMap<String, String>) -> bool,
    ) -> Result<()> {
        let ResolvedUri {
            mut address,
            expected_etag,
            version_id,
        } = resolve_metadata_uri(uri)?;

        let client = self.client(&mut address).await?;
        let bucket = client.bucket.clone();
        let key = address.key()?.into_owned();

        let mut head_builder = client.head_object().bucket(&bucket).key(&key);
        if let Some(ref etag) = expected_etag {
            head_builder = head_builder.if_match(etag);
        }
        let head_builder = Self::apply_request_context(
            head_builder.customize(),
            additional_headers.clone(),
            RequestType::Api,
            client.proxy_interceptor.clone(),
            None,
            client.rate_limit_interceptors.head_object.clone(),
        );
        let head = head_builder
            .send()
            .instrument(info_span!("HeadObject"))
            .await
            .map_err(error::SdkError::from)
            .context(error::Sdk {})?;
        trace_limited!(?head);

        if let Some(ref expected_vid) = version_id {
            let current_vid = head.version_id.as_deref().ok_or_else(|| {
                Error::precondition_failed("server did not return a version_id; versioning may not be enabled")
            })?;
            if expected_vid != current_vid {
                trace_limited!(
                    expected_version = expected_vid,
                    current_version = current_vid,
                    "version-id precondition failed"
                );
                return Err(Error::precondition_failed(
                    "version-id precondition failed: the provided version is not the current head",
                )
                .into());
            }
        }

        let current_etag = head
            .e_tag
            .clone()
            .ok_or_else(|| Error::internal("HeadObject response missing ETag"))?;

        let mut metadata = head.metadata.unwrap_or_default();
        if !modify(&mut metadata) {
            return Ok(());
        }

        let copy_source = format!("{bucket}/{key}");
        let mut copy_builder = client.copy_object().bucket(&bucket).copy_source(copy_source).key(&key);
        copy_builder = copy_builder.metadata_directive(MetadataDirective::Replace);
        copy_builder = copy_builder.set_metadata(Some(metadata));
        copy_builder = copy_builder.copy_source_if_match(current_etag);

        let copy_builder = Self::apply_request_context(
            copy_builder.customize(),
            additional_headers,
            RequestType::Api,
            client.proxy_interceptor.clone(),
            None,
            client.rate_limit_interceptors.copy_object.clone(),
        );
        copy_builder
            .send()
            .instrument(info_span!("CopyObject"))
            .await
            .map_err(error::SdkError::from)
            .context(error::Sdk {})?;

        Ok(())
    }

    /// Update a single user metadata key on a resource.
    ///
    /// Accepts a plain address, versioned address, or identity URI (see `resolve_metadata_uri`).
    /// Precondition and concurrency behavior is handled by `modify_metadata`.
    #[instrument(name = "s3::update_metadata_sdk", skip_all)]
    pub async fn update_metadata_sdk(
        &self,
        uri: &str,
        user_metadata_key: String,
        user_metadata: Value,
        additional_headers: Option<AdditionalHeaders>,
    ) -> Result<()> {
        self.modify_metadata(uri, additional_headers, |metadata| {
            metadata.insert(
                encode_metadata_key(&user_metadata_key, is_valid_s3_metadata_key),
                encode_metadata_value(&user_metadata),
            );
            true
        })
        .await
    }

    /// Delete a single user metadata key from a resource.
    ///
    /// Accepts a plain address, versioned address, or identity URI (see `resolve_metadata_uri`).
    /// Precondition and concurrency behavior is handled by `modify_metadata`. If the key
    /// does not exist, the CopyObject step is skipped and the call returns `Ok(())`.
    #[instrument(name = "s3::delete_metadata_sdk", skip_all)]
    pub async fn delete_metadata_sdk(
        &self,
        uri: &str,
        user_metadata_key: String,
        additional_headers: Option<AdditionalHeaders>,
    ) -> Result<()> {
        self.modify_metadata(uri, additional_headers, |metadata| {
            let encoded_key = encode_metadata_key(&user_metadata_key, is_valid_s3_metadata_key);
            metadata.remove(&encoded_key).is_some()
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_plain_address() {
        let uri = "https://bucket.s3.amazonaws.com/key";
        let resolved = resolve_metadata_uri(uri).unwrap();
        assert_eq!(resolved.address.to_string(), uri);
        assert!(resolved.expected_etag.is_none());
        assert!(resolved.version_id.is_none());
    }

    #[test]
    fn resolve_identity_uri() {
        let uri = "https://bucket.s3.amazonaws.com/key?etag=%22abc123%22&content-length=100&version-id=v1";
        let resolved = resolve_metadata_uri(uri).unwrap();
        assert_eq!(resolved.address.to_string(), "https://bucket.s3.amazonaws.com/key");
        assert_eq!(resolved.expected_etag.as_deref(), Some("\"abc123\""));
        assert_eq!(resolved.version_id.as_deref(), Some("v1"));
    }

    #[test]
    fn resolve_versioned_address() {
        let uri = "https://bucket.s3.amazonaws.com/key?version-id=v42";
        let resolved = resolve_metadata_uri(uri).unwrap();
        assert_eq!(resolved.address.to_string(), uri);
        assert!(resolved.expected_etag.is_none());
        assert_eq!(resolved.version_id.as_deref(), Some("v42"));
    }

    #[test]
    fn s3_validator_accepts_tchar_keys() {
        assert!(is_valid_s3_metadata_key("content-type"));
        assert!(is_valid_s3_metadata_key("my_tag"));
        assert!(is_valid_s3_metadata_key("x-amz-meta!custom"));
        assert!(is_valid_s3_metadata_key("key.with.dots"));
    }

    #[test]
    fn s3_validator_rejects_non_tchar_keys() {
        assert!(!is_valid_s3_metadata_key("my key"));
        assert!(!is_valid_s3_metadata_key("my:key"));
        assert!(!is_valid_s3_metadata_key("my/key"));
        assert!(!is_valid_s3_metadata_key("my=key"));
        assert!(!is_valid_s3_metadata_key("my(key)"));
    }

    #[test]
    fn s3_encode_roundtrip() {
        let keys = ["content-type", "my_tag", "my key", "my:key", "nv_key_foo"];
        for key in keys {
            let encoded = encode_metadata_key(key, is_valid_s3_metadata_key);
            let decoded = decode_metadata_key(&encoded);
            assert_eq!(decoded, key.to_lowercase(), "roundtrip failed for: {key}");
        }
    }

    #[test]
    fn s3_valid_keys_passthrough() {
        // Hyphens and dots are valid tchar, should not be encoded
        assert_eq!(
            encode_metadata_key("content-type", is_valid_s3_metadata_key),
            "content-type"
        );
        assert_eq!(
            encode_metadata_key("key.with.dots", is_valid_s3_metadata_key),
            "key.with.dots"
        );
    }
}
