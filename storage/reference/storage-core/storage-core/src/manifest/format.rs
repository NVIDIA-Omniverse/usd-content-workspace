// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::collections::{HashMap, HashSet};

use percent_encoding::percent_decode_str;
use serde::{Deserialize, Serialize};
use storage_core_protos::google;
use storage_core_util::url_path::{canonicalize_url_path, encode_url_path};
use time::{format_description::well_known::Rfc2822, OffsetDateTime};
use unicode_normalization::UnicodeNormalization;
use url::Url;

use super::error::Error;

const CONTENT_LENGTH: &str = "content-length";
const LAST_MODIFIED: &str = "last-modified";
pub const WORKSPACE_SUFFIX: &str = ".wkspc";
pub const MANIFEST_FILENAME: &str = ".manifest";

/// Percent-encode a *decoded* workspace-relative path (a manifest key, which is stored decoded +
/// NFC) into a URL path so the resulting address is a valid, round-trippable URL.
///
/// Manifest keys may contain characters that are unsafe in a URL path — spaces, `#`, `%`, `?`,
/// etc. Concatenating them raw produced addresses that re-parsed to a *different* object (or failed
/// to parse) on a follow-up stat/read, and were inconsistent with the percent-encoded
/// backend-sourced siblings in the same listing. Internal `/` separators are preserved.
pub fn encode_relative_path(relative_path: &str) -> String {
    encode_url_path(relative_path).into_owned()
}

/// Normalize a workspace path into a stable manifest-state cache key.
///
/// `workspace_path` has canonical percent encoding, but retains the Unicode normalization form the
/// client sent. The manifest-state cache and its invalidation must agree on a single key, so
/// collapse Unicode (NFD→NFC) variants of the workspace segment to one form. Otherwise two
/// equivalent addresses split into separate cache entries and an `invalidate` keyed on one form
/// fails to evict the other — leaving a stale overlay until the TTL elapses. Percent-decoding here
/// also keeps the cache key compatible with entries created before workspace paths were
/// canonicalized at parse time.
pub fn manifest_cache_key(workspace_path: &str) -> String {
    match percent_decode_str(workspace_path).decode_utf8() {
        Ok(decoded) => decoded.nfc().collect(),
        Err(_) => workspace_path.nfc().collect(),
    }
}

/// Cheaply derive the *decoded* path of a backend address relative to its workspace.
///
/// Backend listing addresses share the request's constant `workspace_path` prefix and are
/// percent-encoded. Stripping that prefix and percent-decoding the remainder avoids a full
/// `Url::parse` (+ percent-decode + NFC) per entry on the listing hot path. Returns `None` — so the
/// caller can fall back to the URL parser — when the address doesn't sit directly under the
/// workspace prefix or carries a query/fragment (rare forms the parser handles correctly).
/// The returned remainder is decoded but *not* NFC-normalized; callers normalize as needed.
pub fn workspace_suffix_after_prefix(workspace_path: &str, resource_address: &str) -> Option<String> {
    let rest = resource_address.strip_prefix(workspace_path)?.strip_prefix('/')?;
    if rest.is_empty() || rest.contains('?') || rest.contains('#') {
        return None;
    }
    percent_decode_str(rest)
        .decode_utf8()
        .ok()
        .map(|decoded| decoded.into_owned())
}

// For now, Manifests are immutable, so we can use a simple HashMap.
// The key is the relative path of the object in the manifest, the value is either the minimal
// or full variant of the resource entry.
//
// Wrapped in a newtype so the cache layer can implement `CachedSize` /
// `CachedValue` on it; orphan rules disallow those impls on the bare
// `HashMap<...>` alias.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Manifest(pub HashMap<String, ManifestEntryValue>);

impl Manifest {
    pub fn new() -> Self {
        Self(HashMap::new())
    }

    pub fn into_inner(self) -> HashMap<String, ManifestEntryValue> {
        self.0
    }
}

