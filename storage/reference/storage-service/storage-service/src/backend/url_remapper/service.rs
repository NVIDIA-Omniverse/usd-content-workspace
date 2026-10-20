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

use std::fmt::Debug;

use storage_core::layer::Layer;

use crate::{
    backend::{metadata::Metadata, url_remapper::visibility::ArcAddressFilter},
    common::url_remapper::UrlRemapper,
};

/// URL remapper layer.
pub struct UrlRemapperLayer<T: Metadata> {
    remapper: UrlRemapper<T>,
    address_filter: Option<ArcAddressFilter>,
}

impl<T: Metadata> UrlRemapperLayer<T> {
    pub fn new(remapper: UrlRemapper<T>, address_filter: Option<ArcAddressFilter>) -> Self {
        Self {
            remapper,
            address_filter,
        }
    }
}

impl<T: Metadata> Clone for UrlRemapperLayer<T> {
    fn clone(&self) -> Self {
        Self {
            remapper: self.remapper.clone(),
            address_filter: self.address_filter.clone(),
        }
    }
}

impl<T: Metadata, S> Layer<S> for UrlRemapperLayer<T> {
    type Service = UrlRemapperService<T, S>;

    fn layer(&self, inner: S) -> Self::Service {
        UrlRemapperService {
            mapper: self.remapper.clone(),
            address_filter: self.address_filter.clone(),
            inner,
        }
    }
}

/// URL remapper service.
pub struct UrlRemapperService<T: Metadata, S> {
    pub(super) mapper: UrlRemapper<T>,
    pub(super) address_filter: Option<ArcAddressFilter>,
    pub(super) inner: S,
}

impl<T: Metadata + Debug, S: Debug> Debug for UrlRemapperService<T, S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UrlRemapperService")
            .field("mapper", &self.mapper)
            .field("address_filter", &self.address_filter)
            .field("inner", &self.inner)
            .finish()
    }
}
