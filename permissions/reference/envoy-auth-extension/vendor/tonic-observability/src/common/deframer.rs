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

//! gRPC deframing logic.
//!
//! For metrics, we need to count the unframed payload bytes.

const GRPC_COMPRESSION_NONE: u8 = 0;
const GRPC_COMPRESSION_SOME: u8 = 1;

/// Deframer state.
#[derive(Debug, PartialEq)]
enum State {
    /// Expecting a compression flag (initial state).
    Compression,
    /// Collecting a message length.
    Length { buf: u32, rem: u8 },
    /// Consuming payload bytes.
    Payload { rem: u32 },
    /// Invalid framing detected; count all subsequent bytes as payload.
    Invalid,
}

impl State {
    const fn length() -> Self {
        Self::Length { buf: 0, rem: 4 }
    }

    const fn payload(rem: u32) -> Self {
        if rem > 0 {
            Self::Payload { rem }
        } else {
            Self::Compression
        }
    }

    const fn invalid() -> Self {
        Self::Invalid
    }
}

/// Tracks the payload size of gRPC messages flowing through a body stream,
/// excluding the 5-byte framing header (1 byte compression flag + 4 byte
/// length) per message.
///
/// If the stream contains invalid gRPC framing, falls back to raw byte
/// counting for the remainder of the stream.
#[derive(Debug, PartialEq)]
pub(crate) struct Deframer {
    state: State,
    len: usize,
}

impl Deframer {
    pub(crate) const fn new() -> Self {
        Self {
            state: State::Compression,
            len: 0,
        }
    }

    pub(crate) fn observe(&mut self, mut data: &[u8]) {
        while !data.is_empty() {
            match &mut self.state {
                State::Compression => {
                    // always have at least one byte
                    let byte = data[0];
                    data = &data[1..];

                    match byte {
                        GRPC_COMPRESSION_NONE | GRPC_COMPRESSION_SOME => {
                            // optimization: if we have the length bytes,
                            // process them all at once
                            match data.split_first_chunk::<4>() {
                                Some((bytes, remainder)) => {
                                    data = remainder;
                                    let rem = u32::from_be_bytes(*bytes);
                                    self.state = State::payload(rem);
                                }
                                None => {
                                    self.state = State::length();
                                }
                            }
                        }
                        _ => {
                            self.len = self.len.saturating_add(1);
                            self.state = State::invalid();
                        }
                    }
                }
                State::Length { buf, rem } => {
                    // always have at least one byte
                    let byte = data[0];
                    data = &data[1..];

                    // shift length bytes in big-endian format
                    *buf = ((*buf) << 8) | (byte as u32);
                    *rem -= 1;

                    if *rem == 0 {
                        self.state = State::payload(*buf);
                    }
                }
                State::Payload { rem } => {
                    let take = std::cmp::min(*rem as usize, data.len());

                    self.len = self.len.saturating_add(take);
                    // `take` cannot be > u32 due to `min`, so the cast is safe
                    *rem -= take as u32;
                    data = &data[take..];

                    // message complete
                    if *rem == 0 {
                        self.state = State::Compression;
                    }
                }
                State::Invalid => {
                    // not valid gRPC; count all remaining bytes as payload
                    self.len = self.len.saturating_add(data.len());
                    return;
                }
            }
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.len
    }
}

impl Default for Deframer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
