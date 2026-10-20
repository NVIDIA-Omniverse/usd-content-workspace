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

use std::sync::Arc;

use serde_json::{Map, Value};
use tonic::{Request, Response, Status};
use tracing::{Instrument, warn};

use crate::metrics;

use crate::access::AccessLog;
use crate::config::ForwardHeaders;
use crate::grpc::proto;
use crate::grpc::proto::user_info_service_server::UserInfoService;
use crate::models;
use crate::permission::{
    Action, AuthorizationDecision, PermissionChecker, PermissionError, Resource,
};
use crate::provider::{IdentityProvider, ProviderError};

/// gRPC implementation of the UserInfoService backed by an IdentityProvider.
pub struct UserInfoServiceImpl {
    provider: Arc<dyn IdentityProvider>,
    permission_checker: Option<Arc<PermissionChecker>>,
    access: Arc<AccessLog>,
    forward_headers: Arc<ForwardHeaders>,
}

impl UserInfoServiceImpl {
    pub fn new(
        provider: Arc<dyn IdentityProvider>,
        permission_checker: Option<Arc<PermissionChecker>>,
        access: Arc<AccessLog>,
        forward_headers: Arc<ForwardHeaders>,
    ) -> Self {
        Self {
            provider,
            permission_checker,
            access,
            forward_headers,
        }
    }

    async fn check_permission<T>(
        &self,
        request: &Request<T>,
        action: Action,
        resource: Option<Resource>,
    ) -> Result<(), Status> {
        let checker = match &self.permission_checker {
            Some(c) => c,
            None => return Ok(()),
        };

        let token = extract_bearer_token(request)?;
        let extra_headers = self.forward_headers.select_grpc(request.metadata());

        match checker.check(token, action, resource, extra_headers).await {
            Ok(AuthorizationDecision::Allow | AuthorizationDecision::Skip) => Ok(()),
            Ok(AuthorizationDecision::Deny) => Err(Status::permission_denied("permission denied")),
            Err(PermissionError::Unauthorized(msg)) => Err(Status::unauthenticated(msg)),
            Err(PermissionError::RateLimited { retry_after }) => Err(Status::resource_exhausted(
                format!("rate limited, retry after {retry_after}s"),
            )),
            Err(e) => {
                warn!(%action, error = %e, "permission check failed");
                Err(Status::internal("authorization check failed"))
            }
        }
    }

    /// Post-filters a list of items by batch-checking an action per item.
    ///
    /// The bearer token is `Option<&str>` because authorization is
    /// optional on this service: when no `permission_checker` is
    /// configured the request is allowed through without authentication,
    /// and there is no token to forward. When a checker IS configured the
    /// token is required and a missing one is reported as
    /// `UNAUTHENTICATED`. This mirrors the REST post-filter helper in
    /// `api/mod.rs::filter_page`.
    async fn filter_items<T>(
        &self,
        items: Vec<T>,
        bearer_token: Option<&str>,
        action: Action,
        extra_headers: reqwest::header::HeaderMap,
    ) -> Result<Vec<T>, Status>
    where
        for<'a> Resource: From<&'a T>,
    {
        let checker = match &self.permission_checker {
            Some(c) => c,
            None => return Ok(items),
        };
        if items.is_empty() {
            return Ok(items);
        }
        let Some(token) = bearer_token else {
            return Err(Status::unauthenticated(
                "missing or invalid authorization metadata",
            ));
        };
        let resources = items.iter().map(Resource::from).collect();
        let allowed = checker
            .batch_check(token, action, resources, extra_headers)
            .await
            .map_err(|e| {
                warn!(%action, error = %e, "batch permission check failed");
                Status::internal("authorization check failed")
            })?;
        Ok(allowed
            .into_iter()
            .zip(items)
            .filter_map(|(ok, item)| ok.then_some(item))
            .collect())
    }
}

