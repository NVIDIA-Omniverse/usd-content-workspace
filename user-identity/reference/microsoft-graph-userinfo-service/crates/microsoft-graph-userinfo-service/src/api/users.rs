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

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};

use crate::api::AppState;
use crate::api::errors::ApiError;
use crate::models::{Group, GroupQuery, User, UserQuery};
use crate::permission::Action;

pub async fn list_users(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<UserQuery>,
) -> Result<Response, ApiError> {
    let page = state.provider.list_users(&params).await?;
    let page = super::filter_page(&state, &headers, Action::GetUser, page).await?;
    let detail = super::item_count_detail(page.items.len());
    Ok((Extension(detail), Json(page)).into_response())
}

pub async fn get_user(
    State(state): State<AppState>,
    Path(user_id): Path<String>,
) -> Result<Json<User>, ApiError> {
    let user = state.provider.get_user(&user_id).await?;
    Ok(Json(user))
}

pub async fn get_current_user(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<User>, ApiError> {
    let token = crate::http::extract_bearer(&headers)
        .ok_or_else(|| ApiError::Unauthorized("missing or invalid Authorization header".into()))?;

    let user = state.provider.get_me(token).await?;
    Ok(Json(user))
}

pub async fn list_user_groups(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
    Query(params): Query<GroupQuery>,
) -> Result<Response, ApiError> {
    let page = state.provider.list_user_groups(&user_id, &params).await?;
    let page = super::filter_page(&state, &headers, Action::GetGroup, page).await?;
    let detail = super::item_count_detail(page.items.len());
    Ok((Extension(detail), Json(page)).into_response())
}

pub async fn get_user_group(
    State(state): State<AppState>,
    Path((user_id, group_id)): Path<(String, String)>,
) -> Result<Json<Group>, ApiError> {
    let group = state.provider.get_user_group(&user_id, &group_id).await?;
    Ok(Json(group))
}
