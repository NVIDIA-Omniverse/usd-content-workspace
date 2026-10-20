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

use crate::events::publisher::{publish_metadata_event, MetadataEntity, MetadataOperation};
use crate::http::middlewares::access_log::AccessLog;
use crate::service::authorization::{
    ensure_metadata_can_be_changed, ensure_metadata_can_be_read, PERMISSIONS_SERVICE_NAME,
};
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::{Extension, Json};
use permission_core::{Action, Principal};
use permission_db::{ResourceTypeMeta, ServiceMeta, StorageErr, StorageErrKind};
use reqwest::StatusCode;
use serde_json::{json, Value};
use std::sync::Arc;

const METADATA_TAG: &str = "metadata";

/// Invalidates caches keyed under the intrinsic `permissions` service. Meta
/// checks resolve a resource's hierarchy `parentType` from its resource type
/// metadata and cache both the resolved `Meta` and the decision under
/// `permissions`, so any resource-type mutation must clear them.
fn invalidate_meta_caches(state: &AppState) {
    state
        .metadata_cache
        .invalidate_service(PERMISSIONS_SERVICE_NAME);
    state
        .evaluation_cache
        .invalidate_service(PERMISSIONS_SERVICE_NAME);
}

#[utoipa::path(
    get,
    path = "/v1beta/services/",
    description = "Returns all services and their metadata registered in the database.",
    tag = METADATA_TAG,
    responses(
        (status = OK, body = Vec<ServiceMeta>),
        (status = UNPROCESSABLE_ENTITY, body = String)
    )
)]
#[tracing::instrument(level = "info", skip(requester, access, state))]
pub async fn get_service(
    Extension(requester): Extension<Arc<Principal>>,
    Extension(access): Extension<AccessLog>,
    State(state): State<AppState>,
) -> Result<Json<Vec<ServiceMeta>>, (StatusCode, String)> {
    access.params(json!({}));

    ensure_metadata_can_be_read(&state, &requester)
        .await
        .map_err(|err| access.fail_err(&err))?;

    match state.metadata.get_services().await {
        Ok(services) => Ok(Json(services)),
        Err(StorageErr {
            kind: StorageErrKind::NotSupported,
            ..
        }) => Err(access.fail(
            StatusCode::NOT_IMPLEMENTED,
            "Operation not supported".to_string(),
        )),
        Err(err) => {
            tracing::error!(error = %err, "Metadata storage operation failed");
            Err(access.fail(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error".to_string(),
            ))
        }
    }
}

#[utoipa::path(
    get,
    path = "/v1beta/services/{service}/",
    description = "Returns metadata for the specified service.",
    tag = METADATA_TAG,
    params(
        ("service" = String, Path)
    ),
    responses(
        (status = OK, body = ServiceMeta),
        (status = UNPROCESSABLE_ENTITY, body = String)
    )
)]
#[tracing::instrument(level = "info", skip(requester, access, state))]
pub async fn get_service_metadata(
    Extension(requester): Extension<Arc<Principal>>,
    Extension(access): Extension<AccessLog>,
    State(state): State<AppState>,
    Path(service): Path<String>,
) -> Result<Json<ServiceMeta>, (StatusCode, String)> {
    access.params(json!({ "service": service }));

    ensure_metadata_can_be_read(&state, &requester)
        .await
        .map_err(|err| access.fail_err(&err))?;

    match state.metadata.get_service_meta(&service).await {
        Ok(principal) => Ok(Json(principal)),
        Err(StorageErr {
            kind: StorageErrKind::NotSupported,
            ..
        }) => Err(access.fail(
            StatusCode::NOT_IMPLEMENTED,
            "Operation not supported".to_string(),
        )),
        Err(err) => {
            tracing::error!(error = %err, "Metadata storage operation failed");
            Err(access.fail(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error".to_string(),
            ))
        }
    }
}