#[tonic::async_trait]
impl UserInfoService for UserInfoServiceImpl {
    async fn get_current_user(
        &self,
        request: Request<proto::GetCurrentUserRequest>,
    ) -> Result<Response<proto::GetCurrentUserResponse>, Status> {
        let span = crate::telemetry::grpc_span(request.metadata(), "GetCurrentUser");
        let start = std::time::Instant::now();
        let header_fields = self.access.header_fields_grpc(request.metadata());
        let result = async {
            let token = extract_bearer_token(&request)?;
            let user = self.provider.get_me(token).await.map_err(into_status)?;
            Ok(Response::new(proto::GetCurrentUserResponse {
                user: Some(user.into()),
            }))
        }
        .instrument(span)
        .await;
        record_rpc_metrics("GetCurrentUser", &result, start);
        self.access.emit_grpc(
            header_fields,
            operation("GetCurrentUser"),
            Map::new(),
            None,
            &result,
        );
        result
    }

    async fn list_users(
        &self,
        request: Request<proto::ListUsersRequest>,
    ) -> Result<Response<proto::ListUsersResponse>, Status> {
        let span = crate::telemetry::grpc_span(request.metadata(), "ListUsers");
        let start = std::time::Instant::now();
        let header_fields = self.access.header_fields_grpc(request.metadata());
        let params = user_query_params(
            None,
            request.get_ref().email.as_deref(),
            request.get_ref().display_name.as_deref(),
            request.get_ref().page_size,
            &request.get_ref().page_token,
        );
        let result = async {
            self.check_permission(&request, Action::ListUsers, None)
                .await?;
            let token = metadata_bearer_token(&request).map(str::to_owned);
            let extra_headers = self.forward_headers.select_grpc(request.metadata());
            let req = request.into_inner();
            let query = models::UserQuery {
                email: req.email,
                display_name: req.display_name,
                limit: page_size_to_limit(req.page_size),
                cursor: non_empty(req.page_token),
            };
            let page = self
                .provider
                .list_users(&query)
                .await
                .map_err(into_status)?;
            let users = self
                .filter_items(page.items, token.as_deref(), Action::GetUser, extra_headers)
                .await?;
            Ok(Response::new(proto::ListUsersResponse {
                users: users.into_iter().map(Into::into).collect(),
                next_page_token: page.next_cursor.unwrap_or_default(),
            }))
        }
        .instrument(span)
        .await;
        record_rpc_metrics("ListUsers", &result, start);
        let item_count = result.as_ref().ok().map(|r| r.get_ref().users.len());
        self.access.emit_grpc(
            header_fields,
            operation("ListUsers"),
            params,
            item_count,
            &result,
        );
        result
    }

    async fn get_user(
        &self,
        request: Request<proto::GetUserRequest>,
    ) -> Result<Response<proto::GetUserResponse>, Status> {
        let span = crate::telemetry::grpc_span(request.metadata(), "GetUser");
        let start = std::time::Instant::now();
        let header_fields = self.access.header_fields_grpc(request.metadata());
        let mut params = Map::new();
        insert_param(&mut params, "user_id", &request.get_ref().user_id);
        let result = async {
            let resource = Resource::user(&request.get_ref().user_id);
            self.check_permission(&request, Action::GetUser, Some(resource))
                .await?;
            let user_id = request.into_inner().user_id;
            let user = self
                .provider
                .get_user(&user_id)
                .await
                .map_err(into_status)?;
            Ok(Response::new(proto::GetUserResponse {
                user: Some(user.into()),
            }))
        }
        .instrument(span)
        .await;
        record_rpc_metrics("GetUser", &result, start);
        self.access
            .emit_grpc(header_fields, operation("GetUser"), params, None, &result);
        result
    }

