// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{
    borrow::Cow,
    fmt::{self, Debug},
    hash::Hash,
    ops::Deref,
};

use azure_storage_blobs::blob::{
    operations::{CopyBlobResponse, PutBlockBlobResponse, PutBlockListResponse},
    Blob,
};
use percent_encoding::percent_decode_str;
use storage_core_protos::google;
use storage_core_util::api_types::{AddressInfo, ResourceInfo};
use storage_core_util::url_path::{canonicalize_url_path, encode_url_path};
use url::{Position, Url};

use super::error::Error;

pub static DELIMITER: &str = "/";
static BLOB_URL: &str = "https://myaccount.blob.core.windows.net/mycontainer/myblob";

pub static FOLDER_PLACEHOLDER: &str = ".folder";
pub const CONTENT_LENGTH: &str = "content-length";
pub const ETAG: &str = "etag";
pub const LAST_MODIFIED: &str = "last-modified";
pub const VERSION_ID: &str = "version-id";
// CONTENT_LENGTH and LAST_MODIFIED are intentionally retained in IDENTITY_PARAMS
// even though current Identity instances no longer carry these fields. The
// create_url helper uses this list to STRIP non-identity params from the URL
// before re-appending its own; keeping these names here means any legacy
// identity URL (produced before the etag-only identity change) round-trips cleanly —
// the now-redundant params get silently dropped on re-serialization.
const IDENTITY_PARAMS: &[&str] = &[CONTENT_LENGTH, ETAG, LAST_MODIFIED, VERSION_ID];

fn is_identity_param(name: &str) -> bool {
    IDENTITY_PARAMS.iter().any(|param| name.eq_ignore_ascii_case(param))
}

#[derive(Debug, Clone, Eq)]
pub struct Address {
    pub resource_address: Url,
    // Store positions as usize for arithmetic and comparison
    account_end: usize,
    container_end: usize,
    key_start: usize,
}

impl Address {
    /// Creates an AzureResourceAddress by parsing the URL for a container and blob. An error is returned
    /// if the URL doesn't contain a valid container and blob
    pub fn parse_blob(mut url: Url) -> Result<Self, Error> {
        let hostname = Self::parse_hostname(&url)?;
        let account_end = hostname.find('.').unwrap_or(hostname.len());
        canonicalize_azure_path(&mut url)?;

        // Derive every path offset after canonicalization. `Url::set_path` may rewrite the path,
        // so retaining offsets captured before it would make later slicing unsafe.
        let container_end = match url.path().get(1..).and_then(|path| path.find('/').map(|i| i + 1)) {
            Some(i) => i,
            None => {
                return Err(Error::invalid_argument(format!(
                    "Invalid Azure URL {url}: Missing container name. URLs should be in the form {BLOB_URL}"
                )))
            },
        };
        container_from_path(url.path(), container_end, &url)?;

        let key_start = container_end + 1;
        if key_start >= url.path().len() {
            return Err(Error::invalid_argument(format!(
                "Invalid Azure URL {url}: Missing blob name. URLs should be in the form {BLOB_URL}"
            )));
        }

        let blob_name = percent_decode_str(&url.path()[key_start..])
            .decode_utf8()
            .map_err(|e| Error::invalid_argument(format!("percent decode failed for '{url}': {e}")))?;
        validate_blob_name(&blob_name, &url)?;

        Ok(Self {
            resource_address: url,
            account_end,
            container_end,
            key_start,
        })
    }

    /// Creates an AzureResourceAddress by parsing the URL for a container and search prefix. An error is returned
    /// if the URL doesn't contain a valid container. If the url doesn't contain a prefix, then an empty string
    /// will be stored as the prefix
    pub fn parse_prefix(mut url: Url) -> Result<Self, Error> {
        let hostname = Self::parse_hostname(&url)?;
        let account_end = hostname.find('.').unwrap_or(hostname.len());
        canonicalize_azure_path(&mut url)?;

        // Derive every path offset after canonicalization; see `parse_blob`.
        let container_end = url
            .path()
            .get(1..)
            .and_then(|path| path.find('/').map(|i| i + 1))
            .unwrap_or(url.path().len());
        container_from_path(url.path(), container_end, &url)?;

        let key_start = container_end + 1;

        Ok(Self {
            resource_address: url,
            account_end,
            container_end,
            key_start,
        })
    }

    pub fn parse_hostname(url: &Url) -> Result<&str, Error> {
        match url.host() {
            Some(url::Host::Domain(hostname)) => Ok(hostname),
            Some(url::Host::Ipv4(_)) => Err(Error::invalid_argument("Invalid host (ipv4), must be a valid FQDN")),
            Some(url::Host::Ipv6(_)) => Err(Error::invalid_argument("Invalid host (ipv6), must be a valid FQDN")),
            None => Err(Error::invalid_argument("No host specified")),
        }
    }

    /// Parse the account from a hostname.
    ///
    /// The account is the first part of the hostname before the first dot.
    ///
    /// The hostname must be in the format `account.blob.core.windows.net`.
    /// Parse the account from a URL, combining hostname extraction and account parsing.
    pub fn account_from_url(url: &Url) -> Result<&str, Error> {
        let hostname = Self::parse_hostname(url)?;
        Self::parse_account(hostname)
    }

