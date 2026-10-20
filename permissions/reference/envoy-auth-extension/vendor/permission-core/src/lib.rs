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

use crate::util::encoding::encode_resource_url;
use cedar_policy::{
    ActionConstraint, Authorizer, Context, Decision, Entities, Entity, EntityId, EntityTypeName,
    EntityUid, Policy, PolicySet, PrincipalConstraint, Request, RequestBuilder, ResourceConstraint,
    Response, RestrictedExpression, UnsetSchema,
};
use garde::Validate;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fmt::{Debug, Display, Formatter};
use std::str::FromStr;
use std::sync::OnceLock;
use tracing::debug_span;
use url::{Position, Url};
use utoipa::ToSchema;

pub use util::cache::{PolicySetCache, PolicyTextKey};

#[derive(
    Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq, Hash, ToSchema, Validate,
)]
pub struct Action {
    #[schema(max_length = 255, pattern = "^[a-zA-Z0-9_.:/-]+$")]
    #[garde(length(max = 255), pattern("^[a-zA-Z0-9_.:/-]+$"))]
    pub name: String,

    #[schema(max_length = 255, pattern = "^[a-zA-Z0-9_.:/-]+$")]
    #[garde(length(max = 255), pattern("^[a-zA-Z0-9_.:/-]+$"))]
    pub service: String,
}

impl Action {
    pub fn entity_id(&self) -> String {
        format!("{}:{}", self.service, self.name)
    }
}

impl Display for Action {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if self.service.is_empty() || self.name.is_empty() {
            return write!(f, "");
        }
        write!(f, "Action::\"{}:{}\"", self.service, self.name)
    }
}

impl FromStr for Action {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() || s == "NULL" {
            return Ok(Action::default());
        }

        let parts: Vec<&str> = s.splitn(2, ':').collect();
        if parts.len() != 2 {
            return Err(format!(
                "Failed to parse Action from string \"{s}\" - \
                    must be in \"service:name\" format."
            ));
        }

        Ok(Action {
            service: parts[0].to_string(),
            name: parts[1].to_string(),
        })
    }
}

#[derive(Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, Validate)]
pub struct Principal {
    #[serde(rename = "sub")]
    #[schema(max_length = 255, pattern = "^[^\\x00-\\x1F\\x22\\x27\\x5C]+$")]
    #[garde(length(max = 255), pattern("^[^\\x00-\\x1F\\x22\\x27\\x5C]+$"))]
    pub id: String,

    #[garde(skip)]
    pub info: Option<Value>,
}

impl Display for Principal {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "Principal::\"{}\"", self.id)
    }
}

impl Debug for Principal {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if self.id.is_empty() {
            return write!(f, "");
        }
        write!(f, "Principal::\"{}\"", self.id)
    }
}

impl FromStr for Principal {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() || s == "NULL" {
            return Ok(Principal::default());
        }
        Ok(Principal {
            id: s.to_string(),
            info: None,
        })
    }
}

/// Cedar entity type used to model group membership on the principal. A policy
/// head `principal in Group::"<id>"` is the first-class form of the
/// `principal.groups.contains("<id>")` membership condition.
const GROUP_ENTITY_TYPE: &str = "Group";

#[derive(
    Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, Validate,
)]
pub struct Group {
    #[schema(max_length = 255, pattern = "^[^\\x00-\\x1F\\x22\\x27\\x5C]+$")]
    #[garde(length(max = 255), pattern("^[^\\x00-\\x1F\\x22\\x27\\x5C]+$"))]
    pub id: String,
}

impl Display for Group {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if self.id.is_empty() {
            return write!(f, "");
        }
        write!(f, "Group::\"{}\"", self.id)
    }
}

impl FromStr for Group {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() || s == "NULL" {
            return Ok(Group::default());
        }
        Ok(Group { id: s.to_string() })
    }
}

#[derive(
    Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, Validate,
)]
pub struct Resource {
    #[garde(length(max = 2048), pattern("^[^\\x00-\\x1F\\x22\\x5C]+$"))]
    #[schema(max_length = 2048, pattern = "^[^\\x00-\\x1F\\x22\\x5C]+$")]
    pub id: String,

    #[garde(length(max = 255), pattern("^[a-zA-Z0-9_.:/-]+$"))]
    #[serde(rename = "type")]
    #[schema(max_length = 255, pattern = "^[a-zA-Z0-9_.:/-]+$")]
    pub kind: String,

    #[garde(skip)]
    pub data: Option<Value>,
}

impl Display for Resource {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if self.kind.is_empty() || self.id.is_empty() {
            return write!(f, "");
        }
        write!(f, "{}::\"{}\"", self.kind, self.id)
    }
}

impl FromStr for Resource {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() || s == "NULL" {
            return Ok(Resource::default());
        }

        let parts: Vec<&str> = s.splitn(2, "::").collect();

