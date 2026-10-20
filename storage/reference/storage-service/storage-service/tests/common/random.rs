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

use bytes::Bytes;
use rand::{
    seq::{IndexedRandom, SliceRandom},
    Rng,
};

pub const PATH_SEGMENT_MANDATORY: &[u8] = b"!-_~";
pub const PATH_SEGMENT_OPTIONAL: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// Get a random alphanumeric string.
pub fn get_random_alphanumeric_string(length: usize) -> String {
    let chars: Vec<u8> = PATH_SEGMENT_OPTIONAL
        .choose_multiple(&mut rand::rng(), length)
        .cloned()
        .collect();

    String::from_utf8(chars).expect("failed to create a sequencer")
}

/// Get a number of random bytes.
pub fn get_random_bytes(len: usize) -> Bytes {
    Bytes::from_iter(rand::rng().random_iter().take(len))
}

/// Create a random path, non url-escaped.
pub fn get_random_path() -> String {
    format!("{}/{}", get_random_path_item(), get_random_path_item())
}

/// Create a random path item value, non url-escaped.
pub fn get_random_path_item() -> String {
    let mut rng = rand::rng();
    let mut chars: Vec<u8> = PATH_SEGMENT_OPTIONAL
        .choose_multiple(&mut rng, 24)
        .cloned()
        .chain(PATH_SEGMENT_MANDATORY.iter().cloned())
        .collect();
    chars.shuffle(&mut rng);

    String::from_utf8(chars).expect("failed to create a path segment")
}
