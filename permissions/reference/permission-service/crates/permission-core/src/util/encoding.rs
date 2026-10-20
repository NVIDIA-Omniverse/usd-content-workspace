// SPDX-FileCopyrightText: Copyright 2026 NVIDIA CORPORATION & AFFILIATES
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

use std::borrow::Cow;
use std::fmt::Write;
use url::Url;

/// Re-encodes path segments of a URL-shaped resource id so policy storage and
/// authorization queries compare against the same canonical form.
///
/// Resource ids are matched byte-exactly: whitespace and other characters
/// inside the id are preserved (never trimmed). The WHATWG URL parser used
/// by `Url::parse` would otherwise silently remove trailing C0 controls and
/// ASCII space, and any inline tab/LF/CR — collapsing two distinct ids
/// (e.g. `"foo"` and `"foo "`) into the same canonical form and letting a
/// spaced request id match a policy written on the canonical id. Because
/// the `url` crate offers no parse-option to disable this stripping, those
/// characters are percent-encoded prior to parsing so they survive into the
/// path and get normalized by the per-segment encoder.
///
/// Leading whitespace cannot be represented inside a URL (the scheme must
/// be the first token), so inputs that start with such a character are
/// returned byte-exact via the non-URL fallback.
pub fn encode_resource_url(input: &str) -> Result<String, String> {
    if input.starts_with(is_url_boundary_stripped_char) {
        return Ok(input.to_string());
    }

    let prepared = percent_encode_url_parser_stripped(input);

    let Ok(mut url) = Url::parse(&prepared) else {
        return Ok(input.to_string());
    };

    let path = url.path();
    let segments: Vec<&str> = path.split('/').collect();

    let mut encoded_segments = Vec::with_capacity(segments.len());
    for segment in segments {
        if segment.is_empty() {
            encoded_segments.push(String::new());
        } else {
            let decoded = urlencoding::decode(segment)
                .map_err(|e| format!("Failed to decode URL segment '{}': {}", segment, e))?;
            let encoded = urlencoding::encode(&decoded);
            encoded_segments.push(encoded.to_string());
        }
    }

    let encoded_path = encoded_segments.join("/");
    url.set_path(&encoded_path);

    Ok(url.to_string())
}

/// Percent-encodes every character that the WHATWG URL parser would
/// silently strip from `input`: any character in the trailing run of C0
/// controls or ASCII space, and any inline ASCII tab/LF/CR. The returned
/// string is safe to feed to `Url::parse` without losing those bytes.
fn percent_encode_url_parser_stripped(input: &str) -> Cow<'_, str> {
    let trailing_start = input
        .char_indices()
        .rev()
        .find(|(_, c)| !is_url_boundary_stripped_char(*c))
        .map(|(idx, c)| idx + c.len_utf8())
        .unwrap_or(0);

    let has_trailing = trailing_start < input.len();
    let has_inline = input[..trailing_start]
        .chars()
        .any(is_url_inline_stripped_char);

    if !has_trailing && !has_inline {
        return Cow::Borrowed(input);
    }

    let mut out = String::with_capacity(input.len() + 8);
    for (idx, c) in input.char_indices() {
        let in_trailing_run = idx >= trailing_start;
        if in_trailing_run || is_url_inline_stripped_char(c) {
            let mut buf = [0u8; 4];
            for &b in c.encode_utf8(&mut buf).as_bytes() {
                write!(out, "%{:02X}", b).expect("write to String never fails");
            }
        } else {
            out.push(c);
        }
    }
    Cow::Owned(out)
}

fn is_url_boundary_stripped_char(c: char) -> bool {
    c <= '\u{1F}' || c == ' '
}

