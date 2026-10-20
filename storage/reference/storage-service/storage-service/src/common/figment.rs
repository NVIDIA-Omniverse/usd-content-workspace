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

use figment::{
    value::{Dict, Map, Tag, Value},
    Metadata, Profile, Provider,
};
use std::str::FromStr;

use crate::error::BoxError;

/// A figment configuration provider that interpolates environment variable values from ${KEY}-like placeholders. Upon
/// encountering such a placeholder, the value is deserialized according to the default figment value parsing rules.
pub struct Interpolate<T> {
    inner: T,
}

impl<T> Interpolate<T> {
    pub fn new(inner: T) -> Self {
        Self { inner }
    }
}

impl<T: Provider> Provider for Interpolate<T> {
    fn metadata(&self) -> Metadata {
        self.inner.metadata()
    }

    fn profile(&self) -> Option<Profile> {
        self.inner.profile()
    }

    fn data(&self) -> Result<Map<Profile, Dict>, figment::Error> {
        let mut profiles = Map::new();
        for (key, data) in self.inner.data()?.into_iter() {
            let data = interpolate_dict(data, &mut |key| std::env::var(key))
                .map_err(|error| figment::Error::from(error.to_string()))?;
            profiles.insert(key, data);
        }

        Ok(profiles)
    }
}

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("failed to resolve the value of {0:?}")]
    Resolve(String, #[source] BoxError),
}

fn interpolate_dict<T, E>(value: Dict, resolve: &mut T) -> Result<Dict, Error>
where
    T: FnMut(&str) -> Result<String, E>,
    E: std::error::Error + Send + Sync + 'static,
{
    let mut modified = Dict::new();
    for (key, value) in value.into_iter() {
        let key = interpolate_string_as_string(&key, resolve)?.unwrap_or(key);
        let value = interpolate_value(value, resolve)?;
        modified.insert(key, value);
    }

    Ok(modified)
}

fn interpolate_array<T, E>(value: Vec<Value>, resolve: &mut T) -> Result<Vec<Value>, Error>
where
    T: FnMut(&str) -> Result<String, E>,
    E: std::error::Error + Send + Sync + 'static,
{
    let mut modified = Vec::new();
    for value in value.into_iter() {
        modified.push(interpolate_value(value, resolve)?);
    }

    Ok(modified)
}

fn interpolate_string<T, E>(tag: Tag, original: String, resolve: &mut T) -> Result<Value, Error>
where
    T: FnMut(&str) -> Result<String, E>,
    E: std::error::Error + Send + Sync + 'static,
{
    if let Some(interpolated) = interpolate_string_as_string(&original, resolve)? {
        Ok(Value::from_str(&interpolated).expect("infallible"))
    } else {
        Ok(Value::String(tag, original))
    }
}

fn interpolate_string_as_string<T, E>(value: &str, resolve: &mut T) -> Result<Option<String>, Error>
where
    T: FnMut(&str) -> Result<String, E>,
    E: std::error::Error + Send + Sync + 'static,
{
    let Some(value) = value.strip_prefix("${") else {
        return Ok(None);
    };

    let Some(key) = value.strip_suffix("}") else {
        return Ok(None);
    };

    resolve(key)
        .map(Some)
        .map_err(|error| Error::Resolve(key.into(), Box::new(error)))
}

fn interpolate_value<T, E>(value: Value, resolve: &mut T) -> Result<Value, Error>
where
    T: FnMut(&str) -> Result<String, E>,
    E: std::error::Error + Send + Sync + 'static,
{
    match value {
        Value::String(tag, value) => interpolate_string(tag, value, resolve),
        Value::Array(tag, value) => Ok(Value::Array(tag, interpolate_array(value, resolve)?)),
        Value::Dict(tag, value) => Ok(Value::Dict(tag, interpolate_dict(value, resolve)?)),
        other => Ok(other),
    }
}