        if parts.len() != 2 {
            return Err(format!(
                "Failed to parse Resource from string \"{s}\" - \
                    must be in \"Type::id\" format."
            ));
        }

        Ok(Resource {
            kind: parts[0].to_owned(),
            id: parts[1].trim_matches('"').to_string(),
            data: None,
        })
    }
}

pub type PolicyId = i64;

#[derive(Clone, Debug, PartialEq)]
pub struct PolicyErr {
    pub msg: String,
    pub kind: PolicyErrKind,
}

impl Display for PolicyErr {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.msg)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PolicyErrKind {
    Unknown,
    InvalidPolicy,
    InvalidPrincipal,
    InvalidAction,
    InvalidResource,
    InvalidContext,
}

pub type PolicyDecision = Decision;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PolicyScopes {
    pub principal: Option<Principal>,
    pub group: Option<Group>,
    pub action: Option<Action>,
    pub resource: Option<Resource>,
    pub resource_prefix: Option<Resource>,
}

/// Single-shot policy evaluation. Builds the principal entity (including any
/// `Group::"<id>"` parents derived from `groups_claim`) and evaluates the
/// policy against it. Callers that evaluate many policies for the same
/// principal should instead build a [`PreparedPrincipal`] once via
/// [`prepare_principal`] and call [`evaluate_policy_with_principal`] in a loop.
#[tracing::instrument(level = "debug")]
pub fn evaluate_policy(
    policy: &str,
    authorizer: &Authorizer,
    action: &Action,
    principal: &Principal,
    resource: Option<&Resource>,
    context: Option<Value>,
    groups_claim: &str,
) -> Result<Response, PolicyErr> {
    if policy.is_empty() {
        // If no policies are provided, the service denies the access by default.
        return Ok(Response::new(Decision::Deny, HashSet::new(), vec![]));
    }

    let prepared = prepare_principal(principal, groups_claim)?;
    evaluate_policy_with_principal(
        policy,
        authorizer,
        &prepared,
        action,
        resource,
        &[],
        context,
    )
}

/// Evaluates a single policy against a pre-built principal entity. The
/// principal entity (attributes and `Group` parents) is reused as-is, so the
/// per-policy work is limited to parsing the policy and assembling the
/// action/resource entities.
#[tracing::instrument(level = "debug", skip(authorizer, prepared, resource_parents))]
pub fn evaluate_policy_with_principal(
    policy: &str,
    authorizer: &Authorizer,
    prepared: &PreparedPrincipal,
    action: &Action,
    resource: Option<&Resource>,
    resource_parents: &[EntityUid],
    context: Option<Value>,
) -> Result<Response, PolicyErr> {
    if policy.is_empty() {
        // If no policies are provided, the service denies the access by default.
        return Ok(Response::new(Decision::Deny, HashSet::new(), vec![]));
    }

    let policy_set = parse_policy_set(policy)?;
    evaluate_policy_set_with_principal(
        &policy_set,
        authorizer,
        prepared,
        action,
        resource,
        resource_parents,
        context,
    )
}

/// Evaluates a pre-parsed [`PolicySet`] against a prepared principal. Callers
/// that reuse the same policy text across requests should obtain the set from
/// [`PolicySetCache`] instead of reparsing.
#[tracing::instrument(
    level = "debug",
    skip(authorizer, prepared, resource_parents, policy_set)
)]
pub fn evaluate_policy_set_with_principal(
    policy_set: &PolicySet,
    authorizer: &Authorizer,
    prepared: &PreparedPrincipal,
    action: &Action,
    resource: Option<&Resource>,
    resource_parents: &[EntityUid],
    context: Option<Value>,
) -> Result<Response, PolicyErr> {
    let action_entity = parse_action(action)?;
    let resource_entity = if let Some(resource) = resource {
        Some(parse_resource(resource)?)
    } else {
        None
    };

    let build_span = debug_span!("build_request");
    let (p, a, r) = (
        prepared.uid.clone(),
        action_entity.clone(),
        resource_entity.clone(),
    );
    let request = build_span.in_scope(|| -> Result<Request, PolicyErr> {
        let mut builder = Request::builder().principal(p).action(a);

        if let Some(r) = r {
            builder = builder.resource(r);
        }

        if let Some(context) = context {
            builder = parse_context(builder, context)?;
        }
        Ok(builder.build())
    })?;

    let entities = create_policy_entities(
        prepared,
        action_entity,
        (resource_entity, resource),
        resource_parents,
    )?;
    let authorize_span = debug_span!("is_authorized");
    let response =
        authorize_span.in_scope(|| authorizer.is_authorized(&request, policy_set, &entities));
    Ok(response)
}

/// Principal Cedar entity prepared once and reused across many policy
/// evaluations. Holds both the principal's `EntityUid` (for the request head)
/// and the fully-built `Entity` (attributes from `info`, the `sub` attribute,
/// and `Group::"<id>"` parents) so that constructing it - including parsing the
/// group memberships - is paid once per principal rather than once per policy.
#[derive(Clone, Debug)]
pub struct PreparedPrincipal {
    uid: EntityUid,
    entity: Entity,
}

