// SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{
    borrow::{Borrow, Cow},
    fmt,
    hash::Hash,
    ops::Deref,
};

use aws_sdk_s3::types::Object;
use percent_encoding::percent_decode_str;
use serde::{Deserialize, Serialize};
use storage_core_protos::google;
use storage_core_util::api_types::{AddressInfo, ResourceInfo};
use storage_core_util::trace_limited;
use storage_core_util::url_path::{canonicalize_url_path, encode_url_path};
use tracing::instrument;
use url::{Position, Url};

use super::error::{Error, InvalidArgument, Result};

// custom query parameters that make up a resource identity (vs resource address)
pub const CONTENT_LENGTH: &str = "content-length";
pub const ETAG: &str = "etag";
pub const LAST_MODIFIED: &str = "last-modified";
pub const VERSION_ID: &str = "version-id";
// CONTENT_LENGTH and LAST_MODIFIED are intentionally retained in IDENTITY_PARAMS
// even though current Identity instances no longer carry these fields. The
// Display impl uses this list to STRIP non-identity params from the URL before
// re-appending its own; keeping these names here means any legacy identity URL
// (produced before the etag-only identity change) round-trips cleanly — the now-
// redundant params get silently dropped on re-serialization.
const IDENTITY_PARAMS: &[&str] = &[CONTENT_LENGTH, ETAG, LAST_MODIFIED, VERSION_ID];

fn is_identity_param(name: &str) -> bool {
    IDENTITY_PARAMS.iter().any(|param| name.eq_ignore_ascii_case(param))
}

/// Possible addressing styles for a resource addresses
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AddressingStyle {
    /// Path addressing style, e.g. <https://host/bucket/key>
    Path,
    /// Virtual addressing style, e.g. <https://bucket.host/key>
    #[default]
    Virtual,
}

// Resource Address is a S3 http/https URL where any query parameters or fragments are ignored
/// Default maximum object key size in bytes. This matches the S3 spec and most S3-compatible
/// backends. Can be overridden via the `max_key_bytes` config option.
pub const DEFAULT_MAX_KEY_BYTES: usize = 1024;

#[derive(Debug, Clone)]
pub struct Address {
    pub url: Url,
    pub addressing_style: AddressingStyle,
    pub(super) max_key_bytes: usize,
}

impl Deref for Address {
    type Target = Url;

    fn deref(&self) -> &Self::Target {
        &self.url
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(&self.url, f)
    }
}

// Hash the address up to the end of the path. There shouldn't be a query or fragment of the resource address,
// but we make sure we don't hash them if there are.
impl Hash for Address {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.url[..Position::AfterPath].hash(state);
    }
}

// Similar to Hash, we only compare the url up to the end of the path. We do not include the AddressingStyle
// because actually computing the addressing style requires looking up the resource address in the client map,
// which not all functions do, and it doesn't actually matter for the purposes of equality.
impl PartialEq for Address {
    fn eq(&self, other: &Self) -> bool {
        self.url[..Position::AfterPath] == other.url[..Position::AfterPath]
    }
}

impl Eq for Address {}

impl Address {
    /// parse a input string, a url, into a resource address
    #[instrument(name = "s3::parse", skip_all)]
    pub fn parse<T>(input: T) -> Result<Self>
    where
        T: AsRef<str> + fmt::Debug,
    {
        let url = Url::parse(input.as_ref())?;
        trace_limited!(%url);
        url.try_into()
    }

    /// Parse a folder address from a string.
    pub fn parse_folder_address(url: &str) -> Result<Self> {
        Self::parse(add_trailing_slash_if_missing(url))
    }

    /// get the host part of the resource address.
    /// FUTURE(jroback) currently we don't support IPv4/IPv6 addresses as hosts.
    /// we might want to do this in the future to support things like MinIO on localhost:port
    pub fn host(&self) -> Result<&str> {
        self.url.host_str().ok_or(
            InvalidArgument {
                message: format!("hostname of '{self}' is empty"),
            }
            .build(),
        )
    }