#[utoipa::path(
    put,
    path = "/v1beta/services/{service}/",
    description = "Updates metadata for the specified service.",
    tag = METADATA_TAG,
    params(
        ("service" = String, Path)
    ),
    request_body = ServiceMeta,
    responses(
        (status = OK, body = ServiceMeta),
        (status = UNPROCESSABLE_ENTITY, body = String),
    )
)]
#[tracing::instrument(level = "info", skip(requester, access, state))]
pub async fn put_service_metadata(
    Extension(requester): Extension<Arc<Principal>>,
    Extension(access): Extension<AccessLog>,
    State(state): State<AppState>,
    Path(service): Path<String>,
    Json(mut payload): Json<ServiceMeta>,
) -> Result<Json<ServiceMeta>, (StatusCode, String)> {
    access.params(json!({ "service": service, "id_claim": payload.id_claim }));

    ensure_metadata_can_be_changed(&state, &requester)
        .await
        .map_err(|err| access.fail_err(&err))?;

    payload.service = service.clone();

    match state.metadata.set_service_meta(&payload).await {
        Ok(()) => {
            state.evaluation_cache.invalidate_service(&payload.service);
            state.metadata_cache.invalidate_service(&payload.service);
            publish_metadata_event(
                &state,
                &payload.service,
                MetadataEntity::Service,
                MetadataOperation::Updated,
                None,
            );
            Ok(Json(payload))
        }
        Err(StorageErr {
            kind: StorageErrKind::NotSupported,
            ..
        }) => Err(access.fail(
            StatusCode::NOT_IMPLEMENTED,
            "Operation not supported".to_string(),
        )),
        Err(err) => {
            tracing::error!(error = %err, "Metadata storage operation failed");
            Err(access.fail(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error".to_string(),
            ))
        }
    }
}

#[utoipa::path(
    delete,
    path = "/v1beta/services/{service}/",
    description = "Removes metadata for the specified service.",
    tag = METADATA_TAG,
    params(
        ("service" = String, Path)
    ),
    responses(
        (status = NO_CONTENT),
        (status = UNPROCESSABLE_ENTITY, body = String),
    )
)]
#[tracing::instrument(level = "info", skip(requester, access, state))]
pub async fn delete_service_metadata(
    Extension(requester): Extension<Arc<Principal>>,
    Extension(access): Extension<AccessLog>,
    State(state): State<AppState>,
    Path(service): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    access.params(json!({ "service": service }));

    ensure_metadata_can_be_changed(&state, &requester)
        .await
        .map_err(|err| access.fail_err(&err))?;

    match state.metadata.delete_service_meta(&service).await {
        Ok(()) => {
            state.evaluation_cache.invalidate_service(&service);
            state.metadata_cache.invalidate_service(&service);
            publish_metadata_event(
                &state,
                &service,
                MetadataEntity::Service,
                MetadataOperation::Removed,
                None,
            );
            Ok(StatusCode::NO_CONTENT)
        }
        Err(StorageErr {
            kind: StorageErrKind::NotSupported,
            ..
        }) => Err(access.fail(
            StatusCode::NOT_IMPLEMENTED,
            "Operation not supported".to_string(),
        )),
        Err(err) => {
            tracing::error!(error = %err, "Metadata storage operation failed");
            Err(access.fail(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error".to_string(),
            ))
        }
    }
}

#[utoipa::path(
    get,
    path = "/v1beta/services/{service}/actions/",
    description = "Gets registered actions for the specified service.",
    tag = METADATA_TAG,
    params(
        ("service" = String, Path)
    ),
    responses(
        (status = OK, body = Vec<Action>),
        (status = UNPROCESSABLE_ENTITY, body = String)
    )
)]
#[tracing::instrument(level = "info", skip(requester, access, state))]
pub async fn get_service_actions(
    Extension(requester): Extension<Arc<Principal>>,
    Extension(access): Extension<AccessLog>,
    State(state): State<AppState>,
    Path(service): Path<String>,
) -> Result<Json<Vec<Action>>, (StatusCode, String)> {
    access.params(json!({ "service": service }));

    ensure_metadata_can_be_read(&state, &requester)
        .await
        .map_err(|err| access.fail_err(&err))?;

    match state.metadata.get_service_actions(&service).await {
        Ok(actions) => Ok(Json(actions)),
        Err(StorageErr {
            kind: StorageErrKind::NotSupported,
            ..
        }) => Err(access.fail(
            StatusCode::NOT_IMPLEMENTED,
            "Operation not supported".to_string(),
        )),
        Err(err) => {
            tracing::error!(error = %err, "Metadata storage operation failed");
            Err(access.fail(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error".to_string(),
            ))
        }
    }
}

