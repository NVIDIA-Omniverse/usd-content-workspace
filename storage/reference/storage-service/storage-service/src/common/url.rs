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

use std::{borrow::Cow, str::FromStr};

use percent_encoding::{percent_decode_str, percent_encode, utf8_percent_encode, AsciiSet};
use url_escape::NON_ALPHANUMERIC;

/// The set of characters/bytes escaped by the encodeURIComponent function.
pub const URI_COMPONENT_SET: AsciiSet = NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'!')
    .remove(b'~')
    .remove(b'*')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')');

/// The set of characters/bytes escaped by the encodeURIComponent function minus the path separator.
pub const URI_COMPONENT_SET_PATH: AsciiSet = URI_COMPONENT_SET.remove(b'/');

/// Split fragment and query components from a uri string without parsing it.
pub fn split_query_from_url(uri: &str) -> (&str, Option<&str>) {
    let (before_frag, _) = uri.split_once("#").map(|(a, b)| (a, Some(b))).unwrap_or((uri, None));
    let (before_query, query) = before_frag
        .split_once("?")
        .map(|(a, b)| (a, Some(b)))
        .unwrap_or((before_frag, None));

    (before_query, query)
}

/// Parse a mapped checkpoint number from a query string.
pub fn parse_checkpoint_number(query: &str) -> Option<u64> {
    if let Some(suffix) = query.strip_prefix("&") {
        u64::from_str(suffix).map(Some).unwrap_or_default()
    } else {
        None
    }
}

/// Get the value of the etag query parameter in a query string.
pub fn get_etag_query_parameter(query: &str) -> Option<&str> {
    query.split('&').filter_map(|param| param.strip_prefix("etag=")).next()
}

/// Create a PercentTranscode helper for the specified source string and the desired escape character set.
pub fn percent_transcode<'a>(source: &'a str, desired_set: &'static AsciiSet) -> Cow<'a, str> {
    let decoded: Cow<'a, [u8]> = percent_decode_str(source).into();
    if let Cow::Owned(decoded) = decoded {
        let encoded: Cow<'_, str> = percent_encode(&decoded, desired_set).into();
        if let Cow::Owned(encoded) = encoded {
            Cow::Owned(encoded)
        } else {
            Cow::Owned(unsafe { String::from_utf8_unchecked(decoded) })
        }
    } else {
        utf8_percent_encode(source, desired_set).into()
    }
}

#[cfg(test)]
mod tests {
    use super::percent_transcode;
    use percent_encoding::{AsciiSet, CONTROLS};
    use rstest::rstest;

    #[rstest]
    #[case("with%20whitespace", "with whitesp%61ce")]
    #[case("1234%41%61", "%31234%41%61")]
    fn test_percent_transcode(#[case] value: &str, #[case] expected: &str) {
        static ASCII_SET: AsciiSet = CONTROLS.add(b'1').add(b'a').add(b'A');

        let actual = percent_transcode(value, &ASCII_SET);

        assert_eq!(actual, expected);
    }
}