/// Builds the principal Cedar entity once for reuse across policy evaluations.
/// Group memberships are read from `principal.info[groups_claim]` (a string
/// array) and attached as `Group::"<id>"` parents, so `principal in
/// Group::"<id>"` evaluates natively - equivalent to
/// `principal.groups.contains("<id>")` when `groups_claim` is `groups`.
#[tracing::instrument(level = "debug")]
pub fn prepare_principal(
    principal: &Principal,
    groups_claim: &str,
) -> Result<PreparedPrincipal, PolicyErr> {
    let uid = parse_principal(principal)?;

    let mut attrs = create_entity_attrs(principal.info.clone());
    attrs.insert(
        "sub".to_string(),
        RestrictedExpression::new_string(principal.id.clone()),
    );

    let parents = group_parents(principal, groups_claim);
    let entity = Entity::new(uid.clone(), attrs, parents).map_err(|err| PolicyErr {
        msg: format!("Failed to parse the principal {}. {}", principal, err),
        kind: PolicyErrKind::InvalidPrincipal,
    })?;

    Ok(PreparedPrincipal { uid, entity })
}

/// Resolves the set of `Group::"<id>"` entity ids the principal belongs to from
/// the `groups_claim` attribute of `principal.info`. Missing claim, non-array
/// values, and non-string members are ignored so a malformed claim simply
/// yields no memberships rather than failing the evaluation.
fn group_parents(principal: &Principal, groups_claim: &str) -> HashSet<EntityUid> {
    let type_name = group_entity_type();
    principal_groups(principal, groups_claim)
        .into_iter()
        .map(|id| {
            // `EntityId`'s `FromStr` is infallible - any string is a valid id.
            let entity_id = EntityId::from_str(&id).unwrap();
            EntityUid::from_type_name_and_id(type_name.clone(), entity_id)
        })
        .collect()
}

/// Reads the principal's group memberships from the configured claim in
/// `principal.info`. Returns an empty list when the claim is unset, the info is
/// absent, or the claim is missing or not an array of strings.
pub fn principal_groups(principal: &Principal, groups_claim: &str) -> Vec<String> {
    if groups_claim.is_empty() {
        return Vec::new();
    }

    let Some(Value::Object(info)) = principal.info.as_ref() else {
        return Vec::new();
    };
    let Some(Value::Array(groups)) = info.get(groups_claim) else {
        return Vec::new();
    };

    groups
        .iter()
        .filter_map(|group| group.as_str().map(str::to_string))
        .collect()
}

/// Computes the folder-hierarchy ancestors of a URL-shaped resource id, each
/// typed as `parent_kind`. Used both to select `resource in <parent>::"<url>/"`
/// hierarchy policies from storage and to attach the matching parents to the
/// request resource entity so Cedar `resource in ...` evaluates natively.
///
/// Returns one ancestor per folder level (each ending in `/`), from the URL
/// origin (`scheme://authority/`) down to the deepest containing folder. When
/// `encoded_id` is itself a folder form (ends in `/`) the deepest ancestor
/// equals the id, so an exact request on a hierarchy node matches inclusively.
/// A non-URL id yields no ancestors.
pub fn resource_hierarchy_ancestors(parent_kind: &str, encoded_id: &str) -> Vec<Resource> {
    let Ok(url) = Url::parse(encoded_id) else {
        return Vec::new();
    };
    if url.host_str().is_none() {
        return Vec::new();
    }

    // `encoded_id` is already the canonical form produced by
    // `encode_resource_url` (WHATWG-parsed and re-serialized), so `origin` and
    // `path` are slices of that same normalized string. Each folder ancestor is
    // the origin concatenated with the path truncated at a `/` boundary: one
    // ancestor per `/` in the path, from the origin root down to the deepest
    // containing folder. A trailing `/` makes the id its own deepest ancestor;
    // a non-folder leaf segment has no trailing `/` and is naturally excluded.
    // Slicing avoids cloning and re-serializing the whole `Url` on every level.
    let origin = &url[..Position::BeforePath];
    let path = url.path();

    let mut ancestors = Vec::new();
    for (offset, byte) in path.bytes().enumerate() {
        if byte != b'/' {
            continue;
        }
        let mut id = String::with_capacity(origin.len() + offset + 1);
        id.push_str(origin);
        id.push_str(&path[..=offset]);
        ancestors.push(Resource {
            id,
            kind: parent_kind.to_string(),
            data: None,
        });
    }
    ancestors
}

/// Builds the Cedar `EntityUid` parents for a request resource from the
/// hierarchy ancestors produced by [`resource_hierarchy_ancestors`]. Callers
/// build this once per request and reuse it across the per-policy evaluation
/// loop.
pub fn build_resource_parents(ancestors: &[Resource]) -> Result<Vec<EntityUid>, PolicyErr> {
    ancestors.iter().map(parse_resource).collect()
}

