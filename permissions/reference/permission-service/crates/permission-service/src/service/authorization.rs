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

use crate::cache::{
    context_key, ContextKey, EvaluationCacheKey, EvaluationCacheValue, EvaluationKeyBuilder,
};
use crate::config::{DatabaseType, ServiceConfig};
use crate::metrics::Metrics;
use crate::state::AppState;
use crate::url_remapper::{load_url_remapper_config, UrlRemapper};
use axum::http::StatusCode;
use garde::Validate;
use opentelemetry::KeyValue;
use permission_core::util::encoding::encode_resource_url;
use permission_core::{
    build_resource_parents, evaluate_policy_set_with_principal, prepare_principal,
    principal_groups, process_policy, resource_hierarchy_ancestors, Action, Group, PolicyDecision,
    PolicyErr, PolicyErrKind, PolicyId, PreparedPrincipal, Principal, ProcessedPolicy, Resource,
};
use permission_db::config_file::PolicyConfig;
use permission_db::{
    EvaluationPriority, Meta, MetadataConfig, MetadataPrincipalConfig, MetadataServiceConfig,
    MetadataStorage, Policy, PolicyDbBatch, PolicyQueryBatch, PolicyStorage, ResourceTypeMeta,
    ServiceMeta,
};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{hash_map, BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, OnceLock};
use tokio::sync::OnceCell;
use tonic::Status;
use utoipa::ToSchema;

#[derive(Clone, Debug, Default, Deserialize, Serialize, ToSchema, Validate)]
pub struct AuthorizationPayload {
    #[garde(dive)]
    pub principal: Option<Principal>,

    #[garde(dive)]
    pub action: Action,

    #[schema(default = AuthorizationPayload::default_resource)]
    #[garde(dive)]
    pub resource: Option<Resource>,

    #[garde(skip)]
    pub context: Option<Value>,
}

impl AuthorizationPayload {
    pub fn default_resource() -> Option<Resource> {
        Some(Resource::default())
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, ToSchema)]
pub struct AuthorizationResponse {
    pub decision: AuthorizationDecision,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(max_length = 2048, pattern = "^[\\x20-\\x7E]+$")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(max_length = 256, pattern = "^[a-zA-Z0-9_.:/-]+$")]
    pub service: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(max_length = 256, pattern = "^[a-zA-Z0-9_.:/-]+$")]
    pub action: Option<String>,
}

impl AuthorizationResponse {
    fn for_action(
        decision: AuthorizationDecision,
        reason: Option<String>,
        action: &Action,
    ) -> Self {
        Self {
            decision,
            reason,
            service: Some(action.service.clone()),
            action: Some(action.name.clone()),
        }
    }

    fn for_decision(decision: AuthorizationDecision) -> Self {
        Self {
            decision,
            reason: None,
            service: None,
            action: None,
        }
    }

    fn skip(action: &Action) -> Self {
        Self::for_action(AuthorizationDecision::Skip, None, action)
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, Eq, PartialEq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum AuthorizationDecision {
    #[default]
    Deny,
    Allow,
    Skip,
}

impl From<PolicyDecision> for AuthorizationDecision {
    fn from(value: PolicyDecision) -> Self {
        match value {
            PolicyDecision::Deny => AuthorizationDecision::Deny,
            PolicyDecision::Allow => AuthorizationDecision::Allow,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, ToSchema, Validate)]
pub struct AuthorizationBatchRequest {
    #[schema(max_items = 100)]
    #[garde(length(max = 100), dive)]
    pub batches: Vec<AuthorizationBatch>,

    #[garde(skip)]
    pub condition: Option<AuthorizationBatchCondition>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum AuthorizationBatchCondition {
    #[default]
    /// Evaluates all requests in batches similarly to individual requests.
    /// The summary is compiled similarly to "and" condition but does not stop the evaluation
    /// after first "deny".
    None,

    /// Checks if any of the actions is allowed.
    /// Stops after the first "allow" decision.
    Or,

    /// Checks if all specified actions are allowed.
    /// Stops after the first "deny" decision.
    And,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, ToSchema, Validate)]
pub struct AuthorizationBatch {
    #[garde(dive)]
    pub principal: Option<Principal>,

    #[schema(max_items = 1000)]
    #[garde(length(max = 1000), dive)]
    pub actions: Vec<Action>,

    #[schema(default = AuthorizationPayload::default_resource)]
    #[garde(dive)]
    pub resource: Option<Resource>,

    #[garde(skip)]
    pub context: Option<Value>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, ToSchema)]
pub struct AuthorizationBatchResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<AuthorizationResponse>,

    #[schema(default = AuthorizationBatchResponse::default_decisions, max_items = 100, max_length = 1000, pattern = "^[a-zA-Z0-9_.:/-]+$")]
    pub decisions: Vec<HashMap<String, AuthorizationResponse>>,
}

impl AuthorizationBatchResponse {
    fn default_decisions() -> Vec<HashMap<String, AuthorizationResponse>> {
        let mut map = HashMap::new();
        map.insert(
            "service:action".to_string(),
            AuthorizationResponse {
                decision: AuthorizationDecision::Deny,
                reason: Some("Explicit deny.".to_string()),
                service: Some("service".to_string()),
                action: Some("action".to_string()),
            },
        );
        vec![map]
    }
}

/// Selects the access-log `params` for a single authorization / diagnostics
/// request: `principal.sub`, action, and `resource` id and type. `principal.info`,
/// `resource.data`, and `context` are intentionally omitted, as they may carry
/// caller-supplied sensitive data.
pub fn access_params_single(payload: &AuthorizationPayload) -> Value {
    let mut params = serde_json::Map::new();
    if let Some(principal) = &payload.principal {
        params.insert("principal".to_string(), json!({ "sub": principal.id }));
    }
    params.insert(
        "action".to_string(),
        json!({ "name": payload.action.name, "service": payload.action.service }),
    );
    if let Some(resource) = &payload.resource {
        params.insert(
            "resource".to_string(),
            json!({ "id": resource.id, "type": resource.kind }),
        );
    }
    Value::Object(params)
}

/// Success `detail` for a single authorization decision: `decision` and, when
/// present, `reason`.
pub fn access_detail_single(response: &AuthorizationResponse) -> Value {
    let mut detail = serde_json::Map::new();
    detail.insert(
        "decision".to_string(),
        serde_json::to_value(&response.decision).unwrap_or(Value::Null),
    );
    if let Some(reason) = &response.reason {
        detail.insert("reason".to_string(), json!(reason));
    }
    Value::Object(detail)
}

/// Unwinds a batch request into one access-log `params` object per action, in
/// evaluation order (batch order, then action order). Batches with no actions
/// are skipped; a wholly empty `batches` array yields a single `{ condition? }`
/// line.
pub fn access_params_batch(
    condition: Option<AuthorizationBatchCondition>,
    batches: &[AuthorizationBatch],
) -> Vec<Value> {
    let condition = condition.and_then(|condition| serde_json::to_value(condition).ok());

    if batches.is_empty() {
        let mut params = serde_json::Map::new();
        if let Some(condition) = &condition {
            params.insert("condition".to_string(), condition.clone());
        }
        return vec![Value::Object(params)];
    }

    let mut lines = Vec::new();
    for batch in batches {
        if batch.actions.is_empty() {
            continue;
        }
        for action in &batch.actions {
            let mut params = serde_json::Map::new();
            if let Some(condition) = &condition {
                params.insert("condition".to_string(), condition.clone());
            }
            if let Some(principal) = &batch.principal {
                params.insert("principal".to_string(), json!({ "sub": principal.id }));
            }
            params.insert(
                "action".to_string(),
                json!({ "name": action.name, "service": action.service }),
            );
            if let Some(resource) = &batch.resource {
                params.insert(
                    "resource".to_string(),
                    json!({ "id": resource.id, "type": resource.kind }),
                );
            }
            lines.push(Value::Object(params));
        }
    }
    lines
}

/// Per-line success `detail` for a batch authorization response, aligned with
/// the order produced by [`access_params_batch`].
pub fn access_details_batch(
    batches: &[AuthorizationBatch],
    decisions: &[HashMap<String, AuthorizationResponse>],
) -> Vec<Value> {
    let mut details = Vec::new();
    for (index, batch) in batches.iter().enumerate() {
        if batch.actions.is_empty() {
            continue;
        }
        let decision_map = decisions.get(index);
        for action in &batch.actions {
            let detail = decision_map
                .and_then(|map| map.get(&action.entity_id()))
                .map(access_detail_single)
                .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
            details.push(detail);
        }
    }
    details
}

pub enum AuthorizationErr {
    BadRequest(String),
    Forbidden(String),
    Internal(String),
}

/// Canonical body returned to clients for any 403 produced by authorization
/// checks. Keeping a single uniform string avoids leaking the internal action
/// name and principal sub on routes that previously echoed the Cedar request
/// (e.g. `/policies/*`, `/authorization/*`), while matching the short body
/// already used by `/services/*` and `/diagnostics/*`.
const FORBIDDEN_RESPONSE_BODY: &str = "Permission denied";

impl AuthorizationErr {
    /// Maps the error to its HTTP `(status, body)` pair. `Forbidden` variants
    /// always produce the canonical body so all REST 403s from authorization
    /// checks are byte-identical regardless of which check failed. The verbose
    /// `msg` is preserved internally for logs and for use as the in-payload
    /// `reason` field in batch authorization responses (which is independently
    /// gated by `enable_deny_reason`).
    pub fn to_status(&self) -> (StatusCode, String) {
        match self {
            AuthorizationErr::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            AuthorizationErr::Forbidden(_) => {
                (StatusCode::FORBIDDEN, FORBIDDEN_RESPONSE_BODY.to_string())
            }
            AuthorizationErr::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg.clone()),
        }
    }

    /// gRPC counterpart of [`to_status`]. Same uniform body for `Forbidden` so
    /// gRPC clients see one canonical permission-denied message instead of two.
    pub fn to_grpc(&self) -> Status {
        match self {
            AuthorizationErr::BadRequest(msg) => Status::invalid_argument(msg),
            AuthorizationErr::Forbidden(_) => Status::permission_denied(FORBIDDEN_RESPONSE_BODY),
            AuthorizationErr::Internal(msg) => Status::internal(msg),
        }
    }

