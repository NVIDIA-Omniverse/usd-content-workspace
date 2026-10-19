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

use std::error::Error;

pub(crate) fn error_chain(e: &(dyn Error + 'static)) -> String {
    use std::fmt::Write as _;

    // the format here is similar to the unstable `std::error::Report`:
    // https://doc.rust-lang.org/std/error/struct.Report.html
    // note that `write!` on a string is infallible
    let mut s = String::new();
    write!(&mut s, "{e}").unwrap();

    let mut cause = e.source();
    if cause.is_some() {
        write!(&mut s, "\n\nCaused by:").unwrap();

        let mut i = 0usize;
        while let Some(e) = cause {
            write!(&mut s, "\n{i: >4}: {e}").unwrap();
            cause = e.source();
            i += 1;
        }
    }

    s
}

#[cfg(test)]
mod tests;
