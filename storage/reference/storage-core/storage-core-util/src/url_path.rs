// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Canonical URL-path encoding shared by storage backends, middleware, and API adapters.

use std::{borrow::Cow, fmt, str::Utf8Error};

use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use url::Url;

/// RFC 3986 path encoding that preserves only unreserved bytes and `/` separators.
const URL_PATH_ESCAPE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~')
    .remove(b'/');

/// Error returned when a URL path cannot be converted to its canonical spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanonicalizeUrlPathError {
    /// The percent-decoded path is not valid UTF-8.
    InvalidUtf8(Utf8Error),
    /// Percent-decoding introduced a `.` or `..` path segment.
    DotSegment,
}

impl fmt::Display for CanonicalizeUrlPathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUtf8(source) => write!(f, "percent-decoded path is not valid UTF-8: {source}"),
            Self::DotSegment => write!(f, "percent-decoded path contains a prohibited dot segment"),
        }
    }
}

impl std::error::Error for CanonicalizeUrlPathError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidUtf8(source) => Some(source),
            Self::DotSegment => None,
        }
    }
}

impl From<Utf8Error> for CanonicalizeUrlPathError {
    fn from(value: Utf8Error) -> Self {
        Self::InvalidUtf8(value)
    }
}

/// Encode a decoded URL path using storage-core's canonical spelling.
///
/// This function does not perform Unicode normalization; canonically equivalent NFC and NFD
/// strings remain distinct object keys.
pub fn encode_url_path(path: &str) -> Cow<'_, str> {
    utf8_percent_encode(path, URL_PATH_ESCAPE_SET).into()
}

/// Decode and canonically re-encode the path of an already-parsed hierarchical URL.
///
/// Query and fragment components are unchanged. [`Url::set_path`] intentionally preserves the
/// percent escapes produced by [`encode_url_path`]; using `path_segments_mut` here would encode
/// their `%` bytes a second time.
///
/// Any `.` or `..` segments introduced by this function's percent-decoding are rejected before
/// calling [`Url::set_path`]. For special URLs, `set_path` applies WHATWG dot-segment removal;
/// allowing an encoded slash to turn an opaque object-key segment such as `..%2Fkey` into
/// `../key` would otherwise change the addressed object and could cross a bucket or container
/// boundary. This specifically guards dot segments revealed by percent-decoding: [`Url::parse`]
/// may already have removed literal or independently percent-encoded dot segments before this
/// function receives the URL.
pub fn canonicalize_url_path(url: &mut Url) -> Result<(), CanonicalizeUrlPathError> {
    let decoded = percent_decode_str(url.path()).decode_utf8()?.into_owned();
    if decoded.split('/').any(|segment| matches!(segment, "." | "..")) {
        return Err(CanonicalizeUrlPathError::DotSegment);
    }
    url.set_path(&encode_url_path(&decoded));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOOSE_DIVERGENT_ASCII: &str = "!$%&'()*+,:;=@[]^|";
    const STRICT_DIVERGENT_ASCII: &str = "%21%24%25%26%27%28%29%2A%2B%2C%3A%3B%3D%40%5B%5D%5E%7C";

    fn canonicalized(value: &str) -> Result<String, CanonicalizeUrlPathError> {
        let mut url = Url::parse(value).unwrap();
        canonicalize_url_path(&mut url)?;
        Ok(url.to_string())
    }

    #[test]
    fn loose_divergent_ascii_becomes_strict_and_is_idempotent() {
        let loose = format!("https://host/container/{LOOSE_DIVERGENT_ASCII}?etag=value#fragment");
        let strict = format!("https://host/container/{STRICT_DIVERGENT_ASCII}?etag=value#fragment");

        assert_eq!(canonicalized(&loose).unwrap(), strict);
        assert_eq!(canonicalized(&strict).unwrap(), strict);
    }

    #[test]
    fn encoded_slash_becomes_a_path_separator() {
        assert_eq!(canonicalized("https://host/a%2Fb").unwrap(), "https://host/a/b");
    }

    #[test]
    fn decoded_dot_segments_are_rejected_before_set_path() {
        for value in [
            "https://host/container/..%2Fkey",
            "https://host/container/.%2Fkey",
            "https://host/container/a%2F..%2Fb",
            "https://host/container/%2E%2E%2Fkey",
        ] {
            assert_eq!(
                canonicalized(value),
                Err(CanonicalizeUrlPathError::DotSegment),
                "{value}"
            );
        }
    }

    #[test]
    fn percent_encoded_dot_text_is_not_decoded_twice() {
        assert_eq!(
            canonicalized("https://host/container/%252E%252E%252Fkey").unwrap(),
            "https://host/container/%252E%252E%252Fkey"
        );
    }

    #[test]
    fn literal_percent_is_encoded_once() {
        assert_eq!(canonicalized("https://host/a%2528b").unwrap(), "https://host/a%2528b");
        assert_eq!(canonicalized("https://host/a%28b").unwrap(), "https://host/a%28b");
    }

    #[test]
    fn non_ascii_round_trips() {
        let strict = "https://host/pekann%C3%B6t/%E6%9D%B1%E4%BA%AC.txt";
        assert_eq!(canonicalized("https://host/pekannöt/東京.txt").unwrap(), strict);
        assert_eq!(canonicalized(strict).unwrap(), strict);
    }

    #[test]
    fn invalid_utf8_is_rejected() {
        let mut url = Url::parse("https://host/%FF").unwrap();
        assert!(canonicalize_url_path(&mut url).is_err());
    }
}