    pub fn parse_account(hostname: &str) -> Result<&str, Error> {
        let parts: Vec<&str> = hostname.split('.').collect();

        if parts.len() != 5 || parts[1] != "blob" || parts[2] != "core" || parts[3] != "windows" || parts[4] != "net" {
            return Err(Error::invalid_argument(format!(
                "Invalid Azure hostname format: '{hostname}'. Expected format: account.blob.core.windows.net"
            )));
        }

        // The account must be non-empty
        if parts[0].is_empty() {
            return Err(Error::invalid_argument(format!(
                "Invalid Azure hostname format: '{hostname}'. Expected format: account.blob.core.windows.net"
            )));
        }

        Ok(parts[0])
    }

    pub fn account(&self) -> Result<&str, Error> {
        match self.resource_address.host_str() {
            Some(hostname) => hostname
                .get(..self.account_end)
                .ok_or_else(|| Error::internal("Azure account offset exceeds hostname length")),
            None => Err(Error::invalid_argument("Invalid Azure URL: Missing hostname")),
        }
    }

    pub fn hostname(&self) -> Result<&str, Error> {
        match self.resource_address.host_str() {
            Some(hostname) => Ok(hostname),
            None => Err(Error::invalid_argument("Invalid Azure URL: Missing hostname")),
        }
    }

    pub fn container(&self) -> Result<&str, Error> {
        container_from_path(self.resource_address.path(), self.container_end, &self.resource_address)
    }

    pub fn blob(&self) -> Cow<'_, str> {
        match self.resource_address.path().get(self.key_start..) {
            Some(blob) => percent_decode_str(blob).decode_utf8_lossy(),
            None => Cow::Borrowed(""),
        }
    }

    // prefix should be url decoded, as the azure sdk will reencode it as part of the query string
    // and if any existing encoded characters are present, it will double encode them
    pub fn prefix(&self) -> Cow<'_, str> {
        match self.resource_address.path().get(self.key_start..) {
            Some(prefix) => percent_decode_str(prefix).decode_utf8_lossy(),
            None => Cow::Borrowed(""),
        }
    }

    pub fn is_folder_placeholder(&self) -> bool {
        self.resource_address.path().ends_with(FOLDER_PLACEHOLDER)
    }

    pub fn is_versioned(&self) -> bool {
        self.version_id().is_some()
    }

    pub fn version_id(&self) -> Option<Cow<'_, str>> {
        self.resource_address
            .query_pairs()
            .find(|(k, _)| k == VERSION_ID)
            .map(|(_, v)| v)
    }

    pub fn create_versioned_resource_address(&self, version_id: &str) -> String {
        let mut url = self.resource_address.clone();
        let non_identity_params: Vec<_> = url
            .query_pairs()
            .filter(|(name, _)| !is_identity_param(name.as_ref()))
            .map(|(name, value)| (name.into_owned(), value.into_owned()))
            .collect();
        url.set_query(None);
        if !non_identity_params.is_empty() {
            url.query_pairs_mut().extend_pairs(&non_identity_params);
        }
        url.query_pairs_mut().append_pair(VERSION_ID, version_id);
        url.to_string()
    }
}

// Hash the address up to the end of the path. There shouldn't be a query or fragment of the resource address,
// but we make sure we don't hash them if there are.
impl Hash for Address {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.resource_address[..Position::AfterPath].hash(state);
    }
}

// Similar to Hash, we only compare the url up to the end of the path. We do not include the AddressingStyle
// because actually computing the addressing style requires looking up the resource address in the client map,
// which not all functions do, and it doesn't actually matter for the purposes of equality.
impl PartialEq for Address {
    fn eq(&self, other: &Self) -> bool {
        self.resource_address[..Position::AfterPath] == other.resource_address[..Position::AfterPath]
    }
}

impl From<Address> for Url {
    fn from(value: Address) -> Self {
        value.resource_address
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.resource_address, f)
    }
}

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct Identity {
    pub resource_address: Address,
    pub etag: Option<String>,
    pub version_id: Option<String>,
}

impl Identity {
    pub fn parse(resource_identity: impl AsRef<str>) -> Result<Self, Error> {
        let mut url = Url::parse(resource_identity.as_ref())?;

        let mut etag = None;
        let mut version_id = None;
        let mut non_identity_params = Vec::new();

        // Parse identity metadata from query params. CONTENT_LENGTH and
        // LAST_MODIFIED were removed from Identity when it became etag-only;
        // legacy identity URLs produced before that change may still carry
        // them — we silently ignore those params so older clients continue
        // to work, and `create_url` strips them on re-serialization.
        for query_pair in url.query_pairs() {
            match query_pair.0.to_lowercase().as_str() {
                ETAG => {
                    etag = Some(query_pair.1.to_string());
                },
                VERSION_ID => version_id = Some(query_pair.1.to_string()),
                CONTENT_LENGTH | LAST_MODIFIED => {
                    // Legacy params from pre-Phase-3 identities — ignore.
                },
                _ => non_identity_params.push((query_pair.0.into_owned(), query_pair.1.into_owned())),
            }
        }

        let etag = match etag {
            Some(etag) if !etag.is_empty() => Some(etag),
            _ => {
                return Err(Error::invalid_argument(format!("etag is missing for '{url}'")));
            },
        };

        url.set_query(None);
        if !non_identity_params.is_empty() {
            url.query_pairs_mut().extend_pairs(&non_identity_params);
        }
        Ok(Self {
            resource_address: Address::parse_blob(url)?,
            etag,
            version_id,
        })
    }

