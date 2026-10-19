// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::fmt::Write;

use percent_encoding::percent_decode_str;
use regex::escape;

/// Match Azure Blob folder or object URI prefixes at well-known endpoints.
pub const WELL_KNOWN_AZ_BLOB_ANY_URI_REGEX: &str = concat!(
    r"\A(?i:https://[a-z0-9]+\.blob\.core\.windows\.net)/",
    r"(?i:[a-z0-9-]|%(?:2d|3[0-9]|4[1-9a-f]|5[0-9a]|6[1-9a-f]|7[0-9a]))+",
    r"(?:/|(?i:%2f)|[?#]|\z)",
);

/// Match Azure Blob object URI prefixes at well-known endpoints.
///
/// This pattern matches a subset of the URIs matched by [`WELL_KNOWN_AZ_BLOB_ANY_URI_REGEX`].
pub const WELL_KNOWN_AZ_BLOB_OBJECT_URI_REGEX: &str = concat!(
    r"\A(?i:https://[a-z0-9]+\.blob\.core\.windows\.net)/",
    r"(?i:[a-z0-9-]|%(?:2d|3[0-9]|4[1-9a-f]|5[0-9a]|6[1-9a-f]|7[0-9a]))+",
    r"(?:/|(?i:%2f))",
);

/// Match S3 folder or object URI prefixes at well-known endpoints.
pub const WELL_KNOWN_S3_ANY_URI_REGEX: &str = concat!(
    r"\A(?i:https://[a-z0-9-]+\.s3(?:\.|-)[a-z0-9-]+\.amazonaws\.com)",
    r"(?:/|\z)",
);

/// Match S3 object URI prefixes at well-known endpoints.
///
/// This pattern matches a subset of the URIs matched by [`WELL_KNOWN_S3_ANY_URI_REGEX`].
pub const WELL_KNOWN_S3_OBJECT_URI_REGEX: &str = r"\A(?i:https://[a-z0-9-]+\.s3(?:\.|-)[a-z0-9-]+\.amazonaws\.com)/";

/// Create a regular expression for Azure Blob account URI prefixes.
///
/// The authority must be followed by a literal slash, which is consumed, or the end of the URI.
pub fn az_blob_account_uri_regex(scheme: &str, authority: &str) -> String {
    let mut pattern = String::new();
    append_scheme_authority(&mut pattern, scheme, authority);
    pattern.push_str(r"(?:/|\z)");

    pattern
}

/// Create a regular expression for routing Azure Blob folder or object URIs.
///
/// The regex consumes the slash after the container name, whether literal or percent-encoded.
/// A literal query or fragment separator may also terminate the prefix and is consumed.
pub fn az_blob_any_uri_regex(scheme: &str, authority: &str) -> String {
    any_az_blob_uri_regex(scheme, authority, false)
}

/// Create a regular expression for routing Azure Blob Object URIs.
///
/// This pattern matches a subset of the URIs matched by [`az_blob_any_uri_regex`]
/// with the same arguments.
///
/// The regex consumes the slash after the container name, whether literal or percent-encoded.
pub fn az_blob_object_uri_regex(scheme: &str, authority: &str) -> String {
    any_az_blob_uri_regex(scheme, authority, true)
}

/// Create a regular expression for routing S3 folder or object URIs.
///
/// The path must not be percent-decoded before passing it to this function.
///
/// The regex consumes the trailing slash in the bucket URL, whether after the authority or in
/// the path, and whether literal or percent-encoded.
/// A literal query or fragment separator may also terminate a non-empty path prefix and is consumed.
pub fn s3_any_uri_regex(scheme: &str, authority: &str, path: &str) -> String {
    any_s3_uri_regex(scheme, authority, path, false)
}

/// Create a regular expression for routing S3 Object URIs.
///
/// This pattern matches a subset of the URIs matched by [`s3_any_uri_regex`]
/// with the same arguments.
///
/// The path must not be percent-decoded before passing it to this function.
///
/// The regex consumes the trailing slash in the bucket URL, whether after the authority or in
/// the path, and whether literal or percent-encoded.
pub fn s3_object_uri_regex(scheme: &str, authority: &str, path: &str) -> String {
    any_s3_uri_regex(scheme, authority, path, true)
}

fn any_az_blob_uri_regex(scheme: &str, authority: &str, require_trailing_slash: bool) -> String {
    let mut pattern = String::new();
    append_scheme_authority(&mut pattern, scheme, authority);
    append_authority_slash(&mut pattern);

    // Accept the ASCII character set without imposing Azure's container naming rules.
    pattern.push_str(r"(?i:[a-z0-9-]|%(?:2d|3[0-9]|4[1-9a-f]|5[0-9a]|6[1-9a-f]|7[0-9a]))+");
    pattern.push_str(if require_trailing_slash {
        r"(?:/|(?i:%2f))"
    } else {
        r"(?:/|(?i:%2f)|[?#]|\z)"
    });

    pattern
}

