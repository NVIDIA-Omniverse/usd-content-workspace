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

use crate::backend::az_blob_name_rejecter::error::{invalid_url, InvalidUrl};

pub fn validate_blob_name(url_suffix: impl IntoIterator<Item = u8>) -> Result<(), InvalidUrl> {
    use invalid_url::*;

    enum State {
        Path,
        TrailingDot,
        AfterC2,
    }

    let mut url_suffix = url_suffix.into_iter();
    let mut state = State::Path;
    while let Some(byte) = try_next_byte(&mut url_suffix)? {
        let byte = match byte {
            NextByte::PercentDecoded(byte) => byte,
            NextByte::Literal(byte) => match byte {
                b'?' | b'#' => break,
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
                | b'/' => byte,
                _ => {
                    return PathCharacterInvalid.fail();
                },
            },
        };

        state = match (state, byte) {
            (State::AfterC2, 0x80..=0x9f) => {
                return ControlCharacter.fail();
            },
            (_, byte) if byte.is_ascii_control() => {
                return ControlCharacter.fail();
            },
            (_, b'\\') => {
                return Backslash.fail();
            },
            (State::TrailingDot, b'/') => {
                return TrailingDot.fail();
            },
            (_, b'.') => State::TrailingDot,
            (_, 0xc2) => State::AfterC2,
            _ => State::Path,
        };
    }

    if matches!(state, State::TrailingDot) {
        return TrailingDot.fail();
    }

    Ok(())
}

enum NextByte {
    Literal(u8),
    PercentDecoded(u8),
}

fn try_next_byte(bytes: &mut impl Iterator<Item = u8>) -> Result<Option<NextByte>, InvalidUrl> {
    let Some(byte) = bytes.next() else {
        return Ok(None);
    };
    if byte != b'%' {
        return Ok(Some(NextByte::Literal(byte)));
    }

    Ok(Some(NextByte::PercentDecoded(try_next_hex(bytes)?)))
}

fn try_next_hex(bytes: &mut impl Iterator<Item = u8>) -> Result<u8, InvalidUrl> {
    let high = try_next_octet(bytes)?;
    let low = try_next_octet(bytes)?;
    Ok((high << 4) | low)
}

