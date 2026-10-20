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

use crate::access_log::AccessLog;
use crate::service::authorization::{
    access_detail_single, access_details_batch, access_params_batch, access_params_single,
    authorize, authorize_batch, AuthorizationBatch, AuthorizationBatchCondition,
    AuthorizationBatchRequest, AuthorizationDecision, AuthorizationPayload,
};
use crate::state::AppState;
use garde::Validate;
use permission_core::{Action, Principal, Resource};
use permission_service_proto::nvidia::omniverse::permission::v1beta::permission_service_server::PermissionService;
use permission_service_proto::nvidia::omniverse::permission::v1beta::{
    CheckPermissionBatchRequest, CheckPermissionBatchResponse, CheckPermissionBatchResponseSummary,
    CheckPermissionRequest, CheckPermissionResponse, Condition, Decision, ResourceActionDecision,
    ResourceActionDecisionBatch,
};
use std::str::FromStr;
use std::sync::Arc;
use tonic::{metadata::MetadataValue, Code, Request, Response, Status};

pub struct PermissionServiceImpl {
    pub(crate) state: AppState,
}

#[tonic::async_trait]
impl PermissionService for PermissionServiceImpl {
    async fn check_permission(
        &self,
        request: Request<CheckPermissionRequest>,
    ) -> Result<Response<CheckPermissionResponse>, Status> {
        let requester: Arc<Principal> = request
            .extensions()
            .get::<Arc<Principal>>()
            .cloned()
            .expect("Principal extension is not added for gRPC.");
        let access: AccessLog = request
            .extensions()
            .get::<AccessLog>()
            .cloned()
            .expect("AccessLog extension is not added for gRPC.");

        let data = request.into_inner();
        let principal = data.principal.map(|p| Principal {
            id: p.sub,
            info: p.info.map(to_serde),
        });

        let action = if let Some(action) = data.action {
            Action {
                service: action.service,
                name: action.name,
            }
        } else {
            return Err(access.fail_grpc(Status::new(
                Code::InvalidArgument,
                "\"action\" is required.",
            )));
        };

        let resource = data.resource.map(|r| Resource {
            id: r.id,
            kind: r.r#type,
            data: r.data.map(to_serde),
        });
        let context = data.context.map(to_serde);

        let payload = AuthorizationPayload {
            principal,
            action,
            resource,
            context,
        };
        access.params(access_params_single(&payload));
        payload
            .validate()
            .map_err(|e| access.fail_grpc(Status::new(Code::InvalidArgument, e.to_string())))?;

        match authorize(payload, self.state.clone(), &requester).await {
            Ok(response) => {
                access.detail(access_detail_single(&response));

                let mut grpc_response = Response::new(CheckPermissionResponse {
                    decision: to_decision(&response.decision),
                    reason: response.reason,
                });

                // Add Cache-Control header if cache_control_max_age is configured
                if self.state.config.cache.cache_control_max_age > 0 {
                    let cache_control_value = format!(
                        "max-age={}, must-revalidate",
                        self.state.config.cache.cache_control_max_age
                    );
                    if let Ok(metadata_value) = MetadataValue::from_str(&cache_control_value) {
                        grpc_response
                            .metadata_mut()
                            .insert("cache-control", metadata_value);
                    }
                }

                Ok(grpc_response)
            }
            Err(err) => Err(access.fail_grpc(err.to_grpc())),
        }
    }