    /// Returns the original (potentially verbose) internal message. Used for
    /// logs and for populating the in-payload `reason` field of batch
    /// authorization responses; never returned directly to clients as a 403
    /// body.
    pub fn message(&self) -> &str {
        match self {
            Self::BadRequest(msg) | Self::Forbidden(msg) | Self::Internal(msg) => msg,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct DiagnosticsAuthorizeResponse {
    pub evaluation_priority: EvaluationPriority,
    pub policies: Vec<DiagnosticsPolicy>,
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct DiagnosticsPolicy {
    pub id: PolicyId,
    pub order: i32,
    pub policy: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub principal: Option<Principal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<Group>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<Action>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<Resource>,
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct DiagnosticsAuthorizeBatchResponse {
    pub results: Vec<DiagnosticsBatchResult>,
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct DiagnosticsBatchResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<Resource>,
    pub evaluation_priority: EvaluationPriority,
    pub actions: Vec<DiagnosticsActionPolicies>,
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct DiagnosticsActionPolicies {
    pub service: String,
    pub action: String,
    pub policies: Vec<DiagnosticsPolicy>,
}

#[tracing::instrument(level = "info", skip(requester, state))]
pub async fn authorize(
    payload: AuthorizationPayload,
    state: AppState,
    requester: &Principal,
) -> Result<AuthorizationResponse, AuthorizationErr> {
    let action = payload.action;
    let payload_resource = match state.url_remapper.as_ref() {
        Some(remapper) => remapper.rewrite_resource_opt(payload.resource),
        None => payload.resource,
    };
    let resource = encode_resource_id(&payload_resource).map_err(AuthorizationErr::BadRequest)?;

    let meta = get_meta(&state, &action, resource.as_ref()).await?;
    let (principal, lookup_kind) = resolve_request_principal(
        payload.principal,
        requester,
        &meta,
        &state.config.auth.principal_id_claim,
    );
    let attrs = get_authorization_metric_attrs(&action);

    if lookup_kind == PrincipalLookup::CrossPrincipal {
        if let Err(err) = ensure_permissions_can_be_viewed(
            &state,
            requester,
            AuthorizationRequest {
                principal: Some(&principal),
                action: Some(&action),
                resource: resource.as_ref(),
                context: payload.context.clone(),
            },
        )
        .await
        {
            record_denied(&state, &attrs, DenyReasonCategory::CrossPrincipalDenied);
            return Err(err);
        }
    }

    let eval_cache_key = EvaluationCacheKey::new(
        Some(principal.clone()),
        Some(action.clone()),
        resource.clone(),
        context_key(payload.context.as_ref()),
    );
    let cached = state.evaluation_cache.get(&eval_cache_key).await;
    state.metrics.record_evaluation_cache(cached.is_some());
    let (decision, reason) = if let Some((decision, reason)) = cached {
        (decision, reason)
    } else {
        let groups = principal_groups(&principal, &state.config.auth.principal_groups_claim);
        let resource_prefixes = hierarchy_prefixes(&meta, resource.as_ref());
        let policies = get_policies_by_parc(
            state.policies.clone(),
            &state.metrics,
            &action,
            &principal,
            &groups,
            resource.as_ref(),
            &resource_prefixes,
        )
        .await?;

        let prepared = prepare_principal(&principal, &state.config.auth.principal_groups_claim)
            .map_err(|err| {
                AuthorizationErr::Internal(format!("Failed to prepare principal: {}", err.msg))
            })?;

        let policy_refs: Vec<&Policy> = policies.iter().collect();
        let result = evaluate_default_sync(
            &policy_refs,
            &state,
            &action,
            &prepared,
            resource.as_ref(),
            &meta,
            payload.context.clone(),
            &attrs,
            &resource_prefixes,
        )?;

        state
            .evaluation_cache
            .set(eval_cache_key, result.clone())
            .await;
        result
    };
    tracing::debug!(
        ?decision,
        principal_id = %principal.id,
        "Returning decision for action '{}' to the client.",
        action
    );
    Ok(AuthorizationResponse::for_action(decision, reason, &action))
}

/// Whether a request targets the authenticated requester or a different
/// principal, decided after both sides have been resolved through the
/// target service's `Meta`. Used to gate `permissions:view`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PrincipalLookup {
    SelfPrincipal,
    CrossPrincipal,
}

/// Resolves both the authenticated requester and the request's target
/// principal in the target service's id space, then classifies the lookup
/// as self vs cross by comparing the resolved ids.
///
/// The requester's id is resolved from `requester.info` (verified token
/// claims, never the request body), so a caller cannot manipulate the
/// classification by spoofing their own resolved id. The target's id is
/// resolved from the caller-supplied payload (or defaults to the
/// requester when no payload principal is present).
///
/// Resolving both sides through the same `Meta` ensures that
/// `permissions:view` and Cedar evaluation see exactly the same target id
/// — closing the bypass where the caller picks one id for the view check
/// and a different one for the actual evaluation.
fn resolve_request_principal(
    payload_principal: Option<Principal>,
    requester: &Principal,
    meta: &Meta,
    default_id_claim: &str,
) -> (Principal, PrincipalLookup) {
    let requester_id = resolve_principal_id(
        requester.info.as_ref(),
        &requester.id,
        meta,
        default_id_claim,
    );

    let (target_id, target_info) = match payload_principal {
        None => (requester_id.clone(), requester.info.clone()),
        Some(p) => {
            let target_id = resolve_principal_id(p.info.as_ref(), &p.id, meta, default_id_claim);
            (target_id, p.info)
        }
    };

    let lookup_kind = if requester_id == target_id {
        PrincipalLookup::SelfPrincipal
    } else {
        PrincipalLookup::CrossPrincipal
    };

    (
        Principal {
            id: target_id,
            info: target_info,
        },
        lookup_kind,
    )
}

/// Resolves a principal id under a target service's `Meta` by walking the
/// claim chain (`meta.service.id_claim` → `default_id_claim` → `sub`)
/// against `info`. Falls back to `sub_fallback` (the wire `Principal.id`
/// field) when no chain claim is present.
fn resolve_principal_id(
    info: Option<&Value>,
    sub_fallback: &str,
    meta: &Meta,
    default_id_claim: &str,
) -> String {
    if let Some(Value::Object(info_obj)) = info {
        let claims: &[&str] = &[&meta.service.id_claim, default_id_claim, "sub"];
        for claim in claims {
            if claim.is_empty() {
                continue;
            }
            if let Some(Value::String(id)) = info_obj.get(*claim) {
                return id.clone();
            }
        }
    }
    sub_fallback.to_string()
}

type PrincipalMap = BTreeMap<String, Principal>;
type IndexedBatches<'a> = Vec<(usize, &'a AuthorizationBatch)>;
type PrincipalGroups<'a> = BTreeMap<String, IndexedBatches<'a>>;
type MetaCache = HashMap<(String, Option<String>), Meta>;
type PolicyCache = HashMap<(String, String), Arc<ServicePolicyIndex>>;
type EvaluationMap = HashMap<EvaluationCacheKey, EvaluationCacheValue>;
/// Per-principal-group memoization of resource hierarchy ancestor prefixes,
/// keyed by `(parent_kind, resource id)`. Populated while assembling the
/// policy-fetch prefix filter and reused during evaluation so a resource's URL
/// is parsed into ancestors once per principal group instead of once per phase.
type AncestorCache = HashMap<(String, String), Arc<Vec<Resource>>>;
/// Result of evaluating one parallel work unit: its batch index, action,
/// evaluation-cache key, and the decision/reason to cache and return.
type ParallelEvalResult = (usize, Action, EvaluationCacheKey, EvaluationCacheValue);

/// Per-`(service, principal)` policy set fetched once via `get_policies_batched`
/// and indexed so each resource in a batch resolves its applicable policies in
/// O(hierarchy depth) instead of rescanning and cloning the whole set. Index
/// entries are positions into `policies`, which stays in the fetched
/// `order ASC, id ASC` order, so a gathered subset restores that ordering via a
/// `BTreeSet`.
struct ServicePolicyIndex {
    policies: Vec<Policy>,
    unscoped: Vec<usize>,
    by_resource: HashMap<String, Vec<usize>>,
    by_prefix: HashMap<String, Vec<usize>>,
}

impl ServicePolicyIndex {
    fn build(policies: Vec<Policy>) -> Self {
        let mut unscoped = Vec::new();
        let mut by_resource: HashMap<String, Vec<usize>> = HashMap::new();
        let mut by_prefix: HashMap<String, Vec<usize>> = HashMap::new();
        for (position, policy) in policies.iter().enumerate() {
            if let Some(resource) = policy.resource.as_ref() {
                by_resource
                    .entry(resource.to_string())
                    .or_default()
                    .push(position);
            } else if let Some(prefix) = policy.resource_prefix.as_ref() {
                by_prefix
                    .entry(prefix.to_string())
                    .or_default()
                    .push(position);
            } else {
                unscoped.push(position);
            }
        }
        Self {
            policies,
            unscoped,
            by_resource,
            by_prefix,
        }
    }

    /// Collects the policies applicable to `(action, resource)` in ascending
    /// `(order, id)` order: unscoped policies, exact-resource matches, and
    /// hierarchy-prefix matches whose prefix is an ancestor of `resource`. A
    /// prefix whose folder is not an ancestor of `resource` can never satisfy
    /// the policy's `resource in <prefix>` head, so excluding it here is
    /// equivalent to the implicit non-match Cedar would otherwise produce.
    /// Action filtering happens here, keeping the buckets action-agnostic.
    fn policies_for(
        &self,
        action: &Action,
        resource: Option<&Resource>,
        ancestors: &[Resource],
    ) -> Vec<&Policy> {
        let mut positions: BTreeSet<usize> = self.unscoped.iter().copied().collect();
        if let Some(resource) = resource {
            if let Some(matches) = self.by_resource.get(&resource.to_string()) {
                positions.extend(matches.iter().copied());
            }
        }
        for ancestor in ancestors {
            if let Some(matches) = self.by_prefix.get(&ancestor.to_string()) {
                positions.extend(matches.iter().copied());
            }
        }

        positions
            .into_iter()
            .map(|position| &self.policies[position])
            .filter(|policy| {
                policy
                    .action
                    .as_ref()
                    .is_none_or(|a| a.entity_id() == action.entity_id())
            })
            .collect()
    }
}

/// Immutable per-request context shared by every helper on the hot path.
/// Passed by value (two pointers) since both fields are references.
#[derive(Clone, Copy)]
struct RequestCtx<'a> {
    state: &'a AppState,
    requester: &'a Principal,
}

/// Mutable in-memory caches scoped to a single `authorize_batch` invocation.
/// Passed by `&mut` through the per-principal-group loop; inner helpers
/// reborrow the fields they need. Unrelated to `permission_db` storage backends.
struct BatchCaches<'a> {
    principals: &'a mut PrincipalMap,
    meta: &'a MetaCache,
    policies: &'a mut PolicyCache,
}

/// Read-only borrow bundle consumed by `evaluate_action` for a single
/// action evaluation. Collapses three map references into one argument,
/// reducing argument-register pressure at the `Box::pin` boundary.
struct ActionCaches<'a> {
    evaluations: &'a EvaluationMap,
    policies: &'a PolicyCache,
    meta: &'a MetaCache,
    ancestors: &'a AncestorCache,
}

/// Per-service principal resolution, computed once per `(principal group, service)`
/// in [`evaluate_principal_group`] and shared by every action that targets the
/// service. Resolution is service-determined (the `id_claim` lives on the service
/// `Meta`, not the resource kind), so a single entry is reused for the whole
/// service. Holding a prebuilt [`EvaluationKeyBuilder`] lets the claims tree be
/// cloned and hashed once per service instead of once per action.
struct ResolvedPrincipal {
    principal: Principal,
    lookup_kind: PrincipalLookup,
    key_builder: EvaluationKeyBuilder,
    /// Lazily-built principal Cedar entity (with `Group` parents) reused across
    /// every action and policy for this `(principal group, service)`. Built on
    /// first evaluation that misses the decision cache, so a fully-cached
    /// service never pays the group-parent construction cost. Held in `Arc` so
    /// parallel per-resource workers can share one prepared principal.
    prepared: OnceCell<Arc<PreparedPrincipal>>,
}

/// One independent Cedar evaluation for the parallel `condition: None` path.
/// Shared policy index / prepared principal / meta are `Arc`-cloned so workers
/// do not duplicate the fetched policy set.
struct ParallelEvalWork {
    batch_index: usize,
    action: Action,
    resource: Option<Resource>,
    context: Option<Value>,
    key: EvaluationCacheKey,
    index: Arc<ServicePolicyIndex>,
    prepared: Arc<PreparedPrincipal>,
    meta: Meta,
    ancestors: Arc<Vec<Resource>>,
}

enum ParallelEvalPrep {
    Cached(AuthorizationDecision, Option<String>),
    NeedsEval(Box<ParallelEvalWork>),
}

type ResolvedPrincipals = HashMap<String, ResolvedPrincipal>;

/// Per-action inputs resolved once in [`evaluate_action`] and reused by
/// [`evaluate_uncached_action`]: the target service `Meta`, the request principal
/// in that service's id space (borrowed from the per-service [`ResolvedPrincipal`]),
/// the self/cross classification, and the precomputed evaluation-cache key.
struct ResolvedAction<'a> {
    meta: &'a Meta,
    principal: &'a Principal,
    prepared: &'a PreparedPrincipal,
    lookup_kind: PrincipalLookup,
    key: EvaluationCacheKey,
}

