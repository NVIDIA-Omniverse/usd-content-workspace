// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! API-version partition for cache keys, shared by the stat and small-object caches.
//!
//! The stat/small-object caches are shared across v1alpha and v1beta and keyed by address/identity;
//! because the v1beta proto cannot carry native_metadata, a v1beta fill would otherwise poison a
//! v1alpha read of the same address with an empty map. Keys are partitioned per version so fills
//! never collide. This lives in `storage-core-util` (always compiled) so a cache layer built alone
//! (e.g. `--features small_object_cache` without `stat_cache`) can reference it.
//!
//! Tradeoff (accepted): the partition is unconditional, so when `native_metadata_enabled` is off
//! (the default) — where v1alpha and v1beta responses are identical — a v1beta read following a
//! cached v1alpha read of the same address misses and refetches, storing a duplicate entry. This
//! doubles the stat/small-object cache footprint and backend HEAD/GET traffic **only for addresses
//! served over both API versions in the same deployment** (mixed-version traffic during the
//! v1alpha→v1beta transition); single-version traffic is unaffected. The partition is kept
//! unconditional to avoid plumbing the backend `native_metadata_enabled` flag (which the cache layer
//! does not otherwise see) into every key computation; make it version-neutral-when-disabled if the
//! mixed-version footprint becomes a concern.

#[derive(Clone, Copy)]
pub enum ApiVersion {
    V1Alpha,
    V1Beta,
}

impl ApiVersion {
    pub const ALL: [ApiVersion; 2] = [ApiVersion::V1Alpha, ApiVersion::V1Beta];

    pub fn prefix(self) -> &'static str {
        match self {
            ApiVersion::V1Alpha => "v1alpha",
            ApiVersion::V1Beta => "v1beta",
        }
    }
}