    pub fn resource_address(&self) -> &Address {
        &self.resource_address
    }

    pub fn container(&self) -> Result<&str, Error> {
        self.resource_address.container()
    }

    pub fn blob(&self) -> Cow<'_, str> {
        self.resource_address.blob()
    }

    /// Create a resource identity url from components.
    ///
    /// Identity consists of (etag, version-id) only —
    /// content-length and last-modified are metadata that live on `ResourceInfo`
    /// as separate fields, not inside the identity URL. Legacy identity URLs
    /// carrying those params are handled by the IDENTITY_PARAMS filter below:
    /// they get stripped during URL rebuild so re-serialization is idempotent.
    pub fn create_url(resource_address: Url, etag: Option<String>, version_id: Option<String>) -> String {
        let mut url = resource_address.clone();
        url.set_query(None);

        // We want to keep all the non-identity params, but we need to remove the identity
        // params before we add our own to avoid any duplicates.
        let mut non_identity_params = resource_address
            .query_pairs()
            .filter(|(name, _)| !is_identity_param(name.as_ref()))
            .peekable();

        // only extend the params if there are other query parameters
        // if you do a clear().extend_pairs() and identity_params is empty, it will result in
        // query params being Some("") vs None, which means all generated urls will have a trailing '?'
        if non_identity_params.peek().is_some() {
            url.query_pairs_mut().extend_pairs(non_identity_params);
        }

        // add the identity params
        if let Some(e) = etag {
            url.query_pairs_mut().append_pair(ETAG, &e);
        }
        if let Some(v) = version_id {
            url.query_pairs_mut().append_pair(VERSION_ID, &v);
        }
        url.to_string()
    }
}

pub struct WrappedResourceInfo(pub ResourceInfo);

pub struct WrappedAddressInfo(pub AddressInfo);

impl From<(Url, Blob, Option<String>)> for WrappedResourceInfo {
    fn from(value: (Url, Blob, Option<String>)) -> Self {
        Self::from((value.0, &value.1, &value.2))
    }
}

impl From<(Url, &Blob, &Option<String>)> for WrappedResourceInfo {
    fn from(value: (Url, &Blob, &Option<String>)) -> Self {
        Self(ResourceInfo {
            // Blob metadata is already present on the GetProperties/stat response — no extra call.
            native_metadata: value
                .1
                .metadata
                .clone()
                .map(storage_core_util::api_types::native_metadata_from_strings)
                .unwrap_or_default(),
            encoded_identity: Identity::create_url(
                value.0,
                Some(quote_etag_if_needed(value.1.properties.etag.as_ref())),
                value.2.clone(),
            ),
            // Size and last-modified live on ResourceInfo as separate fields — they
            // used to be baked into encoded_identity.
            data_object_size: Some(value.1.properties.content_length),
            last_modified_timestamp: Some(google::protobuf::Timestamp {
                seconds: value.1.properties.last_modified.unix_timestamp(),
                nanos: value.1.properties.last_modified.nanosecond() as i32,
            }),
        })
    }
}

impl From<(Url, PutBlockBlobResponse, u64)> for WrappedResourceInfo {
    fn from(value: (Url, PutBlockBlobResponse, u64)) -> Self {
        Self(ResourceInfo {
            native_metadata: Default::default(),
            encoded_identity: Identity::create_url(value.0, Some(value.1.etag.clone()), value.1.version_id),
            data_object_size: Some(value.2),
            last_modified_timestamp: Some(google::protobuf::Timestamp {
                seconds: value.1.last_modified.unix_timestamp(),
                nanos: value.1.last_modified.nanosecond() as i32,
            }),
        })
    }
}

impl From<(Url, CopyBlobResponse, Option<u64>)> for WrappedResourceInfo {
    fn from(value: (Url, CopyBlobResponse, Option<u64>)) -> Self {
        Self(ResourceInfo {
            native_metadata: Default::default(),
            encoded_identity: Identity::create_url(value.0, Some(value.1.etag), value.1.version_id),
            data_object_size: value.2,
            last_modified_timestamp: Some(google::protobuf::Timestamp {
                seconds: value.1.last_modified.unix_timestamp(),
                nanos: value.1.last_modified.nanosecond() as i32,
            }),
        })
    }
}

impl From<(Url, PutBlockListResponse)> for WrappedResourceInfo {
    fn from(value: (Url, PutBlockListResponse)) -> Self {
        Self(ResourceInfo {
            native_metadata: Default::default(),
            encoded_identity: Identity::create_url(value.0, Some(value.1.etag), value.1.version_id),
            data_object_size: None,
            last_modified_timestamp: Some(google::protobuf::Timestamp {
                seconds: value.1.last_modified.unix_timestamp(),
                nanos: value.1.last_modified.nanosecond() as i32,
            }),
        })
    }
}

