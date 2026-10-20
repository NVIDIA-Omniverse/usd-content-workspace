// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Secrets-policy generation used to partition every cache tier.
//!
//! When the endpoint allowlist changes on hot-reload — an endpoint added or
//! removed — response caches that sit above the backend can otherwise keep
//! serving (or keep denying) that endpoint until their entries expire. To close
//! that window without depending on how a particular service composes its layer
//! stack, every cache key is prefixed with a *secrets-policy generation*: a
//! stable hash of the active secrets configuration.
//!
//! Because the value is derived from the secrets content rather than a local
//! counter, all replicas and process restarts that load the same secrets
//! compute the same generation, so they share the Redis L2 tier while the policy
//! is stable and move to a fresh partition *together* when it changes. Entries
//! written under the previous generation carry the old prefix, are never read
//! again, and age out via TTL/capacity.

use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use figment::{Figment, Provider};

use crate::secrets::SecretsReloadable;

/// A cheaply-cloneable, lock-free handle to the current secrets-policy
/// generation. Clones share the same underlying value, so a single handle can
/// be given to every cache tier and updated in one place on secrets reload.
///
/// # Wiring (copy-paste reference)
///
/// Create one handle from the startup secrets, hand a clone to each response
/// cache tier via its `*_with_generation` constructor, and register the handle
/// with the secrets watcher so a hot-reload repartitions every tier together:
///
/// ```ignore
/// use std::sync::Arc;
/// use storage_core_util::cache::SecretsGeneration;
/// use storage_core_util::secrets::Secrets;
/// use storage_core::stat_cache::StatCacheLayer;
/// use storage_core::list_cache::ListCacheLayer;
///
/// let mut secrets = Secrets::from_figment(&figment)?;
///
/// // One generation, shared by every cache tier and by the watcher.
/// let generation = SecretsGeneration::new(secrets.figment().as_ref());
///
/// let stat_cache = StatCacheLayer::from_figment_with_redis_and_generation(
///     &figment, redis.clone(), Some(generation.clone()),
/// ).await?;
/// let list_cache = ListCacheLayer::from_figment_with_redis_and_generation(
///     &figment, redis.clone(), Some(generation.clone()),
/// ).await?;
///
/// // Register alongside the storage backends; one reload bumps them all.
/// secrets.register(backend.clone());
/// secrets.register(Arc::new(generation));
/// let _watcher = secrets.start();
/// ```
///
/// # Registration order (required)
///
/// The secrets watcher invokes [`SecretsReloadable::reload_secrets`] on its
/// subscribers **in registration order**. The storage backends **must** be
/// registered *before* this generation handle. If the order is reversed, a
/// hot-reload bumps the generation first: a cache fill that is in flight during
/// the reload snapshots the *new* generation while its `init` future still reads
/// through a backend that has not reloaded yet, writing an old-policy response
/// into the *fresh* partition where it remains readable for a full TTL. Bumping
/// the generation only after every backend has swapped its credentials keeps a
/// fill either entirely on the old policy (old prefix, unreachable) or entirely
/// on the new one.
#[derive(Clone, Debug)]
pub struct SecretsGeneration(Arc<AtomicU64>);

impl SecretsGeneration {
    /// Creates a handle initialized from the given secrets figment (use `None`
    /// when no secrets are configured). Initializing from the startup secrets —
    /// rather than a fixed seed — keeps the generation stable across restarts
    /// so a restarted pod re-uses the warm L2 tier.
    pub fn new(secrets: Option<&Figment>) -> Self {
        Self(Arc::new(AtomicU64::new(hash_secrets(secrets))))
    }

    /// The current generation value, mixed into cache keys.
    pub fn current(&self) -> u64 {
        self.0.load(Ordering::Acquire)
    }

    /// Recomputes and stores the generation from a new secrets figment. Safe to
    /// call concurrently with cache reads.
    pub fn update(&self, secrets: Option<&Figment>) {
        self.0.store(hash_secrets(secrets), Ordering::Release);
    }
}

impl SecretsReloadable for SecretsGeneration {
    /// Registered alongside the storage backends so a single secrets hot-reload
    /// repartitions every cache tier that shares this handle.
    async fn reload_secrets(&self, secrets: Option<&Figment>) {
        self.update(secrets);
    }
}

/// Computes a stable, content-derived hash of the secrets configuration.
///
/// The figment is serialized through its ordered (`BTreeMap`-backed) data view
/// so the hash is deterministic across processes and restarts for identical
/// secrets. `DefaultHasher` uses fixed keys, so it too is deterministic for a
/// given `std`; a toolchain change can shift the value, which merely cold-starts
/// the caches on the next deploy — an acceptable outcome.
fn hash_secrets(secrets: Option<&Figment>) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    match secrets {
        None => 0u8.hash(&mut hasher),
        Some(figment) => match figment.data() {
            Ok(data) => match serde_json::to_vec(&data) {
                Ok(bytes) => bytes.hash(&mut hasher),
                // Serialization of a figment data view is not expected to fail; fall back to a
                // fixed discriminant so we still produce a stable (if content-insensitive) value.
                Err(_) => 1u8.hash(&mut hasher),
            },
            Err(_) => 2u8.hash(&mut hasher),
        },
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn figment_with(pairs: &[(&str, &str)]) -> Figment {
        let mut f = Figment::new();
        for (k, v) in pairs {
            f = f.merge(("secrets".to_string() + ".dummy", ()));
            f = f.join((k.to_string(), v.to_string()));
        }
        f
    }

    #[test]
    fn same_secrets_yield_same_generation() {
        let a = SecretsGeneration::new(Some(&figment_with(&[("https://a.example", "key-a")])));
        let b = SecretsGeneration::new(Some(&figment_with(&[("https://a.example", "key-a")])));
        assert_eq!(
            a.current(),
            b.current(),
            "identical secrets must hash equally across handles"
        );
    }

    #[test]
    fn adding_an_endpoint_changes_the_generation() {
        let gen = SecretsGeneration::new(Some(&figment_with(&[("https://a.example", "key-a")])));
        let before = gen.current();
        gen.update(Some(&figment_with(&[
            ("https://a.example", "key-a"),
            ("https://b.example", "key-b"),
        ])));
        assert_ne!(before, gen.current(), "adding an endpoint must repartition the caches");
    }

    #[test]
    fn removing_an_endpoint_changes_the_generation() {
        let gen = SecretsGeneration::new(Some(&figment_with(&[
            ("https://a.example", "key-a"),
            ("https://b.example", "key-b"),
        ])));
        let before = gen.current();
        gen.update(Some(&figment_with(&[("https://a.example", "key-a")])));
        assert_ne!(
            before,
            gen.current(),
            "removing an endpoint must repartition the caches"
        );
    }

    #[test]
    fn none_differs_from_some() {
        let empty = SecretsGeneration::new(None);
        let populated = SecretsGeneration::new(Some(&figment_with(&[("https://a.example", "key-a")])));
        assert_ne!(empty.current(), populated.current());
    }

    #[tokio::test]
    async fn reload_updates_generation() {
        let gen = SecretsGeneration::new(None);
        let before = gen.current();
        gen.reload_secrets(Some(&figment_with(&[("https://a.example", "key-a")])))
            .await;
        assert_ne!(before, gen.current(), "reload must update the generation");
    }
}
