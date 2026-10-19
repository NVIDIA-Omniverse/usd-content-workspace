// SPDX-FileCopyrightText: Copyright (c) 2024-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
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

use super::{
    TraceParent, TraceParentError, TraceParentErrorInner, lower_hex_byte, lower_hex_digit,
};
use opentelemetry::trace::{SpanId, TraceFlags, TraceId};
use std::collections::HashMap;

#[test]
fn lower_hex_digit_all() {
    let lower_hex = {
        let mut map = HashMap::<u8, u8>::new();
        map.insert(b'0', 0x0);
        map.insert(b'1', 0x1);
        map.insert(b'2', 0x2);
        map.insert(b'3', 0x3);
        map.insert(b'4', 0x4);
        map.insert(b'5', 0x5);
        map.insert(b'6', 0x6);
        map.insert(b'7', 0x7);
        map.insert(b'8', 0x8);
        map.insert(b'9', 0x9);
        map.insert(b'a', 0xa);
        map.insert(b'b', 0xb);
        map.insert(b'c', 0xc);
        map.insert(b'd', 0xd);
        map.insert(b'e', 0xe);
        map.insert(b'f', 0xf);
        assert_eq!(map.len(), 16);
        map
    };

    for b in u8::MIN..=u8::MAX {
        let expected = lower_hex.get(&b).copied();
        let actual = lower_hex_digit(b);
        assert_eq!(expected, actual, "b: {b:02x}");
    }
}

fn lower_hex_byte(b: &[u8]) -> Result<u8, TraceParentError> {
    Ok(lower_hex_byte!(b, 0))
}

#[test]
fn lower_hex_byte_valid() {
    for (b, expected) in [
        // valid
        (b"00", Ok(0x00)),
        (b"10", Ok(0x10)),
        (b"01", Ok(0x01)),
        (b"90", Ok(0x90)),
        (b"09", Ok(0x09)),
        (b"a0", Ok(0xa0)),
        (b"0a", Ok(0x0a)),
        (b"f0", Ok(0xf0)),
        (b"0f", Ok(0x0f)),
        (b"ff", Ok(0xff)),
        // invalid
        (b"A0", Err(TraceParentError::not_hex(0))),
        (b"0A", Err(TraceParentError::not_hex(1))),
        (b"F0", Err(TraceParentError::not_hex(0))),
        (b"0F", Err(TraceParentError::not_hex(1))),
        (b"FF", Err(TraceParentError::not_hex(0))),
        (b"X0", Err(TraceParentError::not_hex(0))),
        (b"0X", Err(TraceParentError::not_hex(1))),
        (b"XX", Err(TraceParentError::not_hex(0))),
    ] {
        let actual = lower_hex_byte(b);
        assert_eq!(expected, actual, "b: {}", b.escape_ascii());
    }
}

#[test]
fn trace_parent_header_valid_version_curr() {
    const HEADER: &[u8; 55] = b"00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";
    const EXPECTED: TraceParent = TraceParent {
        trace_id: TraceId::from_bytes([
            0x0a, 0xf7, 0x65, 0x19, 0x16, 0xcd, 0x43, 0xdd, 0x84, 0x48, 0xeb, 0x21, 0x1c, 0x80,
            0x31, 0x9c,
        ]),
        parent_id: SpanId::from_bytes([0xb7, 0xad, 0x6b, 0x71, 0x69, 0x20, 0x33, 0x31]),
        trace_flags: TraceFlags::SAMPLED,
    };

    let actual = TraceParent::from_header(HEADER).unwrap();
    assert_eq!(EXPECTED, actual);
}

#[test]
fn trace_parent_header_valid_version_fut_short() {
    const EXPECTED: TraceParent = TraceParent {
        trace_id: TraceId::from_bytes([
            0x0a, 0xf7, 0x65, 0x19, 0x16, 0xcd, 0x43, 0xdd, 0x84, 0x48, 0xeb, 0x21, 0x1c, 0x80,
            0x31, 0x9c,
        ]),
        parent_id: SpanId::from_bytes([0xb7, 0xad, 0x6b, 0x71, 0x69, 0x20, 0x33, 0x31]),
        trace_flags: TraceFlags::SAMPLED,
    };

    let header: &[u8; 55] = b"fe-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";
    let actual = TraceParent::from_header(header).unwrap();
    assert_eq!(EXPECTED, actual);

    let header: &[u8; 55] = b"fe-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-ff";
    let actual = TraceParent::from_header(header).unwrap();
    assert_eq!(EXPECTED, actual);
}

