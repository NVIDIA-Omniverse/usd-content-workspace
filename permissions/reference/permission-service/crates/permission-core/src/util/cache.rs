// SPDX-FileCopyrightText: Copyright 2026 NVIDIA CORPORATION & AFFILIATES
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

use crate::{parse_policy_set, PolicyErr};
use cedar_policy::PolicySet;
use moka::sync::Cache;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;

/// Cache key for a parsed [`PolicySet`].
///
/// `Hash` writes only the precomputed `fingerprint` (the stored MD5
/// `policy_hash` folded to `u64`) so large policy bodies are not re-hashed on
/// every map probe. `Eq` compares the full policy text, so a digest collision
/// can never return an unrelated AST.
#[derive(Clone, Debug)]
pub struct PolicyTextKey {
    text: Arc<str>,
    fingerprint: u64,
}

impl PolicyTextKey {
    pub fn new(policy_hash: &str, text: &str) -> Self {
        Self {
            text: Arc::from(text),
            fingerprint: fingerprint_from_policy_hash(policy_hash),
        }
    }

    #[cfg(test)]
    pub fn with_fingerprint(text: &str, fingerprint: u64) -> Self {
        Self {
            text: Arc::from(text),
            fingerprint,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn fingerprint(&self) -> u64 {
        self.fingerprint
    }
}

impl PartialEq for PolicyTextKey {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
    }
}

impl Eq for PolicyTextKey {}

impl Hash for PolicyTextKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.fingerprint);
    }
}

/// Folds the 16-byte MD5 digest into a `u64` for `HashMap` bucket selection.
/// Decodes the hex `policy_hash` and XORs the high and low halves — no second
/// cryptographic or SipHash pass over the digest string.
fn fingerprint_from_policy_hash(policy_hash: &str) -> u64 {
    if let Some(bytes) = decode_md5_hex(policy_hash) {
        let mut hi = [0u8; 8];
        let mut lo = [0u8; 8];
        hi.copy_from_slice(&bytes[..8]);
        lo.copy_from_slice(&bytes[8..]);
        return u64::from_be_bytes(hi) ^ u64::from_be_bytes(lo);
    }
    let mut hasher = DefaultHasher::new();
    policy_hash.hash(&mut hasher);
    hasher.finish()
}

fn decode_md5_hex(policy_hash: &str) -> Option<[u8; 16]> {
    if policy_hash.len() != 32 {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&policy_hash[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

/// In-process cache of parsed Cedar [`PolicySet`] ASTs, keyed by policy text
/// with `policy_hash` as the Hash accelerator.
#[derive(Debug)]
pub struct PolicySetCache {
    cache: Cache<PolicyTextKey, Arc<PolicySet>>,
}

impl PolicySetCache {
    pub fn new(capacity: u64, ttl: Duration) -> Self {
        let cache = Cache::builder()
            .max_capacity(capacity)
            .time_to_live(ttl)
            .build();
        Self { cache }
    }

    /// Returns the parsed set and whether it was already cached (`true` = hit).
    pub fn get_or_parse(
        &self,
        policy_hash: &str,
        text: &str,
    ) -> Result<(Arc<PolicySet>, bool), PolicyErr> {
        let key = PolicyTextKey::new(policy_hash, text);
        if let Some(cached) = self.cache.get(&key) {
            return Ok((cached, true));
        }
        let parsed = Arc::new(parse_policy_set(text)?);
        self.cache.insert(key, parsed.clone());
        Ok((parsed, false))
    }

    pub fn entry_count(&self) -> u64 {
        self.cache.entry_count()
    }

    #[cfg(test)]
    pub fn get(&self, key: &PolicyTextKey) -> Option<Arc<PolicySet>> {
        self.cache.get(key)
    }

    #[cfg(test)]
    pub fn insert(&self, key: PolicyTextKey, value: Arc<PolicySet>) {
        self.cache.insert(key, value);
    }
}