fn any_s3_uri_regex(scheme: &str, authority: &str, path: &str, require_trailing_slash: bool) -> String {
    enum PathRegexState {
        Start,
        LeadingSlash,
        Path,
        PendingSlash,
    }

    let mut pattern = String::new();
    append_scheme_authority(&mut pattern, scheme, authority);

    // Defer the most recent slash so the final slash becomes the route boundary.
    let mut state = PathRegexState::Start;
    for byte in percent_decode_str(path) {
        state = match state {
            PathRegexState::Start => match byte {
                b'/' => PathRegexState::LeadingSlash,
                byte => {
                    append_authority_slash(&mut pattern);
                    append_path_byte(&mut pattern, byte);

                    PathRegexState::Path
                },
            },
            PathRegexState::LeadingSlash => match byte {
                b'/' => {
                    append_authority_slash(&mut pattern);

                    PathRegexState::PendingSlash
                },
                byte => {
                    append_authority_slash(&mut pattern);
                    append_path_byte(&mut pattern, byte);

                    PathRegexState::Path
                },
            },
            PathRegexState::Path => match byte {
                b'/' => PathRegexState::PendingSlash,
                byte => {
                    append_path_byte(&mut pattern, byte);

                    PathRegexState::Path
                },
            },
            PathRegexState::PendingSlash => match byte {
                b'/' => {
                    append_path_byte(&mut pattern, b'/');

                    PathRegexState::PendingSlash
                },
                byte => {
                    append_path_byte(&mut pattern, b'/');
                    append_path_byte(&mut pattern, byte);

                    PathRegexState::Path
                },
            },
        };
    }

    pattern.push_str(match (state, require_trailing_slash) {
        (PathRegexState::Start | PathRegexState::LeadingSlash, true) => "/",
        (PathRegexState::Start | PathRegexState::LeadingSlash, false) => r"(?:/|\z)",
        (PathRegexState::Path | PathRegexState::PendingSlash, true) => r"(?:/|(?i:%2f))",
        (PathRegexState::Path | PathRegexState::PendingSlash, false) => r"(?:/|(?i:%2f)|[?#]|\z)",
    });

    pattern
}

fn append_scheme_authority(pattern: &mut String, scheme: &str, authority: &str) {
    write!(pattern, r"\A(?i:{}://{})", escape(scheme), escape(authority)).expect("writing to a string cannot fail");
}

fn append_authority_slash(pattern: &mut String) {
    pattern.push('/');
}

fn append_path_byte(pattern: &mut String, byte: u8) {
    if is_path_safe(byte) {
        write!(pattern, r"(?:\x{byte:02x}|(?i:%{byte:02x}))").expect("writing to a string cannot fail");
    } else {
        write!(pattern, "(?i:%{byte:02x})").expect("writing to a string cannot fail");
    }
}

fn is_path_safe(byte: u8) -> bool {
    matches!(
        byte,
        b'0'..=b'9'
        | b'A'..=b'Z'
        | b'a'..=b'z'
        | b'-'
        | b'.'
        | b'_'
        | b'~'
        | b'!'
        | b'$'
        | b'&'
        | b'\''
        | b'('
        | b')'
        | b'*'
        | b'+'
        | b','
        | b';'
        | b'='
        | b':'
        | b'@'
        | b'/'
    )
}