impl std::ops::Deref for Manifest {
    type Target = HashMap<String, ManifestEntryValue>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for Manifest {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl FromIterator<(String, ManifestEntryValue)> for Manifest {
    fn from_iter<I: IntoIterator<Item = (String, ManifestEntryValue)>>(iter: I) -> Self {
        Self(HashMap::from_iter(iter))
    }
}

/// The value stored in the manifest for each file entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ManifestEntryValue {
    /// Minimal variant: just the resource identity string
    ResourceIdentity(String),
    /// Full variant: complete ResourceInfo with embedded metadata
    ResourceInfo(ManifestResourceInfo),
}

impl ManifestEntryValue {
    /// Returns the resource identity string for this entry.
    pub fn resource_identity(&self) -> &str {
        match self {
            ManifestEntryValue::ResourceIdentity(id) => id,
            ManifestEntryValue::ResourceInfo(info) => &info.resource_identity,
        }
    }

    /// Returns the entry's identity with its URL path in the canonical storage-core encoding.
    ///
    /// Manifest values stay byte-for-byte as they appeared in the manifest/cache. Canonicalizing
    /// at the egress boundary lets already-cached manifests pick up the canonical spelling without
    /// a cache schema migration.
    pub(crate) fn canonical_resource_identity(&self) -> Result<String, Error> {
        Ok(canonicalize_resource_identity(self.resource_identity())?.to_string())
    }
}

/// The expected format version for manifest files.
pub const MANIFEST_FORMAT_VERSION: &str = "1.0";

/// Top-level manifest file structure.
#[derive(Debug, Deserialize)]
pub struct ManifestFile {
    pub format_version: FormatVersion,
    pub storage_api_version: String,
    pub files: Vec<ManifestEntry>,
}

/// Format version object containing the version string.
#[derive(Debug, Deserialize)]
pub struct FormatVersion {
    pub version_string: String,
}

/// A file entry in the manifest as deserialized from JSON.
/// Contains a relative path and exactly one of resource_identity or resource_info.
#[derive(Debug, Deserialize)]
pub struct ManifestEntry {
    pub relative_path: String,
    /// Minimal variant: just the resource identity string
    #[serde(default)]
    resource_identity: Option<String>,
    /// Full variant: complete ResourceInfo with embedded metadata
    #[serde(default)]
    resource_info: Option<ManifestResourceInfo>,
}

/// ResourceInfo as embedded in manifest files (full variant).
/// Contains the resource identity plus optional metadata.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ManifestResourceInfo {
    pub resource_identity: String,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub modified_time: Option<String>,
}

impl ManifestEntry {
    /// Converts the entry to a ManifestEntryValue enum.
    /// Returns an error if neither resource_identity nor resource_info is present,
    /// or if both are present.
    pub fn into_value(self) -> Result<ManifestEntryValue, Error> {
        match (self.resource_identity, self.resource_info) {
            (Some(identity), None) => Ok(ManifestEntryValue::ResourceIdentity(identity)),
            (None, Some(info)) => Ok(ManifestEntryValue::ResourceInfo(info)),
            (Some(_), Some(_)) => Err(Error::invalid_manifest_entry(format!(
                "Entry '{}' has both resource_identity and resource_info",
                self.relative_path
            ))),
            (None, None) => Err(Error::invalid_manifest_entry(format!(
                "Entry '{}' has neither resource_identity nor resource_info",
                self.relative_path
            ))),
        }
    }
}

impl ManifestFile {
    /// Validates the manifest format version matches the expected version.
    pub fn validate_version(&self) -> Result<(), Error> {
        if self.format_version.version_string != MANIFEST_FORMAT_VERSION {
            return Err(Error::unsupported_format_version(
                MANIFEST_FORMAT_VERSION,
                self.format_version.version_string.clone(),
            ));
        }
        Ok(())
    }