    async fn check_permission_batch(
        &self,
        request: Request<CheckPermissionBatchRequest>,
    ) -> Result<Response<CheckPermissionBatchResponse>, Status> {
        let requester: Arc<Principal> = request
            .extensions()
            .get::<Arc<Principal>>()
            .cloned()
            .expect("Principal extension is not added for gRPC.");
        let access: AccessLog = request
            .extensions()
            .get::<AccessLog>()
            .cloned()
            .expect("AccessLog extension is not added for gRPC.");

        let data = request.into_inner();

        let condition = match data.condition() {
            Condition::Unspecified => AuthorizationBatchCondition::None,
            Condition::Or => AuthorizationBatchCondition::Or,
            Condition::And => AuthorizationBatchCondition::And,
        };

        let mut action_order: Vec<Vec<Action>> = Vec::with_capacity(data.batches.len());
        let mut batches: Vec<AuthorizationBatch> = Vec::with_capacity(data.batches.len());
        for batch in data.batches.into_iter() {
            let principal = batch.principal.map(|p| Principal {
                id: p.sub,
                info: p.info.map(to_serde),
            });

            let actions: Vec<Action> = batch
                .actions
                .into_iter()
                .map(|a| Action {
                    service: a.service,
                    name: a.name,
                })
                .collect();

            let resource = batch.resource.map(|r| Resource {
                id: r.id,
                kind: r.r#type,
                data: r.data.map(to_serde),
            });
            let context = batch.context.map(to_serde);

            action_order.push(actions.clone());
            batches.push(AuthorizationBatch {
                principal,
                actions,
                resource,
                context,
            });
        }

        let request = AuthorizationBatchRequest {
            condition: Some(condition),
            batches,
        };
        if access.is_enabled() {
            access.items(access_params_batch(request.condition, &request.batches));
        }
        request
            .validate()
            .map_err(|e| access.fail_grpc(Status::new(Code::InvalidArgument, e.to_string())))?;

        let logged_batches = access.is_enabled().then(|| request.batches.clone());
        match authorize_batch(request, self.state.clone(), &requester).await {
            Ok(response) => {
                if let Some(logged_batches) = &logged_batches {
                    access.zip_details(access_details_batch(logged_batches, &response.decisions));
                }

                let summary = response
                    .summary
                    .map(|s| CheckPermissionBatchResponseSummary {
                        decision: to_decision(&s.decision),
                        reason: s.reason,
                    });

                let mut decisions = vec![];
                for (batch_idx, decision_map) in response.decisions.iter().enumerate() {
                    let mut results = Vec::new();
                    let actions = &action_order[batch_idx];
                    for action in actions {
                        let key = action.entity_id();
                        let result = decision_map.get(&key).ok_or_else(|| {
                            access.fail_grpc(Status::new(
                                Code::Internal,
                                format!("Missing decision for action '{}'", key),
                            ))
                        })?;

                        results.push(ResourceActionDecision {
                            service: action.service.clone(),
                            action: action.name.clone(),
                            decision: to_decision(&result.decision),
                            reason: result.reason.clone(),
                        });
                    }

                    let batch_decision = ResourceActionDecisionBatch { results };
                    decisions.push(batch_decision);
                }

                let mut grpc_response =
                    Response::new(CheckPermissionBatchResponse { summary, decisions });

                // Add Cache-Control header if cache_control_max_age is configured
                if self.state.config.cache.cache_control_max_age > 0 {
                    let cache_control_value = format!(
                        "max-age={}, must-revalidate",
                        self.state.config.cache.cache_control_max_age
                    );
                    if let Ok(metadata_value) = MetadataValue::from_str(&cache_control_value) {
                        grpc_response
                            .metadata_mut()
                            .insert("cache-control", metadata_value);
                    }
                }

                Ok(grpc_response)
            }
            Err(err) => Err(access.fail_grpc(err.to_grpc())),
        }
    }
}

fn to_serde(prost_struct: prost_types::Struct) -> serde_json::Value {
    let fields = prost_struct.fields;
    let mut map = serde_json::Map::new();

    for (key, value) in fields {
        map.insert(key, value_to_json(value));
    }

    serde_json::Value::Object(map)
}

fn value_to_json(value: prost_types::Value) -> serde_json::Value {
    match value.kind {
        Some(prost_types::value::Kind::NullValue(_)) => serde_json::Value::Null,
        Some(prost_types::value::Kind::NumberValue(n)) => serde_json::Value::Number(
            serde_json::Number::from_f64(n).unwrap_or(serde_json::Number::from(0)),
        ),
        Some(prost_types::value::Kind::StringValue(s)) => serde_json::Value::String(s),
        Some(prost_types::value::Kind::BoolValue(b)) => serde_json::Value::Bool(b),
        Some(prost_types::value::Kind::StructValue(s)) => to_serde(s),
        Some(prost_types::value::Kind::ListValue(l)) => {
            let values = l.values.into_iter().map(value_to_json).collect();
            serde_json::Value::Array(values)
        }
        None => serde_json::Value::Null,
    }
}

fn to_decision(decision: &AuthorizationDecision) -> i32 {
    let result = match decision {
        AuthorizationDecision::Deny => Decision::Deny,
        AuthorizationDecision::Allow => Decision::Allow,
        AuthorizationDecision::Skip => Decision::Skip,
    };
    result as i32
}
