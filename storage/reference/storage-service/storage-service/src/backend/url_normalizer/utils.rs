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

use crate::backend::url_normalizer::error::InvalidUri;

/// Normalize the path component in a resource URI.
///
/// Leaves the scheme, authority, query, and fragment components untouched. In the path component,
/// path-traversal segments are collapsed and empty segments are omitted.
///
/// The input string may be partially modified if normalization returns an error.
pub fn normalize_uri_path_in_place(uri: &mut String) -> Result<(), InvalidUri> {
    // SAFETY: Direct access to the String's byte buffer lets us rewrite it without allocating.
    // Writes never advance past consumed input: decoding shrinks three ASCII bytes to one,
    // and re-encoding uses at most those three bytes. Only ASCII bytes are written, and literal
    // non-ASCII input is rejected before it can be overwritten, including on error paths.
    // The untouched query and fragment retain their UTF-8 bytes, and the gap is drained only
    // at ASCII boundaries. The buffer therefore remains valid UTF-8 when this block exits.
    unsafe {
        let buffer = uri.as_mut_vec();
        let maybe_drain_range = {
            let mut context = SameBufferContext::new(buffer.as_mut_slice());
            continue_with_scheme(&mut context)?;

            context.drain_range()
        };

        if let Some((start, end)) = maybe_drain_range {
            buffer.drain(start..end);
        }

        debug_assert!(std::str::from_utf8(buffer).is_ok());
    }

    Ok(())
}

trait Context {
    fn next_byte(&mut self) -> Option<u8>;

    fn start_path(&mut self);

    fn push_byte(&mut self, value: u8);

    fn pop_segment(&mut self);
}

struct SameBufferContext<'a> {
    buffer: &'a mut [u8],
    read_pos: usize,
    write_pos: usize,
    path_start: usize,
}

impl<'a> SameBufferContext<'a> {
    fn new(buffer: &'a mut [u8]) -> Self {
        Self {
            read_pos: 0,
            write_pos: 0,
            path_start: 0,
            buffer,
        }
    }

    fn drain_range(self) -> Option<(usize, usize)> {
        if self.write_pos != self.read_pos {
            Some((self.write_pos, self.read_pos))
        } else {
            None
        }
    }
}