/// Parses the `Group` entity type name once and caches it, since it is reused
/// for every group membership on every principal.
fn group_entity_type() -> &'static EntityTypeName {
    static GROUP_TYPE: OnceLock<EntityTypeName> = OnceLock::new();
    GROUP_TYPE.get_or_init(|| EntityTypeName::from_str(GROUP_ENTITY_TYPE).unwrap())
}

#[tracing::instrument(level = "debug")]
pub fn parse_policy_set(policy: &str) -> Result<PolicySet, PolicyErr> {
    match policy.parse() {
        Ok(policy) => Ok(policy),
        Err(err) => Err(PolicyErr {
            msg: format!("Failed to parse the Cedar policy: {}", err),
            kind: PolicyErrKind::InvalidPolicy,
        }),
    }
}

#[tracing::instrument(level = "debug")]
fn parse_principal(principal: &Principal) -> Result<EntityUid, PolicyErr> {
    let id = EntityId::from_str(&principal.id).map_err(|err| PolicyErr {
        msg: format!("Failed to parse the principal {}. {}", principal, err),
        kind: PolicyErrKind::InvalidPrincipal,
    })?;
    let type_name = EntityTypeName::from_str("Principal").unwrap();
    let entity = EntityUid::from_type_name_and_id(type_name, id);
    Ok(entity)
}

#[tracing::instrument(level = "debug")]
fn parse_action(action: &Action) -> Result<EntityUid, PolicyErr> {
    let id = EntityId::from_str(&action.entity_id()).map_err(|err| PolicyErr {
        msg: format!("Failed to parse the action {}. {}", action, err),
        kind: PolicyErrKind::InvalidAction,
    })?;
    let type_name = EntityTypeName::from_str("Action").unwrap();
    let entity = EntityUid::from_type_name_and_id(type_name, id);
    Ok(entity)
}

#[tracing::instrument(level = "debug")]
fn parse_resource(resource: &Resource) -> Result<EntityUid, PolicyErr> {
    let id = EntityId::from_str(&resource.id).map_err(|err| PolicyErr {
        msg: format!("Failed to parse the resource {}. {}", resource, err),
        kind: PolicyErrKind::InvalidResource,
    })?;
    let type_name = EntityTypeName::from_str(&resource.kind).map_err(|err| PolicyErr {
        msg: format!("Failed to parse the resource {}. {}", resource, err),
        kind: PolicyErrKind::InvalidResource,
    })?;
    let entity = EntityUid::from_type_name_and_id(type_name, id);
    Ok(entity)
}

#[tracing::instrument(level = "debug", skip(builder, context))]
fn parse_context(
    builder: RequestBuilder<UnsetSchema>,
    context: Value,
) -> Result<RequestBuilder<UnsetSchema>, PolicyErr> {
    if let Ok(context) = Context::from_json_value(context, None) {
        Ok(builder.context(context))
    } else {
        Err(PolicyErr {
            msg: "Failed to parse request context.".to_string(),
            kind: PolicyErrKind::InvalidContext,
        })
    }
}

#[tracing::instrument(level = "debug", skip(prepared, resource_parents))]
fn create_policy_entities(
    prepared: &PreparedPrincipal,
    action_uid: EntityUid,
    resource: (Option<EntityUid>, Option<&Resource>),
    resource_parents: &[EntityUid],
) -> Result<Entities, PolicyErr> {
    let mut entities = Vec::with_capacity(3);

    entities.push(prepared.entity.clone());
    entities.push(Entity::new_no_attrs(action_uid, HashSet::new()));

    let (resource_uid, resource_data) = resource;
    if let Some(resource_uid) = resource_uid {
        if let Some(resource_data) = resource_data {
            let mut resource_attrs = create_entity_attrs(resource_data.data.clone());
            resource_attrs.insert(
                "id".to_string(),
                RestrictedExpression::new_string(resource_data.id.clone()),
            );
            resource_attrs.insert(
                "type".to_string(),
                RestrictedExpression::new_string(resource_data.kind.clone()),
            );

            let parents: HashSet<EntityUid> = resource_parents
                .iter()
                .filter(|parent| **parent != resource_uid)
                .cloned()
                .collect();
            let resource =
                Entity::new(resource_uid, resource_attrs, parents).map_err(|err| PolicyErr {
                    msg: format!("Failed to parse the resource {}. {}", resource_data, err),
                    kind: PolicyErrKind::InvalidResource,
                })?;
            entities.push(resource);
        }
    }

    Entities::from_entities(entities, None).map_err(|err| PolicyErr {
        msg: format!("Failed to create Cedar entities: {}", err),
        kind: PolicyErrKind::Unknown,
    })
}