fn is_url_inline_stripped_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_at_character_in_path() {
        let input = "https://example.com/user@domain/resource";
        let result = encode_resource_url(input).unwrap();
        assert_eq!(result, "https://example.com/user%40domain/resource");
    }

    #[test]
    fn test_encode_multiple_at_characters_in_path() {
        let input = "https://example.com/user@domain.test/resource@file";
        let result = encode_resource_url(input).unwrap();
        assert_eq!(
            result,
            "https://example.com/user%40domain.test/resource%40file"
        );
    }

    #[test]
    fn test_at_character_in_userinfo_not_encoded() {
        let input = "https://user:password@example.com/path";
        let result = encode_resource_url(input).unwrap();
        assert_eq!(result, "https://user:password@example.com/path");
    }

    #[test]
    fn test_at_character_in_query_not_encoded() {
        let input = "https://example.com/path?param=value@test";
        let result = encode_resource_url(input).unwrap();
        assert_eq!(result, "https://example.com/path?param=value@test");
    }

    #[test]
    fn test_at_character_in_fragment_not_encoded() {
        let input = "https://example.com/path#section@anchor";
        let result = encode_resource_url(input).unwrap();
        assert_eq!(result, "https://example.com/path#section@anchor");
    }

    #[test]
    fn test_only_path_encoded_with_special_chars() {
        let input = "https://user@example.com:8080/path with spaces?query=value with spaces#fragment with spaces";
        let result = encode_resource_url(input).unwrap();
        assert_eq!(result, "https://user@example.com:8080/path%20with%20spaces?query=value%20with%20spaces#fragment%20with%20spaces");
    }

    #[test]
    fn test_already_encoded_path_segments() {
        let input = "https://example.com/user%40domain/resource%20file";
        let result = encode_resource_url(input).unwrap();
        assert_eq!(result, "https://example.com/user%40domain/resource%20file");
    }

    #[test]
    fn test_mixed_encoded_and_unencoded_path() {
        let input = "https://example.com/user%40domain/resource file@test";
        let result = encode_resource_url(input).unwrap();
        assert_eq!(
            result,
            "https://example.com/user%40domain/resource%20file%40test"
        );
    }

    #[test]
    fn test_empty_path() {
        let input = "https://example.com";
        let result = encode_resource_url(input).unwrap();
        assert_eq!(result, "https://example.com/");
    }

    #[test]
    fn test_root_path_only() {
        let input = "https://example.com/";
        let result = encode_resource_url(input).unwrap();
        assert_eq!(result, "https://example.com/");
    }

    #[test]
    fn test_path_with_multiple_special_characters() {
        let input = "https://example.com/path/with spaces & symbols@test!$%";
        let result = encode_resource_url(input).unwrap();
        assert_eq!(
            result,
            "https://example.com/path/with%20spaces%20%26%20symbols%40test%21%24%25"
        );
    }

    #[test]
    fn test_single_quote_in_path_encoded() {
        let input = "https://example.com/path/team's asset.usd";
        let result = encode_resource_url(input).unwrap();
        assert_eq!(result, "https://example.com/path/team%27s%20asset.usd");
    }

    #[test]
    fn test_invalid_url_returned_as_is() {
        let input = "not-a-valid-url";
        let result = encode_resource_url(input).unwrap();
        assert_eq!(result, "not-a-valid-url");
    }

    #[test]
    fn test_relative_path_returned_as_is() {
        let input = "/relative/path/with@symbol";
        let result = encode_resource_url(input).unwrap();
        assert_eq!(result, "/relative/path/with@symbol");
    }

    #[test]
    fn test_path_with_empty_segments() {
        let input = "https://example.com//empty//segments/@test";
        let result = encode_resource_url(input).unwrap();
        assert_eq!(result, "https://example.com//empty//segments/%40test");
    }

    #[test]
    fn test_preserve_port_and_scheme() {
        let input = "http://localhost:3000/api/user@test/data";
        let result = encode_resource_url(input).unwrap();
        assert_eq!(result, "http://localhost:3000/api/user%40test/data");
    }

    #[test]
    fn test_preserve_complex_query_and_fragment() {
        let input = "https://api.example.com/users@domain/profile?user=john@doe.com&active=true#profile@section";
        let result = encode_resource_url(input).unwrap();
        assert_eq!(result, "https://api.example.com/users%40domain/profile?user=john@doe.com&active=true#profile@section");
    }

    #[test]
    fn test_unicode_characters_in_path() {
        let input = "https://example.com/path/用户@测试/file";
        let result = encode_resource_url(input).unwrap();
        assert_eq!(
            result,
            "https://example.com/path/%E7%94%A8%E6%88%B7%40%E6%B5%8B%E8%AF%95/file"
        );
    }

    #[test]
    fn test_trailing_space_in_url_encoded_in_path() {
        let input = "https://example.com/rc-bucket/ov-content/test/folder/ ";
        let result = encode_resource_url(input).unwrap();
        assert_eq!(
            result,
            "https://example.com/rc-bucket/ov-content/test/folder/%20"
        );
    }

    #[test]
    fn test_trailing_tab_in_url_encoded_in_path() {
        let input = "https://example.com/path\t";
        let result = encode_resource_url(input).unwrap();
        assert_eq!(result, "https://example.com/path%09");
    }

    #[test]
    fn test_inline_tab_in_url_encoded_in_path() {
        let input = "https://example.com/path\twith-tab";
        let result = encode_resource_url(input).unwrap();
        assert_eq!(result, "https://example.com/path%09with-tab");
    }

    #[test]
    fn test_inline_newline_in_url_encoded_in_path() {
        let input = "https://example.com/path\nwith-newline";
        let result = encode_resource_url(input).unwrap();
        assert_eq!(result, "https://example.com/path%0Awith-newline");
    }

    #[test]
    fn test_inline_carriage_return_in_url_encoded_in_path() {
        let input = "https://example.com/path\rwith-cr";
        let result = encode_resource_url(input).unwrap();
        assert_eq!(result, "https://example.com/path%0Dwith-cr");
    }

    /// Leading whitespace cannot be encoded into a URL scheme, so the input
    /// is preserved byte-exact via the non-URL fallback.
    #[test]
    fn test_leading_space_in_url_preserved_as_is() {
        let input = " https://example.com/path";
        let result = encode_resource_url(input).unwrap();
        assert_eq!(result, input);
    }

    /// Pins the current limitation: when the input starts with a character
    /// the WHATWG URL parser would strip from the front, `encode_resource_url`
    /// returns it byte-exact via the non-URL fallback and therefore skips
    /// path-segment encoding entirely — internal raw spaces, `@`, and other
    /// characters that would normally be percent-encoded are left untouched.
    #[test]
    fn test_leading_whitespace_url_skips_path_segment_encoding() {
        let input = " https://example.com/user@domain/path with spaces";
        let result = encode_resource_url(input).unwrap();
        assert_eq!(
            result, input,
            "URLs that start with a stripped char take the non-URL fallback \
             and are returned byte-exact, so the path is NOT canonicalized"
        );
        assert!(
            result.contains("user@domain"),
            "`@` should remain raw (NOT %40) because no URL encoding ran",
        );
        assert!(
            result.contains("path with spaces"),
            "internal spaces should remain raw (NOT %20) for the same reason",
        );
    }

    #[test]
    fn test_non_url_with_trailing_space_preserved_as_is() {
        let input = "Astronaut.usd ";
        let result = encode_resource_url(input).unwrap();
        assert_eq!(result, "Astronaut.usd ");
    }

    #[test]
    fn test_trailing_space_and_canonical_url_differ_after_encoding() {
        let canonical = "https://example.com/folder/";
        let spaced = "https://example.com/folder/ ";
        assert_ne!(
            encode_resource_url(canonical).unwrap(),
            encode_resource_url(spaced).unwrap(),
        );
    }
}
