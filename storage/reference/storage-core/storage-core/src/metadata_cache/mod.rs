// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

pub mod error;
pub mod metadata;

mod cache;
pub use cache::*;

use storage_core_builder::traits::error::Result;
use url::Url;

pub(crate) fn canonical_cache_key(value: &str) -> Result<String> {
    let mut url = Url::parse(value).map_err(|e| error::Error::invalid_argument(e.to_string()))?;
    storage_core_util::url_path::canonicalize_url_path(&mut url)
        .map_err(|e| error::Error::invalid_argument(e.to_string()))?;
    Ok(url.into())
}