    async fn list_groups(
        &self,
        request: Request<proto::ListGroupsRequest>,
    ) -> Result<Response<proto::ListGroupsResponse>, Status> {
        let span = crate::telemetry::grpc_span(request.metadata(), "ListGroups");
        let start = std::time::Instant::now();
        let header_fields = self.access.header_fields_grpc(request.metadata());
        let params = group_query_params(
            None,
            request.get_ref().display_name.as_deref(),
            request.get_ref().page_size,
            &request.get_ref().page_token,
        );
        let result = async {
            self.check_permission(&request, Action::ListGroups, None)
                .await?;
            let token = metadata_bearer_token(&request).map(str::to_owned);
            let extra_headers = self.forward_headers.select_grpc(request.metadata());
            let req = request.into_inner();
            let query = models::GroupQuery {
                display_name: req.display_name,
                limit: page_size_to_limit(req.page_size),
                cursor: non_empty(req.page_token),
            };
            let page = self
                .provider
                .list_groups(&query)
                .await
                .map_err(into_status)?;
            let groups = self
                .filter_items(
                    page.items,
                    token.as_deref(),
                    Action::GetGroup,
                    extra_headers,
                )
                .await?;
            Ok(Response::new(proto::ListGroupsResponse {
                groups: groups.into_iter().map(Into::into).collect(),
                next_page_token: page.next_cursor.unwrap_or_default(),
            }))
        }
        .instrument(span)
        .await;
        record_rpc_metrics("ListGroups", &result, start);
        let item_count = result.as_ref().ok().map(|r| r.get_ref().groups.len());
        self.access.emit_grpc(
            header_fields,
            operation("ListGroups"),
            params,
            item_count,
            &result,
        );
        result
    }

    async fn get_group(
        &self,
        request: Request<proto::GetGroupRequest>,
    ) -> Result<Response<proto::GetGroupResponse>, Status> {
        let span = crate::telemetry::grpc_span(request.metadata(), "GetGroup");
        let start = std::time::Instant::now();
        let header_fields = self.access.header_fields_grpc(request.metadata());
        let mut params = Map::new();
        insert_param(&mut params, "group_id", &request.get_ref().group_id);
        let result = async {
            let resource = Resource::group(&request.get_ref().group_id);
            self.check_permission(&request, Action::GetGroup, Some(resource))
                .await?;
            let group_id = request.into_inner().group_id;
            let group = self
                .provider
                .get_group(&group_id)
                .await
                .map_err(into_status)?;
            Ok(Response::new(proto::GetGroupResponse {
                group: Some(group.into()),
            }))
        }
        .instrument(span)
        .await;
        record_rpc_metrics("GetGroup", &result, start);
        self.access
            .emit_grpc(header_fields, operation("GetGroup"), params, None, &result);
        result
    }

    async fn list_group_members(
        &self,
        request: Request<proto::ListGroupMembersRequest>,
    ) -> Result<Response<proto::ListGroupMembersResponse>, Status> {
        let span = crate::telemetry::grpc_span(request.metadata(), "ListGroupMembers");
        let start = std::time::Instant::now();
        let header_fields = self.access.header_fields_grpc(request.metadata());
        let params = user_query_params(
            Some(("group_id", &request.get_ref().group_id)),
            request.get_ref().email.as_deref(),
            request.get_ref().display_name.as_deref(),
            request.get_ref().page_size,
            &request.get_ref().page_token,
        );
        let result = async {
            let resource = Resource::group(&request.get_ref().group_id);
            self.check_permission(&request, Action::ListGroupMembers, Some(resource))
                .await?;
            let token = metadata_bearer_token(&request).map(str::to_owned);
            let extra_headers = self.forward_headers.select_grpc(request.metadata());
            let req = request.into_inner();
            let query = models::UserQuery {
                email: req.email,
                display_name: req.display_name,
                limit: page_size_to_limit(req.page_size),
                cursor: non_empty(req.page_token),
            };
            let page = self
                .provider
                .list_group_members(&req.group_id, &query)
                .await
                .map_err(into_status)?;
            let users = self
                .filter_items(page.items, token.as_deref(), Action::GetUser, extra_headers)
                .await?;
            Ok(Response::new(proto::ListGroupMembersResponse {
                users: users.into_iter().map(Into::into).collect(),
                next_page_token: page.next_cursor.unwrap_or_default(),
            }))
        }
        .instrument(span)
        .await;
        record_rpc_metrics("ListGroupMembers", &result, start);
        let item_count = result.as_ref().ok().map(|r| r.get_ref().users.len());
        self.access.emit_grpc(
            header_fields,
            operation("ListGroupMembers"),
            params,
            item_count,
            &result,
        );
        result
    }

