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

use std::{fmt::Debug, sync::Arc};

use bit_vec::BitVec;
use storage_core::traits::Result;
use tonic::{async_trait, metadata::MetadataMap};

/// A reference-counted [`AddressFilter`] trait object.
pub type ArcAddressFilter = Arc<dyn AddressFilter>;

/// Decides which of the folder addresses produced by the URL remapper the caller may see.
/// Both methods return one retain bit per address, in order, evaluated for the principal
/// carried by the request metadata.
#[async_trait]
pub trait AddressFilter: Debug + Send + Sync {
    /// Filter addresses advertised through `ListTopLevelAddresses`.
    async fn retain_top_level_addresses(
        &self,
        metadata: &MetadataMap,
        addresses: &mut (dyn Iterator<Item = &str> + Send),
    ) -> Result<BitVec>;

    /// Filter addresses advertised through `ListRoutes`.
    async fn retain_routes(
        &self,
        metadata: &MetadataMap,
        addresses: &mut (dyn Iterator<Item = &str> + Send),
    ) -> Result<BitVec>;
}
