// SPDX-FileCopyrightText: Copyright (c) 2024 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Traits that the [`super::cache::Cache`] requires of its value type so it can
//! (a) weigh entries in real bytes for moka's L1 capacity, and (b) tag serde
//! envelopes with a schema version that survives rolling deploys.

use serde::de::DeserializeOwned;
use serde::Serialize;

/// Approximate retained heap+inline size of `self` in bytes, clamped to `u32`.
///
/// Called once per L1 insert from the moka weigher, so implementations must be
/// O(1) or O(n) over already-materialised structure — never re-serialize the
/// value or perform I/O. Approximations are fine; moka uses the value only for
/// admission and eviction decisions.
pub trait CachedSize {
    fn cached_size(&self) -> u32;
}

/// Cached value type, with a schema version stamped on every L2 envelope.
/// Bump `SCHEMA` on any breaking serde change; reads with a mismatched schema
/// are treated as a miss and re-populated through the init path.
pub trait CachedValue: Serialize + DeserializeOwned + CachedSize {
    const SCHEMA: u32;
}

// --- common impls so `Cache<String>`, `Cache<Bytes>` etc. work without ceremony

impl<T: CachedSize + ?Sized> CachedSize for std::sync::Arc<T> {
    fn cached_size(&self) -> u32 {
        (**self).cached_size()
    }
}

impl CachedSize for bytes::Bytes {
    fn cached_size(&self) -> u32 {
        u32::try_from(self.len()).unwrap_or(u32::MAX)
    }
}

impl CachedSize for Vec<u8> {
    fn cached_size(&self) -> u32 {
        u32::try_from(self.len()).unwrap_or(u32::MAX)
    }
}

impl CachedSize for String {
    fn cached_size(&self) -> u32 {
        u32::try_from(self.len()).unwrap_or(u32::MAX)
    }
}

impl CachedSize for u64 {
    fn cached_size(&self) -> u32 {
        std::mem::size_of::<u64>() as u32
    }
}

impl CachedSize for () {
    fn cached_size(&self) -> u32 {
        0
    }
}

// Inherit CachedValue through Arc<T> so cache values that opt into Arc-clone
// semantics (e.g. list_cache's `Arc<ListMap>`) don't need a separate impl.
// Requires serde's `rc` feature, which is enabled in this crate so the
// `Arc<T>: Serialize + DeserializeOwned` supertrait bounds are satisfied.
impl<T: CachedValue> CachedValue for std::sync::Arc<T> {
    const SCHEMA: u32 = T::SCHEMA;
}

// Schema = 0 for the primitive convenience impls; layer-owned types start at 1.
impl CachedValue for String {
    const SCHEMA: u32 = 0;
}

impl CachedValue for u64 {
    const SCHEMA: u32 = 0;
}

impl CachedValue for () {
    const SCHEMA: u32 = 0;
}

impl CachedValue for Vec<u8> {
    const SCHEMA: u32 = 0;
}