    async fn list_user_groups(
        &self,
        request: Request<proto::ListUserGroupsRequest>,
    ) -> Result<Response<proto::ListUserGroupsResponse>, Status> {
        let span = crate::telemetry::grpc_span(request.metadata(), "ListUserGroups");
        let start = std::time::Instant::now();
        let header_fields = self.access.header_fields_grpc(request.metadata());
        let params = group_query_params(
            Some(("user_id", &request.get_ref().user_id)),
            request.get_ref().display_name.as_deref(),
            request.get_ref().page_size,
            &request.get_ref().page_token,
        );
        let result = async {
            let resource = Resource::user(&request.get_ref().user_id);
            self.check_permission(&request, Action::ListUserGroups, Some(resource))
                .await?;
            let token = metadata_bearer_token(&request).map(str::to_owned);
            let extra_headers = self.forward_headers.select_grpc(request.metadata());
            let req = request.into_inner();
            let query = models::GroupQuery {
                display_name: req.display_name,
                limit: page_size_to_limit(req.page_size),
                cursor: non_empty(req.page_token),
            };
            let page = self
                .provider
                .list_user_groups(&req.user_id, &query)
                .await
                .map_err(into_status)?;
            let groups = self
                .filter_items(
                    page.items,
                    token.as_deref(),
                    Action::GetGroup,
                    extra_headers,
                )
                .await?;
            Ok(Response::new(proto::ListUserGroupsResponse {
                groups: groups.into_iter().map(Into::into).collect(),
                next_page_token: page.next_cursor.unwrap_or_default(),
            }))
        }
        .instrument(span)
        .await;
        record_rpc_metrics("ListUserGroups", &result, start);
        let item_count = result.as_ref().ok().map(|r| r.get_ref().groups.len());
        self.access.emit_grpc(
            header_fields,
            operation("ListUserGroups"),
            params,
            item_count,
            &result,
        );
        result
    }

    async fn get_user_group(
        &self,
        request: Request<proto::GetUserGroupRequest>,
    ) -> Result<Response<proto::GetUserGroupResponse>, Status> {
        let span = crate::telemetry::grpc_span(request.metadata(), "GetUserGroup");
        let start = std::time::Instant::now();
        let header_fields = self.access.header_fields_grpc(request.metadata());
        let mut params = Map::new();
        insert_param(&mut params, "user_id", &request.get_ref().user_id);
        insert_param(&mut params, "group_id", &request.get_ref().group_id);
        let result = async {
            let resource = Resource::user(&request.get_ref().user_id);
            self.check_permission(&request, Action::GetUserGroup, Some(resource))
                .await?;
            let req = request.into_inner();
            let group = self
                .provider
                .get_user_group(&req.user_id, &req.group_id)
                .await
                .map_err(into_status)?;
            Ok(Response::new(proto::GetUserGroupResponse {
                group: Some(group.into()),
            }))
        }
        .instrument(span)
        .await;
        record_rpc_metrics("GetUserGroup", &result, start);
        self.access.emit_grpc(
            header_fields,
            operation("GetUserGroup"),
            params,
            None,
            &result,
        );
        result
    }

    async fn get_group_member(
        &self,
        request: Request<proto::GetGroupMemberRequest>,
    ) -> Result<Response<proto::GetGroupMemberResponse>, Status> {
        let span = crate::telemetry::grpc_span(request.metadata(), "GetGroupMember");
        let start = std::time::Instant::now();
        let header_fields = self.access.header_fields_grpc(request.metadata());
        let mut params = Map::new();
        insert_param(&mut params, "group_id", &request.get_ref().group_id);
        insert_param(&mut params, "user_id", &request.get_ref().user_id);
        let result = async {
            let resource = Resource::group(&request.get_ref().group_id);
            self.check_permission(&request, Action::GetGroupMember, Some(resource))
                .await?;
            let req = request.into_inner();
            let user = self
                .provider
                .get_group_member(&req.group_id, &req.user_id)
                .await
                .map_err(into_status)?;
            Ok(Response::new(proto::GetGroupMemberResponse {
                user: Some(user.into()),
            }))
        }
        .instrument(span)
        .await;
        record_rpc_metrics("GetGroupMember", &result, start);
        self.access.emit_grpc(
            header_fields,
            operation("GetGroupMember"),
            params,
            None,
            &result,
        );
        result
    }
}