fn try_next_octet(bytes: &mut impl Iterator<Item = u8>) -> Result<u8, InvalidUrl> {
    use invalid_url::*;

    let Some(byte) = bytes.next() else {
        return PercentEncodingIncomplete.fail();
    };
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => HexDigitInvalid.fail(),
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use rstest::rstest;

    use super::validate_blob_name;
    use crate::backend::az_blob_name_rejecter::error::InvalidUrl;

    #[rstest]
    #[case("")]
    #[case("/")]
    #[case("folder/file")]
    #[case("folder/.hidden")]
    #[case("folder/name.ext")]
    #[case("folder/a..b")]
    #[case("AZaz09-._~!$&'()*+,;=:@/file")]
    #[case("folder/%2Ehidden")]
    #[case("folder/%2fchild")]
    #[case("folder/%3F%23")]
    #[case("folder/%255C")]
    #[case("folder/%C2%A0")]
    #[case("folder/%c2%bf")]
    #[case("folder/%E2%82%AC")]
    #[case("folder/%5B1%5D.usd")]
    #[case("folder/%5B1%5D%2Eusd")]
    fn test_valid_blob_name(#[case] path: &str) {
        assert!(validate_blob_name(path.bytes()).is_ok(), "{path:?}");
    }

    #[rstest]
    fn test_ignores_query_and_fragment(
        #[values("", "folder/file")] path: &str,
        #[values("?%00", "#%C2%80")] suffix: &str,
    ) {
        let input = format!("{path}{suffix}");
        assert!(validate_blob_name(input.bytes()).is_ok(), "{input:?}");
    }

    #[rstest]
    #[case(".")]
    #[case("..")]
    #[case("...")]
    #[case("name.")]
    #[case("%2e")]
    #[case("%2E%2e")]
    #[case("name%2E")]
    #[case("name.%2e.")]
    fn test_rejects_trailing_dots(#[case] segment: &str, #[values("", "/child", "%2Fchild", "?query")] suffix: &str) {
        let path = format!("{segment}{suffix}");
        assert_matches!(
            validate_blob_name(path.bytes()),
            Err(InvalidUrl::TrailingDot { .. }),
            "{path:?}"
        );
    }

    #[rstest]
    fn test_rejects_literal_controls_and_spaces(
        #[values(
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10, 0x11,
            0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f, 0x20, 0x7f
        )]
        byte: u8,
        #[values("", "/child")] suffix: &str,
    ) {
        let path = format!("name{}suffix{suffix}", char::from(byte));
        assert_matches!(
            validate_blob_name(path.bytes()),
            Err(InvalidUrl::PathCharacterInvalid { .. }),
            "{path:?}"
        );
    }

    #[rstest]
    fn test_rejects_percent_encoded_controls(
        #[values(
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10, 0x11,
            0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f, 0x7f
        )]
        byte: u8,
        #[values("", "/child")] suffix: &str,
    ) {
        let path = format!("name%{byte:02X}suffix{suffix}");
        assert_matches!(
            validate_blob_name(path.bytes()),
            Err(InvalidUrl::ControlCharacter { .. }),
            "{path:?}"
        );
    }

    #[rstest]
    fn test_rejects_utf8_control_sequences(
        #[values(
            0x80, 0x81, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x8b, 0x8c, 0x8d, 0x8e, 0x8f, 0x90, 0x91,
            0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0x9b, 0x9c, 0x9d, 0x9e, 0x9f
        )]
        byte: u8,
        #[values("", "/child")] suffix: &str,
    ) {
        let path = format!("name%C2%{byte:02X}suffix{suffix}");
        assert_matches!(
            validate_blob_name(path.bytes()),
            Err(InvalidUrl::ControlCharacter { .. }),
            "{path:?}"
        );
    }

    #[rstest]
    fn test_rejects_literal_backslashes(#[values("", "/child")] suffix: &str) {
        let path = format!("name\\suffix{suffix}");
        assert_matches!(
            validate_blob_name(path.bytes()),
            Err(InvalidUrl::PathCharacterInvalid { .. }),
            "{path:?}"
        );
    }

    #[rstest]
    #[case("name%5Csuffix")]
    #[case("name%5csuffix")]
    fn test_rejects_percent_encoded_backslashes(#[case] segment: &str, #[values("", "/child")] suffix: &str) {
        let path = format!("{segment}{suffix}");
        assert_matches!(
            validate_blob_name(path.bytes()),
            Err(InvalidUrl::Backslash { .. }),
            "{path:?}"
        );
    }

    #[rstest]
    #[case("%")]
    #[case("%0")]
    fn test_rejects_incomplete_percent_sequences(#[case] sequence: &str) {
        let path = format!("parent/name{sequence}");
        assert_matches!(
            validate_blob_name(path.bytes()),
            Err(InvalidUrl::PercentEncodingIncomplete { .. })
        );
    }

    #[rstest]
    #[case("%gg")]
    #[case("%0G")]
    #[case("%G0")]
    #[case("%?")]
    #[case("%#")]
    fn test_rejects_invalid_percent_sequences(#[case] segment: &str, #[values("", "/child")] suffix: &str) {
        let path = format!("{segment}{suffix}");
        assert_matches!(
            validate_blob_name(path.bytes()),
            Err(InvalidUrl::HexDigitInvalid { .. }),
            "{path:?}"
        );
    }

    #[rstest]
    #[case("name%3F%00")]
    #[case("name%23%00")]
    fn test_encoded_delimiters_do_not_end_path(#[case] segment: &str, #[values("", "/child")] suffix: &str) {
        let path = format!("{segment}{suffix}");
        assert_matches!(
            validate_blob_name(path.bytes()),
            Err(InvalidUrl::ControlCharacter { .. }),
            "{path:?}"
        );
    }
}
