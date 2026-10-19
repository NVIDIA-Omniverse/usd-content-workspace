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

use super::{EnvFilterSource, env_filter_opt, env_filter_req};
use crate::testing::assert_matches;

#[test]
fn env_filter_opt_valid() {
    let filter = env_filter_opt("test", "info");
    assert_matches!(filter, Some(_));
}

#[test]
fn env_filter_opt_empty() {
    let filter = env_filter_opt("test", "");
    assert_matches!(filter, None);
}

#[test]
fn env_filter_opt_invalid() {
    let filter = env_filter_opt("test", "[");
    assert_matches!(filter, None);
}

#[test]
fn env_filter_req_valid() {
    env_filter_req("test", "info");
}

#[test]
#[should_panic(expected = "test filter directives is empty (did you mean `off`?)")]
fn env_filter_req_empty() {
    env_filter_req("test", "");
}

#[test]
#[should_panic(expected = "test filter directives `[` are invalid: invalid filter directive")]
fn env_filter_req_invalid() {
    env_filter_req("test", "[");
}

#[test]
fn env_filter_source_dirs_valid() {
    EnvFilterSource {
        dirs: Some("info"),
        src: "test",
    }
    .parse("[");
}

#[test]
fn env_filter_source_dirs_invalid() {
    EnvFilterSource {
        dirs: Some("["),
        src: "test",
    }
    .parse("info");
}

#[test]
fn env_filter_source_dirs_default() {
    EnvFilterSource {
        dirs: None,
        src: "test",
    }
    .parse("info");
}