#[utoipa::path(
    put,
    path = "/v1beta/services/{service}/actions/",
    description = "Replaces registered actions for the specified service (deletes all existing actions and replaces them with the specified value).",
    tag = METADATA_TAG,
    params(
        ("service" = String, Path)
    ),
    request_body = Vec<Action>,
    responses(
        (status = OK),
        (status = UNPROCESSABLE_ENTITY, body = String)
    )
)]
#[tracing::instrument(level = "info", skip(requester, access, state))]
pub async fn replace_service_actions(
    Extension(requester): Extension<Arc<Principal>>,
    Extension(access): Extension<AccessLog>,
    State(state): State<AppState>,
    Path(service): Path<String>,
    Json(mut payload): Json<Vec<Action>>,
) -> Result<StatusCode, (StatusCode, String)> {
    if access.is_enabled() {
        if payload.is_empty() {
            access.params(json!({ "service": service }));
        } else {
            access.items(
                payload
                    .iter()
                    .map(|action| json!({ "service": service, "name": action.name })),
            );
        }
    }

    ensure_metadata_can_be_changed(&state, &requester)
        .await
        .map_err(|err| access.fail_err(&err))?;

    for action in payload.iter_mut() {
        action.service = service.clone();
    }

    match state
        .metadata
        .replace_service_actions(&service, payload)
        .await
    {
        Ok(_) => {
            state.metadata_cache.invalidate_service(&service);
            state.evaluation_cache.invalidate_service(&service);
            publish_metadata_event(
                &state,
                &service,
                MetadataEntity::Action,
                MetadataOperation::Updated,
                None,
            );
            Ok(StatusCode::OK)
        }
        Err(StorageErr {
            kind: StorageErrKind::NotSupported,
            ..
        }) => Err(access.fail(
            StatusCode::NOT_IMPLEMENTED,
            "Operation not supported".to_string(),
        )),
        Err(err) => {
            tracing::error!(error = %err, "Metadata storage operation failed");
            Err(access.fail(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error".to_string(),
            ))
        }
    }
}

#[utoipa::path(
    put,
    path = "/v1beta/services/{service}/actions/{action}/",
    description = "Adds or updates an action for the specified service.",
    tag = METADATA_TAG,
    params(
        ("service" = String, Path),
        ("action" = String, Path)
    ),
    responses(
        (status = OK),
        (status = UNPROCESSABLE_ENTITY, body = String)
    )
)]
#[tracing::instrument(level = "info", skip(requester, access, state))]
pub async fn put_service_action(
    Extension(requester): Extension<Arc<Principal>>,
    Extension(access): Extension<AccessLog>,
    State(state): State<AppState>,
    Path((service, action)): Path<(String, String)>,
) -> Result<Json<Action>, (StatusCode, String)> {
    access.params(json!({ "service": service, "name": action }));

    ensure_metadata_can_be_changed(&state, &requester)
        .await
        .map_err(|err| access.fail_err(&err))?;

    match state.metadata.set_service_action(&service, &action).await {
        Ok(created) => {
            state.metadata_cache.invalidate_service(&service);
            state.evaluation_cache.invalidate_service(&service);
            publish_metadata_event(
                &state,
                &service,
                MetadataEntity::Action,
                MetadataOperation::Updated,
                Some(&action),
            );
            Ok(Json(created))
        }
        Err(StorageErr {
            kind: StorageErrKind::NotSupported,
            ..
        }) => Err(access.fail(
            StatusCode::NOT_IMPLEMENTED,
            "Operation not supported".to_string(),
        )),
        Err(err) => {
            tracing::error!(error = %err, "Metadata storage operation failed");
            Err(access.fail(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error".to_string(),
            ))
        }
    }
}

