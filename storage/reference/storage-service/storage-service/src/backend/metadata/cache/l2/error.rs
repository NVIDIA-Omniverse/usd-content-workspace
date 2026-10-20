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

use fred::types::ValueKind;
use snafu::{Location, Snafu};

#[derive(Snafu, Debug)]
#[snafu(visibility(pub), module, context(suffix(false)))]
pub enum Error {
    #[snafu(display("failed to execute a command"))]
    Command {
        source: fred::error::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("unexpected value kind ({kind:?}) in the command result"))]
    UnexpectedValueKind {
        kind: ValueKind,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to encode a value"))]
    ValueEncode {
        source: serde_json::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to decode a value"))]
    ValueDecode {
        source: serde_json::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to initialize a subscriber"))]
    SubscriberInit {
        source: fred::error::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to decode a key"))]
    KeyDecode {
        source: KeyDecodeError,
        #[snafu(provide, implicit)]
        location: Location,
    },
}

#[derive(Snafu, Debug)]
#[snafu(visibility(pub), module, context(suffix(false)))]
pub enum KeyDecodeError {
    #[snafu(display("unexpected key prefix"))]
    UnexpectedPrefix {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("buffer is too short"))]
    BufferTooShort {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("buffer is not exhausted after reading"))]
    BufferNotExhausted {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("value is not a utf-8 encoded string"))]
    Utf8Decode {
        source: std::str::Utf8Error,
        #[snafu(provide, implicit)]
        location: Location,
    },
}