    /// Convert the deserialized file into the in-memory [`Manifest`].
    ///
    /// Any rejected entry (bad version, duplicate `relative_path`, `.`/`..`, leading `/`, the
    /// reserved `.manifest` name, or a nested `.wkspc` segment) fails the *whole* manifest, which
    /// the caller caches as [`ManifestState::Invalid`](super::ManifestState::Invalid). This is the
    /// deliberate fail-closed contract (design §3.5/§3.6): a malformed manifest rejects every
    /// overlay op for the workspace — including passthrough files not named in the manifest — so a
    /// corrupt manifest can never silently downgrade a read-only workspace into a writable plain
    /// folder. Only the `.manifest` object itself stays accessible (the repair path), and fixing
    /// the file invalidates the cache and reactivates the overlay immediately.
    ///
    /// Duplicate `relative_path`s are rejected rather than last-wins-deduped (the pre-overlay
    /// behavior): manifests are machine-generated, so a duplicate signals a generator bug worth
    /// surfacing loudly rather than silently resolving to an arbitrary entry.
    pub fn into_manifest(self) -> Result<Manifest, Error> {
        self.validate_version()?;

        let mut seen = HashSet::new();
        self.files
            .into_iter()
            .map(|entry| {
                // NFC-normalize the key so direct lookups (`WorkspaceKey::parse` also NFC-normalizes
                // the requested object path) and listing dedup agree on Unicode form; dedup detection
                // then catches NFC/NFD variants of the same name.
                let relative_path = entry.relative_path.nfc().collect::<String>();
                validate_manifest_relative_path(&relative_path)?;

                if !seen.insert(relative_path.clone()) {
                    return Err(Error::invalid_manifest_entry(format!(
                        "Duplicate relative path '{}'",
                        relative_path
                    )));
                }

                let value = entry.into_value()?;
                Ok((relative_path, value))
            })
            .collect::<Result<Manifest, Error>>()
    }
}

#[derive(Debug, Clone)]
pub struct WorkspaceKey {
    pub workspace_path: String,
    pub manifest_path: String,
    pub object_path: Option<String>,
}

/// Parsed metadata extracted from a resource identity URL or manifest entry.
struct ParsedMetadata {
    resource_identity: String,
    resource_address: String,
    data_object_size: Option<u64>,
    last_modified_timestamp: Option<google::protobuf::Timestamp>,
}

impl WorkspaceKey {
    pub fn parse(url: impl AsRef<str>) -> Result<Option<Self>, Error> {
        let mut url = Url::parse(url.as_ref())?;
        canonicalize_url_path(&mut url).map_err(|e| {
            Error::invalid_argument(format!(
                "URL path canonicalization failed for workspace URL '{url}': {e}"
            ))
        })?;

        let path = url.path();

        let mut offset = usize::from(path.starts_with('/'));
        for segment in path.trim_start_matches('/').split('/') {
            let segment_start = offset;
            let segment_end = segment_start + segment.len();

            if segment.ends_with(WORKSPACE_SUFFIX) {
                let workspace_portion = &path[..segment_end];
                let workspace_url = url.join(workspace_portion)?;
                let workspace_path = workspace_url.to_string();
                let manifest_path = format!("{workspace_path}/{MANIFEST_FILENAME}");

                let object_path = if segment_end < path.len()
                    && path.as_bytes().get(segment_end) == Some(&b'/')
                    && segment_end + 1 < path.len()
                {
                    let object_portion = &path[segment_end + 1..];
                    let decoded = percent_decode_str(object_portion)
                        .decode_utf8()
                        .map_err(|e| Error::invalid_argument(format!("Failed to decode workspace object path: {e}")))?;
                    // NFC-normalize so a direct read/stat/write lookup against the manifest keys
                    // (also NFC-normalized in `into_manifest`) matches regardless of the client's
                    // Unicode form — consistent with the listing shadow-set dedup.
                    Some(decoded.nfc().collect::<String>())
                } else {
                    None
                };

                return Ok(Some(Self {
                    workspace_path,
                    manifest_path,
                    object_path,
                }));
            }

            offset = segment_end + 1;
        }

        Ok(None)
    }