fn extract_bearer_token<T>(request: &Request<T>) -> Result<&str, Status> {
    metadata_bearer_token(request)
        .ok_or_else(|| Status::unauthenticated("missing or invalid authorization metadata"))
}

/// Returns the bearer token from the `authorization` metadata if present,
/// without erroring when it is missing. Use this in handlers that pass the
/// token to authorization helpers (which decide whether the token is
/// required) so that requests against an instance with no permission
/// checker do not fail unnecessarily.
fn metadata_bearer_token<T>(request: &Request<T>) -> Option<&str> {
    let auth = request
        .metadata()
        .get("authorization")
        .and_then(|v| v.to_str().ok());
    crate::http::parse_bearer_token(auth)
}

fn into_status(err: ProviderError) -> Status {
    match err {
        ProviderError::NotFound(msg) => Status::not_found(msg),
        ProviderError::Unauthorized(msg) => Status::unauthenticated(msg),
        ProviderError::Forbidden(msg) => Status::permission_denied(msg),
        ProviderError::InvalidCursor(msg) => Status::invalid_argument(msg),
        ProviderError::Upstream(msg) => Status::unavailable(msg),
        ProviderError::Internal(msg) => Status::internal(msg),
        ProviderError::RateLimited { retry_after } => {
            Status::resource_exhausted(format!("rate limited, retry after {retry_after}s"))
        }
        ProviderError::DeltaExpired => Status::internal("delta token expired"),
    }
}

fn page_size_to_limit(page_size: i32) -> u32 {
    if page_size <= 0 { 20 } else { page_size as u32 }
}

fn non_empty(s: String) -> Option<String> {
    if s.is_empty() { None } else { Some(s) }
}

const RPC_SERVICE: &str = "nvidia.omniverse.userinfo.v1alpha.UserInfoService";

fn record_rpc_metrics<T>(
    method: &str,
    result: &Result<Response<T>, Status>,
    start: std::time::Instant,
) {
    let code = match result {
        Ok(_) => 0i32,
        Err(status) => status.code() as i32,
    };
    metrics::record_rpc_request(RPC_SERVICE, method, code, start.elapsed().as_secs_f64());
}

fn operation(method: &str) -> String {
    format!("/{RPC_SERVICE}/{method}")
}

fn user_query_params(
    id: Option<(&str, &str)>,
    email: Option<&str>,
    display_name: Option<&str>,
    page_size: i32,
    page_token: &str,
) -> Map<String, Value> {
    let mut params = Map::new();
    if let Some((key, value)) = id {
        insert_param(&mut params, key, value);
    }
    insert_param(&mut params, "email", email.unwrap_or_default());
    insert_param(
        &mut params,
        "display_name",
        display_name.unwrap_or_default(),
    );
    insert_page(&mut params, page_size, page_token);
    params
}

fn group_query_params(
    id: Option<(&str, &str)>,
    display_name: Option<&str>,
    page_size: i32,
    page_token: &str,
) -> Map<String, Value> {
    let mut params = Map::new();
    if let Some((key, value)) = id {
        insert_param(&mut params, key, value);
    }
    insert_param(
        &mut params,
        "display_name",
        display_name.unwrap_or_default(),
    );
    insert_page(&mut params, page_size, page_token);
    params
}

fn insert_page(params: &mut Map<String, Value>, page_size: i32, page_token: &str) {
    if page_size > 0 {
        params.insert("page_size".to_owned(), Value::from(page_size));
    }
    insert_param(params, "page_token", page_token);
}

fn insert_param(params: &mut Map<String, Value>, key: &str, value: &str) {
    if !value.is_empty() {
        params.insert(key.to_owned(), Value::from(value));
    }
}

impl From<models::User> for proto::User {
    fn from(u: models::User) -> Self {
        Self {
            id: u.id,
            display_name: u.display_name,
            given_name: u.given_name,
            family_name: u.family_name,
            email: u.email,
            username: u.username,
            active: u.active,
            job_title: u.job_title,
        }
    }
}

impl From<models::Group> for proto::Group {
    fn from(g: models::Group) -> Self {
        Self {
            id: g.id,
            display_name: g.display_name,
            description: g.description,
        }
    }
}