impl Context for SameBufferContext<'_> {
    fn next_byte(&mut self) -> Option<u8> {
        debug_assert!(self.write_pos >= self.path_start);
        debug_assert!(self.read_pos >= self.write_pos);
        debug_assert!(self.read_pos <= self.buffer.len());

        let byte = *self.buffer.get(self.read_pos)?;
        self.read_pos += 1;

        Some(byte)
    }

    fn start_path(&mut self) {
        debug_assert!(self.write_pos >= self.path_start);
        debug_assert!(self.read_pos >= self.write_pos);
        debug_assert!(self.read_pos <= self.buffer.len());

        self.path_start = self.write_pos;
    }

    fn push_byte(&mut self, value: u8) {
        debug_assert!(self.write_pos >= self.path_start);
        debug_assert!(self.read_pos >= self.write_pos);
        debug_assert!(self.read_pos <= self.buffer.len());

        self.buffer[self.write_pos] = value;
        self.write_pos += 1;
    }

    fn pop_segment(&mut self) {
        debug_assert!(self.write_pos >= self.path_start);
        debug_assert!(self.read_pos >= self.write_pos);
        debug_assert!(self.read_pos <= self.buffer.len());
        debug_assert!(self.write_pos > 0 && self.buffer[self.write_pos - 1] == b'/');

        if self.write_pos > self.path_start {
            self.write_pos -= 1;
            while self.write_pos > self.path_start && self.buffer[self.write_pos - 1] != b'/' {
                self.write_pos -= 1;
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Octet {
    None,
    Literal(u8),
    Decoded(u8),
    Separator(u8),
}

fn continue_with_scheme(context: &mut impl Context) -> Result<(), InvalidUri> {
    use crate::backend::url_normalizer::error::invalid_uri::*;

    enum State {
        First,
        Rest,
    }

    let mut state = State::First;
    loop {
        state = match state {
            State::First => match context.next_byte() {
                Some(byte @ (b'a'..=b'z' | b'A'..=b'Z')) => {
                    context.push_byte(byte);

                    State::Rest
                },
                Some(_) => return UnexpectedCharacter.fail(),
                None => return UnexpectedEndOfString.fail(),
            },
            State::Rest => match context.next_byte() {
                Some(b':') => return continue_with_scheme_separator(context),
                Some(byte @ (b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'+' | b'-' | b'.')) => {
                    context.push_byte(byte);

                    State::Rest
                },
                Some(_) => return UnexpectedCharacter.fail(),
                None => return UnexpectedEndOfString.fail(),
            },
        };
    }
}

fn continue_with_scheme_separator(context: &mut impl Context) -> Result<(), InvalidUri> {
    use crate::backend::url_normalizer::error::invalid_uri::*;

    enum State {
        FirstSlash,
        SecondSlash,
    }

    let mut state = State::FirstSlash;
    loop {
        state = match state {
            State::FirstSlash => match context.next_byte() {
                Some(b'/') => {
                    context.push_byte(b':');
                    context.push_byte(b'/');

                    State::SecondSlash
                },
                Some(_) => return UnexpectedCharacter.fail(),
                None => return UnexpectedEndOfString.fail(),
            },
            State::SecondSlash => match context.next_byte() {
                Some(b'/') => {
                    context.push_byte(b'/');

                    return continue_with_hostname(context);
                },
                Some(_) => return UnexpectedCharacter.fail(),
                None => return UnexpectedEndOfString.fail(),
            },
        };
    }
}

fn continue_with_hostname(context: &mut impl Context) -> Result<(), InvalidUri> {
    use crate::backend::url_normalizer::error::invalid_uri::*;

    enum State {
        FirstLabel,
        NextLabel,
        Label,
        Hyphen,
    }

    let mut state = State::FirstLabel;
    loop {
        state = match state {
            State::FirstLabel => match context.next_byte() {
                Some(byte) if byte.is_ascii_alphanumeric() => {
                    context.push_byte(byte);

                    State::Label
                },
                Some(_) => return UnexpectedCharacter.fail(),
                None => return UnexpectedEndOfString.fail(),
            },
            State::NextLabel => match context.next_byte() {
                Some(byte) if byte.is_ascii_alphanumeric() => {
                    context.push_byte(byte);

                    State::Label
                },
                Some(b':') => return continue_with_port(context),
                Some(b'/') => return continue_with_path(context),
                Some(byte @ (b'?' | b'#')) => {
                    context.push_byte(byte);

                    return Ok(());
                },
                None => return Ok(()),
                Some(_) => return UnexpectedCharacter.fail(),
            },
            State::Label => match context.next_byte() {
                Some(byte) if byte.is_ascii_alphanumeric() => {
                    context.push_byte(byte);

                    State::Label
                },
                Some(b'-') => {
                    context.push_byte(b'-');

                    State::Hyphen
                },
                Some(b'.') => {
                    context.push_byte(b'.');

                    State::NextLabel
                },
                Some(b':') => return continue_with_port(context),
                Some(b'/') => return continue_with_path(context),
                Some(byte @ (b'?' | b'#')) => {
                    context.push_byte(byte);

                    return Ok(());
                },
                None => return Ok(()),
                Some(_) => return UnexpectedCharacter.fail(),
            },
            State::Hyphen => match context.next_byte() {
                Some(byte) if byte.is_ascii_alphanumeric() => {
                    context.push_byte(byte);

                    State::Label
                },
                Some(b'-') => {
                    context.push_byte(b'-');

                    State::Hyphen
                },
                Some(_) => return UnexpectedCharacter.fail(),
                None => return UnexpectedEndOfString.fail(),
            },
        };
    }
}

fn continue_with_port(context: &mut impl Context) -> Result<(), InvalidUri> {
    use crate::backend::url_normalizer::error::invalid_uri::*;

    enum State {
        First,
        Rest,
    }

    let mut state = State::First;
    loop {
        state = match state {
            State::First => match context.next_byte() {
                Some(byte @ b'0'..=b'9') => {
                    context.push_byte(b':');
                    context.push_byte(byte);

                    State::Rest
                },
                Some(_) => return UnexpectedCharacter.fail(),
                None => return UnexpectedEndOfString.fail(),
            },
            State::Rest => match context.next_byte() {
                Some(byte @ b'0'..=b'9') => {
                    context.push_byte(byte);

                    State::Rest
                },
                Some(b'/') => return continue_with_path(context),
                Some(byte @ (b'?' | b'#')) => {
                    context.push_byte(byte);

                    return Ok(());
                },
                None => return Ok(()),
                Some(_) => return UnexpectedCharacter.fail(),
            },
        };
    }
}

fn continue_with_path(context: &mut impl Context) -> Result<(), InvalidUri> {
    enum State {
        SegmentStart,
        SingleDot,
        DoubleDot,
        Segment,
    }

    context.push_byte(b'/');
    context.start_path();

    let mut state = State::SegmentStart;
    loop {
        let octet = try_read_path_octet(context)?;
        state = match state {
            State::SegmentStart => match octet {
                Octet::Literal(b'/') | Octet::Decoded(b'/') => State::SegmentStart,
                Octet::Literal(b'.') | Octet::Decoded(b'.') => State::SingleDot,
                Octet::None | Octet::Separator(_) => State::SegmentStart,
                Octet::Literal(byte) | Octet::Decoded(byte) => {
                    push_path_byte(context, byte);

                    State::Segment
                },
            },
            State::SingleDot => match octet {
                Octet::Literal(b'.') | Octet::Decoded(b'.') => State::DoubleDot,
                Octet::None | Octet::Literal(b'/') | Octet::Decoded(b'/') | Octet::Separator(_) => State::SegmentStart,
                Octet::Literal(byte) | Octet::Decoded(byte) => {
                    context.push_byte(b'.');
                    push_path_byte(context, byte);

                    State::Segment
                },
            },
            State::DoubleDot => match octet {
                Octet::None | Octet::Literal(b'/') | Octet::Decoded(b'/') | Octet::Separator(_) => {
                    context.pop_segment();

                    State::SegmentStart
                },
                Octet::Literal(byte) | Octet::Decoded(byte) => {
                    context.push_byte(b'.');
                    context.push_byte(b'.');
                    push_path_byte(context, byte);

                    State::Segment
                },
            },
            State::Segment => match octet {
                Octet::Literal(b'/') | Octet::Decoded(b'/') => {
                    context.push_byte(b'/');

                    State::SegmentStart
                },
                Octet::None | Octet::Separator(_) => State::Segment,
                Octet::Literal(byte) | Octet::Decoded(byte) => {
                    push_path_byte(context, byte);

                    State::Segment
                },
            },
        };

        match octet {
            Octet::Separator(byte) => {
                context.push_byte(byte);

                return Ok(());
            },
            Octet::None => return Ok(()),
            _ => {},
        }
    }
}

fn try_read_hex_digit(context: &mut impl Context) -> Result<u8, InvalidUri> {
    use crate::backend::url_normalizer::error::invalid_uri::*;

    match context.next_byte() {
        Some(byte @ b'0'..=b'9') => Ok(byte - b'0'),
        Some(byte @ b'a'..=b'f') => Ok(byte - b'a' + 10),
        Some(byte @ b'A'..=b'F') => Ok(byte - b'A' + 10),
        Some(_) => UnexpectedCharacter.fail(),
        None => UnexpectedEndOfString.fail(),
    }
}

fn try_read_hex(context: &mut impl Context) -> Result<u8, InvalidUri> {
    let high = try_read_hex_digit(context)?;
    let low = try_read_hex_digit(context)?;

    Ok((high << 4) | low)
}

fn try_read_path_octet(context: &mut impl Context) -> Result<Octet, InvalidUri> {
    use crate::backend::url_normalizer::error::invalid_uri::UnexpectedCharacter;

    match context.next_byte() {
        Some(b'%') => Ok(Octet::Decoded(try_read_hex(context)?)),
        Some(b'?') => Ok(Octet::Separator(b'?')),
        Some(b'#') => Ok(Octet::Separator(b'#')),
        Some(byte) if is_path_safe_char(byte) => Ok(Octet::Literal(byte)),
        Some(_) => UnexpectedCharacter.fail(),
        None => Ok(Octet::None),
    }
}

fn push_path_byte(context: &mut impl Context, value: u8) {
    if is_path_safe_char(value) {
        context.push_byte(value);
    } else {
        const HEX: &[u8; 16] = b"0123456789ABCDEF";

        context.push_byte(b'%');
        context.push_byte(HEX[(value >> 4) as usize]);
        context.push_byte(HEX[(value & 0x0f) as usize]);
    }
}

fn is_path_safe_char(value: u8) -> bool {
    value.is_ascii_alphanumeric()
        || matches!(
            value,
            b'/' | b'-'
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
        )
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use rstest::rstest;

    use super::normalize_uri_path_in_place;
    use crate::backend::url_normalizer::error::InvalidUri;

    #[rstest]
    #[case::root("s3://host/")]
    #[case::file("s3://host/folder/file")]
    #[case::folder("s3://host/folder/")]
    #[case::case_is_preserved("S3://EXAMPLE.COM/Folder/File")]
    #[case::scheme_symbols("a+1.-://host/file")]
    #[case::hostname_hyphens("s3://a--b.example.com/file")]
    #[case::hostname_trailing_dot("s3://example.com./file")]
    #[case::safe_path_bytes("s3://host/AZaz09-._~!$&'()*+,;=:@/file")]
    #[case::query_and_fragment("s3://host/file?name=value#fragment")]
    fn test_uri_without_normalization_is_unchanged(#[case] uri: &str) {
        assert_eq!(normalize_uri_path(uri).expect("normalization failed"), uri);
    }

    #[rstest]
    #[case::only_dot("s3://host/.", "s3://host/")]
    #[case::only_dot_with_slash("s3://host/./", "s3://host/")]
    #[case::beginning("s3://host/./folder", "s3://host/folder")]
    #[case::beginning_with_slash("s3://host/./folder/", "s3://host/folder/")]
    #[case::end("s3://host/folder/.", "s3://host/folder/")]
    #[case::end_with_slash("s3://host/folder/./", "s3://host/folder/")]
    #[case::both("s3://host/./folder/.", "s3://host/folder/")]
    #[case::both_with_slash("s3://host/./folder/./", "s3://host/folder/")]
    #[case::middle("s3://host/folder/./file", "s3://host/folder/file")]
    fn test_single_dot_segments_are_collapsed(#[case] uri: &str, #[case] expected: &str) {
        assert_eq!(normalize_uri_path(uri).expect("normalization failed"), expected);
    }

    #[rstest]
    #[case::only_double_dot("s3://host/..", "s3://host/")]
    #[case::only_double_dot_with_slash("s3://host/../", "s3://host/")]
    #[case::beginning("s3://host/../folder", "s3://host/folder")]
    #[case::beginning_with_slash("s3://host/../folder/", "s3://host/folder/")]
    #[case::end("s3://host/folder/..", "s3://host/")]
    #[case::end_with_slash("s3://host/folder/../", "s3://host/")]
    #[case::both("s3://host/../folder/..", "s3://host/")]
    #[case::both_with_slash("s3://host/../folder/../", "s3://host/")]
    #[case::nested_end("s3://host/parent/folder/..", "s3://host/parent/")]
    #[case::nested_end_with_slash("s3://host/parent/folder/../", "s3://host/parent/")]
    #[case::middle("s3://host/parent/folder/../file", "s3://host/parent/file")]
    #[case::multiple_parents("s3://host/parent/folder/../../file", "s3://host/file")]
    #[case::above_root("s3://host/../../../file", "s3://host/file")]
    fn test_double_dot_segments_are_collapsed(#[case] uri: &str, #[case] expected: &str) {
        assert_eq!(normalize_uri_path(uri).expect("normalization failed"), expected);
    }

    #[rstest]
    #[case::three_dots("s3://host/...")]
    #[case::three_dots_with_slash("s3://host/.../")]
    #[case::leading_dot("s3://host/.name")]
    #[case::leading_dot_with_slash("s3://host/.name/")]
    #[case::trailing_dot("s3://host/name.")]
    #[case::trailing_dot_with_slash("s3://host/name./")]
    #[case::leading_double_dot("s3://host/..name")]
    #[case::trailing_double_dot("s3://host/name..")]
    #[case::multiple_segments("s3://host/.../.name/name./..name/name../file")]
    fn test_dots_in_segment_names_are_preserved(#[case] uri: &str) {
        assert_eq!(normalize_uri_path(uri).expect("normalization failed"), uri);
    }

    #[rstest]
    #[case::empty_query("s3://host/a//b?", "s3://host/a/b?")]
    #[case::empty_segments("s3://host/a//b?name=%2f/./..", "s3://host/a/b?name=%2f/./..")]
    #[case::single_dot("s3://host/a/.?name=./..//", "s3://host/a/?name=./..//")]
    #[case::double_dot("s3://host/a/..?name=./..//", "s3://host/?name=./..//")]
    #[case::encoded_separator("s3://host/a//b?name=%3F%23", "s3://host/a/b?name=%3F%23")]
    #[case::fragment_after_query("s3://host/a//b?name=value#./..//", "s3://host/a/b?name=value#./..//")]
    fn test_query_is_preserved_when_uri_is_shortened(#[case] uri: &str, #[case] expected: &str) {
        assert_eq!(normalize_uri_path(uri).expect("normalization failed"), expected);
    }

    #[rstest]
    #[case::empty_fragment("s3://host/a//b#", "s3://host/a/b#")]
    #[case::empty_segments("s3://host/a//b#name=%2f/./..", "s3://host/a/b#name=%2f/./..")]
    #[case::single_dot("s3://host/a/.#./..//", "s3://host/a/#./..//")]
    #[case::double_dot("s3://host/a/..#./..//", "s3://host/#./..//")]
    #[case::query_in_fragment("s3://host/a//b#name?value=%3F%23", "s3://host/a/b#name?value=%3F%23")]
    fn test_fragment_is_preserved_when_uri_is_shortened(#[case] uri: &str, #[case] expected: &str) {
        assert_eq!(normalize_uri_path(uri).expect("normalization failed"), expected);
    }

    #[rstest]
    #[case::root("s3://host///", "s3://host/")]
    #[case::beginning("s3://host///folder", "s3://host/folder")]
    #[case::middle("s3://host/parent///file", "s3://host/parent/file")]
    #[case::end("s3://host/folder///", "s3://host/folder/")]
    #[case::all_positions("s3://host///parent///file///", "s3://host/parent/file/")]
    fn test_empty_segments_are_collapsed(#[case] uri: &str, #[case] expected: &str) {
        assert_eq!(normalize_uri_path(uri).expect("normalization failed"), expected);
    }

    #[rstest]
    #[case::single_dots("s3://host///.//parent//.//file//.//", "s3://host/parent/file/")]
    #[case::double_dots("s3://host///..//parent//folder//..//file", "s3://host/parent/file")]
    #[case::mixed("s3://host///.//..//parent//.//folder//..//file//.//", "s3://host/parent/file/")]
    #[case::root("s3://host///.//..//.//..///", "s3://host/")]
    #[case::end("s3://host/parent///folder//.//..///", "s3://host/parent/")]
    fn test_empty_and_dot_segments_are_collapsed(#[case] uri: &str, #[case] expected: &str) {
        assert_eq!(normalize_uri_path(uri).expect("normalization failed"), expected);
    }

    #[rstest]
    #[case::without_port("s3://host/file")]
    #[case::with_port("s3://host:8080/file")]
    #[case::minimum_port("s3://host:1/file")]
    #[case::zero_port("s3://host:0/file")]
    #[case::leading_zero_port("s3://host:080/file")]
    #[case::all_zero_port("s3://host:000/file")]
    #[case::port_exceeds_u16("s3://host:12345678901234567890/file")]
    #[case::without_path("s3://host")]
    #[case::port_without_path("s3://host:8080")]
    #[case::query_without_path("s3://host?name=value")]
    #[case::port_query_without_path("s3://host:8080?name=value")]
    #[case::fragment_without_path("s3://host#fragment")]
    #[case::port_fragment_without_path("s3://host:8080#fragment")]
    fn test_uris_with_and_without_port_are_accepted(#[case] uri: &str) {
        assert_eq!(normalize_uri_path(uri).expect("normalization failed"), uri);
    }

    #[rstest]
    #[case::single_encoded_dot("s3://host/%2e/file", "s3://host/file")]
    #[case::double_encoded_dot("s3://host/parent/%2E%2e/file", "s3://host/file")]
    #[case::literal_then_encoded_dot("s3://host/parent/.%2e/file", "s3://host/file")]
    #[case::encoded_then_literal_dot("s3://host/parent/%2e./file", "s3://host/file")]
    #[case::unreserved("s3://host/%61%7e%41", "s3://host/a~A")]
    #[case::encoded_path_unsafe_bytes("s3://host/%20%25%3f%23%5b%5d", "s3://host/%20%25%3F%23%5B%5D")]
    #[case::encoded_utf8("s3://host/%c3%a9", "s3://host/%C3%A9")]
    fn test_percent_encoded_path_octets_are_normalized(#[case] uri: &str, #[case] expected: &str) {
        assert_eq!(normalize_uri_path(uri).expect("normalization failed"), expected);
    }

    #[rstest]
    #[case::digit_at_start("1s3://host/")]
    #[case::plus_at_start("+s3://host/")]
    #[case::dot_at_start(".s3://host/")]
    #[case::missing_scheme("://host/")]
    #[case::underscore("s_3://host/")]
    #[case::space("s 3://host/")]
    #[case::percent_encoding("s%33://host/")]
    #[case::slash("s3/://host/")]
    fn test_invalid_scheme_character_is_rejected(#[case] uri: &str) {
        assert_matches!(normalize_uri_path(uri), Err(InvalidUri::UnexpectedCharacter { .. }));
    }

    #[rstest]
    #[case::missing_first_slash("s3:host/")]
    #[case::missing_second_slash("s3:/host/")]
    #[case::extra_colon("s3:://host/")]
    #[case::backslash("s3:\\host/")]
    #[case::second_backslash("s3:/\\host/")]
    #[case::query("s3:?name=value")]
    #[case::fragment("s3:/#fragment")]
    fn test_invalid_scheme_separator_is_rejected(#[case] uri: &str) {
        assert_matches!(normalize_uri_path(uri), Err(InvalidUri::UnexpectedCharacter { .. }));
    }

    #[rstest]
    #[case::empty_hostname("s3:///")]
    #[case::leading_dot("s3://.host/")]
    #[case::empty_label("s3://host..example/")]
    #[case::leading_hyphen("s3://-host/")]
    #[case::trailing_hyphen("s3://host-/")]
    #[case::label_leading_hyphen("s3://host.-example/")]
    #[case::label_trailing_hyphen("s3://host-.example/")]
    #[case::underscore("s3://host_name/")]
    #[case::space("s3://host name/")]
    #[case::percent_encoding("s3://ho%73t/")]
    #[case::non_ascii("s3://h\u{e9}st/")]
    fn test_invalid_hostname_label_is_rejected(#[case] uri: &str) {
        assert_matches!(normalize_uri_path(uri), Err(InvalidUri::UnexpectedCharacter { .. }));
    }

    #[rstest]
    #[case::backslash("s3://host\\file")]
    #[case::semicolon("s3://host;file")]
    #[case::at_sign("s3://host@file")]
    #[case::bracket("s3://host[file")]
    #[case::pipe("s3://host|file")]
    fn test_invalid_hostname_separator_is_rejected(#[case] uri: &str) {
        assert_matches!(normalize_uri_path(uri), Err(InvalidUri::UnexpectedCharacter { .. }));
    }

    #[rstest]
    #[case::empty_port("s3://host:/")]
    #[case::letter_at_start("s3://host:a/")]
    #[case::letter_in_port("s3://host:8a/")]
    #[case::negative("s3://host:-80/")]
    #[case::plus("s3://host:+80/")]
    #[case::space("s3://host:8 0/")]
    #[case::percent_encoding("s3://host:%38%30/")]
    #[case::extra_colon("s3://host:80:/")]
    #[case::decimal_point("s3://host:8.0/")]
    fn test_invalid_port_character_is_rejected(#[case] uri: &str) {
        assert_matches!(normalize_uri_path(uri), Err(InvalidUri::UnexpectedCharacter { .. }));
    }

    #[rstest]
    #[case::space("s3://host/a b")]
    #[case::tab("s3://host/a\tb")]
    #[case::newline("s3://host/a\nb")]
    #[case::nul("s3://host/a\0b")]
    #[case::delete("s3://host/a\x7fb")]
    #[case::backslash("s3://host/a\\b")]
    #[case::opening_bracket("s3://host/a[b")]
    #[case::closing_bracket("s3://host/a]b")]
    #[case::opening_brace("s3://host/a{b")]
    #[case::pipe("s3://host/a|b")]
    #[case::quote("s3://host/a\"b")]
    #[case::non_ascii("s3://host/a\u{e9}b")]
    #[case::invalid_first_hex_digit("s3://host/%g0")]
    #[case::invalid_second_hex_digit("s3://host/%0g")]
    #[case::space_in_encoding("s3://host/% 0")]
    #[case::query_in_encoding("s3://host/%0?")]
    #[case::fragment_in_encoding("s3://host/%0#")]
    #[case::slash_in_encoding("s3://host/%0/")]
    fn test_invalid_path_character_is_rejected(#[case] uri: &str) {
        assert_matches!(normalize_uri_path(uri), Err(InvalidUri::UnexpectedCharacter { .. }));
    }

    #[rstest]
    #[case::empty_uri("")]
    #[case::scheme_without_separator("s3")]
    #[case::scheme_separator_without_slashes("s3:")]
    #[case::scheme_separator_with_one_slash("s3:/")]
    #[case::missing_hostname("s3://")]
    #[case::hostname_trailing_hyphen("s3://host-")]
    #[case::missing_port("s3://host:")]
    #[case::missing_hex_octet("s3://host/%")]
    #[case::incomplete_hex_octet("s3://host/%0")]
    fn test_unexpected_end_of_uri_is_rejected(#[case] uri: &str) {
        assert_matches!(normalize_uri_path(uri), Err(InvalidUri::UnexpectedEndOfString { .. }));
    }

    fn normalize_uri_path(uri: impl Into<String>) -> Result<String, InvalidUri> {
        let mut uri = uri.into();
        normalize_uri_path_in_place(&mut uri)?;

        Ok(uri)
    }
}