#[cfg(test)]
mod tests {
    use regex_automata::{dfa::regex::Regex, util::syntax};
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case("https://example.com", 19)]
    #[case("https://example.com/", 20)]
    #[case("HTTPS://EXAMPLE.COM", 19)]
    #[case("HTTPS://EXAMPLE.COM/", 20)]
    #[case("https://example.com/container/blob", 20)]
    #[case("https://example.com/%63ontainer%2fblob", 20)]
    #[case("https://example.com//container", 20)]
    fn test_az_blob_account_uri_regex_match(#[case] uri: &str, #[case] match_length: usize) {
        let regex = create_regex(&az_blob_account_uri_regex("HtTpS", "Example.COM"));
        let matched = regex.find(uri).expect("URI prefix must match");
        assert_eq!((matched.start(), matched.end()), (0, match_length));
    }

    #[rstest]
    #[case("https://example.com?query")]
    #[case("https://example.com#fragment")]
    #[case("https://example.com%2fcontainer")]
    #[case("https://example.com%2Fcontainer")]
    #[case("https://example.com.evil/container")]
    #[case("https://exampleXcom/container")]
    #[case("http://example.com/container")]
    #[case("prefixhttps://example.com/container")]
    #[case("https://other.example/container")]
    #[case("\nhttps://example.com/")]
    fn test_az_blob_account_uri_regex_ignore(#[case] uri: &str) {
        let regex = create_regex(&az_blob_account_uri_regex("HtTpS", "Example.COM"));
        assert!(!regex.is_match(uri));
    }

    #[rstest]
    #[case("https://example.com/a", 21)]
    #[case("https://example.com/-", 21)]
    #[case("https://example.com/--A--", 25)]
    #[case("https://example.com/0123456789", 30)]
    #[case("https://example.com/Container/", 30)]
    #[case("HTTPS://EXAMPLE.COM/Container/blob", 30)]
    #[case("https://example.com/container%2fblob", 32)]
    #[case("https://example.com/container%2Fblob", 32)]
    #[case("https://example.com/%41%4f%50%5A%61%6F%70%7a%30%39%2d/", 54)]
    #[case("https://example.com/c%6Fntainer%2D1%2fblob", 38)]
    #[case("https://example.com/container//blob?query#fragment", 30)]
    #[case(
        "https://example.com/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/blob",
        85
    )]
    #[case("https://example.com/container?query", 30)]
    #[case("https://example.com/container#fragment", 30)]
    #[case("https://example.com/container?", 30)]
    #[case("https://example.com/container#", 30)]
    #[case("https://example.com/%63ontainer?query#fragment", 32)]
    #[case("https://example.com/%63ontainer#fragment", 32)]
    fn test_az_blob_any_uri_regex_match(#[case] uri: &str, #[case] match_length: usize) {
        let regex = create_regex(&az_blob_any_uri_regex("HtTpS", "Example.COM"));
        let matched = regex.find(uri).expect("URI prefix must match");
        assert_eq!((matched.start(), matched.end()), (0, match_length));
    }

    #[rstest]
    #[case("https://example.com")]
    #[case("https://example.com/")]
    #[case("https://example.com//blob")]
    #[case("https://example.com%2fcontainer/blob")]
    #[case("https://example.com%2Fcontainer/blob")]
    #[case("http://example.com/container/blob")]
    #[case("https://exampleXcom/container/blob")]
    #[case("https://example.com.evil/container/blob")]
    #[case("prefixhttps://example.com/container/blob")]
    #[case("https://example.com/container_name/blob")]
    #[case("https://example.com/container.name/blob")]
    #[case("https://example.com/container+name/blob")]
    #[case("https://example.com/container%2e/blob")]
    #[case("https://example.com/container%2b/blob")]
    #[case("https://example.com/%2c/blob")]
    #[case("https://example.com/%2e/blob")]
    #[case("https://example.com/%2F/blob")]
    #[case("https://example.com/%3a/blob")]
    #[case("https://example.com/%40/blob")]
    #[case("https://example.com/%5b/blob")]
    #[case("https://example.com/%5f/blob")]
    #[case("https://example.com/%60/blob")]
    #[case("https://example.com/%7b/blob")]
    #[case("https://example.com/%80/blob")]
    #[case("https://example.com/%GG/blob")]
    #[case("https://example.com/%4/blob")]
    #[case("https://example.com/container%3fquery")]
    #[case("https://example.com/container%23fragment")]
    #[case("\nhttps://example.com/container/blob")]
    fn test_az_blob_any_uri_regex_ignore(#[case] uri: &str) {
        let regex = create_regex(&az_blob_any_uri_regex("HtTpS", "Example.COM"));
        assert!(!regex.is_match(uri));
    }

    #[rstest]
    #[case("https://example.com/Container/", 30)]
    #[case("HTTPS://EXAMPLE.COM/Container/blob", 30)]
    #[case("https://example.com/container%2fblob", 32)]
    #[case("https://example.com/container%2Fblob", 32)]
    #[case("https://example.com/%41%4f%50%5A%61%6F%70%7a%30%39%2d/", 54)]
    #[case("https://example.com/c%6Fntainer%2D1%2fblob", 38)]
    #[case("https://example.com/container//blob?query#fragment", 30)]
    #[case(
        "https://example.com/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/blob",
        85
    )]
    #[case("https://example.com/a/", 22)]
    #[case("https://example.com/-/", 22)]
    #[case("https://example.com/%2D%2f", 26)]
    fn test_az_blob_object_uri_regex_match(#[case] uri: &str, #[case] match_length: usize) {
        let regex = create_regex(&az_blob_object_uri_regex("HtTpS", "Example.COM"));
        let matched = regex.find(uri).expect("URI prefix must match");
        assert_eq!((matched.start(), matched.end()), (0, match_length));
    }

    #[rstest]
    #[case("https://example.com")]
    #[case("https://example.com/")]
    #[case("https://example.com//blob")]
    #[case("https://example.com%2fcontainer/blob")]
    #[case("https://example.com%2Fcontainer/blob")]
    #[case("http://example.com/container/blob")]
    #[case("https://exampleXcom/container/blob")]
    #[case("https://example.com.evil/container/blob")]
    #[case("prefixhttps://example.com/container/blob")]
    #[case("https://example.com/container_name/blob")]
    #[case("https://example.com/container.name/blob")]
    #[case("https://example.com/container+name/blob")]
    #[case("https://example.com/container%2e/blob")]
    #[case("https://example.com/container%2b/blob")]
    #[case("https://example.com/%2c/blob")]
    #[case("https://example.com/%2e/blob")]
    #[case("https://example.com/%2F/blob")]
    #[case("https://example.com/%3a/blob")]
    #[case("https://example.com/%40/blob")]
    #[case("https://example.com/%5b/blob")]
    #[case("https://example.com/%5f/blob")]
    #[case("https://example.com/%60/blob")]
    #[case("https://example.com/%7b/blob")]
    #[case("https://example.com/%80/blob")]
    #[case("https://example.com/%GG/blob")]
    #[case("https://example.com/%4/blob")]
    #[case("https://example.com/container?query")]
    #[case("https://example.com/container#fragment")]
    #[case("https://example.com/a")]
    #[case("https://example.com/container")]
    #[case("https://example.com/container%2D")]
    #[case("\nhttps://example.com/container/blob")]
    fn test_az_blob_object_uri_regex_ignore(#[case] uri: &str) {
        let regex = create_regex(&az_blob_object_uri_regex("HtTpS", "Example.COM"));
        assert!(!regex.is_match(uri));
    }

    #[rstest]
    #[case("", "https://example.com", 19)]
    #[case("/", "https://example.com", 19)]
    #[case("", "https://example.com/", 20)]
    #[case("/", "https://example.com/object", 20)]
    #[case("/bucket", "https://example.com/bucket", 26)]
    #[case("/bucket/", "https://example.com/bucket", 26)]
    #[case("bucket", "https://example.com/bucket/object", 27)]
    #[case("bucket/", "https://example.com/bucket/", 27)]
    #[case("/bucket", "HTTPS://EXAMPLE.COM/bucket/object", 27)]
    #[case("/Bucket", "https://example.com/%42ucket%2Fobject", 31)]
    #[case("/Bucket/", "https://example.com/Bucket%2fobject", 29)]
    #[case("/%42ucket%2f", "https://example.com/Bucket/object", 27)]
    #[case("/bucket/path", "https://example.com/bucket%2Fpath%2fobject", 36)]
    #[case("/bucket//path/", "https://example.com/bucket%2f%2Fpath/object", 37)]
    #[case("//bucket/", "https://example.com//bucket/object", 28)]
    #[case("//", "https://example.com/", 20)]
    #[case("//", "https://example.com//object", 21)]
    #[case("///", "https://example.com/%2f%2Fobject", 26)]
    #[case("///", "https://example.com//", 21)]
    #[case("/bucket//", "https://example.com/bucket/", 27)]
    #[case("/bucket//", "https://example.com/bucket//object", 28)]
    #[case("/a+b", "https://example.com/a%2bb/object", 26)]
    #[case("/a%2Bb/", "https://example.com/a+b%2fobject", 26)]
    #[case("/a%20b", "https://example.com/a%20b/object", 26)]
    #[case("/a%252fb", "https://example.com/a%252fb/object", 28)]
    #[case("/%3f%23%25%c3%a9/", "https://example.com/%3F%23%25%C3%A9/object", 36)]
    #[case(
        "/-._~!$&'()*+,;=:@/",
        "https://example.com/%2d%2e%5f%7e%21%24%26%27%28%29%2a%2b%2c%3b%3d%3a%40/object",
        72
    )]
    #[case("/bucket", "https://example.com/bucket/object?query#fragment", 27)]
    #[case("", "https://example.com/?query", 20)]
    #[case("/", "https://example.com/#fragment", 20)]
    #[case("/bucket", "https://example.com/bucket?query", 27)]
    #[case("/bucket", "https://example.com/bucket#fragment", 27)]
    #[case("/bucket/", "https://example.com/bucket?query#fragment", 27)]
    #[case("/bucket/", "https://example.com/bucket#fragment", 27)]
    #[case("//", "https://example.com/?query", 21)]
    #[case("//", "https://example.com/#fragment", 21)]
    #[case("/%3f%23", "https://example.com/%3F%23?query", 27)]
    #[case("/%3f%23/", "https://example.com/%3F%23#fragment", 27)]
    fn test_s3_any_uri_regex_match(#[case] path: &str, #[case] uri: &str, #[case] match_length: usize) {
        let regex = create_regex(&s3_any_uri_regex("HtTpS", "Example.COM", path));
        let matched = regex.find(uri).expect("URI prefix must match");
        assert_eq!((matched.start(), matched.end()), (0, match_length));
    }

    #[rstest]
    #[case("", "https://example.com%2fobject")]
    #[case("/", "https://example.com%2Fobject")]
    #[case("", "https://example.com.evil/object")]
    #[case("", "https://exampleXcom/object")]
    #[case("", "http://example.com/object")]
    #[case("", "prefixhttps://example.com/object")]
    #[case("/bucket", "https://example.com/bucket-other/object")]
    #[case("/bucket", "https://example.com/bucket2/object")]
    #[case("/bucket", "https://example.com%2fbucket/object")]
    #[case("/bucket", "https://example.com/Bucket/object")]
    #[case("/Bucket", "https://example.com/%62ucket/object")]
    #[case("/a+b", "https://example.com/a%20b/object")]
    #[case("/a%20b", "https://example.com/a+b/object")]
    #[case("/a%252fb", "https://example.com/a%2fb/object")]
    #[case("/a%3fb", "https://example.com/a?b/object")]
    #[case("/bucket//path", "https://example.com/bucket/path/object")]
    #[case("//bucket", "https://example.com/bucket/object")]
    #[case("///", "https://example.com/")]
    #[case("/bucket//", "https://example.com/bucket")]
    #[case("/bucket", "https://example.com/buck%GG/object")]
    #[case("/bucket", "https://example.com/%bucket/object")]
    #[case("/a.b", "https://example.com/aXb/object")]
    #[case("", "https://example.com%3fquery")]
    #[case("/", "https://example.com%23fragment")]
    #[case("/bucket", "https://example.com/bucket%3fquery")]
    #[case("/bucket", "https://example.com/bucket%23fragment")]
    #[case("", "https://example.com?query")]
    #[case("", "https://example.com#fragment")]
    #[case("/", "https://example.com?")]
    #[case("/", "https://example.com#")]
    #[case("/bucket", "\nhttps://example.com/bucket/object")]
    fn test_s3_any_uri_regex_ignore(#[case] path: &str, #[case] uri: &str) {
        let regex = create_regex(&s3_any_uri_regex("HtTpS", "Example.COM", path));
        assert!(!regex.is_match(uri));
    }

    #[rstest]
    #[case("", "https://example.com/", 20)]
    #[case("/", "https://example.com/object", 20)]
    #[case("bucket", "https://example.com/bucket/object", 27)]
    #[case("bucket/", "https://example.com/bucket/", 27)]
    #[case("/bucket", "HTTPS://EXAMPLE.COM/bucket/object", 27)]
    #[case("/Bucket", "https://example.com/%42ucket%2Fobject", 31)]
    #[case("/Bucket/", "https://example.com/Bucket%2fobject", 29)]
    #[case("/%42ucket%2f", "https://example.com/Bucket/object", 27)]
    #[case("/bucket/path", "https://example.com/bucket%2Fpath%2fobject", 36)]
    #[case("/bucket//path/", "https://example.com/bucket%2f%2Fpath/object", 37)]
    #[case("//bucket/", "https://example.com//bucket/object", 28)]
    #[case("//", "https://example.com//object", 21)]
    #[case("///", "https://example.com/%2f%2Fobject", 26)]
    #[case("/bucket//", "https://example.com/bucket//object", 28)]
    #[case("/a+b", "https://example.com/a%2bb/object", 26)]
    #[case("/a%2Bb/", "https://example.com/a+b%2fobject", 26)]
    #[case("/a%20b", "https://example.com/a%20b/object", 26)]
    #[case("/a%252fb", "https://example.com/a%252fb/object", 28)]
    #[case("/%3f%23%25%c3%a9/", "https://example.com/%3F%23%25%C3%A9/object", 36)]
    #[case(
        "/-._~!$&'()*+,;=:@/",
        "https://example.com/%2d%2e%5f%7e%21%24%26%27%28%29%2a%2b%2c%3b%3d%3a%40/object",
        72
    )]
    #[case("/bucket", "https://example.com/bucket/object?query#fragment", 27)]
    fn test_s3_object_uri_regex_match(#[case] path: &str, #[case] uri: &str, #[case] match_length: usize) {
        let regex = create_regex(&s3_object_uri_regex("HtTpS", "Example.COM", path));
        let matched = regex.find(uri).expect("URI prefix must match");
        assert_eq!((matched.start(), matched.end()), (0, match_length));
    }

    #[rstest]
    #[case("", "https://example.com%2fobject")]
    #[case("/", "https://example.com%2Fobject")]
    #[case("", "https://example.com.evil/object")]
    #[case("", "https://exampleXcom/object")]
    #[case("", "http://example.com/object")]
    #[case("", "prefixhttps://example.com/object")]
    #[case("", "https://example.com?query")]
    #[case("/", "https://example.com#fragment")]
    #[case("/bucket", "https://example.com/bucket-other/object")]
    #[case("/bucket", "https://example.com/bucket2/object")]
    #[case("/bucket", "https://example.com%2fbucket/object")]
    #[case("/bucket", "https://example.com/Bucket/object")]
    #[case("/Bucket", "https://example.com/%62ucket/object")]
    #[case("/bucket", "https://example.com/bucket?query")]
    #[case("/bucket", "https://example.com/bucket#fragment")]
    #[case("/a+b", "https://example.com/a%20b/object")]
    #[case("/a%20b", "https://example.com/a+b/object")]
    #[case("/a%252fb", "https://example.com/a%2fb/object")]
    #[case("/a%3fb", "https://example.com/a?b/object")]
    #[case("/bucket//path", "https://example.com/bucket/path/object")]
    #[case("//bucket", "https://example.com/bucket/object")]
    #[case("///", "https://example.com//")]
    #[case("/bucket//", "https://example.com/bucket")]
    #[case("/bucket", "https://example.com/buck%GG/object")]
    #[case("/bucket", "https://example.com/%bucket/object")]
    #[case("/a.b", "https://example.com/aXb/object")]
    #[case("", "https://example.com")]
    #[case("/", "https://example.com")]
    #[case("/bucket", "https://example.com/bucket")]
    #[case("/bucket/", "https://example.com/bucket")]
    #[case("//", "https://example.com/")]
    #[case("/bucket//", "https://example.com/bucket/")]
    #[case("/bucket", "\nhttps://example.com/bucket/object")]
    fn test_s3_object_uri_regex_ignore(#[case] path: &str, #[case] uri: &str) {
        let regex = create_regex(&s3_object_uri_regex("HtTpS", "Example.COM", path));
        assert!(!regex.is_match(uri));
    }

    #[rstest]
    #[case("https://account1.blob.core.windows.net/a", 40)]
    #[case("https://account1.blob.core.windows.net/--A--/", 45)]
    #[case("HTTPS://ACCOUNT1.BLOB.CORE.WINDOWS.NET/container/blob", 49)]
    #[case(
        "https://account1.blob.core.windows.net/%41%4f%50%5A%61%6F%70%7a%30%39%2d%2Fblob",
        75
    )]
    #[case("https://account1.blob.core.windows.net/container%2fblob", 51)]
    #[case("https://account1.blob.core.windows.net/container?query", 49)]
    #[case("https://account1.blob.core.windows.net/container#fragment", 49)]
    #[case("https://account1.blob.core.windows.net/container/blob?query#fragment", 49)]
    fn test_well_known_az_blob_any_uri_regex_match(#[case] uri: &str, #[case] match_length: usize) {
        let regex = create_regex(WELL_KNOWN_AZ_BLOB_ANY_URI_REGEX);
        let matched = regex.find(uri).expect("URI prefix must match");
        assert_eq!((matched.start(), matched.end()), (0, match_length));
    }

    #[rstest]
    #[case("https://account1.blob.core.windows.net")]
    #[case("https://account1.blob.core.windows.net/")]
    #[case("https://account1.blob.core.windows.net?query")]
    #[case("https://account1.blob.core.windows.net#fragment")]
    #[case("https://account1.blob.core.windows.net%2fcontainer/blob")]
    #[case("https://account1.blob.core.windows.net//blob")]
    #[case("https://account1.blob.core.windows.net/container_name/blob")]
    #[case("https://account1.blob.core.windows.net/container%3fquery")]
    #[case("https://account1.blob.core.windows.net/container%23fragment")]
    #[case("https://account1.blob.core.windows.net.evil/container/blob")]
    #[case("http://account1.blob.core.windows.net/container/blob")]
    #[case("https://account-1.blob.core.windows.net/container/blob")]
    #[case("https://account1Xblob.core.windows.net/container/blob")]
    #[case("prefixhttps://account1.blob.core.windows.net/container/blob")]
    #[case("\nhttps://account1.blob.core.windows.net/container/blob")]
    fn test_well_known_az_blob_any_uri_regex_ignore(#[case] uri: &str) {
        let regex = create_regex(WELL_KNOWN_AZ_BLOB_ANY_URI_REGEX);
        assert!(!regex.is_match(uri));
    }

    #[rstest]
    #[case("https://account1.blob.core.windows.net/--A--/", 45)]
    #[case("HTTPS://ACCOUNT1.BLOB.CORE.WINDOWS.NET/container/blob", 49)]
    #[case(
        "https://account1.blob.core.windows.net/%41%4f%50%5A%61%6F%70%7a%30%39%2d%2Fblob",
        75
    )]
    #[case("https://account1.blob.core.windows.net/container%2fblob", 51)]
    #[case("https://account1.blob.core.windows.net/container/blob?query#fragment", 49)]
    fn test_well_known_az_blob_object_uri_regex_match(#[case] uri: &str, #[case] match_length: usize) {
        let regex = create_regex(WELL_KNOWN_AZ_BLOB_OBJECT_URI_REGEX);
        let matched = regex.find(uri).expect("URI prefix must match");
        assert_eq!((matched.start(), matched.end()), (0, match_length));
    }

    #[rstest]
    #[case("https://account1.blob.core.windows.net")]
    #[case("https://account1.blob.core.windows.net/")]
    #[case("https://account1.blob.core.windows.net?query")]
    #[case("https://account1.blob.core.windows.net#fragment")]
    #[case("https://account1.blob.core.windows.net%2fcontainer/blob")]
    #[case("https://account1.blob.core.windows.net//blob")]
    #[case("https://account1.blob.core.windows.net/container_name/blob")]
    #[case("https://account1.blob.core.windows.net/container%3fquery")]
    #[case("https://account1.blob.core.windows.net/container%23fragment")]
    #[case("https://account1.blob.core.windows.net.evil/container/blob")]
    #[case("http://account1.blob.core.windows.net/container/blob")]
    #[case("https://account-1.blob.core.windows.net/container/blob")]
    #[case("https://account1Xblob.core.windows.net/container/blob")]
    #[case("prefixhttps://account1.blob.core.windows.net/container/blob")]
    #[case("https://account1.blob.core.windows.net/a")]
    #[case("https://account1.blob.core.windows.net/container?query")]
    #[case("https://account1.blob.core.windows.net/container#fragment")]
    #[case("\nhttps://account1.blob.core.windows.net/container/blob")]
    fn test_well_known_az_blob_object_uri_regex_ignore(#[case] uri: &str) {
        let regex = create_regex(WELL_KNOWN_AZ_BLOB_OBJECT_URI_REGEX);
        assert!(!regex.is_match(uri));
    }

    #[rstest]
    #[case("https://bucket-1.s3.us-west-2.amazonaws.com", 43)]
    #[case("https://bucket-1.s3-us-west-2.amazonaws.com", 43)]
    #[case("https://bucket-1.s3.us-west-2.amazonaws.com/", 44)]
    #[case("https://bucket-1.s3-us-west-2.amazonaws.com/object", 44)]
    #[case("HTTPS://BUCKET-1.S3.US-WEST-2.AMAZONAWS.COM/path?query#fragment", 44)]
    #[case("HTTPS://BUCKET-1.S3-US-WEST-2.AMAZONAWS.COM/path", 44)]
    #[case("https://bucket-1.s3.us-west-2.amazonaws.com/?query", 44)]
    #[case("https://bucket-1.s3-us-west-2.amazonaws.com/#fragment", 44)]
    #[case("https://bucket-1.s3.us-west-2.amazonaws.com/folder?query", 44)]
    #[case("https://bucket-1.s3-us-west-2.amazonaws.com/folder#fragment", 44)]
    #[case("https://bucket-1.s3.us-west-2.amazonaws.com/folder%2Fobject", 44)]
    fn test_well_known_s3_any_uri_regex_match(#[case] uri: &str, #[case] match_length: usize) {
        let regex = create_regex(WELL_KNOWN_S3_ANY_URI_REGEX);
        let matched = regex.find(uri).expect("URI prefix must match");
        assert_eq!((matched.start(), matched.end()), (0, match_length));
    }

    #[rstest]
    #[case("https://bucket-1.s3.us-west-2.amazonaws.com?query")]
    #[case("https://bucket-1.s3.us-west-2.amazonaws.com#fragment")]
    #[case("https://bucket-1.s3-us-west-2.amazonaws.com?query")]
    #[case("https://bucket-1.s3-us-west-2.amazonaws.com#fragment")]
    #[case("https://bucket-1.s3.us-west-2.amazonaws.com%2fobject")]
    #[case("https://bucket-1.s3-us-west-2.amazonaws.com%2Fobject")]
    #[case("https://bucket-1.s3.us-west-2.amazonaws.com.evil/object")]
    #[case("http://bucket-1.s3.us-west-2.amazonaws.com/object")]
    #[case("https://bucket_1.s3.us-west-2.amazonaws.com/object")]
    #[case("https://bucket-1.s3..amazonaws.com/object")]
    #[case("https://bucket-1.s3-.amazonaws.com/object")]
    #[case("https://bucket-1.s3-us-west-2Xamazonaws.com/object")]
    #[case("prefixhttps://bucket-1.s3.us-west-2.amazonaws.com/object")]
    #[case("https://bucket-1.s3.us-west-2.amazonaws.com%3fquery")]
    #[case("https://bucket-1.s3.us-west-2.amazonaws.com%23fragment")]
    #[case("\nhttps://bucket-1.s3.us-west-2.amazonaws.com/object")]
    fn test_well_known_s3_any_uri_regex_ignore(#[case] uri: &str) {
        let regex = create_regex(WELL_KNOWN_S3_ANY_URI_REGEX);
        assert!(!regex.is_match(uri));
    }

    #[rstest]
    #[case("https://bucket-1.s3.us-west-2.amazonaws.com/", 44)]
    #[case("https://bucket-1.s3-us-west-2.amazonaws.com/object", 44)]
    #[case("HTTPS://BUCKET-1.S3.US-WEST-2.AMAZONAWS.COM/path?query#fragment", 44)]
    #[case("HTTPS://BUCKET-1.S3-US-WEST-2.AMAZONAWS.COM/path", 44)]
    #[case("https://bucket-1.s3.us-west-2.amazonaws.com/?query", 44)]
    #[case("https://bucket-1.s3-us-west-2.amazonaws.com/#fragment", 44)]
    #[case("https://bucket-1.s3.us-west-2.amazonaws.com/folder?query", 44)]
    #[case("https://bucket-1.s3-us-west-2.amazonaws.com/folder#fragment", 44)]
    #[case("https://bucket-1.s3.us-west-2.amazonaws.com/folder%2Fobject", 44)]
    fn test_well_known_s3_object_uri_regex_match(#[case] uri: &str, #[case] match_length: usize) {
        let regex = create_regex(WELL_KNOWN_S3_OBJECT_URI_REGEX);
        let matched = regex.find(uri).expect("URI prefix must match");
        assert_eq!((matched.start(), matched.end()), (0, match_length));
    }

    #[rstest]
    #[case("https://bucket-1.s3.us-west-2.amazonaws.com?query")]
    #[case("https://bucket-1.s3.us-west-2.amazonaws.com#fragment")]
    #[case("https://bucket-1.s3-us-west-2.amazonaws.com?query")]
    #[case("https://bucket-1.s3-us-west-2.amazonaws.com#fragment")]
    #[case("https://bucket-1.s3.us-west-2.amazonaws.com%2fobject")]
    #[case("https://bucket-1.s3-us-west-2.amazonaws.com%2Fobject")]
    #[case("https://bucket-1.s3.us-west-2.amazonaws.com.evil/object")]
    #[case("http://bucket-1.s3.us-west-2.amazonaws.com/object")]
    #[case("https://bucket_1.s3.us-west-2.amazonaws.com/object")]
    #[case("https://bucket-1.s3..amazonaws.com/object")]
    #[case("https://bucket-1.s3-.amazonaws.com/object")]
    #[case("https://bucket-1.s3-us-west-2Xamazonaws.com/object")]
    #[case("prefixhttps://bucket-1.s3.us-west-2.amazonaws.com/object")]
    #[case("https://bucket-1.s3.us-west-2.amazonaws.com%3fquery")]
    #[case("https://bucket-1.s3.us-west-2.amazonaws.com%23fragment")]
    #[case("https://bucket-1.s3.us-west-2.amazonaws.com")]
    #[case("https://bucket-1.s3-us-west-2.amazonaws.com")]
    #[case("\nhttps://bucket-1.s3.us-west-2.amazonaws.com/object")]
    fn test_well_known_s3_object_uri_regex_ignore(#[case] uri: &str) {
        let regex = create_regex(WELL_KNOWN_S3_OBJECT_URI_REGEX);
        assert!(!regex.is_match(uri));
    }

    fn create_regex(pattern: &str) -> Regex {
        Regex::builder()
            .syntax(syntax::Config::new().unicode(false))
            .build(pattern)
            .expect("route regex must compile")
    }
}
