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

use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use snafu::{OptionExt, ResultExt};
use storage_core::{
    layer::Layer,
    traits::{Request, Result},
};

use self::{error::Error, utils::normalize_uri_path_in_place};
use crate::common::future::create_error_future;

pub mod filefolder;
pub mod fileobject;
pub mod metadata;
pub mod versioning;

mod error;
mod utils;

/// URL normalizer configuration.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(default)]
pub struct UrlNormalizerConfig {
    pub enabled: bool,
}

impl Default for UrlNormalizerConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// URL normalizer.
#[derive(Debug, Clone, Default)]
pub struct UrlNormalizerLayer;

impl UrlNormalizerLayer {
    /// Create a url mapper given a set of rewrite rules and checkpoint metadta key.
    pub fn new(config: UrlNormalizerConfig) -> Option<Self> {
        if !config.enabled {
            return None;
        }

        Some(Self {})
    }
}

impl<S> Layer<S> for UrlNormalizerLayer {
    type Service = UrlNormalizerService<S>;
    fn layer(&self, inner: S) -> Self::Service {
        UrlNormalizerService { inner }
    }
}

pub struct UrlNormalizerService<S> {
    inner: S,
}

impl<S> UrlNormalizerService<S> {
    #[track_caller]
    fn normalize_resource_address(&self, uri: &mut String) -> std::result::Result<(), Error> {
        normalize_uri_path_in_place(uri).context(error::error::ResourceAddressInvalid)
    }

    #[track_caller]
    fn normalize_folder_address(&self, uri: Option<&mut String>) -> std::result::Result<(), Error> {
        use error::error::*;

        let uri = uri.context(FolderAddressMissing)?;

        normalize_uri_path_in_place(uri).context(FolderAddressInvalid)
    }

    #[track_caller]
    fn invoke_with_resource_address<'a, T, R>(
        &self,
        mut request: Request<T>,
        parameter: impl FnOnce(&mut T) -> &mut String,
        invoke: impl FnOnce(Request<T>) -> BoxFuture<'a, Result<R>>,
    ) -> BoxFuture<'a, Result<R>> {
        if let Err(error) = self.normalize_resource_address(parameter(request.get_mut())) {
            return create_error_future(error);
        }

        invoke(request)
    }

    #[track_caller]
    fn invoke_with_folder_address<'a, T, R>(
        &self,
        mut request: Request<T>,
        parameter: impl FnOnce(&mut T) -> Option<&mut String>,
        invoke: impl FnOnce(Request<T>) -> BoxFuture<'a, Result<R>>,
    ) -> BoxFuture<'a, Result<R>> {
        if let Err(error) = self.normalize_folder_address(parameter(request.get_mut())) {
            return create_error_future(error);
        }

        invoke(request)
    }
}
