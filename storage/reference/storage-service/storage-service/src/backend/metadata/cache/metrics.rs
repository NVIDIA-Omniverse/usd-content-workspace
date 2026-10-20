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

use storage_core_util::observability::metrics::{metric_base, BoundUIntCounter, BoundUIntGauge};
use storage_service_metrics::utils::BindExt;

pub struct Metrics {
    pub cache_access_count: BoundUIntCounter,
    pub cache_miss_count: BoundUIntCounter,
    pub cache_size: BoundUIntGauge,
    pub cache_entries: BoundUIntGauge,
}

impl Metrics {
    pub fn new() -> Self {
        metric_base!(
            static CACHE_ACCESS_BASE: UIntCounter,
            name = "metadata_cache_access",
            description = "Metadata cache access count",
            { pod }
        );
        metric_base!(
            static CACHE_MISS_BASE: UIntCounter,
            name = "metadata_cache_miss",
            description = "Metadata cache miss count",
            { pod }
        );
        metric_base!(
            static CACHE_SIZE_BASE: UIntGauge,
            name = "metadata_cache_size",
            description = "Metadata cache memory footprint, bytes",
            { pod }
        );
        metric_base!(
            static CACHE_ENTRIES_BASE: UIntGauge,
            name = "metadata_cache_entries",
            description = "Metadata cache entry count",
            { pod }
        );

        Self {
            cache_access_count: CACHE_ACCESS_BASE.bind_default_attributes(),
            cache_miss_count: CACHE_MISS_BASE.bind_default_attributes(),
            cache_size: CACHE_SIZE_BASE.bind_default_attributes(),
            cache_entries: CACHE_ENTRIES_BASE.bind_default_attributes(),
        }
    }
}

impl std::fmt::Debug for Metrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Metrics").finish_non_exhaustive()
    }
}