/// Outcome of scanning the evaluation cache for a per-service query list.
enum ScanOutcome {
    /// A cached decision in a pure-cached prefix satisfies the batch short-circuit
    /// condition. `query_batches` has been cleared; the DB policy fetch may be
    /// skipped for this service.
    ShortCircuit,
    /// No short-circuit reached; `query_batches` has been filtered to contain
    /// only entries that are NOT already in the evaluation cache.
    Proceed,
}

/// Per-action query collected from a batch request, carrying both the
/// DB-level `PolicyQueryBatch` and the canonicalized `ContextKey` of the
/// owning batch. The context key is required to look the right entry up
/// in the evaluation cache - two batches with identical `(action, resource)`
/// but distinct contexts must not share a cached decision.
struct ScannedQuery {
    query: PolicyQueryBatch,
    context_key: ContextKey,
}

#[tracing::instrument(level = "info", skip(requester, state))]
pub async fn authorize_batch(
    mut request: AuthorizationBatchRequest,
    state: AppState,
    requester: &Principal,
) -> Result<AuthorizationBatchResponse, AuthorizationErr> {
    for batch in &mut request.batches {
        if let Some(remapper) = state.url_remapper.as_ref() {
            batch.resource = remapper.rewrite_resource_opt(batch.resource.take());
        }
        batch.resource =
            encode_resource_id(&batch.resource).map_err(AuthorizationErr::BadRequest)?;
    }

    let condition = request.condition.unwrap_or_default();
    let mut response = AuthorizationBatchResponse {
        summary: initial_batch_summary(condition),
        decisions: vec![HashMap::new(); request.batches.len()],
    };

    let (mut principal_map, principal_groups) =
        group_batches_by_principal(&request.batches, requester);
    let meta_cache = prefetch_meta_cache(&state, &request.batches).await?;
    let mut policy_cache: PolicyCache = HashMap::new();
    let mut skip_remaining = false;

    let ctx = RequestCtx {
        state: &state,
        requester,
    };
    let mut caches = BatchCaches {
        principals: &mut principal_map,
        meta: &meta_cache,
        policies: &mut policy_cache,
    };

    for (sub, indexed_batches) in principal_groups {
        if skip_remaining {
            fill_skip_decisions(&mut response, &indexed_batches);
            continue;
        }
        skip_remaining = evaluate_principal_group(
            ctx,
            &mut caches,
            &sub,
            indexed_batches,
            condition,
            &mut response,
        )
        .await?;
    }

    tracing::debug!(
        summary = ?response.summary.as_ref().map(|s| &s.decision),
        batches = response.decisions.len(),
        "Returning batch authorization response to the client."
    );
    Ok(response)
}

/// Baseline summary response for an `And`/`Or` batch before any action is evaluated.
/// `None` condition does not use a summary; `Or` starts at `Deny` and flips to `Allow`
/// on first allow; `And` starts at `Allow` and flips to `Deny` on first deny.
fn initial_batch_summary(condition: AuthorizationBatchCondition) -> Option<AuthorizationResponse> {
    match condition {
        AuthorizationBatchCondition::None => None,
        AuthorizationBatchCondition::Or => Some(AuthorizationResponse::for_decision(
            AuthorizationDecision::Deny,
        )),
        AuthorizationBatchCondition::And => Some(AuthorizationResponse::for_decision(
            AuthorizationDecision::Allow,
        )),
    }
}

/// Groups request batches by principal id and deduplicates principals in
/// `principal_map`. Uses `BTreeMap` for deterministic iteration order, which
/// `And`/`Or` short-circuit semantics rely on.
///
/// Clones the full `Principal` only when the sub is first seen, saving one
/// `Principal` clone per duplicate sub compared to the prior implementation.
fn group_batches_by_principal<'a>(
    batches: &'a [AuthorizationBatch],
    requester: &Principal,
) -> (PrincipalMap, PrincipalGroups<'a>) {
    let mut principal_map: PrincipalMap = BTreeMap::new();
    let mut principal_groups: PrincipalGroups<'a> = BTreeMap::new();

    for (index, batch) in batches.iter().enumerate() {
        let sub = batch
            .principal
            .as_ref()
            .map(|p| p.id.clone())
            .unwrap_or_else(|| requester.id.clone());

        principal_groups
            .entry(sub.clone())
            .or_default()
            .push((index, batch));

        principal_map
            .entry(sub)
            .or_insert_with(|| batch.principal.clone().unwrap_or_else(|| requester.clone()));
    }

    (principal_map, principal_groups)
}

/// Pre-populates metadata per `(service, resource_kind)` so that per-action evaluation
/// never has to block on a metadata fetch.
async fn prefetch_meta_cache(
    state: &AppState,
    batches: &[AuthorizationBatch],
) -> Result<MetaCache, AuthorizationErr> {
    let mut meta_cache: MetaCache = HashMap::new();
    for batch in batches {
        for action in &batch.actions {
            let cache_key = (
                action.service.clone(),
                batch.resource.as_ref().map(|r| r.kind.clone()),
            );
            if let hash_map::Entry::Vacant(e) = meta_cache.entry(cache_key) {
                let meta = get_meta(state, action, batch.resource.as_ref()).await?;
                e.insert(meta);
            }
        }
    }
    Ok(meta_cache)
}

fn fill_skip_decisions(
    response: &mut AuthorizationBatchResponse,
    indexed_batches: &[(usize, &AuthorizationBatch)],
) {
    for (index, batch) in indexed_batches {
        response.decisions[*index] = skip_all_actions(batch);
    }
}

/// Processes all batches belonging to a single principal group:
/// 1. groups actions by service,
/// 2. prefetches cached evaluations and fetches policies per service,
/// 3. evaluates each action in each batch and updates `response`.
///
/// Returns the new `skip_remaining` state for the caller.
async fn evaluate_principal_group(
    ctx: RequestCtx<'_>,
    caches: &mut BatchCaches<'_>,
    sub: &str,
    indexed_batches: IndexedBatches<'_>,
    condition: AuthorizationBatchCondition,
    response: &mut AuthorizationBatchResponse,
) -> Result<bool, AuthorizationErr> {
    let mut service_batches = collect_service_query_batches(&indexed_batches);
    let mut evaluations: EvaluationMap = HashMap::new();
    let mut resolved_principals: ResolvedPrincipals = HashMap::new();
    let mut ancestor_cache: AncestorCache = HashMap::new();

    for (service, query_batches) in service_batches.iter_mut() {
        let group_principal = match caches.principals.get(sub) {
            Some(principal) => principal.clone(),
            None => {
                return Err(AuthorizationErr::Internal(format!(
                    "Failed to get information about \"{}\" principal.",
                    sub
                )));
            }
        };

        let meta_key = (
            service.clone(),
            query_batches
                .first()
                .and_then(|b| b.query.resource.as_ref())
                .map(|r| r.kind.clone()),
        );
        let (principal, lookup_kind) = match caches.meta.get(&meta_key) {
            Some(meta) => resolve_request_principal(
                Some(group_principal),
                ctx.requester,
                meta,
                &ctx.state.config.auth.principal_id_claim,
            ),
            None => (group_principal, PrincipalLookup::SelfPrincipal),
        };
        let key_builder = EvaluationKeyBuilder::new(Some(principal.clone()));

        scan_eval_cache(
            ctx.state,
            &key_builder,
            query_batches,
            &mut evaluations,
            condition,
        )
        .await;
        ensure_service_policies(
            ctx.state,
            caches.policies,
            &mut ancestor_cache,
            service,
            &principal,
            query_batches,
        )
        .await?;

        resolved_principals.insert(
            service.clone(),
            ResolvedPrincipal {
                principal,
                lookup_kind,
                key_builder,
                prepared: OnceCell::new(),
            },
        );
    }

    let action_caches = ActionCaches {
        evaluations: &evaluations,
        policies: caches.policies,
        meta: caches.meta,
        ancestors: &ancestor_cache,
    };

    if condition == AuthorizationBatchCondition::None {
        evaluate_batches_parallel(
            ctx,
            &indexed_batches,
            &action_caches,
            &resolved_principals,
            response,
        )
        .await?;
        return Ok(false);
    }

    let mut skip_remaining = false;
    for (index, batch) in indexed_batches {
        if skip_remaining {
            response.decisions[index] = skip_all_actions(batch);
            continue;
        }
        let response_item = evaluate_batch_actions(
            ctx,
            batch,
            &action_caches,
            &resolved_principals,
            condition,
            response,
            &mut skip_remaining,
        )
        .await?;
        response.decisions[index] = response_item;
    }

    Ok(skip_remaining)
}

