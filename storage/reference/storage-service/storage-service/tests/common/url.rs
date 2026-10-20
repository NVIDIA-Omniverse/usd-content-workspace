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

use std::fmt::{Debug, Display, Write};

use percent_encoding::{utf8_percent_encode, AsciiSet};
use url_escape::{NON_ALPHANUMERIC, QUERY};

const PATH: AsciiSet = NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~')
    .remove(b'/');

/// A URL builder.
#[derive(Debug, Clone)]
pub struct UrlBuilder {
    base_url: String,
    raw_path: Option<String>,
    raw_query: Option<String>,
}

impl UrlBuilder {
    #[track_caller]
    pub fn new(base_url: impl Into<String>) -> Self {
        let base_url = base_url.into();
        if !base_url.ends_with("/") {
            panic!("url builder base url must end with a traling slash");
        }

        Self {
            base_url,
            raw_path: None,
            raw_query: None,
        }
    }

    /// Set the path component for the url without encoding it.
    pub fn raw_path(self, path: impl Into<String>) -> Self {
        Self {
            raw_path: Some(path.into()),
            ..self
        }
    }

    /// Set the path component for the url, encoding it if needed.
    pub fn path(self, path: impl AsRef<str>) -> Self {
        Self {
            raw_path: Some(utf8_percent_encode(path.as_ref(), &PATH).to_string()),
            ..self
        }
    }

    /// Set the query component for the url, encoding it if needed.
    pub fn query(self, query: impl AsRef<str>) -> Self {
        Self {
            raw_query: Some(utf8_percent_encode(query.as_ref(), QUERY).to_string()),
            ..self
        }
    }

    /// Set the query component containing the checkpoint number only.
    pub fn checkpoint_query(self, id: impl Display) -> Self {
        self.raw_query(format!("&{id}"))
    }

    /// Set the query component for the url, without encoding it.
    pub fn raw_query(self, query: impl Into<String>) -> Self {
        Self {
            raw_query: Some(query.into()),
            ..self
        }
    }

    /// Set or reset the query component for the url, without encoding it.
    pub fn maybe_raw_query(self, query: Option<impl Into<String>>) -> Self {
        Self {
            raw_query: query.map(Into::into),
            ..self
        }
    }

    /// Set the query component using the specified query parameters, without encoding them.
    pub fn raw_query_params(self, params: impl IntoIterator<Item = impl Display>) -> Self {
        let mut query = String::new();
        for (idx, param) in params.into_iter().enumerate() {
            if idx == 0 {
                let _ = write!(&mut query, "{}", param);
            } else {
                let _ = write!(&mut query, "&{}", param);
            }
        }

        Self {
            raw_query: Some(query),
            ..self
        }
    }
}

impl Display for UrlBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.base_url)?;
        if let Some(path) = self.raw_path.as_ref() {
            f.write_str(path)?;
        }

        if let Some(query) = self.raw_query.as_ref() {
            f.write_str("?")?;
            f.write_str(query)?;
        }

        Ok(())
    }
}

impl From<UrlBuilder> for String {
    fn from(value: UrlBuilder) -> Self {
        value.to_string()
    }
}

/// Remove a query string from a url, without decoding it.
#[track_caller]
pub fn split_query_string(url: &str) -> (&str, &str) {
    url.split_once("?").expect("query separator is missing")
}

/// Get a query string, without decoding it.
#[track_caller]
pub fn get_query_string(url: &str) -> &str {
    split_query_string(url).1
}

/// Extract the etag, last-modified, and content-length query parameters from a url and return them in order, in
/// which they must appear in a resource identity.
#[track_caller]
pub fn get_stat_query_pairs(url: &str) -> [&str; 2] {
    let (_, query) = split_query_string(url);
    [get_query_pair(query, "etag="), get_query_pair(query, "version-id=")]
}

#[track_caller]
fn get_query_pair<'a>(query: &'a str, prefix: &str) -> &'a str {
    query
        .split('&')
        .find(|param| param.starts_with(prefix))
        .unwrap_or_else(|| panic!("query parameter {:?} is missing", prefix))
}
