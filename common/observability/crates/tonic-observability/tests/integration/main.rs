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

mod bidi_streaming;
mod client_layer;
mod client_streaming;
pub(crate) mod harness;
mod rpc_map;
mod server_streaming;
mod unary;
mod weird;

// TODO: replace with `std::assert_matches` once stable.
// https://doc.rust-lang.org/std/assert_matches/macro.assert_matches.html
macro_rules! assert_matches {
    ($left:expr, $( $pattern:pat_param )|+ $(,)?) => {
        match $left {
            $( $pattern )|+ => {},
            ref left=> {
                let right = stringify!($( $pattern )|+);
                panic!(
                    r#"assertion `left matches right` failed:
  left: {left:?}
 right: {right:?}"#
                );
            }
        }
    };
    ($left:expr, $( $pattern:pat_param )|+, $($arg:tt)+) => {
        match $left {
            $( $pattern )|+ => {},
            ref left=> {
                let right = stringify!($( $pattern )|+);
                panic!(
                    r#"assertion `left matches right` failed: {}
  left: {left:?}
 right: {right:?}"#,
                    format_args!($($arg)+),
                );
            }
        }
    };
}
pub(crate) use assert_matches;
