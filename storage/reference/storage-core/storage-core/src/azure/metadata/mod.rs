// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

pub mod v1alpha;

use std::collections::HashMap;

use azure_core::request_options::{IfMatchCondition, Metadata};
use azure_storage_blobs::prelude::{BlobClient, VersionId};
use storage_core_builder::traits::metadata::v1alpha::Value;
use storage_core_util::additional_headers::{AdditionalHeaders, RequestType};
use storage_core_util::metadata::{
    decode_metadata_key, decode_metadata_value, encode_metadata_key, encode_metadata_value, normalize_metadata_key,
};
use storage_core_util::trace_limited;
use url::Url;

use crate::{
    azure::{
        error::Error,
        resource::{Address, Identity, ETAG},
        service::Storage,
    },
    traits::error::Result,
};

/// Resolved metadata URI components extracted by [`resolve_metadata_uri`].
struct ResolvedUri {
    /// The clean resource address (query params stripped for identities).
    address: Address,
    /// Etag extracted from an identity URI. Used as an `If-Match` precondition on
    /// GetProperties/SetMetadata to ensure the identity still matches the current blob.
    /// `None` for plain and versioned addresses.
    expected_etag: Option<String>,
    /// Version ID extracted from an identity or versioned address. For reads, selects
    /// the specific blob version via `BlobVersioning`. For mutations, verified against
    /// the current blob's version ID to ensure the caller's expected version is still head.
    /// `None` for plain addresses.
    version_id: Option<String>,
}

/// Parse a metadata `uri` into its components. The URI may be one of three forms:
///
/// 1. **Plain address** (e.g. `https://account.blob.core.windows.net/container/blob`):
///    no preconditions are applied.
/// 2. **Versioned address** (`?version-id=…`, no `etag`): the version ID is extracted.
///    Reads target that specific version; mutations verify it is still the head.
/// 3. **Identity** (`?etag=…` and optionally `?version-id=…`): both the etag and version
///    ID are extracted. The etag is used as an `If-Match` server-side precondition, and
///    the version ID is verified client-side against the head.
///
/// Azure's `Identity::parse` does not require an etag (unlike S3), so we explicitly
/// check for the `etag` query param before entering the identity path. If present,
/// we enforce that `Identity::parse` actually produced an etag value.
fn resolve_metadata_uri(uri: &str) -> Result<ResolvedUri> {
    let url = Url::parse(uri).map_err(Error::from)?;

    // Check for identity (has etag param). Azure Identity::parse doesn't require etag,
    // so we check explicitly to distinguish identity from versioned address.
    let has_etag = url.query_pairs().any(|(k, _)| k == ETAG);

    if has_etag {
        let identity = Identity::parse(uri)?;
        let etag = identity
            .etag
            .ok_or_else(|| Error::internal("identity URI has etag query param but Identity::parse returned None"))?;
        let version_id = identity.version_id;
        // Identity::parse already strips query params from the address
        let address = identity.resource_address;
        return Ok(ResolvedUri {
            address,
            expected_etag: Some(etag),
            version_id,
        });
    }

    // Versioned or plain address
    let address = Address::parse_blob(url)?;
    let version_id = address.version_id().map(|v| v.to_string());
    Ok(ResolvedUri {
        address,
        expected_etag: None,
        version_id,
    })
}

