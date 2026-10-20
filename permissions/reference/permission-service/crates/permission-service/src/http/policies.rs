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

use crate::events::publisher::publish_permission_event;
use crate::http::middlewares::access_log::AccessLog;
use crate::service::authorization;
use crate::service::authorization::{
    ensure_permissions_can_be_changed, ensure_permissions_can_be_viewed, AuthorizationRequest,
};
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::{Extension, Json};
use garde::Validate;
use opentelemetry::KeyValue;
use permission_core::{
    infer_policy_scopes, process_policy, resource_hierarchy_ancestors, validate_policy, Action,
    Group, PolicyId, Principal, ProcessedPolicy, Resource,
};
use permission_db::{
    PageResult, Policy, PolicyDbBatch, PolicyUpsertResult, StorageErr, StorageErrKind,
};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};

const POLICY_TAG: &str = "policies";

#[utoipa::path(
    get,
    path = "/v1beta/policies/",
    description = "Returns policies from the database using offset pagination.",
    tag = POLICY_TAG,
    params(
        ("principal" = Option<String>, Query),
        ("group" = Option<String>, Query, description = "Filter by the group scope of `principal in Group::\"<id>\"` policies"),
        ("action" = Option<String>, Query),
        ("resource" = Option<String>, Query),
        ("scope" = inline(Option<PolicyScopeFilter>), Query, description = "Resource scope-matching mode. `exact` (default) returns only policies scoped directly to `resource`; `inherited` also returns hierarchy (`resource in <Type>::\"<url>/\"`) policies whose prefix is an ancestor of `resource`. Scope filter only: global policies (`permit(principal, action, resource)`) are never listed here. Use the authorization endpoint for the full evaluated set."),
        ("order" = Option<i32>, Query, description = "Filter by policy order"),
        ("page" = i64, Query, description = "Start page", minimum = 1),
        ("limit" = i64, Query, description = "Page size", minimum = 1, maximum = 50)
    ),
    responses(
        (status = OK, body = PageResult<PolicyResponse>),
        (status = INTERNAL_SERVER_ERROR, body = String),
        (status = UNPROCESSABLE_ENTITY, body = String),
    )
)]
#[tracing::instrument(level = "info", skip(access, state))]
pub async fn get_policies_paginated_api(
    Extension(requester): Extension<Arc<Principal>>,
    Extension(access): Extension<AccessLog>,
    State(state): State<AppState>,
    Query(query): Query<PolicyPageQuery>,
) -> Result<Json<PageResult<PolicyResponse>>, (StatusCode, String)> {
    access.params(policy_page_params(&query));

    query
        .validate()
        .map_err(|e| access.fail(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;

    ensure_permissions_can_be_viewed(&state, &requester, AuthorizationRequest::default())
        .await
        .map_err(|err| access.fail_err(&err))?;

    let action = query
        .action
        .map(|a| if a == "NULL" { "".to_string() } else { a })
        .map(|a| a.parse::<Action>())
        .transpose()
        .map_err(|e| access.fail(StatusCode::BAD_REQUEST, format!("Invalid action: {}", e)))?;

    let principal: Option<Principal> = query
        .principal
        .map(|sub| if sub == "NULL" { "".to_string() } else { sub })
        .map(|sub| sub.parse::<Principal>())
        .transpose()
        .map_err(|e| access.fail(StatusCode::BAD_REQUEST, format!("Invalid principal: {}", e)))?;

    let group: Option<Group> = query
        .group
        .map(|g| if g == "NULL" { "".to_string() } else { g })
        .map(|g| g.parse::<Group>())
        .transpose()
        .map_err(|e| access.fail(StatusCode::BAD_REQUEST, format!("Invalid group: {}", e)))?;

    let parsed_resource: Option<Resource> = query
        .resource
        .map(|r| if r == "NULL" { "".to_string() } else { r })
        .map(|r| r.parse())
        .transpose()
        .map_err(|e| access.fail(StatusCode::BAD_REQUEST, format!("Invalid resource: {}", e)))?;

    let parsed_resource = match state.url_remapper.as_ref() {
        Some(remapper) => remapper.rewrite_resource_opt(parsed_resource),
        None => parsed_resource,
    };

    let resource = match authorization::encode_resource_id_prefix(&parsed_resource) {
        Ok(res) => res,
        Err(err) => return Err(access.fail(StatusCode::BAD_REQUEST, err)),
    };

    let resource_prefixes = if query.scope == PolicyScopeFilter::Inherited {
        resolve_inherited_prefixes(&state, resource.as_ref()).await
    } else {
        Vec::new()
    };

    match state
        .policies
        .get_policies_paginated(
            action.as_ref(),
            principal.as_ref(),
            group.as_ref(),
            resource.as_ref(),
            &resource_prefixes,
            query.order,
            query.page,
            query.limit,
        )
        .await
    {
        Ok(policies) => {
            access.detail(json!({ "item_count": policies.items.len() }));
            Ok(Json(PageResult {
                page: policies.page,
                page_size: policies.page_size,
                page_count: policies.page_count,
                items: policies
                    .items
                    .into_iter()
                    .map(PolicyResponse::from)
                    .collect(),
            }))
        }
        Err(err) => {
            tracing::error!(error = %err, "Failed to get policies from the database");
            Err(access.fail(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to get policies".to_string(),
            ))
        }
    }
}

/// Builds the access-log `params` for a policy listing from the query filters.
/// `None` filters are omitted; `scope` always appears because it has a serde
/// default of `exact`.
fn policy_page_params(query: &PolicyPageQuery) -> Value {
    let mut params = serde_json::Map::new();
    if let Some(principal) = &query.principal {
        params.insert("principal".to_string(), json!(principal));
    }
    if let Some(group) = &query.group {
        params.insert("group".to_string(), json!(group));
    }
    if let Some(action) = &query.action {
        params.insert("action".to_string(), json!(action));
    }
    if let Some(resource) = &query.resource {
        params.insert("resource".to_string(), json!(resource));
    }
    if let Some(order) = query.order {
        params.insert("order".to_string(), json!(order));
    }
    params.insert(
        "scope".to_string(),
        serde_json::to_value(query.scope).unwrap_or(Value::Null),
    );
    params.insert("page".to_string(), json!(query.page));
    params.insert("limit".to_string(), json!(query.limit));
    Value::Object(params)
}

/// Computes the folder-hierarchy ancestor prefixes for a `scope=inherited`
/// listing. Resource types are global (not tied to a service), so the parent
/// type is resolved from the resource type metadata by its `kind` alone,
/// independent of any `action`. Falls back to the resource's own kind when the
/// type is unregistered or declares no `parentType`. Returns empty for wildcard
/// or missing resources.
async fn resolve_inherited_prefixes(
    state: &AppState,
    resource: Option<&Resource>,
) -> Vec<Resource> {
    let Some(resource) = resource else {
        return Vec::new();
    };
    if resource.id.ends_with('*') {
        return Vec::new();
    }

    let parent_kind = match state
        .metadata
        .get_resource_type_by_kind(&resource.kind)
        .await
    {
        Ok(meta) => meta
            .and_then(|meta| meta.parent_type)
            .unwrap_or_else(|| resource.kind.clone()),
        Err(err) => {
            tracing::debug!(
                error = %err,
                resource_type = %resource.kind,
                "Failed to resolve resource type metadata for inherited-scope listing; \
                 defaulting parent type to the resource kind"
            );
            resource.kind.clone()
        }
    };

    resource_hierarchy_ancestors(&parent_kind, &resource.id)
}

#[utoipa::path(
    get,
    path = "/v1beta/policies/{id}",
    description = "Returns a Cedar policy by its ID or HTTP404 if such ID does not exist.",
    tag = POLICY_TAG,
    params(
        ("id" = i64, Path, description = "Policy ID")
    ),
    responses(
        (status = OK, body = PolicyResponse),
        (status = NOT_FOUND, body = String),
        (status = UNPROCESSABLE_ENTITY, body = String),
    )
)]
#[tracing::instrument(level = "info", skip(access, state))]
pub async fn get_policy_api(
    Extension(requester): Extension<Arc<Principal>>,
    Extension(access): Extension<AccessLog>,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<PolicyResponse>, (StatusCode, String)> {
    access.params(json!({ "id": id }));

    ensure_permissions_can_be_viewed(&state, &requester, AuthorizationRequest::default())
        .await
        .map_err(|err| access.fail_err(&err))?;

    match state.policies.get_policy_by_id(&id).await {
        Ok(policy) => Ok(Json(PolicyResponse::from(policy))),
        Err(_) => Err(access.fail(
            StatusCode::NOT_FOUND,
            format!("Policy with ID '{}' could not be found.", id),
        )),
    }
}

#[utoipa::path(
    put,
    path = "/v1beta/policies/",
    description = "Adds a single Cedar policy. Policy scopes are inferred from policy text. \
                   Returns HTTP 201 when the policy was newly inserted and HTTP 200 when an \
                   existing policy with the same canonical text was updated in place; the \
                   `created` field in the body mirrors that signal.",
    tag = POLICY_TAG,
    request_body = PutPolicyPayload,
    responses(
        (status = CREATED, body = PutPolicyResponse, description = "Policy was newly inserted"),
        (status = OK, body = PutPolicyResponse, description = "Existing policy was updated in place"),
        (status = BAD_REQUEST, body = String),
        (status = UNPROCESSABLE_ENTITY, body = String),
    )
)]
#[tracing::instrument(level = "info", skip(access, state, payload))]
pub async fn put_policy_api(
    Extension(requester): Extension<Arc<Principal>>,
    Extension(access): Extension<AccessLog>,
    State(state): State<AppState>,
    Json(payload): Json<PutPolicyPayload>,
) -> Result<(StatusCode, Json<PutPolicyResponse>), (StatusCode, String)> {
    if access.is_enabled() {
        access.params(put_policy_params(&payload));
    }

    if let Err(e) = payload.validate() {
        record_policy_mutation(&state, "create", "invalid");
        return Err(access.fail(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()));
    }

    let mut policy = payload.policy;
    if let Some(remapper) = state.url_remapper.as_ref() {
        policy = match remapper.rewrite_policy_text(&policy) {
            Ok(policy) => policy,
            Err(err) => {
                record_policy_mutation(&state, "create", "invalid");
                return Err(access.fail(StatusCode::BAD_REQUEST, err.to_string()));
            }
        };
    }
    let scopes = match infer_policy_scopes(&policy) {
        Ok(scopes) => scopes,
        Err(err) => {
            record_policy_mutation(&state, "create", "invalid");
            return Err(access.fail(StatusCode::BAD_REQUEST, err.to_string()));
        }
    };
    let principal = scopes.principal;
    let group = scopes.group;
    let action = scopes.action;
    let resource = scopes.resource;
    let resource_prefix = scopes.resource_prefix;

    let edit_context =
        authorization::build_edit_context(principal.as_ref(), group.as_ref(), action.as_ref());
    if let Err(err) = ensure_permissions_can_be_changed(
        &state,
        &requester,
        resource.as_ref().or(resource_prefix.as_ref()),
        Some(edit_context),
    )
    .await
    {
        record_policy_mutation(&state, "create", "forbidden");
        return Err(access.fail_err(&err));
    }

    if state.config.policies.policy_validation {
        match validate_policy(
            &policy,
            action.as_ref(),
            principal.as_ref().map(|p| p.id.as_str()),
            group.as_ref().map(|g| g.id.as_str()),
            resource.as_ref(),
        ) {
            Ok(validated_policy) => policy = validated_policy,
            Err(err) => {
                record_policy_mutation(&state, "create", "invalid");
                return Err(access.fail(StatusCode::BAD_REQUEST, err.to_string()));
            }
        }
    }

    let order = payload
        .order
        .unwrap_or(state.config.policies.default_policy_order);
    let comment = normalize_comment(payload.comment);

    match state
        .policies
        .add_policy(
            &policy,
            action.as_ref(),
            principal.as_ref(),
            group.as_ref(),
            resource.as_ref(),
            resource_prefix.as_ref(),
            order,
            &requester.id,
            comment.as_deref(),
        )
        .await
    {
        Ok(result) => {
            record_policy_mutation(
                &state,
                if result.created { "create" } else { "update" },
                "success",
            );

            state
                .evaluation_cache
                .invalidate(&(principal, action, resource));

            publish_permission_event(
                &state,
                result.policy.principal.as_ref(),
                result.policy.action.as_ref(),
                result.policy.resource.as_ref(),
            );

            access.detail(json!({ "created": result.created }));

            let status = if result.created {
                StatusCode::CREATED
            } else {
                StatusCode::OK
            };
            Ok((status, Json(PutPolicyResponse::from(result))))
        }
        Err(StorageErr {
            kind: StorageErrKind::Forbidden,
            msg,
        }) => {
            record_policy_mutation(&state, "create", "forbidden");
            Err(access.fail(StatusCode::FORBIDDEN, msg))
        }
        Err(StorageErr {
            kind: StorageErrKind::NotSupported,
            ..
        }) => {
            record_policy_mutation(&state, "create", "invalid");
            Err(access.fail(
                StatusCode::NOT_IMPLEMENTED,
                "Operation not supported".to_string(),
            ))
        }
        Err(
            err @ StorageErr {
                kind: StorageErrKind::Internal,
                ..
            },
        ) => {
            record_policy_mutation(&state, "create", "error");
            tracing::error!(error = %err, "Failed to add the policy");
            Err(access.fail(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to add the policy".to_string(),
            ))
        }
        Err(StorageErr { msg, .. }) => {
            record_policy_mutation(&state, "create", "invalid");
            Err(access.fail(StatusCode::BAD_REQUEST, msg))
        }
    }
}