#[utoipa::path(
    delete,
    path = "/v1beta/services/{service}/actions/{action}/",
    description = "Deletes an action with the specified name for the specified service.",
    tag = METADATA_TAG,
    params(
        ("service" = String, Path),
        ("action" = String, Path)
    ),
    responses(
        (status = NO_CONTENT),
        (status = UNPROCESSABLE_ENTITY, body = String)
    )
)]
#[tracing::instrument(level = "info", skip(requester, access, state))]
pub async fn delete_service_action(
    Extension(requester): Extension<Arc<Principal>>,
    Extension(access): Extension<AccessLog>,
    State(state): State<AppState>,
    Path((service, action)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    access.params(json!({ "service": service, "name": action }));

    ensure_metadata_can_be_changed(&state, &requester)
        .await
        .map_err(|err| access.fail_err(&err))?;

    match state
        .metadata
        .delete_service_action(&service, &action)
        .await
    {
        Ok(_) => {
            state.metadata_cache.invalidate_service(&service);
            state.evaluation_cache.invalidate_service(&service);
            publish_metadata_event(
                &state,
                &service,
                MetadataEntity::Action,
                MetadataOperation::Removed,
                Some(&action),
            );
            Ok(StatusCode::NO_CONTENT)
        }
        Err(StorageErr {
            kind: StorageErrKind::NotSupported,
            ..
        }) => Err(access.fail(
            StatusCode::NOT_IMPLEMENTED,
            "Operation not supported".to_string(),
        )),
        Err(err) => {
            tracing::error!(error = %err, "Metadata storage operation failed");
            Err(access.fail(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error".to_string(),
            ))
        }
    }
}

#[utoipa::path(
    get,
    path = "/v1beta/services/{service}/resource-types/",
    description = "Gets registered resource types for the specified service.",
    tag = METADATA_TAG,
    params(
        ("service" = String, Path)
    ),
    responses(
        (status = OK, body = Vec<ResourceTypeMeta>),
        (status = UNPROCESSABLE_ENTITY, body = String)
    )
)]
#[tracing::instrument(level = "info", skip(requester, access, state))]
pub async fn get_service_resource_types(
    Extension(requester): Extension<Arc<Principal>>,
    Extension(access): Extension<AccessLog>,
    State(state): State<AppState>,
    Path(service): Path<String>,
) -> Result<Json<Vec<ResourceTypeMeta>>, (StatusCode, String)> {
    access.params(json!({ "service": service }));

    ensure_metadata_can_be_read(&state, &requester)
        .await
        .map_err(|err| access.fail_err(&err))?;

    match state.metadata.get_service_resource_types(&service).await {
        Ok(resource_types) => Ok(Json(resource_types)),
        Err(StorageErr {
            kind: StorageErrKind::NotSupported,
            ..
        }) => Err(access.fail(
            StatusCode::NOT_IMPLEMENTED,
            "Operation not supported".to_string(),
        )),
        Err(err) => {
            tracing::error!(error = %err, "Metadata storage operation failed");
            Err(access.fail(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error".to_string(),
            ))
        }
    }
}