impl From<(Url, &Blob)> for WrappedAddressInfo {
    fn from(value: (Url, &Blob)) -> Self {
        Self(AddressInfo {
            resource_address: value.0.to_string(),
            data_object_size: Some(value.1.properties.content_length),
            last_modified_timestamp: Some(google::protobuf::Timestamp {
                seconds: value.1.properties.last_modified.unix_timestamp(),
                nanos: value.1.properties.last_modified.nanosecond() as i32,
            }),
        })
    }
}

/// The address of a storage container.
pub struct ContainerAddress(pub Url);

impl Deref for ContainerAddress {
    type Target = Url;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl ContainerAddress {
    /// Decode the container address from a url.
    pub fn try_from_url(url: &Url) -> Result<Self, Error> {
        let mut url = url.clone();
        canonicalize_azure_path(&mut url)?;
        let container = url
            .path_segments()
            .and_then(|mut segments| segments.next())
            .filter(|container| !container.is_empty())
            .ok_or_else(|| Error::invalid_argument("Url does not belong to a storage container"))?
            .to_owned();

        url.set_fragment(None);
        url.set_query(None);
        url.set_path(&container);

        Ok(Self(url))
    }

    /// Get the url of a resource in the container.
    pub fn get_resource_url(&self, key: &str) -> Url {
        let mut url = self.0.clone();
        debug_assert!(!url.path().is_empty());
        debug_assert!(!url.path().ends_with('/'));
        debug_assert!(url.query().is_none());
        debug_assert!(url.fragment().is_none());

        // When the path component is modified through the path_segments_mut method, whitespace characters
        // such as \t, \r, and \n are silently discarded...
        let path = format!("{}/{}", url.path(), encode_path(key));
        url.set_path(&path);

        url
    }

    /// Get the url of a folder in the container.
    pub fn get_folder_url(&self, key: &str) -> Url {
        debug_assert!(key.ends_with(DELIMITER));
        self.get_resource_url(key)
    }

    /// Get the url of a blob in the container.
    pub fn get_blob_url(&self, key: &str) -> Url {
        self.get_resource_url(key)
    }

    /// Get the address of a folder in the container.
    pub fn get_folder_address(&self, key: &str) -> String {
        self.get_folder_url(key).to_string()
    }
}

/// Append a trailing delimiter if needed.
pub fn append_delimiter_if_missing(url: &str) -> Cow<'_, str> {
    if url.ends_with(DELIMITER) {
        Cow::Borrowed(url)
    } else {
        Cow::Owned(format!("{url}{DELIMITER}"))
    }
}

/// Create a folder placeholder address from a url.
pub fn create_folder_placeholder(url: &str) -> String {
    if url.ends_with(DELIMITER) {
        format!("{url}{FOLDER_PLACEHOLDER}")
    } else {
        format!("{url}{DELIMITER}{FOLDER_PLACEHOLDER}")
    }
}

/// Parse the container and the folder addresses from a url.
pub fn parse_container_and_folder_address(url: &str) -> Result<(ContainerAddress, Address), Error> {
    let url = append_delimiter_if_missing(url);

    let url = Url::parse(&url).map_err(Error::from)?;

    let container_address = ContainerAddress::try_from_url(&url)?;
    let folder_address = Address::parse_prefix(url)?;

    Ok((container_address, folder_address))
}

fn container_from_path<'a>(path: &'a str, container_end: usize, url: &Url) -> Result<&'a str, Error> {
    path.get(1..container_end)
        .filter(|container| !container.is_empty())
        .ok_or_else(|| {
            Error::invalid_argument(format!(
                "Invalid Azure URL {url}: Missing container name. URLs should be in the form {BLOB_URL}"
            ))
        })
}

fn canonicalize_azure_path(url: &mut Url) -> Result<(), Error> {
    canonicalize_url_path(url)
        .map_err(|e| Error::invalid_argument(format!("URL path canonicalization failed for '{url}': {e}")))
}

/// Validate an Azure blob name against hard server-enforced limits.
///
/// These rules are enforced by the Azure Blob Storage service and will result in HTTP 400 (Bad
/// Request) or HTTP 414 (URI Too Long) if violated. We validate early to provide clear error
/// messages instead of opaque backend errors.
///
/// Azure also has soft recommendations (avoid trailing `.`, `/`, `\` and path segments ending
/// with `.`) that we intentionally do not enforce here — existing operations rely on trailing
/// slashes for folder semantics.
///
/// @see <https://learn.microsoft.com/en-us/rest/api/storageservices/naming-and-referencing-containers--blobs--and-metadata>
fn validate_blob_name(blob_name: &str, url: &Url) -> Result<(), Error> {
    const MAX_BLOB_NAME_LENGTH: usize = 1024;
    const MAX_PATH_SEGMENTS: usize = 254;

    if blob_name.len() > MAX_BLOB_NAME_LENGTH {
        return Err(Error::invalid_argument(format!(
            "blob name length {} exceeds maximum of {MAX_BLOB_NAME_LENGTH} characters: {url}",
            blob_name.len()
        )));
    }

    if blob_name.bytes().any(|b| b < 0x20) {
        return Err(Error::invalid_argument(format!(
            "blob name contains control characters: {url}"
        )));
    }

    let segment_count = blob_name.split('/').count();
    if segment_count > MAX_PATH_SEGMENTS {
        return Err(Error::invalid_argument(format!(
            "blob name has {segment_count} path segments, exceeds maximum of {MAX_PATH_SEGMENTS}: {url}"
        )));
    }

    Ok(())
}

