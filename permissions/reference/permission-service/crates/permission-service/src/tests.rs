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

use crate::create_database_pool;
use crate::metrics::Metrics;
use axum_test::{TestResponse, TestServer};
use clap::Parser;
use once_cell::sync::Lazy;
use opentelemetry::metrics::MeterProvider;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use permission_auth::claim_rules::parse_claim_rules;
use permission_auth::config::AuthType;
use permission_auth::AuthCache;
use permission_core::{Action, Principal, Resource};
use permission_db::config_file::{ConfigFileMetadataStorage, ConfigFilePolicyStorage};
use permission_db::postgres::{
    PostgresDatabaseOptions, PostgresMetadataStorage, PostgresPolicyStorage,
};
use permission_db::{ConnectionPool, EvaluationPriority, MetadataStorage, PolicyStorage};
use permission_service::cache::Caches;
use permission_service::config::ServiceConfig;
use permission_service::http::configure_router;
use permission_service::pool::InstrumentedPool;
use permission_service::service::authorization::{
    encode_resource_id, encode_resource_id_prefix, set_system_metadata, set_system_policies,
    AuthorizationBatchCondition, AuthorizationDecision, AuthorizationResponse,
    DiagnosticsAuthorizeBatchResponse, DiagnosticsAuthorizeResponse,
};
use permission_service::state::{AppState, MockEventPublisher};
use permission_service_notification_proto::nvidia::omniverse::notifications::publisher::v1beta::PublishEventRequest;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::ops::{Add, Sub};
use std::sync::Arc;
use std::time::Duration;
use time::OffsetDateTime;
use tokio::runtime::Runtime;
use tokio::time::sleep;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockGuard, MockServer, ResponseTemplate};

static POSTGRES_RUNTIME: Lazy<Arc<Runtime>> = Lazy::new(|| {
    Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .thread_name("postgres-test")
            .worker_threads(4)
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime for tests"),
    )
});

async fn setup() -> TestServer {
    let config = setup_config();
    let cache = setup_cache(&config);
    setup_server(config, cache).await
}

fn setup_metrics(config: &ServiceConfig, auth_cache: Arc<AuthCache>) -> (Metrics, Caches) {
    let meter_provider = SdkMeterProvider::default();
    let meter = meter_provider.meter("test");
    let caches = Caches::new(&config.cache, auth_cache);
    let metrics = Metrics::new(meter_provider, meter, &caches);
    (metrics, caches)
}

async fn setup_server(mut config: ServiceConfig, cache: Arc<AuthCache>) -> TestServer {
    let client = Arc::new(reqwest::Client::new());
    config.auth.load(&client).await.unwrap();

    let (metrics, caches) = setup_metrics(&config, cache);
    let db_pool: Arc<dyn ConnectionPool> = Arc::new(InstrumentedPool::new(
        create_database_pool(&config).await,
        &metrics.meter,
    ));
    let options = PostgresDatabaseOptions { temporary: true };

    let dbname = config.postgres.dbname.as_deref().unwrap();

    let mut policies = PostgresPolicyStorage::new(
        db_pool.clone(),
        POSTGRES_RUNTIME.clone(),
        dbname,
        options.clone(),
    );
    policies.initialize().await.unwrap();
    set_system_policies(&mut policies, &config).await;

    let metadata = PostgresMetadataStorage::new(
        db_pool.clone(),
        POSTGRES_RUNTIME.clone(),
        dbname,
        options.clone(),
        config.auth.principal_id_claim.clone(),
    );
    metadata.initialize().await.unwrap();
    set_system_metadata(&metadata, &config).await;

    let policies: Arc<dyn PolicyStorage> = Arc::new(policies);
    let metadata: Arc<dyn MetadataStorage> = Arc::new(metadata);

    let state = AppState::new(caches, config, policies, metadata, metrics, client).await;
    let router = configure_router(state);
    TestServer::new(router).unwrap()
}

async fn setup_config_file_server(policies_yaml: &str, metadata_yaml: &str) -> TestServer {
    setup_config_file_server_with_principal_id_claim(policies_yaml, metadata_yaml, "sub").await
}

async fn setup_config_file_server_with_principal_id_claim(
    policies_yaml: &str,
    metadata_yaml: &str,
    principal_id_claim: &str,
) -> TestServer {
    let mut config = setup_config();
    config.auth.principal_id_claim = principal_id_claim.to_string();
    let cache = setup_cache(&config);

    let client = Arc::new(reqwest::Client::new());

    let dir = std::env::temp_dir().join(format!("permission-test-{}", std::process::id()));
    tokio::fs::create_dir_all(&dir).await.unwrap();

    let policy_path = dir.join(format!("policies-{}.yaml", rand_suffix()));
    let metadata_path = dir.join(format!("metadata-{}.yaml", rand_suffix()));

    tokio::fs::write(&policy_path, policies_yaml).await.unwrap();
    tokio::fs::write(&metadata_path, metadata_yaml)
        .await
        .unwrap();

    let policies = ConfigFilePolicyStorage::new(policy_path);
    policies.initialize().await.unwrap();

    let metadata =
        ConfigFileMetadataStorage::new(metadata_path, config.auth.principal_id_claim.clone());
    metadata.initialize().await.unwrap();

    let policies: Arc<dyn PolicyStorage> = Arc::new(policies);
    let metadata: Arc<dyn MetadataStorage> = Arc::new(metadata);

    let (metrics, caches) = setup_metrics(&config, cache);
    let state = AppState::new(caches, config, policies, metadata, metrics, client).await;
    let router = configure_router(state);
    TestServer::new(router).unwrap()
}

fn rand_suffix() -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    std::thread::current().id().hash(&mut h);
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
        .hash(&mut h);
    h.finish()
}

fn setup_config() -> ServiceConfig {
    std::env::set_var("ENABLE_AUTH", "false");
    ServiceConfig::parse_from(Vec::<String>::new())
}

fn write_init_policies_file(content: &str) -> String {
    let path = std::env::temp_dir().join(format!(
        "init-policies-{}-{}.yaml",
        std::process::id(),
        rand_suffix()
    ));
    std::fs::write(&path, content).expect("failed to write init policies file");
    path.to_string_lossy().into_owned()
}

fn setup_cache(config: &ServiceConfig) -> Arc<AuthCache> {
    Arc::new(AuthCache::new(
        config.auth.token_cache_size,
        config.auth.token_ttl,
        config.auth.invalid_token_cache_size,
        config.auth.invalid_token_cache_ttl,
    ))
}

async fn setup_metadata(server: &TestServer, priority: EvaluationPriority) {
    let metadata = json!({
        "service": "tags",
        "type": "ResourceAddress",
        "evaluation_priority": priority,
    });
    let response = server
        .put("/v1beta/services/tags/resource-types/ResourceAddress/")
        .json(&metadata)
        .await;
    assert_eq!(response.status_code(), 200);
}

fn new_principal() -> Principal {
    Principal {
        id: "test-user".to_string(),
        info: None,
    }
}

fn new_action() -> Action {
    Action {
        service: "tags".to_string(),
        name: "get".to_string(),
    }
}

fn new_resource() -> Resource {
    Resource {
        id: "Astronaut.usd".to_string(),
        kind: "ResourceAddress".to_string(),
        data: None,
    }
}

fn other_resource() -> Resource {
    Resource {
        id: "Marbles_Assets.usd".to_string(),
        kind: "ResourceAddress".to_string(),
        data: None,
    }
}

fn batch_payload(condition: AuthorizationBatchCondition) -> Value {
    json!({
        "condition": condition,
        "batches": [
            {
                "principal": {"sub": "test-user", "email": "user@test.com", "exp": 0},
                "actions": [{"name": "get", "service": "tags"}],
                "resource": {
                    "id": "Astronaut.usd",
                    "type": "ResourceAddress",
                }
            },
            {
                "principal": {"sub": "test-user", "email": "user@test.com", "exp": 0},
                "actions": [{"name": "get", "service": "tags"}],
                "resource": {
                    "id": "Marbles_Assets.usd",
                    "type": "ResourceAddress"
                }
            }
        ]
    })
}

fn new_policy_payload(
    effect: &str,
    action: Option<&Action>,
    principal: Option<&Principal>,
    resource: Option<&Resource>,
    conditions: &str,
) -> Value {
    let principal_scope = principal.map_or("".to_string(), |p| format!("== {}", p));
    let action_scope = action.map_or("".to_string(), |a| format!("== {}", a));
    let resource_scope = resource.map_or("".to_string(), |r| format!("== {}", r));
    let policy = format!(
        "{}(principal {}, action {}, resource {}) {};",
        effect, principal_scope, action_scope, resource_scope, conditions
    );

    let principal_payload = principal.map(|p| p.id.clone());
    let action_payload = action.map(|a| {
        json!({
            "name": a.name,
            "service": a.service
        })
    });
    let resource_payload = resource.map(|r| {
        json!({
            "id": r.id,
            "type": r.kind,
            "data": r.data
        })
    });
    json!({
        "policy": policy,
        "principal": principal_payload,
        "action": action_payload,
        "resource": resource_payload
    })
}

fn get_default_policy_payload() -> Value {
    new_policy_payload(
        "permit",
        Some(&new_action()),
        Some(&new_principal()),
        Some(&new_resource()),
        "",
    )
}

async fn setup_policy(server: &TestServer) -> TestResponse {
    let payload = get_default_policy_payload();
    let response = server.put("/v1beta/policies/").json(&payload).await;
    assert_eq!(
        response.status_code(),
        201,
        "a brand-new policy must be reported via HTTP 201 Created"
    );

    let result: Value = response.json();
    assert_eq!(result["id"], json!(1));
    assert_eq!(
        result["created"],
        json!(true),
        "the `created` body field must mirror the 201 status for a fresh insert"
    );
    assert_eq!(
        result["policy"],
        json!(
            "permit(principal == Principal::\"test-user\", action == Action::\"tags:get\", resource == ResourceAddress::\"Astronaut.usd\");"
        )
    );
    assert_eq!(
        result["resource"],
        json!({
            "id": "Astronaut.usd",
            "type": "ResourceAddress",
            "data": null
        })
    );
    assert_eq!(
        result["action"],
        json!({
            "name": "get",
            "service": "tags"
        })
    );
    assert_eq!(
        result["principal"],
        json!({
            "sub": "test-user",
            "info": null
        })
    );
    assert_eq!(result["created_by"], json!(""));
    assert!(result["created_at"]
        .as_str()
        .is_some_and(|created_at| !created_at.is_empty()));

    response
}

fn assert_policy_metadata(value: &Value, expected_created_by: &str) {
    assert_eq!(value["created_by"], json!(expected_created_by));
    assert!(value["created_at"]
        .as_str()
        .is_some_and(|created_at| !created_at.is_empty()));
}

fn get_policy_batch_payload() -> Vec<Value> {
    let mut batches = vec![];
    for i in 1..=5 {
        batches.push(json!({
            "policy": format!("permit(principal == Principal::\"test-user-{}\", action == Action::\"tags:get\", resource == ResourceAddress::\"Astronaut.usd\");", i),
            "action": {
                "service": "tags",
                "name": "get"
            },
            "principal": format!("test-user-{}", i),
            "resource": {
                "id": "Astronaut.usd",
                "type": "ResourceAddress"
            }
        }));
    }
    batches
}

async fn verify(
    server: &TestServer,
    principal: &Principal,
    action: &Action,
    resource: Option<&Resource>,
) -> AuthorizationDecision {
    let response = server
        .post("/v1beta/authorization/")
        .json(&json!({
            "principal": principal,
            "action": action,
            "resource": resource,
        }))
        .await;
    assert_eq!(response.status_code(), 200);

    let result = response.json();
    serde_json::from_value::<AuthorizationResponse>(result)
        .unwrap()
        .decision
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Claims {
    sub: String,
    aud: String,
    iss: String,
    exp: i64,
    email: String,
    groups: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    appid: Option<String>,
}

impl Claims {
    pub fn new(exp: i64) -> Self {
        let mut claims = Self::default();
        claims.exp = exp;
        claims
    }

    pub fn default() -> Self {
        Claims {
            sub: "test-user".to_string(),
            aud: "https://localhost:12345".to_string(),
            iss: String::new(),
            exp: OffsetDateTime::now_utc()
                .add(Duration::from_secs(10))
                .unix_timestamp(),
            email: "test-user@test.test".to_string(),
            groups: vec![],
            appid: None,
        }
    }
}

struct JsonWebKeyFixture {
    idp: MockServer,
    jwk: jsonwebkey::JsonWebKey,
}

impl JsonWebKeyFixture {
    pub async fn new_async() -> Self {
        let mut jwk = jsonwebkey::JsonWebKey::new(jsonwebkey::Key::generate_p256());
        jwk.key_id = Some("test-key".to_string());
        jwk.set_algorithm(jsonwebkey::Algorithm::ES256).unwrap();

        let idp = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/.well-known/jwks.json"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/json")
                    .set_body_json(json!({"keys": [jwk.clone()]})),
            )
            .mount(&idp)
            .await;

        Mock::given(method("GET"))
            .and(path("/.well-known/openid-configuration"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/json")
                    .set_body_json(json!({
                        "issuer": idp.uri(),
                        "jwks_uri": format!("{}/.well-known/jwks.json", idp.uri()),
                        "token_endpoint": format!("{}/token", idp.uri()),
                        "userinfo_endpoint":  format!("{}/userinfo", idp.uri())
                    })),
            )
            .mount(&idp)
            .await;

        Self { idp, jwk }
    }

    pub fn config(&self) -> ServiceConfig {
        let mut config = setup_config();
        config.auth.openid_configuration = Some(format!(
            "{}/.well-known/openid-configuration",
            self.idp.uri()
        ));
        config.auth.enabled = true;

        let admin_group = self.admin_group();
        config.database.init.policies_file = write_init_policies_file(&format!(
            r#"
policies:
  - policy: 'permit(principal, action == Action::"permissions:edit", resource) when {{ principal.groups.contains("{admin_group}") }};'
  - policy: 'permit(principal, action == Action::"permissions:view", resource) when {{ principal.groups.contains("{admin_group}") }};'
  - policy: 'permit(principal, action == Action::"permissions:meta:read", resource) when {{ principal.groups.contains("{admin_group}") }};'
  - policy: 'permit(principal, action == Action::"permissions:meta:write", resource) when {{ principal.groups.contains("{admin_group}") }};'
"#
        ));

        config.auth.jwt_audience = Some(vec!["https://localhost:12345".to_string()]);
        // Pin the expected issuer to the mock IdP so the new `jwt_issuers`
        // validation in `permission-auth` is exercised end-to-end.
        config.auth.jwt_issuers = Some(vec![self.idp.uri()]);
        config
    }

    pub fn admin_group(&self) -> String {
        "administrators".to_string()
    }

    pub fn encoding_key(&self) -> jsonwebtoken::EncodingKey {
        jsonwebtoken::EncodingKey::from_ec_pem(self.jwk.key.try_to_pem().unwrap().as_bytes())
            .unwrap()
    }

    pub fn encode(&self, claims: &Claims) -> String {
        let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256);
        header.kid = self.jwk.key_id.clone();

        // Default `iss` to the mock IdP URI so tokens satisfy the issuer
        // validation enforced by `permission-auth`. Tests that need to simulate
        // a mismatched issuer can set `claims.iss` explicitly before encoding.
        let mut claims = Claims {
            sub: claims.sub.clone(),
            aud: claims.aud.clone(),
            iss: claims.iss.clone(),
            exp: claims.exp,
            email: claims.email.clone(),
            groups: claims.groups.clone(),
            appid: claims.appid.clone(),
        };
        if claims.iss.is_empty() {
            claims.iss = self.idp.uri();
        }

        jsonwebtoken::encode(&header, &claims, &self.encoding_key()).unwrap()
    }

    pub async fn userinfo_mock(&self, status: u16, body: Value, expect: u64) -> MockGuard {
        Mock::given(method("GET"))
            .and(path("/userinfo"))
            .respond_with(
                ResponseTemplate::new(status)
                    .insert_header("content-type", "application/json")
                    .set_body_json(body),
            )
            .expect(expect)
            .mount_as_scoped(&self.idp)
            .await
    }

    /// Mounts a `/userinfo` mock without an upper bound on the expected
    /// number of calls. Useful for tests that flood the endpoint with
    /// distinct bogus tokens, where pinning an exact count would conflict
    /// with the loop iteration count or with the negative-cache short-
    /// circuit.
    pub async fn userinfo_mock_unbounded(&self, status: u16, body: Value) -> MockGuard {
        Mock::given(method("GET"))
            .and(path("/userinfo"))
            .respond_with(
                ResponseTemplate::new(status)
                    .insert_header("content-type", "application/json")
                    .set_body_json(body),
            )
            .mount_as_scoped(&self.idp)
            .await
    }
}

#[tokio::test]
async fn test_put_policy() {
    let server = setup().await;
    setup_policy(&server).await;
}

#[tokio::test]
async fn test_put_policy_invalid() {
    let server = setup().await;
    let response = server.put("/v1beta/policies/").json(&json!({
        "policy": "permit(principal == Principal::\"test-user\", resource == ResourceAddress::\"Astronaut.usd\");",
        "action": {
            "service": "tags",
            "name": "get"
        },
        "principal": "test-user",
        "resource": {
            "id": "Astronaut.usd",
            "type": "ResourceAddress"
        }
    })).await;
    assert_eq!(response.status_code(), 400);
    assert_eq!(response.text(), "Failed to parse the Cedar policy: found the variable `resource` where the variable `action` must be used");
}

#[tokio::test]
async fn test_put_policy_required() {
    let server = setup().await;
    let response = server
        .put("/v1beta/policies/")
        .json(&json!({
            "action": {
                "service": "tags",
                "name": "get"
            },
            "principal": "test-user",
            "resource": {
                "id": "Astronaut.usd",
                "type": "ResourceAddress"
            }
        }))
        .await;
    assert_eq!(response.status_code(), 422);
    assert_eq!(response.text(), "Failed to deserialize the JSON body into the target type: missing field `policy` at line 1 column 125");
}

#[tokio::test]
async fn test_put_policy_authorization() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let mut claims = Claims::default();
    let token = jwk.encode(&claims);
    let config = jwk.config();
    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;
    let body = get_default_policy_payload();
    let response = server.put("/v1beta/policies/").json(&body).await;
    assert_eq!(response.status_code(), 401);

    let response = server
        .put("/v1beta/policies/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .await;
    assert_eq!(response.status_code(), 403);

    claims.groups = vec![jwk.admin_group()];
    let token = jwk.encode(&claims);
    let response = server
        .put("/v1beta/policies/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .await;
    assert_eq!(response.status_code(), 201);
    assert_eq!(response.json::<Value>()["created"], json!(true));
}

async fn setup_scoped_edit_server(jwk: &JsonWebKeyFixture, init_policies: &str) -> TestServer {
    let mut config = jwk.config();
    config.database.init.policies_file = write_init_policies_file(init_policies);
    let cache = setup_cache(&config);
    setup_server(config, cache).await
}

fn storage_action() -> Action {
    Action {
        service: "storage-service".to_string(),
        name: "write".to_string(),
    }
}

const SCOPED_EDIT_POLICIES: &str = r#"
policies:
  - policy: 'permit(principal, action == Action::"permissions:edit", resource) when { principal.groups.contains("administrators") && context.action.service == "storage-service" };'
"#;

#[tokio::test]
async fn test_put_policy_resource_scoped_allow() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let server = setup_scoped_edit_server(&jwk, SCOPED_EDIT_POLICIES).await;

    let mut claims = Claims::default();
    claims.groups = vec![jwk.admin_group()];
    let token = jwk.encode(&claims);

    let body = new_policy_payload("permit", Some(&storage_action()), None, None, "");
    let response = server
        .put("/v1beta/policies/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .await;
    assert_eq!(response.status_code(), 201);
}

#[tokio::test]
async fn test_put_policy_resource_scoped_denied() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let server = setup_scoped_edit_server(&jwk, SCOPED_EDIT_POLICIES).await;

    let mut claims = Claims::default();
    claims.groups = vec![jwk.admin_group()];
    let token = jwk.encode(&claims);

    // `tags` is outside the `storage-service` scope the editor is limited to.
    let body = new_policy_payload("permit", Some(&new_action()), None, None, "");
    let response = server
        .put("/v1beta/policies/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .await;
    assert_eq!(response.status_code(), 403);
}

#[tokio::test]
async fn test_put_policy_batch_resource_scoped_denied() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let server = setup_scoped_edit_server(&jwk, SCOPED_EDIT_POLICIES).await;

    let mut claims = Claims::default();
    claims.groups = vec![jwk.admin_group()];
    let token = jwk.encode(&claims);

    let batches = json!([
        new_policy_payload("permit", Some(&storage_action()), None, None, ""),
        new_policy_payload("permit", Some(&new_action()), None, None, ""),
    ]);
    let response = server
        .put("/v1beta/policies/batch/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&batches)
        .await;
    assert_eq!(response.status_code(), 403);
}

#[tokio::test]
async fn test_delete_policy_resource_scoped() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let init_policies = r#"
policies:
  - policy: 'permit(principal, action == Action::"permissions:edit", resource) when { principal.groups.contains("superadmins") };'
  - policy: 'permit(principal, action == Action::"permissions:edit", resource) when { principal.groups.contains("administrators") && context.action.service == "storage-service" };'
"#;
    let server = setup_scoped_edit_server(&jwk, init_policies).await;

    let mut superadmin = Claims::default();
    superadmin.groups = vec!["superadmins".to_string()];
    let superadmin_token = jwk.encode(&superadmin);

    let storage_response = server
        .put("/v1beta/policies/")
        .add_header("Authorization", format!("Bearer {superadmin_token}"))
        .json(&new_policy_payload(
            "permit",
            Some(&storage_action()),
            None,
            None,
            "",
        ))
        .await;
    assert_eq!(storage_response.status_code(), 201);
    let storage_id = storage_response.json::<Value>()["id"].as_i64().unwrap();

    let tags_response = server
        .put("/v1beta/policies/")
        .add_header("Authorization", format!("Bearer {superadmin_token}"))
        .json(&new_policy_payload(
            "permit",
            Some(&new_action()),
            None,
            None,
            "",
        ))
        .await;
    assert_eq!(tags_response.status_code(), 201);
    let tags_id = tags_response.json::<Value>()["id"].as_i64().unwrap();

    let mut admin = Claims::default();
    admin.groups = vec![jwk.admin_group()];
    let admin_token = jwk.encode(&admin);

    let response = server
        .delete(&format!("/v1beta/policies/{tags_id}"))
        .add_header("Authorization", format!("Bearer {admin_token}"))
        .await;
    assert_eq!(response.status_code(), 403);

    let response = server
        .delete(&format!("/v1beta/policies/{storage_id}"))
        .add_header("Authorization", format!("Bearer {admin_token}"))
        .await;
    assert_eq!(response.status_code(), 204);
}

const HIERARCHY_EDIT_POLICIES: &str = r#"
policies:
  - policy: 'permit(principal, action == Action::"permissions:edit", resource) when { principal.groups.contains("superadmins") };'
  - policy: 'permit(principal, action == Action::"permissions:meta:write", resource) when { principal.groups.contains("superadmins") };'
  - policy: 'permit(principal, action == Action::"permissions:edit", resource in Folder::"https://main/docs/") when { principal.groups.contains("docs-editors") };'
"#;

/// A `permissions:edit` grant scoped to a folder hierarchy lets an editor
/// create and delete policies whose real resource is a descendant of that
/// folder, and rejects siblings. The resource's `Folder` parent type is
/// resolved from the globally-unique resource kind, so the meta check evaluates
/// against the real resource without a wrapper.
#[tokio::test]
async fn test_edit_permissions_hierarchy_scoped() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let server = setup_scoped_edit_server(&jwk, HIERARCHY_EDIT_POLICIES).await;

    let mut superadmin = Claims::default();
    superadmin.groups = vec!["superadmins".to_string()];
    let superadmin_token = jwk.encode(&superadmin);

    let mut editor = Claims::default();
    editor.groups = vec!["docs-editors".to_string()];
    let editor_token = jwk.encode(&editor);

    // Register the `object` kind with a `Folder` hierarchy parent.
    let response = server
        .put("/v1beta/services/storage/resource-types/object/")
        .add_header("Authorization", format!("Bearer {superadmin_token}"))
        .json(&json!({ "service": "storage", "type": "object", "evaluation_priority": "forbid", "parentType": "Folder" }))
        .await;
    assert_eq!(response.status_code(), 200);

    // Superadmin seeds one descendant and one sibling policy.
    let descendant_response = server
        .put("/v1beta/policies/")
        .add_header("Authorization", format!("Bearer {superadmin_token}"))
        .json(&json!({
            "policy": "permit(principal == Principal::\"bob\", action == Action::\"storage:read\", resource == object::\"https://main/docs/reports/q1.pdf\");"
        }))
        .await;
    assert_eq!(descendant_response.status_code(), 201);
    let descendant_id = descendant_response.json::<Value>()["id"].as_i64().unwrap();

    let sibling_response = server
        .put("/v1beta/policies/")
        .add_header("Authorization", format!("Bearer {superadmin_token}"))
        .json(&json!({
            "policy": "permit(principal == Principal::\"bob\", action == Action::\"storage:read\", resource == object::\"https://main/other/q1.pdf\");"
        }))
        .await;
    assert_eq!(sibling_response.status_code(), 201);
    let sibling_id = sibling_response.json::<Value>()["id"].as_i64().unwrap();

    // Editor may create a policy whose resource is inside the docs folder.
    let allowed = server
        .put("/v1beta/policies/")
        .add_header("Authorization", format!("Bearer {editor_token}"))
        .json(&json!({
            "policy": "permit(principal == Principal::\"carol\", action == Action::\"storage:read\", resource == object::\"https://main/docs/reports/q2.pdf\");"
        }))
        .await;
    assert_eq!(allowed.status_code(), 201);

    // Editor may not create a policy for a sibling resource.
    let denied = server
        .put("/v1beta/policies/")
        .add_header("Authorization", format!("Bearer {editor_token}"))
        .json(&json!({
            "policy": "permit(principal == Principal::\"carol\", action == Action::\"storage:read\", resource == object::\"https://main/other/q2.pdf\");"
        }))
        .await;
    assert_eq!(denied.status_code(), 403);

    // Editor may delete a descendant policy but not a sibling one.
    let delete_sibling = server
        .delete(&format!("/v1beta/policies/{sibling_id}"))
        .add_header("Authorization", format!("Bearer {editor_token}"))
        .await;
    assert_eq!(delete_sibling.status_code(), 403);

    let delete_descendant = server
        .delete(&format!("/v1beta/policies/{descendant_id}"))
        .add_header("Authorization", format!("Bearer {editor_token}"))
        .await;
    assert_eq!(delete_descendant.status_code(), 204);
}

#[tokio::test]
async fn test_put_policy_batch() {
    let server = setup().await;

    let batches = get_policy_batch_payload();
    let response = server
        .put("/v1beta/policies/batch/")
        .json(&json!(batches))
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    match result {
        Value::Object(response) => {
            let values = response.get("results").unwrap().as_array().unwrap();
            assert_eq!(values.len(), batches.len());

            for (i, value) in values.iter().enumerate() {
                assert_eq!(value["id"], json!(i + 1));
                assert_eq!(
                    value["policy"],
                    json!(format!(
                        "permit(principal == Principal::\"test-user-{}\", action == Action::\"tags:get\", resource == ResourceAddress::\"Astronaut.usd\");",
                        i + 1
                    ))
                );
                assert_eq!(
                    value["resource"],
                    json!({
                        "id": "Astronaut.usd",
                        "type": "ResourceAddress",
                        "data": null
                    })
                );
                assert_eq!(
                    value["action"],
                    json!({
                        "service": "tags",
                        "name": "get"
                    })
                );
                assert_eq!(
                    value["principal"],
                    json!({
                        "sub": format!("test-user-{}", i+1),
                        "info": null
                    })
                );
                assert_eq!(value["created_by"], json!(""));
                assert!(value["created_at"]
                    .as_str()
                    .is_some_and(|created_at| !created_at.is_empty()));
                assert_eq!(
                    value["created"],
                    json!(true),
                    "every item in a fresh batch must report created=true"
                );
            }
        }
        _ => panic!("Expected an array, got {:?}", result),
    }
}

