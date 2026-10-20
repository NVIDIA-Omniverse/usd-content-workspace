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

/// Set the environment to the specified keys only.
///
/// # Safety
///
/// On Unix for some libc implementations, `set_env`/`remove_env` is only safe
/// in single-threaded programs. `cargo nextest` provides per-test isolation, so
/// these tests are safe only when run via `cargo nextest` and not `cargo test`:
/// https://nexte.st/docs/configuration/env-vars/#altering-the-environment-within-tests
pub(crate) unsafe fn set_env(vars: &[(&str, &str)]) {
    for (key, _value) in std::env::vars() {
        unsafe {
            std::env::remove_var(&key);
        }
    }
    for (key, value) in vars.iter().copied() {
        unsafe {
            std::env::set_var(key, value);
        }
    }
    for (key, value) in std::env::vars() {
        println!("`{key}`: `{value}`");
    }
}

macro_rules! unsafe_env {
    ($($k:tt : $v:expr),* $(,)?) => {
        unsafe {
            $crate::testing::set_env(&[
                $((&$k as &str, $v)),*
            ])
        }
    };
}
pub(crate) use unsafe_env;