#[tracing::instrument(level = "debug")]
fn create_entity_attrs(info: Option<Value>) -> HashMap<String, RestrictedExpression> {
    let mut attrs = HashMap::new();
    if let Some(info) = info {
        if let Ok(info_map) = serde_json::from_value::<HashMap<String, Value>>(info) {
            for (key, value) in info_map {
                if let Some(expr) = create_restricted_expression(&value) {
                    attrs.insert(key, expr);
                }
            }
        }
    }
    attrs
}

fn create_restricted_expression(value: &Value) -> Option<RestrictedExpression> {
    match value {
        Value::Null => None,
        Value::Bool(bool) => Some(RestrictedExpression::new_bool(*bool)),
        Value::Number(number) => number.as_i64().map(RestrictedExpression::new_long),
        Value::String(string) => Some(RestrictedExpression::new_string(string.clone())),
        Value::Array(array) => {
            let mut values = vec![];
            for value in array {
                if let Some(expr) = create_restricted_expression(value) {
                    values.push(expr);
                }
            }
            Some(RestrictedExpression::new_set(values))
        }
        Value::Object(obj) => {
            let mut fields = vec![];
            for (key, value) in obj.iter() {
                if let Some(expr) = create_restricted_expression(value) {
                    fields.push((key.clone(), expr));
                }
            }
            RestrictedExpression::new_record(fields).ok()
        }
    }
}

#[tracing::instrument(level = "info")]
pub fn validate_policy(
    policy: &str,
    action: Option<&Action>,
    principal: Option<&str>,
    group: Option<&str>,
    resource: Option<&Resource>,
) -> Result<String, PolicyErr> {
    let policy_set = parse_and_encode_policy_set(policy)?;
    validate_policy_set(&policy_set, action, principal, group, resource)?;
    Ok(policy_set.to_string())
}

/// Parses `policy` with Cedar and returns a canonical serialization derived
/// from the AST (JSON form). Whitespace inside string
/// literals is preserved as it is part of the literal's value.
#[tracing::instrument(level = "info")]
pub fn canonicalize_policy(policy: &str) -> Result<String, PolicyErr> {
    if policy.trim().is_empty() {
        return Err(PolicyErr {
            msg: "`policy` cannot be empty.".to_string(),
            kind: PolicyErrKind::InvalidPolicy,
        });
    }

    let policy_set = policy.parse::<PolicySet>().map_err(|err| PolicyErr {
        msg: format!("Failed to parse the Cedar policy: {}", err),
        kind: PolicyErrKind::InvalidPolicy,
    })?;

    canonicalize_policy_set(&policy_set)
}

#[tracing::instrument(level = "info")]
pub fn infer_policy_scopes(policy: &str) -> Result<PolicyScopes, PolicyErr> {
    let policy_set = parse_and_encode_policy_set(policy)?;
    scopes_from_policy_set(&policy_set)
}

/// Fully-processed result of running a Cedar policy through the same pipeline
/// the HTTP layer used to spread across [`infer_policy_scopes`],
/// [`validate_policy`] and [`canonicalize_policy`] - but in a single parse.
///
/// `canonical_hash` is computed from the URL-encoded policy set so that it
/// stays stable regardless of whether validation is enabled (matching the
/// behavior of the validation-enabled path, which is the default).
#[derive(Clone, Debug)]
pub struct ProcessedPolicy {
    /// Normalized Cedar text to store (URL-encoded resource ids, no formatting
    /// fluff). Equivalent to what `validate_policy` used to return.
    pub text: String,
    pub scopes: PolicyScopes,
    /// MD5 hex digest of the canonical JSON form. Storage layers use it
    /// directly as the dedup key instead of re-parsing and hashing on their
    /// own.
    pub canonical_hash: String,
}

/// Single-parse version of `infer_policy_scopes` + (optional) `validate_policy`
/// + `canonicalize_policy`. The hot batch path uses this to pay Cedar's parse
/// cost exactly once per policy.
#[tracing::instrument(level = "info")]
pub fn process_policy(policy: &str, validate: bool) -> Result<ProcessedPolicy, PolicyErr> {
    let policy_set = parse_and_encode_policy_set(policy)?;
    let scopes = scopes_from_policy_set(&policy_set)?;
    if validate {
        validate_policy_set(
            &policy_set,
            scopes.action.as_ref(),
            scopes.principal.as_ref().map(|p| p.id.as_str()),
            scopes.group.as_ref().map(|g| g.id.as_str()),
            scopes.resource.as_ref(),
        )?;
    }
    let canonical = canonicalize_policy_set(&policy_set)?;
    let canonical_hash = md5_hex(&canonical);
    Ok(ProcessedPolicy {
        text: policy_set.to_string(),
        scopes,
        canonical_hash,
    })
}