/// Builds per-service `ScannedQuery` lists for a single principal group.
/// Clones the service key once per unique service (on first insertion) rather
/// than once per action. Each entry remembers its batch's context key so the
/// evaluation-cache scan can build the correct, context-aware key.
fn collect_service_query_batches(
    indexed_batches: &[(usize, &AuthorizationBatch)],
) -> BTreeMap<String, Vec<ScannedQuery>> {
    let mut service_batches: BTreeMap<String, Vec<ScannedQuery>> = BTreeMap::new();
    for &(_, batch) in indexed_batches {
        let batch_context_key = context_key(batch.context.as_ref());
        for action in &batch.actions {
            let scanned = ScannedQuery {
                query: PolicyQueryBatch {
                    action: action.clone(),
                    resource: batch.resource.clone(),
                },
                context_key: batch_context_key.clone(),
            };
            if let Some(list) = service_batches.get_mut(&action.service) {
                list.push(scanned);
            } else {
                service_batches.insert(action.service.clone(), vec![scanned]);
            }
        }
    }
    service_batches
}

/// Scans the evaluation cache for each query in `query_batches`:
/// - populates `evaluations` with cached decisions;
/// - short-circuits when a pure-cached prefix already satisfies the batch's
///   short-circuit condition (matches legacy behavior - once any uncached entry
///   is encountered, cache short-circuit is disabled for the remainder);
/// - otherwise retains only uncached entries in `query_batches`, so the DB
///   fetch never re-asks for decisions the evaluation cache already has.
async fn scan_eval_cache(
    state: &AppState,
    key_builder: &EvaluationKeyBuilder,
    query_batches: &mut Vec<ScannedQuery>,
    evaluations: &mut EvaluationMap,
    condition: AuthorizationBatchCondition,
) -> ScanOutcome {
    let mut cached_flags: Vec<bool> = Vec::with_capacity(query_batches.len());
    let mut has_uncached = false;
    let mut short_circuit = false;

    for batch in query_batches.iter() {
        let key = key_builder.key(
            Some(batch.query.action.clone()),
            batch.query.resource.clone(),
            batch.context_key.clone(),
        );
        let cached = state.evaluation_cache.get(&key).await;
        state.metrics.record_evaluation_cache(cached.is_some());
        match cached {
            Some(value) => {
                let can_short_circuit = !has_uncached && short_circuit_reached(condition, &value.0);
                evaluations.insert(key, value);
                cached_flags.push(true);
                if can_short_circuit {
                    short_circuit = true;
                    break;
                }
            }
            None => {
                has_uncached = true;
                cached_flags.push(false);
            }
        }
    }

    if short_circuit {
        query_batches.clear();
        return ScanOutcome::ShortCircuit;
    }

    let mut idx = 0;
    query_batches.retain(|_| {
        let keep = !cached_flags[idx];
        idx += 1;
        keep
    });

    ScanOutcome::Proceed
}

/// Returns `true` when the given decision satisfies the short-circuit condition for
/// the batch: `And` short-circuits on `Deny`, `Or` short-circuits on `Allow`.
fn short_circuit_reached(
    condition: AuthorizationBatchCondition,
    decision: &AuthorizationDecision,
) -> bool {
    matches!(
        (condition, decision),
        (
            AuthorizationBatchCondition::And,
            AuthorizationDecision::Deny
        ) | (
            AuthorizationBatchCondition::Or,
            AuthorizationDecision::Allow
        )
    )
}

/// Populates `policy_cache` for `(service, principal.id)` if not already present.
/// Skips the DB call entirely when `query_batches` is empty (every decision was
/// served from the evaluation cache).
async fn ensure_service_policies(
    state: &AppState,
    policy_cache: &mut PolicyCache,
    ancestor_cache: &mut AncestorCache,
    service: &str,
    principal: &Principal,
    query_batches: &[ScannedQuery],
) -> Result<(), AuthorizationErr> {
    let key = (service.to_owned(), principal.id.clone());
    let hash_map::Entry::Vacant(entry) = policy_cache.entry(key) else {
        return Ok(());
    };

    if query_batches.is_empty() {
        entry.insert(Arc::new(ServicePolicyIndex::build(vec![])));
        return Ok(());
    }

    let db_queries: Vec<PolicyQueryBatch> = query_batches.iter().map(|q| q.query.clone()).collect();
    let groups = principal_groups(principal, &state.config.auth.principal_groups_claim);

    let mut resource_prefixes: Vec<Resource> = Vec::new();
    for query in query_batches {
        let Some(resource) = query.query.resource.as_ref() else {
            continue;
        };
        let meta = get_meta(state, &query.query.action, Some(resource)).await?;
        for ancestor in cached_hierarchy_prefixes(ancestor_cache, &meta, Some(resource)).iter() {
            if !resource_prefixes.contains(ancestor) {
                resource_prefixes.push(ancestor.clone());
            }
        }
    }

    let policies = state
        .policies
        .get_policies_batched(Some(principal), &groups, &db_queries, &resource_prefixes)
        .await
        .map_err(|err| {
            AuthorizationErr::Internal(format!("Failed to get policies from database: {}", err))
        })?;
    state.metrics.policy_fetch_size.record(
        policies.len() as u64,
        &[
            KeyValue::new("service", service.to_owned()),
            KeyValue::new("source", "batch"),
        ],
    );
    entry.insert(Arc::new(ServicePolicyIndex::build(policies)));
    Ok(())
}

/// Evaluates all actions of a single batch, updating `response.summary` and
/// flipping `skip_remaining` the moment a decision satisfies the batch
/// short-circuit condition. Any actions after the short-circuit trigger are
/// filled with `Skip` responses; the triggering action itself keeps its real
/// decision.
async fn evaluate_batch_actions(
    ctx: RequestCtx<'_>,
    batch: &AuthorizationBatch,
    caches: &ActionCaches<'_>,
    resolved_principals: &ResolvedPrincipals,
    condition: AuthorizationBatchCondition,
    response: &mut AuthorizationBatchResponse,
    skip_remaining: &mut bool,
) -> Result<HashMap<String, AuthorizationResponse>, AuthorizationErr> {
    let mut response_item = HashMap::with_capacity(batch.actions.len());
    let batch_context_key = context_key(batch.context.as_ref());
    let mut actions_iter = batch.actions.iter();

    for action in actions_iter.by_ref() {
        let (decision, reason) = evaluate_action(
            ctx,
            action,
            batch,
            caches,
            &batch_context_key,
            resolved_principals,
        )
        .await?;

        let triggers = short_circuit_reached(condition, &decision);
        if triggers {
            response.summary = Some(AuthorizationResponse::for_decision(decision.clone()));
            *skip_remaining = true;
        }

        response_item.insert(
            action.entity_id(),
            AuthorizationResponse::for_action(decision, reason, action),
        );

        if triggers {
            break;
        }
    }

    for action in actions_iter {
        response_item.insert(action.entity_id(), AuthorizationResponse::skip(action));
    }

    Ok(response_item)
}

/// Evaluates every `(batch, action)` for `condition: None` with independent
/// Cedar work fanned out across Rayon inside `spawn_blocking`. Cache hits and
/// cross-principal gates stay on the async path; only CPU-bound policy
/// evaluation runs in parallel, sharing one `Arc<ServicePolicyIndex>` /
/// `Arc<PreparedPrincipal>` per `(service, principal)`.
async fn evaluate_batches_parallel(
    ctx: RequestCtx<'_>,
    indexed_batches: &[(usize, &AuthorizationBatch)],
    caches: &ActionCaches<'_>,
    resolved_principals: &ResolvedPrincipals,
    response: &mut AuthorizationBatchResponse,
) -> Result<(), AuthorizationErr> {
    let mut work = Vec::new();

    for &(batch_index, batch) in indexed_batches {
        let batch_context_key = context_key(batch.context.as_ref());
        let mut response_item = HashMap::with_capacity(batch.actions.len());

        for action in &batch.actions {
            match prepare_parallel_eval(
                ctx,
                batch_index,
                action,
                batch,
                caches,
                &batch_context_key,
                resolved_principals,
            )
            .await?
            {
                ParallelEvalPrep::Cached(decision, reason) => {
                    response_item.insert(
                        action.entity_id(),
                        AuthorizationResponse::for_action(decision, reason, action),
                    );
                }
                ParallelEvalPrep::NeedsEval(unit) => work.push(*unit),
            }
        }

        response.decisions[batch_index] = response_item;
    }

    if work.is_empty() {
        return Ok(());
    }

    // A single uncached resource gains nothing from the Rayon fan-out yet pays a
    // `spawn_blocking` handoff plus an extra task reschedule, so batches at or
    // below the configured threshold are evaluated directly on the current task.
    // Larger batches keep the parallel path.
    let parallel_threshold = ctx.state.config.service.parallel_evaluation_threshold;
    let results = if work.len() <= parallel_threshold {
        work.into_iter()
            .map(|unit| evaluate_parallel_work_unit(ctx.state, unit))
            .collect::<Vec<_>>()
    } else {
        let state = ctx.state.clone();
        tokio::task::spawn_blocking(move || {
            work.into_par_iter()
                .map(|unit| evaluate_parallel_work_unit(&state, unit))
                .collect::<Vec<_>>()
        })
        .await
        .map_err(|err| {
            AuthorizationErr::Internal(format!("Parallel policy evaluation failed: {err}"))
        })?
    };

    for (batch_index, action, key, result) in results {
        ctx.state.evaluation_cache.set(key, result.clone()).await;
        response.decisions[batch_index].insert(
            action.entity_id(),
            AuthorizationResponse::for_action(result.0, result.1, &action),
        );
    }

    Ok(())
}