/// Builds the access-log `params` for a single policy upsert: the request body
/// (`policy` text, `order`, `comment`). Absent optional fields are omitted.
fn put_policy_params(payload: &PutPolicyPayload) -> Value {
    let mut params = serde_json::Map::new();
    params.insert("policy".to_string(), json!(payload.policy));
    if let Some(order) = payload.order {
        params.insert("order".to_string(), json!(order));
    }
    if let Some(comment) = &payload.comment {
        params.insert("comment".to_string(), json!(comment));
    }
    Value::Object(params)
}

#[utoipa::path(
    put,
    path = "/v1beta/policies/batch/",
    description = "Batched version of /policies/ API. Each item must contain exactly one Cedar \
                   policy and scopes are inferred from policy text. At most 100 items per request. \
                   Always returns HTTP 200; each `results[i].created` indicates whether that \
                   policy was newly inserted (`true`) or matched an existing canonical policy \
                   and was updated in place (`false`).",
    tag = POLICY_TAG,
    request_body = Vec<PutPolicyPayload>,
    responses(
        (status = OK, body = PutPolicyBatchResponse),
        (status = BAD_REQUEST, body = String),
        (status = UNPROCESSABLE_ENTITY, body = String),
    )
)]
#[tracing::instrument(level = "info", skip(access, state, request))]
pub async fn put_policy_batch_api(
    Extension(requester): Extension<Arc<Principal>>,
    Extension(access): Extension<AccessLog>,
    State(state): State<AppState>,
    Json(request): Json<PutPolicyBatchRequest>,
) -> Result<Json<PutPolicyBatchResponse>, (StatusCode, String)> {
    if access.is_enabled() {
        if request.0.is_empty() {
            access.params(json!({}));
        } else {
            access.items(request.0.iter().map(put_policy_params));
        }
    }

    request
        .validate()
        .map_err(|e| access.fail(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;
    let batches = request.0;
    if batches.is_empty() {
        return Ok(Json(PutPolicyBatchResponse { results: vec![] }));
    }

    // All per-item work below is CPU-bound (Cedar parsing dominates). Fan it
    // out across Rayon's pool inside `spawn_blocking` so we (a) don't block
    // the tokio runtime and (b) actually use multiple cores per request
    // instead of processing items one-by-one on the request task.
    let url_remapper = state.url_remapper.clone();
    let validate = state.config.policies.policy_validation;
    let default_order = state.config.policies.default_policy_order;
    let created_by = requester.id.clone();
    let processing_result = tokio::task::spawn_blocking(move || {
        batches
            .into_par_iter()
            .enumerate()
            .map(
                |(index, payload)| -> Result<PolicyDbBatch, (StatusCode, String)> {
                    let policy_text = if let Some(remapper) = url_remapper.as_ref() {
                        remapper
                            .rewrite_policy_text(&payload.policy)
                            .map_err(|err| {
                                (StatusCode::BAD_REQUEST, format!("batches.{index}: {}", err))
                            })?
                    } else {
                        payload.policy
                    };
                    let ProcessedPolicy {
                        text,
                        scopes,
                        canonical_hash,
                    } = process_policy(&policy_text, validate).map_err(|err| {
                        (StatusCode::BAD_REQUEST, format!("batches.{index}: {}", err))
                    })?;
                    Ok(PolicyDbBatch {
                        policy: text,
                        action: scopes.action,
                        principal: scopes.principal,
                        group: scopes.group,
                        resource: scopes.resource,
                        resource_prefix: scopes.resource_prefix,
                        order: payload.order.unwrap_or(default_order),
                        system: false,
                        created_by: created_by.clone(),
                        comment: normalize_comment(payload.comment),
                        canonical_hash,
                    })
                },
            )
            .collect::<Result<Vec<_>, _>>()
    })
    .await
    .map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("policy processing task failed: {err}"),
        )
    });

    let processed_batches = match processing_result {
        Ok(Ok(processed_batches)) => processed_batches,
        Ok(Err((status, msg))) => {
            record_policy_mutation(&state, "batch", "invalid");
            return Err(access.fail(status, msg));
        }
        Err((status, msg)) => {
            record_policy_mutation(&state, "batch", "error");
            return Err(access.fail(status, msg));
        }
    };

    state
        .metrics
        .policy_mutation_batch_size
        .record(processed_batches.len() as u64, &[]);

    for batch in processed_batches.iter() {
        let edit_context = authorization::build_edit_context(
            batch.principal.as_ref(),
            batch.group.as_ref(),
            batch.action.as_ref(),
        );
        if let Err(err) = ensure_permissions_can_be_changed(
            &state,
            &requester,
            batch.resource.as_ref().or(batch.resource_prefix.as_ref()),
            Some(edit_context),
        )
        .await
        {
            record_policy_mutation(&state, "batch", "forbidden");
            return Err(access.fail_err(&err));
        }
    }

    match state.policies.add_policy_batch(processed_batches).await {
        Ok(results) => {
            access.zip_details(
                results
                    .iter()
                    .map(|result| json!({ "created": result.created })),
            );

            for result in results.iter() {
                record_policy_mutation(
                    &state,
                    if result.created { "create" } else { "update" },
                    "success",
                );

                state.evaluation_cache.invalidate(&(
                    result.policy.principal.clone(),
                    result.policy.action.clone(),
                    result.policy.resource.clone(),
                ));

                publish_permission_event(
                    &state,
                    result.policy.principal.as_ref(),
                    result.policy.action.as_ref(),
                    result.policy.resource.as_ref(),
                );
            }

            Ok(Json(PutPolicyBatchResponse {
                results: results.into_iter().map(PutPolicyResponse::from).collect(),
            }))
        }
        Err(StorageErr {
            kind: StorageErrKind::Forbidden,
            msg,
        }) => {
            record_policy_mutation(&state, "batch", "forbidden");
            Err(access.fail(StatusCode::FORBIDDEN, msg))
        }
        Err(StorageErr {
            kind: StorageErrKind::NotSupported,
            ..
        }) => {
            record_policy_mutation(&state, "batch", "invalid");
            Err(access.fail(
                StatusCode::NOT_IMPLEMENTED,
                "Operation not supported".to_string(),
            ))
        }
        Err(
            err @ StorageErr {
                kind: StorageErrKind::Internal,
                ..
            },
        ) => {
            record_policy_mutation(&state, "batch", "error");
            tracing::error!(error = %err, "Failed to add the policy batch");
            Err(access.fail(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to add the policy".to_string(),
            ))
        }
        Err(StorageErr { msg, .. }) => {
            record_policy_mutation(&state, "batch", "invalid");
            Err(access.fail(StatusCode::BAD_REQUEST, msg))
        }
    }
}