    pub fn is_manifest_file(&self) -> bool {
        self.object_path.as_deref() == Some(MANIFEST_FILENAME)
    }
}

fn validate_manifest_relative_path(relative_path: &str) -> Result<(), Error> {
    if relative_path.is_empty() {
        return Err(Error::invalid_manifest_entry("relative_path must not be empty"));
    }

    if relative_path.starts_with('/') {
        return Err(Error::invalid_manifest_entry(format!(
            "relative_path '{}' must not start with '/'",
            relative_path
        )));
    }

    let mut segments = relative_path.split('/');
    let first_segment = segments
        .next()
        .ok_or_else(|| Error::invalid_manifest_entry("relative_path must not be empty"))?;

    if first_segment == MANIFEST_FILENAME {
        return Err(Error::invalid_manifest_entry(format!(
            "relative_path '{}' uses reserved manifest file name",
            relative_path
        )));
    }

    for segment in std::iter::once(first_segment).chain(segments) {
        if segment.is_empty() {
            return Err(Error::invalid_manifest_entry(format!(
                "relative_path '{}' contains an empty path segment",
                relative_path
            )));
        }
        if segment == "." || segment == ".." {
            return Err(Error::invalid_manifest_entry(format!(
                "relative_path '{}' contains a reserved path segment",
                relative_path
            )));
        }
        if segment.ends_with(WORKSPACE_SUFFIX) {
            return Err(Error::invalid_manifest_entry(format!(
                "relative_path '{}' contains a nested workspace segment",
                relative_path
            )));
        }
    }

    Ok(())
}

/// Parses a timestamp string from URL query parameters.
/// Supports ISO 8601 (used by S3) and RFC 2822 (used by Azure).
fn parse_timestamp_from_url_query(timestamp_str: &str) -> Result<google::protobuf::Timestamp, Error> {
    let datetime = OffsetDateTime::parse(timestamp_str, &time::format_description::well_known::Iso8601::DEFAULT)
        .or_else(|_| OffsetDateTime::parse(timestamp_str, &Rfc2822))
        .map_err(|e| {
            Error::invalid_resource_identity(format!(
                "invalid timestamp '{}': {}. Expected ISO 8601 or RFC 2822 format",
                timestamp_str, e
            ))
        })?;

    Ok(google::protobuf::Timestamp {
        seconds: datetime.unix_timestamp(),
        nanos: datetime.nanosecond() as i32,
    })
}

/// Parses a timestamp string in RFC 3339 format (used by protobuf JSON serialization).
fn parse_rfc3339_timestamp(timestamp_str: &str) -> Result<google::protobuf::Timestamp, Error> {
    let datetime = OffsetDateTime::parse(timestamp_str, &time::format_description::well_known::Rfc3339)
        .map_err(|e| Error::invalid_manifest_entry(format!("invalid RFC 3339 timestamp '{}': {}", timestamp_str, e)))?;

    Ok(google::protobuf::Timestamp {
        seconds: datetime.unix_timestamp(),
        nanos: datetime.nanosecond() as i32,
    })
}

/// Parses metadata from a resource identity URL (minimal variant).
/// Extracts content-length and last-modified from query parameters.
fn parse_metadata_from_resource_identity(resource_identity: &str) -> Result<ParsedMetadata, Error> {
    let url = canonicalize_resource_identity(resource_identity)?;

    let mut content_length = None;
    let mut last_modified = None;

    // parse out the metadata from the encoded identity query params
    for query_pair in url.query_pairs() {
        match query_pair.0.to_lowercase().as_str() {
            CONTENT_LENGTH => {
                content_length = Some(query_pair.1.parse::<u64>().map_err(|e| {
                    Error::invalid_resource_identity(format!(
                        "invalid content-length ({}) for '{url}': {e}",
                        query_pair.1
                    ))
                })?);
            },
            LAST_MODIFIED => {
                last_modified = Some(parse_timestamp_from_url_query(query_pair.1.as_ref())?);
            },
            _ => {},
        }
    }

    let resource_address = resource_address_from_identity(&url);

    Ok(ParsedMetadata {
        resource_identity: url.to_string(),
        resource_address,
        data_object_size: content_length,
        last_modified_timestamp: last_modified,
    })
}

