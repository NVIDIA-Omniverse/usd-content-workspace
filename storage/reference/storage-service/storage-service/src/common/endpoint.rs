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

use snafu::{Location, Snafu};

/// EndpointURL parsing error.
#[derive(Debug, Snafu)]
#[snafu(module, context(suffix(false)))]
pub enum ParseError {
    #[snafu(display("unexpected character at :{idx}"))]
    UnexpectedCharacter {
        idx: usize,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("unexpected end of input"))]
    UnexpectedEOF {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("too many path segments"))]
    TooManyPathSegments {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("url scheme is not supported"))]
    InvalidScheme {
        #[snafu(provide, implicit)]
        location: Location,
    },
}

impl ParseError {
    #[track_caller]
    fn unexpected_character_error<T>(idx: usize) -> Result<T, Self> {
        parse_error::UnexpectedCharacter { idx }.fail()
    }

    #[track_caller]
    fn unexpected_eof_error<T>() -> Result<T, Self> {
        parse_error::UnexpectedEOF.fail()
    }
}

/// An API endpoint URL, trailing slash included.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct EndpointUrl {
    url: String,
    scheme_end: usize,
    hostname_port_end: usize,
}

impl EndpointUrl {
    /// Parse with options.
    pub fn parse_with_options<T>(url: &str, options: ParseOptions<T>) -> Result<Self, ParseError>
    where
        T: Fn(&str) -> bool,
    {
        let parse_result = options.parse(url)?;
        let maybe_trailing_slash = if parse_result.path.ends_with("/") { "" } else { "/" };

        Ok(Self {
            url: format!(
                "{}://{}{}{}",
                parse_result.scheme, parse_result.hostname_port, parse_result.path, maybe_trailing_slash
            ),
            scheme_end: parse_result.scheme.len(),
            hostname_port_end: parse_result.scheme.len() + parse_result.hostname_port.len() + 3,
        })
    }

    /// Parse an Azure Blob endpoint URL.
    pub fn parse_azure_blob_endpoint_url(url: &str) -> Result<Self, ParseError> {
        Self::parse_with_options(
            url,
            ParseOptions {
                max_path_segments: 0,
                validate_scheme: |scheme: &str| {
                    scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")
                },
            },
        )
    }

    /// Parse an S3 endpoint URL.
    pub fn parse_s3_endpoint_url(url: &str) -> Result<Self, ParseError> {
        Self::parse_with_options(
            url,
            ParseOptions {
                max_path_segments: 1,
                validate_scheme: |scheme: &str| {
                    scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")
                },
            },
        )
    }

    /// Return the scheme component.
    pub fn scheme(&self) -> &str {
        debug_assert!(!self.url.is_empty());
        debug_assert!(self.url.len() > self.scheme_end);

        &self.url[..self.scheme_end]
    }

    /// Return the authority component.
    pub fn hostname_port(&self) -> &str {
        debug_assert!(!self.url.is_empty());
        debug_assert!(self.url.len() > self.hostname_port_end);
        debug_assert!(self.scheme_end < self.hostname_port_end);

        &self.url[self.scheme_end + 3..self.hostname_port_end]
    }

    /// Return the path component.
    pub fn path(&self) -> &str {
        debug_assert!(!self.url.is_empty());
        debug_assert!(self.url.len() > self.hostname_port_end);

        &self.url[self.hostname_port_end..]
    }

    /// Return the normalized URL string (scheme, authority, and path with a trailing slash).
    pub fn url(&self) -> &str {
        debug_assert!(self.url.ends_with("/"));

        &self.url
    }

    /// Return the URL string.
    pub fn into_inner(self) -> String {
        debug_assert!(!self.url.is_empty());
        debug_assert!(self.url.ends_with("/"));

        self.url
    }

    /// Drop the trailing slash from the internal representation and return the underlying URL string.
    pub fn into_inner_without_trailing_slash(mut self) -> String {
        debug_assert!(!self.url.is_empty());
        debug_assert!(self.url.ends_with("/"));

        self.url.pop();
        self.url
    }
}

impl AsRef<EndpointUrl> for EndpointUrl {
    fn as_ref(&self) -> &EndpointUrl {
        self
    }
}

impl std::fmt::Display for EndpointUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.url)
    }
}

/// Endpoint URL matcher build error.
#[derive(Debug, Snafu)]
#[snafu(module, context(suffix(false)))]
pub enum BuildError {
    #[snafu(display("failed to create a pattern matcher"))]
    RegexCreate {
        source: Box<dyn std::error::Error + Send + Sync>,
        #[snafu(provide, implicit)]
        location: Location,
    },
}

/// Endpoint URL parsing options.
pub struct ParseOptions<T> {
    pub max_path_segments: usize,
    pub validate_scheme: T,
}

