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

use crate::service::authorization::AuthorizationDecision;
use clap::Args;
use moka::future::Cache;
use permission_auth::AuthCache;
use permission_core::{Action, PolicySetCache, Principal, Resource};
use permission_db::Meta;
use serde_json::Value;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;

pub type PrincipalKey = Option<Principal>;
pub type ActionKey = Option<Action>;
pub type ResourceKey = Option<Resource>;

/// Partial pattern used by [`EvaluationCache::invalidate`]. Cache invalidation
/// triggered by policy or metadata mutations is intentionally context-agnostic:
/// a change that affects a `(principal, action, resource)` triple must drop
/// every cached decision for it, regardless of which `context` produced it.
pub type InvalidationKey = (PrincipalKey, ActionKey, ResourceKey);

pub type EvaluationCacheValue = (AuthorizationDecision, Reason);
pub type Reason = Option<String>;

/// Identity of a cached authorization decision, keyed on
/// `(principal, action, resource, context)`.
///
/// `Hash` writes only a precomputed 64-bit `fingerprint`, so a cache lookup
/// never re-walks the principal claims tree or the context JSON — those are
/// hashed once when the key is built (see [`EvaluationKeyBuilder`] and
/// [`ContextKey`]). `Eq` remains the authoritative identity check and compares
/// every component structurally, so a fingerprint collision can never return a
/// foreign decision. The principal is held behind an `Arc` so cloning a key per
/// resource in a batch does not duplicate the claims tree.
#[derive(Clone, Debug)]
pub struct EvaluationCacheKey {
    principal: Option<Arc<Principal>>,
    action: ActionKey,
    resource: ResourceKey,
    context: ContextKey,
    fingerprint: u64,
}

impl EvaluationCacheKey {
    pub fn new(
        principal: PrincipalKey,
        action: ActionKey,
        resource: ResourceKey,
        context: ContextKey,
    ) -> Self {
        let principal = principal.map(Arc::new);
        let principal_hash = hash_component(&principal);
        Self::from_parts(principal, principal_hash, action, resource, context)
    }

    pub fn principal(&self) -> Option<&Principal> {
        self.principal.as_deref()
    }

    pub fn action(&self) -> Option<&Action> {
        self.action.as_ref()
    }

    pub fn resource(&self) -> Option<&Resource> {
        self.resource.as_ref()
    }

    fn from_parts(
        principal: Option<Arc<Principal>>,
        principal_hash: u64,
        action: ActionKey,
        resource: ResourceKey,
        context: ContextKey,
    ) -> Self {
        let fingerprint = combine_fingerprint(
            principal_hash,
            hash_component(&action),
            hash_component(&resource),
            context.hash,
        );
        Self {
            principal,
            action,
            resource,
            context,
            fingerprint,
        }
    }
}

impl PartialEq for EvaluationCacheKey {
    fn eq(&self, other: &Self) -> bool {
        self.fingerprint == other.fingerprint
            && self.principal == other.principal
            && self.action == other.action
            && self.resource == other.resource
            && self.context == other.context
    }
}

impl Eq for EvaluationCacheKey {}

impl Hash for EvaluationCacheKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.fingerprint);
    }
}

/// Reusable key prefix for one principal group: the principal (id + claims
/// tree) is cloned into an `Arc` and hashed a single time, then shared across
/// every `(action, resource, context)` key produced for the batch. This is what
/// removes the per-resource re-hash of the claims tree on the cache hot path.
pub struct EvaluationKeyBuilder {
    principal: Option<Arc<Principal>>,
    principal_hash: u64,
}

impl EvaluationKeyBuilder {
    pub fn new(principal: PrincipalKey) -> Self {
        let principal = principal.map(Arc::new);
        let principal_hash = hash_component(&principal);
        Self {
            principal,
            principal_hash,
        }
    }

    pub fn key(
        &self,
        action: ActionKey,
        resource: ResourceKey,
        context: ContextKey,
    ) -> EvaluationCacheKey {
        EvaluationCacheKey::from_parts(
            self.principal.clone(),
            self.principal_hash,
            action,
            resource,
            context,
        )
    }
}

/// Hashes a single key component with a fixed-seed hasher so the result is
/// stable across lookups and processes (equal components always fold into the
/// same fingerprint).
fn hash_component<T: Hash>(component: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    component.hash(&mut hasher);
    hasher.finish()
}