/// Hex-encoded MD5 of a canonical Cedar policy string. Storage layers compare
/// these strings as the dedup key on `policies.policy_hash`.
pub fn md5_hex(canonical: &str) -> String {
    use md5::Digest;
    let digest = md5::Md5::digest(canonical.as_bytes());
    let mut out = String::with_capacity(32);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(out, "{:02x}", byte);
    }
    out
}

fn parse_and_encode_policy_set(policy: &str) -> Result<PolicySet, PolicyErr> {
    if policy.trim().is_empty() {
        return Err(PolicyErr {
            msg: "`policy` cannot be empty.".to_string(),
            kind: PolicyErrKind::InvalidPolicy,
        });
    }

    let policy_set = policy.parse::<PolicySet>().map_err(|err| PolicyErr {
        msg: format!("Failed to parse the Cedar policy: {}", err),
        kind: PolicyErrKind::InvalidPolicy,
    })?;

    encode_policy_set_resources(policy_set)
}

fn scopes_from_policy_set(policy_set: &PolicySet) -> Result<PolicyScopes, PolicyErr> {
    let mut policies = policy_set.policies();
    let policy = policies.next().ok_or_else(|| PolicyErr {
        msg: "Expected exactly one Cedar policy, but none were provided.".to_string(),
        kind: PolicyErrKind::InvalidPolicy,
    })?;
    if policies.next().is_some() {
        return Err(PolicyErr {
            msg: "Expected exactly one Cedar policy, but multiple policies were provided."
                .to_string(),
            kind: PolicyErrKind::InvalidPolicy,
        });
    }

    let (principal, group) = infer_principal_and_group_scope(policy.principal_constraint());
    let (resource, resource_prefix) = infer_resource_scopes(policy.resource_constraint());
    Ok(PolicyScopes {
        principal,
        group,
        action: infer_action_scope(policy.action_constraint()),
        resource,
        resource_prefix,
    })
}

fn validate_policy_set(
    policy_set: &PolicySet,
    action: Option<&Action>,
    principal: Option<&str>,
    group: Option<&str>,
    resource: Option<&Resource>,
) -> Result<(), PolicyErr> {
    for policy in policy_set.policies() {
        if let Some(principal) = principal {
            validate_policy_principal(policy, principal)?;
        }

        if let Some(group) = group {
            validate_policy_group(policy, group)?;
        }

        if let Some(action) = action {
            validate_policy_action(policy, action)?;
        }

        if let Some(resource) = resource {
            validate_policy_resource(policy, resource)?;
        }
    }
    Ok(())
}

fn canonicalize_policy_set(policy_set: &PolicySet) -> Result<String, PolicyErr> {
    let mut serialized = policy_set
        .policies()
        .map(|policy| {
            let json = policy.to_json().map_err(|err| PolicyErr {
                msg: format!("Failed to serialize Cedar policy AST: {}", err),
                kind: PolicyErrKind::InvalidPolicy,
            })?;
            serde_json::to_string(&json).map_err(|err| PolicyErr {
                msg: format!("Failed to serialize Cedar policy AST: {}", err),
                kind: PolicyErrKind::InvalidPolicy,
            })
        })
        .collect::<Result<Vec<String>, PolicyErr>>()?;

    // The iteration order of `PolicySet::policies()` is not guaranteed to be
    // stable across parses of equivalent inputs, so sort before joining.
    serialized.sort();
    Ok(serialized.join("\n"))
}

/// Infers the principal and group scopes from a single principal constraint.
/// `principal == Principal::"<id>"` yields a principal scope; `principal in
/// Group::"<id>"` yields a group scope. All other forms (`in` on a non-`Group`
/// type, `is`, or an unconstrained `principal`) leave both unset.
#[tracing::instrument(level = "debug")]
fn infer_principal_and_group_scope(
    constraint: PrincipalConstraint,
) -> (Option<Principal>, Option<Group>) {
    match constraint {
        PrincipalConstraint::Eq(entity) => {
            let id: &str = entity.id().as_ref();
            (
                Some(Principal {
                    id: id.to_string(),
                    info: None,
                }),
                None,
            )
        }
        PrincipalConstraint::In(entity) if entity.type_name().to_string() == GROUP_ENTITY_TYPE => {
            let id: &str = entity.id().as_ref();
            (None, Some(Group { id: id.to_string() }))
        }
        _ => (None, None),
    }
}

#[tracing::instrument(level = "debug")]
fn infer_action_scope(constraint: ActionConstraint) -> Option<Action> {
    match constraint {
        ActionConstraint::Eq(entity) => parse_action_entity(&entity),
        ActionConstraint::In(entities) if entities.len() == 1 => {
            entities.first().and_then(parse_action_entity)
        }
        ActionConstraint::Any | ActionConstraint::In(_) => None,
    }
}

#[tracing::instrument(level = "debug")]
fn parse_action_entity(entity: &EntityUid) -> Option<Action> {
    let type_name = entity.type_name().to_string();
    if type_name != "Action" {
        return None;
    }
    let raw: &str = entity.id().as_ref();
    Action::from_str(raw).ok()
}