/// Parses metadata from a ManifestResourceInfo (full variant).
fn parse_metadata_from_resource_info(info: &ManifestResourceInfo) -> Result<ParsedMetadata, Error> {
    let url = canonicalize_resource_identity(&info.resource_identity)?;

    let resource_address = resource_address_from_identity(&url);

    // Parse the modified_time if present (RFC 3339 format from protobuf JSON serialization)
    let last_modified_timestamp = info
        .modified_time
        .as_ref()
        .map(|ts| parse_rfc3339_timestamp(ts))
        .transpose()?;

    Ok(ParsedMetadata {
        resource_identity: url.to_string(),
        resource_address,
        data_object_size: info.size,
        last_modified_timestamp,
    })
}

fn canonicalize_resource_identity(resource_identity: &str) -> Result<Url, Error> {
    let mut url = Url::parse(resource_identity)?;
    canonicalize_url_path(&mut url).map_err(|e| {
        Error::invalid_resource_identity(format!(
            "URL path canonicalization failed for resource identity '{url}': {e}"
        ))
    })?;
    Ok(url)
}

fn resource_address_from_identity(identity: &Url) -> String {
    let mut address = identity.clone();
    address.set_query(None);
    address.set_fragment(None);
    address.to_string()
}

impl ManifestEntryValue {
    /// Parses the manifest entry value into metadata components.
    fn parse_metadata(&self) -> Result<ParsedMetadata, Error> {
        match self {
            ManifestEntryValue::ResourceIdentity(identity) => parse_metadata_from_resource_identity(identity),
            ManifestEntryValue::ResourceInfo(info) => parse_metadata_from_resource_info(info),
        }
    }
}

pub mod v1alpha {
    use super::{Error, ManifestEntryValue};
    use crate::traits::{
        fileobject::v1alpha::{AddressInfo, Metadata, ResourceIdentity, ResourceInfo},
        versioning::v1alpha::VersionInfo,
    };

    impl TryFrom<&ManifestEntryValue> for AddressInfo {
        type Error = Error;

        fn try_from(value: &ManifestEntryValue) -> Result<Self, Self::Error> {
            let parsed = value.parse_metadata()?;
            Ok(Self {
                resource_address: parsed.resource_address,
                metadata: Some(Metadata {
                    native_metadata: Default::default(),
                    data_object_size: parsed.data_object_size,
                    last_modified_timestamp: parsed.last_modified_timestamp,
                }),
            })
        }
    }

    impl TryFrom<&ManifestEntryValue> for ResourceInfo {
        type Error = Error;

        fn try_from(value: &ManifestEntryValue) -> Result<Self, Self::Error> {
            let parsed = value.parse_metadata()?;
            Ok(Self {
                resource_identity: Some(ResourceIdentity {
                    encoded_identity: parsed.resource_identity,
                }),
                metadata: Some(Metadata {
                    native_metadata: Default::default(),
                    data_object_size: parsed.data_object_size,
                    last_modified_timestamp: parsed.last_modified_timestamp,
                }),
            })
        }
    }

    impl TryFrom<&ManifestEntryValue> for VersionInfo {
        type Error = Error;

        fn try_from(value: &ManifestEntryValue) -> Result<Self, Self::Error> {
            Ok(Self {
                resource_info: Some(ResourceInfo::try_from(value)?),
                sorting_key: None,
                // We can't make a versioned resource address because we don't have a version id to use, so just leave it as None.
                resource_address: None,
            })
        }
    }
}

pub mod v1beta {
    use super::{Error, ManifestEntryValue};
    use crate::traits::{
        fileobject::v1beta::{AddressInfo, Metadata, ResourceIdentity, ResourceInfo},
        versioning::v1beta::VersionInfo,
    };

    impl TryFrom<&ManifestEntryValue> for AddressInfo {
        type Error = Error;