/// Encode a path component.
pub fn encode_path(value: &str) -> Cow<'_, str> {
    encode_url_path(value)
}

/// Append quotes to an etag value.
pub fn quote_etag_if_needed(value: impl AsRef<str> + Into<String>) -> String {
    let value_ref = value.as_ref();
    if value_ref.starts_with('"') && value_ref.ends_with('"') {
        value.into()
    } else {
        format!("\"{value_ref}\"")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    fn assert_missing_container_error<T>(result: std::result::Result<T, Error>) {
        match result {
            Ok(_) => panic!("expected missing container error"),
            Err(Error::InvalidArgument { message, .. }) => {
                assert!(message.contains("Missing container name"), "{message}");
            },
            Err(error) => panic!("expected invalid argument error, got {error}"),
        }
    }

    #[test_log::test()]
    fn test_path_basic() {
        let url = Url::parse("https://account.blob.core.windows.net/container/blob").unwrap();
        let address = Address::parse_blob(url).unwrap();
        assert_eq!(address.container().unwrap(), "container");
        assert_eq!(address.blob(), "blob");
    }

    #[test_log::test()]
    fn test_path_basic_with_prefix() {
        let url = Url::parse("https://account.blob.core.windows.net/container/prefix").unwrap();
        let address = Address::parse_prefix(url).unwrap();
        assert_eq!(address.container().unwrap(), "container");
        assert_eq!(address.prefix(), "prefix");
    }

    #[test_log::test()]
    fn test_path_with_slashes() {
        let url = Url::parse("https://account.blob.core.windows.net/container/blob/with/slashes").unwrap();
        let address = Address::parse_blob(url).unwrap();
        assert_eq!(address.container().unwrap(), "container");
        assert_eq!(address.blob(), "blob/with/slashes");
    }

    #[test_log::test()]
    fn test_path_with_trailing_slash() {
        let url = Url::parse("https://account.blob.core.windows.net/container/blob/with/slashes/").unwrap();
        let address = Address::parse_blob(url).unwrap();
        assert_eq!(address.container().unwrap(), "container");
        assert_eq!(address.blob(), "blob/with/slashes/");
    }

    #[test_log::test()]
    fn test_path_no_blob() {
        let url = Url::parse("https://account.blob.core.windows.net/container").unwrap();
        assert!(Address::parse_blob(url).is_err());
    }

    #[test_log::test()]
    fn test_path_empty_blob() {
        let url = Url::parse("https://account.blob.core.windows.net/container/").unwrap();
        assert!(Address::parse_blob(url).is_err());
    }

    #[test_log::test()]
    fn test_path_empty() {
        let url = Url::parse("https://account.blob.core.windows.net/").unwrap();
        assert_missing_container_error(Address::parse_blob(url));
    }

    #[test_log::test()]
    fn test_parse_blob_empty_container() {
        let url = Url::parse("https://account.blob.core.windows.net//blob").unwrap();
        assert_missing_container_error(Address::parse_blob(url));
    }

    #[rstest]
    #[case::case_01_empty_container("https://account.blob.core.windows.net/")]
    #[case::case_02_double_slash_prefix("https://account.blob.core.windows.net//prefix")]
    #[case::case_03_custom_scheme("custom://account.blob.core.windows.net")]
    fn test_parse_prefix_empty_container(#[case] url: &str) {
        let url = Url::parse(url).unwrap();
        assert_missing_container_error(Address::parse_prefix(url));
    }

    #[test_log::test()]
    fn test_container_empty_path_returns_error() {
        let mut address =
            Address::parse_blob(Url::parse("https://account.blob.core.windows.net/container/blob").unwrap()).unwrap();

        address.resource_address = Url::parse("mailto:").unwrap();
        assert!(address.resource_address.path().is_empty());
        assert_missing_container_error(address.container());
    }

    #[test_log::test()]
    fn test_url() {
        let url = Url::parse(
            "https://exampleaccount.blob.core.windows.net/example-public/hub-storage-test/test_multiplart_blob",
        )
        .unwrap();
        let address = Address::parse_blob(url).unwrap();
        assert_eq!(address.container().unwrap(), "example-public");
        assert_eq!(address.blob(), "hub-storage-test/test_multiplart_blob");
    }

    #[rstest]
    #[case::case_01_valid_account("myaccount.blob.core.windows.net", Ok("myaccount"))]
    #[case::case_02_empty_account(".blob.core.windows.net", Err("Invalid Azure hostname format"))]
    #[case::case_03_missing_blob_segment("myaccount.core.windows.net", Err("Invalid Azure hostname format"))]
    #[case::case_04_missing_core_segment("myaccount.blob.windows.net", Err("Invalid Azure hostname format"))]
    #[case::case_05_missing_windows_segment("myaccount.blob.core.net", Err("Invalid Azure hostname format"))]
    #[case::case_06_missing_net_segment("myaccount.blob.core.windows", Err("Invalid Azure hostname format"))]
    #[case::case_07_extra_suffix("myaccount.blob.core.windows.net.extra", Err("Invalid Azure hostname format"))]
    #[case::case_08_reordered_segments("myaccount.core.blob.windows.net", Err("Invalid Azure hostname format"))]
    #[case::case_09_s3_hostname(
        "example-bucket-public.s3.us-west-2.amazonaws.com",
        Err("Invalid Azure hostname format")
    )]
    #[case::case_10_empty_hostname("", Err("Invalid Azure hostname format"))]
    fn test_parse_account(#[case] hostname: &str, #[case] expected: Result<&str, &str>) {
        let result = Address::parse_account(hostname);
        match expected {
            Ok(expected_account) => {
                assert!(result.is_ok());
                assert_eq!(result.unwrap(), expected_account);
            },
            Err(expected_error) => {
                assert!(result.is_err());
                assert!(result.unwrap_err().to_string().contains(expected_error));
            },
        }
    }

    #[rstest]
    #[case::case_01_container_root("https://host/container", "https://host/container")]
    #[case::case_02_container_with_path("https://host/container/key/key/key/", "https://host/container")]
    fn test_container_address_try_from_url(#[case] url: &str, #[case] expected_container_url: &str) {
        let url = Url::parse(url).unwrap();
        let container_address = ContainerAddress::try_from_url(&url);
        assert!(container_address.is_ok());
        assert_eq!(container_address.unwrap().as_str(), expected_container_url);
    }

    #[rstest]
    #[case::case_01_leading_slash("http://host/container", "/path", "http://host/container//path")]
    #[case::case_02_plain_path("http://host/container", "path", "http://host/container/path")]
    #[case::case_03_trailing_slash("http://host/container", "path/", "http://host/container/path/")]
    #[case::case_04_double_leading_slash(
        "http://host/container",
        "//path/to/file/",
        "http://host/container///path/to/file/"
    )]
    #[case::case_05_encoded_whitespace(
        "http://host/container",
        " path\twith\nwhitespaces\r\\",
        "http://host/container/%20path%09with%0Awhitespaces%0D%5C"
    )]
    fn test_container_address_get_resource_url(
        #[case] container_url: &str,
        #[case] path: &str,
        #[case] expected_url: &str,
    ) {
        let container_url = Url::parse(container_url).unwrap();
        let container_address = ContainerAddress::try_from_url(&container_url).unwrap();
        let url = container_address.get_resource_url(path);
        assert_eq!(url.as_str(), expected_url);
    }

    #[rstest]
    #[case::case_01_mailto("mailto:")]
    #[case::case_02_missing_path("https://myhost.mydomain")]
    #[case::case_03_empty_path("https://myhost.mydomain/")]
    fn test_container_address_try_from_url_errors_on_invalid_url(#[case] url: &str) {
        let container_url = Url::parse(url).unwrap();
        let container_address = ContainerAddress::try_from_url(&container_url);
        assert!(container_address.is_err());
    }

    #[rstest]
    #[case::case_01_missing_delimiter("http://host/path", "http://host/path/")]
    #[case::case_02_existing_delimiter("http://host/path/", "http://host/path/")]
    fn test_append_delimiter_if_missing(#[case] url: &str, #[case] expected: &str) {
        let url = append_delimiter_if_missing(url);
        assert_eq!(url, expected);
    }

    #[rstest]
    #[case::case_01_container_root(
        "https://account.blob.core.windows.net/container",
        "https://account.blob.core.windows.net/container",
        ""
    )]
    #[case::case_02_container_trailing_slash(
        "https://account.blob.core.windows.net/container/",
        "https://account.blob.core.windows.net/container",
        ""
    )]
    #[case::case_03_container_double_slash(
        "https://account.blob.core.windows.net/container//",
        "https://account.blob.core.windows.net/container",
        "/"
    )]
    #[case::case_04_container_with_path(
        "https://account.blob.core.windows.net/container/path",
        "https://account.blob.core.windows.net/container",
        "path/"
    )]
    #[case::case_05_encoded_whitespace_path(
        "https://account.blob.core.windows.net/container/path%20with%20whitespaces",
        "https://account.blob.core.windows.net/container",
        "path with whitespaces/"
    )]
    fn test_parse_container_url_and_folder_address(
        #[case] url: &str,
        #[case] expected_container_url: &str,
        #[case] expected_prefix: &str,
    ) {
        let (container_url, address) = parse_container_and_folder_address(url).unwrap();
        assert_eq!(container_url.as_str(), expected_container_url);
        assert_eq!(address.prefix(), expected_prefix);
    }

    #[test_log::test()]
    fn test_blob_name_at_max_length() {
        let blob_name = "a".repeat(1024);
        let url = Url::parse(&format!("https://account.blob.core.windows.net/container/{blob_name}")).unwrap();
        let address = Address::parse_blob(url);
        assert!(address.is_ok());
    }

    #[test_log::test()]
    fn test_blob_name_exceeds_max_length() {
        let blob_name = "a".repeat(1025);
        let url = Url::parse(&format!("https://account.blob.core.windows.net/container/{blob_name}")).unwrap();
        let result = Address::parse_blob(url);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("exceeds maximum of 1024 characters"), "{err}");
    }

    #[test_log::test()]
    fn test_blob_name_with_control_character() {
        let url = Url::parse("https://account.blob.core.windows.net/container/blob%00name").unwrap();
        let result = Address::parse_blob(url);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("control characters"), "{err}");
    }

    #[test_log::test()]
    fn test_blob_name_too_many_path_segments() {
        // 255 segments of single char = 255 + 254 slashes = 509 chars (under 1024)
        let blob_name = vec!["a"; 255].join("/");
        let url = Url::parse(&format!("https://account.blob.core.windows.net/container/{blob_name}")).unwrap();
        let result = Address::parse_blob(url);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("exceeds maximum of 254"), "{err}");
    }

    #[test_log::test()]
    fn test_blob_name_max_path_segments_ok() {
        // 254 segments of single char = 254 + 253 slashes = 507 chars (under 1024)
        let blob_name = vec!["a"; 254].join("/");
        let url = Url::parse(&format!("https://account.blob.core.windows.net/container/{blob_name}")).unwrap();
        let result = Address::parse_blob(url);
        assert!(result.is_ok());
    }

    #[test_log::test()]
    fn unit_identity_parse_strips_legacy_params() {
        // A legacy identity URL that still carries the pre-Phase-3 content-length and
        // last-modified query params must parse successfully — those params are
        // silently accepted and dropped. Only etag + version-id survive on Identity,
        // and `create_url` must strip the legacy params on re-serialization.
        let legacy = "https://account.blob.core.windows.net/container/blob?content-length=123&etag=%22abc%22&last-modified=Wed%2C+21+Oct+2015+07%3A28%3A00+GMT&version-id=v1";
        let identity = Identity::parse(legacy).unwrap();

        assert_eq!(identity.etag.as_deref(), Some("\"abc\""));
        assert_eq!(identity.version_id.as_deref(), Some("v1"));
        assert_eq!(identity.container().unwrap(), "container");
        assert_eq!(identity.blob(), "blob");

        // Re-serializing must round-trip without the legacy metadata params.
        let reserialized = Identity::create_url(
            Url::parse(legacy).unwrap(),
            identity.etag.clone(),
            identity.version_id.clone(),
        );
        assert!(
            !reserialized.contains("content-length"),
            "reserialized identity retained content-length: {reserialized}"
        );
        assert!(
            !reserialized.contains("last-modified"),
            "reserialized identity retained last-modified: {reserialized}"
        );
        assert!(reserialized.contains("etag=%22abc%22"), "{reserialized}");
        assert!(reserialized.contains("version-id=v1"), "{reserialized}");
    }

    #[test_log::test()]
    fn test_identity_parse_preserves_non_identity_query_parameters() {
        let identity = Identity::parse(
            "https://account.blob.core.windows.net/container/blob?download=1&etag=%22abc%22&custom=a%2Bb&version-id=v1",
        )
        .unwrap();

        assert_eq!(
            identity.resource_address.resource_address.query(),
            Some("download=1&custom=a%2Bb")
        );
        assert_eq!(
            Identity::create_url(
                identity.resource_address.resource_address.clone(),
                identity.etag.clone(),
                identity.version_id.clone(),
            ),
            "https://account.blob.core.windows.net/container/blob?download=1&custom=a%2Bb&etag=%22abc%22&version-id=v1"
        );
        assert_eq!(
            identity.resource_address.create_versioned_resource_address("v2"),
            "https://account.blob.core.windows.net/container/blob?download=1&custom=a%2Bb&version-id=v2"
        );
    }

    #[test_log::test()]
    fn test_versioned_address_strips_all_identity_parameters() {
        let address = Address::parse_prefix(
            Url::parse(
                "https://account.blob.core.windows.net/container/blob?ETAG=%22stale%22&content-length=5&LAST-MODIFIED=yesterday&Version-Id=old&download=1",
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(
            address.create_versioned_resource_address("v2"),
            "https://account.blob.core.windows.net/container/blob?download=1&version-id=v2"
        );
    }

    #[test_log::test()]
    fn unit_identity_parse_rejects_missing_etag() {
        let result = Identity::parse("https://account.blob.core.windows.net/container/blob?version-id=v1");
        let err = result.unwrap_err().to_string();
        assert!(err.contains("etag is missing"), "{err}");
    }

    #[test_log::test()]
    fn test_identity_parse_rejects_empty_etag() {
        let result = Identity::parse("https://account.blob.core.windows.net/container/blob?etag=&version-id=v1");
        let err = result.unwrap_err().to_string();
        assert!(err.contains("etag is missing"), "{err}");
    }

    #[test_log::test()]
    fn test_address_canonicalizes_loose_ascii_path() {
        let address = Address::parse_blob(
            Url::parse("https://account.blob.core.windows.net/container/!$%&'()*+,:;=@[]^|").unwrap(),
        )
        .unwrap();
        assert_eq!(
            address.to_string(),
            "https://account.blob.core.windows.net/container/%21%24%25%26%27%28%29%2A%2B%2C%3A%3B%3D%40%5B%5D%5E%7C"
        );

        let prefix = Address::parse_prefix(
            Url::parse("https://account.blob.core.windows.net/container/!$%&'()*+,:;=@[]^|/").unwrap(),
        )
        .unwrap();
        assert_eq!(
            prefix.to_string(),
            "https://account.blob.core.windows.net/container/%21%24%25%26%27%28%29%2A%2B%2C%3A%3B%3D%40%5B%5D%5E%7C/"
        );
    }

    #[test_log::test()]
    fn test_address_canonicalizes_container_and_blob_together() {
        let address =
            Address::parse_blob(Url::parse("https://account.blob.core.windows.net/my%2Dcontainer/a%2Fb").unwrap())
                .unwrap();
        assert_eq!(
            address.to_string(),
            "https://account.blob.core.windows.net/my-container/a/b"
        );
        assert_eq!(address.container().unwrap(), "my-container");
        assert_eq!(address.blob(), "a/b");

        let container = ContainerAddress::try_from_url(
            &Url::parse("https://account.blob.core.windows.net/my%2Dcontainer/a%2Fb").unwrap(),
        )
        .unwrap();
        assert_eq!(container.as_str(), "https://account.blob.core.windows.net/my-container");
    }

    #[test_log::test()]
    fn test_address_rejects_dot_segments_created_by_encoded_slashes() {
        for value in [
            "https://account.blob.core.windows.net/container/..%2Fabcdefghi",
            "https://account.blob.core.windows.net/container/.%2Fabcdefghi",
            "https://account.blob.core.windows.net/container/a%2F..%2Fb",
        ] {
            let error = Address::parse_blob(Url::parse(value).unwrap()).unwrap_err();
            assert!(error.to_string().contains("prohibited dot segment"), "{value}: {error}");
        }

        let prefix = "https://account.blob.core.windows.net/aaaaaaa/..%2Fsecrets/";
        let error = Address::parse_prefix(Url::parse(prefix).unwrap()).unwrap_err();
        assert!(error.to_string().contains("prohibited dot segment"), "{error}");

        let error = match ContainerAddress::try_from_url(&Url::parse(prefix).unwrap()) {
            Ok(_) => panic!("dot-segment container address should be rejected"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("prohibited dot segment"), "{error}");
    }

    #[test_log::test()]
    fn test_blob_accessor_does_not_panic_on_stale_offset() {
        let address = Address {
            resource_address: Url::parse("https://account.blob.core.windows.net/abcdefghi").unwrap(),
            account_end: 7,
            container_end: 10,
            key_start: 11,
        };

        assert_eq!(address.container().unwrap(), "abcdefghi");
        assert_eq!(address.blob(), "");
    }

    #[test_log::test()]
    fn test_account_accessor_does_not_panic_on_stale_offset() {
        let address = Address {
            resource_address: Url::parse("https://account.blob.core.windows.net/container/blob").unwrap(),
            account_end: usize::MAX,
            container_end: 10,
            key_start: 11,
        };

        let error = address.account().unwrap_err();
        assert!(error.to_string().contains("account offset exceeds hostname"), "{error}");
    }

    #[test_log::test()]
    fn test_address_canonicalization_is_idempotent() {
        let strict =
            "https://account.blob.core.windows.net/container/%21%24%25%26%27%28%29%2A%2B%2C%3A%3B%3D%40%5B%5D%5E%7C";
        assert_eq!(
            Address::parse_blob(Url::parse(strict).unwrap()).unwrap().to_string(),
            strict
        );
    }

    #[test_log::test()]
    fn test_address_canonicalization_preserves_decoded_slash() {
        let address =
            Address::parse_blob(Url::parse("https://account.blob.core.windows.net/container/a%2Fb").unwrap()).unwrap();
        assert_eq!(
            address.to_string(),
            "https://account.blob.core.windows.net/container/a/b"
        );
        assert_eq!(address.blob(), "a/b");
    }

    #[test_log::test()]
    fn test_address_canonicalization_round_trips_non_ascii() {
        let loose = Address::parse_blob(
            Url::parse("https://account.blob.core.windows.net/container/pekannöt/東京.txt").unwrap(),
        )
        .unwrap();
        let strict = "https://account.blob.core.windows.net/container/pekann%C3%B6t/%E6%9D%B1%E4%BA%AC.txt";
        assert_eq!(loose.to_string(), strict);
        assert_eq!(
            Address::parse_blob(Url::parse(strict).unwrap()).unwrap().to_string(),
            strict
        );
        assert_eq!(loose.blob(), "pekannöt/東京.txt");
    }

    #[test_log::test()]
    fn test_address_canonicalization_rejects_invalid_utf8() {
        let error = Address::parse_blob(Url::parse("https://account.blob.core.windows.net/container/%FF").unwrap())
            .unwrap_err();
        assert!(error.to_string().contains("path canonicalization failed"), "{error}");

        let error = Address::parse_prefix(Url::parse("https://account.blob.core.windows.net/container/%FF/").unwrap())
            .unwrap_err();
        assert!(error.to_string().contains("path canonicalization failed"), "{error}");
    }
}
