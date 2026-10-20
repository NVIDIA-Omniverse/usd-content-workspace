// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::borrow::Cow;

use regex::Regex;
use storage_core_util::url_path::canonicalize_url_path;
use url::Url;

/// Ensure that a resource identity has the expected path and parameters that identify an object.
///
/// The expected address is canonicalized before comparison because callers may use an equivalent
/// non-canonical spelling (for example, percent-encoded RFC 3986 unreserved characters). The
/// identity itself is deliberately left untouched so this assertion still rejects non-canonical
/// identity paths.
///
/// - `expect_version_id = true` → version-id must be present and non-empty.
/// - `expect_version_id = false` → version-id is optional. If it happens to be
///   present (e.g. on an Azure container where versioning was enabled at some
///   point in the past — Azure keeps returning version-ids for blobs that
///   have ever had one, even after versioning is disabled account-wide), it
///   still has to be well-formed (non-empty, not the literal sentinel "null").
///   We intentionally do not assert absence here because the backend's output
///   is not fully under the test's control.
#[track_caller]
pub fn assert_identity_has_object_identifier_parameters(
    identity: &str,
    expected_address: &str,
    expect_version_id: bool,
) {
    let resource_identity = Url::parse(identity).expect("resource identity parse failed");
    let mut resource_address = Url::parse(expected_address).expect("resource address parse failed");
    canonicalize_url_path(&mut resource_address).expect("resource address path canonicalization failed");
    assert_eq!(
        resource_identity.path(),
        resource_address.path(),
        "resource identity path does not match resource address"
    );
    let etag = get_query_parameter(&resource_identity, "etag").expect("etag is missing from query");
    let etag_regex = Regex::new("^\"[^\"]+\"$").unwrap();
    assert!(etag_regex.is_match(&etag));

    let version_id = get_query_parameter(&resource_identity, "version-id");
    if expect_version_id {
        let v = version_id.expect("version-id is missing from query");
        assert!(!v.is_empty(), "version-id is present but empty: {identity}");
        assert_ne!(&*v, "null", "version-id has sentinel value \"null\": {identity}");
    } else if let Some(v) = version_id {
        // Optional, but if present must be well-formed.
        assert!(!v.is_empty(), "version-id is present but empty: {identity}");
        assert_ne!(&*v, "null", "version-id has sentinel value \"null\": {identity}");
    }
}

/// Filter query parameters in an identity.
pub fn filter_identity_query(identity: &str, filter: impl Fn(&str) -> bool) -> String {
    let (prefix, query) = identity.split_once('?').expect("query component is missing");
    let query = filter_query(query, filter);

    format!("{prefix}?{query}")
}

/// Filter query parameters based on a predicate.
pub fn filter_query(query: &str, filter: impl Fn(&str) -> bool) -> String {
    query.split('&').filter(|p| filter(p)).collect::<Vec<_>>().join("&")
}

/// Get the query component from a url.
pub fn get_query(value: &str) -> &str {
    value.split_once("?").expect("query component is missing").1
}

/// Get the value of a query parameter.
pub fn get_query_parameter<'a>(uri: &'a Url, key: &str) -> Option<Cow<'a, str>> {
    uri.query_pairs()
        .filter_map(|(k, v)| if k == key { Some(v) } else { None })
        .next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_path_matches_overencoded_expected_address() {
        assert_identity_has_object_identifier_parameters(
            "https://host/~_-.txt?etag=%22value%22",
            "https://host/%7E%5F%2D%2Etxt",
            false,
        );
    }

    #[test]
    fn canonical_identity_path_matches_loose_expected_address() {
        assert_identity_has_object_identifier_parameters(
            "https://host/file%28one%29.txt?etag=%22value%22",
            "https://host/file(one).txt",
            false,
        );
    }

    #[test]
    #[should_panic(expected = "resource identity path does not match resource address")]
    fn noncanonical_identity_path_is_rejected() {
        assert_identity_has_object_identifier_parameters(
            "https://host/file(one).txt?etag=%22value%22",
            "https://host/file(one).txt",
            false,
        );
    }
}