#[utoipa::path(
    delete,
    path = "/v1beta/policies/{id}",
    description = "Removes a Cedar policy with the specified ID from the database.",
    tag = POLICY_TAG,
    params(
        ("id" = i64, Path, description = "Policy ID")
    ),
    responses((status = NO_CONTENT))
)]
#[tracing::instrument(level = "info", skip(access, state))]
pub async fn delete_policy_api(
    Extension(requester): Extension<Arc<Principal>>,
    Extension(access): Extension<AccessLog>,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, (StatusCode, String)> {
    access.params(json!({ "id": id }));

    let result = state.policies.get_policy_by_id(&id).await.ok();

    // Canonicalize the stored scope the same way the authorize path does
    // (remap aliases, then encode) so an edit grant scoped to the canonical
    // URL or its folder hierarchy matches a policy stored under an alias.
    let stored_resource = result.as_ref().and_then(|policy| {
        policy
            .resource
            .clone()
            .or_else(|| policy.resource_prefix.clone())
    });
    let remapped_resource = match state.url_remapper.as_ref() {
        Some(remapper) => remapper.rewrite_resource_opt(stored_resource),
        None => stored_resource,
    };
    let edit_resource = authorization::encode_resource_id(&remapped_resource)
        .map_err(|err| access.fail(StatusCode::BAD_REQUEST, err))?;
    let edit_context = result.as_ref().map(|policy| {
        authorization::build_edit_context(
            policy.principal.as_ref(),
            policy.group.as_ref(),
            policy.action.as_ref(),
        )
    });

    if let Err(err) =
        ensure_permissions_can_be_changed(&state, &requester, edit_resource.as_ref(), edit_context)
            .await
    {
        record_policy_mutation(&state, "delete", "forbidden");
        return Err(access.fail_err(&err));
    }

    let result = match result {
        Some(policy) => policy,
        None => return Ok(StatusCode::NO_CONTENT),
    };

    match state.policies.remove_policy(&id).await {
        Ok(_) => {
            record_policy_mutation(&state, "delete", "success");

            state.evaluation_cache.invalidate(&(
                result.principal.clone(),
                result.action.clone(),
                result.resource.clone(),
            ));

            publish_permission_event(
                &state,
                result.principal.as_ref(),
                result.action.as_ref(),
                result.resource.as_ref(),
            );

            Ok(StatusCode::NO_CONTENT)
        }
        Err(StorageErr {
            kind: StorageErrKind::Forbidden,
            msg,
        }) => {
            record_policy_mutation(&state, "delete", "forbidden");
            Err(access.fail(StatusCode::FORBIDDEN, msg))
        }
        Err(StorageErr {
            kind: StorageErrKind::NotSupported,
            ..
        }) => {
            record_policy_mutation(&state, "delete", "error");
            Err(access.fail(
                StatusCode::NOT_IMPLEMENTED,
                "Operation not supported".to_string(),
            ))
        }
        Err(err) => {
            record_policy_mutation(&state, "delete", "error");
            tracing::error!(error = %err, "Failed to remove the policy");
            Err(access.fail(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to remove the policy".to_string(),
            ))
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema, Validate)]
pub struct PutPolicyPayload {
    #[garde(length(max = 65535))]
    pub policy: String,

    #[garde(skip)]
    pub order: Option<i32>,

    /// Optional free-form note describing what the policy does or who it was
    /// created for. Stored as metadata and never used during authorization.
    #[garde(length(max = 4096))]
    pub comment: Option<String>,
}

/// Trims surrounding whitespace from an administrator-supplied comment and
/// treats an empty result as "no comment", so a `Some("")` or `Some("   ")`
/// payload does not roundtrip as a non-null but empty value in storage.
fn normalize_comment(comment: Option<String>) -> Option<String> {
    comment
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty())
}

/// Records one `policy.mutation.count` event for a single create/update/delete/batch
/// attempt. `operation` is `"create"`, `"update"`, `"delete"`, or `"batch"` (the latter
/// for whole-batch failures that occur before per-item results exist); `outcome` is
/// `"success"`, `"invalid"`, `"forbidden"`, or `"error"`.
fn record_policy_mutation(state: &AppState, operation: &'static str, outcome: &'static str) {
    state.metrics.policy_mutation_count.add(
        1,
        &[
            KeyValue::new("operation", operation),
            KeyValue::new("outcome", outcome),
        ],
    );
}

/// JSON array of [`PutPolicyPayload`] items. Transparent over the wire so the
/// body shape stays `[{...}, ...]`, but exists as a named type so `garde` can
/// enforce the per-request item-count limit on top of per-item validation.
#[derive(Clone, Debug, Deserialize, Serialize, Validate)]
#[serde(transparent)]
pub struct PutPolicyBatchRequest(#[garde(length(max = 100), dive)] pub Vec<PutPolicyPayload>);

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct PolicyResponse {
    pub id: PolicyId,
    pub order: i32,
    pub policy: String,
    pub resource: Option<Resource>,
    /// Hierarchy scope of a `resource in <Type>::"<url>/"` policy. Mutually
    /// exclusive with `resource`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_prefix: Option<Resource>,
    pub action: Option<Action>,
    pub principal: Option<Principal>,
    pub group: Option<Group>,
    pub created_at: String,
    pub created_by: String,
    pub comment: Option<String>,
}

impl From<Policy> for PolicyResponse {
    fn from(value: Policy) -> Self {
        Self {
            id: value.id,
            order: value.order,
            policy: value.text,
            resource: value.resource,
            resource_prefix: value.resource_prefix,
            action: value.action,
            principal: value.principal,
            group: value.group,
            created_at: value.created_at,
            created_by: value.created_by,
            comment: value.comment,
        }
    }
}

/// Response body for the upsert endpoints (`PUT /v1beta/policies/` and each
/// item of `PUT /v1beta/policies/batch/`). Wraps a [`PolicyResponse`] with a
/// `created` flag: `true` when the policy was newly inserted, `false` when an
/// existing policy with the same canonical text was updated in place. The
/// [`PolicyResponse`] fields are flattened so the JSON shape is the existing
/// policy object plus one extra `created` boolean.
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct PutPolicyResponse {
    #[serde(flatten)]
    pub policy: PolicyResponse,
    pub created: bool,
}

impl From<PolicyUpsertResult> for PutPolicyResponse {
    fn from(value: PolicyUpsertResult) -> Self {
        Self {
            policy: PolicyResponse::from(value.policy),
            created: value.created,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct PutPolicyBatchResponse {
    results: Vec<PutPolicyResponse>,
}

#[derive(Clone, Debug, Deserialize, Serialize, IntoParams, ToSchema, Validate)]
pub struct PolicyPageQuery {
    #[garde(skip)]
    principal: Option<String>,
    #[garde(skip)]
    group: Option<String>,
    #[garde(skip)]
    action: Option<String>,
    #[garde(skip)]
    resource: Option<String>,
    #[garde(skip)]
    order: Option<i32>,

    /// Controls how a concrete `resource` filter matches policy scopes. This is
    /// a scope filter, not an authorization decision: global policies
    /// (`permit(principal, action, resource)`) are never attached to a specific
    /// resource and so are not returned here regardless of this value.
    #[garde(skip)]
    #[serde(default)]
    scope: PolicyScopeFilter,

    #[serde(default = "PolicyPageQuery::default_page")]
    #[garde(range(min = 1))]
    page: i64,

    #[serde(default = "PolicyPageQuery::default_limit")]
    #[garde(range(min = 1, max = 50))]
    limit: i64,
}

impl PolicyPageQuery {
    fn default_page() -> i64 {
        1
    }

    fn default_limit() -> i64 {
        10
    }
}

/// Scope-matching mode for a resource-filtered policy listing.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum PolicyScopeFilter {
    /// Only policies scoped directly to the exact `resource` (an exact
    /// `resource ==` match, or a hierarchy scope defined on exactly this URL).
    #[default]
    Exact,

    /// Also include policies inherited from ancestor hierarchy scopes
    /// (`resource in <Type>::"<url>/"`) whose prefix is an ancestor of
    /// `resource`, walking up the folder tree.
    Inherited,
}