    /// Get the key to look up in the config map.
    /// For virtual addressing style, the key is the scheme://authority
    /// For path addressing style, the key is the scheme://authority/bucket
    pub fn config_key(&self, addressing_style: AddressingStyle) -> &str {
        match addressing_style {
            AddressingStyle::Path => {
                // Determine the length of the bucket name
                let bucket_len = self
                    .url
                    .path()
                    .chars()
                    .enumerate()
                    .skip(1) // Skip leading '/'
                    .find(|(_, c)| *c == '/') // Find next '/'
                    .map(|(i, _)| i)
                    .unwrap_or(self.url.path().len());

                // Trim the rest of the path from the url, leaving the bucket name
                let to_trim = self.url.path().len() - bucket_len;
                let full_path = &self.url[..Position::AfterPath];
                &full_path[..full_path.len() - to_trim]
            },
            AddressingStyle::Virtual => &self.url[..Position::BeforePath],
        }
    }

    /// get the key portion of the resource address, url decoded
    pub fn key(&self) -> Result<Cow<'_, str>> {
        let key_start = match self.addressing_style {
            AddressingStyle::Path => {
                // Split path into segments and skip the first two components:
                // First '/' and bucket name
                let path = self.url.path();
                path.chars()
                    .enumerate()
                    .skip(1) // Skip leading '/'
                    .find(|(_, c)| *c == '/') // Find next '/'
                    .map(|(i, _)| i + 1) // Get position after '/'
                    .unwrap_or(path.len())
            },
            // Skip the leading '/' for virtual addressing style
            AddressingStyle::Virtual => 1,
        };

        let key = percent_decode_str(&self.url.path()[key_start..])
            .decode_utf8()
            .map_err(|e| {
                InvalidArgument {
                    message: format!("percent decode failed for '{self}': {e}"),
                }
                .build()
            })?;
        validate_object_key(&key, &self.url, self.max_key_bytes)?;
        Ok(key)
    }

    /// Returns the first path segment of the resource address, url decoded,
    /// which may be the bucket name if the addressing style is path style.
    pub fn path_style_bucket_name(&self) -> Result<Cow<'_, str>> {
        let bucket_end = self
            .url
            .path()
            .chars()
            .enumerate()
            .skip(1) // Skip leading '/'
            .find(|(_, c)| *c == '/') // Find next '/'
            .map(|(i, _)| i)
            .unwrap_or(self.url.path().len());
        let bucket = &self.url.path()[1..bucket_end];
        percent_decode_str(bucket).decode_utf8().map_err(|e| {
            InvalidArgument {
                message: format!("percent decode failed for '{self}': {e}"),
            }
            .build()
        })
    }

    pub fn bucket_address(&self) -> Result<BucketAddress> {
        let mut bucket_url = self.url.clone();
        bucket_url.set_fragment(None);
        bucket_url.set_query(None);

        match self.addressing_style {
            AddressingStyle::Path => {
                bucket_url.set_path(&self.path_style_bucket_name()?);
            },
            AddressingStyle::Virtual => bucket_url.set_path(""),
        }

        Ok(BucketAddress::new(Address {
            url: bucket_url,
            addressing_style: self.addressing_style,
            max_key_bytes: self.max_key_bytes,
        }))
    }

    pub fn is_versioned(&self) -> bool {
        self.version_id().is_some()
    }

    pub fn version_id(&self) -> Option<Cow<'_, str>> {
        self.url.query_pairs().find(|(k, _)| k == VERSION_ID).map(|(_, v)| v)
    }

    pub fn create_versioned_resource_address(&self, version_id: &str) -> String {
        let mut url = self.url.clone();
        let non_identity_params: Vec<_> = url
            .query_pairs()
            .filter(|(name, _)| !is_identity_param(name.as_ref()))
            .map(|(name, value)| (name.into_owned(), value.into_owned()))
            .collect();
        url.set_query(None);
        if !non_identity_params.is_empty() {
            url.query_pairs_mut().extend_pairs(&non_identity_params);
        }
        if !version_id.is_empty() && version_id != "null" {
            url.query_pairs_mut().append_pair(VERSION_ID, version_id);
        }
        url.to_string()
    }
}

impl From<Address> for String {
    fn from(value: Address) -> Self {
        value.url.into()
    }
}

/// convert from Url to resource Address
impl TryFrom<Url> for Address {
    type Error = Error;

    #[instrument(name = "s3::try_from", skip_all)]
    fn try_from(mut url: Url) -> Result<Self, Self::Error> {
        if url.scheme() != "http" && url.scheme() != "https" {
            return InvalidArgument {
                message: format!("invalid scheme for '{url}', only http/https supported"),
            }
            .fail();
        }

        canonicalize_url_path(&mut url).map_err(|e| {
            InvalidArgument {
                message: format!("URL path canonicalization failed for '{url}': {e}"),
            }
            .build()
        })?;

        Ok(Self {
            url,
            addressing_style: AddressingStyle::Virtual,
            max_key_bytes: DEFAULT_MAX_KEY_BYTES,
        })
    }
}