fn combine_fingerprint(principal: u64, action: u64, resource: u64, context: u64) -> u64 {
    let mut hasher = DefaultHasher::new();
    hasher.write_u64(principal);
    hasher.write_u64(action);
    hasher.write_u64(resource);
    hasher.write_u64(context);
    hasher.finish()
}

/// Context dimension of the evaluation cache key. Wraps the request `context`
/// so that two requests with the same `(principal, action, resource)` but
/// distinct contexts never share a cached decision.
///
/// `Eq` is the authoritative identity check (the cache's `HashMap` walks each
/// bucket with `Eq`) and is delegated to `serde_json::Value`'s built-in
/// structural `PartialEq`, which compares JSON objects as unordered sets of
/// key/value pairs. The 64-bit `hash` is precomputed once at construction by
/// walking the tree and sorting object keys at every depth, so the contract
/// `k1 == k2 ⇒ hash(k1) == hash(k2)` holds even when the `preserve_order`
/// feature is enabled for `serde_json` (which is the case in this build, pulled
/// in transitively by `cedar-policy-validator`).
///
/// `Value::Number` is guaranteed finite (`Number::from_f64` rejects NaN/Inf),
/// so implementing `Eq` here is total and safe.
#[derive(Clone, Debug)]
pub struct ContextKey {
    value: Option<Arc<Value>>,
    hash: u64,
}

impl PartialEq for ContextKey {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl Eq for ContextKey {}

impl Hash for ContextKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.hash);
    }
}

impl Default for ContextKey {
    fn default() -> Self {
        context_key(None)
    }
}

pub fn context_key(context: Option<&Value>) -> ContextKey {
    let value = context.cloned().map(Arc::new);
    let hash = context_hash(value.as_deref());
    ContextKey { value, hash }
}

fn context_hash(value: Option<&Value>) -> u64 {
    let mut hasher = DefaultHasher::new();
    match value {
        None => 0u8.hash(&mut hasher),
        Some(value) => {
            1u8.hash(&mut hasher);
            hash_value(value, &mut hasher);
        }
    }
    hasher.finish()
}

fn hash_value<H: Hasher>(value: &Value, state: &mut H) {
    match value {
        Value::Null => 0u8.hash(state),
        Value::Bool(b) => {
            1u8.hash(state);
            b.hash(state);
        }
        Value::Number(n) => {
            2u8.hash(state);
            n.to_string().hash(state);
        }
        Value::String(s) => {
            3u8.hash(state);
            s.hash(state);
        }
        Value::Array(items) => {
            4u8.hash(state);
            items.len().hash(state);
            for item in items {
                hash_value(item, state);
            }
        }
        Value::Object(map) => {
            5u8.hash(state);
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_unstable();
            keys.len().hash(state);
            for key in keys {
                key.hash(state);
                hash_value(&map[key], state);
            }
        }
    }
}

#[derive(Args, Clone, Debug)]
pub struct CacheArgs {
    /// Defines for how many seconds the clients can cache authorization results.
    /// If not specified or specified as zero, then cache control headers are disabled.
    #[arg(long, env = "CACHE_CONTROL_MAX_AGE", default_value_t = 0)]
    pub cache_control_max_age: u64,

    /// The LRU cache size for authorization calls.
    #[arg(long, env = "EVALUATION_CACHE_SIZE", default_value_t = 10000)]
    pub evaluation_cache_size: u64,

    /// Determines how long policy evaluations are cached.
    #[arg(long, env = "EVALUATION_CACHE_TTL", default_value = "15min", value_parser = humantime::parse_duration)]
    pub evaluation_cache_ttl: Duration,

    /// The LRU cache size for parsed Cedar PolicySet ASTs keyed by policy text.
    #[arg(long, env = "POLICY_SET_CACHE_SIZE", default_value_t = 1000)]
    pub policy_set_cache_size: u64,

    /// Determines how long parsed PolicySet ASTs are cached.
    #[arg(long, env = "POLICY_SET_CACHE_TTL", default_value = "1h", value_parser = humantime::parse_duration)]
    pub policy_set_cache_ttl: Duration,

    /// The LRU cache size for service metadata retrieved from the database.
    #[arg(long, env = "METADATA_CACHE_SIZE", default_value_t = 100)]
    pub metadata_cache_size: u64,

    /// Determines how long metadata is cached for each service.
    #[arg(long, env = "METADATA_CACHE_TTL", default_value = "1h", value_parser = humantime::parse_duration)]
    pub metadata_cache_ttl: Duration,
}