/// Evaluates one [`ParallelEvalWork`] unit, fail-closing to `Deny` on an
/// evaluation error. Shared by the inline single-item path and the Rayon
/// fan-out, so it borrows `AppState` rather than owning a clone.
fn evaluate_parallel_work_unit(state: &AppState, unit: ParallelEvalWork) -> ParallelEvalResult {
    let ParallelEvalWork {
        batch_index,
        action,
        resource,
        context,
        key,
        index,
        prepared,
        meta,
        ancestors,
    } = unit;
    let policies = index.policies_for(&action, resource.as_ref(), &ancestors);
    let attrs = get_authorization_metric_attrs(&action);
    let result = match evaluate_default_sync(
        &policies,
        state,
        &action,
        prepared.as_ref(),
        resource.as_ref(),
        &meta,
        context,
        &attrs,
        &ancestors,
    ) {
        Ok(result) => result,
        Err(err) => {
            record_denied(state, &attrs, DenyReasonCategory::EvaluationError);
            (
                AuthorizationDecision::Deny,
                state
                    .config
                    .policies
                    .enable_deny_reason
                    .then(|| err.message().to_owned()),
            )
        }
    };
    (batch_index, action, key, result)
}

/// Resolves a single `(batch, action)` for the parallel path: serves evaluation-
/// cache hits (including after a cross-principal `permissions:view` gate) or
/// builds a [`ParallelEvalWork`] unit with shared `Arc` policy/principal state.
async fn prepare_parallel_eval(
    ctx: RequestCtx<'_>,
    batch_index: usize,
    action: &Action,
    batch: &AuthorizationBatch,
    caches: &ActionCaches<'_>,
    batch_context_key: &ContextKey,
    resolved_principals: &ResolvedPrincipals,
) -> Result<ParallelEvalPrep, AuthorizationErr> {
    let meta_key = (
        action.service.clone(),
        batch.resource.as_ref().map(|r| r.kind.clone()),
    );
    let Some(meta) = caches.meta.get(&meta_key) else {
        return Err(AuthorizationErr::Internal(format!(
            "Failed to get meta for evaluating \"{}\" action.",
            action
        )));
    };

    let Some(resolved_principal) = resolved_principals.get(&action.service) else {
        return Err(AuthorizationErr::Internal(format!(
            "Failed to resolve principal for \"{}\" service.",
            action.service
        )));
    };

    let key = resolved_principal.key_builder.key(
        Some(action.clone()),
        batch.resource.clone(),
        batch_context_key.clone(),
    );

    if resolved_principal.lookup_kind == PrincipalLookup::SelfPrincipal {
        if let Some((decision, reason)) = caches.evaluations.get(&key) {
            return Ok(ParallelEvalPrep::Cached(decision.clone(), reason.clone()));
        }
    }

    if resolved_principal.lookup_kind == PrincipalLookup::CrossPrincipal {
        if let Err(err) = ensure_permissions_can_be_viewed(
            ctx.state,
            ctx.requester,
            AuthorizationRequest {
                principal: Some(&resolved_principal.principal),
                action: Some(action),
                resource: batch.resource.as_ref(),
                context: batch.context.clone(),
            },
        )
        .await
        {
            let attrs = get_authorization_metric_attrs(action);
            record_denied(ctx.state, &attrs, DenyReasonCategory::CrossPrincipalDenied);
            let reason = ctx
                .state
                .config
                .policies
                .enable_deny_reason
                .then(|| err.message().to_owned());
            return Ok(ParallelEvalPrep::Cached(
                AuthorizationDecision::Deny,
                reason,
            ));
        }

        if let Some((decision, reason)) = caches.evaluations.get(&key) {
            return Ok(ParallelEvalPrep::Cached(decision.clone(), reason.clone()));
        }
    }

    let prepared = resolved_principal
        .prepared
        .get_or_try_init(|| async {
            prepare_principal(
                &resolved_principal.principal,
                &ctx.state.config.auth.principal_groups_claim,
            )
            .map(Arc::new)
            .map_err(|err| {
                AuthorizationErr::Internal(format!("Failed to prepare principal: {}", err.msg))
            })
        })
        .await?;

    let policy_cache_key = (
        action.service.clone(),
        resolved_principal.principal.id.clone(),
    );
    let Some(index) = caches.policies.get(&policy_cache_key) else {
        return Err(AuthorizationErr::Internal(format!(
            "Failed to get policies for evaluating \"{}\" action.",
            action
        )));
    };

    let ancestors = lookup_or_build_ancestors(caches.ancestors, meta, batch.resource.as_ref());

    Ok(ParallelEvalPrep::NeedsEval(Box::new(ParallelEvalWork {
        batch_index,
        action: action.clone(),
        resource: batch.resource.clone(),
        context: batch.context.clone(),
        key,
        index: Arc::clone(index),
        prepared: Arc::clone(prepared),
        meta: meta.clone(),
        ancestors,
    })))
}

/// Builds `action`'s evaluation-cache key from the per-service
/// [`ResolvedPrincipal`] (resolved once per `(principal group, service)` in
/// [`evaluate_principal_group`]) and the batch-level `context_key`, then serves a
/// self-principal cache hit without allocating the heavier evaluation future.
/// Cross-principal lookups and cache misses fall through to
/// [`evaluate_uncached_action`].
async fn evaluate_action(
    ctx: RequestCtx<'_>,
    action: &Action,
    batch: &AuthorizationBatch,
    caches: &ActionCaches<'_>,
    batch_context_key: &ContextKey,
    resolved_principals: &ResolvedPrincipals,
) -> Result<(AuthorizationDecision, Option<String>), AuthorizationErr> {
    let meta_key = (
        action.service.clone(),
        batch.resource.as_ref().map(|r| r.kind.clone()),
    );
    let Some(meta) = caches.meta.get(&meta_key) else {
        return Err(AuthorizationErr::Internal(format!(
            "Failed to get meta for evaluating \"{}\" action.",
            action
        )));
    };

    let Some(resolved_principal) = resolved_principals.get(&action.service) else {
        return Err(AuthorizationErr::Internal(format!(
            "Failed to resolve principal for \"{}\" service.",
            action.service
        )));
    };

    let key = resolved_principal.key_builder.key(
        Some(action.clone()),
        batch.resource.clone(),
        batch_context_key.clone(),
    );

    if resolved_principal.lookup_kind == PrincipalLookup::SelfPrincipal {
        if let Some((decision, reason)) = caches.evaluations.get(&key) {
            return Ok((decision.clone(), reason.clone()));
        }
    }

    let prepared = resolved_principal
        .prepared
        .get_or_try_init(|| async {
            prepare_principal(
                &resolved_principal.principal,
                &ctx.state.config.auth.principal_groups_claim,
            )
            .map(Arc::new)
            .map_err(|err| {
                AuthorizationErr::Internal(format!("Failed to prepare principal: {}", err.msg))
            })
        })
        .await?;

    let resolved = ResolvedAction {
        meta,
        principal: &resolved_principal.principal,
        prepared: prepared.as_ref(),
        lookup_kind: resolved_principal.lookup_kind,
        key,
    };
    evaluate_uncached_action(ctx, action, batch, caches, resolved).await
}

/// Evaluation path reached only on a cross-principal lookup or an
/// evaluation-cache miss: enforces `permissions:view` for cross-principal
/// requests, then returns the cached decision or evaluates policies and stores
/// the result.
async fn evaluate_uncached_action(
    ctx: RequestCtx<'_>,
    action: &Action,
    batch: &AuthorizationBatch,
    caches: &ActionCaches<'_>,
    resolved: ResolvedAction<'_>,
) -> Result<(AuthorizationDecision, Option<String>), AuthorizationErr> {
    let ResolvedAction {
        meta,
        principal,
        prepared,
        lookup_kind,
        key,
    } = resolved;

    if lookup_kind == PrincipalLookup::CrossPrincipal {
        if let Err(err) = ensure_permissions_can_be_viewed(
            ctx.state,
            ctx.requester,
            AuthorizationRequest {
                principal: Some(principal),
                action: Some(action),
                resource: batch.resource.as_ref(),
                context: batch.context.clone(),
            },
        )
        .await
        {
            let attrs = get_authorization_metric_attrs(action);
            record_denied(ctx.state, &attrs, DenyReasonCategory::CrossPrincipalDenied);
            let reason = ctx
                .state
                .config
                .policies
                .enable_deny_reason
                .then(|| err.message().to_owned());
            return Ok((AuthorizationDecision::Deny, reason));
        }

        if let Some((decision, reason)) = caches.evaluations.get(&key) {
            return Ok((decision.clone(), reason.clone()));
        }
    }

    let policy_cache_key = (action.service.clone(), principal.id.clone());
    let ancestors = lookup_or_build_ancestors(caches.ancestors, meta, batch.resource.as_ref());
    let action_policies: Vec<&Policy> = caches
        .policies
        .get(&policy_cache_key)
        .map(|index| index.policies_for(action, batch.resource.as_ref(), &ancestors))
        .unwrap_or_default();

    let attrs = get_authorization_metric_attrs(action);
    let result = match evaluate_default_sync(
        &action_policies,
        ctx.state,
        action,
        prepared,
        batch.resource.as_ref(),
        meta,
        batch.context.clone(),
        &attrs,
        &ancestors,
    ) {
        Ok(result) => result,
        Err(err) => {
            record_denied(ctx.state, &attrs, DenyReasonCategory::EvaluationError);
            (
                AuthorizationDecision::Deny,
                ctx.state
                    .config
                    .policies
                    .enable_deny_reason
                    .then(|| err.message().to_owned()),
            )
        }
    };

    ctx.state.evaluation_cache.set(key, result.clone()).await;
    Ok(result)
}

