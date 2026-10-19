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

use super::{BOOL_ERR, OtelEnvVal, Var};
use crate::testing::assert_matches;

macro_rules! var {
    ($val:expr) => {{
        let val: &str = $val;
        Var {
            key: "foo",
            val: val.to_string(),
        }
    }};
}

#[test]
fn bool_from_env() {
    const VALUES: &[(&str, Result<bool, &str>)] = &[
        ("true", Ok(true)),
        ("TRUE", Ok(true)),
        ("True", Ok(true)),
        ("tRuE", Ok(true)),
        ("false", Ok(false)),
        ("FALSE", Ok(false)),
        ("False", Ok(false)),
        ("fAlSe", Ok(false)),
        ("invalid", Err(BOOL_ERR)),
        ("y", Err(BOOL_ERR)),
        ("yes", Err(BOOL_ERR)),
        ("n", Err(BOOL_ERR)),
        ("no", Err(BOOL_ERR)),
        ("0", Err(BOOL_ERR)),
        ("1", Err(BOOL_ERR)),
        ("", Err(BOOL_ERR)),
    ];

    for (s, expected) in VALUES.iter().copied() {
        let actual: Result<bool, _> = OtelEnvVal::from_env(s);
        assert_eq!(actual, expected, "value `{s}`");
    }
}

#[test]
fn u32_from_env_ok() {
    const VALUES: &[(&str, u32)] = &[("0", 0), ("1", 1), ("42", 42), ("4294967295", u32::MAX)];

    for (s, expected) in VALUES.iter().copied() {
        let actual: Result<u32, _> = OtelEnvVal::from_env(s);
        assert_eq!(actual, Ok(expected), "value `{s}`");
    }
}

#[test]
fn u32_from_env_err() {
    const VALUES: &[&str] = &["", "-1", "0xFF", "a"];

    for s in VALUES.iter().copied() {
        let actual: Result<u32, _> = OtelEnvVal::from_env(s);
        assert_matches!(actual, Err(_), "value `{}`", s);
    }
}

#[test]
fn u64_from_env_ok() {
    const VALUES: &[(&str, u64)] = &[
        ("0", 0),
        ("1", 1),
        ("42", 42),
        ("18446744073709551615", u64::MAX),
    ];

    for (s, expected) in VALUES.iter().copied() {
        let actual: Result<u64, _> = OtelEnvVal::from_env(s);
        assert_eq!(actual, Ok(expected), "value `{s}`");
    }
}

#[test]
fn u64_from_env_err() {
    const VALUES: &[&str] = &["", "-1", "0xFF", "a"];

    for s in VALUES.iter().copied() {
        let actual: Result<u64, _> = OtelEnvVal::from_env(s);
        assert_matches!(actual, Err(_), "value `{}`", s);
    }
}

#[test]
fn duration_from_env() {
    const VALUES: &[(&str, u64)] = &[("0", 0), ("1", 1), ("42", 42)];

    for (s, millis) in VALUES.iter().copied() {
        let expected = std::time::Duration::from_millis(millis);
        let actual = OtelEnvVal::from_env(s);
        assert_eq!(actual, Ok(expected), "value `{s}`");
    }
}

#[test]
fn var_get_valid() {
    let expected = var!("bar");
    // SAFETY: only safe due to `cargo nextest`/per-process isolation
    unsafe {
        std::env::set_var(expected.key, &expected.val);
    }
    let actual = Var::get(expected.key);
    assert_eq!(actual, Some(expected));
}

#[test]
fn var_get_missing() {
    // SAFETY: only safe due to `cargo nextest`/per-process isolation
    unsafe {
        std::env::remove_var("foo");
    }
    let actual = Var::get("foo");
    assert_eq!(actual, None);
}

#[cfg(unix)]
#[test]
fn var_get_invalid() {
    use std::os::unix::ffi::OsStrExt;
    let value = std::ffi::OsStr::from_bytes(&[0xE0u8, 0x80, 0x80]);
    // SAFETY: only safe due to `cargo nextest`/per-process isolation
    unsafe {
        std::env::set_var("foo", value);
    }
    let actual = Var::get("foo");
    assert_eq!(actual, None);
}

#[test]
fn var_parse_ok() {
    let var = var!("true");
    let actual: Option<bool> = var.parse();
    assert_eq!(actual, Some(true), "value: `{}`", var.val);
}

#[test]
fn var_parse_err() {
    let var = var!("0");
    let actual: Option<bool> = var.parse();
    assert_eq!(actual, None, "value: `{}`", var.val);
}