// S3 Resource Identity consists of:
//  - resource address
//  - etag
//  - version id (for versioned buckets)
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct Identity {
    pub resource_address: Address,
    pub etag: Option<String>,
    pub version_id: Option<String>,
}

impl fmt::Display for Identity {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut url = self.resource_address.url.clone();
        url.set_query(None);

        // We want to keep all the non-identity params, but we need to remove the identity
        // params before we add our own to avoid any duplicates.
        let mut non_identity_params = self
            .resource_address
            .url
            .query_pairs()
            .filter(|(name, _)| !is_identity_param(name.as_ref()))
            .peekable();

        // only extend the params if there are other query parameters
        // if you do a clear().extend_pairs() and identity_params is empty, it will result in
        // query params being Some("") vs None, which means all generated urls will have a trailing '?'
        if non_identity_params.peek().is_some() {
            url.query_pairs_mut().extend_pairs(non_identity_params);
        }

        // add the identity params. Identity is composed of (etag, version-id) only;
        // content-length and last-modified used to be encoded here but were removed
        // when Identity became etag-only — they're metadata, not identity, and live on
        // ResourceInfo as separate fields.
        if let Some(etag) = &self.etag {
            url.query_pairs_mut().append_pair(ETAG, etag.as_str());
        }

        if let Some(version_id) = &self.version_id {
            if !version_id.is_empty() && version_id.as_str() != "null" {
                url.query_pairs_mut().append_pair(VERSION_ID, version_id.as_str());
            }
        }

        trace_limited!(%url);
        fmt::Display::fmt(&url, f)
    }
}

impl Identity {
    // parse input (url/str) into url
    #[instrument(name = "s3::parse", skip_all)]
    pub fn parse<T>(input: T) -> Result<Self>
    where
        T: AsRef<str> + fmt::Display + fmt::Debug,
    {
        let url = Url::parse(input.as_ref())?;
        trace_limited!(?url);

        let mut etag = None;
        let mut version_id = None;

        // Parse out the identity metadata from the URL query parameters.
        // CONTENT_LENGTH and LAST_MODIFIED were removed from Identity when it
        // became etag-only — silently ignore them if present on legacy identity URLs so
        // older clients holding older identities continue to work.
        for query_pair in url.query_pairs() {
            match query_pair.0.to_lowercase().as_str() {
                ETAG => {
                    etag = Some(query_pair.1.to_string());
                },
                VERSION_ID => {
                    version_id = Some(query_pair.1.to_string());
                },
                CONTENT_LENGTH | LAST_MODIFIED => {
                    // Legacy identity params — accept and ignore, they're no longer
                    // part of the identity. The IDENTITY_PARAMS list will strip them
                    // on re-serialization.
                },
                _ => {
                    // ignore others, they maybe part of the users url
                    trace_limited!(%url, ?query_pair, "unknown query parameter found in resource identity");
                },
            }
        }

        if etag.is_none() {
            return InvalidArgument {
                message: format!("etag is missing for '{url}'"),
            }
            .fail();
        }

        let identity = Self {
            resource_address: Address::try_from(url)?,
            etag,
            version_id,
        };
        trace_limited!(%identity);

        Ok(identity)
    }

    pub fn key(&self) -> Result<Cow<'_, str>> {
        self.resource_address.key()
    }

    pub fn etag(&self) -> Result<&String> {
        self.etag.as_ref().ok_or(
            InvalidArgument {
                message: format!("etag is missing for '{self}'"),
            }
            .build(),
        )
    }
}

impl From<(Address, Option<String>, Option<String>)> for Identity {
    fn from(value: (Address, Option<String>, Option<String>)) -> Self {
        Identity {
            resource_address: value.0,
            etag: value.1,
            version_id: value.2,
        }
    }
}

#[derive(Debug)]
pub struct BucketAddress(Address);

impl BucketAddress {
    /// Attempt to extract the bucket address from a url.
    fn new(address: Address) -> Self {
        Self(address)
    }