#[tracing::instrument(level = "info", skip(requester, state))]
pub async fn diagnostics_authorize(
    payload: AuthorizationPayload,
    state: AppState,
    requester: &Principal,
) -> Result<DiagnosticsAuthorizeResponse, AuthorizationErr> {
    let action = payload.action;
    let payload_resource = match state.url_remapper.as_ref() {
        Some(remapper) => remapper.rewrite_resource_opt(payload.resource),
        None => payload.resource,
    };
    let resource = encode_resource_id(&payload_resource).map_err(AuthorizationErr::BadRequest)?;

    ensure_diagnostics_allowed(&state, requester).await?;

    let meta = get_meta(&state, &action, resource.as_ref()).await?;
    let (principal, _lookup_kind) = resolve_request_principal(
        payload.principal,
        requester,
        &meta,
        &state.config.auth.principal_id_claim,
    );

    let groups = principal_groups(&principal, &state.config.auth.principal_groups_claim);
    let resource_prefixes = hierarchy_prefixes(&meta, resource.as_ref());
    let policies = get_policies_by_parc(
        state.policies.clone(),
        &state.metrics,
        &action,
        &principal,
        &groups,
        resource.as_ref(),
        &resource_prefixes,
    )
    .await?;

    let diagnostics_policies = policies
        .into_iter()
        .map(|p| DiagnosticsPolicy {
            id: p.id,
            order: p.order,
            policy: p.text,
            principal: p.principal,
            group: p.group,
            action: p.action,
            resource: p.resource,
        })
        .collect();

    Ok(DiagnosticsAuthorizeResponse {
        evaluation_priority: meta.resource.evaluation_priority,
        policies: diagnostics_policies,
    })
}

#[tracing::instrument(level = "info", skip(requester, state, request))]
pub async fn diagnostics_authorize_batch(
    request: AuthorizationBatchRequest,
    state: AppState,
    requester: &Principal,
) -> Result<DiagnosticsAuthorizeBatchResponse, AuthorizationErr> {
    ensure_diagnostics_allowed(&state, requester).await?;

    let mut results = Vec::with_capacity(request.batches.len());
    for batch in request.batches {
        let resource = match state.url_remapper.as_ref() {
            Some(remapper) => remapper.rewrite_resource_opt(batch.resource),
            None => batch.resource,
        };
        let resource = encode_resource_id(&resource).map_err(AuthorizationErr::BadRequest)?;

        let mut evaluation_priority = EvaluationPriority::default();
        let mut actions = Vec::with_capacity(batch.actions.len());
        for action in &batch.actions {
            let meta = get_meta(&state, action, resource.as_ref()).await?;
            evaluation_priority = meta.resource.evaluation_priority.clone();

            let (principal, _lookup_kind) = resolve_request_principal(
                batch.principal.clone(),
                requester,
                &meta,
                &state.config.auth.principal_id_claim,
            );

            let groups = principal_groups(&principal, &state.config.auth.principal_groups_claim);
            let resource_prefixes = hierarchy_prefixes(&meta, resource.as_ref());
            let policies = get_policies_by_parc(
                state.policies.clone(),
                &state.metrics,
                action,
                &principal,
                &groups,
                resource.as_ref(),
                &resource_prefixes,
            )
            .await?
            .into_iter()
            .map(|p| DiagnosticsPolicy {
                id: p.id,
                order: p.order,
                policy: p.text,
                principal: p.principal,
                group: p.group,
                action: p.action,
                resource: p.resource,
            })
            .collect();

            actions.push(DiagnosticsActionPolicies {
                service: action.service.clone(),
                action: action.name.clone(),
                policies,
            });
        }

        results.push(DiagnosticsBatchResult {
            resource,
            evaluation_priority,
            actions,
        });
    }

    Ok(DiagnosticsAuthorizeBatchResponse { results })
}

fn skip_all_actions(batch: &AuthorizationBatch) -> HashMap<String, AuthorizationResponse> {
    batch
        .actions
        .iter()
        .map(|a| (a.entity_id(), AuthorizationResponse::skip(a)))
        .collect()
}

fn get_authorization_metric_attrs(action: &Action) -> Vec<KeyValue> {
    vec![KeyValue::new("service", action.service.clone())]
}

/// Categorizes why an authorization decision was denied, as a strictly-bounded
/// set of `&'static str` values suitable for use as a metric attribute. Never
/// derived from user input, a Cedar policy body, or an error message - the
/// free-text `reason` field returned to API callers is a separate concern,
/// unaffected by this categorization.
#[derive(Clone, Copy)]
enum DenyReasonCategory {
    /// A policy group resolved to `Deny` with at least one diagnostic reason -
    /// an explicit `forbid` matched the request.
    ExplicitForbid,
    /// No policy group matched the request at all; denied by default.
    Implicit,
    /// A cross-principal `permissions:view` check failed, blocking the lookup
    /// before the underlying action was ever evaluated.
    CrossPrincipalDenied,
    /// An internal evaluation error (e.g. a Cedar error) was converted into a
    /// fail-closed deny instead of aborting the whole batch.
    EvaluationError,
}

impl DenyReasonCategory {
    fn as_str(self) -> &'static str {
        match self {
            Self::ExplicitForbid => "explicit_forbid",
            Self::Implicit => "implicit",
            Self::CrossPrincipalDenied => "cross_principal_denied",
            Self::EvaluationError => "evaluation_error",
        }
    }
}

fn record_denied(state: &AppState, attrs: &[KeyValue], category: DenyReasonCategory) {
    let mut combined_attrs = attrs.to_vec();
    combined_attrs.push(KeyValue::new("deny_reason", category.as_str()));
    state.metrics.authorization_denied.add(1, &combined_attrs);
}

#[tracing::instrument(level = "info", skip(state, prepared, attrs))]
/// Evaluates policies grouped by order (firewall-style).
/// Policies must arrive pre-sorted by order ASC, id ASC.
/// Within each order group, EvaluationPriority (Permit/Forbid) controls short-circuiting.
/// Once any policy in a group matches, that group's result is final.
/// Sync so Rayon workers can call it from `spawn_blocking` without an async runtime.
#[allow(clippy::too_many_arguments)]
fn evaluate_default_sync(
    policies: &[&Policy],
    state: &AppState,
    action: &Action,
    prepared: &PreparedPrincipal,
    resource: Option<&Resource>,
    meta: &Meta,
    context: Option<Value>,
    attrs: &[KeyValue],
    ancestors: &[Resource],
) -> Result<(AuthorizationDecision, Option<String>), AuthorizationErr> {
    let service_attr = [KeyValue::new("service", action.service.clone())];
    state
        .metrics
        .policy_evaluation_depth
        .record(policies.len() as u64, &service_attr);

    let mut resource_parents: Option<Vec<_>> = None;
    let mut groups_scanned: u64 = 0;
    for group in policies.chunk_by(|a, b| a.order == b.order) {
        groups_scanned += 1;
        let mut decision: PolicyDecision = PolicyDecision::Deny;
        let mut deny_reason = None;
        let mut matched = false;

        for policy in group {
            let parents: &[_] = if policy.resource_prefix.is_none() {
                &[]
            } else if let Some(parents) = resource_parents.as_deref() {
                parents
            } else {
                resource_parents
                    .insert(build_resource_parents(ancestors).map_err(|err| {
                        AuthorizationErr::Internal(format!(
                            "Failed to build resource hierarchy parents: {}",
                            err.msg
                        ))
                    })?)
                    .as_slice()
            };
            let (policy_set, policy_set_hit) = match state
                .policy_set_cache
                .get_or_parse(&policy.policy_hash, &policy.text)
            {
                Ok(result) => result,
                Err(PolicyErr {
                    kind: PolicyErrKind::Unknown,
                    msg,
                }) => {
                    return Err(AuthorizationErr::Internal(format!(
                        "Failed to evaluate the policy: {}",
                        msg
                    )))
                }
                Err(PolicyErr { msg, .. }) => return Err(AuthorizationErr::BadRequest(msg)),
            };
            state.metrics.record_policy_set_cache(policy_set_hit);
            match evaluate_policy_set_with_principal(
                policy_set.as_ref(),
                &state.authorizer,
                prepared,
                action,
                resource,
                parents,
                context.clone(),
            ) {
                Ok(response) => {
                    let errors: Vec<String> = response
                        .diagnostics()
                        .errors()
                        .map(|err| err.to_string())
                        .collect();
                    if !errors.is_empty() {
                        tracing::warn!(
                            policy_id = policy.id,
                            order = policy.order,
                            "Ignoring policy '{}' for action '{}' that failed to evaluate: {}",
                            policy.id,
                            action,
                            errors.join("; ")
                        );
                        continue;
                    }

                    let policy_decision = response.decision();
                    tracing::debug!(
                        policy_id = policy.id,
                        order = policy.order,
                        decision = ?policy_decision,
                        "Evaluated policy '{}' for action '{}'.",
                        policy.id,
                        action
                    );
                    // A policy's `Decision::Deny` with zero diagnostic reasons is Cedar's
                    // implicit deny for a `permit` whose condition simply did not hold - it
                    // does not contribute to this group's outcome. `decision`/`matched` are
                    // only updated on a real contribution (an `Allow` match, or a `Deny` with
                    // an explicit forbid reason), so a later non-matching policy in the same
                    // order group can never overwrite an earlier real match.
                    match policy_decision {
                        PolicyDecision::Allow => {
                            decision = policy_decision;
                            matched = true;
                            if meta.resource.evaluation_priority == EvaluationPriority::Permit {
                                break;
                            }
                        }
                        PolicyDecision::Deny => {
                            let reasons = response.diagnostics().reason().count();
                            if reasons == 0 {
                                continue;
                            }
                            decision = policy_decision;
                            matched = true;
                            if state.config.policies.enable_deny_reason
                                && response.diagnostics().reason().next().is_some()
                            {
                                deny_reason = Some("Explicit deny.".to_string());
                            }
                            if meta.resource.evaluation_priority == EvaluationPriority::Forbid {
                                break;
                            }
                        }
                    }
                }
                Err(PolicyErr {
                    kind: PolicyErrKind::Unknown,
                    msg,
                }) => {
                    return Err(AuthorizationErr::Internal(format!(
                        "Failed to evaluate the policy: {}",
                        msg
                    )))
                }
                Err(PolicyErr { msg, .. }) => return Err(AuthorizationErr::BadRequest(msg)),
            }
        }

        if matched {
            match decision {
                PolicyDecision::Allow => state.metrics.authorization_allowed.add(1, attrs),
                PolicyDecision::Deny => {
                    record_denied(state, attrs, DenyReasonCategory::ExplicitForbid)
                }
            }
            state
                .metrics
                .policy_evaluation_groups_scanned
                .record(groups_scanned, &service_attr);
            let resolved = AuthorizationDecision::from(decision);
            tracing::debug!(
                ?resolved,
                "Resolved decision for action '{}' by matched policy group.",
                action
            );
            return Ok((
                resolved,
                if decision == PolicyDecision::Deny {
                    deny_reason
                } else {
                    None
                },
            ));
        }
    }

    let decision = PolicyDecision::Deny;
    record_denied(state, attrs, DenyReasonCategory::Implicit);
    state
        .metrics
        .policy_evaluation_groups_scanned
        .record(groups_scanned, &service_attr);
    let resolved = AuthorizationDecision::from(decision);
    tracing::debug!(
        ?resolved,
        "Resolved decision for action '{}' by implicit deny (no policy matched).",
        action
    );
    Ok((resolved, None))
}