#[derive(Clone)]
pub struct Caches {
    pub auth: Arc<AuthCache>,
    pub metadata: Arc<MetadataCache>,
    pub evaluation: Arc<EvaluationCache>,
    pub policy_set: Arc<PolicySetCache>,
}

impl Caches {
    pub fn new(config: &CacheArgs, auth: Arc<AuthCache>) -> Self {
        let metadata = Arc::new(MetadataCache::new(
            config.metadata_cache_ttl,
            config.metadata_cache_size,
        ));
        let evaluation = Arc::new(EvaluationCache::new(
            config.evaluation_cache_size,
            config.evaluation_cache_ttl,
        ));
        let policy_set = Arc::new(PolicySetCache::new(
            config.policy_set_cache_size,
            config.policy_set_cache_ttl,
        ));
        Self {
            auth,
            metadata,
            evaluation,
            policy_set,
        }
    }
}

#[derive(Debug)]
pub struct EvaluationCache {
    cache: Cache<EvaluationCacheKey, EvaluationCacheValue>,
}

impl EvaluationCache {
    pub fn new(capacity: u64, ttl: Duration) -> Self {
        let cache = Cache::builder()
            .max_capacity(capacity)
            .time_to_live(ttl)
            .support_invalidation_closures()
            .build();
        Self { cache }
    }

    #[tracing::instrument(level = "debug")]
    pub fn contains_key(&self, key: &EvaluationCacheKey) -> bool {
        self.cache.contains_key(key)
    }

    #[tracing::instrument(level = "debug")]
    pub async fn get(&self, key: &EvaluationCacheKey) -> Option<EvaluationCacheValue> {
        self.cache.get(key).await
    }

    #[tracing::instrument(level = "debug")]
    pub async fn set(&self, key: EvaluationCacheKey, value: EvaluationCacheValue) {
        self.cache.insert(key, value).await;
    }

    #[tracing::instrument(level = "debug")]
    pub fn invalidate(&self, key: &InvalidationKey) {
        let (invalidated_principal, invalidated_action, invalidated_resource) = key.clone();

        self.cache
            .invalidate_entries_if(move |cache_key, _| {
                let invalidated = match (
                    &invalidated_principal,
                    &invalidated_action,
                    &invalidated_resource,
                ) {
                    // If principal is set, it must match exactly
                    (Some(p), _, _) => cache_key.principal().is_some_and(|cp| p.id == cp.id),
                    // If action is set, it must match exactly
                    (_, Some(a), _) => cache_key.action() == Some(a),
                    // If resource is set, it must match exactly
                    (_, _, Some(r)) => cache_key
                        .resource()
                        .is_some_and(|cr| r.id == cr.id && r.kind == cr.kind),
                    // Otherwise, this key matches our partial pattern
                    _ => true,
                };
                invalidated
            })
            // unwrap should be safe here since the only error
            // returned from invalidate_entries_if is PredicateErr::InvalidationClosuresDisabled.
            .unwrap();
    }

    #[tracing::instrument(level = "debug")]
    pub fn invalidate_service(&self, service: &str) {
        let invalidated_service = service.to_string();
        self.cache
            .invalidate_entries_if(move |cache_key, _| match cache_key.action() {
                Some(action) if action.service == invalidated_service => true,
                None => true,
                _ => false,
            })
            // unwrap should be safe here since the only error
            // returned from invalidate_entries_if is PredicateErr::InvalidationClosuresDisabled.
            .unwrap();
    }

    #[tracing::instrument(level = "debug")]
    pub fn invalidate_all(&self) {
        self.cache.invalidate_all();
    }

    pub fn entry_count(&self) -> u64 {
        self.cache.entry_count()
    }
}

pub type MetadataCacheKey = (MetadataService, Option<MetadataResourceType>);
pub type MetadataService = String;
pub type MetadataResourceType = String;

#[derive(Debug)]
pub struct MetadataCache {
    cache: Cache<MetadataCacheKey, Meta>,
}

impl MetadataCache {
    pub fn new(ttl: Duration, max_capacity: u64) -> Self {
        let cache = Cache::builder()
            .time_to_live(ttl)
            .max_capacity(max_capacity)
            .support_invalidation_closures()
            .build();
        Self { cache }
    }

    #[tracing::instrument(level = "debug")]
    pub async fn get(&self, key: &MetadataCacheKey) -> Option<Meta> {
        self.cache.get(key).await
    }

    #[tracing::instrument(level = "debug")]
    pub async fn set(&self, key: MetadataCacheKey, value: Meta) {
        self.cache.insert(key, value).await;
    }