impl<T> ParseOptions<T>
where
    T: Fn(&str) -> bool,
{
    fn parse<'a>(&self, value: &'a str) -> Result<ParseResult<'a>, ParseError> {
        let mut parse_result = ParseResult {
            scheme: "",
            hostname_port: "",
            path: "",
        };
        self.continue_with_scheme(&mut parse_result, value)?;

        Ok(parse_result)
    }

    fn continue_with_scheme<'a>(&self, result: &mut ParseResult<'a>, value: &'a str) -> Result<(), ParseError> {
        for (idx, b) in value.bytes().enumerate() {
            match b {
                b'a'..=b'z' | b'A'..=b'Z' => {
                    continue;
                },
                b'0'..=b'9' | b'-' | b'+' | b'.' => {
                    if idx == 0 {
                        return ParseError::unexpected_character_error(idx);
                    }
                    continue;
                },
                b':' => {
                    if idx == 0 {
                        return ParseError::unexpected_character_error(idx);
                    }

                    let scheme = &value[..idx];
                    if !(self.validate_scheme)(scheme) {
                        return parse_error::InvalidScheme.fail();
                    }

                    result.scheme = scheme;
                    return self.continue_with_scheme_sep(result, &value[idx + 1..], idx + 1);
                },
                _ => {
                    return ParseError::unexpected_character_error(idx);
                },
            }
        }

        ParseError::unexpected_eof_error()
    }

    fn continue_with_scheme_sep<'a>(
        &self,
        result: &mut ParseResult<'a>,
        value: &'a str,
        offset: usize,
    ) -> Result<(), ParseError> {
        for (idx, b) in value.bytes().enumerate() {
            match b {
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' => {
                    if idx != 2 {
                        return ParseError::unexpected_character_error(offset + idx);
                    }

                    return self.continue_with_hostname_and_port(result, &value[idx..], offset + idx);
                },
                b'/' => {
                    if idx > 1 {
                        return ParseError::unexpected_character_error(offset + idx);
                    }

                    continue;
                },
                _ => {
                    return ParseError::unexpected_character_error(offset + idx);
                },
            }
        }

        ParseError::unexpected_eof_error()
    }

    fn continue_with_hostname_and_port<'a>(
        &self,
        result: &mut ParseResult<'a>,
        value: &'a str,
        offset: usize,
    ) -> Result<(), ParseError> {
        #[allow(clippy::enum_variant_names)]
        enum State {
            ReadAlnum,
            ReadAlnumHyphen,
            ReadAlnumHyphenDotColonSlashEof,
            ReadNonZero,
            ReadNumSlashEof,
        }

        let mut state = State::ReadAlnum;
        for (idx, b) in value.bytes().enumerate() {
            match state {
                State::ReadAlnum => match b {
                    b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' => {
                        state = State::ReadAlnumHyphenDotColonSlashEof;
                    },
                    _ => {
                        return ParseError::unexpected_character_error(offset + idx);
                    },
                },
                State::ReadAlnumHyphen => match b {
                    b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' => {
                        state = State::ReadAlnumHyphenDotColonSlashEof;
                    },
                    b'-' => {
                        state = State::ReadAlnumHyphen;
                    },
                    _ => {
                        return ParseError::unexpected_character_error(offset + idx);
                    },
                },
                State::ReadAlnumHyphenDotColonSlashEof => match b {
                    b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' => {
                        state = State::ReadAlnumHyphenDotColonSlashEof;
                    },
                    b'-' => {
                        state = State::ReadAlnumHyphen;
                    },
                    b'.' => {
                        state = State::ReadAlnum;
                    },
                    b':' => {
                        state = State::ReadNonZero;
                    },
                    b'/' => {
                        result.hostname_port = &value[..idx];
                        return self.continue_with_path(result, &value[idx..], offset + idx);
                    },
                    _ => {
                        return ParseError::unexpected_character_error(offset + idx);
                    },
                },
                State::ReadNonZero => match b {
                    b'1'..=b'9' => {
                        state = State::ReadNumSlashEof;
                    },
                    _ => {
                        return ParseError::unexpected_character_error(offset + idx);
                    },
                },
                State::ReadNumSlashEof => match b {
                    b'0'..=b'9' => {
                        state = State::ReadNumSlashEof;
                    },
                    b'/' => {
                        result.hostname_port = &value[..idx];
                        return self.continue_with_path(result, &value[idx..], offset + idx);
                    },
                    _ => {
                        return ParseError::unexpected_character_error(offset + idx);
                    },
                },
            }
        }

        if !matches!(state, State::ReadAlnumHyphenDotColonSlashEof | State::ReadNumSlashEof) {
            return ParseError::unexpected_eof_error();
        }

        result.hostname_port = value;

        Ok(())
    }

    fn continue_with_path<'a>(
        &self,
        result: &mut ParseResult<'a>,
        value: &'a str,
        offset: usize,
    ) -> Result<(), ParseError> {
        debug_assert!(!value.is_empty());
        debug_assert_eq!(value.as_bytes()[0], b'/');

        let mut path_segments = 0usize;
        let mut has_trailing_slash = false;
        for (idx, b) in value.bytes().enumerate() {
            match b {
                b'a'..=b'z'
                | b'A'..=b'Z'
                | b'0'..=b'9'
                | b'-'
                | b'.'
                | b'_'
                | b'~'
                | b'!'
                | b'$'
                | b'&'
                | b'('
                | b')'
                | b'+'
                | b','
                | b';'
                | b'=' => {
                    if has_trailing_slash {
                        has_trailing_slash = false;

                        path_segments += 1;
                        if path_segments > self.max_path_segments {
                            return parse_error::TooManyPathSegments.fail();
                        }
                    }
                },
                b'/' => {
                    has_trailing_slash = true;
                },
                _ => {
                    return ParseError::unexpected_character_error(offset + idx);
                },
            }
        }

        result.path = value;

        Ok(())
    }
}

