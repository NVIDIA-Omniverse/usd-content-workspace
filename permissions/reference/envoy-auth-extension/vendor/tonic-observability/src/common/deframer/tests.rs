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

use super::{Deframer, GRPC_COMPRESSION_NONE, GRPC_COMPRESSION_SOME, State};

#[test]
fn empty_input() {
    let data = Payload::new();

    let mut d = Deframer::new();
    d.observe(&data);
    assert_eq!(d, data.complete());
}

#[test]
fn empty_frame() {
    let data = Payload::new().frame(b"");

    let mut d = Deframer::new();
    d.observe(&data);
    assert_eq!(d, data.complete());
}

#[test]
fn empty_byte_by_byte() {
    let data = Payload::new().frame(b"");

    let mut d = Deframer::new();
    for byte in &data.data {
        d.observe(&[*byte]);
    }
    assert_eq!(d, data.complete());
}

#[test]
fn uncompressed_single() {
    let data = Payload::new().frame(b"hello");

    let mut d = Deframer::new();
    d.observe(&data);
    assert_eq!(d, data.complete());
}

#[test]
fn compressed_single() {
    let data = Payload::new().compressed(b"hello");

    let mut d = Deframer::new();
    d.observe(&data);
    assert_eq!(d, data.complete());
}

#[test]
fn uncompressed_multiple() {
    let data = Payload::new().frame(b"aaa").frame(b"bb");

    let mut d = Deframer::new();
    d.observe(&data);
    assert_eq!(d, data.complete());
}

#[test]
fn uncompressed_byte_by_byte() {
    let data = Payload::new().frame(b"hello");

    let mut d = Deframer::new();
    for byte in &data.data {
        d.observe(&[*byte]);
    }
    assert_eq!(d, data.complete());
}

#[test]
fn invalid_frame() {
    let mut d = Deframer::new();

    let invalid_data = &[0x02, 0xAA, 0xBB, 0xCC];
    d.observe(invalid_data);
    assert_eq!(
        d,
        Deframer {
            state: State::Invalid,
            len: invalid_data.len(),
        }
    );

    // check that the deframer remains in an invalid state, as it's impossible
    // to know that a new valid frame was started
    let more_data = Payload::new().frame(b"hello");
    d.observe(&more_data);
    assert_eq!(
        d,
        Deframer {
            state: State::Invalid,
            len: invalid_data.len() + more_data.len(),
        }
    );
}

#[test]
fn state_length_partial() {
    const DATA: &[u8] = &[0, 0, 0, 0, 1];

    for (data, rem) in [
        (&DATA[..1], 4),
        (&DATA[..2], 3),
        (&DATA[..3], 2),
        (&DATA[..4], 1),
    ] {
        let mut d = Deframer::new();
        d.observe(data);
        assert_eq!(
            d,
            Deframer {
                state: State::Length { buf: 0, rem },
                len: 0,
            }
        );
    }

    let mut d = Deframer::new();
    d.observe(DATA);
    assert_eq!(
        d,
        Deframer {
            state: State::Payload { rem: 1 },
            len: 0,
        }
    );
}

#[test]
fn state_payload_partial() {
    const DATA: &[u8] = &[0, 0, 0, 0, 2, 0];

    let mut d = Deframer::new();
    d.observe(DATA);
    assert_eq!(
        d,
        Deframer {
            state: State::Payload { rem: 1 },
            len: 1,
        }
    );
}

struct Payload {
    data: Vec<u8>,
    len: usize,
}

impl Payload {
    fn new() -> Self {
        Self {
            data: Vec::new(),
            len: 0,
        }
    }

    fn frame(mut self, payload: &[u8]) -> Self {
        let len = payload.len();
        self.len += len;
        let len = (len as u32).to_be_bytes();
        self.data.push(GRPC_COMPRESSION_NONE);
        self.data.extend_from_slice(&len);
        self.data.extend_from_slice(payload);
        self
    }

    fn compressed(mut self, payload: &[u8]) -> Self {
        let len = payload.len();
        self.len += len;
        let len = (len as u32).to_be_bytes();
        self.data.push(GRPC_COMPRESSION_SOME);
        self.data.extend_from_slice(&len);
        self.data.extend_from_slice(payload);
        self
    }

    fn complete(&self) -> Deframer {
        Deframer {
            state: State::Compression,
            len: self.len,
        }
    }
}

impl std::ops::Deref for Payload {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.data
    }
}