#[tokio::test]
async fn test_put_policy_batch_authorization() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let mut claims = Claims::default();
    let token = jwk.encode(&claims);
    let config = jwk.config();
    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;

    let batches = get_policy_batch_payload();
    let response = server
        .put("/v1beta/policies/batch/")
        .json(&json!(batches))
        .await;
    assert_eq!(response.status_code(), 401);

    let response = server
        .put("/v1beta/policies/batch/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&batches)
        .await;
    assert_eq!(response.status_code(), 403);

    claims.groups = vec![jwk.admin_group()];
    let token = jwk.encode(&claims);
    let response = server
        .put("/v1beta/policies/batch/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&batches)
        .await;
    assert_eq!(response.status_code(), 200);
    let result: Value = response.json();
    let items = result["results"].as_array().unwrap();
    assert_eq!(items.len(), batches.len());
    for item in items {
        assert_eq!(item["created_by"], json!(claims.sub));
        assert!(item["created_at"]
            .as_str()
            .is_some_and(|created_at| !created_at.is_empty()));
        assert_eq!(item["created"], json!(true));
    }
}

#[tokio::test]
async fn test_put_policy_with_comment() {
    let server = setup().await;

    let expected_comment = "owned by platform team; reviewed 2026-Q1";
    let mut payload = get_default_policy_payload();
    payload["comment"] = json!(expected_comment);

    let response = server.put("/v1beta/policies/").json(&payload).await;
    assert_eq!(response.status_code(), 201);
    let body: Value = response.json();
    assert_eq!(body["comment"], json!(expected_comment));
    assert_eq!(body["created"], json!(true));

    let id = body["id"].as_i64().unwrap();
    let fetched = server.get(&format!("/v1beta/policies/{id}")).await;
    assert_eq!(fetched.status_code(), 200);
    let fetched_body: Value = fetched.json();
    assert_eq!(fetched_body["comment"], json!(expected_comment));
    assert!(
        fetched_body.get("created").is_none(),
        "GET responses must not include the upsert-only `created` field"
    );
}

#[tokio::test]
async fn test_put_policy_without_comment_is_null() {
    let server = setup().await;

    let response = server
        .put("/v1beta/policies/")
        .json(&get_default_policy_payload())
        .await;
    assert_eq!(response.status_code(), 201);
    let body: Value = response.json();
    assert!(
        body.get("comment").is_some(),
        "PolicyResponse must always serialize the comment field"
    );
    assert_eq!(body["comment"], Value::Null);
}

#[tokio::test]
async fn test_put_policy_blank_comment_is_normalized_to_null() {
    let server = setup().await;

    let mut payload = get_default_policy_payload();
    payload["comment"] = json!("   ");

    let response = server.put("/v1beta/policies/").json(&payload).await;
    assert_eq!(response.status_code(), 201);
    let body: Value = response.json();
    assert_eq!(body["comment"], Value::Null);
}

#[tokio::test]
async fn test_put_policy_batch_with_comments() {
    let server = setup().await;

    let mut batches = get_policy_batch_payload();
    batches[0]["comment"] = json!("first policy note");
    batches[2]["comment"] = json!("third policy note");

    let response = server
        .put("/v1beta/policies/batch/")
        .json(&json!(batches))
        .await;
    assert_eq!(response.status_code(), 200);

    let body: Value = response.json();
    let items = body["results"].as_array().unwrap();
    assert_eq!(items.len(), batches.len());
    assert_eq!(items[0]["comment"], json!("first policy note"));
    assert_eq!(items[1]["comment"], Value::Null);
    assert_eq!(items[2]["comment"], json!("third policy note"));
    assert_eq!(items[3]["comment"], Value::Null);
    assert_eq!(items[4]["comment"], Value::Null);
    for item in items {
        assert_eq!(item["created"], json!(true));
    }
}

#[tokio::test]
async fn test_put_policy_duplicate_returns_existing_id() {
    let server = setup().await;

    let payload = get_default_policy_payload();
    let first = server.put("/v1beta/policies/").json(&payload).await;
    assert_eq!(
        first.status_code(),
        201,
        "first PUT of a new policy must return 201 Created"
    );
    let first_body: Value = first.json();
    assert_eq!(first_body["created"], json!(true));
    let first_id = first_body["id"].as_i64().unwrap();
    let first_order = first_body["order"].as_i64().unwrap();

    let duplicate = server.put("/v1beta/policies/").json(&payload).await;
    assert_eq!(
        duplicate.status_code(),
        200,
        "duplicate PUT must return 200 OK because no new row was inserted"
    );
    let duplicate_body: Value = duplicate.json();
    assert_eq!(duplicate_body["created"], json!(false));
    assert_eq!(duplicate_body["id"].as_i64().unwrap(), first_id);
    assert_eq!(duplicate_body["policy"], first_body["policy"]);
    assert_eq!(duplicate_body["order"].as_i64().unwrap(), first_order);

    let all = server.get("/v1beta/policies/?page=1&limit=50").await;
    let all_body: Value = all.json();
    assert_eq!(all_body["items"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn test_put_policy_duplicate_with_different_order_updates_existing() {
    let server = setup().await;

    let principal = new_principal();
    let action = new_action();
    let resource = new_resource();
    let policy_text = format!(
        "permit(principal == {}, action == {}, resource == {});",
        principal, action, resource
    );

    let first_payload = json!({
        "policy": policy_text,
        "principal": principal.id,
        "action": { "name": action.name, "service": action.service },
        "resource": { "id": resource.id, "type": resource.kind, "data": resource.data },
        "order": 5,
    });
    let first = server.put("/v1beta/policies/").json(&first_payload).await;
    assert_eq!(first.status_code(), 201);
    let first_body: Value = first.json();
    assert_eq!(first_body["created"], json!(true));
    let first_id = first_body["id"].as_i64().unwrap();
    assert_eq!(first_body["order"].as_i64().unwrap(), 5);

    // Same canonical policy, different order. PUT semantics overwrite the
    // existing row's mutable fields (`order`, `comment`) in place; the row's
    // identity (`id`) is preserved.
    let second_payload = json!({
        "policy": policy_text,
        "principal": principal.id,
        "action": { "name": action.name, "service": action.service },
        "resource": { "id": resource.id, "type": resource.kind, "data": resource.data },
        "order": 99,
    });
    let second = server.put("/v1beta/policies/").json(&second_payload).await;
    assert_eq!(second.status_code(), 200);
    let second_body: Value = second.json();
    assert_eq!(second_body["created"], json!(false));
    assert_eq!(second_body["id"].as_i64().unwrap(), first_id);
    assert_eq!(second_body["order"].as_i64().unwrap(), 99);

    let all = server.get("/v1beta/policies/?page=1&limit=50").await;
    let all_body: Value = all.json();
    let items = all_body["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"].as_i64().unwrap(), first_id);
    assert_eq!(items[0]["order"].as_i64().unwrap(), 99);
}

#[tokio::test]
async fn test_put_policy_whitespace_variant_returns_existing_id() {
    let server = setup().await;

    let principal = new_principal();
    let action = new_action();
    let resource = new_resource();

    let first_payload = json!({
        "policy": format!(
            "permit(principal == {}, action == {}, resource == {});",
            principal, action, resource
        ),
        "principal": principal.id,
        "action": { "name": action.name, "service": action.service },
        "resource": { "id": resource.id, "type": resource.kind, "data": resource.data },
        "order": 7,
    });
    let first_response = server.put("/v1beta/policies/").json(&first_payload).await;
    assert_eq!(first_response.status_code(), 201);
    let first_body: Value = first_response.json();
    assert_eq!(first_body["created"], json!(true));
    let first_id = first_body["id"].as_i64().unwrap();
    assert_eq!(first_body["order"].as_i64().unwrap(), 7);

    let whitespace_variant = json!({
        "policy": format!(
            "permit(\n\tprincipal   ==\t{},\n    action == {},\n    resource   ==  {}\n);\n",
            principal, action, resource
        ),
        "principal": principal.id,
        "action": { "name": action.name, "service": action.service },
        "resource": { "id": resource.id, "type": resource.kind, "data": resource.data },
        "order": 42,
    });

    let duplicate = server
        .put("/v1beta/policies/")
        .json(&whitespace_variant)
        .await;
    assert_eq!(duplicate.status_code(), 200);
    let duplicate_body: Value = duplicate.json();
    assert_eq!(duplicate_body["created"], json!(false));
    assert_eq!(duplicate_body["id"].as_i64().unwrap(), first_id);
    // The whitespace variant canonicalizes to the same policy hash; the
    // existing row is updated in place with the new `order`.
    assert_eq!(duplicate_body["order"].as_i64().unwrap(), 42);

    let all = server.get("/v1beta/policies/?page=1&limit=50").await;
    let all_body: Value = all.json();
    let items = all_body["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["order"].as_i64().unwrap(), 42);
}

#[tokio::test]
async fn test_put_policy_comment_variant_returns_existing_id() {
    let server = setup().await;

    let principal = new_principal();
    let action = new_action();
    let resource = new_resource();

    let first_payload = json!({
        "policy": format!(
            "permit(principal == {}, action == {}, resource == {});",
            principal, action, resource
        ),
        "principal": principal.id,
        "action": { "name": action.name, "service": action.service },
        "resource": { "id": resource.id, "type": resource.kind, "data": resource.data },
        "order": 3,
    });
    let first_response = server.put("/v1beta/policies/").json(&first_payload).await;
    assert_eq!(first_response.status_code(), 201);
    let first_body: Value = first_response.json();
    assert_eq!(first_body["created"], json!(true));
    let first_id = first_body["id"].as_i64().unwrap();
    assert_eq!(first_body["order"].as_i64().unwrap(), 3);

    let comment_variant = json!({
        "policy": format!(
            "// top-level comment\npermit(principal == {}, action == {}, resource == {}); // trailing",
            principal, action, resource
        ),
        "principal": principal.id,
        "action": { "name": action.name, "service": action.service },
        "resource": { "id": resource.id, "type": resource.kind, "data": resource.data },
        "order": 11,
    });

    let duplicate = server.put("/v1beta/policies/").json(&comment_variant).await;
    assert_eq!(duplicate.status_code(), 200);
    let duplicate_body: Value = duplicate.json();
    assert_eq!(duplicate_body["created"], json!(false));
    assert_eq!(duplicate_body["id"].as_i64().unwrap(), first_id);
    // Cedar comments are stripped during canonicalization, so this re-PUT
    // matches the existing row by hash and overwrites its `order`.
    assert_eq!(duplicate_body["order"].as_i64().unwrap(), 11);

    let all = server.get("/v1beta/policies/?page=1&limit=50").await;
    let all_body: Value = all.json();
    let items = all_body["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["order"].as_i64().unwrap(), 11);
}

#[tokio::test]
async fn test_put_policy_batch_duplicate_returns_existing_id() {
    let server = setup().await;

    let batches = get_policy_batch_payload();
    let first = server
        .put("/v1beta/policies/batch/")
        .json(&json!(batches))
        .await;
    assert_eq!(first.status_code(), 200);

    let first_body: Value = first.json();
    let first_items = first_body["results"].as_array().unwrap().clone();
    assert_eq!(first_items.len(), batches.len());
    for item in &first_items {
        assert_eq!(
            item["created"],
            json!(true),
            "every item in the first batch must report created=true"
        );
    }
    let first_ids: Vec<i64> = first_items
        .iter()
        .map(|item| item["id"].as_i64().unwrap())
        .collect();
    let first_orders: Vec<i64> = first_items
        .iter()
        .map(|item| item["order"].as_i64().unwrap())
        .collect();

    let duplicate = server
        .put("/v1beta/policies/batch/")
        .json(&json!(batches))
        .await;
    assert_eq!(duplicate.status_code(), 200);

    let duplicate_body: Value = duplicate.json();
    let duplicate_items = duplicate_body["results"].as_array().unwrap();
    assert_eq!(duplicate_items.len(), batches.len());
    for (i, item) in duplicate_items.iter().enumerate() {
        assert_eq!(item["id"].as_i64().unwrap(), first_ids[i]);
        assert_eq!(item["policy"], first_items[i]["policy"]);
        assert_eq!(item["order"].as_i64().unwrap(), first_orders[i]);
        assert_eq!(
            item["created"],
            json!(false),
            "every item in the duplicate batch must report created=false"
        );
    }

    let all = server.get("/v1beta/policies/?page=1&limit=50").await;
    let all_body: Value = all.json();
    assert_eq!(all_body["items"].as_array().unwrap().len(), batches.len());
}

#[tokio::test]
async fn test_put_policy_batch_duplicate_with_different_order_updates_existing() {
    let server = setup().await;

    let batches_with_order: Vec<Value> = get_policy_batch_payload()
        .into_iter()
        .enumerate()
        .map(|(i, mut payload)| {
            payload["order"] = json!(i as i32);
            payload
        })
        .collect();

    let first = server
        .put("/v1beta/policies/batch/")
        .json(&json!(batches_with_order))
        .await;
    assert_eq!(first.status_code(), 200);
    let first_body: Value = first.json();
    let first_items = first_body["results"].as_array().unwrap().clone();
    assert_eq!(first_items.len(), batches_with_order.len());
    for (i, item) in first_items.iter().enumerate() {
        assert_eq!(item["order"].as_i64().unwrap(), i as i64);
        assert_eq!(item["created"], json!(true));
    }
    let first_ids: Vec<i64> = first_items
        .iter()
        .map(|item| item["id"].as_i64().unwrap())
        .collect();

    // Re-submit the same policies with a uniform `order` of 999. The stored
    // rows must keep their identity (`id`) but adopt the new `order` from the
    // payload.
    let batches_shuffled_order: Vec<Value> = batches_with_order
        .iter()
        .cloned()
        .map(|mut payload| {
            payload["order"] = json!(999);
            payload
        })
        .collect();

    let duplicate = server
        .put("/v1beta/policies/batch/")
        .json(&json!(batches_shuffled_order))
        .await;
    assert_eq!(duplicate.status_code(), 200);
    let duplicate_body: Value = duplicate.json();
    let duplicate_items = duplicate_body["results"].as_array().unwrap();
    assert_eq!(duplicate_items.len(), batches_with_order.len());
    for (i, item) in duplicate_items.iter().enumerate() {
        assert_eq!(item["id"].as_i64().unwrap(), first_ids[i]);
        assert_eq!(item["order"].as_i64().unwrap(), 999);
        assert_eq!(item["created"], json!(false));
    }
}

/// Mixed batch: half the rows are already in the database from a previous
/// PUT, half are brand new. The response must report `created` per item in
/// the original request order so clients can tell which inputs ended up
/// inserting a fresh row and which collapsed onto an existing one.
#[tokio::test]
async fn test_put_policy_batch_mixed_created_per_item() {
    let server = setup().await;

    let all_batches = get_policy_batch_payload();
    assert!(
        all_batches.len() >= 4,
        "fixture must have enough items for a meaningful mixed batch"
    );
    let preexisting = vec![all_batches[0].clone(), all_batches[2].clone()];

    let seed = server
        .put("/v1beta/policies/batch/")
        .json(&json!(preexisting))
        .await;
    assert_eq!(seed.status_code(), 200);
    let seed_body: Value = seed.json();
    for item in seed_body["results"].as_array().unwrap() {
        assert_eq!(item["created"], json!(true));
    }

    let mixed = server
        .put("/v1beta/policies/batch/")
        .json(&json!(all_batches))
        .await;
    assert_eq!(mixed.status_code(), 200);
    let mixed_body: Value = mixed.json();
    let items = mixed_body["results"].as_array().unwrap();
    assert_eq!(items.len(), all_batches.len());

    let expected_created = [false, true, false, true, true];
    for (i, item) in items.iter().enumerate() {
        assert_eq!(
            item["created"],
            json!(expected_created[i]),
            "item {i} reported wrong `created` flag",
        );
    }
}

/// End-to-end mixed batch: seed two policies with explicit `order` values,
/// then re-PUT a batch where those two appear again (with new orders, so they
/// must be updated in place) alongside three brand-new policies (which must
/// be inserted). Verifies that:
///   * the response status is 200,
///   * `created` is reported per item in the request order,
///   * pre-existing items keep their original `id` (the row was updated, not
///     replaced),
///   * the new `order` values from the payload take effect on the updated
///     rows (proving the update actually fired and was not a silent no-op),
///   * the total number of policies in the database equals the size of the
///     mixed batch (no duplicate rows were created).
#[tokio::test]
async fn test_put_policy_batch_mixed_updates_and_inserts() {
    let server = setup().await;

    let mut payloads = get_policy_batch_payload();
    assert_eq!(
        payloads.len(),
        5,
        "fixture must produce five distinct policy payloads"
    );
    for (i, payload) in payloads.iter_mut().enumerate() {
        payload["order"] = json!(i as i32);
    }

    let preexisting = vec![payloads[1].clone(), payloads[3].clone()];
    let seed = server
        .put("/v1beta/policies/batch/")
        .json(&json!(preexisting))
        .await;
    assert_eq!(seed.status_code(), 200);
    let seed_body: Value = seed.json();
    let seed_items = seed_body["results"].as_array().unwrap();
    assert_eq!(seed_items.len(), 2);
    let preexisting_ids: Vec<i64> = seed_items
        .iter()
        .map(|item| {
            assert_eq!(
                item["created"],
                json!(true),
                "seed-batch items must report created=true"
            );
            item["id"].as_i64().unwrap()
        })
        .collect();

    for (i, payload) in payloads.iter_mut().enumerate() {
        payload["order"] = json!(100 + i as i32);
    }

    let mixed = server
        .put("/v1beta/policies/batch/")
        .json(&json!(payloads))
        .await;
    assert_eq!(mixed.status_code(), 200);
    let mixed_body: Value = mixed.json();
    let items = mixed_body["results"].as_array().unwrap();
    assert_eq!(items.len(), payloads.len());

    let expected_created = [true, false, true, false, true];
    for (i, item) in items.iter().enumerate() {
        assert_eq!(
            item["created"],
            json!(expected_created[i]),
            "item {i} reported wrong `created` flag",
        );
        assert_eq!(
            item["order"].as_i64().unwrap(),
            100 + i as i64,
            "item {i} did not adopt the updated `order` from the payload",
        );
    }

    assert_eq!(
        items[1]["id"].as_i64().unwrap(),
        preexisting_ids[0],
        "updated row must keep its original id (index 1)",
    );
    assert_eq!(
        items[3]["id"].as_i64().unwrap(),
        preexisting_ids[1],
        "updated row must keep its original id (index 3)",
    );

    let all = server.get("/v1beta/policies/?page=1&limit=50").await;
    let all_body: Value = all.json();
    let all_items = all_body["items"].as_array().unwrap();
    assert_eq!(
        all_items.len(),
        payloads.len(),
        "mixed batch must not produce duplicate rows in storage"
    );
}

#[tokio::test]
async fn test_get_policies() {
    let server = setup().await;

    let batches = get_policy_batch_payload();
    server
        .put("/v1beta/policies/batch/")
        .json(&json!(batches))
        .await;

    let response = server.get("/v1beta/policies/?page=1&limit=1").await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    match result {
        Value::Object(response) => {
            let page = response.get("page").unwrap().as_u64().unwrap();
            assert_eq!(page, 1);

            let values = response.get("items").unwrap().as_array().unwrap();
            assert_eq!(values.len(), 1);
        }
        _ => panic!("Expected an array, got {:?}", result),
    }
}

#[tokio::test]
async fn test_get_policies_filtered_by_order() {
    let server = setup().await;

    for (i, order) in [10, 20, 20].into_iter().enumerate() {
        let payload = json!({
            "policy": format!("permit(principal == Principal::\"user-{}\", action == Action::\"tags:get\", resource == ResourceAddress::\"Astronaut.usd\");", i),
            "principal": format!("user-{}", i),
            "action": { "name": "get", "service": "tags" },
            "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" },
            "order": order,
        });
        let response = server.put("/v1beta/policies/").json(&payload).await;
        assert_eq!(response.status_code(), 201);
    }

    let response = server
        .get("/v1beta/policies/?order=20&page=1&limit=50")
        .await;
    assert_eq!(response.status_code(), 200);
    let result: Value = response.json();
    let items = result["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert!(items
        .iter()
        .all(|item| item["order"].as_i64().unwrap() == 20));

    let response = server
        .get("/v1beta/policies/?order=10&page=1&limit=50")
        .await;
    assert_eq!(response.status_code(), 200);
    let result: Value = response.json();
    let items = result["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["order"].as_i64().unwrap(), 10);

    let response = server
        .get("/v1beta/policies/?order=999&page=1&limit=50")
        .await;
    assert_eq!(response.status_code(), 200);
    let result: Value = response.json();
    assert_eq!(result["items"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn test_get_policies_paginated_rejects_limit_above_max() {
    let server = setup().await;
    let response = server.get("/v1beta/policies/?page=1&limit=999").await;
    assert_eq!(response.status_code(), 422);
    assert!(
        response.text().to_lowercase().contains("limit"),
        "expected body to mention `limit`, got: {}",
        response.text()
    );
}

#[tokio::test]
async fn test_get_policies_paginated_rejects_limit_zero() {
    let server = setup().await;
    let response = server.get("/v1beta/policies/?page=1&limit=0").await;
    assert_eq!(response.status_code(), 422);
    assert!(
        response.text().to_lowercase().contains("limit"),
        "expected body to mention `limit`, got: {}",
        response.text()
    );
}

#[tokio::test]
async fn test_get_policies_paginated_rejects_page_zero() {
    let server = setup().await;
    let response = server.get("/v1beta/policies/?page=0&limit=10").await;
    assert_eq!(response.status_code(), 422);
    assert!(
        response.text().to_lowercase().contains("page"),
        "expected body to mention `page`, got: {}",
        response.text()
    );
}

#[tokio::test]
async fn test_get_policies_paginated_accepts_limit_at_max() {
    let server = setup().await;
    let response = server.get("/v1beta/policies/?page=1&limit=50").await;
    assert_eq!(response.status_code(), 200);
}

#[tokio::test]
async fn test_get_policies_authorization() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let mut claims = Claims::default();
    let token = jwk.encode(&claims);
    let config = jwk.config();
    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;

    let response = server.get("/v1beta/policies/").await;
    assert_eq!(response.status_code(), 401);

    let response = server
        .get("/v1beta/policies/")
        .add_header("Authorization", format!("Bearer {token}"))
        .await;
    assert_eq!(response.status_code(), 403);

    claims.groups = vec![jwk.admin_group()];
    let token = jwk.encode(&claims);
    let response = server
        .get("/v1beta/policies/")
        .add_header("Authorization", format!("Bearer {token}"))
        .await;
    assert_eq!(response.status_code(), 200);
}

#[tokio::test]
async fn test_get_policy() {
    let server = setup().await;
    setup_policy(&server).await;

    let response = server.get("/v1beta/policies/1").await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    assert_eq!(result["id"], json!(1));
    assert_eq!(
        result["policy"],
        json!(
            "permit(principal == Principal::\"test-user\", action == Action::\"tags:get\", resource == ResourceAddress::\"Astronaut.usd\");"
        )
    );
    assert_eq!(
        result["resource"],
        json!({
            "id": "Astronaut.usd",
            "type": "ResourceAddress",
            "data": null
        })
    );
    assert_eq!(
        result["action"],
        json!({
            "name": "get",
            "service": "tags"
        })
    );
    assert_eq!(
        result["principal"],
        json!({
            "sub": "test-user",
            "info": null
        })
    );
    assert_eq!(result["created_by"], json!(""));
    assert!(result["created_at"]
        .as_str()
        .is_some_and(|created_at| !created_at.is_empty()));
}

#[tokio::test]
async fn test_get_policy_authorization() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let mut claims = Claims::default();
    let config = jwk.config();
    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;

    claims.groups = vec![jwk.admin_group()];
    let token = jwk.encode(&claims);
    let body = get_default_policy_payload();
    let response = server
        .put("/v1beta/policies/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .await;
    assert_eq!(response.status_code(), 201);

    claims.groups = vec![];
    let token = jwk.encode(&claims);

    let response = server.get("/v1beta/policies/1").await;
    assert_eq!(response.status_code(), 401);

    let response = server
        .get("/v1beta/policies/1")
        .add_header("Authorization", format!("Bearer {token}"))
        .await;
    assert_eq!(response.status_code(), 403);

    claims.groups = vec![jwk.admin_group()];
    let token = jwk.encode(&claims);
    let response = server
        .get("/v1beta/policies/1")
        .add_header("Authorization", format!("Bearer {token}"))
        .await;
    assert_eq!(response.status_code(), 200);
}

#[tokio::test]
async fn test_get_policy_not_found() {
    let server = setup().await;

    let response = server.get("/v1beta/policies/1").await;
    assert_eq!(response.status_code(), 404);
}

#[tokio::test]
async fn test_delete_policy() {
    let server = setup().await;
    setup_policy(&server).await;

    let response = server.delete("/v1beta/policies/1").await;
    assert_eq!(response.status_code(), 204);

    let response = server.get("/v1beta/policies/1").await;
    assert_eq!(response.status_code(), 404);
}

#[tokio::test]
async fn test_delete_policy_authorization() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let mut claims = Claims::default();
    let token = jwk.encode(&claims);
    let config = jwk.config();
    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;

    let response = server.delete("/v1beta/policies/1").await;
    assert_eq!(response.status_code(), 401);

    let response = server
        .delete("/v1beta/policies/1")
        .add_header("Authorization", format!("Bearer {token}"))
        .await;
    assert_eq!(response.status_code(), 403);

    claims.groups = vec![jwk.admin_group()];
    let token = jwk.encode(&claims);

    let response = server
        .delete("/v1beta/policies/1")
        .add_header("Authorization", format!("Bearer {token}"))
        .await;
    assert_eq!(response.status_code(), 403);

    let put = server
        .put("/v1beta/policies/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&get_default_policy_payload())
        .await;
    assert_eq!(put.status_code(), 201);
    let id = put.json::<Value>()["id"].as_i64().unwrap();

    let response = server
        .delete(&format!("/v1beta/policies/{id}"))
        .add_header("Authorization", format!("Bearer {token}"))
        .await;
    assert_eq!(response.status_code(), 204);
}

#[tokio::test]
async fn test_delete_policy_cache_invalidation() {
    let server = setup().await;
    setup_policy(&server).await;

    let response = server
        .post("/v1beta/authorization/")
        .json(&json!({
            "principal": {
                "sub": "test-user",
                "info": {}
            },
            "action": {
                "service": "tags",
                "name": "get"
            },
            "resource": {
                "id": "Astronaut.usd",
                "type": "ResourceAddress",
                "data": {}
            },
            "context": {}
        }))
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    assert_eq!(
        result,
        json!({
            "decision": "allow",
            "service": "tags",
            "action": "get"
        })
    );

    let response = server.delete("/v1beta/policies/1").await;
    assert_eq!(response.status_code(), 204);

    let response = server
        .post("/v1beta/authorization/")
        .json(&json!({
            "principal": {
                "sub": "test-user",
                "info": {}
            },
            "action": {
                "service": "tags",
                "name": "get"
            },
            "resource": {
                "id": "Astronaut.usd",
                "type": "ResourceAddress",
                "data": {}
            },
            "context": {}
        }))
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    assert_eq!(
        result,
        json!({
            "decision": "deny",
            "service": "tags",
            "action": "get"
        })
    );
}

#[tokio::test]
async fn test_delete_system_policy_rejected() {
    let mut config = setup_config();
    config.database.init.policies_file = write_init_policies_file(
        r#"
policies:
  - policy: 'permit(principal == Principal::"test-user", action == Action::"tags:get", resource == ResourceAddress::"Astronaut.usd");'
"#,
    );
    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;

    let response = server.get("/v1beta/policies/1").await;
    assert_eq!(response.status_code(), 200);
    let result: Value = response.json();
    assert_eq!(result["created_by"], json!("SYSTEM"));

    let response = server.delete("/v1beta/policies/1").await;
    assert_eq!(response.status_code(), 403);
    assert!(response
        .text()
        .contains("System policies cannot be deleted"));

    let response = server.get("/v1beta/policies/1").await;
    assert_eq!(response.status_code(), 200);
}

#[tokio::test]
async fn test_put_policy_matching_system_policy_rejected() {
    let mut config = setup_config();
    config.database.init.policies_file = write_init_policies_file(
        r#"
policies:
  - policy: 'permit(principal == Principal::"test-user", action == Action::"tags:get", resource == ResourceAddress::"Astronaut.usd");'
    order: 7
"#,
    );
    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;

    let response = server
        .put("/v1beta/policies/")
        .json(&get_default_policy_payload())
        .await;
    assert_eq!(response.status_code(), 403);
    assert!(response
        .text()
        .contains("A system policy with the same text already exists"));

    let result: Value = server.get("/v1beta/policies/1").await.json();
    assert_eq!(result["created_by"], json!("SYSTEM"));
    assert_eq!(result["order"], json!(7));
}

#[tokio::test]
async fn test_put_policy_batch_matching_system_policy_rejected() {
    let mut config = setup_config();
    config.database.init.policies_file = write_init_policies_file(
        r#"
policies:
  - policy: 'permit(principal == Principal::"test-user", action == Action::"tags:get", resource == ResourceAddress::"Astronaut.usd");'
"#,
    );
    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;

    let batch = json!([
        { "policy": "permit(principal == Principal::\"new-user\", action == Action::\"tags:get\", resource == ResourceAddress::\"Astronaut.usd\");" },
        { "policy": "permit(principal == Principal::\"test-user\", action == Action::\"tags:get\", resource == ResourceAddress::\"Astronaut.usd\");" }
    ]);
    let response = server.put("/v1beta/policies/batch/").json(&batch).await;
    assert_eq!(response.status_code(), 403);
    assert!(response
        .text()
        .contains("A system policy with the same text already exists"));

    let result: Value = server.get("/v1beta/policies/").await.json();
    let items = result["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["created_by"], json!("SYSTEM"));
}

#[tokio::test]
async fn test_authorization() {
    let server = setup().await;
    setup_policy(&server).await;

    let response = server
        .post("/v1beta/authorization/")
        .json(&json!({
            "principal": {
                "sub": "test-user",
                "info": {}
            },
            "action": {
                "service": "tags",
                "name": "get"
            },
            "resource": {
                "id": "Astronaut.usd",
                "type": "ResourceAddress",
                "data": {}
            },
            "context": {}
        }))
        .await;

    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    assert_eq!(
        result,
        json!({
            "decision": "allow",
            "service": "tags",
            "action": "get"
        })
    );
}

#[tokio::test]
async fn test_authorization_cache_control() {
    let mut config = setup_config();
    config.cache.cache_control_max_age = 60;

    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;
    setup_policy(&server).await;

    let response = server
        .post("/v1beta/authorization/")
        .json(&json!({
            "principal": {
                "sub": "test-user",
                "info": {}
            },
            "action": {
                "service": "tags",
                "name": "get"
            },
            "resource": {
                "id": "Astronaut.usd",
                "type": "ResourceAddress",
                "data": {}
            },
            "context": {}
        }))
        .await;

    assert_eq!(response.status_code(), 200);
    assert_eq!(
        response.header("cache-control").to_str().unwrap(),
        "max-age=60, must-revalidate"
    );
}

#[tokio::test]
async fn test_authorization_deny_without_reason() {
    let mut config = setup_config();
    config.policies.enable_deny_reason = false;

    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;

    let principal = new_principal();
    let action = new_action();
    let policy = new_policy_payload("forbid", Some(&action), Some(&principal), None, "");
    let response = server.put("/v1beta/policies/").json(&policy).await;
    assert_eq!(response.status_code(), 201);

    let response = server
        .post("/v1beta/authorization/")
        .json(&json!({
            "principal": {
                "sub": "test-user",
            },
            "action": {
                "service": "tags",
                "name": "get"
            }
        }))
        .await;

    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    assert_eq!(
        result,
        json!({
            "decision": "deny",
            "service": "tags",
            "action": "get"
        })
    );
}

#[tokio::test]
async fn test_authorization_permit_priority() {
    let server = setup().await;

    let principal = new_principal();
    let action = new_action();
    let resource = new_resource();

    let policy = new_policy_payload(
        "permit",
        Some(&action),
        Some(&principal),
        Some(&resource),
        "",
    );
    let response = server.put("/v1beta/policies/").json(&policy).await;
    assert_eq!(response.status_code(), 201);

    let policy = new_policy_payload(
        "forbid",
        Some(&action),
        Some(&principal),
        Some(&resource),
        "",
    );
    let response = server.put("/v1beta/policies/").json(&policy).await;
    assert_eq!(response.status_code(), 201);

    setup_metadata(&server, EvaluationPriority::Forbid).await;
    let decision = verify(&server, &principal, &action, Some(&resource)).await;
    assert_eq!(decision, AuthorizationDecision::Deny);

    setup_metadata(&server, EvaluationPriority::Permit).await;
    let decision = verify(&server, &principal, &action, Some(&resource)).await;
    assert_eq!(decision, AuthorizationDecision::Allow);
}

/// A policy that fails to evaluate (it references a claim the principal does
/// not have) must be ignored rather than denying access, so a second valid
/// permit for the same action still results in allow.
#[tokio::test]
async fn test_authorization_ignores_policy_that_fails_to_evaluate() {
    let server = setup().await;

    let principal = new_principal();
    let action = new_action();
    let resource = new_resource();

    // Permit that cannot be evaluated: `principal.groups` does not exist on a
    // principal without group claims, so Cedar errors on this policy.
    let broken = new_policy_payload(
        "permit",
        Some(&action),
        Some(&principal),
        Some(&resource),
        "when { principal.groups.contains(\"test\") }",
    );
    let response = server.put("/v1beta/policies/").json(&broken).await;
    assert_eq!(response.status_code(), 201);

    // Permit that grants the action unconditionally.
    let allow = new_policy_payload(
        "permit",
        Some(&action),
        Some(&principal),
        Some(&resource),
        "",
    );
    let response = server.put("/v1beta/policies/").json(&allow).await;
    assert_eq!(response.status_code(), 201);

    let decision = verify(&server, &principal, &action, Some(&resource)).await;
    assert_eq!(decision, AuthorizationDecision::Allow);
}

#[tokio::test]
async fn test_authorization_without_resource() {
    let server = setup().await;
    setup_policy(&server).await;

    let response = server
        .post("/v1beta/authorization/")
        .json(&json!({
            "principal": {
                "sub": "test-user",
            },
            "action": {
                "service": "tags",
                "name": "get"
            }
        }))
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    assert_eq!(
        result,
        json!({
            "decision": "deny",
            "service": "tags",
            "action": "get"
        })
    );
}

#[tokio::test]
async fn test_authorization_batch() {
    let server = setup().await;
    setup_policy(&server).await;

    let response = server
        .post("/v1beta/authorization/batch/")
        .json(&json!({
            "batches": [
                {
                    "principal": {"sub": "test-user", "email": "user@test.com", "exp": 0},
                    "actions": [
                        {"name": "get", "service": "tags"}
                    ],
                    "resource": {
                        "id": "Astronaut.usd",
                        "type": "ResourceAddress",
                    }
                }
            ]}
        ))
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    assert_eq!(
        result,
        json!({
            "decisions": [
                {
                    "tags:get": {
                        "decision": "allow",
                        "service": "tags",
                        "action": "get"
                    },
                }
            ]
        })
    );
}

#[tokio::test]
async fn test_authorization_batch_deny_without_reason() {
    let mut config = setup_config();
    config.policies.enable_deny_reason = false;

    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;

    let principal = new_principal();
    let action = new_action();
    let resource = new_resource();
    let policy = new_policy_payload(
        "forbid",
        Some(&action),
        Some(&principal),
        Some(&resource),
        "",
    );
    let response = server.put("/v1beta/policies/").json(&policy).await;
    assert_eq!(response.status_code(), 201);

    let response = server
        .post("/v1beta/authorization/batch/")
        .json(&json!({
            "batches": [
                {
                    "principal": {"sub": "test-user"},
                    "actions": [
                        {"name": "get", "service": "tags"}
                    ],
                    "resource": {
                        "id": "Astronaut.usd",
                        "type": "ResourceAddress",
                    }
                }
            ]}
        ))
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    assert_eq!(
        result,
        json!({
            "decisions": [
                {
                    "tags:get": {
                        "decision": "deny",
                        "service": "tags",
                        "action": "get"
                    },
                }
            ]
        })
    );
}

#[tokio::test]
async fn test_authorization_batch_without_resource() {
    let server = setup().await;
    let payload = new_policy_payload(
        "permit",
        Some(&new_action()),
        Some(&new_principal()),
        None,
        "",
    );
    let response = server.put("/v1beta/policies/").json(&payload).await;
    assert_eq!(response.status_code(), 201);

    let response = server
        .post("/v1beta/authorization/batch/")
        .json(&json!({
            "batches": [
                {
                    "principal": {"sub": "test-user", "email": "user@test.com", "exp": 0},
                    "actions": [
                        {"name": "get", "service": "tags"}
                    ]
                }
            ]}
        ))
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    assert_eq!(
        result,
        json!({
            "decisions": [
                {
                    "tags:get": {
                        "decision": "allow",
                        "service": "tags",
                        "action": "get"
                    },
                }
            ]
        })
    );
}

#[tokio::test]
async fn test_authorization_batch_multiple_resources() {
    let server = setup().await;
    setup_policy(&server).await;

    let response = server
        .post("/v1beta/authorization/batch/")
        .json(&json!({
            "batches": [
                {
                    "principal": {"sub": "test-user", "email": "user@test.com", "exp": 0},
                    "actions": [{"name": "get", "service": "tags"}],
                    "resource": {
                        "id": "Astronaut.usd",
                        "type": "ResourceAddress",
                        "data": {
                            "resourceIdentity": "Astronaut.usd",
                            "metadata": {
                                "size": 28563210,
                                "timestamp": 0
                            }
                        }
                    }
                },
                {
                    "principal": {"sub": "test-user", "email": "user@test.com", "exp": 0},
                    "actions": [{"name": "get", "service": "tags"}],
                    "resource": {
                        "id": "Marbles_Assets.usd",
                        "type": "ResourceAddress",
                        "data": {
                            "resourceIdentity": "Marbles_Assets.usd",
                            "metadata": {
                                "size": 47104,
                                "timestamp": 0
                            }
                        }
                    }
                }
            ]
        }))
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    assert_eq!(
        result,
        json!({
            "decisions": [
                {
                    "tags:get": {
                        "decision": "allow",
                        "service": "tags",
                        "action": "get"
                    }
                },
                {
                    "tags:get": {
                        "decision": "deny",
                        "service": "tags",
                        "action": "get"
                    }
                }
            ]
        })
    );
}

#[tokio::test]
async fn test_authorization_batch_multiple_principals() {
    let server = setup().await;
    setup_policy(&server).await;

    let mut other_user = new_principal();
    other_user.id = "other-user".to_string();

    let payload = new_policy_payload(
        "permit",
        Some(&new_action()),
        Some(&other_user),
        Some(&new_resource()),
        "",
    );
    let response = server.put("/v1beta/policies/").json(&payload).await;
    assert_eq!(response.status_code(), 201);

    let response = server
        .post("/v1beta/authorization/batch/")
        .json(&json!({
            "batches": [
                {
                    "principal": {"sub": "test-user", "email": "user@test.com", "exp": 0},
                    "actions": [{"name": "get", "service": "tags"}],
                    "resource": {
                        "id": "Astronaut.usd",
                        "type": "ResourceAddress"
                    }
                },
                {
                    "principal": {"sub": "other-user", "email": "other@test.com", "exp": 0},
                    "actions": [{"name": "get", "service": "tags"}],
                    "resource": {
                        "id": "Astronaut.usd",
                        "type": "ResourceAddress"
                    }
                }
            ]
        }))
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    assert_eq!(
        result,
        json!({
            "decisions": [
                {
                    "tags:get": {
                        "decision": "allow",
                        "service": "tags",
                        "action": "get"
                    }
                },
                {
                    "tags:get": {
                        "decision": "allow",
                        "service": "tags",
                        "action": "get"
                    }
                }
            ]
        })
    );
}

/// Verifies that /authorization/batch/ returns "deny" summary
/// if any of the batches are denied and that evaluation
/// stops after the first denied batch.
#[tokio::test]
async fn test_authorization_batch_and_condition_summary_deny() {
    let server = setup().await;

    let payload = new_policy_payload(
        "forbid",
        Some(&new_action()),
        Some(&new_principal()),
        Some(&new_resource()),
        "",
    );
    let response = server.put("/v1beta/policies/").json(&payload).await;
    assert_eq!(response.status_code(), 201);

    let payload = new_policy_payload(
        "permit",
        Some(&new_action()),
        Some(&new_principal()),
        Some(&other_resource()),
        "",
    );
    let response = server.put("/v1beta/policies/").json(&payload).await;
    assert_eq!(response.status_code(), 201);

    let response = server
        .post("/v1beta/authorization/batch/")
        .json(&batch_payload(AuthorizationBatchCondition::And))
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    assert_eq!(
        result,
        json!({
            "summary": {"decision": "deny"},
            "decisions": [
                {
                    "tags:get": {
                        "decision": "deny",
                        "reason": "Explicit deny.",
                        "service": "tags",
                        "action": "get"
                    }
                },
                {
                    "tags:get": {
                        "decision": "skip",
                        "service": "tags",
                        "action": "get"
                    }
                }
            ]
        })
    );
}

#[tokio::test]
async fn test_authorization_batch_and_condition_summary_allow() {
    let server = setup().await;

    let payload = new_policy_payload(
        "permit",
        Some(&new_action()),
        Some(&new_principal()),
        Some(&new_resource()),
        "",
    );
    let response = server.put("/v1beta/policies/").json(&payload).await;
    assert_eq!(response.status_code(), 201);

    let payload = new_policy_payload(
        "permit",
        Some(&new_action()),
        Some(&new_principal()),
        Some(&other_resource()),
        "",
    );
    let response = server.put("/v1beta/policies/").json(&payload).await;
    assert_eq!(response.status_code(), 201);

    let response = server
        .post("/v1beta/authorization/batch/")
        .json(&batch_payload(AuthorizationBatchCondition::And))
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    assert_eq!(
        result,
        json!({
            "summary": {"decision": "allow"},
            "decisions": [
                {
                    "tags:get": {
                        "decision": "allow",
                        "service": "tags",
                        "action": "get"
                    }
                },
                {
                    "tags:get": {
                        "decision": "allow",
                        "service": "tags",
                        "action": "get"
                    }
                }
            ]
        })
    );
}

/// Same principal + action + resource type across many distinct resource IDs
/// with `condition: None` — the parallel evaluation path. Mixes allow and deny
/// to prove per-resource policy filtering is preserved.
#[tokio::test]
async fn test_authorization_batch_homogeneous_resources_mixed_decisions() {
    let server = setup().await;

    let allow_resources = ["batch-res-0.usd", "batch-res-1.usd", "batch-res-2.usd"];
    let deny_resource = "batch-res-denied.usd";

    for id in allow_resources {
        let resource = Resource {
            id: id.to_string(),
            kind: "ResourceAddress".to_string(),
            data: None,
        };
        let payload = new_policy_payload(
            "permit",
            Some(&new_action()),
            Some(&new_principal()),
            Some(&resource),
            "",
        );
        let response = server.put("/v1beta/policies/").json(&payload).await;
        assert_eq!(response.status_code(), 201);
    }

    let deny_res = Resource {
        id: deny_resource.to_string(),
        kind: "ResourceAddress".to_string(),
        data: None,
    };
    let payload = new_policy_payload(
        "forbid",
        Some(&new_action()),
        Some(&new_principal()),
        Some(&deny_res),
        "",
    );
    let response = server.put("/v1beta/policies/").json(&payload).await;
    assert_eq!(response.status_code(), 201);

    let mut batches = Vec::new();
    for id in allow_resources {
        batches.push(json!({
            "principal": {"sub": "test-user", "email": "user@test.com", "exp": 0},
            "actions": [{"name": "get", "service": "tags"}],
            "resource": {"id": id, "type": "ResourceAddress"}
        }));
    }
    batches.push(json!({
        "principal": {"sub": "test-user", "email": "user@test.com", "exp": 0},
        "actions": [{"name": "get", "service": "tags"}],
        "resource": {"id": deny_resource, "type": "ResourceAddress"}
    }));

    let response = server
        .post("/v1beta/authorization/batch/")
        .json(&json!({
            "condition": null,
            "batches": batches
        }))
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    assert!(result.get("summary").is_none() || result["summary"].is_null());
    assert_eq!(result["decisions"].as_array().unwrap().len(), 4);
    for i in 0..3 {
        assert_eq!(
            result["decisions"][i]["tags:get"]["decision"], "allow",
            "resource index {i} should allow"
        );
    }
    assert_eq!(
        result["decisions"][3]["tags:get"]["decision"], "deny",
        "explicit forbid resource should deny"
    );
}

#[tokio::test]
async fn test_authorization_batch_and_condition_multiple_actions() {
    let server = setup().await;
    setup_policy(&server).await;

    let response = server
        .post("/v1beta/authorization/batch/")
        .json(&json!({
            "condition": "and",
            "batches": [
                {
                    "principal": { "sub": "test-user", "email": "user@test.com", "exp": 0 },
                    "actions": [
                        { "name": "get", "service": "tags" },
                        { "name": "read", "service": "storage" },
                        { "name": "write", "service": "storage" },
                        { "name": "set", "service": "tags" },
                    ],
                    "resource": {
                        "id": "Astronaut.usd",
                        "type": "ResourceAddress",
                        "data": {
                            "resourceIdentity": "Astronaut.usd",
                            "metadata": {
                                "size": 1024,
                                "timestamp": 0
                            }
                        }
                    }
                }
           ]
        }
        ))
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    assert_eq!(
        result,
        json!({
            "summary": {"decision": "deny"},
            "decisions": [
                {
                    "tags:get": {
                        "decision": "allow",
                        "service": "tags",
                        "action": "get"
                    },
                    "storage:read": {
                        "decision": "deny",
                        "service": "storage",
                        "action": "read"
                    },
                    "storage:write": {
                        "decision": "skip",
                        "service": "storage",
                        "action": "write"
                    },
                    "tags:set": {
                        "decision": "skip",
                        "service": "tags",
                        "action": "set"
                    }
                }
            ]
        })
    );
}

/// Verifies that /authorization/batch/ returns "allow" summary
/// if any of the batches are allowed.
#[tokio::test]
async fn test_authorization_batch_or_condition_summary_allow() {
    let server = setup().await;
    setup_policy(&server).await;

    let payload = new_policy_payload(
        "forbid",
        Some(&new_action()),
        Some(&new_principal()),
        Some(&other_resource()),
        "",
    );
    let response = server.put("/v1beta/policies/").json(&payload).await;
    assert_eq!(response.status_code(), 201);

    let response = server
        .post("/v1beta/authorization/batch/")
        .json(&batch_payload(AuthorizationBatchCondition::Or))
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    assert_eq!(
        result,
        json!({
            "summary": {"decision": "allow"},
            "decisions": [
                {
                    "tags:get": {
                        "decision": "allow",
                        "service": "tags",
                        "action": "get"
                    }
                },
                {
                    "tags:get": {
                        "decision": "skip",
                        "service": "tags",
                        "action": "get"
                    }
                }
            ]
        })
    );
}

#[tokio::test]
async fn test_authorization_batch_or_condition_summary_deny() {
    let server = setup().await;
    let payload = new_policy_payload(
        "forbid",
        Some(&new_action()),
        Some(&new_principal()),
        Some(&new_resource()),
        "",
    );
    let response = server.put("/v1beta/policies/").json(&payload).await;
    assert_eq!(response.status_code(), 201);

    let payload = new_policy_payload(
        "forbid",
        Some(&new_action()),
        Some(&new_principal()),
        Some(&other_resource()),
        "",
    );
    let response = server.put("/v1beta/policies/").json(&payload).await;
    assert_eq!(response.status_code(), 201);

    let response = server
        .post("/v1beta/authorization/batch/")
        .json(&batch_payload(AuthorizationBatchCondition::Or))
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    assert_eq!(
        result,
        json!({
            "summary": {"decision": "deny"},
            "decisions": [
                {
                    "tags:get": {
                        "decision": "deny",
                        "reason": "Explicit deny.",
                        "service": "tags",
                        "action": "get"
                    }
                },
                {
                    "tags:get": {
                        "decision": "deny",
                        "reason": "Explicit deny.",
                        "service": "tags",
                        "action": "get"
                    }
                }
            ]
        })
    );
}

#[tokio::test]
async fn test_authorization_batch_or_condition_hierarchy() {
    let server = setup().await;
    setup_policy(&server).await;

    let mut projects = new_resource();
    projects.id = "/Projects/".to_string();

    let payload = new_policy_payload(
        "permit",
        Some(&new_action()),
        Some(&new_principal()),
        Some(&projects),
        "",
    );
    let response = server.put("/v1beta/policies/").json(&payload).await;
    assert_eq!(response.status_code(), 201);

    let response = server
        .post("/v1beta/authorization/batch/")
        .json(&json!({
            "condition": "or",
            "batches": [
                {
                    "principal": {"sub": "test-user", "email": "user@test.com", "exp": 0},
                    "actions": [{"name": "get", "service": "tags"}],
                    "resource": {
                        "id": "/Projects/Astronaut/Scene.usd",
                        "type": "ResourceAddress"
                    }
                },
                {
                    "principal": {"sub": "test-user", "email": "user@test.com", "exp": 0},
                    "actions": [{"name": "get", "service": "tags"}],
                    "resource": {
                        "id": "/Projects/Astronaut/",
                        "type": "ResourceAddress"
                    }
                },
                {
                    "principal": {"sub": "test-user", "email": "user@test.com", "exp": 0},
                    "actions": [{"name": "get", "service": "tags"}],
                    "resource": {
                        "id": "/Projects/",
                        "type": "ResourceAddress"
                    }
                },
                {
                    "principal": {"sub": "test-user", "email": "user@test.com", "exp": 0},
                    "actions": [{"name": "get", "service": "tags"}],
                    "resource": {
                        "id": "/",
                        "type": "ResourceAddress"
                    }
                }
            ]
        }))
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    assert_eq!(
        result,
        json!({
            "summary": {"decision": "allow"},
            "decisions": [
                {
                    "tags:get": {
                        "decision": "deny",
                        "service": "tags",
                        "action": "get"
                    }
                },
                {
                    "tags:get": {
                        "decision": "deny",
                        "service": "tags",
                        "action": "get"
                    }
                },
                {
                    "tags:get": {
                        "decision": "allow",
                        "service": "tags",
                        "action": "get"
                    }
                },
                {
                    "tags:get": {
                        "decision": "skip",
                        "service": "tags",
                        "action": "get"
                    }
                }
            ]
        })
    );
}

#[tokio::test]
async fn test_authorization_batch_permit_priority() {
    let server = setup().await;
    setup_policy(&server).await;

    // Allow the user to get tags for the resource.
    let payload = new_policy_payload(
        "permit",
        Some(&new_action()),
        Some(&new_principal()),
        Some(&new_resource()),
        "",
    );
    let response = server.put("/v1beta/policies/").json(&payload).await;
    assert_eq!(response.status_code(), 200);

    // Forbid all other users to get tags for the resource.
    let payload = new_policy_payload(
        "forbid",
        Some(&new_action()),
        None,
        Some(&new_resource()),
        "",
    );
    let response = server.put("/v1beta/policies/").json(&payload).await;
    assert_eq!(response.status_code(), 201);

    // Forbid the user to read the resource from storage.
    let mut action = new_action();
    action.name = "read".to_string();
    action.service = "storage".to_string();

    let payload = new_policy_payload(
        "forbid",
        Some(&action),
        Some(&new_principal()),
        Some(&new_resource()),
        "",
    );
    let response = server.put("/v1beta/policies/").json(&payload).await;
    assert_eq!(response.status_code(), 201);

    let payload = json!({
        "batches": [
            {
                "principal": {"sub": "test-user", "email": "user@test.com", "exp": 0},
                "actions": [{"name": "get", "service": "tags"}],
                "resource": {
                    "id": "Astronaut.usd",
                    "type": "ResourceAddress"
                }
            },
            {
                "principal": {"sub": "test-user", "email": "user@test.com", "exp": 0},
                "actions": [{"name": "read", "service": "storage"}],
                "resource": {
                    "id": "Astronaut.usd",
                    "type": "ResourceAddress"
                }
            }
        ]
    });

    let response = server
        .post("/v1beta/authorization/batch/")
        .json(&payload)
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    assert_eq!(
        result,
        json!({
            "decisions": [
                {
                    "tags:get": {
                        "decision": "deny",
                        "reason": "Explicit deny.",
                        "service": "tags",
                        "action": "get"
                    }
                },
                {
                    "storage:read": {
                        "decision": "deny",
                        "reason": "Explicit deny.",
                        "service": "storage",
                        "action": "read"
                    }
                }
            ]
        })
    );

    // Change evaluation priority to permit for tags.
    setup_metadata(&server, EvaluationPriority::Permit).await;

    let response = server
        .post("/v1beta/authorization/batch/")
        .json(&payload)
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    assert_eq!(
        result,
        json!({
            "decisions": [
                {
                    "tags:get": {
                        "decision": "allow",
                        "service": "tags",
                        "action": "get"
                    }
                },
                {
                    "storage:read": {
                        "decision": "deny",
                        "reason": "Explicit deny.",
                        "service": "storage",
                        "action": "read"
                    }
                }
            ]
        })
    );
}

#[tokio::test]
async fn test_authorization_batch_meta_permissions_denied() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let claims = Claims::default();
    let token = jwk.encode(&claims);

    let mut config = jwk.config();
    config.database.init.policies_file = write_init_policies_file(
        r#"
policies:
  - policy: 'permit(principal, action == Action::"permissions:edit", resource);'
  - policy: 'permit(principal, action == Action::"permissions:meta:write", resource);'
"#,
    );

    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;

    let mut action = new_action();
    action.service = "permissions".to_string();
    action.name = "view".to_string();

    let payload = new_policy_payload("forbid", Some(&action), Some(&new_principal()), None, "");
    let response = server
        .put("/v1beta/policies/")
        .json(&payload)
        .add_header("Authorization", format!("Bearer {}", token))
        .await;
    assert_eq!(response.status_code(), 201);

    let payload = json!({
        "batches": [
            {
                "principal": {"sub": "other-user"},
                "actions": [{"name": "get", "service": "tags"}],
                "resource": {
                    "id": "Astronaut.usd",
                    "type": "ResourceAddress"
                }
            }
        ]
    });

    let response = server
        .post("/v1beta/authorization/batch/")
        .json(&payload)
        .add_header("Authorization", format!("Bearer {}", token))
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    assert_eq!(
        result,
        json!({
            "decisions": [
                {
                    "tags:get": {
                        "decision": "deny",
                        "reason": "Permission denied for evaluating Action::\"permissions:view\" for Principal::\"test-user\"",
                        "service": "tags",
                        "action": "get"
                    }
                }
            ]
        })
    );
}

#[tokio::test]
async fn test_authentication() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let claims = Claims::default();
    let token = jwk.encode(&claims);

    let config = jwk.config();
    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;
    let body = json!({
        "action": {
            "service": "tags",
            "name": "get"
        }
    });
    let response = server.post("/v1beta/authorization/").json(&body).await;
    assert_eq!(response.status_code(), 401);

    let response = server
        .post("/v1beta/authorization/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .await;
    assert_eq!(response.status_code(), 200);
}

#[tokio::test]
async fn test_authentication_jwks_cache() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let claims = Claims::default();
    let token = jwk.encode(&claims);

    let config = jwk.config();
    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;
    for _ in 0..5 {
        let body = json!({
            "principal": {
                "sub": "test-user",
            },
            "action": {
                "service": "tags",
                "name": "get"
            }
        });
        server
            .post("/v1beta/authorization/")
            .add_header("Authorization", format!("Bearer {token}"))
            .json(&body)
            .await;
    }
}

#[tokio::test]
async fn test_authentication_jwt_cache() {
    let jwk = JsonWebKeyFixture::new_async().await;

    let claims = Claims::default();
    let token = jwk.encode(&claims);

    let _userinfo_guard = jwk.userinfo_mock(200, json!({"sub": claims.sub}), 1).await;

    let mut config = jwk.config();
    config.auth.authentication_type = AuthType::Opaque;

    let cache = setup_cache(&config);

    let server = setup_server(config, cache).await;
    let body = json!({
        "principal": {
            "sub": "test-user",
        },
        "action": {
            "service": "tags",
            "name": "get"
        }
    });

    let response = server
        .post("/v1beta/authorization/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .await;
    assert_eq!(response.status_code(), 200);

    let response = server
        .post("/v1beta/authorization/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .await;
    assert_eq!(response.status_code(), 200);
}

#[tokio::test]
async fn test_authentication_expired() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let claims = Claims::new(
        OffsetDateTime::now_utc()
            .sub(Duration::from_secs(300))
            .unix_timestamp(),
    );
    let token = jwk.encode(&claims);

    let config = jwk.config();
    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;
    let body = json!({
        "principal": {
            "sub": "test-user",
        },
        "action": {
            "service": "tags",
            "name": "get"
        }
    });

    let response = server
        .post("/v1beta/authorization/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .await;
    assert_eq!(response.status_code(), 401);
}

#[tokio::test]
async fn test_authentication_jwt_cache_expired() {
    let jwk = JsonWebKeyFixture::new_async().await;

    let mut config = jwk.config();
    config.auth.jwt_leeway = 0;

    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;

    let claims = Claims::new(
        OffsetDateTime::now_utc()
            .add(Duration::from_secs(3))
            .unix_timestamp(),
    );
    let token = jwk.encode(&claims);

    let body = json!({
        "principal": {
            "sub": "test-user",
        },
        "action": {
            "service": "tags",
            "name": "get"
        }
    });

    let response = server
        .post("/v1beta/authorization/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .await;
    assert_eq!(response.status_code(), 200);

    sleep(Duration::from_secs(4)).await;
    let response = server
        .post("/v1beta/authorization/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .await;
    assert_eq!(response.status_code(), 401);
}

#[tokio::test]
async fn test_authentication_invalid_jwt_cache() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let claims = Claims::new(
        OffsetDateTime::now_utc()
            .sub(Duration::from_secs(10))
            .unix_timestamp(),
    );
    let token = jwk.encode(&claims);

    let mut config = jwk.config();
    config.auth.jwt_leeway = 0;

    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;
    let body = json!({
        "principal": {
            "sub": "test-user",
        },
        "action": {
            "service": "tags",
            "name": "get"
        }
    });

    let response = server
        .post("/v1beta/authorization/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .await;
    assert_eq!(response.status_code(), 401);

    let response = server
        .post("/v1beta/authorization/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .await;
    assert_eq!(response.status_code(), 401);
}

/// Flooding the negative cache must not displace verified principals from
/// the positive cache. Verifies the security property motivating the split
/// of `AuthCache` into two independent moka instances.
#[tokio::test]
async fn test_invalid_token_flood_does_not_evict_valid_principal() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let claims = Claims::default();
    let valid_token = jwk.encode(&claims);

    let mut config = jwk.config();
    config.auth.authentication_type = AuthType::Opaque;
    config.auth.invalid_token_cache_size = 4;

    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;
    let body = json!({
        "principal": {"sub": "test-user"},
        "action": {"service": "tags", "name": "get"}
    });

    {
        let _userinfo_ok = jwk.userinfo_mock(200, json!({"sub": claims.sub}), 1).await;
        let response = server
            .post("/v1beta/authorization/")
            .add_header("Authorization", format!("Bearer {valid_token}"))
            .json(&body)
            .await;
        assert_eq!(response.status_code(), 200);
    }

    // From here the IdP rejects everything. Flood the negative cache far
    // past its capacity (4); a regression that funnels both kinds of
    // entries through one moka instance would evict the previously cached
    // valid principal and force the next valid-token request to re-hit
    // `/userinfo`, which would now return 401.
    let _userinfo_unauth = jwk.userinfo_mock_unbounded(401, json!({})).await;
    for i in 0..32 {
        let bogus = format!("bogus-token-{i}");
        let response = server
            .post("/v1beta/authorization/")
            .add_header("Authorization", format!("Bearer {bogus}"))
            .json(&body)
            .await;
        assert_eq!(response.status_code(), 401);
    }

    let response = server
        .post("/v1beta/authorization/")
        .add_header("Authorization", format!("Bearer {valid_token}"))
        .json(&body)
        .await;
    assert_eq!(response.status_code(), 200);
}

/// Issue #9: the cached lifetime of a verified JWT is capped at
/// `now + token_ttl`, even when the JWT's own `exp` is far in the future.
/// Without this cap, an IdP-side revocation (logout, key rotation, account
/// disable) would take up to the JWT's natural `exp` (commonly 8-24 h) to
/// be observed by this service.
///
/// The moka cache is intentionally constructed with a `time_to_live`
/// longer than `token_ttl` so that the per-entry cap applied inside
/// `cache_valid_token` is what bounds the lifetime here — not the moka
/// eviction. This isolates the regression to the cap logic itself.
#[tokio::test]
async fn test_valid_jwt_cache_lifetime_capped_at_token_ttl() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let claims = Claims::new(
        OffsetDateTime::now_utc()
            .add(Duration::from_secs(3600))
            .unix_timestamp(),
    );
    let token = jwk.encode(&claims);

    let mut config = jwk.config();
    config.auth.authentication_type = AuthType::JwtWithUserInfo;
    config.auth.token_ttl = Duration::from_secs(1);
    config.auth.jwt_leeway = 0;

    let cache = Arc::new(AuthCache::new(
        config.auth.token_cache_size,
        Duration::from_secs(60),
        config.auth.invalid_token_cache_size,
        config.auth.invalid_token_cache_ttl,
    ));
    let server = setup_server(config, cache).await;
    let body = json!({
        "principal": {"sub": "test-user"},
        "action": {"service": "tags", "name": "get"}
    });

    {
        let _userinfo_ok = jwk.userinfo_mock(200, json!({"sub": claims.sub}), 1).await;
        let response = server
            .post("/v1beta/authorization/")
            .add_header("Authorization", format!("Bearer {token}"))
            .json(&body)
            .await;
        assert_eq!(response.status_code(), 200);
    }

    sleep(Duration::from_secs(2)).await;

    // Simulate an IdP-side revocation: userinfo now rejects the same JWT.
    // Without the cap the cached principal would still be honored for
    // the JWT's full 1 h lifetime; with the cap the cache entry has
    // expired, re-verification reaches the (now-rejecting) IdP, and the
    // request is denied.
    let _userinfo_unauth = jwk.userinfo_mock(401, json!({}), 1).await;
    let response = server
        .post("/v1beta/authorization/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .await;
    assert_eq!(response.status_code(), 401);
}

/// Negative-cache TTL is independent of `token_ttl`: a token rejected by
/// the IdP must be re-checked once the (much shorter) negative TTL elapses,
/// even when the positive `token_ttl` is still valid for hours.
#[tokio::test]
async fn test_invalid_token_cache_ttl_independent_from_token_ttl() {
    let jwk = JsonWebKeyFixture::new_async().await;

    let mut config = jwk.config();
    config.auth.authentication_type = AuthType::Opaque;
    config.auth.token_ttl = Duration::from_secs(60 * 30);
    config.auth.invalid_token_cache_ttl = Duration::from_secs(1);

    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;
    let body = json!({
        "principal": {"sub": "test-user"},
        "action": {"service": "tags", "name": "get"}
    });

    let bogus = "bogus-opaque-token";

    // The IdP is hit on first sight (cache miss) and again after the
    // negative-cache TTL elapses; the in-window second request must be
    // served from the negative cache (no IdP call).
    let _userinfo_unauth = jwk.userinfo_mock(401, json!({}), 2).await;

    let response = server
        .post("/v1beta/authorization/")
        .add_header("Authorization", format!("Bearer {bogus}"))
        .json(&body)
        .await;
    assert_eq!(response.status_code(), 401);

    let response = server
        .post("/v1beta/authorization/")
        .add_header("Authorization", format!("Bearer {bogus}"))
        .json(&body)
        .await;
    assert_eq!(response.status_code(), 401);

    sleep(Duration::from_secs(2)).await;

    let response = server
        .post("/v1beta/authorization/")
        .add_header("Authorization", format!("Bearer {bogus}"))
        .json(&body)
        .await;
    assert_eq!(response.status_code(), 401);
}

/// A JWT whose `principal_id_claim` (here the default `sub`) is the
/// empty string must be rejected. The downstream service uses
/// `principal.id == ""` as the "no principal" sentinel for auth-disabled
/// and public endpoints, so honoring an empty-string id from a verified
/// JWT would silently conflate an authenticated caller with the
/// unauthenticated case.
#[tokio::test]
async fn test_jwt_with_empty_sub_is_rejected() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let mut claims = Claims::default();
    claims.sub = String::new();
    let token = jwk.encode(&claims);

    let config = jwk.config();
    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;
    let body = json!({
        "principal": {"sub": "test-user"},
        "action": {"service": "tags", "name": "get"}
    });

    let response = server
        .post("/v1beta/authorization/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .await;
    assert_eq!(response.status_code(), 401);
}

/// When the configured `principal_id_claim` is something other than
/// `sub`, both that claim and `sub` need to be missing or empty for the
/// token to be rejected. The configured-claim fallback to `sub` keeps
/// working as long as `sub` carries a meaningful value.
#[tokio::test]
async fn test_jwt_with_empty_configured_claim_and_empty_sub_is_rejected() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let mut claims = Claims::default();
    claims.sub = String::new();
    claims.email = String::new();
    let token = jwk.encode(&claims);

    let mut config = jwk.config();
    config.auth.principal_id_claim = "email".to_string();
    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;
    let body = json!({
        "principal": {"sub": "test-user"},
        "action": {"service": "tags", "name": "get"}
    });

    let response = server
        .post("/v1beta/authorization/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .await;
    assert_eq!(response.status_code(), 401);
}

#[tokio::test]
async fn test_jwt_required_claims_are_enforced() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let mut config = jwk.config();
    config.auth.jwt_required_claims = parse_claim_rules(
        r#"[{"claim": "appid", "operator": "in", "values": ["web-client", "cli-client"]}]"#,
    )
    .unwrap();
    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;
    let body = json!({
        "principal": {"sub": "test-user"},
        "action": {"service": "tags", "name": "get"}
    });

    for (appid, expected_status) in [
        (Some("web-client"), 200),
        (Some("other-client"), 401),
        (None, 401),
    ] {
        let mut claims = Claims::default();
        claims.appid = appid.map(str::to_string);
        let token = jwk.encode(&claims);

        let response = server
            .post("/v1beta/authorization/")
            .add_header("Authorization", format!("Bearer {token}"))
            .json(&body)
            .await;
        assert_eq!(
            response.status_code(),
            expected_status,
            "unexpected status for appid {appid:?}"
        );
    }
}

#[tokio::test]
async fn test_opaque_token_violating_required_claims_is_negative_cached() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let token = jwk.encode(&Claims::default());

    let _userinfo_guard = jwk
        .userinfo_mock(200, json!({"sub": "test-user", "appid": "other-client"}), 1)
        .await;

    let mut config = jwk.config();
    config.auth.authentication_type = AuthType::Opaque;
    config.auth.jwt_required_claims =
        parse_claim_rules(r#"[{"claim": "appid", "operator": "equals", "value": "web-client"}]"#)
            .unwrap();
    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;
    let body = json!({
        "principal": {"sub": "test-user"},
        "action": {"service": "tags", "name": "get"}
    });

    for _ in 0..2 {
        let response = server
            .post("/v1beta/authorization/")
            .add_header("Authorization", format!("Bearer {token}"))
            .json(&body)
            .await;
        assert_eq!(response.status_code(), 401);
    }
}

#[tokio::test]
async fn test_opaque_token_with_empty_userinfo_sub_is_rejected() {
    let jwk = JsonWebKeyFixture::new_async().await;

    let mut config = jwk.config();
    config.auth.authentication_type = AuthType::Opaque;
    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;
    let body = json!({
        "principal": {"sub": "test-user"},
        "action": {"service": "tags", "name": "get"}
    });

    let _userinfo_empty = jwk.userinfo_mock(200, json!({"sub": ""}), 1).await;

    let response = server
        .post("/v1beta/authorization/")
        .add_header("Authorization", "Bearer opaque-token")
        .json(&body)
        .await;
    assert_eq!(response.status_code(), 401);
}

#[tokio::test]
async fn test_authorization_opaque_token_cache_expired() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let claims = Claims::default();
    let token = jwk.encode(&claims);

    let mut config = jwk.config();
    config.auth.authentication_type = AuthType::Opaque;
    config.auth.token_ttl = Duration::from_secs(1);

    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;
    let body = json!({
        "principal": {
            "sub": "test-user",
        },
        "action": {
            "service": "tags",
            "name": "get"
        }
    });

    {
        let _userinfo_guard = jwk.userinfo_mock(200, json!({"sub": claims.sub}), 1).await;

        let response = server
            .post("/v1beta/authorization/")
            .add_header("Authorization", format!("Bearer {token}"))
            .json(&body)
            .await;
        assert_eq!(response.status_code(), 200);
    }

    let _userinfo_guard = jwk.userinfo_mock(401, json!({}), 1).await;

    // Verify that the token is still cached
    let response = server
        .post("/v1beta/authorization/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .await;
    assert_eq!(response.status_code(), 200);

    // Wait until the cache ttl expires and verify that the token is no longer valid
    sleep(Duration::from_secs(2)).await;
    let response = server
        .post("/v1beta/authorization/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .await;
    assert_eq!(response.status_code(), 401);
}

#[tokio::test]
async fn test_authorization_invalid_opaque_token_cache() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let claims = Claims::default();
    let token = jwk.encode(&claims);

    let _userinfo_guard = jwk.userinfo_mock(401, "invalid token.".into(), 1).await;

    let mut config = jwk.config();
    config.auth.authentication_type = AuthType::Opaque;

    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;
    let body = json!({
        "principal": {
            "sub": "test-user",
        },
        "action": {
            "service": "tags",
            "name": "get"
        }
    });

    let response = server
        .post("/v1beta/authorization/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .await;
    assert_eq!(response.status_code(), 401);

    let response = server
        .post("/v1beta/authorization/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .await;
    assert_eq!(response.status_code(), 401);
}

#[tokio::test]
async fn test_authorization_with_explicit_principal() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let claims = Claims::default();
    let token = jwk.encode(&claims);

    let mut config = jwk.config();
    config.database.init.policies_file = write_init_policies_file(
        r#"
policies:
  - policy: 'permit(principal == Principal::"test-user", action, resource);'
"#,
    );

    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;

    let body = json!({
        "principal": {
            "sub": "test-service"
        },
        "action": {
            "service": "tags",
            "name": "get"
        },
        "resource": {
            "id": "Astronaut.usd",
            "type": "ResourceAddress"
        }
    });

    let response = server
        .post("/v1beta/authorization/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    assert_eq!(
        result,
        json!({
            "decision": "deny",
            "service": "tags",
            "action": "get"
        })
    );

    let policy = json!({
        "policy": "permit(principal == Principal::\"test-service\", action == Action::\"tags:get\", resource == ResourceAddress::\"Astronaut.usd\");",
        "action": {
            "service": "tags",
            "name": "get"
        },
        "principal": "test-service",
        "resource": {
            "id": "Astronaut.usd",
            "type": "ResourceAddress"
        }
    });
    let response = server
        .put("/v1beta/policies/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&policy)
        .await;
    assert_eq!(response.status_code(), 201);

    let response = server
        .post("/v1beta/authorization/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    assert_eq!(
        result,
        json!({
            "decision": "allow",
            "service": "tags",
            "action": "get"
        })
    );
}

/// A request whose payload carries `principal.sub` matching the
/// requester but `info` claims that resolve to a different id must be
/// classified as a cross-principal lookup. The `permissions:view` gate
/// then prevents the caller from probing another principal's
/// permissions through the request body.
#[tokio::test]
async fn test_authorization_caller_supplied_info_cannot_rewrite_principal_id() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let claims = Claims::default();
    let token = jwk.encode(&claims);

    let mut config = jwk.config();
    // `permissions:meta:write` lets the test register `tags` metadata. Only
    // `system-admin` is granted any other capability, so a successful
    // bypass would surface as `allow` instead of the rejected lookup.
    config.database.init.policies_file = write_init_policies_file(
        r#"
policies:
  - policy: 'permit(principal, action == Action::"permissions:meta:write", resource);'
  - policy: 'permit(principal == Principal::"system-admin", action, resource);'
"#,
    );

    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;

    // Default `id_claim`: `info.sub` resolves the target to "system-admin"
    // while the requester resolves to "test-user". The diverging resolved
    // ids force the cross-principal path; without `permissions:view` the
    // lookup is rejected with 403.
    let body = json!({
        "principal": {
            "sub": claims.sub,
            "info": { "sub": "system-admin" }
        },
        "action": { "service": "tags", "name": "get" },
        "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
    });

    let response = server
        .post("/v1beta/authorization/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .await;
    assert_eq!(
        response.status_code(),
        403,
        "caller-supplied info.sub must not bypass the cross-principal view check"
    );

    // Same exploit through the batch endpoint: `permissions:view` failures
    // map to per-action `Deny` rather than 403.
    let payload = json!({
        "batches": [{
            "principal": {
                "sub": claims.sub,
                "info": { "sub": "system-admin" }
            },
            "actions": [{ "name": "get", "service": "tags" }],
            "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
        }]
    });
    let response = server
        .post("/v1beta/authorization/batch/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&payload)
        .await;
    assert_eq!(response.status_code(), 200);
    assert_eq!(
        response.json::<Value>()["decisions"][0]["tags:get"]["decision"],
        "deny",
        "batch path must also reject caller-supplied info.sub for id rewrite"
    );

    // Same exploit through the per-service `id_claim` path: register
    // `tags` with `id_claim: email` and set `info.email` to a foreign id.
    let metadata = json!({ "service": "tags", "id_claim": "email" });
    let response = server
        .put("/v1beta/services/tags/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&metadata)
        .await;
    assert_eq!(response.status_code(), 200);

    let body = json!({
        "principal": {
            "sub": claims.sub,
            "info": { "email": "system-admin", "sub": "system-admin" }
        },
        "action": { "service": "tags", "name": "get" },
        "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
    });
    let response = server
        .post("/v1beta/authorization/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .await;
    assert_eq!(
        response.status_code(),
        403,
        "caller-supplied info must not bypass the view check via per-service id_claim"
    );

    // Batch path mirrors the single-action behavior, except that
    // `permissions:view` failures map to per-action `Deny` instead of 403.
    let payload = json!({
        "batches": [{
            "principal": {
                "sub": claims.sub,
                "info": { "email": "system-admin", "sub": "system-admin" }
            },
            "actions": [{ "name": "get", "service": "tags" }],
            "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
        }]
    });
    let response = server
        .post("/v1beta/authorization/batch/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&payload)
        .await;
    assert_eq!(response.status_code(), 200);
    let result: Value = response.json();
    assert_eq!(
        result["decisions"][0]["tags:get"]["decision"], "deny",
        "batch path must also reject caller-supplied info that resolves to a foreign id"
    );
}

/// Self lookups still expose caller-supplied `principal.info` to Cedar
/// as attribute data.
#[tokio::test]
async fn test_authorization_self_principal_info_attributes_are_preserved_for_cedar() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let claims = Claims::default();
    let token = jwk.encode(&claims);

    let mut config = jwk.config();
    config.database.init.policies_file = write_init_policies_file(
        r#"
policies:
  - policy: 'permit(principal == Principal::"test-user", action == Action::"tags:get", resource) when { principal has feature_flag && principal.feature_flag == "enabled" };'
"#,
    );

    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;

    // Same authenticated user, but no attribute supplied: the policy's
    // `when { principal.feature_flag == "enabled" }` clause cannot be satisfied.
    let body_without_attr = json!({
        "action": { "service": "tags", "name": "get" },
        "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
    });
    let response = server
        .post("/v1beta/authorization/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&body_without_attr)
        .await;
    assert_eq!(response.status_code(), 200);
    assert_eq!(response.json::<Value>()["decision"], "deny");

    // Caller supplies `feature_flag` as a Cedar attribute on their own
    // principal. The id stays `test-user` (from the verified token), and
    // the attribute is visible to the policy.
    let body_with_attr = json!({
        "principal": {
            "sub": claims.sub,
            "info": { "feature_flag": "enabled" }
        },
        "action": { "service": "tags", "name": "get" },
        "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
    });
    let response = server
        .post("/v1beta/authorization/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&body_with_attr)
        .await;
    assert_eq!(response.status_code(), 200);
    assert_eq!(
        response.json::<Value>()["decision"],
        "allow",
        "caller-supplied principal.info attributes must be visible to Cedar"
    );

    // Batch endpoint: same two cases, same outcomes.
    let payload_without_attr = json!({
        "batches": [{
            "actions": [{ "name": "get", "service": "tags" }],
            "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
        }]
    });
    let response = server
        .post("/v1beta/authorization/batch/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&payload_without_attr)
        .await;
    assert_eq!(response.status_code(), 200);
    assert_eq!(
        response.json::<Value>()["decisions"][0]["tags:get"]["decision"],
        "deny"
    );

    let payload_with_attr = json!({
        "batches": [{
            "principal": {
                "sub": claims.sub,
                "info": { "feature_flag": "enabled" }
            },
            "actions": [{ "name": "get", "service": "tags" }],
            "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
        }]
    });
    let response = server
        .post("/v1beta/authorization/batch/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&payload_with_attr)
        .await;
    assert_eq!(response.status_code(), 200);
    assert_eq!(
        response.json::<Value>()["decisions"][0]["tags:get"]["decision"],
        "allow",
        "batch path must also expose caller-supplied principal.info to Cedar"
    );
}

/// Cross-principal lookups still expose caller-supplied `principal.info`
/// to Cedar as attribute data.
#[tokio::test]
async fn test_authorization_cross_principal_info_attributes_are_preserved_for_cedar() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let claims = Claims::default();
    let token = jwk.encode(&claims);

    let mut config = jwk.config();
    // Grant the requester `permissions:view` so the cross-principal lookup
    // is allowed, plus a target policy that depends on a caller-supplied
    // attribute on the lookup principal.
    config.database.init.policies_file = write_init_policies_file(
        r#"
policies:
  - policy: 'permit(principal == Principal::"test-user", action == Action::"permissions:view", resource);'
  - policy: 'permit(principal == Principal::"target-user", action == Action::"tags:get", resource) when { principal has tier && principal.tier == "gold" };'
"#,
    );

    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;

    // Without the attribute, the gold-tier guard fails.
    let body_without_attr = json!({
        "principal": { "sub": "target-user" },
        "action": { "service": "tags", "name": "get" },
        "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
    });
    let response = server
        .post("/v1beta/authorization/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&body_without_attr)
        .await;
    assert_eq!(response.status_code(), 200);
    assert_eq!(response.json::<Value>()["decision"], "deny");

    // With the attribute supplied, Cedar can satisfy the `when` clause.
    let body_with_attr = json!({
        "principal": {
            "sub": "target-user",
            "info": { "tier": "gold" }
        },
        "action": { "service": "tags", "name": "get" },
        "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
    });
    let response = server
        .post("/v1beta/authorization/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&body_with_attr)
        .await;
    assert_eq!(response.status_code(), 200);
    assert_eq!(
        response.json::<Value>()["decision"],
        "allow",
        "cross-principal lookups must still accept caller-supplied principal.info attributes"
    );

    // Batch endpoint: same two cases, same outcomes.
    let payload_without_attr = json!({
        "batches": [{
            "principal": { "sub": "target-user" },
            "actions": [{ "name": "get", "service": "tags" }],
            "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
        }]
    });
    let response = server
        .post("/v1beta/authorization/batch/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&payload_without_attr)
        .await;
    assert_eq!(response.status_code(), 200);
    assert_eq!(
        response.json::<Value>()["decisions"][0]["tags:get"]["decision"],
        "deny"
    );

    let payload_with_attr = json!({
        "batches": [{
            "principal": {
                "sub": "target-user",
                "info": { "tier": "gold" }
            },
            "actions": [{ "name": "get", "service": "tags" }],
            "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
        }]
    });
    let response = server
        .post("/v1beta/authorization/batch/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&payload_with_attr)
        .await;
    assert_eq!(response.status_code(), 200);
    assert_eq!(
        response.json::<Value>()["decisions"][0]["tags:get"]["decision"],
        "allow",
        "batch path must also accept caller-supplied principal.info on cross-principal lookups"
    );
}

/// For cross-principal lookups, the per-service `id_claim` must resolve
/// the target id from the caller-supplied `principal.info` (the trusted
/// proxy use case).
#[tokio::test]
async fn test_authorization_cross_principal_id_resolves_from_info() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let claims = Claims::default();
    let token = jwk.encode(&claims);

    let mut config = jwk.config();
    // The requester is permitted to view permissions and edit metadata.
    // The target policy permits the principal whose id is the email
    // `target@example.test` — the value the per-service `id_claim: email`
    // resolution must produce from the caller-supplied `info`.
    config.database.init.policies_file = write_init_policies_file(
        r#"
policies:
  - policy: 'permit(principal == Principal::"test-user", action == Action::"permissions:view", resource);'
  - policy: 'permit(principal, action == Action::"permissions:meta:write", resource);'
  - policy: 'permit(principal == Principal::"target@example.test", action == Action::"tags:get", resource);'
"#,
    );

    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;

    let metadata = json!({ "service": "tags", "id_claim": "email" });
    let response = server
        .put("/v1beta/services/tags/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&metadata)
        .await;
    assert_eq!(response.status_code(), 200);

    // Caller forwards the target principal's full claim set; `tags` resolves
    // the id from `email`.
    let body = json!({
        "principal": {
            "sub": "target-opaque-sub",
            "info": {
                "sub": "target-opaque-sub",
                "email": "target@example.test"
            }
        },
        "action": { "service": "tags", "name": "get" },
        "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
    });

    let response = server
        .post("/v1beta/authorization/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .await;
    assert_eq!(response.status_code(), 200);
    assert_eq!(
        response.json::<Value>()["decision"],
        "allow",
        "per-service id_claim must resolve cross-principal id from caller-supplied info"
    );

    // Same request through the batch endpoint must produce the same outcome.
    let payload = json!({
        "batches": [{
            "principal": {
                "sub": "target-opaque-sub",
                "info": {
                    "sub": "target-opaque-sub",
                    "email": "target@example.test"
                }
            },
            "actions": [{ "name": "get", "service": "tags" }],
            "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
        }]
    });
    let response = server
        .post("/v1beta/authorization/batch/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&payload)
        .await;
    assert_eq!(response.status_code(), 200);
    assert_eq!(
        response.json::<Value>()["decisions"][0]["tags:get"]["decision"],
        "allow",
        "batch path must apply per-service id_claim resolution to caller-supplied info"
    );
}

/// `permissions:view` must evaluate against the same target id Cedar will
/// see at evaluation time. Resolving the target id through the target
/// service's `id_claim` BEFORE the view check closes the bypass where a
/// caller picks one id for the view check and a different one for the
/// actual evaluation by stuffing `info` with claims that resolve elsewhere.
#[tokio::test]
async fn test_authorization_view_check_uses_resolved_target_id() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let claims = Claims::default();
    let token = jwk.encode(&claims);

    let mut config = jwk.config();
    // `test-user` may view permissions only for `alice@example.test`. The
    // target action would permit `system-admin` if the caller could
    // smuggle that id through the view check.
    config.database.init.policies_file = write_init_policies_file(
        r#"
policies:
  - policy: 'permit(principal, action == Action::"permissions:meta:write", resource);'
  - policy: 'permit(principal == Principal::"test-user", action == Action::"permissions:view", resource) when { context.principal.id == "alice@example.test" };'
  - policy: 'permit(principal == Principal::"system-admin", action == Action::"tags:get", resource);'
  - policy: 'permit(principal == Principal::"alice@example.test", action == Action::"tags:get", resource);'
"#,
    );

    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;

    let metadata = json!({ "service": "tags", "id_claim": "email" });
    let response = server
        .put("/v1beta/services/tags/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&metadata)
        .await;
    assert_eq!(response.status_code(), 200);

    // Bypass attempt: payload `sub` names alice (whom the caller can view),
    // but `info.email` resolves the target to `system-admin` for `tags`.
    // The view check must deny because the resolved target id is
    // `system-admin`, not `alice@example.test`.
    let bypass = json!({
        "principal": {
            "sub": "alice@example.test",
            "info": { "email": "system-admin" }
        },
        "action": { "service": "tags", "name": "get" },
        "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
    });
    let response = server
        .post("/v1beta/authorization/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&bypass)
        .await;
    assert_eq!(response.status_code(), 403);

    // Positive control: the legitimate proxy case using the same id on both
    // sides resolves to alice and is permitted.
    let allowed = json!({
        "principal": {
            "sub": "alice@example.test",
            "info": { "email": "alice@example.test" }
        },
        "action": { "service": "tags", "name": "get" },
        "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
    });
    let response = server
        .post("/v1beta/authorization/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&allowed)
        .await;
    assert_eq!(response.status_code(), 200);
    assert_eq!(response.json::<Value>()["decision"], "allow");

    // Same bypass shape via the batch endpoint must also be denied
    // (batch `permissions:view` failures map to per-action `Deny`, not 403).
    let bypass_batch = json!({
        "batches": [{
            "principal": {
                "sub": "alice@example.test",
                "info": { "email": "system-admin" }
            },
            "actions": [{ "name": "get", "service": "tags" }],
            "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
        }]
    });
    let response = server
        .post("/v1beta/authorization/batch/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&bypass_batch)
        .await;
    assert_eq!(response.status_code(), 200);
    assert_eq!(
        response.json::<Value>()["decisions"][0]["tags:get"]["decision"],
        "deny",
        "batch path must also see the resolved target id at the view check"
    );

    // Batch positive control: legitimate proxy case still allowed.
    let allowed_batch = json!({
        "batches": [{
            "principal": {
                "sub": "alice@example.test",
                "info": { "email": "alice@example.test" }
            },
            "actions": [{ "name": "get", "service": "tags" }],
            "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
        }]
    });
    let response = server
        .post("/v1beta/authorization/batch/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&allowed_batch)
        .await;
    assert_eq!(response.status_code(), 200);
    assert_eq!(
        response.json::<Value>()["decisions"][0]["tags:get"]["decision"],
        "allow",
        "batch path must allow the legitimate proxy case under the same resolution"
    );
}

/// A caller looking up their own permissions through a service whose
/// `id_claim` differs from the auth `sub` is classified as a self lookup
/// once both sides are resolved through the target service's `Meta`. The
/// view check is skipped and evaluation runs against the resolved id.
#[tokio::test]
async fn test_authorization_self_lookup_via_per_service_id_claim() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let claims = Claims::default();
    let token = jwk.encode(&claims);

    let mut config = jwk.config();
    // No `permissions:view` is granted: only a self lookup can succeed.
    // The eval policy is keyed on the requester's email, which `tags`
    // selects via `id_claim`.
    config.database.init.policies_file = write_init_policies_file(
        r#"
policies:
  - policy: 'permit(principal, action == Action::"permissions:meta:write", resource);'
  - policy: 'permit(principal == Principal::"test-user@test.test", action == Action::"tags:get", resource);'
"#,
    );

    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;

    let metadata = json!({ "service": "tags", "id_claim": "email" });
    let response = server
        .put("/v1beta/services/tags/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&metadata)
        .await;
    assert_eq!(response.status_code(), 200);

    // Caller addresses themselves using the `tags` id form (their email).
    // `requester.info.email` resolves to the same value, so the request is
    // a self lookup and bypasses the (absent) `permissions:view` grant.
    let body = json!({
        "principal": { "sub": "test-user@test.test" },
        "action": { "service": "tags", "name": "get" },
        "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
    });
    let response = server
        .post("/v1beta/authorization/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .await;
    assert_eq!(response.status_code(), 200);
    assert_eq!(
        response.json::<Value>()["decision"],
        "allow",
        "self lookup must be classified after resolving both ids through the target service's id_claim"
    );

    // Omitting `principal` entirely is the canonical self-lookup shape and
    // must produce the same outcome.
    let body = json!({
        "action": { "service": "tags", "name": "get" },
        "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
    });
    let response = server
        .post("/v1beta/authorization/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .await;
    assert_eq!(response.status_code(), 200);
    assert_eq!(response.json::<Value>()["decision"], "allow");

    // Batch endpoint: same two self-lookup shapes, same outcomes.
    let payload = json!({
        "batches": [{
            "principal": { "sub": "test-user@test.test" },
            "actions": [{ "name": "get", "service": "tags" }],
            "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
        }]
    });
    let response = server
        .post("/v1beta/authorization/batch/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&payload)
        .await;
    assert_eq!(response.status_code(), 200);
    assert_eq!(
        response.json::<Value>()["decisions"][0]["tags:get"]["decision"],
        "allow",
        "batch path must classify per-service self lookups identically"
    );

    let payload = json!({
        "batches": [{
            "actions": [{ "name": "get", "service": "tags" }],
            "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
        }]
    });
    let response = server
        .post("/v1beta/authorization/batch/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&payload)
        .await;
    assert_eq!(response.status_code(), 200);
    assert_eq!(
        response.json::<Value>()["decisions"][0]["tags:get"]["decision"],
        "allow"
    );
}

/// Cross-principal lookups remain gated by `permissions:view`.
#[tokio::test]
async fn test_authorization_cross_principal_lookup_still_requires_view_permission() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let claims = Claims::default();
    let token = jwk.encode(&claims);

    // No `permissions:view` policy granted to anyone, so the requester is
    // not authorized to look up another principal's permissions.
    let mut config = jwk.config();
    config.database.init.policies_file = write_init_policies_file(
        r#"
policies:
  - policy: 'permit(principal == Principal::"system-admin", action, resource);'
"#,
    );

    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;

    let body = json!({
        "principal": { "sub": "system-admin" },
        "action": { "service": "tags", "name": "get" },
        "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
    });
    let response = server
        .post("/v1beta/authorization/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .await;
    assert_eq!(
        response.status_code(),
        403,
        "cross-principal lookup without permissions:view must be rejected"
    );
}

#[tokio::test]
async fn test_authorization_with_custom_id_claim() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let claims = Claims::default();
    let token = jwk.encode(&claims);

    let mut config = jwk.config();
    config.database.init.policies_file = write_init_policies_file(
        r#"
policies:
  - policy: 'permit(principal, action == Action::"permissions:edit", resource);'
  - policy: 'permit(principal, action == Action::"permissions:meta:write", resource);'
"#,
    );

    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;

    let mut principal = new_principal();
    principal.id = claims.email.clone();

    let policy = new_policy_payload(
        "permit",
        Some(&new_action()),
        Some(&principal),
        Some(&new_resource()),
        "",
    );
    let response = server
        .put("/v1beta/policies/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&policy)
        .await;
    assert_eq!(response.status_code(), 201);

    let body = json!({
        "action": {
            "service": "tags",
            "name": "get"
        },
        "resource": {
            "id": "Astronaut.usd",
            "type": "ResourceAddress"
        }
    });

    let response = server
        .post("/v1beta/authorization/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    assert_eq!(
        result,
        json!({
            "decision": "deny",
            "service": "tags",
            "action": "get"
        })
    );

    let metadata = json!({
        "service": "tags",
        "id_claim": "email"
    });
    let response = server
        .put("/v1beta/services/tags/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&metadata)
        .await;
    assert_eq!(response.status_code(), 200);

    let response = server
        .post("/v1beta/authorization/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    assert_eq!(
        result,
        json!({
            "decision": "allow",
            "service": "tags",
            "action": "get"
        })
    );
}

#[tokio::test]
async fn test_authorization_batch_with_custom_id_claim() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let claims = Claims::default();
    let token = jwk.encode(&claims);

    let mut config = jwk.config();
    config.database.init.policies_file = write_init_policies_file(
        r#"
policies:
  - policy: 'permit(principal, action == Action::"permissions:edit", resource);'
  - policy: 'permit(principal, action == Action::"permissions:meta:write", resource);'
"#,
    );

    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;

    let mut principal = new_principal();
    principal.id = claims.email.clone();

    let policy = new_policy_payload(
        "permit",
        Some(&new_action()),
        Some(&principal),
        Some(&new_resource()),
        "",
    );
    let response = server
        .put("/v1beta/policies/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&policy)
        .await;
    assert_eq!(response.status_code(), 201);

    let payload = json!({
        "batches": [
            {
                "actions": [
                    {"name": "get", "service": "tags"},
                    {"name": "read", "service": "storage"}
                ],
                "resource": {
                    "id": "Astronaut.usd",
                    "type": "ResourceAddress"
                }
            },
        ]
    });

    let response = server
        .post("/v1beta/authorization/batch/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&payload)
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    assert_eq!(
        result,
        json!({
            "decisions": [
                {
                    "tags:get": {
                        "decision": "deny",
                        "service": "tags",
                        "action": "get"
                    },
                    "storage:read": {
                        "decision": "deny",
                        "service": "storage",
                        "action": "read"
                    }
                }
            ]
        })
    );

    let metadata = json!({
        "service": "tags",
        "id_claim": "email"
    });
    let response = server
        .put("/v1beta/services/tags/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&metadata)
        .await;
    assert_eq!(response.status_code(), 200);

    let response = server
        .post("/v1beta/authorization/batch/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&payload)
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    assert_eq!(
        result,
        json!({
            "decisions": [
                {
                    "tags:get": {
                        "decision": "allow",
                        "service": "tags",
                        "action": "get"
                    },
                    "storage:read": {
                        "decision": "deny",
                        "service": "storage",
                        "action": "read"
                    }
                }
            ]
        })
    );
}

#[tokio::test]
async fn test_resource_encoding_in_authorization() {
    let server = setup().await;

    let mut resource = new_resource();
    resource.id = "https://example.com/user@domain/resource file.usd".to_string();

    let policy = new_policy_payload(
        "permit",
        Some(&new_action()),
        Some(&new_principal()),
        Some(&resource),
        "",
    );
    let response = server.put("/v1beta/policies/").json(&policy).await;
    assert_eq!(response.status_code(), 201);

    let response = server
        .post("/v1beta/authorization/")
        .json(&json!({
            "principal": {
                "sub": "test-user",
                "info": {}
            },
            "action": {
                "service": "tags",
                "name": "get"
            },
            "resource": {
                "id": "https://example.com/user@domain/resource file.usd",
                "type": "ResourceAddress",
                "data": {}
            },
            "context": {}
        }))
        .await;

    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    assert_eq!(
        result,
        json!({
            "decision": "allow",
            "service": "tags",
            "action": "get"
        })
    );
}

#[tokio::test]
async fn test_resource_encoding_in_batch_authorization() {
    let server = setup().await;

    let mut resource = new_resource();
    resource.id = "https://example.com/user@domain/resource file.usd".to_string();

    let policy = new_policy_payload(
        "permit",
        Some(&new_action()),
        Some(&new_principal()),
        Some(&resource),
        "",
    );
    let response = server.put("/v1beta/policies/").json(&policy).await;
    assert_eq!(response.status_code(), 201);

    let response = server
        .post("/v1beta/authorization/batch/")
        .json(&json!({
            "batches": [
                {
                    "principal": {"sub": "test-user", "email": "user@test.com", "exp": 0},
                    "actions": [
                        {"name": "get", "service": "tags"}
                    ],
                    "resource": {
                        "id": "https://example.com/user@domain/resource file.usd",
                        "type": "ResourceAddress",
                    }
                }
            ]}
        ))
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    assert_eq!(
        result,
        json!({
            "decisions": [
                {
                    "tags:get": {
                        "decision": "allow",
                        "service": "tags",
                        "action": "get"
                    },
                }
            ]
        })
    );
}

#[tokio::test]
async fn test_boundary_whitespace_resource_id_does_not_match_canonical_policy() {
    let server = setup().await;

    let canonical = "https://example.com/rc-bucket/ov-content/test/folder/";

    let mut resource = new_resource();
    resource.id = canonical.to_string();

    let policy = new_policy_payload(
        "permit",
        Some(&new_action()),
        Some(&new_principal()),
        Some(&resource),
        "",
    );
    let response = server.put("/v1beta/policies/").json(&policy).await;
    assert_eq!(response.status_code(), 201);

    let trailing_space = format!("{}{}", canonical, ' ');
    let leading_space = format!("{}{}", ' ', canonical);
    let trailing_char = format!("{}X", canonical);

    for (id, expected) in [
        (canonical, "allow"),
        (trailing_space.as_str(), "deny"),
        (leading_space.as_str(), "deny"),
        (trailing_char.as_str(), "deny"),
    ] {
        let response = server
            .post("/v1beta/authorization/")
            .json(&json!({
                "principal": {"sub": "test-user", "info": {}},
                "action": {"service": "tags", "name": "get"},
                "resource": { "id": id, "type": "ResourceAddress" },
                "context": {}
            }))
            .await;
        assert_eq!(response.status_code(), 200, "id {:?}", id);
        assert_eq!(
            response.json::<Value>()["decision"],
            json!(expected),
            "unexpected decision for id {:?}",
            id
        );
    }
}

#[tokio::test]
async fn test_resource_encoding_in_policies_get() {
    let server = setup().await;

    let mut resource = new_resource();
    resource.id = "https://example.com/user@domain/resource file.usd".to_string();

    let policy = new_policy_payload(
        "permit",
        Some(&new_action()),
        Some(&new_principal()),
        Some(&resource),
        "",
    );
    let response = server.put("/v1beta/policies/").json(&policy).await;
    assert_eq!(response.status_code(), 201);

    let response = server
        .get("/v1beta/policies/")
        .add_query_param(
            "resource",
            "ResourceAddress::\"https://example.com/user@domain/resource file.usd\"",
        )
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    let items = result.get("items").unwrap().as_array().unwrap();
    assert_eq!(items.len(), 1);

    let policy_response = &items[0];
    assert_eq!(
        policy_response
            .get("resource")
            .unwrap()
            .get("id")
            .unwrap()
            .as_str()
            .unwrap(),
        "https://example.com/user%40domain/resource%20file.usd"
    );
    assert_policy_metadata(policy_response, "");
}

#[tokio::test]
async fn test_resource_encoding_with_special_characters() {
    let server = setup().await;

    let mut resource = new_resource();
    resource.id = "https://example.com/path/with spaces & symbols@test!$%".to_string();

    let policy = new_policy_payload(
        "permit",
        Some(&new_action()),
        Some(&new_principal()),
        Some(&resource),
        "",
    );
    let response = server.put("/v1beta/policies/").json(&policy).await;
    assert_eq!(response.status_code(), 201);

    let result: Value = response.json();
    assert_eq!(
        result
            .get("resource")
            .unwrap()
            .get("id")
            .unwrap()
            .as_str()
            .unwrap(),
        "https://example.com/path/with%20spaces%20%26%20symbols%40test%21%24%25"
    );
    assert_policy_metadata(&result, "");
}

#[tokio::test]
async fn test_resource_encoding_with_single_quote_in_uri_path() {
    let server = setup_config_file_server(
        r#"
policies:
  - policy: 'permit(principal == Principal::"test-user", action == Action::"tags:get", resource == ResourceAddress::"https://example.com/path/team%27s%20asset.usd");'
"#,
        TEST_METADATA_YAML,
    )
    .await;

    let response = server
        .post("/v1beta/authorization/")
        .json(&json!({
            "principal": { "sub": "test-user" },
            "action": { "service": "tags", "name": "get" },
            "resource": {
                "id": "https://example.com/path/team's asset.usd",
                "type": "ResourceAddress"
            }
        }))
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    assert_eq!(result["decision"], "allow");
}

#[tokio::test]
async fn test_resource_encoding_preserves_userinfo_query_fragment() {
    let server = setup().await;

    let mut resource = new_resource();
    resource.id =
        "https://user@example.com:8080/path with@symbols?query=value@test#fragment@anchor"
            .to_string();

    let policy = new_policy_payload(
        "permit",
        Some(&new_action()),
        Some(&new_principal()),
        Some(&resource),
        "",
    );
    let response = server.put("/v1beta/policies/").json(&policy).await;
    assert_eq!(response.status_code(), 201);

    let result: Value = response.json();
    assert_eq!(
        result
            .get("resource")
            .unwrap()
            .get("id")
            .unwrap()
            .as_str()
            .unwrap(),
        "https://user@example.com:8080/path%20with%40symbols?query=value@test#fragment@anchor"
    );
    assert_policy_metadata(&result, "");
}

#[tokio::test]
async fn test_resource_encoding_batch_policies() {
    let server = setup().await;

    let mut batch_payloads = Vec::new();
    for i in 1..=3 {
        let mut resource = new_resource();
        resource.id = format!("https://example.com/user@domain/resource{}.usd", i);

        batch_payloads.push(json!({
            "policy": format!("permit(principal == Principal::\"test-user-{}\", action == Action::\"tags:get\", resource == ResourceAddress::\"https://example.com/user@domain/resource{}.usd\");", i, i),
            "action": {
                "service": "tags",
                "name": "get"
            },
            "principal": format!("test-user-{}", i),
            "resource": {
                "id": format!("https://example.com/user@domain/resource{}.usd", i),
                "type": "ResourceAddress"
            }
        }));
    }

    let response = server
        .put("/v1beta/policies/batch/")
        .json(&json!(batch_payloads))
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    let results = result.get("results").unwrap().as_array().unwrap();
    assert_eq!(results.len(), 3);

    for (i, policy_result) in results.iter().enumerate() {
        let expected_encoded_id =
            format!("https://example.com/user%40domain/resource{}.usd", i + 1);
        assert_eq!(
            policy_result
                .get("resource")
                .unwrap()
                .get("id")
                .unwrap()
                .as_str()
                .unwrap(),
            expected_encoded_id
        );
        assert_policy_metadata(policy_result, "");
    }
}

#[tokio::test]
async fn test_resource_encoding_non_url_strings() {
    let server = setup().await;

    let mut resource = new_resource();
    resource.id = "simple-resource-name@test".to_string();

    let policy = new_policy_payload(
        "permit",
        Some(&new_action()),
        Some(&new_principal()),
        Some(&resource),
        "",
    );
    let response = server.put("/v1beta/policies/").json(&policy).await;
    assert_eq!(response.status_code(), 201);

    let result: Value = response.json();
    assert_eq!(
        result
            .get("resource")
            .unwrap()
            .get("id")
            .unwrap()
            .as_str()
            .unwrap(),
        "simple-resource-name@test"
    );
    assert_policy_metadata(&result, "");
}

#[tokio::test]
async fn test_resource_encoding_already_encoded_urls() {
    let server = setup().await;

    let mut resource = new_resource();
    resource.id = "https://example.com/user%40domain/resource%20file.usd".to_string();

    let policy = new_policy_payload(
        "permit",
        Some(&new_action()),
        Some(&new_principal()),
        Some(&resource),
        "",
    );
    let response = server.put("/v1beta/policies/").json(&policy).await;
    assert_eq!(response.status_code(), 201);

    let result: Value = response.json();
    assert_eq!(
        result
            .get("resource")
            .unwrap()
            .get("id")
            .unwrap()
            .as_str()
            .unwrap(),
        "https://example.com/user%40domain/resource%20file.usd"
    );
    assert_policy_metadata(&result, "");
}

#[tokio::test]
async fn test_resource_encoding_unicode_characters() {
    let server = setup().await;

    let mut resource = new_resource();
    resource.id = "https://example.com/path/用户@测试/file.usd".to_string();

    let policy = new_policy_payload(
        "permit",
        Some(&new_action()),
        Some(&new_principal()),
        Some(&resource),
        "",
    );
    let response = server.put("/v1beta/policies/").json(&policy).await;
    assert_eq!(response.status_code(), 201);

    let result: Value = response.json();
    assert_eq!(
        result
            .get("resource")
            .unwrap()
            .get("id")
            .unwrap()
            .as_str()
            .unwrap(),
        "https://example.com/path/%E7%94%A8%E6%88%B7%40%E6%B5%8B%E8%AF%95/file.usd"
    );
    assert_policy_metadata(&result, "");
}

#[tokio::test]
async fn test_authorization_with_encoded_resource_matches_policy() {
    let server = setup().await;

    let mut resource = new_resource();
    resource.id = "https://example.com/user@domain.test/resource@file.usd".to_string();

    let policy = new_policy_payload(
        "permit",
        Some(&new_action()),
        Some(&new_principal()),
        Some(&resource),
        "",
    );
    let response = server.put("/v1beta/policies/").json(&policy).await;
    assert_eq!(response.status_code(), 201);

    let response = server
        .post("/v1beta/authorization/")
        .json(&json!({
            "principal": {
                "sub": "test-user",
                "info": {}
            },
            "action": {
                "service": "tags",
                "name": "get"
            },
            "resource": {
                "id": "https://example.com/user@domain.test/resource@file.usd",
                "type": "ResourceAddress",
                "data": {}
            },
            "context": {}
        }))
        .await;

    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    assert_eq!(
        result,
        json!({
            "decision": "allow",
            "service": "tags",
            "action": "get"
        })
    );
}

#[tokio::test]
async fn test_authorization_batch_with_encoded_resources() {
    let server = setup().await;

    let mut resource1 = new_resource();
    resource1.id = "https://example.com/user@domain/resource1.usd".to_string();

    let mut resource2 = new_resource();
    resource2.id = "https://example.com/user@domain/resource2.usd".to_string();

    let policy1 = new_policy_payload(
        "permit",
        Some(&new_action()),
        Some(&new_principal()),
        Some(&resource1),
        "",
    );
    let response = server.put("/v1beta/policies/").json(&policy1).await;
    assert_eq!(response.status_code(), 201);

    let policy2 = new_policy_payload(
        "forbid",
        Some(&new_action()),
        Some(&new_principal()),
        Some(&resource2),
        "",
    );
    let response = server.put("/v1beta/policies/").json(&policy2).await;
    assert_eq!(response.status_code(), 201);

    let response = server
        .post("/v1beta/authorization/batch/")
        .json(&json!({
            "batches": [
                {
                    "principal": {"sub": "test-user", "email": "user@test.com", "exp": 0},
                    "actions": [{"name": "get", "service": "tags"}],
                    "resource": {
                        "id": "https://example.com/user@domain/resource1.usd",
                        "type": "ResourceAddress",
                    }
                },
                {
                    "principal": {"sub": "test-user", "email": "user@test.com", "exp": 0},
                    "actions": [{"name": "get", "service": "tags"}],
                    "resource": {
                        "id": "https://example.com/user@domain/resource2.usd",
                        "type": "ResourceAddress",
                    }
                }
            ]
        }))
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    assert_eq!(
        result,
        json!({
            "decisions": [
                {
                    "tags:get": {
                        "decision": "allow",
                        "service": "tags",
                        "action": "get"
                    }
                },
                {
                    "tags:get": {
                        "decision": "deny",
                        "reason": "Explicit deny.",
                        "service": "tags",
                        "action": "get"
                    }
                }
            ]
        })
    );
}

#[tokio::test]
async fn test_encode_resource_id_function_with_valid_url() {
    let resource = Some(Resource {
        id: "https://example.com/user@domain/resource file.usd".to_string(),
        kind: "ResourceAddress".to_string(),
        data: None,
    });

    let result = encode_resource_id(&resource).unwrap();
    assert!(result.is_some());
    let encoded_resource = result.unwrap();
    assert_eq!(
        encoded_resource.id,
        "https://example.com/user%40domain/resource%20file.usd"
    );
    assert_eq!(encoded_resource.kind, "ResourceAddress");
    assert_eq!(encoded_resource.data, None);
}

#[tokio::test]
async fn test_encode_resource_id_prefix_encodes_only_literal_part() {
    let resource = Some(Resource {
        id: "https://example.com/My Projects/*".to_string(),
        kind: "ResourceAddress".to_string(),
        data: None,
    });

    let encoded = encode_resource_id_prefix(&resource).unwrap().unwrap();
    assert_eq!(encoded.id, "https://example.com/My%20Projects/*");
    assert_eq!(encoded.kind, "ResourceAddress");
}

#[tokio::test]
async fn test_encode_resource_id_prefix_preserves_type_wildcard() {
    let resource = Some(Resource {
        id: "*".to_string(),
        kind: "File".to_string(),
        data: None,
    });

    let encoded = encode_resource_id_prefix(&resource).unwrap().unwrap();
    assert_eq!(encoded.id, "*");
    assert_eq!(encoded.kind, "File");
}

#[tokio::test]
async fn test_get_policies_with_null_principal() {
    let server = setup().await;

    let principal = new_principal();
    let action = new_action();
    let resource = new_resource();

    let policy_with_principal = new_policy_payload(
        "permit",
        Some(&action),
        Some(&principal),
        Some(&resource),
        "",
    );
    let response = server
        .put("/v1beta/policies/")
        .json(&policy_with_principal)
        .await;
    assert_eq!(response.status_code(), 201);

    let policy_without_principal =
        new_policy_payload("permit", Some(&action), None, Some(&resource), "");
    let response = server
        .put("/v1beta/policies/")
        .json(&policy_without_principal)
        .await;
    assert_eq!(response.status_code(), 201);

    let global_policy = json!({
        "policy": "permit(principal, action, resource);",
        "action": null,
        "principal": null,
        "resource": null
    });
    let response = server.put("/v1beta/policies/").json(&global_policy).await;
    assert_eq!(response.status_code(), 201);

    let response = server
        .get("/v1beta/policies/")
        .add_query_param("principal", "NULL")
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    let items = result.get("items").unwrap().as_array().unwrap();
    assert_eq!(items.len(), 2);

    for item in items {
        assert!(item.get("principal").unwrap().is_null());
        assert_policy_metadata(item, "");
    }
}

#[tokio::test]
async fn test_get_policies_with_null_action() {
    let server = setup().await;

    let principal = new_principal();
    let action = new_action();
    let resource = new_resource();

    let policy_with_action = new_policy_payload(
        "permit",
        Some(&action),
        Some(&principal),
        Some(&resource),
        "",
    );
    let response = server
        .put("/v1beta/policies/")
        .json(&policy_with_action)
        .await;
    assert_eq!(response.status_code(), 201);

    let policy_without_action =
        new_policy_payload("permit", None, Some(&principal), Some(&resource), "");
    let response = server
        .put("/v1beta/policies/")
        .json(&policy_without_action)
        .await;
    assert_eq!(response.status_code(), 201);

    let global_policy = new_policy_payload("permit", None, None, None, "");
    let response = server.put("/v1beta/policies/").json(&global_policy).await;
    assert_eq!(response.status_code(), 201);

    let response = server
        .get("/v1beta/policies/")
        .add_query_param("action", "NULL")
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    let items = result.get("items").unwrap().as_array().unwrap();
    assert_eq!(items.len(), 2);

    for item in items {
        assert!(item.get("action").unwrap().is_null());
        assert_policy_metadata(item, "");
    }
}

#[tokio::test]
async fn test_get_policies_with_null_resource() {
    let server = setup().await;

    let principal = new_principal();
    let action = new_action();
    let resource = new_resource();

    let policy_with_resource = new_policy_payload(
        "permit",
        Some(&action),
        Some(&principal),
        Some(&resource),
        "",
    );
    let response = server
        .put("/v1beta/policies/")
        .json(&policy_with_resource)
        .await;
    assert_eq!(response.status_code(), 201);

    let policy_without_resource =
        new_policy_payload("permit", Some(&action), Some(&principal), None, "");
    let response = server
        .put("/v1beta/policies/")
        .json(&policy_without_resource)
        .await;
    assert_eq!(response.status_code(), 201);

    let global_policy = new_policy_payload("permit", None, None, None, "");
    let response = server.put("/v1beta/policies/").json(&global_policy).await;
    assert_eq!(response.status_code(), 201);

    let response = server
        .get("/v1beta/policies/")
        .add_query_param("resource", "NULL")
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    let items = result.get("items").unwrap().as_array().unwrap();
    assert_eq!(items.len(), 2);

    for item in items {
        assert!(item.get("resource").unwrap().is_null());
        assert_policy_metadata(item, "");
    }
}

#[tokio::test]
async fn test_get_policies_with_multiple_null_parameters() {
    let server = setup().await;

    let principal = new_principal();
    let action = new_action();
    let resource = new_resource();

    let specific_policy = new_policy_payload(
        "permit",
        Some(&action),
        Some(&principal),
        Some(&resource),
        "",
    );
    let response = server.put("/v1beta/policies/").json(&specific_policy).await;
    assert_eq!(response.status_code(), 201);

    let policy_without_principal =
        new_policy_payload("permit", Some(&action), None, Some(&resource), "");
    let response = server
        .put("/v1beta/policies/")
        .json(&policy_without_principal)
        .await;
    assert_eq!(response.status_code(), 201);

    let policy_without_action =
        new_policy_payload("permit", None, Some(&principal), Some(&resource), "");
    let response = server
        .put("/v1beta/policies/")
        .json(&policy_without_action)
        .await;
    assert_eq!(response.status_code(), 201);

    let global_policy = new_policy_payload("permit", None, None, None, "");
    let response = server.put("/v1beta/policies/").json(&global_policy).await;
    assert_eq!(response.status_code(), 201);

    let response = server
        .get("/v1beta/policies/")
        .add_query_param("principal", "NULL")
        .add_query_param("action", "NULL")
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    let items = result.get("items").unwrap().as_array().unwrap();
    assert_eq!(items.len(), 1);

    let item = &items[0];
    assert!(item.get("principal").unwrap().is_null());
    assert!(item.get("action").unwrap().is_null());
    assert_policy_metadata(item, "");
}

#[tokio::test]
async fn test_get_policies_with_null_parameters_mixed_with_regular_filters() {
    let server = setup().await;

    let principal = new_principal();
    let action = new_action();
    let resource = new_resource();

    let specific_policy = new_policy_payload(
        "permit",
        Some(&action),
        Some(&principal),
        Some(&resource),
        "",
    );
    let response = server.put("/v1beta/policies/").json(&specific_policy).await;
    assert_eq!(response.status_code(), 201);

    let policy_without_principal =
        new_policy_payload("permit", Some(&action), None, Some(&resource), "");
    let response = server
        .put("/v1beta/policies/")
        .json(&policy_without_principal)
        .await;
    assert_eq!(response.status_code(), 201);

    let mut other_action = new_action();
    other_action.name = "write".to_string();
    let policy_without_principal_different_action =
        new_policy_payload("permit", Some(&other_action), None, Some(&resource), "");
    let response = server
        .put("/v1beta/policies/")
        .json(&policy_without_principal_different_action)
        .await;
    assert_eq!(response.status_code(), 201);

    let response = server
        .get("/v1beta/policies/")
        .add_query_param("principal", "NULL")
        .add_query_param("action", "tags:get")
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    let items = result.get("items").unwrap().as_array().unwrap();
    assert_eq!(items.len(), 1);

    let item = &items[0];
    assert!(item.get("principal").unwrap().is_null());
    assert_eq!(
        item.get("action")
            .unwrap()
            .get("name")
            .unwrap()
            .as_str()
            .unwrap(),
        "get"
    );
    assert_eq!(
        item.get("action")
            .unwrap()
            .get("service")
            .unwrap()
            .as_str()
            .unwrap(),
        "tags"
    );
    assert_policy_metadata(item, "");
}

#[tokio::test]
async fn test_put_policy_rejects_principal_with_invalid_characters() {
    let server = setup().await;
    let response = server
        .put("/v1beta/policies/")
        .json(&json!({
            "policy": "permit(principal, action, resource);",
            "principal": "user'\"\\name"
        }))
        .await;
    assert_eq!(response.status_code(), 201);
    let result: Value = response.json();
    assert!(result["principal"].is_null());
    assert!(result["action"].is_null());
    assert!(result["resource"].is_null());
    assert_policy_metadata(&result, "");
}

#[tokio::test]
async fn test_put_policy_batch_rejects_principal_with_invalid_characters() {
    let server = setup().await;
    let response = server
        .put("/v1beta/policies/batch/")
        .json(&json!([
            {
                "policy": "permit(principal, action, resource);",
                "principal": "user'\"\\name"
            }
        ]))
        .await;
    assert_eq!(response.status_code(), 200);
    let result: Value = response.json();
    let items = result["results"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert!(items[0]["principal"].is_null());
    assert!(items[0]["action"].is_null());
    assert!(items[0]["resource"].is_null());
    assert_policy_metadata(&items[0], "");
}

#[tokio::test]
async fn test_put_policy_accepts_policy_only_payload_and_infers_scopes() {
    let server = setup().await;
    let response = server
        .put("/v1beta/policies/")
        .json(&json!({
            "policy": "permit(principal == Principal::\"test-user\", action == Action::\"tags:get\", resource == ResourceAddress::\"Astronaut.usd\");"
        }))
        .await;
    assert_eq!(response.status_code(), 201);
    let result: Value = response.json();
    assert_eq!(result["principal"]["sub"], "test-user");
    assert_eq!(result["action"]["service"], "tags");
    assert_eq!(result["action"]["name"], "get");
    assert_eq!(result["resource"]["id"], "Astronaut.usd");
    assert_eq!(result["resource"]["type"], "ResourceAddress");
    assert_policy_metadata(&result, "");
}

#[tokio::test]
async fn test_put_policy_batch_accepts_policy_only_payload_and_infers_scopes() {
    let server = setup().await;
    let response = server
        .put("/v1beta/policies/batch/")
        .json(&json!([
            {
                "policy": "permit(principal == Principal::\"test-user-1\", action == Action::\"tags:get\", resource == ResourceAddress::\"Astronaut.usd\");"
            },
            {
                "policy": "permit(principal == Principal::\"test-user-2\", action == Action::\"tags:get\", resource == ResourceAddress::\"Astronaut.usd\");"
            }
        ]))
        .await;
    assert_eq!(response.status_code(), 200);
    let result: Value = response.json();
    let items = result["results"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["principal"]["sub"], "test-user-1");
    assert_eq!(items[1]["principal"]["sub"], "test-user-2");
    assert_policy_metadata(&items[0], "");
    assert_policy_metadata(&items[1], "");
}

#[tokio::test]
async fn test_put_policy_rejects_multiple_cedar_statements() {
    let server = setup().await;
    let response = server
        .put("/v1beta/policies/")
        .json(&json!({
            "policy": "permit(principal, action, resource); forbid(principal, action, resource);"
        }))
        .await;
    assert_eq!(response.status_code(), 400);
    assert!(response
        .text()
        .contains("Expected exactly one Cedar policy"));
}

#[tokio::test]
async fn test_put_policy_batch_rejects_multiple_cedar_statements() {
    let server = setup().await;
    let response = server
        .put("/v1beta/policies/batch/")
        .json(&json!([
            {
                "policy": "permit(principal, action, resource); forbid(principal, action, resource);"
            }
        ]))
        .await;
    assert_eq!(response.status_code(), 400);
    assert!(response
        .text()
        .contains("Expected exactly one Cedar policy"));
}

#[test]
fn test_put_policy_batch_request_garde_rules() {
    use garde::Validate;
    use permission_service::http::policies::{PutPolicyBatchRequest, PutPolicyPayload};

    let make_item = |policy: &str| PutPolicyPayload {
        policy: policy.to_string(),
        order: None,
        comment: None,
    };

    // Empty batch is allowed.
    PutPolicyBatchRequest(Vec::new()).validate().unwrap();

    // Exactly 100 items is the boundary and must pass.
    let ok_payload =
        "permit(principal == Principal::\"u\", action == Action::\"a:b\", resource == R::\"r\");";
    PutPolicyBatchRequest((0..100).map(|_| make_item(ok_payload)).collect())
        .validate()
        .unwrap();

    // 101 items must fail with a length violation.
    let err = PutPolicyBatchRequest((0..101).map(|_| make_item(ok_payload)).collect())
        .validate()
        .expect_err("101 items should fail size validation");
    let text = err.to_string();
    assert!(
        text.contains("100"),
        "expected size-limit error to mention 100, got: {text}"
    );

    // dive: an oversized policy string inside a small batch must still be rejected.
    let oversized = "x".repeat(70_000);
    let err = PutPolicyBatchRequest(vec![make_item(&oversized)])
        .validate()
        .expect_err("oversized policy should fail per-item validation");
    let text = err.to_string();
    assert!(
        text.contains("65535"),
        "expected per-item length error, got: {text}"
    );
}

#[tokio::test]
async fn test_put_policy_batch_rejects_batch_over_size_limit() {
    // The size limit is enforced before any backend call, so the cheap
    // config-file server is enough to exercise it.
    let server = setup_config_file_server(TEST_POLICIES_YAML, TEST_METADATA_YAML).await;
    let batches: Vec<Value> = (0..101)
        .map(|i| {
            json!({
                "policy": format!(
                    "permit(principal == Principal::\"size-limit-user-{i}\", action == Action::\"tags:get\", resource == ResourceAddress::\"Astronaut.usd\");"
                )
            })
        })
        .collect();
    let response = server
        .put("/v1beta/policies/batch/")
        .json(&json!(batches))
        .await;
    assert_eq!(response.status_code(), 422);
    let body = response.text();
    assert!(
        body.contains("100"),
        "expected error body to mention the limit, got: {body}"
    );
}

#[tokio::test]
async fn test_postgres_init_policies_infer_scopes_from_policy_text() {
    let mut config = setup_config();
    config.database.init.policies_file = write_init_policies_file(
        r#"
policies:
  - policy: 'permit(principal == Principal::"test-user", action == Action::"tags:get", resource == ResourceAddress::"Astronaut.usd");'
"#,
    );

    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;

    let response = server
        .get("/v1beta/policies/")
        .add_query_param("action", "tags:get")
        .add_query_param("principal", "test-user")
        .add_query_param("resource", "ResourceAddress::Astronaut.usd")
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    let items = result["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["action"]["service"], "tags");
    assert_eq!(items[0]["action"]["name"], "get");
    assert_eq!(items[0]["principal"]["sub"], "test-user");
    assert_eq!(items[0]["resource"]["id"], "Astronaut.usd");
    assert_eq!(items[0]["resource"]["type"], "ResourceAddress");
    assert_policy_metadata(&items[0], "SYSTEM");
}

const TEST_POLICIES_YAML: &str = r#"
policies:
  - policy: 'permit(principal == Principal::"test-user", action == Action::"tags:get", resource == ResourceAddress::"Astronaut.usd");'
  - policy: 'permit(principal, action == Action::"tags:list", resource);'
  - policy: 'permit(principal, action, resource);'
"#;

const TEST_POLICIES_NO_GLOBAL_YAML: &str = r#"
policies:
  - policy: 'permit(principal == Principal::"test-user", action == Action::"tags:get", resource == ResourceAddress::"Astronaut.usd");'
  - policy: 'permit(principal, action == Action::"tags:list", resource);'
"#;

const TEST_METADATA_YAML: &str = r#"
services:
  - name: "tags"
    principal:
      idClaim: "sub"
    actions:
      - "get"
      - "list"
    resource_types:
      - type: "ResourceAddress"
        evaluation_priority: "forbid"
"#;

const TEST_METADATA_CAMEL_YAML: &str = r#"
services:
  - name: "tags"
    principal:
      idClaim: "sub"
    actions:
      - "get"
      - "list"
    resourceTypes:
      - type: "ResourceAddress"
        evaluationPriority: "forbid"
"#;

const TEST_METADATA_EMAIL_CLAIM_YAML: &str = r#"
services:
  - name: "tags"
    principal:
      idClaim: "email"
    actions:
      - "get"
      - "list"
    resourceTypes:
      - type: "ResourceAddress"
        evaluationPriority: "forbid"
"#;

const TEST_METADATA_MULTI_SERVICE_NO_PRINCIPAL_YAML: &str = r#"
services:
  - name: "storage-service"
    actions:
      - "read"
      - "write"
    resourceTypes:
      - type: "object"
        evaluationPriority: "permit"
      - type: "folder"
        evaluationPriority: "permit"

  - name: "event-consumer-service"
    actions:
      - "consume-all-storage-create-events"
      - "consume-all-storage-delete-events"
      - "consume-durable-queues"
      - "create-durable-queues"
      - "delete-durable-queues"

  - name: "event-aggregation-service"
    actions:
      - "publish-event"
    resourceTypes:
      - type: "EventType"
"#;

#[tokio::test]
async fn test_config_file_authorization() {
    let server = setup_config_file_server(TEST_POLICIES_YAML, TEST_METADATA_YAML).await;

    let response = server
        .post("/v1beta/authorization/")
        .json(&json!({
            "principal": { "sub": "test-user" },
            "action": { "service": "tags", "name": "get" },
            "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
        }))
        .await;
    assert_eq!(response.status_code(), 200);
    let result: Value = response.json();
    assert_eq!(result["decision"], "allow");
}

#[tokio::test]
async fn test_config_file_authorization_with_camel_case_metadata() {
    let server = setup_config_file_server(TEST_POLICIES_YAML, TEST_METADATA_CAMEL_YAML).await;

    let response = server
        .post("/v1beta/authorization/")
        .json(&json!({
            "principal": { "sub": "test-user" },
            "action": { "service": "tags", "name": "get" },
            "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
        }))
        .await;
    assert_eq!(response.status_code(), 200);
    let result: Value = response.json();
    assert_eq!(result["decision"], "allow");
}

#[tokio::test]
async fn test_config_file_authorization_with_metadata_principal_id_claim() {
    let server = setup_config_file_server(TEST_POLICIES_YAML, TEST_METADATA_EMAIL_CLAIM_YAML).await;

    let response = server
        .post("/v1beta/authorization/")
        .json(&json!({
            "principal": { "sub": "wrong-user", "email": "test-user" },
            "action": { "service": "tags", "name": "get" },
            "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
        }))
        .await;
    assert_eq!(response.status_code(), 200);
    let result: Value = response.json();
    assert_eq!(result["decision"], "allow");
}

#[tokio::test]
async fn test_config_file_authorization_deny() {
    let server = setup_config_file_server(TEST_POLICIES_NO_GLOBAL_YAML, TEST_METADATA_YAML).await;

    let response = server
        .post("/v1beta/authorization/")
        .json(&json!({
            "principal": { "sub": "other-user" },
            "action": { "service": "tags", "name": "get" },
            "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
        }))
        .await;
    assert_eq!(response.status_code(), 200);
    let result: Value = response.json();
    assert_eq!(result["decision"], "deny");
}

#[tokio::test]
async fn test_config_file_authorization_global_policy() {
    let server = setup_config_file_server(TEST_POLICIES_YAML, TEST_METADATA_YAML).await;

    let response = server
        .post("/v1beta/authorization/")
        .json(&json!({
            "principal": { "sub": "any-user" },
            "action": { "service": "tags", "name": "list" },
            "resource": { "id": "file.usd", "type": "ResourceAddress" }
        }))
        .await;
    assert_eq!(response.status_code(), 200);
    let result: Value = response.json();
    assert_eq!(result["decision"], "allow");
}

const HIERARCHY_POLICIES_YAML: &str = r#"
policies:
  - policy: 'permit(principal == Principal::"alice", action == Action::"storage:read", resource in Folder::"https://main/docs/");'
"#;

const HIERARCHY_METADATA_YAML: &str = r#"
services:
  - name: "storage"
    principal:
      idClaim: "sub"
    actions:
      - "read"
      - "write"
    resource_types:
      - type: "object"
        evaluation_priority: "forbid"
        parentType: "Folder"
      - type: "Folder"
        evaluation_priority: "forbid"
"#;

fn hierarchy_batch_payload(condition: &str, resource_ids: &[&str]) -> Value {
    let batches: Vec<Value> = resource_ids
        .iter()
        .map(|id| {
            json!({
                "principal": { "sub": "alice" },
                "actions": [{ "name": "read", "service": "storage" }],
                "resource": { "id": id, "type": "object" }
            })
        })
        .collect();
    json!({ "condition": condition, "batches": batches })
}

#[tokio::test]
async fn test_authorization_batch_in_scope_matches_descendant_and_denies_sibling() {
    let server = setup_config_file_server(HIERARCHY_POLICIES_YAML, HIERARCHY_METADATA_YAML).await;

    let response = server
        .post("/v1beta/authorization/batch/")
        .json(&hierarchy_batch_payload(
            "and",
            &[
                "https://main/docs/reports/q1.pdf",
                "https://main/other/q1.pdf",
            ],
        ))
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    assert_eq!(result["summary"]["decision"], "deny");
    assert_eq!(result["decisions"][0]["storage:read"]["decision"], "allow");
    assert_eq!(result["decisions"][1]["storage:read"]["decision"], "deny");
}

#[tokio::test]
async fn test_authorization_batch_in_scope_is_inclusive_of_exact_node() {
    let server = setup_config_file_server(HIERARCHY_POLICIES_YAML, HIERARCHY_METADATA_YAML).await;

    let response = server
        .post("/v1beta/authorization/batch/")
        .json(&hierarchy_batch_payload("and", &["https://main/docs/"]))
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    assert_eq!(result["summary"]["decision"], "allow");
    assert_eq!(result["decisions"][0]["storage:read"]["decision"], "allow");
}

#[tokio::test]
async fn test_authorization_batch_in_scope_denies_parent_of_prefix() {
    let server = setup_config_file_server(HIERARCHY_POLICIES_YAML, HIERARCHY_METADATA_YAML).await;

    let response = server
        .post("/v1beta/authorization/batch/")
        .json(&hierarchy_batch_payload("none", &["https://main/"]))
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    assert_eq!(result["decisions"][0]["storage:read"]["decision"], "deny");
}

#[tokio::test]
async fn test_authorization_in_scope_cross_type_file_in_folder() {
    let server = setup_config_file_server(HIERARCHY_POLICIES_YAML, HIERARCHY_METADATA_YAML).await;

    let response = server
        .post("/v1beta/authorization/")
        .json(&json!({
            "principal": { "sub": "alice" },
            "action": { "service": "storage", "name": "read" },
            "resource": { "id": "https://main/docs/report.obj", "type": "object" }
        }))
        .await;
    assert_eq!(response.status_code(), 200);
    let result: Value = response.json();
    assert_eq!(result["decision"], "allow");
}

#[tokio::test]
async fn test_get_policies_scope_inherited_returns_hierarchy_policy_for_descendant() {
    let server = setup_config_file_server(HIERARCHY_POLICIES_YAML, HIERARCHY_METADATA_YAML).await;

    let response = server
        .get("/v1beta/policies/")
        .add_query_param("action", "storage:read")
        .add_query_param("resource", "object::https://main/docs/reports/q1.pdf")
        .add_query_param("scope", "inherited")
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    let items = result["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0]["resource_prefix"],
        json!({ "id": "https://main/docs/", "type": "Folder", "data": null })
    );
}

#[tokio::test]
async fn test_get_policies_scope_inherited_returns_hierarchy_policy_without_action() {
    let server = setup_config_file_server(HIERARCHY_POLICIES_YAML, HIERARCHY_METADATA_YAML).await;

    let response = server
        .get("/v1beta/policies/")
        .add_query_param("resource", "object::https://main/docs/reports/q1.pdf")
        .add_query_param("scope", "inherited")
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    let items = result["items"].as_array().unwrap();
    assert_eq!(
        items.len(),
        1,
        "folder hierarchy policy must be returned for an object resource even without an action: {result}"
    );
    assert_eq!(
        items[0]["resource_prefix"],
        json!({ "id": "https://main/docs/", "type": "Folder", "data": null })
    );
}

#[tokio::test]
async fn test_get_policies_scope_inherited_is_inclusive_of_exact_prefix() {
    let server = setup_config_file_server(HIERARCHY_POLICIES_YAML, HIERARCHY_METADATA_YAML).await;

    let response = server
        .get("/v1beta/policies/")
        .add_query_param("action", "storage:read")
        .add_query_param("resource", "object::https://main/docs/")
        .add_query_param("scope", "inherited")
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    let items = result["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
}

#[tokio::test]
async fn test_get_policies_scope_exact_excludes_ancestor_hierarchy_policy() {
    let server = setup_config_file_server(HIERARCHY_POLICIES_YAML, HIERARCHY_METADATA_YAML).await;

    let response = server
        .get("/v1beta/policies/")
        .add_query_param("action", "storage:read")
        .add_query_param("resource", "object::https://main/docs/reports/q1.pdf")
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    let items = result["items"].as_array().unwrap();
    assert!(
        items.is_empty(),
        "default (exact) scope must not walk up to ancestor hierarchy policies: {result}"
    );
}

#[tokio::test]
async fn test_put_policy_rejects_invalid_hierarchy_url() {
    let server = setup_config_file_server(HIERARCHY_POLICIES_YAML, HIERARCHY_METADATA_YAML).await;

    let response = server
        .put("/v1beta/policies/")
        .json(&json!({
            "policy": "permit(principal, action, resource in Folder::\"not-a-url\");"
        }))
        .await;
    assert_eq!(response.status_code(), 400);
}

const OMNIVERSE_POLICIES_YAML: &str = r#"
policies:
  - policy: 'permit(principal, action, resource in folder::"omniverse://server.example.com/abc/");'
"#;

const OMNIVERSE_METADATA_YAML: &str = r#"
services:
  - name: "storage"
    principal:
      idClaim: "sub"
    actions:
      - "read"
    resource_types:
      - type: "object"
        evaluation_priority: "forbid"
        parentType: "folder"
      - type: "folder"
        evaluation_priority: "forbid"
"#;

#[tokio::test]
async fn test_authorization_batch_in_scope_non_special_scheme_descendant() {
    let server = setup_config_file_server(OMNIVERSE_POLICIES_YAML, OMNIVERSE_METADATA_YAML).await;
    let response = server
        .post("/v1beta/authorization/batch/")
        .json(&json!({
            "condition": null,
            "batches": [{
                "actions": [{ "name": "read", "service": "storage" }],
                "resource": { "id": "omniverse://server.example.com/abc/test.obj", "type": "object", "data": null }
            }]
        }))
        .await;
    assert_eq!(response.status_code(), 200);
    let result: Value = response.json();
    assert_eq!(result["decisions"][0]["storage:read"]["decision"], "allow");
}

#[tokio::test]
async fn test_authorization_batch_in_scope_same_type_exact_node_no_cycle() {
    let server = setup_config_file_server(OMNIVERSE_POLICIES_YAML, OMNIVERSE_METADATA_YAML).await;
    let response = server
        .post("/v1beta/authorization/batch/")
        .json(&json!({
            "condition": null,
            "batches": [{
                "actions": [{ "name": "read", "service": "storage" }],
                "resource": { "id": "omniverse://server.example.com/abc/", "type": "folder", "data": null }
            }]
        }))
        .await;
    assert_eq!(response.status_code(), 200);
    let result: Value = response.json();
    let decision = &result["decisions"][0]["storage:read"];
    assert_eq!(
        decision["decision"], "allow",
        "exact hierarchy node must match without a self-referential parent cycle: {result}"
    );
    assert!(
        decision["reason"].is_null(),
        "no evaluation error expected, got: {result}"
    );
}

#[tokio::test]
async fn test_config_file_get_policies() {
    let server = setup_config_file_server(TEST_POLICIES_YAML, TEST_METADATA_YAML).await;

    let response = server.get("/v1beta/policies/").await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    let items = result["items"].as_array().unwrap();
    assert_eq!(items.len(), 3);
}

#[tokio::test]
async fn test_config_file_get_policies_filtered() {
    let server = setup_config_file_server(TEST_POLICIES_YAML, TEST_METADATA_YAML).await;

    let response = server
        .get("/v1beta/policies/")
        .add_query_param("action", "tags:get")
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    let items = result["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
}

#[tokio::test]
async fn test_config_file_get_policies_filtered_by_order() {
    let policies_yaml = r#"
policies:
  - policy: 'permit(principal == Principal::"test-user", action == Action::"tags:get", resource == ResourceAddress::"Astronaut.usd");'
    order: 10
  - policy: 'permit(principal, action == Action::"tags:list", resource);'
    order: 20
  - policy: 'permit(principal, action, resource);'
    order: 20
"#;
    let server = setup_config_file_server(policies_yaml, TEST_METADATA_YAML).await;

    let response = server
        .get("/v1beta/policies/")
        .add_query_param("order", "20")
        .await;
    assert_eq!(response.status_code(), 200);
    let result: Value = response.json();
    let items = result["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert!(items
        .iter()
        .all(|item| item["order"].as_i64().unwrap() == 20));

    let response = server
        .get("/v1beta/policies/")
        .add_query_param("order", "10")
        .await;
    assert_eq!(response.status_code(), 200);
    let result: Value = response.json();
    let items = result["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["order"].as_i64().unwrap(), 10);
}

#[tokio::test]
async fn test_config_file_get_policies_null_action() {
    let server = setup_config_file_server(TEST_POLICIES_YAML, TEST_METADATA_YAML).await;

    let response = server
        .get("/v1beta/policies/")
        .add_query_param("action", "NULL")
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    let items = result["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert!(items[0]["action"].is_null());
}

#[tokio::test]
async fn test_config_file_get_policy_by_id() {
    let server = setup_config_file_server(TEST_POLICIES_YAML, TEST_METADATA_YAML).await;

    let response = server.get("/v1beta/policies/1").await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    assert_eq!(result["id"], 1);
    assert!(result["action"].is_object());
}

#[tokio::test]
async fn test_config_file_put_policy_rejected() {
    let server = setup_config_file_server(TEST_POLICIES_YAML, TEST_METADATA_YAML).await;

    let response = server
        .put("/v1beta/policies/")
        .json(&get_default_policy_payload())
        .await;
    assert_eq!(response.status_code(), 501);
    assert!(response.text().contains("not supported"));
}

#[tokio::test]
async fn test_config_file_delete_policy_rejected() {
    let server = setup_config_file_server(TEST_POLICIES_YAML, TEST_METADATA_YAML).await;

    let response = server.delete("/v1beta/policies/1").await;
    assert_eq!(response.status_code(), 501);
    assert!(response.text().contains("not supported"));
}

#[tokio::test]
async fn test_config_file_put_policy_batch_rejected() {
    let server = setup_config_file_server(TEST_POLICIES_YAML, TEST_METADATA_YAML).await;

    let response = server
        .put("/v1beta/policies/batch/")
        .json(&json!(get_policy_batch_payload()))
        .await;
    assert_eq!(response.status_code(), 501);
    assert!(response.text().contains("not supported"));
}

#[tokio::test]
async fn test_get_service_metadata_with_registered_service_default_config_id_claim() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let claims = Claims::default();
    let token = jwk.encode(&claims);

    let mut config = jwk.config();
    config.auth.principal_id_claim = "email".to_string();
    config.database.init.policies_file = write_init_policies_file(
        r#"
policies:
  - policy: 'permit(principal, action == Action::"permissions:edit", resource);'
  - policy: 'permit(principal, action == Action::"permissions:meta:read", resource);'
"#,
    );

    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;

    let response = server
        .get("/v1beta/services/tags/")
        .add_header("Authorization", format!("Bearer {token}"))
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    assert_eq!(
        result,
        json!({
            "service": "tags",
            "id_claim": "email"
        })
    );
}

#[tokio::test]
async fn test_metadata_read_action_allows_get_but_not_write() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let claims = Claims::default();
    let token = jwk.encode(&claims);

    let mut config = jwk.config();
    config.database.init.policies_file = write_init_policies_file(
        r#"
policies:
  - policy: 'permit(principal, action == Action::"permissions:meta:read", resource);'
"#,
    );

    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;

    let response = server
        .get("/v1beta/services/tags/")
        .add_header("Authorization", format!("Bearer {token}"))
        .await;
    assert_eq!(response.status_code(), 200);

    let response = server
        .put("/v1beta/services/tags/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&json!({ "service": "tags", "id_claim": "email" }))
        .await;
    assert_eq!(response.status_code(), 403);
}

#[tokio::test]
async fn test_metadata_write_action_allows_write_but_not_get() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let claims = Claims::default();
    let token = jwk.encode(&claims);

    let mut config = jwk.config();
    config.database.init.policies_file = write_init_policies_file(
        r#"
policies:
  - policy: 'permit(principal, action == Action::"permissions:meta:write", resource);'
"#,
    );

    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;

    let response = server
        .put("/v1beta/services/tags/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&json!({ "service": "tags", "id_claim": "email" }))
        .await;
    assert_eq!(response.status_code(), 200);

    let response = server
        .get("/v1beta/services/tags/")
        .add_header("Authorization", format!("Bearer {token}"))
        .await;
    assert_eq!(response.status_code(), 403);
}

#[tokio::test]
async fn test_metadata_without_meta_actions_denies_get_and_write() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let claims = Claims::default();
    let token = jwk.encode(&claims);

    let mut config = jwk.config();
    config.database.init.policies_file = write_init_policies_file(
        r#"
policies: []
"#,
    );

    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;

    let response = server
        .get("/v1beta/services/tags/")
        .add_header("Authorization", format!("Bearer {token}"))
        .await;
    assert_eq!(response.status_code(), 403);

    let response = server
        .put("/v1beta/services/tags/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&json!({ "service": "tags", "id_claim": "email" }))
        .await;
    assert_eq!(response.status_code(), 403);
}

#[tokio::test]
async fn test_config_file_get_services() {
    let server = setup_config_file_server(TEST_POLICIES_YAML, TEST_METADATA_YAML).await;

    let response = server.get("/v1beta/services/").await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    let items = result.as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["service"], "tags");
}

#[tokio::test]
async fn test_config_file_get_services_uses_config_default_id_claim_for_missing_principal() {
    let server = setup_config_file_server_with_principal_id_claim(
        TEST_POLICIES_YAML,
        TEST_METADATA_MULTI_SERVICE_NO_PRINCIPAL_YAML,
        "email",
    )
    .await;

    let response = server.get("/v1beta/services/").await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    let items = result.as_array().unwrap();
    assert_eq!(items.len(), 3);

    assert!(items
        .iter()
        .any(|item| { item["service"] == "storage-service" && item["id_claim"] == "email" }));
    assert!(items.iter().any(|item| {
        item["service"] == "event-consumer-service" && item["id_claim"] == "email"
    }));
    assert!(items.iter().any(|item| {
        item["service"] == "event-aggregation-service" && item["id_claim"] == "email"
    }));
}

#[tokio::test]
async fn test_config_file_get_service_actions() {
    let server = setup_config_file_server(TEST_POLICIES_YAML, TEST_METADATA_YAML).await;

    let response = server.get("/v1beta/services/tags/actions/").await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    let items = result.as_array().unwrap();
    assert_eq!(items.len(), 2);
}

#[tokio::test]
async fn test_config_file_get_resource_types() {
    let server = setup_config_file_server(TEST_POLICIES_YAML, TEST_METADATA_YAML).await;

    let response = server.get("/v1beta/services/tags/resource-types/").await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    let items = result.as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["type"], "ResourceAddress");
}

#[tokio::test]
async fn test_config_file_set_metadata_rejected() {
    let server = setup_config_file_server(TEST_POLICIES_YAML, TEST_METADATA_YAML).await;

    let response = server
        .put("/v1beta/services/tags/resource-types/ResourceAddress/")
        .json(&json!({
            "service": "tags",
            "type": "ResourceAddress",
            "evaluation_priority": "forbid"
        }))
        .await;
    assert_eq!(response.status_code(), 501);
}

#[tokio::test]
async fn test_config_file_authorization_batch() {
    let server = setup_config_file_server(TEST_POLICIES_NO_GLOBAL_YAML, TEST_METADATA_YAML).await;

    let response = server
        .post("/v1beta/authorization/batch/")
        .json(&json!({
            "batches": [
                {
                    "principal": {"sub": "test-user"},
                    "actions": [{"name": "get", "service": "tags"}],
                    "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
                },
                {
                    "principal": {"sub": "other-user"},
                    "actions": [{"name": "get", "service": "tags"}],
                    "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
                }
            ]
        }))
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    let decisions = result["decisions"].as_array().unwrap();
    assert_eq!(decisions.len(), 2);
    assert_eq!(decisions[0]["tags:get"]["decision"], "allow");
    assert_eq!(decisions[1]["tags:get"]["decision"], "deny");
}

#[tokio::test]
async fn test_config_file_infers_exact_scopes() {
    let policies = r#"
policies:
  - policy: 'permit(principal == Principal::"test-user", action == Action::"tags:get", resource == ResourceAddress::"Astronaut.usd");'
"#;
    let server = setup_config_file_server(policies, TEST_METADATA_YAML).await;

    let response = server.get("/v1beta/policies/1").await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    assert_eq!(result["principal"]["sub"], "test-user");
    assert_eq!(result["action"]["service"], "tags");
    assert_eq!(result["action"]["name"], "get");
    assert_eq!(result["resource"]["id"], "Astronaut.usd");
    assert_eq!(result["resource"]["type"], "ResourceAddress");
}

#[tokio::test]
async fn test_config_file_global_policy_has_null_scopes() {
    let policies = r#"
policies:
  - policy: 'permit(principal, action, resource);'
"#;
    let server = setup_config_file_server(policies, TEST_METADATA_YAML).await;

    let response = server.get("/v1beta/policies/1").await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    assert!(result["principal"].is_null());
    assert!(result["action"].is_null());
    assert!(result["resource"].is_null());
}

#[tokio::test]
async fn test_config_file_partial_scope_inference() {
    let policies = r#"
policies:
  - policy: 'permit(principal == Principal::"test-user", action, resource);'
"#;
    let server = setup_config_file_server(policies, TEST_METADATA_YAML).await;

    let response = server.get("/v1beta/policies/1").await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    assert_eq!(result["principal"]["sub"], "test-user");
    assert!(result["action"].is_null());
    assert!(result["resource"].is_null());
}

fn new_policy_payload_with_order(
    effect: &str,
    action: Option<&Action>,
    principal: Option<&Principal>,
    resource: Option<&Resource>,
    conditions: &str,
    order: i32,
) -> Value {
    let principal_scope = principal.map_or("".to_string(), |p| format!("== {}", p));
    let action_scope = action.map_or("".to_string(), |a| format!("== {}", a));
    let resource_scope = resource.map_or("".to_string(), |r| format!("== {}", r));
    let policy = format!(
        "{}(principal {}, action {}, resource {}) {};",
        effect, principal_scope, action_scope, resource_scope, conditions
    );
    json!({
        "policy": policy,
        "order": order,
    })
}

#[tokio::test]
async fn test_order_permit_at_0_forbid_at_1() {
    let server = setup().await;

    let principal = new_principal();
    let action = new_action();
    let resource = new_resource();

    let permit_policy = new_policy_payload_with_order(
        "permit",
        Some(&action),
        Some(&principal),
        Some(&resource),
        "",
        0,
    );
    let response = server.put("/v1beta/policies/").json(&permit_policy).await;
    assert_eq!(response.status_code(), 201);

    let forbid_policy = new_policy_payload_with_order(
        "forbid",
        Some(&action),
        Some(&principal),
        Some(&resource),
        "",
        1,
    );
    let response = server.put("/v1beta/policies/").json(&forbid_policy).await;
    assert_eq!(response.status_code(), 201);

    let decision = verify(&server, &principal, &action, Some(&resource)).await;
    assert_eq!(decision, AuthorizationDecision::Allow);
}

#[tokio::test]
async fn test_order_forbid_at_0_permit_at_1() {
    let server = setup().await;

    let principal = new_principal();
    let action = new_action();
    let resource = new_resource();

    let forbid_policy = new_policy_payload_with_order(
        "forbid",
        Some(&action),
        Some(&principal),
        Some(&resource),
        "",
        0,
    );
    let response = server.put("/v1beta/policies/").json(&forbid_policy).await;
    assert_eq!(response.status_code(), 201);

    let permit_policy = new_policy_payload_with_order(
        "permit",
        Some(&action),
        Some(&principal),
        Some(&resource),
        "",
        1,
    );
    let response = server.put("/v1beta/policies/").json(&permit_policy).await;
    assert_eq!(response.status_code(), 201);

    let decision = verify(&server, &principal, &action, Some(&resource)).await;
    assert_eq!(decision, AuthorizationDecision::Deny);
}

#[tokio::test]
async fn test_order_no_match_at_0_permit_at_1() {
    let server = setup().await;

    let principal = new_principal();
    let action = new_action();
    let resource = new_resource();
    let other_principal = Principal {
        id: "unrelated-user".to_string(),
        info: None,
    };

    let unrelated_policy = new_policy_payload_with_order(
        "permit",
        Some(&action),
        Some(&other_principal),
        Some(&resource),
        "",
        0,
    );
    let response = server
        .put("/v1beta/policies/")
        .json(&unrelated_policy)
        .await;
    assert_eq!(response.status_code(), 201);

    let matching_policy = new_policy_payload_with_order(
        "permit",
        Some(&action),
        Some(&principal),
        Some(&resource),
        "",
        1,
    );
    let response = server.put("/v1beta/policies/").json(&matching_policy).await;
    assert_eq!(response.status_code(), 201);

    let decision = verify(&server, &principal, &action, Some(&resource)).await;
    assert_eq!(decision, AuthorizationDecision::Allow);
}

#[tokio::test]
async fn test_order_same_group_evaluation_priority() {
    let server = setup().await;

    let principal = new_principal();
    let action = new_action();
    let resource = new_resource();

    let permit_policy = new_policy_payload_with_order(
        "permit",
        Some(&action),
        Some(&principal),
        Some(&resource),
        "",
        0,
    );
    let response = server.put("/v1beta/policies/").json(&permit_policy).await;
    assert_eq!(response.status_code(), 201);

    let forbid_policy = new_policy_payload_with_order(
        "forbid",
        Some(&action),
        Some(&principal),
        Some(&resource),
        "",
        0,
    );
    let response = server.put("/v1beta/policies/").json(&forbid_policy).await;
    assert_eq!(response.status_code(), 201);

    setup_metadata(&server, EvaluationPriority::Forbid).await;
    let decision = verify(&server, &principal, &action, Some(&resource)).await;
    assert_eq!(decision, AuthorizationDecision::Deny);

    setup_metadata(&server, EvaluationPriority::Permit).await;
    let decision = verify(&server, &principal, &action, Some(&resource)).await;
    assert_eq!(decision, AuthorizationDecision::Allow);
}

#[tokio::test]
async fn test_policy_api_explicit_order() {
    let server = setup().await;

    let action = new_action();
    let principal = new_principal();
    let resource = new_resource();

    let payload = new_policy_payload_with_order(
        "permit",
        Some(&action),
        Some(&principal),
        Some(&resource),
        "",
        5,
    );
    let response = server.put("/v1beta/policies/").json(&payload).await;
    assert_eq!(response.status_code(), 201);

    let result: Value = response.json();
    assert_eq!(result["order"], json!(5));
}

#[tokio::test]
async fn test_policy_api_default_order() {
    let server = setup().await;

    let action = new_action();
    let principal = new_principal();
    let resource = new_resource();

    let payload = new_policy_payload(
        "permit",
        Some(&action),
        Some(&principal),
        Some(&resource),
        "",
    );
    let response = server.put("/v1beta/policies/").json(&payload).await;
    assert_eq!(response.status_code(), 201);

    let result: Value = response.json();
    assert_eq!(result["order"], json!(0));
}

#[tokio::test]
async fn test_order_forbid_at_0_overrides_permit_evaluation_priority() {
    let server = setup().await;

    let principal = new_principal();
    let action = new_action();
    let resource = new_resource();

    let forbid_policy = new_policy_payload_with_order(
        "forbid",
        Some(&action),
        Some(&principal),
        Some(&resource),
        "",
        0,
    );
    let response = server.put("/v1beta/policies/").json(&forbid_policy).await;
    assert_eq!(response.status_code(), 201);

    let permit_policy = new_policy_payload_with_order(
        "permit",
        Some(&action),
        Some(&principal),
        Some(&resource),
        "",
        1,
    );
    let response = server.put("/v1beta/policies/").json(&permit_policy).await;
    assert_eq!(response.status_code(), 201);

    setup_metadata(&server, EvaluationPriority::Permit).await;
    let decision = verify(&server, &principal, &action, Some(&resource)).await;
    assert_eq!(decision, AuthorizationDecision::Deny);
}

fn principal_with_groups(groups: &[&str]) -> Principal {
    Principal {
        id: "test-user".to_string(),
        info: Some(json!({ "groups": groups })),
    }
}

#[tokio::test]
async fn test_authorize_group_scope_member_allowed_non_member_denied() {
    let policies_yaml = r#"
policies:
  - policy: 'permit(principal in Group::"editors", action == Action::"tags:get", resource == ResourceAddress::"Astronaut.usd");'
"#;
    let server = setup_config_file_server(policies_yaml, TEST_METADATA_YAML).await;
    let action = new_action();
    let resource = new_resource();

    let member = principal_with_groups(&["editors"]);
    assert_eq!(
        verify(&server, &member, &action, Some(&resource)).await,
        AuthorizationDecision::Allow,
        "a principal in the scoped group must be allowed"
    );

    let other_group = principal_with_groups(&["viewers"]);
    assert_eq!(
        verify(&server, &other_group, &action, Some(&resource)).await,
        AuthorizationDecision::Deny,
        "a principal in a different group must be denied"
    );

    let no_groups = Principal {
        id: "test-user".to_string(),
        info: None,
    };
    assert_eq!(
        verify(&server, &no_groups, &action, Some(&resource)).await,
        AuthorizationDecision::Deny,
        "a principal without group memberships must be denied"
    );
}

#[tokio::test]
async fn test_get_policies_filter_by_group() {
    let policies_yaml = r#"
policies:
  - policy: 'permit(principal in Group::"editors", action == Action::"tags:get", resource == ResourceAddress::"Astronaut.usd");'
  - policy: 'permit(principal == Principal::"test-user", action == Action::"tags:get", resource == ResourceAddress::"Astronaut.usd");'
"#;
    let server = setup_config_file_server(policies_yaml, TEST_METADATA_YAML).await;

    let response = server
        .get("/v1beta/policies/")
        .add_query_param("group", "editors")
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    let items = result["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "only the editors-scoped policy must match");
    assert_eq!(items[0]["group"]["id"], "editors");
    assert!(items[0]["principal"].is_null());
}

#[tokio::test]
async fn test_authorize_group_scope_honors_configured_claim() {
    let policies_yaml = r#"
policies:
  - policy: 'permit(principal in Group::"editors", action == Action::"tags:get", resource == ResourceAddress::"Astronaut.usd");'
"#;
    let server =
        setup_config_file_server_with_groups_claim(policies_yaml, TEST_METADATA_YAML, "roles")
            .await;
    let action = new_action();
    let resource = new_resource();

    let via_roles = Principal {
        id: "test-user".to_string(),
        info: Some(json!({ "roles": ["editors"] })),
    };
    assert_eq!(
        verify(&server, &via_roles, &action, Some(&resource)).await,
        AuthorizationDecision::Allow,
        "membership read from the configured claim must be allowed"
    );

    let via_groups = principal_with_groups(&["editors"]);
    assert_eq!(
        verify(&server, &via_groups, &action, Some(&resource)).await,
        AuthorizationDecision::Deny,
        "the default `groups` claim must be ignored when another claim is configured"
    );
}

async fn setup_config_file_server_with_groups_claim(
    policies_yaml: &str,
    metadata_yaml: &str,
    principal_groups_claim: &str,
) -> TestServer {
    let mut config = setup_config();
    config.auth.principal_groups_claim = principal_groups_claim.to_string();
    let cache = setup_cache(&config);

    let client = Arc::new(reqwest::Client::new());

    let dir = std::env::temp_dir().join(format!("permission-test-{}", std::process::id()));
    tokio::fs::create_dir_all(&dir).await.unwrap();

    let policy_path = dir.join(format!("policies-{}.yaml", rand_suffix()));
    let metadata_path = dir.join(format!("metadata-{}.yaml", rand_suffix()));

    tokio::fs::write(&policy_path, policies_yaml).await.unwrap();
    tokio::fs::write(&metadata_path, metadata_yaml)
        .await
        .unwrap();

    let policies = ConfigFilePolicyStorage::new(policy_path);
    policies.initialize().await.unwrap();

    let metadata =
        ConfigFileMetadataStorage::new(metadata_path, config.auth.principal_id_claim.clone());
    metadata.initialize().await.unwrap();

    let policies: Arc<dyn PolicyStorage> = Arc::new(policies);
    let metadata: Arc<dyn MetadataStorage> = Arc::new(metadata);

    let (metrics, caches) = setup_metrics(&config, cache);
    let state = AppState::new(caches, config, policies, metadata, metrics, client).await;
    let router = configure_router(state);
    TestServer::new(router).unwrap()
}

async fn setup_config_file_server_with_auth(
    jwk: &JsonWebKeyFixture,
    policies_yaml: &str,
    metadata_yaml: &str,
) -> TestServer {
    let mut config = jwk.config();
    config.database.init.policies_file = String::new();

    let cache = setup_cache(&config);
    let client = Arc::new(reqwest::Client::new());
    config.auth.load(&client).await.unwrap();

    let dir = std::env::temp_dir().join(format!("permission-test-{}", std::process::id()));
    tokio::fs::create_dir_all(&dir).await.unwrap();

    let policy_path = dir.join(format!("policies-{}.yaml", rand_suffix()));
    let metadata_path = dir.join(format!("metadata-{}.yaml", rand_suffix()));

    tokio::fs::write(&policy_path, policies_yaml).await.unwrap();
    tokio::fs::write(&metadata_path, metadata_yaml)
        .await
        .unwrap();

    let policies = ConfigFilePolicyStorage::new(policy_path);
    policies.initialize().await.unwrap();

    let metadata =
        ConfigFileMetadataStorage::new(metadata_path, config.auth.principal_id_claim.clone());
    metadata.initialize().await.unwrap();

    let policies: Arc<dyn PolicyStorage> = Arc::new(policies);
    let metadata: Arc<dyn MetadataStorage> = Arc::new(metadata);

    let (metrics, caches) = setup_metrics(&config, cache);
    let state = AppState::new(caches, config, policies, metadata, metrics, client).await;
    let router = configure_router(state);
    TestServer::new(router).unwrap()
}

#[tokio::test]
async fn test_diagnostics_authorize_returns_matching_policies() {
    let policies_yaml = r#"
policies:
  - policy: 'permit(principal == Principal::"test-user", action == Action::"tags:get", resource == ResourceAddress::"Astronaut.usd");'
  - policy: 'permit(principal == Principal::"other-user", action == Action::"tags:get", resource == ResourceAddress::"Astronaut.usd");'
  - policy: 'permit(principal, action, resource);'
"#;
    let server = setup_config_file_server(policies_yaml, TEST_METADATA_YAML).await;

    let response = server
        .post("/v1beta/diagnostics/authorize/")
        .json(&json!({
            "principal": { "sub": "test-user" },
            "action": { "service": "tags", "name": "get" },
            "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
        }))
        .await;
    assert_eq!(response.status_code(), 200);

    let result: DiagnosticsAuthorizeResponse = response.json();
    assert_eq!(result.policies.len(), 2);

    assert_eq!(result.policies[0].id, 1);
    assert!(result.policies[0].policy.contains("test-user"));
    assert_eq!(
        result.policies[0].principal.as_ref().unwrap().id,
        "test-user"
    );
    assert_eq!(result.policies[0].action.as_ref().unwrap().name, "get");
    assert_eq!(
        result.policies[0].resource.as_ref().unwrap().id,
        "Astronaut.usd"
    );

    assert_eq!(result.policies[1].id, 3);
    assert!(result.policies[1].principal.is_none());
    assert!(result.policies[1].action.is_none());
    assert!(result.policies[1].resource.is_none());
}

#[tokio::test]
async fn test_diagnostics_authorize_evaluation_order_preserved() {
    let policies_yaml = r#"
policies:
  - policy: 'permit(principal, action == Action::"tags:get", resource);'
    order: 10
  - policy: 'forbid(principal, action == Action::"tags:get", resource);'
    order: 0
  - policy: 'permit(principal, action == Action::"tags:get", resource);'
    order: 5
"#;
    let server = setup_config_file_server(policies_yaml, TEST_METADATA_YAML).await;

    let response = server
        .post("/v1beta/diagnostics/authorize/")
        .json(&json!({
            "principal": { "sub": "test-user" },
            "action": { "service": "tags", "name": "get" },
            "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
        }))
        .await;
    assert_eq!(response.status_code(), 200);

    let result: DiagnosticsAuthorizeResponse = response.json();
    assert_eq!(result.policies.len(), 3);
    assert_eq!(result.policies[0].order, 0);
    assert_eq!(result.policies[1].order, 5);
    assert_eq!(result.policies[2].order, 10);
}

#[tokio::test]
async fn test_diagnostics_authorize_excludes_unrelated_policies() {
    let policies_yaml = r#"
policies:
  - policy: 'permit(principal == Principal::"test-user", action == Action::"tags:get", resource == ResourceAddress::"Astronaut.usd");'
  - policy: 'permit(principal == Principal::"other-user", action == Action::"tags:get", resource == ResourceAddress::"Astronaut.usd");'
  - policy: 'permit(principal == Principal::"test-user", action == Action::"tags:list", resource == ResourceAddress::"Astronaut.usd");'
  - policy: 'permit(principal == Principal::"test-user", action == Action::"tags:get", resource == ResourceAddress::"Other.usd");'
"#;
    let server = setup_config_file_server(policies_yaml, TEST_METADATA_YAML).await;

    let response = server
        .post("/v1beta/diagnostics/authorize/")
        .json(&json!({
            "principal": { "sub": "test-user" },
            "action": { "service": "tags", "name": "get" },
            "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
        }))
        .await;
    assert_eq!(response.status_code(), 200);

    let result: DiagnosticsAuthorizeResponse = response.json();
    assert_eq!(result.policies.len(), 1);
    assert_eq!(result.policies[0].id, 1);
    assert!(result.policies[0].policy.contains("test-user"));
    assert!(result.policies[0].policy.contains("tags:get"));
    assert!(result.policies[0].policy.contains("Astronaut.usd"));
}

#[tokio::test]
async fn test_diagnostics_authorize_returns_evaluation_priority() {
    let metadata_permit_yaml = r#"
services:
  - name: "tags"
    principal:
      idClaim: "sub"
    actions:
      - "get"
    resource_types:
      - type: "ResourceAddress"
        evaluation_priority: "permit"
"#;
    let policies_yaml = r#"
policies:
  - policy: 'permit(principal, action == Action::"tags:get", resource);'
"#;
    let server = setup_config_file_server(policies_yaml, metadata_permit_yaml).await;

    let response = server
        .post("/v1beta/diagnostics/authorize/")
        .json(&json!({
            "principal": { "sub": "test-user" },
            "action": { "service": "tags", "name": "get" },
            "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
        }))
        .await;
    assert_eq!(response.status_code(), 200);

    let result: DiagnosticsAuthorizeResponse = response.json();
    assert_eq!(result.evaluation_priority, EvaluationPriority::Permit);
    assert_eq!(result.policies.len(), 1);

    let server_forbid = setup_config_file_server(policies_yaml, TEST_METADATA_YAML).await;

    let response = server_forbid
        .post("/v1beta/diagnostics/authorize/")
        .json(&json!({
            "principal": { "sub": "test-user" },
            "action": { "service": "tags", "name": "get" },
            "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
        }))
        .await;
    assert_eq!(response.status_code(), 200);

    let result: DiagnosticsAuthorizeResponse = response.json();
    assert_eq!(result.evaluation_priority, EvaluationPriority::Forbid);
}

#[tokio::test]
async fn test_diagnostics_authorize_requires_permission() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let admin_group = jwk.admin_group();

    let mut claims = Claims::default();
    claims.groups = vec![admin_group.clone()];
    let token = jwk.encode(&claims);

    let config = jwk.config();
    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;

    let diagnostics_body = json!({
        "principal": { "sub": "test-user" },
        "action": { "service": "tags", "name": "get" },
        "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
    });

    let response = server
        .post("/v1beta/diagnostics/authorize/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&diagnostics_body)
        .await;
    assert_eq!(response.status_code(), 403);

    let diagnostics_policy = format!(
        r#"permit(principal, action == Action::"permissions:diagnostics", resource) when {{ principal.groups.contains("{admin_group}") }};"#
    );
    let response = server
        .put("/v1beta/policies/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&json!({ "policy": diagnostics_policy }))
        .await;
    assert_eq!(response.status_code(), 201);

    let response = server
        .post("/v1beta/diagnostics/authorize/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&diagnostics_body)
        .await;
    assert_eq!(response.status_code(), 200);
}

#[tokio::test]
async fn test_diagnostics_authorize_forbidden_without_auth() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let admin_group = jwk.admin_group();

    let policies_yaml = format!(
        r#"
policies:
  - policy: 'permit(principal, action == Action::"permissions:diagnostics", resource) when {{ principal.groups.contains("{admin_group}") }};'
  - policy: 'permit(principal, action == Action::"tags:get", resource);'
"#
    );

    let server = setup_config_file_server_with_auth(&jwk, &policies_yaml, TEST_METADATA_YAML).await;

    let response = server
        .post("/v1beta/diagnostics/authorize/")
        .json(&json!({
            "principal": { "sub": "test-user" },
            "action": { "service": "tags", "name": "get" },
            "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
        }))
        .await;
    assert_eq!(response.status_code(), 401);
}

#[tokio::test]
async fn test_diagnostics_authorize_empty_result() {
    let policies_yaml = r#"
policies:
  - policy: 'permit(principal == Principal::"other-user", action == Action::"tags:get", resource == ResourceAddress::"Astronaut.usd");'
"#;
    let server = setup_config_file_server(policies_yaml, TEST_METADATA_YAML).await;

    let response = server
        .post("/v1beta/diagnostics/authorize/")
        .json(&json!({
            "principal": { "sub": "test-user" },
            "action": { "service": "tags", "name": "get" },
            "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
        }))
        .await;
    assert_eq!(response.status_code(), 200);

    let result: DiagnosticsAuthorizeResponse = response.json();
    assert!(result.policies.is_empty());
}

#[tokio::test]
async fn test_diagnostics_authorize_same_order_sorted_by_id() {
    let policies_yaml = r#"
policies:
  - policy: 'permit(principal, action == Action::"tags:get", resource);'
    order: 0
  - policy: 'forbid(principal, action == Action::"tags:get", resource);'
    order: 0
  - policy: 'permit(principal, action == Action::"tags:get", resource == ResourceAddress::"Astronaut.usd");'
    order: 0
"#;
    let server = setup_config_file_server(policies_yaml, TEST_METADATA_YAML).await;

    let response = server
        .post("/v1beta/diagnostics/authorize/")
        .json(&json!({
            "principal": { "sub": "test-user" },
            "action": { "service": "tags", "name": "get" },
            "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
        }))
        .await;
    assert_eq!(response.status_code(), 200);

    let result: DiagnosticsAuthorizeResponse = response.json();
    assert_eq!(result.policies.len(), 3);
    assert_eq!(result.policies[0].order, 0);
    assert_eq!(result.policies[1].order, 0);
    assert_eq!(result.policies[2].order, 0);
    assert!(result.policies[0].id < result.policies[1].id);
    assert!(result.policies[1].id < result.policies[2].id);
}

#[tokio::test]
async fn test_diagnostics_authorize_batch_preserves_order_per_entry() {
    let policies_yaml = r#"
policies:
  - policy: 'permit(principal == Principal::"test-user", action == Action::"tags:get", resource == ResourceAddress::"Astronaut.usd");'
  - policy: 'permit(principal == Principal::"test-user", action == Action::"tags:get", resource == ResourceAddress::"Rocket.usd");'
  - policy: 'permit(principal, action == Action::"tags:get", resource);'
"#;
    let server = setup_config_file_server(policies_yaml, TEST_METADATA_YAML).await;

    let response = server
        .post("/v1beta/diagnostics/authorize/batch/")
        .json(&json!({
            "batches": [
                {
                    "principal": { "sub": "test-user" },
                    "actions": [{ "service": "tags", "name": "get" }],
                    "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
                },
                {
                    "principal": { "sub": "test-user" },
                    "actions": [{ "service": "tags", "name": "get" }],
                    "resource": { "id": "Rocket.usd", "type": "ResourceAddress" }
                }
            ]
        }))
        .await;
    assert_eq!(response.status_code(), 200);

    let result: DiagnosticsAuthorizeBatchResponse = response.json();
    assert_eq!(result.results.len(), 2);

    assert_eq!(
        result.results[0].resource.as_ref().unwrap().id,
        "Astronaut.usd"
    );
    assert_eq!(result.results[0].actions.len(), 1);
    let astronaut = &result.results[0].actions[0];
    assert_eq!(astronaut.service, "tags");
    assert_eq!(astronaut.action, "get");
    assert_eq!(astronaut.policies.len(), 2);
    assert_eq!(astronaut.policies[0].id, 1);
    assert_eq!(astronaut.policies[1].id, 3);

    assert_eq!(
        result.results[1].resource.as_ref().unwrap().id,
        "Rocket.usd"
    );
    let rocket = &result.results[1].actions[0];
    assert_eq!(rocket.policies.len(), 2);
    assert_eq!(rocket.policies[0].id, 2);
    assert_eq!(rocket.policies[1].id, 3);
}

#[tokio::test]
async fn test_diagnostics_authorize_batch_returns_evaluation_priority() {
    let metadata_permit_yaml = r#"
services:
  - name: "tags"
    principal:
      idClaim: "sub"
    actions:
      - "get"
    resource_types:
      - type: "ResourceAddress"
        evaluation_priority: "permit"
"#;
    let policies_yaml = r#"
policies:
  - policy: 'permit(principal, action == Action::"tags:get", resource);'
"#;
    let server = setup_config_file_server(policies_yaml, metadata_permit_yaml).await;

    let response = server
        .post("/v1beta/diagnostics/authorize/batch/")
        .json(&json!({
            "batches": [{
                "principal": { "sub": "test-user" },
                "actions": [{ "service": "tags", "name": "get" }],
                "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
            }]
        }))
        .await;
    assert_eq!(response.status_code(), 200);

    let result: DiagnosticsAuthorizeBatchResponse = response.json();
    assert_eq!(result.results.len(), 1);
    assert_eq!(
        result.results[0].evaluation_priority,
        EvaluationPriority::Permit
    );

    let server_forbid = setup_config_file_server(policies_yaml, TEST_METADATA_YAML).await;

    let response = server_forbid
        .post("/v1beta/diagnostics/authorize/batch/")
        .json(&json!({
            "batches": [{
                "principal": { "sub": "test-user" },
                "actions": [{ "service": "tags", "name": "get" }],
                "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
            }]
        }))
        .await;
    assert_eq!(response.status_code(), 200);

    let result: DiagnosticsAuthorizeBatchResponse = response.json();
    assert_eq!(
        result.results[0].evaluation_priority,
        EvaluationPriority::Forbid
    );
}

#[tokio::test]
async fn test_diagnostics_authorize_batch_empty_result() {
    let policies_yaml = r#"
policies:
  - policy: 'permit(principal == Principal::"other-user", action == Action::"tags:get", resource == ResourceAddress::"Astronaut.usd");'
"#;
    let server = setup_config_file_server(policies_yaml, TEST_METADATA_YAML).await;

    let response = server
        .post("/v1beta/diagnostics/authorize/batch/")
        .json(&json!({
            "batches": [{
                "principal": { "sub": "test-user" },
                "actions": [{ "service": "tags", "name": "get" }],
                "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
            }]
        }))
        .await;
    assert_eq!(response.status_code(), 200);

    let result: DiagnosticsAuthorizeBatchResponse = response.json();
    assert_eq!(result.results.len(), 1);
    assert_eq!(result.results[0].actions.len(), 1);
    assert!(result.results[0].actions[0].policies.is_empty());
}

#[tokio::test]
async fn test_diagnostics_authorize_batch_requires_permission() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let admin_group = jwk.admin_group();

    let mut claims = Claims::default();
    claims.groups = vec![admin_group.clone()];
    let token = jwk.encode(&claims);

    let config = jwk.config();
    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;

    let diagnostics_body = json!({
        "batches": [{
            "principal": { "sub": "test-user" },
            "actions": [{ "service": "tags", "name": "get" }],
            "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
        }]
    });

    let response = server
        .post("/v1beta/diagnostics/authorize/batch/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&diagnostics_body)
        .await;
    assert_eq!(response.status_code(), 403);

    let diagnostics_policy = format!(
        r#"permit(principal, action == Action::"permissions:diagnostics", resource) when {{ principal.groups.contains("{admin_group}") }};"#
    );
    let response = server
        .put("/v1beta/policies/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&json!({ "policy": diagnostics_policy }))
        .await;
    assert_eq!(response.status_code(), 201);

    let response = server
        .post("/v1beta/diagnostics/authorize/batch/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&diagnostics_body)
        .await;
    assert_eq!(response.status_code(), 200);
}

#[tokio::test]
async fn test_diagnostics_authorize_batch_forbidden_without_auth() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let admin_group = jwk.admin_group();

    let policies_yaml = format!(
        r#"
policies:
  - policy: 'permit(principal, action == Action::"permissions:diagnostics", resource) when {{ principal.groups.contains("{admin_group}") }};'
  - policy: 'permit(principal, action == Action::"tags:get", resource);'
"#
    );

    let server = setup_config_file_server_with_auth(&jwk, &policies_yaml, TEST_METADATA_YAML).await;

    let response = server
        .post("/v1beta/diagnostics/authorize/batch/")
        .json(&json!({
            "batches": [{
                "principal": { "sub": "test-user" },
                "actions": [{ "service": "tags", "name": "get" }],
                "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
            }]
        }))
        .await;
    assert_eq!(response.status_code(), 401);
}

#[tokio::test]
async fn test_config_file_authorization_without_resource_excludes_resource_scoped() {
    let policies_yaml = r#"
policies:
  - policy: 'permit(principal == Principal::"test-user", action == Action::"tags:get", resource == ResourceAddress::"Astronaut.usd");'
  - policy: 'permit(principal, action == Action::"tags:list", resource);'
"#;
    let server = setup_config_file_server(policies_yaml, TEST_METADATA_YAML).await;

    // With resource specified: the resource-scoped policy matches and allows access
    let response = server
        .post("/v1beta/authorization/")
        .json(&json!({
            "principal": { "sub": "test-user" },
            "action": { "service": "tags", "name": "get" },
            "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
        }))
        .await;
    assert_eq!(response.status_code(), 200);
    let result: Value = response.json();
    assert_eq!(result["decision"], "allow");

    // Without resource: the resource-scoped policy should NOT be fetched,
    // leaving only the unscoped "tags:list" policy which doesn't match "tags:get".
    // This should result in a deny.
    let response = server
        .post("/v1beta/authorization/")
        .json(&json!({
            "principal": { "sub": "test-user" },
            "action": { "service": "tags", "name": "get" }
        }))
        .await;
    assert_eq!(response.status_code(), 200);
    let result: Value = response.json();
    assert_eq!(result["decision"], "deny");
}

#[tokio::test]
async fn test_config_file_search_without_resource_includes_resource_scoped() {
    let policies_yaml = r#"
policies:
  - policy: 'permit(principal == Principal::"test-user", action == Action::"tags:get", resource == ResourceAddress::"Astronaut.usd");'
  - policy: 'permit(principal, action == Action::"tags:list", resource);'
  - policy: 'permit(principal, action, resource);'
"#;
    let server = setup_config_file_server(policies_yaml, TEST_METADATA_YAML).await;

    // Search with no filters: returns ALL policies regardless of scopes
    let response = server.get("/v1beta/policies/").await;
    assert_eq!(response.status_code(), 200);
    let result: Value = response.json();
    assert_eq!(result["items"].as_array().unwrap().len(), 3);

    // Search with principal filter: exact match on principal
    let response = server
        .get("/v1beta/policies/")
        .add_query_param("principal", "test-user")
        .await;
    assert_eq!(response.status_code(), 200);
    let result: Value = response.json();
    assert_eq!(result["items"].as_array().unwrap().len(), 1);

    // Search with action filter: exact match on action
    let response = server
        .get("/v1beta/policies/")
        .add_query_param("action", "tags:get")
        .await;
    assert_eq!(response.status_code(), 200);
    let result: Value = response.json();
    let items = result["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
}

#[tokio::test]
async fn test_postgres_authorization_without_resource_excludes_resource_scoped() {
    let server = setup().await;
    setup_metadata(&server, EvaluationPriority::Forbid).await;

    let principal = new_principal();
    let action = new_action();
    let resource = new_resource();

    let scoped_policy = new_policy_payload(
        "permit",
        Some(&action),
        Some(&principal),
        Some(&resource),
        "",
    );
    let response = server.put("/v1beta/policies/").json(&scoped_policy).await;
    assert_eq!(response.status_code(), 201);

    // With resource: resource-scoped policy matches → allow
    let decision = verify(&server, &principal, &action, Some(&resource)).await;
    assert_eq!(decision, AuthorizationDecision::Allow);

    // Without resource: resource-scoped policy excluded → deny
    let decision = verify(&server, &principal, &action, None).await;
    assert_eq!(decision, AuthorizationDecision::Deny);
}

#[tokio::test]
async fn test_postgres_search_without_resource_includes_resource_scoped() {
    let server = setup().await;
    setup_metadata(&server, EvaluationPriority::Forbid).await;

    let principal = new_principal();
    let action = new_action();
    let resource = new_resource();

    let scoped_policy = new_policy_payload(
        "permit",
        Some(&action),
        Some(&principal),
        Some(&resource),
        "",
    );
    let response = server.put("/v1beta/policies/").json(&scoped_policy).await;
    assert_eq!(response.status_code(), 201);

    let unscoped_policy = new_policy_payload("permit", None, None, None, "");
    let response = server.put("/v1beta/policies/").json(&unscoped_policy).await;
    assert_eq!(response.status_code(), 201);

    // Search with no filters: returns ALL policies
    let response = server.get("/v1beta/policies/").await;
    assert_eq!(response.status_code(), 200);
    let result: Value = response.json();
    assert_eq!(result["items"].as_array().unwrap().len(), 2);

    // Search with principal filter: exact match on principal
    let response = server
        .get("/v1beta/policies/")
        .add_query_param("principal", &principal.id)
        .await;
    assert_eq!(response.status_code(), 200);
    let result: Value = response.json();
    assert_eq!(result["items"].as_array().unwrap().len(), 1);

    // Search with action filter: exact match on action
    let response = server
        .get("/v1beta/policies/")
        .add_query_param("action", format!("{}:{}", action.service, action.name))
        .await;
    assert_eq!(response.status_code(), 200);
    let result: Value = response.json();
    assert_eq!(result["items"].as_array().unwrap().len(), 1);
}

async fn setup_server_with_mock_events(
    config: ServiceConfig,
    cache: Arc<AuthCache>,
) -> (TestServer, Arc<MockEventPublisher>) {
    let client = Arc::new(reqwest::Client::new());
    let mut config = config;
    config.auth.load(&client).await.unwrap();

    let (metrics, caches) = setup_metrics(&config, cache);
    let db_pool: Arc<dyn ConnectionPool> = Arc::new(InstrumentedPool::new(
        create_database_pool(&config).await,
        &metrics.meter,
    ));
    let options = PostgresDatabaseOptions { temporary: true };
    let dbname = config.postgres.dbname.as_deref().unwrap();

    let mut policies = PostgresPolicyStorage::new(
        db_pool.clone(),
        POSTGRES_RUNTIME.clone(),
        dbname,
        options.clone(),
    );
    policies.initialize().await.unwrap();
    set_system_policies(&mut policies, &config).await;

    let metadata = PostgresMetadataStorage::new(
        db_pool.clone(),
        POSTGRES_RUNTIME.clone(),
        dbname,
        options.clone(),
        config.auth.principal_id_claim.clone(),
    );
    metadata.initialize().await.unwrap();
    set_system_metadata(&metadata, &config).await;

    let policies: Arc<dyn PolicyStorage> = Arc::new(policies);
    let metadata: Arc<dyn MetadataStorage> = Arc::new(metadata);

    let mut state = AppState::new(caches, config, policies, metadata, metrics, client).await;

    let (event_client, mock_publisher) = permission_service::state::EventClient::mock();
    state.event_client = Some(event_client);

    let router = configure_router(state);
    (TestServer::new(router).unwrap(), mock_publisher)
}

async fn discard_metadata_setup_event(mock_publisher: &Arc<MockEventPublisher>) {
    mock_publisher
        .wait_for_events(1, Duration::from_secs(5))
        .await;
    mock_publisher.clear();
}

#[tokio::test]
async fn test_create_policy_publishes_event() {
    let config = setup_config();
    let cache = setup_cache(&config);
    let (server, mock_publisher) = setup_server_with_mock_events(config, cache).await;
    setup_metadata(&server, EvaluationPriority::Forbid).await;
    discard_metadata_setup_event(&mock_publisher).await;

    let payload = get_default_policy_payload();
    let response = server.put("/v1beta/policies/").json(&payload).await;
    assert_eq!(response.status_code(), 201);

    mock_publisher
        .wait_for_events(1, Duration::from_secs(5))
        .await;

    let requests = mock_publisher.published_requests();
    assert_eq!(
        requests.len(),
        1,
        "expected exactly one event to be published"
    );

    let event = requests[0].event.as_ref().unwrap();
    assert_eq!(event.event_type, "omni.permissions.changed");

    let fields = &event.message.as_ref().unwrap().fields;
    assert_eq!(
        fields["principal"].kind,
        Some(prost_types::value::Kind::StringValue(
            "test-user".to_string()
        ))
    );
    assert_eq!(
        fields["action"].kind,
        Some(prost_types::value::Kind::StringValue(
            "tags:get".to_string()
        ))
    );
    assert_eq!(
        fields["resource"].kind,
        Some(prost_types::value::Kind::StringValue(
            "ResourceAddress::\"Astronaut.usd\"".to_string()
        ))
    );
}

#[tokio::test]
async fn test_delete_policy_publishes_event() {
    let config = setup_config();
    let cache = setup_cache(&config);
    let (server, mock_publisher) = setup_server_with_mock_events(config, cache).await;
    setup_metadata(&server, EvaluationPriority::Forbid).await;
    discard_metadata_setup_event(&mock_publisher).await;

    let payload = get_default_policy_payload();
    let response = server.put("/v1beta/policies/").json(&payload).await;
    assert_eq!(response.status_code(), 201);
    let policy_id = response.json::<Value>()["id"].as_i64().unwrap();

    mock_publisher
        .wait_for_events(1, Duration::from_secs(5))
        .await;

    let response = server
        .delete(&format!("/v1beta/policies/{}", policy_id))
        .await;
    assert_eq!(response.status_code(), 204);

    mock_publisher
        .wait_for_events(2, Duration::from_secs(5))
        .await;

    let requests = mock_publisher.published_requests();
    assert_eq!(
        requests.len(),
        2,
        "expected two events: one create, one delete"
    );

    let delete_event = requests[1].event.as_ref().unwrap();
    assert_eq!(delete_event.event_type, "omni.permissions.changed");
}

#[tokio::test]
async fn test_put_service_metadata_publishes_event() {
    let config = setup_config();
    let cache = setup_cache(&config);
    let (server, mock_publisher) = setup_server_with_mock_events(config, cache).await;

    let response = server
        .put("/v1beta/services/tags/")
        .json(&json!({ "service": "tags", "id_claim": "email" }))
        .await;
    assert_eq!(response.status_code(), 200);

    mock_publisher
        .wait_for_events(1, Duration::from_secs(5))
        .await;

    let requests = mock_publisher.published_requests();
    assert_eq!(requests.len(), 1, "expected exactly one metadata event");
    assert_meta_event(&requests[0], "tags", "service", "updated", "");
}

#[tokio::test]
async fn test_delete_service_metadata_publishes_event() {
    let config = setup_config();
    let cache = setup_cache(&config);
    let (server, mock_publisher) = setup_server_with_mock_events(config, cache).await;

    let response = server.delete("/v1beta/services/tags/").await;
    assert_eq!(response.status_code(), 204);

    mock_publisher
        .wait_for_events(1, Duration::from_secs(5))
        .await;

    let requests = mock_publisher.published_requests();
    assert_eq!(requests.len(), 1, "expected exactly one metadata event");
    assert_meta_event(&requests[0], "tags", "service", "removed", "");
}

#[tokio::test]
async fn test_replace_service_actions_publishes_event() {
    let config = setup_config();
    let cache = setup_cache(&config);
    let (server, mock_publisher) = setup_server_with_mock_events(config, cache).await;

    let response = server
        .put("/v1beta/services/tags/actions/")
        .json(&json!([{ "name": "get", "service": "tags" }]))
        .await;
    assert_eq!(response.status_code(), 200);

    mock_publisher
        .wait_for_events(1, Duration::from_secs(5))
        .await;

    let requests = mock_publisher.published_requests();
    assert_eq!(requests.len(), 1, "expected exactly one metadata event");
    assert_meta_event(&requests[0], "tags", "action", "updated", "");
}

#[tokio::test]
async fn test_put_service_action_publishes_event() {
    let config = setup_config();
    let cache = setup_cache(&config);
    let (server, mock_publisher) = setup_server_with_mock_events(config, cache).await;

    let response = server.put("/v1beta/services/tags/actions/get/").await;
    assert_eq!(response.status_code(), 200);

    mock_publisher
        .wait_for_events(1, Duration::from_secs(5))
        .await;

    let requests = mock_publisher.published_requests();
    assert_eq!(requests.len(), 1, "expected exactly one metadata event");
    assert_meta_event(&requests[0], "tags", "action", "updated", "get");
}

#[tokio::test]
async fn test_delete_service_action_publishes_event() {
    let config = setup_config();
    let cache = setup_cache(&config);
    let (server, mock_publisher) = setup_server_with_mock_events(config, cache).await;

    let response = server.delete("/v1beta/services/tags/actions/get/").await;
    assert_eq!(response.status_code(), 204);

    mock_publisher
        .wait_for_events(1, Duration::from_secs(5))
        .await;

    let requests = mock_publisher.published_requests();
    assert_eq!(requests.len(), 1, "expected exactly one metadata event");
    assert_meta_event(&requests[0], "tags", "action", "removed", "get");
}

#[tokio::test]
async fn test_replace_service_resource_types_publishes_event() {
    let config = setup_config();
    let cache = setup_cache(&config);
    let (server, mock_publisher) = setup_server_with_mock_events(config, cache).await;

    let response = server
        .put("/v1beta/services/tags/resource-types/")
        .json(&json!([{
            "service": "tags",
            "type": "ResourceAddress",
            "evaluation_priority": "forbid"
        }]))
        .await;
    assert_eq!(response.status_code(), 200);

    mock_publisher
        .wait_for_events(1, Duration::from_secs(5))
        .await;

    let requests = mock_publisher.published_requests();
    assert_eq!(requests.len(), 1, "expected exactly one metadata event");
    assert_meta_event(&requests[0], "tags", "resource_type", "updated", "");
}

#[tokio::test]
async fn test_put_resource_metadata_publishes_event() {
    let config = setup_config();
    let cache = setup_cache(&config);
    let (server, mock_publisher) = setup_server_with_mock_events(config, cache).await;

    let response = server
        .put("/v1beta/services/tags/resource-types/ResourceAddress/")
        .json(&json!({
            "service": "tags",
            "type": "ResourceAddress",
            "evaluation_priority": "forbid"
        }))
        .await;
    assert_eq!(response.status_code(), 200);

    mock_publisher
        .wait_for_events(1, Duration::from_secs(5))
        .await;

    let requests = mock_publisher.published_requests();
    assert_eq!(requests.len(), 1, "expected exactly one metadata event");
    assert_meta_event(
        &requests[0],
        "tags",
        "resource_type",
        "updated",
        "ResourceAddress",
    );
}

#[tokio::test]
async fn test_put_resource_metadata_cross_service_kind_conflicts() {
    let config = setup_config();
    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;

    let created = server
        .put("/v1beta/services/storage/resource-types/Widget/")
        .json(&json!({ "service": "storage", "type": "Widget", "evaluation_priority": "forbid" }))
        .await;
    assert_eq!(created.status_code(), 200);

    // The same kind under another service is rejected with 409 Conflict.
    let conflict = server
        .put("/v1beta/services/billing/resource-types/Widget/")
        .json(&json!({ "service": "billing", "type": "Widget", "evaluation_priority": "forbid" }))
        .await;
    assert_eq!(conflict.status_code(), 409, "{}", conflict.text());
    assert!(
        conflict
            .text()
            .to_lowercase()
            .contains("already registered"),
        "conflict body should explain the duplicate kind: {}",
        conflict.text()
    );

    // Updating the same kind under its owning service still succeeds.
    let updated = server
        .put("/v1beta/services/storage/resource-types/Widget/")
        .json(&json!({ "service": "storage", "type": "Widget", "evaluation_priority": "permit" }))
        .await;
    assert_eq!(updated.status_code(), 200);
}

#[tokio::test]
async fn test_replace_service_resource_types_cross_service_kind_conflicts() {
    let config = setup_config();
    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;

    let created = server
        .put("/v1beta/services/storage/resource-types/Shared/")
        .json(&json!({ "service": "storage", "type": "Shared", "evaluation_priority": "forbid" }))
        .await;
    assert_eq!(created.status_code(), 200);

    // A bulk replace that steals another service's kind is rejected and rolled
    // back, so none of the batch (including the non-conflicting entry) persists.
    let conflict = server
        .put("/v1beta/services/billing/resource-types/")
        .json(&json!([
            { "service": "billing", "type": "Invoice", "evaluation_priority": "forbid" },
            { "service": "billing", "type": "Shared", "evaluation_priority": "forbid" }
        ]))
        .await;
    assert_eq!(conflict.status_code(), 409, "{}", conflict.text());

    let billing_types = server.get("/v1beta/services/billing/resource-types/").await;
    assert_eq!(billing_types.status_code(), 200);
    assert_eq!(
        billing_types.json::<Value>().as_array().unwrap().len(),
        0,
        "conflicting replace must roll back the whole batch"
    );
}

#[tokio::test]
async fn test_resource_type_kind_move_between_services_requires_delete() {
    let config = setup_config();
    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;

    let created = server
        .put("/v1beta/services/storage/resource-types/Movable/")
        .json(&json!({ "service": "storage", "type": "Movable", "evaluation_priority": "forbid" }))
        .await;
    assert_eq!(created.status_code(), 200);

    let blocked = server
        .put("/v1beta/services/billing/resource-types/Movable/")
        .json(&json!({ "service": "billing", "type": "Movable", "evaluation_priority": "forbid" }))
        .await;
    assert_eq!(blocked.status_code(), 409);

    let removed = server
        .delete("/v1beta/services/storage/resource-types/Movable/")
        .await;
    assert_eq!(removed.status_code(), 204);

    // Once the old owner releases the kind, another service may claim it.
    let moved = server
        .put("/v1beta/services/billing/resource-types/Movable/")
        .json(&json!({ "service": "billing", "type": "Movable", "evaluation_priority": "forbid" }))
        .await;
    assert_eq!(moved.status_code(), 200, "{}", moved.text());
}

#[tokio::test]
async fn test_delete_resource_metadata_publishes_event() {
    let config = setup_config();
    let cache = setup_cache(&config);
    let (server, mock_publisher) = setup_server_with_mock_events(config, cache).await;

    let response = server
        .delete("/v1beta/services/tags/resource-types/ResourceAddress/")
        .await;
    assert_eq!(response.status_code(), 204);

    mock_publisher
        .wait_for_events(1, Duration::from_secs(5))
        .await;

    let requests = mock_publisher.published_requests();
    assert_eq!(requests.len(), 1, "expected exactly one metadata event");
    assert_meta_event(
        &requests[0],
        "tags",
        "resource_type",
        "removed",
        "ResourceAddress",
    );
}

#[tokio::test]
async fn test_metadata_read_publishes_no_event() {
    let config = setup_config();
    let cache = setup_cache(&config);
    let (server, mock_publisher) = setup_server_with_mock_events(config, cache).await;

    let response = server.get("/v1beta/services/").await;
    assert_eq!(response.status_code(), 200);

    sleep(Duration::from_millis(200)).await;
    assert!(
        mock_publisher.published_requests().is_empty(),
        "read endpoints must not publish events"
    );
}

fn assert_meta_event(
    request: &PublishEventRequest,
    service: &str,
    entity: &str,
    operation: &str,
    name: &str,
) {
    let event = request.event.as_ref().unwrap();
    assert_eq!(event.event_type, "omni.permissions.meta.changed");
    assert_eq!(meta_string_field(request, "service"), service);
    assert_eq!(meta_string_field(request, "entity"), entity);
    assert_eq!(meta_string_field(request, "operation"), operation);
    assert_eq!(meta_string_field(request, "name"), name);
    assert_eq!(event.resource.as_ref().unwrap().resource_id, service);
}

fn meta_string_field(request: &PublishEventRequest, key: &str) -> String {
    let fields = &request
        .event
        .as_ref()
        .unwrap()
        .message
        .as_ref()
        .unwrap()
        .fields;
    match fields.get(key).and_then(|value| value.kind.as_ref()) {
        Some(prost_types::value::Kind::StringValue(value)) => value.clone(),
        _ => panic!("field `{key}` missing or not a string"),
    }
}

// ---------------------------------------------------------------------------
// URL remapper end-to-end tests
//
// Validates that the forward URL-prefix remapper applies uniformly to the
// policy management API, the authorization API, and the diagnostics API. Uses
// the file-backed `config-file` storage (no Postgres) so tests stay
// hermetic and parallel-safe.
// ---------------------------------------------------------------------------

const REMAPPER_TEST_RULES_YAML: &str = r#"
enabled: true
rewriteRules:
  "omniverse://content.example.com/": "https://exampleaccount.blob.core.windows.net/examplecontainer/"
  "omniverse://content.example.com/NVIDIA/":
    target: "https://nvidia-bucket.s3.us-west2.aws.com/"
    description: "ignored by the parser"
"#;

fn write_url_remapper_file(content: &str) -> String {
    let path = std::env::temp_dir().join(format!(
        "url-remapper-{}-{}.yaml",
        std::process::id(),
        rand_suffix()
    ));
    std::fs::write(&path, content).expect("failed to write url-remapper file");
    path.to_string_lossy().into_owned()
}

async fn setup_remapper_server(remapper_yaml: Option<&str>) -> TestServer {
    let mut config = setup_config();
    if let Some(yaml) = remapper_yaml {
        config.url_remapper.enabled = true;
        config.url_remapper.file = write_url_remapper_file(yaml);
    }
    let cache = setup_cache(&config);
    let server = setup_server(config, cache).await;
    setup_metadata(&server, EvaluationPriority::Forbid).await;
    server
}

fn omniverse_resource(id: &str) -> Value {
    json!({
        "id": id,
        "type": "ResourceAddress",
    })
}

#[tokio::test]
async fn test_url_remapper_put_policy_rewrites_text_and_scope() {
    let server = setup_remapper_server(Some(REMAPPER_TEST_RULES_YAML)).await;

    let payload = json!({
        "policy": "permit(principal == Principal::\"test-user\", action == Action::\"tags:get\", resource == ResourceAddress::\"omniverse://content.example.com/NVIDIA/Astronaut.usd\");"
    });
    let response = server.put("/v1beta/policies/").json(&payload).await;
    assert_eq!(response.status_code(), 201, "{}", response.text());

    let result: Value = response.json();
    assert_eq!(
        result["policy"],
        json!(
            "permit(principal == Principal::\"test-user\", action == Action::\"tags:get\", resource == ResourceAddress::\"https://nvidia-bucket.s3.us-west2.aws.com/Astronaut.usd\");"
        ),
        "policy text must reflect the rewritten URL exactly"
    );
    assert_eq!(
        result["resource"]["id"],
        json!("https://nvidia-bucket.s3.us-west2.aws.com/Astronaut.usd"),
        "resource scope id must reflect the rewritten URL"
    );
}

#[tokio::test]
async fn test_url_remapper_put_policy_batch_rewrites_each_entry() {
    let server = setup_remapper_server(Some(REMAPPER_TEST_RULES_YAML)).await;

    let batches = json!([
        {
            "policy": "permit(principal == Principal::\"u1\", action == Action::\"tags:get\", resource == ResourceAddress::\"omniverse://content.example.com/NVIDIA/Astronaut.usd\");"
        },
        {
            "policy": "permit(principal == Principal::\"u2\", action == Action::\"tags:get\", resource == ResourceAddress::\"omniverse://content.example.com/Projects/scene.usd\");"
        }
    ]);
    let response = server.put("/v1beta/policies/batch/").json(&batches).await;
    assert_eq!(response.status_code(), 200, "{}", response.text());

    let result: Value = response.json();
    let items = result["results"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(
        items[0]["resource"]["id"],
        json!("https://nvidia-bucket.s3.us-west2.aws.com/Astronaut.usd"),
        "longest-prefix rule must apply to the NVIDIA path"
    );
    assert_eq!(
        items[1]["resource"]["id"],
        json!("https://exampleaccount.blob.core.windows.net/examplecontainer/Projects/scene.usd"),
        "shorter prefix rule must apply when the longer one does not match"
    );
}

#[tokio::test]
async fn test_url_remapper_get_policies_filters_by_rewritten_resource() {
    let server = setup_remapper_server(Some(REMAPPER_TEST_RULES_YAML)).await;

    let create = server
        .put("/v1beta/policies/")
        .json(&json!({
            "policy": "permit(principal == Principal::\"test-user\", action == Action::\"tags:get\", resource == ResourceAddress::\"omniverse://content.example.com/NVIDIA/Astronaut.usd\");"
        }))
        .await;
    assert_eq!(create.status_code(), 201);

    let list = server
        .get("/v1beta/policies/")
        .add_query_param(
            "resource",
            "ResourceAddress::\"omniverse://content.example.com/NVIDIA/Astronaut.usd\"",
        )
        .add_query_param("page", "1")
        .add_query_param("limit", "10")
        .await;
    assert_eq!(list.status_code(), 200, "{}", list.text());

    let result: Value = list.json();
    let items = result["items"].as_array().unwrap();
    assert_eq!(
        items.len(),
        1,
        "lookup by source URL must find the policy stored under the rewritten URL"
    );
    assert_eq!(
        items[0]["resource"]["id"],
        json!("https://nvidia-bucket.s3.us-west2.aws.com/Astronaut.usd")
    );
}

#[tokio::test]
async fn test_url_remapper_authorization_uses_rewritten_resource() {
    let server = setup_remapper_server(Some(REMAPPER_TEST_RULES_YAML)).await;

    let create = server
        .put("/v1beta/policies/")
        .json(&json!({
            "policy": "permit(principal == Principal::\"test-user\", action == Action::\"tags:get\", resource == ResourceAddress::\"omniverse://content.example.com/NVIDIA/Astronaut.usd\");"
        }))
        .await;
    assert_eq!(create.status_code(), 201);

    let principal = Principal {
        id: "test-user".to_string(),
        info: None,
    };
    let action = Action {
        service: "tags".to_string(),
        name: "get".to_string(),
    };
    let omniverse_res = Resource {
        id: "omniverse://content.example.com/NVIDIA/Astronaut.usd".into(),
        kind: "ResourceAddress".into(),
        data: None,
    };
    let rewritten_res = Resource {
        id: "https://nvidia-bucket.s3.us-west2.aws.com/Astronaut.usd".into(),
        kind: "ResourceAddress".into(),
        data: None,
    };

    let omniverse_decision = verify(&server, &principal, &action, Some(&omniverse_res)).await;
    let rewritten_decision = verify(&server, &principal, &action, Some(&rewritten_res)).await;
    assert_eq!(omniverse_decision, AuthorizationDecision::Allow);
    assert_eq!(omniverse_decision, rewritten_decision);
}

#[tokio::test]
async fn test_url_remapper_authorization_resolves_dot_segments_before_matching() {
    let server = setup_remapper_server(Some(REMAPPER_TEST_RULES_YAML)).await;

    let create = server
        .put("/v1beta/policies/")
        .json(&json!({
            "policy": "permit(principal == Principal::\"test-user\", action == Action::\"tags:get\", resource == ResourceAddress::\"https://nvidia-bucket.s3.us-west2.aws.com/Projects/scene.usd\");"
        }))
        .await;
    assert_eq!(create.status_code(), 201, "{}", create.text());

    let principal = Principal {
        id: "test-user".to_string(),
        info: None,
    };
    let action = Action {
        service: "tags".to_string(),
        name: "get".to_string(),
    };
    let dotted_res = Resource {
        id: "omniverse://content.example.com/NVIDIA/../Projects/scene.usd".into(),
        kind: "ResourceAddress".into(),
        data: None,
    };

    let decision = verify(&server, &principal, &action, Some(&dotted_res)).await;

    assert_eq!(
        decision,
        AuthorizationDecision::Deny,
        "a dot segment must not move the id under a longer rewrite rule"
    );
}

#[tokio::test]
async fn test_url_remapper_authorization_batch_rewrites_resource() {
    let server = setup_remapper_server(Some(REMAPPER_TEST_RULES_YAML)).await;

    let create = server
        .put("/v1beta/policies/")
        .json(&json!({
            "policy": "permit(principal == Principal::\"test-user\", action == Action::\"tags:get\", resource == ResourceAddress::\"omniverse://content.example.com/NVIDIA/Astronaut.usd\");"
        }))
        .await;
    assert_eq!(create.status_code(), 201);

    let response = server
        .post("/v1beta/authorization/batch/")
        .json(&json!({
            "batches": [
                {
                    "principal": { "sub": "test-user" },
                    "actions": [{ "service": "tags", "name": "get" }],
                    "resource": omniverse_resource("omniverse://content.example.com/NVIDIA/Astronaut.usd"),
                }
            ]
        }))
        .await;
    assert_eq!(response.status_code(), 200, "{}", response.text());

    let result: Value = response.json();
    let decision = &result["decisions"][0]["tags:get"]["decision"];
    assert_eq!(decision, &json!("allow"));
}

#[tokio::test]
async fn test_url_remapper_diagnostics_returns_policy_for_rewritten_resource() {
    let server = setup_remapper_server(Some(REMAPPER_TEST_RULES_YAML)).await;

    let create = server
        .put("/v1beta/policies/")
        .json(&json!({
            "policy": "permit(principal == Principal::\"test-user\", action == Action::\"tags:get\", resource == ResourceAddress::\"omniverse://content.example.com/NVIDIA/Astronaut.usd\");"
        }))
        .await;
    assert_eq!(create.status_code(), 201);

    let response = server
        .post("/v1beta/diagnostics/authorize/")
        .json(&json!({
            "principal": { "sub": "test-user" },
            "action": { "service": "tags", "name": "get" },
            "resource": omniverse_resource("omniverse://content.example.com/NVIDIA/Astronaut.usd"),
        }))
        .await;
    assert_eq!(response.status_code(), 200, "{}", response.text());

    let result: DiagnosticsAuthorizeResponse = response.json();
    assert_eq!(
        result.policies.len(),
        1,
        "diagnostics must return the rewritten policy"
    );
    let resource_id = result.policies[0]
        .resource
        .as_ref()
        .map(|r| r.id.clone())
        .unwrap_or_default();
    assert_eq!(
        resource_id,
        "https://nvidia-bucket.s3.us-west2.aws.com/Astronaut.usd"
    );
}

#[tokio::test]
async fn test_url_remapper_disabled_leaves_resources_unchanged() {
    let server = setup_remapper_server(None).await;

    let response = server
        .put("/v1beta/policies/")
        .json(&json!({
            "policy": "permit(principal == Principal::\"test-user\", action == Action::\"tags:get\", resource == ResourceAddress::\"omniverse://content.example.com/NVIDIA/Astronaut.usd\");"
        }))
        .await;
    assert_eq!(response.status_code(), 201, "{}", response.text());
    let result: Value = response.json();
    assert_eq!(
        result["policy"],
        json!(
            "permit(principal == Principal::\"test-user\", action == Action::\"tags:get\", resource == ResourceAddress::\"omniverse://content.example.com/NVIDIA/Astronaut.usd\");"
        ),
        "feature flag off must leave policy text unchanged"
    );
    assert_eq!(
        result["resource"]["id"],
        json!("omniverse://content.example.com/NVIDIA/Astronaut.usd")
    );
}

#[tokio::test]
async fn test_url_remapper_non_matching_url_passes_through() {
    let server = setup_remapper_server(Some(REMAPPER_TEST_RULES_YAML)).await;

    let response = server
        .put("/v1beta/policies/")
        .json(&json!({
            "policy": "permit(principal == Principal::\"test-user\", action == Action::\"tags:get\", resource == ResourceAddress::\"https://other.example.com/path\");"
        }))
        .await;
    assert_eq!(response.status_code(), 201, "{}", response.text());

    let result: Value = response.json();
    assert_eq!(
        result["resource"]["id"],
        json!("https://other.example.com/path"),
        "URLs that do not match any rule must pass through unchanged"
    );
}

async fn setup_remapper_auth_server(
    jwk: &JsonWebKeyFixture,
    remapper_yaml: &str,
    init_policies: &str,
) -> TestServer {
    let mut config = jwk.config();
    config.url_remapper.enabled = true;
    config.url_remapper.file = write_url_remapper_file(remapper_yaml);
    config.database.init.policies_file = write_init_policies_file(init_policies);
    let cache = setup_cache(&config);
    setup_server(config, cache).await
}

/// A `permissions:edit` grant scoped to a canonical `Folder` hierarchy governs
/// policies whose resource is written as an alias. The alias is remapped to its
/// canonical URL before the meta hierarchy check on both PUT and DELETE, so the
/// original "can edit by canonical URL but not by alias" bug stays fixed. Also
/// covers the equivalent canonical-form request and denies a sibling.
#[tokio::test]
async fn test_edit_permissions_hierarchy_scoped_with_url_remapper() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let init_policies = r#"
policies:
  - policy: 'permit(principal, action == Action::"permissions:edit", resource) when { principal.groups.contains("superadmins") };'
  - policy: 'permit(principal, action == Action::"permissions:meta:write", resource) when { principal.groups.contains("superadmins") };'
  - policy: 'permit(principal, action == Action::"permissions:edit", resource in Folder::"https://exampleaccount.blob.core.windows.net/examplecontainer/docs/") when { principal.groups.contains("docs-editors") };'
"#;
    let server = setup_remapper_auth_server(&jwk, REMAPPER_TEST_RULES_YAML, init_policies).await;

    let mut superadmin = Claims::default();
    superadmin.groups = vec!["superadmins".to_string()];
    let superadmin_token = jwk.encode(&superadmin);

    let mut editor = Claims::default();
    editor.groups = vec!["docs-editors".to_string()];
    let editor_token = jwk.encode(&editor);

    // Register the `object` kind with a `Folder` hierarchy parent.
    let response = server
        .put("/v1beta/services/storage/resource-types/object/")
        .add_header("Authorization", format!("Bearer {superadmin_token}"))
        .json(&json!({ "service": "storage", "type": "object", "evaluation_priority": "forbid", "parentType": "Folder" }))
        .await;
    assert_eq!(response.status_code(), 200);

    // Editor creates a policy whose resource is an ALIAS descendant of the
    // granted canonical folder; the alias must be remapped before the check.
    let alias_descendant = server
        .put("/v1beta/policies/")
        .add_header("Authorization", format!("Bearer {editor_token}"))
        .json(&json!({
            "policy": "permit(principal == Principal::\"bob\", action == Action::\"storage:read\", resource == object::\"omniverse://content.example.com/docs/reports/q1.pdf\");"
        }))
        .await;
    assert_eq!(
        alias_descendant.status_code(),
        201,
        "edit by alias must be allowed once remapped: {}",
        alias_descendant.text()
    );
    let alias_descendant_id = alias_descendant.json::<Value>()["id"].as_i64().unwrap();

    // The same descendant expressed in canonical form is equally allowed.
    let canonical_descendant = server
        .put("/v1beta/policies/")
        .add_header("Authorization", format!("Bearer {editor_token}"))
        .json(&json!({
            "policy": "permit(principal == Principal::\"carol\", action == Action::\"storage:read\", resource == object::\"https://exampleaccount.blob.core.windows.net/examplecontainer/docs/reports/q2.pdf\");"
        }))
        .await;
    assert_eq!(canonical_descendant.status_code(), 201);

    // A sibling outside the granted folder (via alias) must be denied.
    let alias_sibling = server
        .put("/v1beta/policies/")
        .add_header("Authorization", format!("Bearer {editor_token}"))
        .json(&json!({
            "policy": "permit(principal == Principal::\"bob\", action == Action::\"storage:read\", resource == object::\"omniverse://content.example.com/other/q1.pdf\");"
        }))
        .await;
    assert_eq!(alias_sibling.status_code(), 403);

    // Delete of the stored (canonical) descendant policy is allowed: the delete
    // path remaps + encodes the stored resource before the hierarchy check.
    let delete_descendant = server
        .delete(&format!("/v1beta/policies/{alias_descendant_id}"))
        .add_header("Authorization", format!("Bearer {editor_token}"))
        .await;
    assert_eq!(delete_descendant.status_code(), 204);

    // A sibling policy the editor cannot govern stays undeletable by them.
    let sibling = server
        .put("/v1beta/policies/")
        .add_header("Authorization", format!("Bearer {superadmin_token}"))
        .json(&json!({
            "policy": "permit(principal == Principal::\"bob\", action == Action::\"storage:read\", resource == object::\"omniverse://content.example.com/other/q3.pdf\");"
        }))
        .await;
    assert_eq!(sibling.status_code(), 201);
    let sibling_id = sibling.json::<Value>()["id"].as_i64().unwrap();

    let delete_sibling = server
        .delete(&format!("/v1beta/policies/{sibling_id}"))
        .add_header("Authorization", format!("Bearer {editor_token}"))
        .await;
    assert_eq!(delete_sibling.status_code(), 403);
}

/// The client `checkUserAccess` probe sends the real resource (no wrapper) with
/// the `permissions:view`/`permissions:edit` actions to the batch authorization
/// endpoint. This must agree with the internal enforcement path: a hierarchy
/// grant on the canonical folder resolves for an alias resource (remapped) and
/// via the kind-derived `Folder` parents, allowing a descendant and denying a
/// sibling.
#[tokio::test]
async fn test_check_user_access_probe_matches_hierarchy_with_remapper() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let init_policies = r#"
policies:
  - policy: 'permit(principal, action == Action::"permissions:meta:write", resource) when { principal.groups.contains("superadmins") };'
  - policy: 'permit(principal, action == Action::"permissions:edit", resource in Folder::"https://exampleaccount.blob.core.windows.net/examplecontainer/docs/") when { principal.groups.contains("docs-editors") };'
  - policy: 'permit(principal, action == Action::"permissions:view", resource in Folder::"https://exampleaccount.blob.core.windows.net/examplecontainer/docs/") when { principal.groups.contains("docs-editors") };'
"#;
    let server = setup_remapper_auth_server(&jwk, REMAPPER_TEST_RULES_YAML, init_policies).await;

    let mut superadmin = Claims::default();
    superadmin.groups = vec!["superadmins".to_string()];
    let superadmin_token = jwk.encode(&superadmin);

    let mut editor = Claims::default();
    editor.groups = vec!["docs-editors".to_string()];
    let editor_token = jwk.encode(&editor);

    let response = server
        .put("/v1beta/services/storage/resource-types/object/")
        .add_header("Authorization", format!("Bearer {superadmin_token}"))
        .json(&json!({ "service": "storage", "type": "object", "evaluation_priority": "forbid", "parentType": "Folder" }))
        .await;
    assert_eq!(response.status_code(), 200);

    // Probe the same shape `checkUserAccess` sends: the real resource by alias.
    let probe = |resource_id: &str| {
        json!({
            "batches": [{
                "actions": [
                    { "service": "permissions", "name": "view" },
                    { "service": "permissions", "name": "edit" }
                ],
                "resource": { "id": resource_id, "type": "object" }
            }]
        })
    };

    let descendant = server
        .post("/v1beta/authorization/batch/")
        .add_header("Authorization", format!("Bearer {editor_token}"))
        .json(&probe(
            "omniverse://content.example.com/docs/reports/q1.pdf",
        ))
        .await;
    assert_eq!(descendant.status_code(), 200, "{}", descendant.text());
    let descendant: Value = descendant.json();
    assert_eq!(
        descendant["decisions"][0]["permissions:view"]["decision"],
        "allow"
    );
    assert_eq!(
        descendant["decisions"][0]["permissions:edit"]["decision"],
        "allow"
    );

    let sibling = server
        .post("/v1beta/authorization/batch/")
        .add_header("Authorization", format!("Bearer {editor_token}"))
        .json(&probe("omniverse://content.example.com/other/q1.pdf"))
        .await;
    assert_eq!(sibling.status_code(), 200, "{}", sibling.text());
    let sibling: Value = sibling.json();
    assert_eq!(
        sibling["decisions"][0]["permissions:view"]["decision"],
        "deny"
    );
    assert_eq!(
        sibling["decisions"][0]["permissions:edit"]["decision"],
        "deny"
    );
}

/// Canonical body returned for every authorization 403. Mirrors the
/// `FORBIDDEN_RESPONSE_BODY` constant in `service::authorization` and
/// guards against accidental regressions to the old verbose Cedar-style
/// body that leaked the internal action name and principal sub.
const FORBIDDEN_RESPONSE_BODY: &str = "Permission denied";

/// Policies that grant the authenticated test user nothing — every
/// `permissions:*` action will deny, which exercises the 403 path on each
/// of the routes asserted below. Using an empty policy set keeps the
/// config-file backend happy without leaking permissions to the user.
const FORBIDDEN_TEST_POLICIES_YAML: &str = r#"
policies: []
"#;

/// Asserts that 403s from routes guarded by different inner permission
/// checks (`permissions:view` for `/policies` and `/authorization`,
/// `permissions:meta:read` for `/services`, `permissions:diagnostics` for
/// `/diagnostics`) all return the same canonical text/plain body. This is
/// the regression test for a reported inconsistency: previously
/// `/policies` and `/authorization` echoed a verbose Cedar-style message
/// including `permissions:view` and the requester sub, while `/services` and
/// `/diagnostics` returned plain `Permission denied`.
#[tokio::test]
async fn test_forbidden_response_body_is_uniform_across_routes() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let claims = Claims::default();
    let token = jwk.encode(&claims);

    let server =
        setup_config_file_server_with_auth(&jwk, FORBIDDEN_TEST_POLICIES_YAML, TEST_METADATA_YAML)
            .await;

    let auth_header = format!("Bearer {token}");
    let auth_body = json!({
        "principal": { "sub": "other-user" },
        "action": { "service": "tags", "name": "get" },
        "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
    });

    let policies = server
        .get("/v1beta/policies/")
        .add_header("Authorization", auth_header.clone())
        .await;
    let authorization = server
        .post("/v1beta/authorization/")
        .add_header("Authorization", auth_header.clone())
        .json(&auth_body)
        .await;
    let services = server
        .get("/v1beta/services/")
        .add_header("Authorization", auth_header.clone())
        .await;
    let diagnostics = server
        .post("/v1beta/diagnostics/authorize/")
        .add_header("Authorization", auth_header)
        .json(&auth_body)
        .await;

    for (label, response) in [
        ("/v1beta/policies/", &policies),
        ("/v1beta/authorization/", &authorization),
        ("/v1beta/services/", &services),
        ("/v1beta/diagnostics/authorize/", &diagnostics),
    ] {
        assert_eq!(
            response.status_code(),
            403,
            "{label} must produce a 403 for a caller without meta-permissions"
        );
        assert_eq!(
            response.text(),
            FORBIDDEN_RESPONSE_BODY,
            "{label} 403 body must be the canonical short form, not a verbose Cedar-style message"
        );
    }
}

/// The verbose deny reason produced by `ensure_permissions_can_be_viewed`
/// still surfaces inside the batch authorization payload's `reason` field
/// (gated by `enable_deny_reason`). Locking this in alongside the uniform
/// 403 body test makes sure the consistency fix did not regress the
/// existing per-action diagnostic reason on the batch endpoint.
#[tokio::test]
async fn test_forbidden_batch_reason_still_includes_internal_message() {
    let jwk = JsonWebKeyFixture::new_async().await;
    let claims = Claims::default();
    let token = jwk.encode(&claims);

    let server =
        setup_config_file_server_with_auth(&jwk, FORBIDDEN_TEST_POLICIES_YAML, TEST_METADATA_YAML)
            .await;

    let payload = json!({
        "batches": [
            {
                "principal": { "sub": "other-user" },
                "actions": [{ "name": "get", "service": "tags" }],
                "resource": { "id": "Astronaut.usd", "type": "ResourceAddress" }
            }
        ]
    });

    let response = server
        .post("/v1beta/authorization/batch/")
        .add_header("Authorization", format!("Bearer {token}"))
        .json(&payload)
        .await;
    assert_eq!(response.status_code(), 200);

    let result: Value = response.json();
    assert_eq!(
        result["decisions"][0]["tags:get"]["decision"],
        json!("deny")
    );
    assert_eq!(
        result["decisions"][0]["tags:get"]["reason"],
        json!("Permission denied for evaluating Action::\"permissions:view\" for Principal::\"test-user\"")
    );
}