#[tracing::instrument(level = "debug", skip(state))]
async fn get_meta(
    state: &AppState,
    action: &Action,
    resource: Option<&Resource>,
) -> Result<Meta, AuthorizationErr> {
    // Meta actions carry the intrinsic `permissions` service, which owns no
    // resource types. Resolve the target's hierarchy from its globally-unique
    // kind so a client `permissions:edit`/`permissions:view` probe walks the
    // same `Folder` ancestors the internal enforcement path uses.
    if action.service == PERMISSIONS_SERVICE_NAME {
        return meta_hierarchy_for_resource(state, resource).await;
    }

    let cache_key = (action.service.clone(), resource.map(|r| r.kind.clone()));

    let cached = state.metadata_cache.get(&cache_key).await;
    state.metrics.record_metadata_cache(cached.is_some());
    if let Some(meta) = cached {
        return Ok(meta);
    }

    let meta = state
        .metadata
        .get_meta(action, resource)
        .await
        .map_err(|err| AuthorizationErr::Internal(err.to_string()))?;

    state.metadata_cache.set(cache_key, meta.clone()).await;
    Ok(meta)
}

/// Service name under which the permission-service's own inner actions
/// and resource types are registered in the metadata storage.
pub const PERMISSIONS_SERVICE_NAME: &str = "permissions";

pub fn get_view_permissions_action() -> &'static Action {
    static VIEW_PERMISSIONS_ACTION: OnceLock<Action> = OnceLock::new();
    VIEW_PERMISSIONS_ACTION.get_or_init(|| Action {
        name: "view".to_string(),
        service: PERMISSIONS_SERVICE_NAME.to_string(),
    })
}

pub fn get_edit_permissions_action() -> &'static Action {
    static EDIT_PERMISSIONS_ACTION: OnceLock<Action> = OnceLock::new();
    EDIT_PERMISSIONS_ACTION.get_or_init(|| Action {
        name: "edit".to_string(),
        service: PERMISSIONS_SERVICE_NAME.to_string(),
    })
}

pub fn get_read_metadata_action() -> &'static Action {
    static READ_METADATA_ACTION: OnceLock<Action> = OnceLock::new();
    READ_METADATA_ACTION.get_or_init(|| Action {
        name: "meta:read".to_string(),
        service: PERMISSIONS_SERVICE_NAME.to_string(),
    })
}

pub fn get_write_metadata_action() -> &'static Action {
    static WRITE_METADATA_ACTION: OnceLock<Action> = OnceLock::new();
    WRITE_METADATA_ACTION.get_or_init(|| Action {
        name: "meta:write".to_string(),
        service: PERMISSIONS_SERVICE_NAME.to_string(),
    })
}

pub fn get_diagnostics_permissions_action() -> &'static Action {
    static DIAGNOSTICS_PERMISSIONS_ACTION: OnceLock<Action> = OnceLock::new();
    DIAGNOSTICS_PERMISSIONS_ACTION.get_or_init(|| Action {
        name: "diagnostics".to_string(),
        service: PERMISSIONS_SERVICE_NAME.to_string(),
    })
}

/// Returns the metadata describing the permission-service's own inner
/// actions and resource types. This is the single source of truth for the
/// intrinsic `permissions` service entry that gets layered on top of the
/// operator-supplied configuration during bootstrap so the service is
/// discoverable via the metadata APIs without forcing operators to repeat
/// it in their metadata file.
pub fn permission_service_metadata() -> MetadataConfig {
    MetadataConfig {
        services: vec![MetadataServiceConfig {
            name: PERMISSIONS_SERVICE_NAME.to_string(),
            principal: MetadataPrincipalConfig::default(),
            actions: vec![
                get_view_permissions_action().name.clone(),
                get_edit_permissions_action().name.clone(),
                get_read_metadata_action().name.clone(),
                get_write_metadata_action().name.clone(),
                get_diagnostics_permissions_action().name.clone(),
            ],
            resource_types: Vec::new(),
        }],
    }
}

/// Builds the Cedar `context` describing the policy being added or deleted, so
/// `permissions:edit` policies can be scoped to the changed policy's principal,
/// group, or action via `context.principal`, `context.group`, `context.action`.
/// The `resource` scope is evaluated directly against the real resource head
/// (`resource == <Kind>::"..."` / `resource in <ParentType>::"..."`).
pub fn build_edit_context(
    principal: Option<&Principal>,
    group: Option<&Group>,
    action: Option<&Action>,
) -> Value {
    json!({
        "principal": principal.map_or_else(|| json!({}), |principal| json!({ "id": principal.id })),
        "group": group.map_or_else(|| json!({}), |group| json!({ "id": group.id })),
        "action": action.map_or_else(|| json!({}), |action| json!({
            "name": action.name,
            "service": action.service,
        })),
    })
}

#[tracing::instrument(level = "info", skip(state))]
pub async fn ensure_permissions_can_be_changed(
    state: &AppState,
    principal: &Principal,
    resource: Option<&Resource>,
    context: Option<Value>,
) -> Result<(), AuthorizationErr> {
    if !state.config.auth.enabled {
        return Ok(());
    }

    let action = get_edit_permissions_action();
    evaluate_meta_permissions(state, principal, action, resource, context).await
}

#[tracing::instrument(level = "info", skip(state))]
pub async fn ensure_metadata_can_be_read(
    state: &AppState,
    principal: &Principal,
) -> Result<(), AuthorizationErr> {
    if !state.config.auth.enabled {
        return Ok(());
    }

    let action = get_read_metadata_action();
    evaluate_meta_permissions(state, principal, action, None, None).await
}

#[tracing::instrument(level = "info", skip(state))]
pub async fn ensure_metadata_can_be_changed(
    state: &AppState,
    principal: &Principal,
) -> Result<(), AuthorizationErr> {
    if !state.config.auth.enabled {
        return Ok(());
    }

    let action = get_write_metadata_action();
    evaluate_meta_permissions(state, principal, action, None, None).await
}

#[tracing::instrument(level = "info", skip(state))]
pub async fn ensure_diagnostics_allowed(
    state: &AppState,
    principal: &Principal,
) -> Result<(), AuthorizationErr> {
    if !state.config.auth.enabled {
        return Ok(());
    }

    let action = get_diagnostics_permissions_action();
    evaluate_meta_permissions(state, principal, action, None, None).await
}

/// Shared gate for the meta-permission actions above (`permissions:edit`,
/// `permissions:meta:read`, `permissions:meta:write`,
/// `permissions:diagnostics`). Delegates to [`evaluate_meta_decision`] so
/// these checks are evaluated exactly like `authorize` - grouped by `order`
/// with `evaluation_priority` short-circuiting, cached in the evaluation
/// cache - rather than combining every matching policy into a single Cedar
/// policy set that discards ordering.
async fn evaluate_meta_permissions(
    state: &AppState,
    principal: &Principal,
    action: &Action,
    resource: Option<&Resource>,
    context: Option<Value>,
) -> Result<(), AuthorizationErr> {
    let (decision, _reason) =
        evaluate_meta_decision(state, principal, action, resource, context).await?;
    match decision {
        AuthorizationDecision::Allow => Ok(()),
        _ => Err(AuthorizationErr::Forbidden("Permission denied".to_string())),
    }
}

#[derive(Default)]
pub struct AuthorizationRequest<'a> {
    pub principal: Option<&'a Principal>,
    pub action: Option<&'a Action>,
    pub resource: Option<&'a Resource>,
    pub context: Option<Value>,
}

#[tracing::instrument(level = "debug", skip(state, payload))]
pub async fn ensure_permissions_can_be_viewed(
    state: &AppState,
    principal: &Principal,
    payload: AuthorizationRequest<'_>,
) -> Result<(), AuthorizationErr> {
    if !state.config.auth.enabled {
        return Ok(());
    }

    if let Some(payload_principal) = payload.principal {
        if principal.id == payload_principal.id {
            // Users can see their own permissions by design
            return Ok(());
        }
    }

    let action = get_view_permissions_action();
    let context = json!({
        "principal": payload.principal.map_or_else(|| json!({}), |principal| json!({
            "id": principal.id,
        })),
        "action": payload.action.map_or_else(|| json!({}), |action| json!({
            "name": action.name,
            "service": action.service,
        })),
    });

    let (decision, _reason) =
        evaluate_meta_decision(state, principal, action, payload.resource, Some(context)).await?;
    match decision {
        AuthorizationDecision::Allow => Ok(()),
        _ => Err(AuthorizationErr::Forbidden(format!(
            "Permission denied for evaluating {} for {}",
            action, principal
        ))),
    }
}

/// Builds the `Meta` used to compute hierarchy ancestors for a meta-permission
/// check. Meta checks always deny by default (`Forbid`), so only the resource's
/// `parentType` is resolved - from the resource `kind` alone, since resource
/// type kinds are globally unique. The synthetic `Meta` is cached under the
/// intrinsic `permissions` service so repeated checks avoid a metadata lookup;
/// resource-type mutations invalidate the `permissions` metadata cache.
async fn meta_hierarchy_for_resource(
    state: &AppState,
    resource: Option<&Resource>,
) -> Result<Meta, AuthorizationErr> {
    let cache_key = (
        PERMISSIONS_SERVICE_NAME.to_string(),
        resource.map(|r| r.kind.clone()),
    );

    let cached = state.metadata_cache.get(&cache_key).await;
    state.metrics.record_metadata_cache(cached.is_some());
    if let Some(meta) = cached {
        return Ok(meta);
    }

    let parent_type = match resource {
        Some(r) => state
            .metadata
            .get_resource_type_by_kind(&r.kind)
            .await
            .map_err(|err| AuthorizationErr::Internal(err.to_string()))?
            .and_then(|rt| rt.parent_type),
        None => None,
    };

    let meta = Meta {
        service: ServiceMeta::new(PERMISSIONS_SERVICE_NAME),
        resource: ResourceTypeMeta {
            service: PERMISSIONS_SERVICE_NAME.to_string(),
            kind: resource.map(|r| r.kind.clone()).unwrap_or_default(),
            evaluation_priority: EvaluationPriority::Forbid,
            parent_type,
        },
    };

    state.metadata_cache.set(cache_key, meta.clone()).await;
    Ok(meta)
}