#[utoipa::path(
    put,
    path = "/v1beta/services/{service}/resource-types/",
    description = "Replaces registered resource types for the specified service (deletes all existing resource types and replaces them with specified value).",
    tag = METADATA_TAG,
    params(
        ("service" = String, Path)
    ),
    request_body = Vec<ResourceTypeMeta>,
    responses(
        (status = OK),
        (status = UNPROCESSABLE_ENTITY, body = String)
    )
)]
#[tracing::instrument(level = "info", skip(requester, access, state))]
pub async fn replace_service_resource_types(
    Extension(requester): Extension<Arc<Principal>>,
    Extension(access): Extension<AccessLog>,
    State(state): State<AppState>,
    Path(service): Path<String>,
    Json(mut payload): Json<Vec<ResourceTypeMeta>>,
) -> Result<StatusCode, (StatusCode, String)> {
    if access.is_enabled() {
        if payload.is_empty() {
            access.params(json!({ "service": service }));
        } else {
            access.items(
                payload
                    .iter()
                    .map(|resource_type| resource_type_params(&service, resource_type)),
            );
        }
    }

    ensure_metadata_can_be_changed(&state, &requester)
        .await
        .map_err(|err| access.fail_err(&err))?;

    for resource_type in payload.iter_mut() {
        resource_type.service = service.clone();
    }

    match state
        .metadata
        .replace_resource_types(&service, payload)
        .await
    {
        Ok(_) => {
            state.metadata_cache.invalidate_service(&service);
            state.evaluation_cache.invalidate_service(&service);
            invalidate_meta_caches(&state);
            publish_metadata_event(
                &state,
                &service,
                MetadataEntity::ResourceType,
                MetadataOperation::Updated,
                None,
            );
            Ok(StatusCode::OK)
        }
        Err(StorageErr {
            kind: StorageErrKind::Conflict,
            msg,
        }) => Err(access.fail(StatusCode::CONFLICT, msg)),
        Err(StorageErr {
            kind: StorageErrKind::NotSupported,
            ..
        }) => Err(access.fail(
            StatusCode::NOT_IMPLEMENTED,
            "Operation not supported".to_string(),
        )),
        Err(err) => {
            tracing::error!(error = %err, "Metadata storage operation failed");
            Err(access.fail(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error".to_string(),
            ))
        }
    }
}

/// Builds the access-log `params` for a resource-type entry: path `service`, the
/// resource `type`, `evaluation_priority`, and `parent_type` when present.
fn resource_type_params(service: &str, resource_type: &ResourceTypeMeta) -> Value {
    let mut params = serde_json::Map::new();
    params.insert("service".to_string(), json!(service));
    params.insert("type".to_string(), json!(resource_type.kind));
    params.insert(
        "evaluation_priority".to_string(),
        serde_json::to_value(&resource_type.evaluation_priority).unwrap_or(Value::Null),
    );
    if let Some(parent_type) = &resource_type.parent_type {
        params.insert("parent_type".to_string(), json!(parent_type));
    }
    Value::Object(params)
}

#[utoipa::path(
    get,
    path = "/v1beta/services/{service}/resource-types/{type}/",
    description = "Gets metadata for the specified service resource type.",
    tag = METADATA_TAG,
    params(
        ("service" = String, Path),
        ("type" = String, Path)
    ),
    responses(
        (status = OK, body = ResourceTypeMeta),
        (status = UNPROCESSABLE_ENTITY, body = String)
    )
)]
#[tracing::instrument(level = "info", skip(requester, access, state))]
pub async fn get_resource_metadata(
    Extension(requester): Extension<Arc<Principal>>,
    Extension(access): Extension<AccessLog>,
    State(state): State<AppState>,
    Path((service, resource_type)): Path<(String, String)>,
) -> Result<Json<ResourceTypeMeta>, (StatusCode, String)> {
    access.params(json!({ "service": service, "type": resource_type }));

    ensure_metadata_can_be_read(&state, &requester)
        .await
        .map_err(|err| access.fail_err(&err))?;

    match state
        .metadata
        .get_resource_type(&service, &resource_type)
        .await
    {
        Ok(resource) => Ok(Json(resource)),
        Err(StorageErr {
            kind: StorageErrKind::NotSupported,
            ..
        }) => Err(access.fail(
            StatusCode::NOT_IMPLEMENTED,
            "Operation not supported".to_string(),
        )),
        Err(err) => {
            tracing::error!(error = %err, "Metadata storage operation failed");
            Err(access.fail(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error".to_string(),
            ))
        }
    }
}