    /// Return the address of the resource in the bucket.
    pub fn get_resource_address(&self, key: &str) -> Address {
        let mut url = self.0.url.clone();
        debug_assert!(url.query().is_none());
        debug_assert!(url.fragment().is_none());

        // When the path component is modified through the path_segments_mut method, whitespace characters
        // such as \t, \r, and \n are silently discarded...
        let path = match self.0.addressing_style {
            AddressingStyle::Path => format!("{}/{}", url.path(), encode_path(key)),
            AddressingStyle::Virtual => format!("/{}", encode_path(key)),
        };
        url.set_path(&path);

        Address {
            url,
            addressing_style: self.0.addressing_style,
            max_key_bytes: self.0.max_key_bytes,
        }
    }

    /// Return the address of an object in the bucket.
    pub fn get_object_address(&self, key: &str) -> Address {
        self.get_resource_address(key)
    }

    /// Return the address of a folder in the bucket.
    pub fn get_folder_address(&self, key: &str) -> Address {
        debug_assert!(key.ends_with('/'));
        self.get_resource_address(key)
    }
}

impl Deref for BucketAddress {
    type Target = Url;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Add a trailing slash to a string, if not present.
fn add_trailing_slash_if_missing(value: &str) -> Cow<'_, str> {
    if value.ends_with('/') {
        Cow::Borrowed(value)
    } else {
        Cow::Owned(format!("{value}/"))
    }
}

//
// conversions from local respresntations into grpc structures
//

impl TryFrom<&ResourceInfo> for Identity {
    type Error = Error;
    fn try_from(value: &ResourceInfo) -> Result<Self, Self::Error> {
        Self::parse(&value.encoded_identity)
    }
}

impl TryFrom<Option<&crate::traits::fileobject::v1alpha::ResourceIdentity>> for Identity {
    type Error = Error;
    fn try_from(value: Option<&crate::traits::fileobject::v1alpha::ResourceIdentity>) -> Result<Self, Self::Error> {
        let resource_identity = value.ok_or(
            InvalidArgument {
                message: "resource identity empty",
            }
            .build(),
        )?;
        Self::parse(&resource_identity.encoded_identity)
    }
}

impl TryFrom<crate::traits::fileobject::v1alpha::ResourceInfo> for Identity {
    type Error = Error;
    fn try_from(value: crate::traits::fileobject::v1alpha::ResourceInfo) -> Result<Self, Self::Error> {
        let resource_identity = value.resource_identity.ok_or(
            InvalidArgument {
                message: "resource identity empty",
            }
            .build(),
        )?;
        Self::parse(&resource_identity.encoded_identity)
    }
}

impl TryFrom<Option<&crate::traits::fileobject::v1beta::ResourceIdentity>> for Identity {
    type Error = Error;
    fn try_from(value: Option<&crate::traits::fileobject::v1beta::ResourceIdentity>) -> Result<Self, Self::Error> {
        let resource_identity = value.ok_or(
            InvalidArgument {
                message: "resource identity empty",
            }
            .build(),
        )?;
        Self::parse(&resource_identity.encoded_identity)
    }
}

impl TryFrom<Option<crate::traits::fileobject::v1alpha::ResourceInfo>> for Identity {
    type Error = Error;
    fn try_from(value: Option<crate::traits::fileobject::v1alpha::ResourceInfo>) -> Result<Self, Self::Error> {
        let resource_info = value.ok_or(
            InvalidArgument {
                message: "resource identity empty",
            }
            .build(),
        )?;

        Self::try_from(resource_info)
    }
}

impl TryFrom<Option<crate::traits::fileobject::v1beta::ResourceInfo>> for Identity {
    type Error = Error;
    fn try_from(value: Option<crate::traits::fileobject::v1beta::ResourceInfo>) -> Result<Self, Self::Error> {
        let resource_info = value.ok_or(
            InvalidArgument {
                message: "resource identity empty",
            }
            .build(),
        )?;
        let resource_identity = resource_info.resource_identity.ok_or(
            InvalidArgument {
                message: "resource identity empty",
            }
            .build(),
        )?;
        Self::parse(&resource_identity.encoded_identity)
    }
}

pub struct WrappedResourceInfo(pub ResourceInfo);
pub struct WrappedAddressInfo(pub AddressInfo);