#[test]
fn trace_parent_header_valid_version_fut_long() {
    const EXPECTED: TraceParent = TraceParent {
        trace_id: TraceId::from_bytes([
            0x0a, 0xf7, 0x65, 0x19, 0x16, 0xcd, 0x43, 0xdd, 0x84, 0x48, 0xeb, 0x21, 0x1c, 0x80,
            0x31, 0x9c,
        ]),
        parent_id: SpanId::from_bytes([0xb7, 0xad, 0x6b, 0x71, 0x69, 0x20, 0x33, 0x31]),
        trace_flags: TraceFlags::SAMPLED,
    };

    let header: &[u8] = b"fe-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01-more";
    let actual = TraceParent::from_header(header).unwrap();
    assert_eq!(EXPECTED, actual);

    let header: &[u8] = b"fe-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-ff-more";
    let actual = TraceParent::from_header(header).unwrap();
    assert_eq!(EXPECTED, actual);
}

macro_rules! assert_err {
    ($res:ident, $err:expr, $header:ident) => {
        assert_eq!(
            $res,
            Err(TraceParentError($err)),
            "header: {}",
            $header.escape_ascii()
        );
    };
}

#[test]
fn trace_parent_header_invalid_len_short() {
    const HEADER: &[u8; 55] = b"ff-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";
    for i in 0..HEADER.len() {
        let header = &HEADER[..i];
        let res = TraceParent::from_header(header);
        assert_err!(res, TraceParentErrorInner::TooShort, header);
    }
}

#[test]
fn trace_parent_header_invalid_len_long() {
    const HEADER: &[u8; 55 + 1] = b"00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01-";
    let res = TraceParent::from_header(HEADER);
    assert_err!(res, TraceParentErrorInner::TooLong, HEADER);
}

#[test]
fn trace_parent_header_invalid_version_hex() {
    let header: &[u8; 55] = b"Ff-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";
    let res = TraceParent::from_header(header);
    assert_err!(res, TraceParentErrorInner::NotHex(0), header);

    let header: &[u8; 55] = b"fF-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";
    let res = TraceParent::from_header(header);
    assert_err!(res, TraceParentErrorInner::NotHex(1), header);

    let header: &[u8; 55] = b"XX-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";
    let res = TraceParent::from_header(header);
    assert_err!(res, TraceParentErrorInner::NotHex(0), header);
}

#[test]
fn trace_parent_header_invalid_version_dash() {
    const HEADER: &[u8; 55] = b"ff_0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";
    let res = TraceParent::from_header(HEADER);
    assert_err!(res, TraceParentErrorInner::NotDelim(2), HEADER);
}

#[test]
fn trace_parent_header_invalid_version_num() {
    const HEADER: &[u8; 55] = b"ff-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";
    let res = TraceParent::from_header(HEADER);
    assert_err!(res, TraceParentErrorInner::InvalidVersion, HEADER);
}

#[test]
fn trace_parent_header_invalid_trace_id_hex() {
    let header: &[u8; 55] = b"00-Faf7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";
    let res = TraceParent::from_header(header);
    assert_err!(res, TraceParentErrorInner::NotHex(3), header);

    let header: &[u8; 55] = b"00-0af7651916cd43dd8448eb211c80319F-b7ad6b7169203331-01";
    let res = TraceParent::from_header(header);
    assert_err!(res, TraceParentErrorInner::NotHex(34), header);

    let header: &[u8; 55] = b"00-Xaf7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";
    let res = TraceParent::from_header(header);
    assert_err!(res, TraceParentErrorInner::NotHex(3), header);

    let header: &[u8; 55] = b"00-0af7651916cd43dd8448eb211c80319X-b7ad6b7169203331-01";
    let res = TraceParent::from_header(header);
    assert_err!(res, TraceParentErrorInner::NotHex(34), header);
}

