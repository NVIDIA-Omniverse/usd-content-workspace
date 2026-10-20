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

use super::error_chain;
use std::error::Error;
use std::fmt;

#[derive(Debug)]
struct E {
    msg: &'static str,
    source: Option<Box<E>>,
}

impl E {
    fn new(msg: &'static str) -> Self {
        Self { msg, source: None }
    }

    fn source(self, e: E) -> Self {
        let Self { msg, source: _ } = self;
        Self {
            msg,
            source: Some(Box::new(e)),
        }
    }
}

impl fmt::Display for E {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.msg)
    }
}

impl Error for E {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_ref()
            .map(|src| src as &(dyn Error + 'static))
    }
}

#[test]
fn chain_depth_0() {
    let e = E::new("foo");
    let s = error_chain(&e);
    assert_eq!(s, "foo");
}

#[test]
fn chain_depth_1() {
    let e = E::new("foo").source(E::new("bar"));
    let s = error_chain(&e);
    assert_eq!(
        s,
        r#"foo

Caused by:
   0: bar"#
    );
}

#[test]
fn chain_depth_2() {
    let e = E::new("foo").source(E::new("bar").source(E::new("baz")));
    let s = error_chain(&e);
    assert_eq!(
        s,
        r#"foo

Caused by:
   0: bar
   1: baz"#
    );
}
