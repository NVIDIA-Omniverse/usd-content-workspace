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

use http::uri::{InvalidUri, Uri};
use std::time::Duration;

/// Conversions from env var values.
pub(crate) trait OtelEnvVal: Sized + 'static {
    type Err: std::fmt::Display;

    fn from_env(s: &str) -> Result<Self, Self::Err>;
}

const BOOL_ERR: &str = "expected `true` or `false` (case-insensitive)";

/// Parse the env variable per [Otel boolean value] spec.
///
/// [Otel boolean value]: https://opentelemetry.io/docs/specs/otel/configuration/sdk-environment-variables/#boolean-value
impl OtelEnvVal for bool {
    type Err = &'static str;

    fn from_env(s: &str) -> Result<Self, Self::Err> {
        if s.eq_ignore_ascii_case("true") {
            Ok(true)
        } else if s.eq_ignore_ascii_case("false") {
            Ok(false)
        } else {
            Err(BOOL_ERR)
        }
    }
}

/// Parse an env variable per [Otel numeric value] spec.
///
/// [Otel numeric value]: https://opentelemetry.io/docs/specs/otel/configuration/sdk-environment-variables/#numeric-value
impl OtelEnvVal for u32 {
    type Err = std::num::ParseIntError;

    fn from_env(s: &str) -> Result<Self, Self::Err> {
        s.parse()
    }
}

/// Parse an env variable per [Otel numeric value] spec.
///
/// [Otel numeric value]: https://opentelemetry.io/docs/specs/otel/configuration/sdk-environment-variables/#numeric-value
impl OtelEnvVal for u64 {
    type Err = std::num::ParseIntError;

    fn from_env(s: &str) -> Result<Self, Self::Err> {
        s.parse()
    }
}

impl OtelEnvVal for Uri {
    type Err = InvalidUri;

    fn from_env(s: &str) -> Result<Self, Self::Err> {
        s.parse()
    }
}

/// Parse an env variable per [Otel duration] spec.
///
/// [Otel duration]: https://opentelemetry.io/docs/specs/otel/configuration/sdk-environment-variables/#duration
impl OtelEnvVal for Duration {
    type Err = std::num::ParseIntError;

    fn from_env(s: &str) -> Result<Self, Self::Err> {
        s.parse().map(Duration::from_millis)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Var {
    key: &'static str,
    val: String,
}

impl Var {
    pub(crate) fn get(key: &'static str) -> Option<Self> {
        match std::env::var(key) {
            Ok(val) => Some(Self { key, val }),
            Err(std::env::VarError::NotPresent) => None,
            Err(std::env::VarError::NotUnicode(_)) => {
                eprintln!("env var `{key}` is not Unicode");
                None
            }
        }
    }

    pub(crate) fn parse<V: OtelEnvVal>(&self) -> Option<V> {
        let Self { key, val } = self;
        match V::from_env(val) {
            Ok(v) => Some(v),
            Err(e) => {
                eprintln!("env var `{key}` has an invalid value: {e}");
                None
            }
        }
    }

    pub(crate) fn modify<F>(mut self, f: F) -> Self
    where
        F: FnOnce(String) -> String,
    {
        self.val = f(self.val);
        self
    }

    pub(crate) fn into_inner(self) -> String {
        self.val
    }
}

#[cfg(test)]
mod tests;