// Pass-through conversion from Identity → ResourceInfo.
//
// The Identity no longer carries `content_length` or
// `last_modified`, so this impl only populates `encoded_identity`. Callers that
// need `data_object_size` / `last_modified_timestamp` should use the richer
// tuple impl below (which takes them alongside the identity) or build
// `ResourceInfo` explicitly from the SDK response.
impl<T> From<T> for WrappedResourceInfo
where
    T: Borrow<Identity>,
{
    fn from(value: T) -> Self {
        Self(ResourceInfo {
            native_metadata: Default::default(),
            encoded_identity: value.borrow().to_string(),
            data_object_size: None,
            last_modified_timestamp: None,
        })
    }
}

// Construct a `WrappedResourceInfo` from an identity plus the metadata that
// used to be embedded inside it. Callers that have `data_object_size` and/or
// `last_modified_timestamp` available from a stat/head response should prefer
// this form so the resulting `ResourceInfo` carries the metadata as separate
// fields (it's no longer part of the identity URL).
impl From<(&Identity, Option<u64>, Option<google::protobuf::Timestamp>)> for WrappedResourceInfo {
    fn from(value: (&Identity, Option<u64>, Option<google::protobuf::Timestamp>)) -> Self {
        Self(ResourceInfo {
            native_metadata: Default::default(),
            encoded_identity: value.0.to_string(),
            data_object_size: value.1,
            last_modified_timestamp: value.2,
        })
    }
}

impl From<(&Address, &Object)> for WrappedAddressInfo {
    fn from(value: (&Address, &Object)) -> Self {
        Self(AddressInfo {
            resource_address: value.0.url.to_string(),
            data_object_size: value.1.size.map(|len| len as u64),
            last_modified_timestamp: value.1.last_modified.map(|ts| google::protobuf::Timestamp {
                seconds: ts.secs(),
                nanos: ts.subsec_nanos() as i32,
            }),
        })
    }
}

/// Validate an S3 object key against the configured maximum key length.
///
/// The default limit of 1024 bytes matches the S3 spec. This is configurable via
/// `s3.max_key_bytes` to support S3-compatible backends with different limits.
///
/// S3 also recommends avoiding control characters and certain special characters in keys, but
/// these are not hard limits — objects with such characters can exist and be accessed. We do not
/// enforce soft recommendations to avoid breaking reads of existing objects.
///
/// @see <https://docs.aws.amazon.com/AmazonS3/latest/userguide/object-keys.html>
fn validate_object_key(key: &str, url: &Url, max_key_bytes: usize) -> Result<()> {
    if key.len() > max_key_bytes {
        return InvalidArgument {
            message: format!(
                "object key size {} bytes exceeds maximum of {max_key_bytes} bytes: {url}",
                key.len()
            ),
        }
        .fail();
    }

    Ok(())
}