    #[tracing::instrument(level = "debug")]
    pub async fn invalidate(&self, key: &MetadataCacheKey) {
        self.cache.invalidate(key).await;
    }

    #[tracing::instrument(level = "debug")]
    pub fn invalidate_all(&self) {
        self.cache.invalidate_all();
    }

    #[tracing::instrument(level = "debug")]
    pub fn invalidate_service(&self, service: &str) {
        let invalidated_service = service.to_string();
        self.cache
            .invalidate_entries_if(move |cache_key, _| {
                let (cached_service, _) = cache_key;
                cached_service == &invalidated_service
            })
            .unwrap();
    }

    pub fn entry_count(&self) -> u64 {
        self.cache.entry_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::hash_map::DefaultHasher;

    fn eval_cache(capacity: u64, ttl: Duration) -> EvaluationCache {
        EvaluationCache::new(capacity, ttl)
    }

    fn principal(id: &str) -> Principal {
        Principal {
            id: id.to_string(),
            info: None,
        }
    }

    fn action(service: &str, name: &str) -> Action {
        Action {
            service: service.to_string(),
            name: name.to_string(),
        }
    }

    fn resource(kind: &str, id: &str) -> Resource {
        Resource {
            kind: kind.to_string(),
            id: id.to_string(),
            data: None,
        }
    }

    fn allow() -> EvaluationCacheValue {
        (AuthorizationDecision::Allow, None)
    }

    fn deny() -> EvaluationCacheValue {
        (AuthorizationDecision::Deny, None)
    }

    fn hash_of(key: &ContextKey) -> u64 {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        hasher.finish()
    }

    fn principal_with_info(id: &str, info: Value) -> Principal {
        Principal {
            id: id.to_string(),
            info: Some(info),
        }
    }

    fn eval_key_hash(key: &EvaluationCacheKey) -> u64 {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn eval_key_builder_matches_direct_construction() {
        let principal = Some(principal("alice"));
        let action = Some(action("storage", "read"));
        let resource = Some(resource("file", "secret.txt"));
        let ctx = json!({ "mfa": true });

        let built = EvaluationKeyBuilder::new(principal.clone()).key(
            action.clone(),
            resource.clone(),
            context_key(Some(&ctx)),
        );
        let direct = EvaluationCacheKey::new(principal, action, resource, context_key(Some(&ctx)));

        assert_eq!(built, direct);
        assert_eq!(eval_key_hash(&built), eval_key_hash(&direct));
    }

    #[tokio::test]
    async fn distinct_principal_info_does_not_share_decision() {
        let cache = eval_cache(100, Duration::from_secs(60));
        let action = Some(action("storage", "read"));
        let resource = Some(resource("file", "secret.txt"));

        let admin = Some(principal_with_info("alice", json!({ "groups": ["admin"] })));
        let member = Some(principal_with_info("alice", json!({ "groups": [] })));

        let admin_key =
            EvaluationCacheKey::new(admin, action.clone(), resource.clone(), context_key(None));
        let member_key = EvaluationCacheKey::new(member, action, resource, context_key(None));

        assert_ne!(admin_key, member_key);

        cache.set(admin_key.clone(), allow()).await;
        assert_eq!(cache.get(&admin_key).await, Some(allow()));
        assert_eq!(
            cache.get(&member_key).await,
            None,
            "same id with different claims must not reuse a cached decision"
        );
    }

    #[test]
    fn context_key_equates_objects_regardless_of_member_order() {
        let a = json!({ "ip": "10.0.0.1", "mfa": true });
        let b = json!({ "mfa": true, "ip": "10.0.0.1" });
        assert_eq!(context_key(Some(&a)), context_key(Some(&b)));
    }

    #[test]
    fn context_key_equates_nested_objects_regardless_of_member_order() {
        let a = json!({
            "outer": { "z": 1, "a": 2, "m": [{ "k": 1, "a": 2 }] },
            "extra": null,
        });
        let b = json!({
            "extra": null,
            "outer": { "a": 2, "m": [{ "a": 2, "k": 1 }], "z": 1 },
        });
        assert_eq!(context_key(Some(&a)), context_key(Some(&b)));
    }

    #[test]
    fn context_key_hash_matches_equality_regardless_of_member_order() {
        let a = json!({
            "outer": { "z": 1, "a": 2, "m": [{ "k": 1, "a": 2 }] },
            "extra": null,
        });
        let b = json!({
            "extra": null,
            "outer": { "a": 2, "m": [{ "a": 2, "k": 1 }], "z": 1 },
        });
        let key_a = context_key(Some(&a));
        let key_b = context_key(Some(&b));
        assert_eq!(key_a, key_b);
        assert_eq!(hash_of(&key_a), hash_of(&key_b));
    }

    #[test]
    fn context_key_preserves_array_order() {
        let a = json!([1, 2, 3]);
        let b = json!([3, 2, 1]);
        assert_ne!(context_key(Some(&a)), context_key(Some(&b)));
    }

    #[test]
    fn context_key_distinguishes_different_values() {
        let a = json!({ "ip": "10.0.0.1" });
        let b = json!({ "ip": "10.0.0.2" });
        assert_ne!(context_key(Some(&a)), context_key(Some(&b)));
    }

    #[test]
    fn context_key_none_differs_from_empty_object() {
        let empty = json!({});
        assert_ne!(context_key(None), context_key(Some(&empty)));
    }

    #[test]
    fn context_key_integer_differs_from_float_with_same_value() {
        let int = json!({ "n": 1 });
        let float = json!({ "n": 1.0 });
        assert_ne!(context_key(Some(&int)), context_key(Some(&float)));
    }

    #[tokio::test]
    async fn different_context_does_not_poison_cache() {
        let cache = eval_cache(100, Duration::from_secs(60));

        let principal = Some(principal("alice"));
        let action = Some(action("storage", "read"));
        let resource = Some(resource("file", "secret.txt"));

        let trusted_context = json!({ "ip": "10.0.0.1", "mfa": true });
        let untrusted_context = json!({ "ip": "203.0.113.7", "mfa": false });

        let trusted_key = EvaluationCacheKey::new(
            principal.clone(),
            action.clone(),
            resource.clone(),
            context_key(Some(&trusted_context)),
        );
        let untrusted_key = EvaluationCacheKey::new(
            principal.clone(),
            action.clone(),
            resource.clone(),
            context_key(Some(&untrusted_context)),
        );

        assert_ne!(trusted_key, untrusted_key);

        cache.set(trusted_key.clone(), allow()).await;

        assert_eq!(cache.get(&trusted_key).await, Some(allow()));
        assert_eq!(
            cache.get(&untrusted_key).await,
            None,
            "untrusted context must not reuse trusted context's cached decision"
        );

        cache.set(untrusted_key.clone(), deny()).await;
        assert_eq!(cache.get(&trusted_key).await, Some(allow()));
        assert_eq!(cache.get(&untrusted_key).await, Some(deny()));
    }

    #[tokio::test]
    async fn missing_context_does_not_collide_with_explicit_context() {
        let cache = eval_cache(100, Duration::from_secs(60));

        let principal = Some(principal("alice"));
        let action = Some(action("storage", "read"));
        let resource = Some(resource("file", "secret.txt"));

        let with_context = json!({ "mfa": true });
        let key_no_context = EvaluationCacheKey::new(
            principal.clone(),
            action.clone(),
            resource.clone(),
            context_key(None),
        );
        let key_with_context = EvaluationCacheKey::new(
            principal.clone(),
            action.clone(),
            resource.clone(),
            context_key(Some(&with_context)),
        );

        cache.set(key_no_context.clone(), deny()).await;

        assert_eq!(cache.get(&key_no_context).await, Some(deny()));
        assert_eq!(cache.get(&key_with_context).await, None);
    }

    #[tokio::test]
    async fn invalidate_drops_all_contexts_for_matching_par() {
        let cache = eval_cache(100, Duration::from_secs(60));

        let principal = Some(principal("alice"));
        let action = Some(action("storage", "read"));
        let resource = Some(resource("file", "secret.txt"));

        let key_a = EvaluationCacheKey::new(
            principal.clone(),
            action.clone(),
            resource.clone(),
            context_key(Some(&json!({ "ip": "10.0.0.1" }))),
        );
        let key_b = EvaluationCacheKey::new(
            principal.clone(),
            action.clone(),
            resource.clone(),
            context_key(Some(&json!({ "ip": "10.0.0.2" }))),
        );

        cache.set(key_a.clone(), allow()).await;
        cache.set(key_b.clone(), deny()).await;
        cache.cache.run_pending_tasks().await;
        assert_eq!(cache.entry_count(), 2);

        let invalidation: InvalidationKey = (principal, action, resource);
        cache.invalidate(&invalidation);
        cache.cache.run_pending_tasks().await;

        assert_eq!(cache.get(&key_a).await, None);
        assert_eq!(cache.get(&key_b).await, None);
    }
}