/// Infers the exact (`resource == Type::"id"`) and hierarchy
/// (`resource in Type::"url/"`) resource scopes from a resource constraint.
/// Only one is ever set: `==` yields the exact scope, `in` yields the
/// hierarchy prefix scope (already URL-encoded and trailing-slash normalized by
/// [`encode_resource_constraint`]). Other forms leave both unset.
#[tracing::instrument(level = "debug")]
fn infer_resource_scopes(constraint: ResourceConstraint) -> (Option<Resource>, Option<Resource>) {
    match constraint {
        ResourceConstraint::Eq(entity) => {
            let id: &str = entity.id().as_ref();
            let resource = Resource {
                id: id.to_string(),
                kind: entity.type_name().to_string(),
                data: None,
            };
            (Some(resource), None)
        }
        ResourceConstraint::In(entity) => {
            let id: &str = entity.id().as_ref();
            let prefix = Resource {
                id: id.to_string(),
                kind: entity.type_name().to_string(),
                data: None,
            };
            (None, Some(prefix))
        }
        _ => (None, None),
    }
}

#[tracing::instrument(level = "debug")]
fn encode_policy_set_resources(policy_set: PolicySet) -> Result<PolicySet, PolicyErr> {
    let mut policy_json = policy_set.to_json().map_err(|err| PolicyErr {
        msg: format!("Failed to serialize policy to JSON: {}", err),
        kind: PolicyErrKind::InvalidPolicy,
    })?;

    encode_resources_in_policy_set_json(&mut policy_json)?;

    PolicySet::from_json_value(policy_json).map_err(|err| PolicyErr {
        msg: format!("Failed to deserialize policy from JSON: {}", err),
        kind: PolicyErrKind::InvalidPolicy,
    })
}

#[tracing::instrument(level = "debug")]
fn encode_resources_in_policy_set_json(value: &mut Value) -> Result<(), PolicyErr> {
    if let Value::Object(obj) = value {
        if let Some(Value::Object(static_policies_map)) = obj.get_mut("staticPolicies") {
            for (_, policy_val) in static_policies_map.iter_mut() {
                if let Value::Object(policy_obj) = policy_val {
                    if let Some(resource_val) = policy_obj.get_mut("resource") {
                        encode_resource_constraint(resource_val)?;
                    }
                }
            }
        }
    }
    Ok(())
}