/// AWS S3 URI encoding follows RFC 3986 and the AWS Signature V4 specification:
/// encode every byte except the RFC 3986 unreserved characters — A–Z, a–z, 0–9, -, _, ., and ~.
/// All other characters including every control character (0x00–0x1F and 0x7F), must be percent-encoded.
/// Encode a path component.
pub fn encode_path(value: &str) -> Cow<'_, str> {
    encode_url_path(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_at_max_byte_length() {
        let key = "a".repeat(1024);
        let url = Url::parse(&format!("https://bucket.s3.amazonaws.com/{key}")).unwrap();
        let address = Address::try_from(url).unwrap();
        assert!(address.key().is_ok());
    }

    #[test]
    fn test_key_exceeds_max_byte_length() {
        let key = "a".repeat(1025);
        let url = Url::parse(&format!("https://bucket.s3.amazonaws.com/{key}")).unwrap();
        let address = Address::try_from(url).unwrap();
        let result = address.key();
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("exceeds maximum of 1024 bytes"), "{err}");
    }

    #[test]
    fn test_key_multibyte_utf8_over_limit() {
        // Each '€' is 3 bytes in UTF-8 (%E2%82%AC). 342 chars × 3 bytes = 1026 bytes > 1024
        let key = "€".repeat(342);
        let encoded = encode_path(&key);
        let url = Url::parse(&format!("https://bucket.s3.amazonaws.com/{encoded}")).unwrap();
        let address = Address::try_from(url).unwrap();
        let result = address.key();
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("exceeds maximum of 1024 bytes"), "{err}");
    }

    #[test]
    fn test_key_valid_with_special_chars() {
        let url = Url::parse("https://bucket.s3.amazonaws.com/path/to/my-file_v2.0~backup").unwrap();
        let address = Address::try_from(url).unwrap();
        let key = address.key().unwrap();
        assert_eq!(key, "path/to/my-file_v2.0~backup");
    }

    #[test]
    fn test_encode_path_brackets() {
        let encoded = encode_path("[w1200 x h400]");
        assert_eq!(encoded, "%5Bw1200%20x%20h400%5D");
    }

    #[test]
    fn test_encode_path_unicode() {
        assert_eq!(encode_path("pekannöt"), "pekann%C3%B6t");
    }

    #[test]
    fn test_encode_path_round_trip() {
        let encoded = "9728-pekann%C3%B6t%20%5Bw1200%20x%20h400%5Dmm%20bump_noise.png";
        let decoded = percent_decode_str(encoded).decode_utf8().unwrap();
        assert_eq!(
            encode_path(&decoded),
            "9728-pekann%C3%B6t%20%5Bw1200%20x%20h400%5Dmm%20bump_noise.png"
        );
    }

    #[test]
    fn test_address_canonicalizes_loose_ascii_path() {
        let address = Address::parse("https://bucket.s3.amazonaws.com/!$%&'()*+,:;=@[]^|").unwrap();
        assert_eq!(
            address.to_string(),
            "https://bucket.s3.amazonaws.com/%21%24%25%26%27%28%29%2A%2B%2C%3A%3B%3D%40%5B%5D%5E%7C"
        );
    }

    #[test]
    fn test_address_canonicalization_is_idempotent() {
        let strict = "https://bucket.s3.amazonaws.com/%21%24%25%26%27%28%29%2A%2B%2C%3A%3B%3D%40%5B%5D%5E%7C";
        assert_eq!(Address::parse(strict).unwrap().to_string(), strict);
    }

    #[test]
    fn test_address_canonicalization_preserves_decoded_slash() {
        let address = Address::parse("https://bucket.s3.amazonaws.com/a%2Fb").unwrap();
        assert_eq!(address.to_string(), "https://bucket.s3.amazonaws.com/a/b");
        assert_eq!(address.key().unwrap(), "a/b");
    }

    #[test]
    fn test_address_rejects_dot_segments_created_by_encoded_slashes() {
        for value in [
            "https://bucket.s3.amazonaws.com/..%2Fkey",
            "https://bucket.s3.amazonaws.com/a%2F..%2Fb",
            "https://s3.amazonaws.com/bucket/..%2Fotherbucket%2Fkey",
        ] {
            let error = Address::parse(value).unwrap_err();
            assert!(error.to_string().contains("prohibited dot segment"), "{value}: {error}");
        }
    }

    #[test]
    fn test_versioned_address_preserves_only_non_identity_query_parameters() {
        let address = Address::parse(
            "https://bucket.s3.amazonaws.com/key?ETAG=%22stale%22&content-length=5&LAST-MODIFIED=yesterday&Version-Id=old&download=1",
        )
        .unwrap();

        assert_eq!(
            address.create_versioned_resource_address("v2"),
            "https://bucket.s3.amazonaws.com/key?download=1&version-id=v2"
        );
    }

    #[test]
    fn test_address_canonicalizes_path_style_key_only() {
        let mut address = Address::parse("https://s3.amazonaws.com/dns-safe.bucket/foo()+bar").unwrap();
        address.addressing_style = AddressingStyle::Path;
        assert_eq!(
            address.to_string(),
            "https://s3.amazonaws.com/dns-safe.bucket/foo%28%29%2Bbar"
        );
        assert_eq!(address.path_style_bucket_name().unwrap(), "dns-safe.bucket");
        assert_eq!(address.key().unwrap(), "foo()+bar");
    }

    #[test]
    fn test_address_canonicalization_round_trips_non_ascii() {
        let loose = Address::parse("https://bucket.s3.amazonaws.com/pekannöt/東京.txt").unwrap();
        let strict = "https://bucket.s3.amazonaws.com/pekann%C3%B6t/%E6%9D%B1%E4%BA%AC.txt";
        assert_eq!(loose.to_string(), strict);
        assert_eq!(Address::parse(strict).unwrap().to_string(), strict);
        assert_eq!(loose.key().unwrap(), "pekannöt/東京.txt");
    }

    #[test]
    fn test_address_canonicalization_rejects_invalid_utf8() {
        let error = Address::parse("https://bucket.s3.amazonaws.com/%FF").unwrap_err();
        assert!(error.to_string().contains("path canonicalization failed"), "{error}");
    }
}
