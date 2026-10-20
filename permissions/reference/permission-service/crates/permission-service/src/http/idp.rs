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

use crate::http::middlewares::access_log::AccessLog;
use crate::state::AppState;
use axum::extract::State;
use axum::{Extension, Json};
use jsonwebtoken::jwk::JwkSet;
use serde_json::json;

const IDP_TAG: &str = "idp";

#[utoipa::path(
    get,
    path = "/v1beta/idp/keys/",
    description = "Returns public keys used for token validation.",
    tag = IDP_TAG,
    responses(
        (status = OK, body = Option<serde_json::Value>, description = "JWK Set containing public keys", example = json!({
            "keys": [{
                "kty": "RSA",
                "kid": "example-key-id", 
                "use": "sig",
                "alg": "RS256",
                "n": "example-modulus",
                "e": "AQAB"
            }]
        })),
        (status = INTERNAL_SERVER_ERROR, body = String),
    )
)]
#[tracing::instrument(level = "info", skip(access, state))]
pub async fn get_jwks(
    Extension(access): Extension<AccessLog>,
    State(state): State<AppState>,
) -> Json<Option<JwkSet>> {
    access.params(json!({}));
    let keys = state.auth_cache.get_keys().await;
    Json(keys.map(|k| (*k).clone()))
}