#[tracing::instrument(level = "debug")]
fn encode_resource_constraint(value: &mut Value) -> Result<(), PolicyErr> {
    if let Value::Object(resource_obj) = value {
        if let Some(Value::String(op)) = resource_obj.get("op") {
            let op = op.clone();
            if let Some(entity_val) = resource_obj.get_mut("entity") {
                match op.as_str() {
                    "==" => encode_entity_id(entity_val)?,
                    "in" => encode_hierarchy_entity_id(entity_val)?,
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

#[tracing::instrument(level = "debug")]
fn encode_entity_id(value: &mut Value) -> Result<(), PolicyErr> {
    if let Value::Object(entity_obj) = value {
        if let Some(Value::String(id)) = entity_obj.get_mut("id") {
            let encoded = encode_resource_url(id).map_err(|err| PolicyErr {
                msg: format!("Failed to encode resource URL: {}", err),
                kind: PolicyErrKind::InvalidResource,
            })?;
            *id = encoded;
        }
    }
    Ok(())
}

/// Encodes and normalizes the entity id of a `resource in <Type>::"<url>"`
/// hierarchy constraint. The id must be a valid absolute URL (rejected
/// otherwise, independent of `POLICY_VALIDATION`) and is normalized to a
/// trailing `/` so that the stored scope, the policy text, and request
/// ancestors share the same folder-boundary form.
#[tracing::instrument(level = "debug")]
fn encode_hierarchy_entity_id(value: &mut Value) -> Result<(), PolicyErr> {
    if let Value::Object(entity_obj) = value {
        if let Some(Value::String(id)) = entity_obj.get_mut("id") {
            let url = Url::parse(id).ok().filter(|u| u.has_host());
            if url.is_none() {
                return Err(PolicyErr {
                    msg: format!(
                        "Resource hierarchy scope \"{}\" must be a valid URL \
                         (e.g. resource in Type::\"https://host/path/\").",
                        id
                    ),
                    kind: PolicyErrKind::InvalidResource,
                });
            }

            let mut encoded = encode_resource_url(id).map_err(|err| PolicyErr {
                msg: format!("Failed to encode resource URL: {}", err),
                kind: PolicyErrKind::InvalidResource,
            })?;
            if !encoded.ends_with('/') {
                encoded.push('/');
            }
            *id = encoded;
        }
    }
    Ok(())
}

#[tracing::instrument(level = "info", skip(policy))]
fn validate_policy_principal(policy: &Policy, principal: &str) -> Result<(), PolicyErr> {
    if principal.is_empty() {
        return Err(PolicyErr {
            msg: r#"
                `principal` cannot be empty. 
                Remove it from the request if it's not used in the policy.
            "#
            .to_string(),
            kind: PolicyErrKind::InvalidPrincipal,
        });
    }

    match policy.principal_constraint() {
        PrincipalConstraint::Eq(entity) => {
            let policy_entity: &str = entity.id().as_ref();
            if principal == policy_entity {
                Ok(())
            } else {
                Err(PolicyErr {
                    msg: format!(
                        "Cannot assign policy \"{}\" for principal {} -- \
                        the policy principal must match the provided principal.",
                        policy, principal
                    ),
                    kind: PolicyErrKind::InvalidPrincipal,
                })
            }
        }
        PrincipalConstraint::Any
        | PrincipalConstraint::In(_)
        | PrincipalConstraint::Is(_)
        | PrincipalConstraint::IsIn(_, _) => Ok(()),
    }
}

/// Ensures a provided group scope matches a `principal in Group::"<id>"`
/// policy head. Non-group principal constraints accept any group (the group
/// scope simply does not apply to them).
#[tracing::instrument(level = "info", skip(policy))]
fn validate_policy_group(policy: &Policy, group: &str) -> Result<(), PolicyErr> {
    if group.is_empty() {
        return Err(PolicyErr {
            msg: r#"
                `group` cannot be empty. 
                Remove it from the request if it's not used in the policy.
            "#
            .to_string(),
            kind: PolicyErrKind::InvalidPrincipal,
        });
    }

    match policy.principal_constraint() {
        PrincipalConstraint::In(entity) if entity.type_name().to_string() == GROUP_ENTITY_TYPE => {
            let policy_entity: &str = entity.id().as_ref();
            if group == policy_entity {
                Ok(())
            } else {
                Err(PolicyErr {
                    msg: format!(
                        "Cannot assign policy \"{}\" for group {} -- \
                        the policy group must match the provided group.",
                        policy, group
                    ),
                    kind: PolicyErrKind::InvalidPrincipal,
                })
            }
        }
        _ => Ok(()),
    }
}

#[tracing::instrument(level = "info", skip(policy))]
fn validate_policy_action(policy: &Policy, action: &Action) -> Result<(), PolicyErr> {
    if action.name.is_empty() {
        return Err(PolicyErr {
            msg: "`action.name` cannot be empty.".to_string(),
            kind: PolicyErrKind::InvalidAction,
        });
    }

    if action.service.is_empty() {
        return Err(PolicyErr {
            msg: "`action.service` cannot be empty.".to_string(),
            kind: PolicyErrKind::InvalidAction,
        });
    }

    match policy.action_constraint() {
        ActionConstraint::Any => Ok(()),
        ActionConstraint::In(entities) => {
            let requested_action = action.to_string();
            let policy_action = entities
                .iter()
                .find(|action| action.to_string() == requested_action);

            match policy_action {
                Some(_) => Ok(()),
                None => Err(PolicyErr {
                    msg: format!(
                        "Cannot assign policy \"{}\" for action {} -- \
                        the provided action must be included in the policy action constraint.",
                        policy, action
                    ),
                    kind: PolicyErrKind::InvalidAction,
                }),
            }
        }
        ActionConstraint::Eq(entity) => {
            let policy_action = entity.to_string();
            if action.to_string() == policy_action {
                Ok(())
            } else {
                Err(PolicyErr {
                    msg: format!(
                        "Cannot assign policy \"{}\" for action {} -- \
                        the policy action ({}) must match the provided action.",
                        policy, action, policy_action
                    ),
                    kind: PolicyErrKind::InvalidAction,
                })
            }
        }
    }
}

#[tracing::instrument(level = "info", skip(policy))]
fn validate_policy_resource(policy: &Policy, resource: &Resource) -> Result<(), PolicyErr> {
    if resource.id.is_empty() {
        return Err(PolicyErr {
            msg: "`resource.id` cannot be empty.".to_string(),
            kind: PolicyErrKind::InvalidResource,
        });
    }

    if resource.kind.is_empty() {
        return Err(PolicyErr {
            msg: "`resource.type` cannot be empty.".to_string(),
            kind: PolicyErrKind::InvalidResource,
        });
    }

    match policy.resource_constraint() {
        ResourceConstraint::In(entity) | ResourceConstraint::Eq(entity) => {
            if resource.to_string() == entity.to_string() {
                Ok(())
            } else {
                Err(PolicyErr {
                    msg: format!(
                        "Cannot assign policy \"{}\" for resource {} -- \
                        the policy resource must match the provided resource.",
                        policy, resource
                    ),
                    kind: PolicyErrKind::InvalidResource,
                })
            }
        }
        ResourceConstraint::Any | ResourceConstraint::Is(_) | ResourceConstraint::IsIn(_, _) => {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests;
pub mod util;