struct ParseResult<'a> {
    scheme: &'a str,
    hostname_port: &'a str,
    path: &'a str,
}

#[cfg(test)]
mod tests {
    use super::{EndpointUrl, ParseOptions};

    use rstest::rstest;

    #[rstest]
    #[case(
        "https://myhost.mydomain",
        "https://myhost.mydomain/",
        "https",
        "myhost.mydomain",
        "/"
    )]
    #[case(
        "https://myhost.mydomain/",
        "https://myhost.mydomain/",
        "https",
        "myhost.mydomain",
        "/"
    )]
    #[case(
        "https://myhost.mydomain/path",
        "https://myhost.mydomain/path/",
        "https",
        "myhost.mydomain",
        "/path/"
    )]
    #[case(
        "https://myhost.mydomain/path/",
        "https://myhost.mydomain/path/",
        "https",
        "myhost.mydomain",
        "/path/"
    )]
    #[case(
        "https://myhost.mydomain/path//",
        "https://myhost.mydomain/path//",
        "https",
        "myhost.mydomain",
        "/path//"
    )]
    #[case("https://64.129.33.22", "https://64.129.33.22/", "https", "64.129.33.22", "/")]
    #[case(
        "https://localhost.localdomain:3389",
        "https://localhost.localdomain:3389/",
        "https",
        "localhost.localdomain:3389",
        "/"
    )]
    #[case("ftp://localhost:3389", "ftp://localhost:3389/", "ftp", "localhost:3389", "/")]
    #[case(
        "ftp://localhost:3389/path",
        "ftp://localhost:3389/path/",
        "ftp",
        "localhost:3389",
        "/path/"
    )]
    #[case(
        "ftp://localhost:3389/path/a/",
        "ftp://localhost:3389/path/a/",
        "ftp",
        "localhost:3389",
        "/path/a/"
    )]
    fn test_endpoint_url_parse_with_options(
        #[case] value: &str,
        #[case] expected: &str,
        #[case] expected_scheme: &str,
        #[case] expected_authorty: &str,
        #[case] expected_path: &str,
    ) {
        let parsed = EndpointUrl::parse_with_options(
            value,
            ParseOptions {
                max_path_segments: 2,
                validate_scheme: |scheme: &str| matches!(scheme, "http" | "https" | "ftp"),
            },
        )
        .expect("endpoint url parse failed");

        assert_eq!(parsed.scheme(), expected_scheme);
        assert_eq!(parsed.hostname_port(), expected_authorty);
        assert_eq!(parsed.path(), expected_path);
        assert_eq!(parsed.into_inner(), expected);
    }

    #[rstest]
    #[case("0://localhost.localdomain")]
    #[case("-scheme://localhost.localdomain")]
    #[case("scheme:://localhost.localdomain")]
    #[case("scheme:///localhost.localdomain")]
    #[case("scheme:/localhost.localdomain")]
    #[case("scheme")]
    #[case("scheme:")]
    #[case("scheme://")]
    #[case("scheme://-label")]
    #[case("scheme://label-")]
    #[case("scheme://label-.label")]
    #[case("scheme://label-1.label?")]
    #[case("scheme://label-1.label:")]
    #[case("scheme://label-1.label:0999")]
    #[case("scheme://label-1.label:/")]
    #[case("scheme://label-1.label:223?")]
    #[case("scheme://label-1.label:223/?query")]
    #[case("scheme://label-1.label:223/%61%62%63")]
    #[case("scheme://label-1.label:223/path?query")]
    #[case("scheme://label-1.label:223/path#frag")]
    #[case("scheme://label-1.label:223/a/b/c")]
    #[case("https://localhost.localdomain")]
    fn test_endpoint_url_parse_with_options_invalid(#[case] value: &str) {
        EndpointUrl::parse_with_options(
            value,
            ParseOptions {
                max_path_segments: 2,
                validate_scheme: |scheme: &str| matches!(scheme, "scheme" | "-scheme" | "0"),
            },
        )
        .expect_err("url parse is expected to fail");
    }
}