/// Core evaluator shared by [`evaluate_meta_permissions`] and
/// [`ensure_permissions_can_be_viewed`]. Mirrors the `authorize` evaluation
/// path: resolves hierarchy `Meta` for the real resource, serves a cached
/// decision when available, and otherwise fetches the matching policies,
/// prepares the principal once, and evaluates them grouped by `order` via
/// [`evaluate_default_sync`]. Meta checks always deny by default, so the
/// resource's own `evaluation_priority` is not consulted.
async fn evaluate_meta_decision(
    state: &AppState,
    principal: &Principal,
    action: &Action,
    resource: Option<&Resource>,
    context: Option<Value>,
) -> Result<(AuthorizationDecision, Option<String>), AuthorizationErr> {
    let groups_claim = &state.config.auth.principal_groups_claim;
    let meta = meta_hierarchy_for_resource(state, resource).await?;

    let eval_cache_key = EvaluationCacheKey::new(
        Some(principal.clone()),
        Some(action.clone()),
        resource.cloned(),
        context_key(context.as_ref()),
    );
    let cached = state.evaluation_cache.get(&eval_cache_key).await;
    state.metrics.record_evaluation_cache(cached.is_some());
    if let Some(value) = cached {
        return Ok(value);
    }

    let groups = principal_groups(principal, groups_claim);
    let resource_prefixes = hierarchy_prefixes(&meta, resource);
    let policies = get_policies_by_parc(
        state.policies.clone(),
        &state.metrics,
        action,
        principal,
        &groups,
        resource,
        &resource_prefixes,
    )
    .await?;

    let prepared = prepare_principal(principal, groups_claim).map_err(|err| {
        AuthorizationErr::Internal(format!("Failed to prepare principal: {}", err.msg))
    })?;

    let attrs = get_authorization_metric_attrs(action);
    let policy_refs: Vec<&Policy> = policies.iter().collect();
    let result = evaluate_default_sync(
        &policy_refs,
        state,
        action,
        &prepared,
        resource,
        &meta,
        context,
        &attrs,
        &resource_prefixes,
    )?;

    state
        .evaluation_cache
        .set(eval_cache_key, result.clone())
        .await;
    Ok(result)
}

#[tracing::instrument(level = "info", skip(storage, config))]
pub async fn set_system_policies(storage: &mut impl PolicyStorage, config: &ServiceConfig) {
    if config.database.init.policies_file.is_empty() {
        return;
    }

    let content =
        std::fs::read_to_string(&config.database.init.policies_file).unwrap_or_else(|e| {
            panic!(
                "Unable to read init policies file '{}': {}",
                config.database.init.policies_file, e
            )
        });

    let parsed: PolicyConfig = serde_norway::from_str(&content)
        .expect("Unable to parse system policies: must be valid YAML in the policy configuration file format");

    let url_remapper = if config.url_remapper.enabled {
        let remapper_config = load_url_remapper_config(&config.url_remapper.file);
        UrlRemapper::from_config(remapper_config).unwrap_or_else(|e| {
            tracing::error!("Invalid URL remapper configuration: {}", e);
            std::process::exit(1);
        })
    } else {
        None
    };

    let policies: Vec<PolicyDbBatch> = parsed
        .policies
        .into_iter()
        .map(|entry| {
            let comment = entry.comment;
            let mut policy = entry.policy;
            if let Some(remapper) = url_remapper.as_ref() {
                policy = remapper
                    .rewrite_policy_text(&policy)
                    .expect("Unable to apply URL remapping to system policy");
            }
            // System policies always validate scopes - matches the previous
            // two-step "infer then validate" flow but in a single Cedar parse.
            let ProcessedPolicy {
                text,
                scopes,
                canonical_hash,
            } = process_policy(&policy, true)
                .expect("Unable to process system policy through Cedar");
            PolicyDbBatch {
                policy: text,
                action: scopes.action,
                principal: scopes.principal,
                group: scopes.group,
                resource: scopes.resource,
                resource_prefix: scopes.resource_prefix,
                order: entry.order.unwrap_or(0),
                system: true,
                created_by: "SYSTEM".to_string(),
                comment,
                canonical_hash,
            }
        })
        .collect();

    tracing::info!("Assigning system policies: {:#?}", policies);
    storage.set_system_policies(policies).await.unwrap();
}

#[tracing::instrument(level = "info", skip(storage, config))]
pub async fn set_system_metadata(storage: &impl MetadataStorage, config: &ServiceConfig) {
    let mut parsed = match config.database.database_type {
        // The config-file backend already loads the metadata file directly;
        // we only need to layer the permission-service's intrinsic entries
        // on top so the `permissions` service is discoverable without
        // re-parsing or duplicating the file's contents.
        DatabaseType::ConfigFile => MetadataConfig {
            services: Vec::new(),
        },
        DatabaseType::Postgres => {
            if config.database.init.metadata_file.is_empty() {
                MetadataConfig {
                    services: Vec::new(),
                }
            } else {
                let content = std::fs::read_to_string(&config.database.init.metadata_file)
                    .unwrap_or_else(|e| {
                        panic!(
                            "Unable to read init metadata file '{}': {}",
                            config.database.init.metadata_file, e
                        )
                    });

                serde_norway::from_str(&content).expect(
                    "Unable to parse system metadata: must be valid YAML in the metadata configuration file format",
                )
            }
        }
    };

    parsed.merge(&permission_service_metadata());

    parsed
        .validate_unique_resource_types()
        .unwrap_or_else(|msg| panic!("Invalid system metadata: {}", msg));

    tracing::info!("Assigning system metadata: {:#?}", parsed);
    storage.set_system_metadata(parsed).await.unwrap();
}

/// Computes the folder-hierarchy ancestor prefixes for `resource`, typing each
/// ancestor with the resource's configured parent type (`parentType`, falling
/// back to the resource's own kind). Empty when `resource` is `None` or not a
/// URL-shaped id. `resource.id` is expected to already be remapped and encoded.
pub fn hierarchy_prefixes(meta: &Meta, resource: Option<&Resource>) -> Vec<Resource> {
    match resource {
        Some(r) => resource_hierarchy_ancestors(meta.resource.parent_kind(), &r.id),
        None => Vec::new(),
    }
}

/// [`hierarchy_prefixes`] memoized in `cache`, inserting on a miss. Used by the
/// policy-fetch phase to seed the cache so the subsequent evaluation phase can
/// reuse the ancestors via [`lookup_or_build_ancestors`] without re-parsing the
/// resource URL.
fn cached_hierarchy_prefixes(
    cache: &mut AncestorCache,
    meta: &Meta,
    resource: Option<&Resource>,
) -> Arc<Vec<Resource>> {
    let Some(r) = resource else {
        return Arc::new(Vec::new());
    };
    let parent_kind = meta.resource.parent_kind();
    cache
        .entry((parent_kind.to_string(), r.id.clone()))
        .or_insert_with(|| Arc::new(resource_hierarchy_ancestors(parent_kind, &r.id)))
        .clone()
}

/// Read-only counterpart of [`cached_hierarchy_prefixes`] for the evaluation
/// phase: returns the ancestors memoized during policy fetch, falling back to a
/// fresh computation on the rare miss (e.g. a policy set already cached from an
/// earlier request whose prefixes were never seeded this call).
fn lookup_or_build_ancestors(
    cache: &AncestorCache,
    meta: &Meta,
    resource: Option<&Resource>,
) -> Arc<Vec<Resource>> {
    let Some(r) = resource else {
        return Arc::new(Vec::new());
    };
    let parent_kind = meta.resource.parent_kind();
    match cache.get(&(parent_kind.to_string(), r.id.clone())) {
        Some(ancestors) => Arc::clone(ancestors),
        None => Arc::new(resource_hierarchy_ancestors(parent_kind, &r.id)),
    }
}

#[tracing::instrument(level = "debug", skip(storage, metrics))]
pub async fn get_policies_by_parc(
    storage: Arc<dyn PolicyStorage>,
    metrics: &Metrics,
    action: &Action,
    principal: &Principal,
    groups: &[String],
    resource: Option<&Resource>,
    resource_prefixes: &[Resource],
) -> Result<Vec<Policy>, AuthorizationErr> {
    let policies = match storage
        .get_policies(
            Some(action),
            Some(principal),
            groups,
            resource,
            resource_prefixes,
        )
        .await
    {
        Ok(policies) => policies,
        Err(err) => {
            return Err(AuthorizationErr::Internal(format!(
                "Failed to get policies from the database: {err}"
            )))
        }
    };

    metrics.policy_fetch_size.record(
        policies.len() as u64,
        &[
            KeyValue::new("service", action.service.clone()),
            KeyValue::new("source", "single"),
        ],
    );
    tracing::debug!("Found {} policies for '{}' action.", policies.len(), action);
    Ok(policies)
}

/// Extracts the principal's group memberships from `principal.info[claim]`,
/// the same claim used to attach `Group::"<id>"` parents during evaluation.
/// The claim must be a JSON array of strings; anything else yields no groups.
#[tracing::instrument(level = "debug")]
pub fn encode_resource_id(resource: &Option<Resource>) -> Result<Option<Resource>, String> {
    resource
        .as_ref()
        .map(|res| {
            Ok(Resource {
                id: encode_resource_url(&res.id)?,
                kind: res.kind.clone(),
                data: res.data.clone(),
            })
        })
        .transpose()
}

/// Like [`encode_resource_id`] but preserves a trailing `*` used by prefix
/// listing queries. Only the literal prefix before the `*` is URL-encoded so it
/// lines up with the stored, already-encoded resource ids; the `*` marker is
/// re-appended for the storage layer to expand into a prefix range. Without
/// this, encoding a URL-shaped id would percent-encode the `*` (`%2A`) and the
/// wildcard would no longer be recognized.
#[tracing::instrument(level = "debug")]
pub fn encode_resource_id_prefix(resource: &Option<Resource>) -> Result<Option<Resource>, String> {
    resource
        .as_ref()
        .map(|res| {
            let (prefix, wildcard) = match res.id.strip_suffix('*') {
                Some(prefix) => (prefix, true),
                None => (res.id.as_str(), false),
            };
            let mut id = encode_resource_url(prefix)?;
            if wildcard {
                id.push('*');
            }
            Ok(Resource {
                id,
                kind: res.kind.clone(),
                data: res.data.clone(),
            })
        })
        .transpose()
}