#[test]
fn trace_parent_header_invalid_trace_id_dash() {
    const HEADER: &[u8; 55] = b"00-0af7651916cd43dd8448eb211c80319c_b7ad6b7169203331-01";
    let res = TraceParent::from_header(HEADER);
    assert_err!(res, TraceParentErrorInner::NotDelim(35), HEADER);
}

#[test]
fn trace_parent_header_invalid_trace_id_zero() {
    const HEADER: &[u8; 55] = b"00-00000000000000000000000000000000-b7ad6b7169203331-01";
    let res = TraceParent::from_header(HEADER);
    assert_err!(res, TraceParentErrorInner::TraceId, HEADER);
}

#[test]
fn trace_parent_header_invalid_parent_id_hex() {
    let header: &[u8; 55] = b"00-0af7651916cd43dd8448eb211c80319c-F7ad6b7169203331-01";
    let res = TraceParent::from_header(header);
    assert_err!(res, TraceParentErrorInner::NotHex(36), header);

    let header: &[u8; 55] = b"00-0af7651916cd43dd8448eb211c80319c-b7ad6b716920333F-01";
    let res = TraceParent::from_header(header);
    assert_err!(res, TraceParentErrorInner::NotHex(51), header);

    let header: &[u8; 55] = b"00-0af7651916cd43dd8448eb211c80319c-X7ad6b7169203331-01";
    let res = TraceParent::from_header(header);
    assert_err!(res, TraceParentErrorInner::NotHex(36), header);

    let header: &[u8; 55] = b"00-0af7651916cd43dd8448eb211c803190-b7ad6b716920333X-01";
    let res = TraceParent::from_header(header);
    assert_err!(res, TraceParentErrorInner::NotHex(51), header);
}

#[test]
fn trace_parent_header_invalid_parent_id_dash() {
    const HEADER: &[u8; 55] = b"00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331_01";
    let res = TraceParent::from_header(HEADER);
    assert_err!(res, TraceParentErrorInner::NotDelim(52), HEADER);
}

#[test]
fn trace_parent_header_invalid_parent_id_zero() {
    const HEADER: &[u8; 55] = b"00-0af7651916cd43dd8448eb211c803190-0000000000000000-01";
    let res = TraceParent::from_header(HEADER);
    assert_err!(res, TraceParentErrorInner::ParentId, HEADER);
}

#[test]
fn trace_parent_header_invalid_flags_hex() {
    let header: &[u8; 55] = b"00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-F1";
    let res = TraceParent::from_header(header);
    assert_err!(res, TraceParentErrorInner::NotHex(53), header);

    let header: &[u8; 55] = b"00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-0F";
    let res = TraceParent::from_header(header);
    assert_err!(res, TraceParentErrorInner::NotHex(54), header);

    let header: &[u8; 55] = b"00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-X1";
    let res = TraceParent::from_header(header);
    assert_err!(res, TraceParentErrorInner::NotHex(53), header);

    let header: &[u8; 55] = b"00-0af7651916cd43dd8448eb211c803190-b7ad6b7169203331-0X";
    let res = TraceParent::from_header(header);
    assert_err!(res, TraceParentErrorInner::NotHex(54), header);
}

#[test]
fn trace_parent_header_invalid_flags_dash() {
    const HEADER: &[u8; 56] = b"aa-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01X";
    let res = TraceParent::from_header(HEADER);
    assert_err!(res, TraceParentErrorInner::NotDelim(55), HEADER);
}

macro_rules! assert_display {
    ($e:expr, $expected:literal) => {{
        let e: TraceParentError = $e;
        let actual = format!("{}", e);
        assert_eq!($expected, actual, "{:?}", e);
    }};
}

#[test]
fn trace_parent_error_display() {
    assert_display!(TraceParentError::too_short(), "value is too short");
    assert_display!(TraceParentError::too_long(), "value is too long");
    assert_display!(
        TraceParentError::not_hex(42),
        "expected a hexadecimal value at byte 42"
    );
    assert_display!(
        TraceParentError::not_delim(42),
        "expected a delimiter at byte 42"
    );
    assert_display!(TraceParentError::invalid_version(), "version is invalid");
    assert_display!(TraceParentError::trace_id(), "trace ID is invalid");
    assert_display!(TraceParentError::parent_id(), "parent ID is invalid");
}
