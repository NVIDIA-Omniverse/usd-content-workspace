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

use super::FilterDirectives;

#[test]
fn filter_directives_new_valid() {
    let fd = FilterDirectives::new("info");
    assert_eq!(
        fd,
        FilterDirectives {
            dirs: "info".to_string(),
            valid: true,
        }
    )
}

#[test]
fn filter_directives_new_empty() {
    let fd = FilterDirectives::new("");
    assert_eq!(
        fd,
        FilterDirectives {
            dirs: "".to_string(),
            valid: false,
        }
    )
}

#[test]
fn filter_directives_new_invalid() {
    let fd = FilterDirectives::new("[");
    assert_eq!(
        fd,
        FilterDirectives {
            dirs: "[".to_string(),
            valid: false,
        }
    )
}