#[utoipa::path(
    put,
    path = "/v1beta/services/{service}/resource-types/{type}/",
    description = "Updates metadata for the specified service resource type.",
    tag = METADATA_TAG,
    params(
        ("service" = String, Path),
        ("type" = String, Path)
    ),
    request_body = ResourceTypeMeta,
    responses(
        (status = OK, body = ResourceTypeMeta),
        (status = UNPROCESSABLE_ENTITY, body = String),
    )
)]
#[tracing::instrument(level = "info", skip(requester, access, state))]
pub async fn put_resource_metadata(
    Extension(requester): Extension<Arc<Principal>>,
    Extension(access): Extension<AccessLog>,
    State(state): State<AppState>,
    Path((service, resource_type)): Path<(String, String)>,
    Json(mut payload): Json<ResourceTypeMeta>,
) -> Result<Json<ResourceTypeMeta>, (StatusCode, String)> {
    payload.service = service.clone();
    payload.kind = resource_type.clone();
    access.params(resource_type_params(&service, &payload));

    ensure_metadata_can_be_changed(&state, &requester)
        .await
        .map_err(|err| access.fail_err(&err))?;

    match state.metadata.set_resource_type_meta(&payload).await {
        Ok(()) => {
            let cache_key = (service.clone(), Some(resource_type.clone()));
            state.metadata_cache.invalidate(&cache_key).await;
            state.evaluation_cache.invalidate_service(&service);
            invalidate_meta_caches(&state);
            publish_metadata_event(
                &state,
                &service,
                MetadataEntity::ResourceType,
                MetadataOperation::Updated,
                Some(&resource_type),
            );

            Ok(Json(payload))
        }
        Err(StorageErr {
            kind: StorageErrKind::Conflict,
            msg,
        }) => Err(access.fail(StatusCode::CONFLICT, msg)),
        Err(StorageErr {
            kind: StorageErrKind::NotSupported,
            ..
        }) => Err(access.fail(
            StatusCode::NOT_IMPLEMENTED,
            "Operation not supported".to_string(),
        )),
        Err(err) => {
            tracing::error!(error = %err, "Metadata storage operation failed");
            Err(access.fail(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error".to_string(),
            ))
        }
    }
}

#[utoipa::path(
    delete,
    path = "/v1beta/services/{service}/resource-types/{type}/",
    description = "Deletes metadata for the specified service resource type.",
    tag = METADATA_TAG,
    params(
        ("service" = String, Path),
        ("type" = String, Path)
    ),
    responses(
        (status = NO_CONTENT),
        (status = UNPROCESSABLE_ENTITY, body = String),
    )
)]
#[tracing::instrument(level = "info", skip(requester, access, state))]
pub async fn delete_resource_metadata(
    Extension(requester): Extension<Arc<Principal>>,
    Extension(access): Extension<AccessLog>,
    State(state): State<AppState>,
    Path((service, resource_type)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    access.params(json!({ "service": service, "type": resource_type }));

    ensure_metadata_can_be_changed(&state, &requester)
        .await
        .map_err(|err| access.fail_err(&err))?;

    match state
        .metadata
        .delete_resource_type_meta(&service, &resource_type)
        .await
    {
        Ok(()) => {
            let cache_key = (service.clone(), Some(resource_type.clone()));
            state.metadata_cache.invalidate(&cache_key).await;
            state.evaluation_cache.invalidate_service(&service);
            invalidate_meta_caches(&state);
            publish_metadata_event(
                &state,
                &service,
                MetadataEntity::ResourceType,
                MetadataOperation::Removed,
                Some(&resource_type),
            );

            Ok(StatusCode::NO_CONTENT)
        }
        Err(StorageErr {
            kind: StorageErrKind::NotSupported,
            ..
        }) => Err(access.fail(
            StatusCode::NOT_IMPLEMENTED,
            "Operation not supported".to_string(),
        )),
        Err(err) => {
            tracing::error!(error = %err, "Metadata storage operation failed");
            Err(access.fail(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error".to_string(),
            ))
        }
    }
}