/// Returns `true` if `key` is a valid Azure Blob metadata key.
///
/// Azure requires C# identifier rules: must start with a letter or underscore,
/// remaining characters must be letters, digits, or underscores.
///
/// Assumes `key` is already lowercased.
pub(crate) fn is_valid_azure_metadata_key(key: &str) -> bool {
    if key.is_empty() {
        return false;
    }
    let first = key.as_bytes()[0];
    if !first.is_ascii_alphabetic() && first != b'_' {
        return false;
    }
    key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

impl Storage {
    async fn blob_client_from_address(&self, address: &Address) -> Result<BlobClient> {
        let blob_client = self
            .client(address)
            .await?
            .container_client(address.container()?)
            .blob_client(address.blob());
        Ok(blob_client)
    }

    /// Read-modify-write helper for metadata mutations with internal optimistic concurrency.
    /// GetProperties -> modify -> SetMetadata.
    ///
    /// The `modify` closure returns `true` if it changed the metadata and a write-back is
    /// needed, or `false` to skip the write (e.g. deleting a key that doesn't exist).
    ///
    /// Concurrency control depends on the URI type:
    /// - **Plain address** (`expected_etag` and `version_id` are both `None`):
    ///   GetProperties has no precondition; SetMetadata uses the blob's current etag
    ///   as `If-Match` for optimistic concurrency.
    /// - **Identity** (`expected_etag` is `Some`): the identity's etag is used as
    ///   `If-Match` on *both* GetProperties and SetMetadata — the server rejects if
    ///   the blob has been modified since the identity was issued.
    /// - **Versioned** (`version_id` is `Some`): after GetProperties, the blob's
    ///   current version ID is compared client-side to ensure the expected version is
    ///   still the head before proceeding with the write.
    async fn modify_metadata(
        &self,
        address: &Address,
        additional_headers: Option<AdditionalHeaders>,
        expected_etag: Option<&str>,
        version_id: Option<&str>,
        modify: impl FnOnce(&mut Metadata) -> bool,
    ) -> Result<()> {
        let blob_client = self.blob_client_from_address(address).await?;

        let mut get_builder = blob_client.get_properties();
        if let Some(etag) = expected_etag {
            get_builder = get_builder.if_match(IfMatchCondition::Match(etag.to_string()));
        }
        if let Some(ref additional_headers) = additional_headers {
            get_builder = get_builder.context(Self::create_custom_context(
                additional_headers.clone(),
                RequestType::Api,
            )?);
        }

        let get_response = get_builder.into_future().await.map_err(Error::from)?;

        if let Some(expected_vid) = version_id {
            let current_vid = get_response.blob.version_id.as_deref().ok_or_else(|| {
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

        // Convert blob metadata (Option<HashMap>) to Metadata type
        let mut metadata: Metadata = Metadata::new();
        if let Some(map) = get_response.blob.metadata {
            for (k, v) in map {
                metadata.insert(k, v);
            }
        }

        if !modify(&mut metadata) {
            return Ok(());
        }

        // Use the identity's expected etag for if_match when provided — this both
        // validates identity freshness and guards against concurrent changes.
        // For plain addresses, fall back to the current etag for optimistic concurrency.
        let etag_for_match = expected_etag
            .map(|e| e.to_string())
            .unwrap_or_else(|| get_response.blob.properties.etag.to_string());

        let mut set_builder = blob_client
            .set_metadata()
            .if_match(IfMatchCondition::Match(etag_for_match))
            .metadata(metadata);
        if let Some(additional_headers) = additional_headers {
            set_builder = set_builder.context(Self::create_custom_context(additional_headers, RequestType::Api)?);
        }

        set_builder.into_future().await.map_err(Error::from)?;

        Ok(())
    }

    /// Fetch user metadata for a resource.
    ///
    /// Accepts a plain address, versioned address, or identity URI (see `resolve_metadata_uri`).
    ///
    /// - **Plain address**: GetProperties with no preconditions; returns current metadata.
    /// - **Versioned address**: GetProperties targets the specific version via `BlobVersioning`.
    /// - **Identity**: GetProperties uses `If-Match` with the identity's etag (server rejects
    ///   if stale) and targets the specific version via `BlobVersioning` if present.
    pub async fn get_metadata_internal(
        &self,
        uri: &str,
        user_metadata_keys: Vec<String>,
        additional_headers: Option<AdditionalHeaders>,
    ) -> Result<HashMap<String, Value>> {
        let ResolvedUri {
            address,
            expected_etag,
            version_id,
        } = resolve_metadata_uri(uri)?;

        let blob_client = self.blob_client_from_address(&address).await?;

        let mut builder = blob_client.get_properties();
        if let Some(ref vid) = version_id {
            builder = builder.blob_versioning(VersionId::new(vid.clone()));
        }
        if let Some(ref etag) = expected_etag {
            builder = builder.if_match(IfMatchCondition::Match(etag.clone()));
        }

        if let Some(additional_headers) = additional_headers {
            builder = builder.context(Self::create_custom_context(additional_headers, RequestType::Api)?);
        }

        let get_response = builder.into_future().await.map_err(Error::from)?;

        let normalized_keys: Vec<String> = user_metadata_keys.into_iter().map(normalize_metadata_key).collect();

        // Convert blob metadata (Option<HashMap>) to decoded keys and values
        let metadata = get_response
            .blob
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

    /// Update a single user metadata key on a resource.
    ///
    /// Accepts a plain address, versioned address, or identity URI (see `resolve_metadata_uri`).
    /// Precondition and concurrency behavior is handled by `modify_metadata`.
    pub async fn update_metadata_internal(
        &self,
        uri: &str,
        user_metadata_key: String,
        user_metadata: Value,
        additional_headers: Option<AdditionalHeaders>,
    ) -> Result<()> {
        let ResolvedUri {
            address,
            expected_etag,
            version_id,
        } = resolve_metadata_uri(uri)?;

        self.modify_metadata(
            &address,
            additional_headers,
            expected_etag.as_deref(),
            version_id.as_deref(),
            |metadata| {
                let encoded_key = encode_metadata_key(&user_metadata_key, is_valid_azure_metadata_key);
                metadata.insert(&encoded_key, encode_metadata_value(&user_metadata));
                true
            },
        )
        .await
    }

    /// Delete a single user metadata key from a resource.
    ///
    /// Accepts a plain address, versioned address, or identity URI (see `resolve_metadata_uri`).
    /// Precondition and concurrency behavior is handled by `modify_metadata`.
    pub async fn delete_metadata_internal(
        &self,
        uri: &str,
        user_metadata_key: String,
        additional_headers: Option<AdditionalHeaders>,
    ) -> Result<()> {
        let ResolvedUri {
            address,
            expected_etag,
            version_id,
        } = resolve_metadata_uri(uri)?;

        self.modify_metadata(
            &address,
            additional_headers,
            expected_etag.as_deref(),
            version_id.as_deref(),
            |metadata| {
                let encoded_key = encode_metadata_key(&user_metadata_key, is_valid_azure_metadata_key);
                metadata.as_mut().remove(&encoded_key).is_some()
            },
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_plain_address() {
        let uri = "https://account.blob.core.windows.net/container/blob";
        let resolved = resolve_metadata_uri(uri).unwrap();
        assert_eq!(resolved.address.to_string(), uri);
        assert!(resolved.expected_etag.is_none());
        assert!(resolved.version_id.is_none());
    }

    #[test]
    fn resolve_identity_uri() {
        let uri =
            "https://account.blob.core.windows.net/container/blob?etag=%22abc123%22&content-length=100&version-id=v1";
        let resolved = resolve_metadata_uri(uri).unwrap();
        assert_eq!(
            resolved.address.to_string(),
            "https://account.blob.core.windows.net/container/blob"
        );
        assert_eq!(resolved.expected_etag.as_deref(), Some("\"abc123\""));
        assert_eq!(resolved.version_id.as_deref(), Some("v1"));
    }

    #[test]
    fn resolve_versioned_address() {
        let uri = "https://account.blob.core.windows.net/container/blob?version-id=v42";
        let resolved = resolve_metadata_uri(uri).unwrap();
        assert_eq!(resolved.address.to_string(), uri);
        assert!(resolved.expected_etag.is_none());
        assert_eq!(resolved.version_id.as_deref(), Some("v42"));
    }

    #[test]
    fn azure_validator_accepts_valid_keys() {
        assert!(is_valid_azure_metadata_key("my_tag"));
        assert!(is_valid_azure_metadata_key("_leading"));
        assert!(is_valid_azure_metadata_key("abc123"));
    }

    #[test]
    fn azure_validator_rejects_invalid_keys() {
        assert!(!is_valid_azure_metadata_key("content-type"));
        assert!(!is_valid_azure_metadata_key("last.modified"));
        assert!(!is_valid_azure_metadata_key("1st_author"));
        assert!(!is_valid_azure_metadata_key("my key"));
    }

    #[test]
    fn azure_validator_rejects_empty_key_without_panicking() {
        // Indexing key.as_bytes()[0] on an empty key would panic; the empty-string
        // check at the top of the validator guards against that.
        assert!(!is_valid_azure_metadata_key(""));
    }

    #[test]
    fn azure_encode_roundtrip() {
        let keys = ["content-type", "my_tag", "1st-author", "my key", "nv_key_foo"];
        for key in keys {
            let encoded = encode_metadata_key(key, is_valid_azure_metadata_key);
            let decoded = decode_metadata_key(&encoded);
            assert_eq!(decoded, key.to_lowercase(), "roundtrip failed for: {key}");
        }
    }
}