        fn try_from(value: &ManifestEntryValue) -> Result<Self, Self::Error> {
            let parsed = value.parse_metadata()?;
            Ok(Self {
                resource_address: parsed.resource_address,
                metadata: Some(Metadata {
                    data_object_size: parsed.data_object_size,
                    last_modified_timestamp: parsed.last_modified_timestamp,
                }),
            })
        }
    }

    impl TryFrom<&ManifestEntryValue> for ResourceInfo {
        type Error = Error;

        fn try_from(value: &ManifestEntryValue) -> Result<Self, Self::Error> {
            let parsed = value.parse_metadata()?;
            Ok(Self {
                resource_identity: Some(ResourceIdentity {
                    encoded_identity: parsed.resource_identity,
                }),
                metadata: Some(Metadata {
                    data_object_size: parsed.data_object_size,
                    last_modified_timestamp: parsed.last_modified_timestamp,
                }),
            })
        }
    }

    impl TryFrom<&ManifestEntryValue> for VersionInfo {
        type Error = Error;

        fn try_from(value: &ManifestEntryValue) -> Result<Self, Self::Error> {
            Ok(Self {
                resource_info: Some(ResourceInfo::try_from(value)?),
                sorting_key: None,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOOSE_DIVERGENT_ASCII: &str = "!$%&'()*+,:;=@[]^|";
    const STRICT_DIVERGENT_ASCII: &str = "%21%24%25%26%27%28%29%2A%2B%2C%3A%3B%3D%40%5B%5D%5E%7C";

    fn manifest_file(paths: &[&str]) -> ManifestFile {
        ManifestFile {
            format_version: FormatVersion {
                version_string: MANIFEST_FORMAT_VERSION.to_string(),
            },
            storage_api_version: "v1beta".to_string(),
            files: paths
                .iter()
                .map(|path| ManifestEntry {
                    relative_path: (*path).to_string(),
                    resource_identity: Some("s3://bucket/object".to_string()),
                    resource_info: None,
                })
                .collect(),
        }
    }

    #[test]
    fn workspace_key_parses_first_workspace_segment() {
        let key = WorkspaceKey::parse("s3://bucket/a/root.wkspc/nested.wkspc/file.usd")
            .expect("parse")
            .expect("workspace key");

        assert_eq!(key.workspace_path, "s3://bucket/a/root.wkspc");
        assert_eq!(key.manifest_path, "s3://bucket/a/root.wkspc/.manifest");
        assert_eq!(key.object_path.as_deref(), Some("nested.wkspc/file.usd"));
    }

    #[test]
    fn workspace_key_decodes_relative_path() {
        let key = WorkspaceKey::parse("s3://bucket/a/root.wkspc/folder%20name/file.usd")
            .expect("parse")
            .expect("workspace key");

        assert_eq!(key.object_path.as_deref(), Some("folder name/file.usd"));
    }

    #[test]
    fn workspace_key_canonicalizes_all_divergent_ascii_and_is_idempotent() {
        let loose = format!("https://host/{LOOSE_DIVERGENT_ASCII}/{LOOSE_DIVERGENT_ASCII}.wkspc/file.usd");
        let strict_workspace = format!("https://host/{STRICT_DIVERGENT_ASCII}/{STRICT_DIVERGENT_ASCII}.wkspc");

        let loose_key = WorkspaceKey::parse(&loose)
            .expect("parse loose")
            .expect("workspace key");
        assert_eq!(loose_key.workspace_path, strict_workspace);
        assert_eq!(loose_key.manifest_path, format!("{strict_workspace}/.manifest"));
        assert_eq!(loose_key.object_path.as_deref(), Some("file.usd"));

        let strict = format!("{strict_workspace}/file.usd");
        let strict_key = WorkspaceKey::parse(&strict)
            .expect("parse strict")
            .expect("workspace key");
        assert_eq!(strict_key.workspace_path, loose_key.workspace_path);
        assert_eq!(strict_key.manifest_path, loose_key.manifest_path);
    }

    #[test]
    fn workspace_key_collapses_encoded_slash_deliberately() {
        let key = WorkspaceKey::parse("https://host/a%2Fb.wkspc/file.usd")
            .expect("parse")
            .expect("workspace key");

        assert_eq!(key.workspace_path, "https://host/a/b.wkspc");
        assert_eq!(key.manifest_path, "https://host/a/b.wkspc/.manifest");
        assert_eq!(key.object_path.as_deref(), Some("file.usd"));
    }

    #[test]
    fn workspace_key_detects_encoded_workspace_boundary_aliases() {
        for address in [
            "https://host/root%2Ewkspc/file.usd",
            "https://host/root.wkspc%2Ffile.usd",
        ] {
            let key = WorkspaceKey::parse(address).expect("parse").expect("workspace key");

            assert_eq!(key.workspace_path, "https://host/root.wkspc", "{address}");
            assert_eq!(key.object_path.as_deref(), Some("file.usd"), "{address}");
        }
    }

    #[test]
    fn workspace_key_canonicalizes_non_ascii_and_rejects_invalid_utf8() {
        let key = WorkspaceKey::parse("https://host/pekannöt/東京.wkspc/file.usd")
            .expect("parse")
            .expect("workspace key");
        assert_eq!(
            key.workspace_path,
            "https://host/pekann%C3%B6t/%E6%9D%B1%E4%BA%AC.wkspc"
        );

        let error = WorkspaceKey::parse("https://host/%FF.wkspc/file.usd").expect_err("invalid UTF-8");
        assert!(error.to_string().contains("path canonicalization failed"));

        let error = WorkspaceKey::parse("https://host/root.wkspc/%FF").expect_err("invalid object UTF-8");
        assert!(error.to_string().contains("path canonicalization failed"));
    }

    #[test]
    fn workspace_key_does_not_match_substrings() {
        let key = WorkspaceKey::parse("s3://bucket/a/root.wkspced/file.usd").expect("parse");
        assert!(key.is_none());
    }

    #[test]
    fn workspace_key_folder_with_trailing_slash_has_no_object_path() {
        // The bare workspace folder, with and without a trailing slash, has no relative object path.
        for url in ["s3://bucket/a/root.wkspc", "s3://bucket/a/root.wkspc/"] {
            let key = WorkspaceKey::parse(url).expect("parse").expect("workspace key");
            assert_eq!(key.workspace_path, "s3://bucket/a/root.wkspc");
            assert_eq!(key.object_path, None, "{url} should have no object path");
        }
    }

    #[test]
    fn workspace_key_normalizes_object_path_to_nfc() {
        // Request uses precomposed é (U+00E9); parse decodes + NFC-normalizes so direct lookups
        // match NFC-normalized manifest keys.
        let key = WorkspaceKey::parse("s3://bucket/a/root.wkspc/caf\u{e9}.usd")
            .expect("parse")
            .expect("workspace key");
        assert_eq!(key.object_path.as_deref(), Some("caf\u{e9}.usd"));
    }

    #[test]
    fn workspace_key_identifies_manifest_file() {
        let key = WorkspaceKey::parse("s3://bucket/a/root.wkspc/.manifest")
            .expect("parse")
            .expect("workspace key");

        assert!(key.is_manifest_file());
    }

    #[test]
    fn manifest_validation_rejects_reserved_manifest_name() {
        let err = manifest_file(&[".manifest/child"])
            .into_manifest()
            .expect_err("reserved name should be invalid");

        assert!(err.to_string().contains("reserved manifest file name"));
    }

    #[test]
    fn manifest_validation_rejects_nested_workspace_segment() {
        let err = manifest_file(&["packages/nested.wkspc/file.usd"])
            .into_manifest()
            .expect_err("nested workspace should be invalid");

        assert!(err.to_string().contains("nested workspace segment"));
    }

    #[test]
    fn encoded_relative_path_round_trips_through_workspace_key() {
        // Manifest keys are stored decoded; building a listing address must percent-encode them so
        // the address re-parses to the *same* object on a follow-up stat/read. Spaces survive even
        // raw (Url leniency), but '#', '%', and '?' break unless encoded.
        let workspace = "s3://bucket/a/root.wkspc";
        for relative_path in [
            "my docs/from_manifest.txt",
            "caf\u{e9}.txt",
            "a#b.usd",
            "100%done.usd",
            "what?.usd",
            "folder/jellyfish - hevc.mkv",
        ] {
            let address = format!("{workspace}/{}", encode_relative_path(relative_path));
            let key = WorkspaceKey::parse(&address)
                .unwrap_or_else(|e| panic!("parse {address}: {e}"))
                .unwrap_or_else(|| panic!("no workspace key for {address}"));
            assert_eq!(
                key.object_path.as_deref(),
                Some(relative_path),
                "address {address} did not round-trip"
            );
        }
    }

    #[test]
    fn manifest_cache_key_collapses_encoding_and_unicode_variants() {
        // NFC vs NFD of the same accented .wkspc segment, and a percent-encoded equivalent, must
        // all map to one cache key so invalidation reaches every form.
        let nfc = manifest_cache_key("s3://bucket/r\u{e9}sum\u{e9}.wkspc");
        let nfd = manifest_cache_key("s3://bucket/re\u{301}sume\u{301}.wkspc");
        let encoded = manifest_cache_key("s3://bucket/r%C3%A9sum%C3%A9.wkspc");
        assert_eq!(nfc, nfd);
        assert_eq!(nfc, encoded);
    }

    #[test]
    fn manifest_source_identities_are_canonical_for_minimal_and_full_entries() {
        let loose_identity =
            format!("https://host/objects/{LOOSE_DIVERGENT_ASCII}?etag=value&content-length=7#fragment");
        let strict_identity =
            format!("https://host/objects/{STRICT_DIVERGENT_ASCII}?etag=value&content-length=7#fragment");
        let strict_address = format!("https://host/objects/{STRICT_DIVERGENT_ASCII}");

        let values = [
            ManifestEntryValue::ResourceIdentity(loose_identity.clone()),
            ManifestEntryValue::ResourceInfo(ManifestResourceInfo {
                resource_identity: loose_identity,
                size: Some(7),
                modified_time: None,
            }),
            ManifestEntryValue::ResourceIdentity(strict_identity.clone()),
        ];

        for value in values {
            let parsed = value.parse_metadata().expect("parse metadata");
            assert_eq!(parsed.resource_identity, strict_identity);
            assert_eq!(parsed.resource_address, strict_address);
            assert_eq!(
                value.canonical_resource_identity().expect("canonical identity"),
                strict_identity
            );
        }
    }

    #[test]
    fn manifest_source_identity_canonicalization_handles_slash_unicode_and_invalid_utf8() {
        let encoded_slash = ManifestEntryValue::ResourceIdentity("https://host/a%2Fb?etag=value".to_string());
        assert_eq!(
            encoded_slash.canonical_resource_identity().expect("encoded slash"),
            "https://host/a/b?etag=value"
        );

        let unicode = ManifestEntryValue::ResourceIdentity("https://host/pekannöt/東京.txt".to_string());
        assert_eq!(
            unicode.canonical_resource_identity().expect("unicode"),
            "https://host/pekann%C3%B6t/%E6%9D%B1%E4%BA%AC.txt"
        );

        let invalid = ManifestEntryValue::ResourceIdentity("https://host/%FF".to_string());
        let error = invalid.canonical_resource_identity().expect_err("invalid UTF-8");
        assert!(error.to_string().contains("path canonicalization failed"));
    }

    #[test]
    fn manifest_validation_rejects_duplicate_paths() {
        let err = manifest_file(&["file.usd", "file.usd"])
            .into_manifest()
            .expect_err("duplicates should be invalid");

        assert!(err.to_string().contains("Duplicate relative path"));
    }
}
