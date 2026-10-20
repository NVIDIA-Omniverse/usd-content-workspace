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

use crate::config_file::ConfigFilePolicyStorage;
use crate::pool::{ConnectionPool, DirectPool};
use crate::postgres::{PostgresDatabaseOptions, PostgresMetadataStorage, PostgresPolicyStorage};
use crate::{
    combine_policies, EvaluationPriority, Meta, MetadataStorage, PolicyDbBatch, PolicyStorage,
    ResourceTypeMeta, ServiceMeta, StorageErr, StorageErrKind,
};
use deadpool_postgres::{ManagerConfig, PoolConfig, RecyclingMethod, Runtime as PgRuntime};
use once_cell::sync::Lazy;
use permission_core::{canonicalize_policy, md5_hex, Action, Group, Principal, Resource};
use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio_postgres::NoTls;

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

const DEFAULT_ID_CLAIM: &str = "sub";
const DEFAULT_CREATED_BY: &str = "test-creator";
const SYSTEM_CREATED_BY: &str = "SYSTEM";

/// Tests construct `PolicyDbBatch` directly; this mirrors what the HTTP layer
/// computes via `permission_core::process_policy` so the dedup logic in
/// `add_policy_batch` sees real hashes.
fn test_canonical_hash(policy: &str) -> String {
    md5_hex(&canonicalize_policy(policy).expect("test policy must canonicalize"))
}

async fn connect_database() -> (PostgresPolicyStorage, PostgresMetadataStorage) {
    let host = std::env::var("POSTGRES_HOST").expect("POSTGRES_HOST should be specified.");
    let user = std::env::var("POSTGRES_USER").expect("POSTGRES_USER should be specified.");
    let password =
        std::env::var("POSTGRES_PASSWORD").expect("POSTGRES_PASSWORD should be specified.");
    let dbname = std::env::var("POSTGRES_DBNAME").expect("POSTGRES_DBNAME should be specified.");

    let mut pg_config = deadpool_postgres::Config::new();
    pg_config.dbname = Some(dbname.clone());
    pg_config.host = Some(host);
    pg_config.user = Some(user);
    pg_config.password = Some(password);
    pg_config.manager = Some(ManagerConfig {
        recycling_method: RecyclingMethod::Fast,
    });
    pg_config.pool = Some(PoolConfig::new(32));

    let pool: Arc<dyn ConnectionPool> = Arc::new(DirectPool::new(Arc::new(
        pg_config
            .create_pool(Some(PgRuntime::Tokio1), NoTls)
            .unwrap(),
    )));

    let options = PostgresDatabaseOptions { temporary: true };
    let policies = PostgresPolicyStorage::new(
        pool.clone(),
        POSTGRES_RUNTIME.clone(),
        dbname.as_str(),
        options.clone(),
    );
    policies.initialize().await.unwrap();

    let metadata = PostgresMetadataStorage::new(
        pool.clone(),
        POSTGRES_RUNTIME.clone(),
        dbname.as_str(),
        options.clone(),
        DEFAULT_ID_CLAIM.to_string(),
    );
    metadata.initialize().await.unwrap();

    (policies, metadata)
}

fn new_action() -> Action {
    Action {
        name: "read".to_string(),
        service: "storage".to_string(),
    }
}

fn new_principal() -> Principal {
    Principal {
        id: "test-user".to_string(),
        info: None,
    }
}

fn new_resource() -> Resource {
    Resource {
        id: "/test.data".to_string(),
        kind: "File".to_string(),
        data: None,
    }
}

fn new_resource_meta(service: &str, resource: &Resource) -> ResourceTypeMeta {
    ResourceTypeMeta::new(service, &resource.kind)
}

fn new_service_meta(service: &str) -> ServiceMeta {
    ServiceMeta {
        service: service.to_string(),
        id_claim: "email".to_string(),
    }
}

fn default_service_meta(service: &str) -> ServiceMeta {
    ServiceMeta {
        service: service.to_string(),
        id_claim: DEFAULT_ID_CLAIM.to_string(),
    }
}

#[tokio::test]
async fn test_get_policy() {
    let (policies, _) = connect_database().await;

    let policy = r#"
        permit(
            principal == Principal::"test-user",
            action == Action::"storage:read",
            resource == File::"/test.data"
        );
    "#;

    let action = new_action();
    let principal = new_principal();
    let resource = new_resource();

    let policy_id = policies
        .add_policy(
            policy,
            Some(&action),
            Some(&principal),
            None,
            Some(&resource),
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await;
    assert!(policy_id.is_ok());
    let policy_id = policy_id.unwrap();
    assert!(policy_id.created);
    let policy_id = policy_id.policy;
    assert_eq!(policy_id.id, 1);
    assert_eq!(policy_id.text, policy.trim().to_string());
    assert_eq!(policy_id.resource, Some(resource.clone()));
    assert_eq!(policy_id.action, Some(action.clone()));
    assert_eq!(policy_id.principal, Some(principal.clone()));
    assert_eq!(policy_id.created_by, DEFAULT_CREATED_BY.to_string());
    assert!(!policy_id.created_at.is_empty());

    let result = policies
        .get_policies(Some(&action), Some(&principal), &[], Some(&resource), &[])
        .await;
    assert!(result.is_ok());

    let policies = result.unwrap();
    assert_eq!(combine_policies(&policies), policy.trim());
}

#[tokio::test]
async fn test_get_policy_by_id() {
    let (policies, _) = connect_database().await;

    let text = r#"
        permit(
            principal == Principal::"test-user",
            action == Action::"storage:read",
            resource == File::"/test.data"
        );
    "#;

    let action = new_action();
    let principal = new_principal();
    let resource = new_resource();

    let policy = policies
        .add_policy(
            text,
            Some(&action),
            Some(&principal),
            None,
            Some(&resource),
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap()
        .policy;

    let result = policies.get_policy_by_id(&policy.id).await;
    assert!(result.is_ok());

    let db_policy = result.unwrap();
    assert_eq!(db_policy, policy);
}

#[tokio::test]
async fn test_get_policy_for_principal() {
    let (policies, _) = connect_database().await;

    let principal_policy = r#"
        permit(
            principal == Principal::"test-user",
            action == Action::"storage:read",
            resource == File::"/test.data"
        );
    "#;

    let action = new_action();
    let principal = new_principal();
    let resource = new_resource();

    let policy_id = policies
        .add_policy(
            principal_policy,
            Some(&action),
            Some(&principal),
            None,
            Some(&resource),
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await;
    assert!(policy_id.is_ok());
    let policy_id = policy_id.unwrap().policy;
    assert_eq!(policy_id.id, 1);
    assert_eq!(policy_id.text, principal_policy.trim().to_string());
    assert_eq!(policy_id.resource, Some(resource.clone()));
    assert_eq!(policy_id.action, Some(action.clone()));
    assert_eq!(policy_id.principal, Some(principal.clone()));
    assert_eq!(policy_id.created_by, DEFAULT_CREATED_BY.to_string());
    assert!(!policy_id.created_at.is_empty());

    let other_policy = r#"
        permit(
            principal == Principal::"other-user",
            action == Action::"storage:read",
            resource == File::"/test.data"
        );
    "#;
    let policy_id = policies
        .add_policy(
            other_policy,
            Some(&action),
            Some(&Principal {
                id: "other-user".to_string(),
                info: None,
            }),
            None,
            Some(&resource),
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await;
    assert!(policy_id.is_ok());
    let policy_id = policy_id.unwrap().policy;
    assert_eq!(policy_id.id, 2);
    assert_eq!(policy_id.text, other_policy.trim().to_string());
    assert_eq!(policy_id.resource, Some(resource.clone()));
    assert_eq!(policy_id.action, Some(action.clone()));
    assert_eq!(
        policy_id.principal,
        Some(Principal {
            id: "other-user".to_string(),
            info: None,
        })
    );
    assert_eq!(policy_id.created_by, DEFAULT_CREATED_BY.to_string());
    assert!(!policy_id.created_at.is_empty());

    let result = policies
        .get_policies(Some(&action), Some(&principal), &[], Some(&resource), &[])
        .await;
    assert!(result.is_ok());

    let policies = result.unwrap();
    assert_eq!(combine_policies(&policies), principal_policy.trim());
}

#[tokio::test]
async fn test_get_policy_without_resource() {
    let (policies, _) = connect_database().await;

    let policy = r#"
        permit(
            principal == Principal::"test-user",
            action == Action::"storage:read",
            resource
        ) when {
            resource is File
        };
    "#;

    let action = new_action();
    let principal = new_principal();

    policies
        .add_policy(
            policy,
            Some(&action),
            Some(&principal),
            None,
            None,
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap();

    let resource = new_resource();
    let result = policies
        .get_policies(Some(&action), Some(&principal), &[], Some(&resource), &[])
        .await;
    assert!(result.is_ok());

    let policies = result.unwrap();
    assert_eq!(combine_policies(&policies), policy.trim());
}

#[tokio::test]
async fn test_get_policy_without_principal() {
    let (policies, _) = connect_database().await;

    let policy = r#"
        permit(
            principal,
            action == Action::"storage:read",
            resource
        ) when {
            resource is File &&
            resource.id == "/test.data"
        };
    "#;

    let action = new_action();
    policies
        .add_policy(
            policy,
            Some(&action),
            None,
            None,
            None,
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap();

    let resource = new_resource();
    let principal = new_principal();
    let result = policies
        .get_policies(Some(&action), Some(&principal), &[], Some(&resource), &[])
        .await;
    assert!(result.is_ok());

    let policies = result.unwrap();
    assert_eq!(combine_policies(&policies), policy.trim());
}

#[tokio::test]
async fn test_get_policy_without_action() {
    let (policies, _) = connect_database().await;
    let policy = r#"
        permit(
            principal,
            action,
            resource
        ) when {
            principal.groups.contains("admin")
        };
    "#;

    policies
        .add_policy(
            policy,
            None,
            None,
            None,
            None,
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap();

    let action = new_action();
    let resource = new_resource();
    let principal = new_principal();
    let result = policies
        .get_policies(Some(&action), Some(&principal), &[], Some(&resource), &[])
        .await;
    assert!(result.is_ok());

    let policies = result.unwrap();
    assert_eq!(combine_policies(&policies), policy.trim());
}

#[tokio::test]
async fn test_get_policies_paginated() {
    let (policies, _) = connect_database().await;

    let mut batches = vec![];
    for i in 1..=5 {
        let text = format!(
            "permit(\
                principal == Principal::\"test-{i}\", \
                action == Action::\"storage:read\", \
                resource == File::\"/test.data\" \
            );",
        );
        let mut principal = new_principal();
        principal.id = format!("test-{i}");
        let action = new_action();
        let resource = new_resource();
        let canonical_hash = test_canonical_hash(&text);
        batches.push(PolicyDbBatch {
            policy: text,
            action: Some(action),
            principal: Some(principal),
            resource: Some(resource),
            group: None,
            order: 0,
            system: false,
            created_by: DEFAULT_CREATED_BY.to_string(),
            comment: None,
            canonical_hash,
            resource_prefix: None,
        });
    }

    policies.add_policy_batch(batches).await.unwrap();

    let page = 1;
    let limit = 10;
    let page_result = policies
        .get_policies_paginated(None, None, None, None, &[], None, page, limit)
        .await
        .unwrap();
    assert_eq!(page_result.items.len(), 5);

    for policy in page_result.items.iter() {
        let text = format!(
            "permit(\
                principal == Principal::\"test-{}\", \
                action == Action::\"storage:read\", \
                resource == File::\"/test.data\" \
            );",
            policy.id
        );
        assert_eq!(text, policy.text);
    }
}

#[tokio::test]
async fn test_resource_prefix_persistence_and_matching() {
    let (policies, _) = connect_database().await;

    let prefix = Resource {
        id: "https://main/docs/".to_string(),
        kind: "folder".to_string(),
        data: None,
    };
    let text = format!(
        "permit(principal, action, resource in {}::\"{}\");",
        prefix.kind, prefix.id
    );
    let canonical_hash = test_canonical_hash(&text);

    let batches = vec![PolicyDbBatch {
        policy: text,
        action: None,
        principal: None,
        resource: None,
        group: None,
        order: 0,
        system: false,
        created_by: DEFAULT_CREATED_BY.to_string(),
        comment: None,
        canonical_hash,
        resource_prefix: Some(prefix.clone()),
    }];
    policies.add_policy_batch(batches).await.unwrap();

    let exact = policies
        .get_policies(None, None, &[], None, std::slice::from_ref(&prefix))
        .await
        .unwrap();
    assert_eq!(exact.len(), 1, "querying the exact prefix must return it");
    assert_eq!(exact[0].resource_prefix, Some(prefix.clone()));

    let ancestors = [
        Resource {
            id: "https://main/".to_string(),
            kind: "folder".to_string(),
            data: None,
        },
        prefix.clone(),
        Resource {
            id: "https://main/docs/reports/".to_string(),
            kind: "folder".to_string(),
            data: None,
        },
    ];
    let descendant = policies
        .get_policies(None, None, &[], None, &ancestors)
        .await
        .unwrap();
    assert_eq!(
        descendant.len(),
        1,
        "a descendant's ancestor set containing the prefix must return it"
    );
    assert_eq!(descendant[0].resource_prefix, Some(prefix.clone()));

    let unrelated = Resource {
        id: "https://other/docs/".to_string(),
        kind: "folder".to_string(),
        data: None,
    };
    let miss = policies
        .get_policies(None, None, &[], None, std::slice::from_ref(&unrelated))
        .await
        .unwrap();
    assert!(
        miss.is_empty(),
        "an unrelated prefix must not match the hierarchy policy"
    );
}

#[tokio::test]
async fn test_get_policies_paginated_with_principal() {
    let (policies, _) = connect_database().await;

    let action = new_action();
    let resource = new_resource();

    let principal = new_principal();
    let policy = "permit(\
            principal == Principal::\"test-user\", \
            action == Action::\"storage:read\", \
            resource == File::\"/test.data\"
        );";
    policies
        .add_policy(
            policy,
            Some(&action),
            Some(&principal),
            None,
            Some(&resource),
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap();

    let mut other_user = new_principal();
    other_user.id = "other-user".to_string();

    let other_policy = "permit(\
            principal == Principal::\"other-user\", \
            action == Action::\"storage:read\", \
            resource == File::\"/test.data\"
        );";
    policies
        .add_policy(
            other_policy,
            Some(&action),
            Some(&other_user),
            None,
            Some(&resource),
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap();

    let global_policy = "permit(principal, action, resource);";
    policies
        .add_policy(
            global_policy,
            None,
            None,
            None,
            None,
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap();

    let page_result = policies
        .get_policies_paginated(
            Some(&action),
            Some(&principal),
            None,
            Some(&resource),
            &[],
            None,
            1,
            10,
        )
        .await
        .unwrap();
    assert_eq!(page_result.items.len(), 1);

    assert_eq!(page_result.items[0].id, 1);
    assert_eq!(page_result.items[0].action, Some(action));
    assert_eq!(page_result.items[0].principal, Some(principal));
    assert_eq!(page_result.items[0].resource, Some(resource));
    assert_eq!(page_result.items[0].text, policy.to_string());
    assert_eq!(
        page_result.items[0].created_by,
        DEFAULT_CREATED_BY.to_string()
    );
    assert!(!page_result.items[0].created_at.is_empty());
}

#[tokio::test]
async fn test_get_policies_paginated_with_action() {
    let (policies, _) = connect_database().await;

    let action = new_action();
    let resource = new_resource();

    let principal = new_principal();
    let policy = "permit(\
            principal == Principal::\"test-user\", \
            action == Action::\"storage:read\", \
            resource == File::\"/test.data\"
        );";
    policies
        .add_policy(
            policy,
            Some(&action),
            Some(&principal),
            None,
            Some(&resource),
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap();

    let mut other_action = new_action();
    other_action.name = "write".to_string();

    let other_policy = "permit(\
            principal == Principal::\"test-user\", \
            action == Action::\"storage:write\", \
            resource == File::\"/test.data\"
        );";
    policies
        .add_policy(
            other_policy,
            Some(&other_action),
            Some(&principal),
            None,
            Some(&resource),
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap();

    let global_policy = "permit(principal, action, resource);";
    policies
        .add_policy(
            global_policy,
            None,
            None,
            None,
            None,
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap();

    let page_result = policies
        .get_policies_paginated(
            Some(&action),
            Some(&principal),
            None,
            Some(&resource),
            &[],
            None,
            1,
            10,
        )
        .await
        .unwrap();
    assert_eq!(page_result.items.len(), 1);

    assert_eq!(page_result.items[0].id, 1);
    assert_eq!(page_result.items[0].action, Some(action));
    assert_eq!(page_result.items[0].principal, Some(principal));
    assert_eq!(page_result.items[0].resource, Some(resource));
    assert_eq!(page_result.items[0].text, policy.to_string());
    assert_eq!(
        page_result.items[0].created_by,
        DEFAULT_CREATED_BY.to_string()
    );
    assert!(!page_result.items[0].created_at.is_empty());
}

#[tokio::test]
async fn test_get_policies_paginated_with_resource() {
    let (policies, _) = connect_database().await;

    let action = new_action();
    let resource = new_resource();

    let principal = new_principal();
    let policy = "permit(\
            principal == Principal::\"test-user\", \
            action == Action::\"storage:read\", \
            resource == File::\"/test.data\"
        );";
    policies
        .add_policy(
            policy,
            Some(&action),
            Some(&principal),
            None,
            Some(&resource),
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap();

    let mut other_resource = new_resource();
    other_resource.id = "scene.usd".to_string();

    let other_policy = "permit(\
            principal == Principal::\"test-user\", \
            action == Action::\"storage:write\", \
            resource == File::\"/test.data\"
        );";
    policies
        .add_policy(
            other_policy,
            Some(&action),
            Some(&principal),
            None,
            Some(&other_resource),
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap();

    let global_policy = "permit(principal, action, resource);";
    policies
        .add_policy(
            global_policy,
            None,
            None,
            None,
            None,
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap();

    let page_result = policies
        .get_policies_paginated(
            Some(&action),
            Some(&principal),
            None,
            Some(&resource),
            &[],
            None,
            1,
            10,
        )
        .await
        .unwrap();
    assert_eq!(page_result.items.len(), 1);

    assert_eq!(page_result.items[0].id, 1);
    assert_eq!(page_result.items[0].action, Some(action));
    assert_eq!(page_result.items[0].principal, Some(principal));
    assert_eq!(page_result.items[0].resource, Some(resource));
    assert_eq!(page_result.items[0].text, policy.to_string());
    assert_eq!(
        page_result.items[0].created_by,
        DEFAULT_CREATED_BY.to_string()
    );
    assert!(!page_result.items[0].created_at.is_empty());
}

#[tokio::test]
async fn test_get_policies_paginated_invalid_page() {
    let (policies, _) = connect_database().await;
    let result = policies
        .get_policies_paginated(None, None, None, None, &[], None, -1, 10)
        .await;
    match result {
        Err(StorageErr {
            kind: StorageErrKind::InvalidPagination,
            msg,
        }) => {
            assert_eq!(&msg, "`page` must be greater than 1.");
        }
        result => panic!("{:?}", result),
    }
}

#[tokio::test]
async fn test_get_policies_paginated_invalid_limit() {
    let (policies, _) = connect_database().await;

    let result = policies
        .get_policies_paginated(None, None, None, None, &[], None, 1, 0)
        .await;
    match result {
        Err(StorageErr {
            kind: StorageErrKind::InvalidPagination,
            msg,
        }) => {
            assert_eq!(&msg, "`limit` must be in range between 0 and 50.");
        }
        result => panic!("{:?}", result),
    }

    let result = policies
        .get_policies_paginated(None, None, None, None, &[], None, 1, 100)
        .await;
    match result {
        Err(StorageErr {
            kind: StorageErrKind::InvalidPagination,
            msg,
        }) => {
            assert_eq!(&msg, "`limit` must be in range between 0 and 50.");
        }
        result => panic!("{:?}", result),
    }
}

#[tokio::test]
async fn test_remove_policy() {
    let (policies, _) = connect_database().await;
    let text = r#"
        permit(
            principal,
            action,
            resource
        ) when {
            principal.groups.contains("admin")
        };
    "#;

    let policy = policies
        .add_policy(
            text,
            None,
            None,
            None,
            None,
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap()
        .policy;
    policies.remove_policy(&policy.id).await.unwrap();

    let action = new_action();
    let resource = new_resource();
    let principal = new_principal();
    let result = policies
        .get_policies(Some(&action), Some(&principal), &[], Some(&resource), &[])
        .await
        .unwrap();

    assert_eq!(result.len(), 0);
}

#[tokio::test]
async fn test_get_meta() {
    let (_, meta) = connect_database().await;
    let action = new_action();
    let resource = new_resource();

    let service_meta = new_service_meta(&action.service);
    meta.set_service_meta(&service_meta).await.unwrap();

    let resource_meta = new_resource_meta(&action.service, &resource);
    meta.set_resource_type_meta(&resource_meta).await.unwrap();

    let result = meta.get_meta(&action, Some(&resource)).await.unwrap();
    assert_eq!(
        result,
        Meta {
            service: service_meta,
            resource: resource_meta,
        }
    );
}

#[tokio::test]
async fn test_get_resource_types() {
    let (_, meta) = connect_database().await;
    let service = "storage";

    let resource_types = vec![
        ResourceTypeMeta {
            service: service.to_string(),
            kind: "File".to_string(),
            evaluation_priority: EvaluationPriority::Permit,
            parent_type: None,
        },
        ResourceTypeMeta {
            service: service.to_string(),
            kind: "Folder".to_string(),
            evaluation_priority: EvaluationPriority::Forbid,
            parent_type: None,
        },
        ResourceTypeMeta {
            service: service.to_string(),
            kind: "Bucket".to_string(),
            evaluation_priority: EvaluationPriority::Permit,
            parent_type: None,
        },
    ];

    for resource_type in &resource_types {
        meta.set_resource_type_meta(resource_type).await.unwrap();
    }

    let result = meta.get_service_resource_types(service).await.unwrap();
    assert_eq!(result.len(), resource_types.len());

    let mut result = result;
    result.sort_by(|a, b| a.kind.cmp(&b.kind));
    let mut expected = resource_types;
    expected.sort_by(|a, b| a.kind.cmp(&b.kind));

    for (actual, expected) in result.iter().zip(expected.iter()) {
        assert_eq!(actual.service, expected.service);
        assert_eq!(actual.kind, expected.kind);
        assert_eq!(actual.evaluation_priority, expected.evaluation_priority);
    }
}

#[tokio::test]
async fn test_get_default_meta() {
    let (_, meta) = connect_database().await;
    let action = new_action();
    let resource = new_resource();

    let result = meta.get_meta(&action, Some(&resource)).await.unwrap();
    assert_eq!(
        result,
        Meta {
            service: default_service_meta(&action.service),
            resource: ResourceTypeMeta::new(&action.service, &resource.kind),
        }
    );
}

#[tokio::test]
async fn test_get_meta_without_resource_meta() {
    let (_, meta) = connect_database().await;
    let action = new_action();
    let resource = new_resource();

    let service_meta = new_service_meta(&action.service);
    meta.set_service_meta(&service_meta).await.unwrap();

    let result = meta.get_meta(&action, Some(&resource)).await.unwrap();
    assert_eq!(
        result,
        Meta {
            service: service_meta,
            resource: ResourceTypeMeta::new(&action.service, &resource.kind),
        }
    );
}

#[tokio::test]
async fn test_get_meta_with_multiple_resource_meta() {
    let (_, meta) = connect_database().await;
    let action = new_action();

    let service_meta = new_service_meta(&action.service);
    meta.set_service_meta(&service_meta).await.unwrap();

    let mut folder = new_resource();
    folder.kind = "Folder".to_string();

    meta.set_resource_type_meta(&ResourceTypeMeta {
        service: action.service.clone(),
        kind: "Folder".to_string(),
        evaluation_priority: EvaluationPriority::Permit,
        parent_type: None,
    })
    .await
    .unwrap();

    let mut file = new_resource();
    file.kind = "File".to_string();

    meta.set_resource_type_meta(&ResourceTypeMeta {
        service: action.service.clone(),
        kind: "File".to_string(),
        evaluation_priority: EvaluationPriority::Permit,
        parent_type: None,
    })
    .await
    .unwrap();

    let result = meta.get_meta(&action, Some(&file)).await.unwrap();
    assert_eq!(
        result,
        Meta {
            service: service_meta.clone(),
            resource: ResourceTypeMeta {
                service: action.service.clone(),
                kind: file.kind.clone(),
                evaluation_priority: EvaluationPriority::Permit,
                parent_type: None,
            }
        }
    );

    let result = meta.get_meta(&action, Some(&folder)).await.unwrap();
    assert_eq!(
        result,
        Meta {
            service: service_meta.clone(),
            resource: ResourceTypeMeta {
                service: action.service.clone(),
                kind: folder.kind.clone(),
                evaluation_priority: EvaluationPriority::Permit,
                parent_type: None,
            }
        }
    );

    let result = meta.get_meta(&action, None).await.unwrap();
    assert_eq!(
        result,
        Meta {
            service: service_meta.clone(),
            resource: ResourceTypeMeta {
                service: action.service.clone(),
                kind: String::new(),
                evaluation_priority: EvaluationPriority::default(),
                parent_type: None,
            }
        }
    );
}

#[tokio::test]
async fn test_get_meta_without_service_meta() {
    let (_, meta) = connect_database().await;
    let action = new_action();
    let resource = new_resource();

    let resource_meta = new_resource_meta(&action.service, &resource);
    meta.set_resource_type_meta(&resource_meta).await.unwrap();

    let result = meta.get_meta(&action, Some(&resource)).await.unwrap();
    assert_eq!(
        result,
        Meta {
            service: default_service_meta(&action.service),
            resource: resource_meta,
        }
    );
}

#[tokio::test]
async fn test_delete_resource_meta() {
    let (_, meta) = connect_database().await;
    let action = new_action();
    let resource = new_resource();

    let resource_meta = new_resource_meta(&action.service, &resource);
    meta.set_resource_type_meta(&resource_meta).await.unwrap();
    meta.delete_resource_type_meta(&action.service, &resource.kind)
        .await
        .unwrap();

    let result = meta
        .get_resource_type(&action.service, &resource.kind)
        .await
        .unwrap();
    assert_eq!(
        result,
        ResourceTypeMeta::new(&action.service, &resource.kind)
    );
}

#[tokio::test]
async fn test_delete_service_meta() {
    let (_, meta) = connect_database().await;
    let action = new_action();

    let service_meta = new_service_meta(&action.service);
    meta.set_service_meta(&service_meta).await.unwrap();
    meta.delete_service_meta(&action.service).await.unwrap();

    let result = meta.get_service_meta(&action.service).await.unwrap();
    assert_eq!(result, default_service_meta(&action.service));
}

#[tokio::test]
async fn test_get_services() {
    let (_, meta) = connect_database().await;

    let services = vec![
        ServiceMeta {
            service: "storage".to_string(),
            id_claim: "email".to_string(),
        },
        ServiceMeta {
            service: "compute".to_string(),
            id_claim: "sub".to_string(),
        },
        ServiceMeta {
            service: "network".to_string(),
            id_claim: "username".to_string(),
        },
    ];
    for service in &services {
        meta.set_service_meta(service).await.unwrap();
    }

    let result = meta.get_services().await.unwrap();
    assert_eq!(result.len(), services.len());

    let mut result = result;
    result.sort_by(|a, b| a.service.cmp(&b.service));
    let mut expected = services;
    expected.sort_by(|a, b| a.service.cmp(&b.service));

    for (actual, expected) in result.iter().zip(expected.iter()) {
        assert_eq!(actual.service, expected.service);
        assert_eq!(actual.id_claim, expected.id_claim);
    }
}

#[tokio::test]
async fn test_get_service_actions() {
    let (_, meta) = connect_database().await;
    let service = "storage";
    let action_name = "read";

    let action = meta.set_service_action(service, action_name).await.unwrap();
    assert_eq!(action.service, service);
    assert_eq!(action.name, action_name);

    let actions = meta.get_service_actions(service).await.unwrap();
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0], action);
}

#[tokio::test]
async fn test_set_multiple_service_actions() {
    let (_, meta) = connect_database().await;
    let service = "storage";
    let action_names = vec!["read", "write", "delete"];

    for action_name in &action_names {
        meta.set_service_action(service, action_name).await.unwrap();
    }

    let actions = meta.get_service_actions(service).await.unwrap();
    assert_eq!(actions.len(), action_names.len());

    let mut actual_actions: Vec<String> = actions.iter().map(|a| a.name.clone()).collect();
    actual_actions.sort();
    let mut expected_actions = action_names.clone();
    expected_actions.sort();

    assert_eq!(actual_actions, expected_actions);
}

#[tokio::test]
async fn test_delete_service_action() {
    let (_, meta) = connect_database().await;
    let service = "storage";
    let action_name = "read";

    meta.set_service_action(service, action_name).await.unwrap();
    meta.delete_service_action(service, action_name)
        .await
        .unwrap();

    let actions = meta.get_service_actions(service).await.unwrap();
    assert_eq!(actions.len(), 0);
}

#[tokio::test]
async fn test_delete_unknown_action() {
    let (_, meta) = connect_database().await;
    let service = "storage";
    let action_name = "unknown";

    meta.delete_service_action(service, action_name)
        .await
        .unwrap();

    let actions = meta.get_service_actions(service).await.unwrap();
    assert_eq!(actions.len(), 0);
}

#[tokio::test]
async fn test_set_duplicate_service_action() {
    let (_, meta) = connect_database().await;
    let service = "storage";
    let action_name = "read";

    let action1 = meta.set_service_action(service, action_name).await.unwrap();
    let action2 = meta.set_service_action(service, action_name).await.unwrap();

    assert_eq!(action1, action2);

    let actions = meta.get_service_actions(service).await.unwrap();
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0], action1);
}

#[tokio::test]
async fn test_replace_service_actions() {
    let (_, meta) = connect_database().await;
    let service = "storage";

    let initial_actions = vec!["read", "write", "delete"];
    for action in &initial_actions {
        meta.set_service_action(service, action).await.unwrap();
    }

    let actions = meta.get_service_actions(service).await.unwrap();
    assert_eq!(actions.len(), initial_actions.len());

    // Replace with new set of actions
    let new_actions = vec![
        Action {
            service: service.to_string(),
            name: "create".to_string(),
        },
        Action {
            service: service.to_string(),
            name: "update".to_string(),
        },
    ];

    meta.replace_service_actions(service, new_actions.clone())
        .await
        .unwrap();

    // Verify old actions are gone and new ones are set
    let actions = meta.get_service_actions(service).await.unwrap();
    assert_eq!(actions.len(), new_actions.len());

    let mut actual_actions: Vec<String> = actions.iter().map(|a| a.name.clone()).collect();
    actual_actions.sort();
    let mut expected_actions: Vec<String> = new_actions.iter().map(|a| a.name.clone()).collect();
    expected_actions.sort();

    assert_eq!(actual_actions, expected_actions);
}

#[tokio::test]
async fn test_replace_service_actions_empty() {
    let (_, meta) = connect_database().await;
    let service = "storage";

    let initial_actions = vec!["read", "write", "delete"];
    for action in &initial_actions {
        meta.set_service_action(service, action).await.unwrap();
    }

    meta.replace_service_actions(service, vec![]).await.unwrap();

    let actions = meta.get_service_actions(service).await.unwrap();
    assert_eq!(actions.len(), 0);
}

#[tokio::test]
async fn test_replace_resource_types() {
    let (_, meta) = connect_database().await;
    let service = "storage";

    let initial_types = vec![
        ResourceTypeMeta {
            service: service.to_string(),
            kind: "File".to_string(),
            evaluation_priority: EvaluationPriority::Permit,
            parent_type: None,
        },
        ResourceTypeMeta {
            service: service.to_string(),
            kind: "Folder".to_string(),
            evaluation_priority: EvaluationPriority::Forbid,
            parent_type: None,
        },
    ];

    for resource_type in &initial_types {
        meta.set_resource_type_meta(resource_type).await.unwrap();
    }

    let types = meta.get_service_resource_types(service).await.unwrap();
    assert_eq!(types.len(), initial_types.len());

    // Replace with new set of resource types
    let new_types = vec![
        ResourceTypeMeta {
            service: service.to_string(),
            kind: "Bucket".to_string(),
            evaluation_priority: EvaluationPriority::Permit,
            parent_type: None,
        },
        ResourceTypeMeta {
            service: service.to_string(),
            kind: "Object".to_string(),
            evaluation_priority: EvaluationPriority::Forbid,
            parent_type: None,
        },
    ];

    meta.replace_resource_types(service, new_types.clone())
        .await
        .unwrap();

    // Verify old types are gone and new ones are set
    let types = meta.get_service_resource_types(service).await.unwrap();
    assert_eq!(types.len(), new_types.len());

    let mut actual_types: Vec<(String, EvaluationPriority)> = types
        .iter()
        .map(|t| (t.kind.clone(), t.evaluation_priority.clone()))
        .collect();
    actual_types.sort_by(|a, b| a.0.cmp(&b.0));

    let mut expected_types: Vec<(String, EvaluationPriority)> = new_types
        .iter()
        .map(|t| (t.kind.clone(), t.evaluation_priority.clone()))
        .collect();
    expected_types.sort_by(|a, b| a.0.cmp(&b.0));

    assert_eq!(actual_types, expected_types);
}

#[tokio::test]
async fn test_replace_resource_types_empty() {
    let (_, meta) = connect_database().await;
    let service = "storage";

    let initial_types = vec![
        ResourceTypeMeta {
            service: service.to_string(),
            kind: "File".to_string(),
            evaluation_priority: EvaluationPriority::Permit,
            parent_type: None,
        },
        ResourceTypeMeta {
            service: service.to_string(),
            kind: "Folder".to_string(),
            evaluation_priority: EvaluationPriority::Forbid,
            parent_type: None,
        },
    ];

    for resource_type in &initial_types {
        meta.set_resource_type_meta(resource_type).await.unwrap();
    }

    meta.replace_resource_types(service, vec![]).await.unwrap();

    let types = meta.get_service_resource_types(service).await.unwrap();
    assert_eq!(types.len(), 0);
}

#[tokio::test]
async fn test_get_resource_type_by_kind_returns_single_owner() {
    let (_, meta) = connect_database().await;

    let resource_meta = ResourceTypeMeta {
        service: "storage".to_string(),
        kind: "Object".to_string(),
        evaluation_priority: EvaluationPriority::Permit,
        parent_type: Some("Folder".to_string()),
    };
    meta.set_resource_type_meta(&resource_meta).await.unwrap();

    let resolved = meta
        .get_resource_type_by_kind("Object")
        .await
        .unwrap()
        .expect("registered kind resolves");
    assert_eq!(resolved.service, "storage");
    assert_eq!(resolved.kind, "Object");
    assert_eq!(resolved.parent_type.as_deref(), Some("Folder"));
    assert_eq!(resolved.evaluation_priority, EvaluationPriority::Permit);

    let missing = meta.get_resource_type_by_kind("Unknown").await.unwrap();
    assert!(missing.is_none());
}

#[tokio::test]
async fn test_set_resource_type_meta_cross_service_conflict() {
    let (_, meta) = connect_database().await;

    meta.set_resource_type_meta(&ResourceTypeMeta {
        service: "storage".to_string(),
        kind: "Widget".to_string(),
        evaluation_priority: EvaluationPriority::Forbid,
        parent_type: None,
    })
    .await
    .unwrap();

    let err = meta
        .set_resource_type_meta(&ResourceTypeMeta {
            service: "billing".to_string(),
            kind: "Widget".to_string(),
            evaluation_priority: EvaluationPriority::Forbid,
            parent_type: None,
        })
        .await
        .expect_err("cross-service duplicate kind must conflict");
    assert_eq!(err.kind, StorageErrKind::Conflict);

    // The original owner is unchanged.
    let resolved = meta
        .get_resource_type_by_kind("Widget")
        .await
        .unwrap()
        .expect("kind still resolves to its owner");
    assert_eq!(resolved.service, "storage");
}

#[tokio::test]
async fn test_replace_resource_types_conflict_rolls_back() {
    let (_, meta) = connect_database().await;

    meta.set_resource_type_meta(&ResourceTypeMeta {
        service: "storage".to_string(),
        kind: "Shared".to_string(),
        evaluation_priority: EvaluationPriority::Forbid,
        parent_type: None,
    })
    .await
    .unwrap();

    let billing_types = vec![
        ResourceTypeMeta {
            service: "billing".to_string(),
            kind: "Invoice".to_string(),
            evaluation_priority: EvaluationPriority::Forbid,
            parent_type: None,
        },
        ResourceTypeMeta {
            service: "billing".to_string(),
            kind: "Account".to_string(),
            evaluation_priority: EvaluationPriority::Permit,
            parent_type: None,
        },
    ];
    for resource_type in &billing_types {
        meta.set_resource_type_meta(resource_type).await.unwrap();
    }

    // Replacing with a set that steals another service's kind must fail and
    // leave billing's original resource types intact (transactional rollback).
    let err = meta
        .replace_resource_types(
            "billing",
            vec![
                ResourceTypeMeta {
                    service: "billing".to_string(),
                    kind: "Report".to_string(),
                    evaluation_priority: EvaluationPriority::Forbid,
                    parent_type: None,
                },
                ResourceTypeMeta {
                    service: "billing".to_string(),
                    kind: "Shared".to_string(),
                    evaluation_priority: EvaluationPriority::Forbid,
                    parent_type: None,
                },
            ],
        )
        .await
        .expect_err("replace stealing another service's kind must conflict");
    assert_eq!(err.kind, StorageErrKind::Conflict);

    let mut kinds: Vec<String> = meta
        .get_service_resource_types("billing")
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.kind)
        .collect();
    kinds.sort();
    assert_eq!(kinds, vec!["Account".to_string(), "Invoice".to_string()]);
}

fn resource_type_config(kind: &str) -> crate::MetadataResourceTypeConfig {
    crate::MetadataResourceTypeConfig {
        kind: kind.to_string(),
        evaluation_priority: EvaluationPriority::Forbid,
        parent_type: None,
    }
}

fn service_config(name: &str, kinds: &[&str]) -> crate::MetadataServiceConfig {
    crate::MetadataServiceConfig {
        name: name.to_string(),
        principal: crate::MetadataPrincipalConfig::default(),
        actions: Vec::new(),
        resource_types: kinds.iter().map(|k| resource_type_config(k)).collect(),
    }
}

#[test]
fn test_validate_unique_resource_types_accepts_distinct_kinds() {
    let config = crate::MetadataConfig {
        services: vec![
            service_config("storage", &["File", "Folder"]),
            service_config("billing", &["Invoice"]),
        ],
    };
    assert!(config.validate_unique_resource_types().is_ok());
}

#[test]
fn test_validate_unique_resource_types_rejects_cross_service_duplicate() {
    let config = crate::MetadataConfig {
        services: vec![
            service_config("storage", &["File", "Shared"]),
            service_config("billing", &["Shared"]),
        ],
    };
    let err = config
        .validate_unique_resource_types()
        .expect_err("duplicate kind across services must be rejected");
    assert!(
        err.contains("Shared"),
        "error should name the conflict: {err}"
    );
}

#[tokio::test]
async fn test_get_policies_paginated_with_wildcard_action() {
    let (policies, _) = connect_database().await;

    let service = "storage";
    let principal = new_principal();
    let resource = new_resource();

    let actions = vec!["read", "write", "delete", "list"];
    let mut created_policies = Vec::new();

    for action_name in &actions {
        let action = Action {
            name: action_name.to_string(),
            service: service.to_string(),
        };
        let policy_text = format!(
            "permit(\
                principal == Principal::\"{}\", \
                action == Action::\"{}:{}\", \
                resource == {}::\"{}\" \
            );",
            principal.id, service, action_name, resource.kind, resource.id
        );

        let policy = policies
            .add_policy(
                &policy_text,
                Some(&action),
                Some(&principal),
                None,
                Some(&resource),
                None,
                0,
                DEFAULT_CREATED_BY,
                None,
            )
            .await
            .unwrap()
            .policy;
        created_policies.push(policy);
    }

    // Add a global policy (no action)
    let global_policy = "permit(principal, action, resource);";
    policies
        .add_policy(
            global_policy,
            None,
            None,
            None,
            None,
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap();

    // Add a policy for a different service
    let different_service_action = Action {
        service: "compute".to_string(),
        name: "delete".to_string(),
    };
    let different_service_policy = "permit(\
        principal == Principal::\"test-user\", \
        action == Action::\"compute:delete\", \
        resource == File::\"/test.data\" \
    );";
    policies
        .add_policy(
            different_service_policy,
            Some(&different_service_action),
            Some(&principal),
            None,
            Some(&resource),
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap();

    let wildcard_action = Action {
        name: "*".to_string(),
        service: service.to_string(),
    };

    let page_result = policies
        .get_policies_paginated(
            Some(&wildcard_action),
            Some(&principal),
            None,
            Some(&resource),
            &[],
            None,
            1,
            10,
        )
        .await
        .unwrap();

    // Should return only policies for the specified service (4 specific actions)
    // Global policies and policies for other services should NOT be returned
    assert_eq!(page_result.items.len(), 4);

    // Verify that all returned policies have actions from the specified service
    for policy in &page_result.items {
        if let Some(policy_action) = &policy.action {
            assert_eq!(policy_action.service, service);
            assert!(actions.contains(&policy_action.name.as_str()));
        } else {
            panic!("Wildcard query should not return global policies");
        }
    }

    // Test with a different service wildcard - should not match any policies
    let different_service_wildcard = Action {
        name: "*".to_string(),
        service: "different-service".to_string(),
    };

    let page_result = policies
        .get_policies_paginated(
            Some(&different_service_wildcard),
            Some(&principal),
            None,
            Some(&resource),
            &[],
            None,
            1,
            10,
        )
        .await
        .unwrap();
    assert_eq!(page_result.items.len(), 0);
}

#[tokio::test]
async fn test_set_system_policies() {
    let (policies, _) = connect_database().await;

    let regular_policy = "permit(principal, action, resource);";
    let regular_policy_result = policies
        .add_policy(
            regular_policy,
            None,
            None,
            None,
            None,
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap()
        .policy;

    let system_policies = vec![
        PolicyDbBatch {
            canonical_hash: test_canonical_hash(
                "permit(principal == Principal::\"admin\", action, resource);",
            ),
            policy: "permit(principal == Principal::\"admin\", action, resource);".to_string(),
            action: None,
            principal: Some(Principal {
                id: "admin".to_string(),
                info: None,
            }),
            resource: None,
            group: None,
            order: 0,
            system: false, // This should be set to true by set_system_policies
            created_by: DEFAULT_CREATED_BY.to_string(),
            comment: None,
            resource_prefix: None,
        },
        PolicyDbBatch {
            canonical_hash: test_canonical_hash(
                "permit(principal == Principal::\"system\", action, resource);",
            ),
            policy: "permit(principal == Principal::\"system\", action, resource);".to_string(),
            action: None,
            principal: Some(Principal {
                id: "system".to_string(),
                info: None,
            }),
            resource: None,
            group: None,
            order: 0,
            system: false, // This should be set to true by set_system_policies
            created_by: DEFAULT_CREATED_BY.to_string(),
            comment: None,
            resource_prefix: None,
        },
    ];

    let result = policies.set_system_policies(system_policies).await;
    assert!(result.is_ok());

    let system_policy_results = result.unwrap();
    assert_eq!(system_policy_results.len(), 2);

    for (i, policy) in system_policy_results.iter().enumerate() {
        assert_eq!(policy.id, (i + 2) as i64); // IDs should be 2 and 3 (after regular policy with ID 1)
        assert_eq!(
            policy.text,
            if i == 0 {
                "permit(principal == Principal::\"admin\", action, resource);"
            } else {
                "permit(principal == Principal::\"system\", action, resource);"
            }
        );
        assert_eq!(policy.created_by, SYSTEM_CREATED_BY.to_string());
        assert!(!policy.created_at.is_empty());
    }

    let all_policies_after = policies
        .get_policies_paginated(None, None, None, None, &[], None, 1, 50)
        .await
        .unwrap();
    assert_eq!(all_policies_after.items.len(), 3); // 1 regular + 2 system

    let new_system_policies = vec![PolicyDbBatch {
        canonical_hash: test_canonical_hash(
            "permit(principal == Principal::\"new-admin\", action, resource);",
        ),
        policy: "permit(principal == Principal::\"new-admin\", action, resource);".to_string(),
        action: None,
        principal: Some(Principal {
            id: "new-admin".to_string(),
            info: None,
        }),
        resource: None,
        group: None,
        order: 0,
        system: false,
        created_by: DEFAULT_CREATED_BY.to_string(),
        comment: None,
        resource_prefix: None,
    }];

    let replace_result = policies.set_system_policies(new_system_policies).await;
    assert!(replace_result.is_ok());

    let new_system_results = replace_result.unwrap();
    assert_eq!(new_system_results.len(), 1);
    assert_eq!(
        new_system_results[0].created_by,
        SYSTEM_CREATED_BY.to_string()
    );
    assert!(!new_system_results[0].created_at.is_empty());

    let all_policies_final = policies
        .get_policies_paginated(None, None, None, None, &[], None, 1, 50)
        .await
        .unwrap();
    assert_eq!(all_policies_final.items.len(), 2); // 1 regular + 1 new system

    let regular_policy_exists = all_policies_final
        .items
        .iter()
        .any(|p| p.id == regular_policy_result.id);
    assert!(regular_policy_exists);

    let new_system_policy_exists = all_policies_final
        .items
        .iter()
        .any(|p| p.text == "permit(principal == Principal::\"new-admin\", action, resource);");
    assert!(new_system_policy_exists);
}

#[tokio::test]
async fn test_set_system_policies_empty() {
    let (policies, _) = connect_database().await;

    let system_policies = vec![PolicyDbBatch {
        canonical_hash: test_canonical_hash(
            "permit(principal == Principal::\"admin\", action, resource);",
        ),
        policy: "permit(principal == Principal::\"admin\", action, resource);".to_string(),
        action: Some(new_action()),
        principal: Some(new_principal()),
        resource: Some(new_resource()),
        group: None,
        order: 0,
        system: true,
        created_by: DEFAULT_CREATED_BY.to_string(),
        comment: None,
        resource_prefix: None,
    }];

    policies.set_system_policies(system_policies).await.unwrap();

    let page_result = policies
        .get_policies_paginated(None, None, None, None, &[], None, 1, 10)
        .await
        .unwrap();
    assert_eq!(page_result.items.len(), 1);
    assert_eq!(
        page_result.items[0].created_by,
        SYSTEM_CREATED_BY.to_string()
    );

    let empty_result = policies.set_system_policies(vec![]).await;
    assert!(empty_result.is_ok());

    let empty_results = empty_result.unwrap();
    assert_eq!(empty_results.len(), 0);

    let all_policies_after = policies
        .get_policies_paginated(None, None, None, None, &[], None, 1, 50)
        .await
        .unwrap();
    assert_eq!(all_policies_after.items.len(), 0);
}

#[tokio::test]
async fn test_get_policies_paginated_with_empty_principal() {
    let (policies, _) = connect_database().await;

    let action = new_action();
    let resource = new_resource();

    let no_principal_policy = "permit(\
        principal, \
        action == Action::\"storage:read\", \
        resource == File::\"/test.data\" \
    );";
    policies
        .add_policy(
            no_principal_policy,
            Some(&action),
            None,
            None,
            Some(&resource),
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap();

    let specific_principal_policy = "permit(\
        principal == Principal::\"test-user\", \
        action == Action::\"storage:read\", \
        resource == File::\"/test.data\" \
    );";
    let principal = new_principal();
    policies
        .add_policy(
            specific_principal_policy,
            Some(&action),
            Some(&principal),
            None,
            Some(&resource),
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap();

    let empty_principal = Principal::default();

    let page_result = policies
        .get_policies_paginated(
            Some(&action),
            Some(&empty_principal),
            None,
            Some(&resource),
            &[],
            None,
            1,
            10,
        )
        .await
        .unwrap();

    assert_eq!(page_result.items.len(), 1);
    assert_eq!(page_result.items[0].text, no_principal_policy);
    assert_eq!(page_result.items[0].principal, None);
    assert_eq!(
        page_result.items[0].created_by,
        DEFAULT_CREATED_BY.to_string()
    );
    assert!(!page_result.items[0].created_at.is_empty());
}

#[tokio::test]
async fn test_get_policies_paginated_with_empty_action() {
    let (policies, _) = connect_database().await;

    let action = new_action();
    let principal = new_principal();
    let resource = new_resource();

    let no_action_policy = "permit(\
        principal == Principal::\"test-user\", \
        action, \
        resource == File::\"/test.data\" \
    );";
    policies
        .add_policy(
            no_action_policy,
            None,
            Some(&principal),
            None,
            Some(&resource),
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap();

    let specific_action_policy = "permit(\
        principal == Principal::\"test-user\", \
        action == Action::\"storage:read\", \
        resource == File::\"/test.data\" \
    );";
    policies
        .add_policy(
            specific_action_policy,
            Some(&action),
            Some(&principal),
            None,
            Some(&resource),
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap();

    let empty_action = Action::default();

    let page_result = policies
        .get_policies_paginated(
            Some(&empty_action),
            Some(&principal),
            None,
            Some(&resource),
            &[],
            None,
            1,
            10,
        )
        .await
        .unwrap();

    assert_eq!(page_result.items.len(), 1);
    assert_eq!(page_result.items[0].text, no_action_policy);
    assert_eq!(page_result.items[0].action, None);
    assert_eq!(
        page_result.items[0].created_by,
        DEFAULT_CREATED_BY.to_string()
    );
    assert!(!page_result.items[0].created_at.is_empty());
}

#[tokio::test]
async fn test_get_policies_paginated_with_empty_resource() {
    let (policies, _) = connect_database().await;

    let action = new_action();
    let principal = new_principal();
    let resource = new_resource();

    let no_resource_policy = "permit(\
        principal == Principal::\"test-user\", \
        action == Action::\"storage:read\", \
        resource \
    );";
    policies
        .add_policy(
            no_resource_policy,
            Some(&action),
            Some(&principal),
            None,
            None,
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap();

    let specific_resource_policy = "permit(\
        principal == Principal::\"test-user\", \
        action == Action::\"storage:read\", \
        resource == File::\"/test.data\" \
    );";
    policies
        .add_policy(
            specific_resource_policy,
            Some(&action),
            Some(&principal),
            None,
            Some(&resource),
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap();

    let empty_resource = Resource::default();

    let page_result = policies
        .get_policies_paginated(
            Some(&action),
            Some(&principal),
            None,
            Some(&empty_resource),
            &[],
            None,
            1,
            10,
        )
        .await
        .unwrap();

    assert_eq!(page_result.items.len(), 1);
    assert_eq!(page_result.items[0].text, no_resource_policy);
    assert_eq!(page_result.items[0].resource, None);
    assert_eq!(
        page_result.items[0].created_by,
        DEFAULT_CREATED_BY.to_string()
    );
    assert!(!page_result.items[0].created_at.is_empty());
}

#[tokio::test]
async fn test_get_policies_paginated_with_wildcard_resource() {
    let (policies, _) = connect_database().await;

    let action = new_action();
    let principal = new_principal();
    let resource = new_resource();

    let file_policies = [
        "permit(\
            principal == Principal::\"test-user\", \
            action == Action::\"storage:read\", \
            resource == File::\"/test1.data\" \
        );",
        "permit(\
            principal == Principal::\"test-user\", \
            action == Action::\"storage:read\", \
            resource == File::\"/test2.data\" \
        );",
        "permit(\
            principal == Principal::\"test-user\", \
            action == Action::\"storage:read\", \
            resource == File::\"/test3.data\" \
        );",
    ];

    for (i, policy_text) in file_policies.iter().enumerate() {
        let mut test_resource = resource.clone();
        test_resource.id = format!("/test{}.data", i + 1);
        policies
            .add_policy(
                policy_text,
                Some(&action),
                Some(&principal),
                None,
                Some(&test_resource),
                None,
                0,
                DEFAULT_CREATED_BY,
                None,
            )
            .await
            .unwrap();
    }

    let folder_policy = "permit(\
        principal == Principal::\"test-user\", \
        action == Action::\"storage:read\", \
        resource == Folder::\"/folder\" \
    );";
    let folder_resource = Resource {
        id: "/folder".to_string(),
        kind: "Folder".to_string(),
        data: None,
    };
    policies
        .add_policy(
            folder_policy,
            Some(&action),
            Some(&principal),
            None,
            Some(&folder_resource),
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap();

    let wildcard_resource = Resource {
        id: "*".to_string(),
        kind: "File".to_string(),
        data: None,
    };

    let page_result = policies
        .get_policies_paginated(
            Some(&action),
            Some(&principal),
            None,
            Some(&wildcard_resource),
            &[],
            None,
            1,
            10,
        )
        .await
        .unwrap();

    assert_eq!(page_result.items.len(), 3);

    for policy in &page_result.items {
        if let Some(policy_resource) = &policy.resource {
            assert_eq!(policy_resource.kind, "File");
            assert!(policy_resource.id.starts_with("/test"));
            assert!(policy_resource.id.ends_with(".data"));
        } else {
            panic!("Wildcard resource query should not return policies without resource");
        }
    }
}

#[tokio::test]
async fn test_get_policies_paginated_wildcard_returns_all_of_type() {
    let (policies, _) = connect_database().await;

    let action = new_action();
    let principal = new_principal();

    let folder_ids = ["/Projects", "/Projects/Astronaut", "/Shared/Library"];
    for id in folder_ids {
        let policy_text = format!(
            "permit(\
                principal == Principal::\"test-user\", \
                action == Action::\"storage:read\", \
                resource == folder::\"{id}\" \
            );"
        );
        let resource = Resource {
            id: id.to_string(),
            kind: "folder".to_string(),
            data: None,
        };
        policies
            .add_policy(
                &policy_text,
                Some(&action),
                Some(&principal),
                None,
                Some(&resource),
                None,
                0,
                DEFAULT_CREATED_BY,
                None,
            )
            .await
            .unwrap();
    }

    // A resource of a different type must be excluded by a `folder` wildcard.
    let file_policy = "permit(\
        principal == Principal::\"test-user\", \
        action == Action::\"storage:read\", \
        resource == file::\"/Projects/notes.txt\" \
    );";
    let file_resource = Resource {
        id: "/Projects/notes.txt".to_string(),
        kind: "file".to_string(),
        data: None,
    };
    policies
        .add_policy(
            file_policy,
            Some(&action),
            Some(&principal),
            None,
            Some(&file_resource),
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap();

    // A request like `folder::"*"` parses into a type wildcard.
    let wildcard: Resource = r#"folder::"*""#.parse().unwrap();
    assert_eq!(wildcard.kind, "folder");
    assert_eq!(wildcard.id, "*");

    let page_result = policies
        .get_policies_paginated(
            Some(&action),
            Some(&principal),
            None,
            Some(&wildcard),
            &[],
            None,
            1,
            10,
        )
        .await
        .unwrap();

    assert_eq!(page_result.items.len(), folder_ids.len());
    for policy in &page_result.items {
        let resource = policy
            .resource
            .as_ref()
            .expect("type wildcard must not match resourceless policies");
        assert_eq!(resource.kind, "folder");
    }
}

#[test]
fn test_prefix_upper_bound() {
    use crate::postgres::prefix_upper_bound;

    assert_eq!(
        prefix_upper_bound(r#"File::"/Projects/Astronaut/"#).as_deref(),
        Some(r#"File::"/Projects/Astronaut0"#)
    );
    assert_eq!(prefix_upper_bound(r#"File::""#).as_deref(), Some("File::#"));
    assert_eq!(prefix_upper_bound("a").as_deref(), Some("b"));
    assert_eq!(prefix_upper_bound("").as_deref(), None);

    // Multibyte scalars are incremented by code point, which matches the
    // byte-order boundary because UTF-8 preserves code-point order.
    assert_eq!(prefix_upper_bound("café").as_deref(), Some("cafê")); // é U+00E9 -> ê U+00EA
    assert_eq!(prefix_upper_bound("📁").as_deref(), Some("📂")); // U+1F4C1 -> U+1F4C2

    // The surrogate gap: U+D7FF + 1 is the unrepresentable U+D800, so the bound
    // must skip to the next valid scalar U+E000.
    assert_eq!(
        prefix_upper_bound("a\u{D7FF}").as_deref(),
        Some("a\u{E000}")
    );

    // A trailing maximum scalar has no greater string at that position, so the
    // bound falls back to incrementing the previous character.
    assert_eq!(
        prefix_upper_bound(&format!("a{}", char::MAX)).as_deref(),
        Some("b")
    );
    // Every trailing scalar being the maximum leaves no upper bound at all.
    assert_eq!(
        prefix_upper_bound(&format!("{0}{0}", char::MAX)).as_deref(),
        None
    );
}

#[tokio::test]
async fn test_get_policies_paginated_with_resource_prefix() {
    let (policies, _) = connect_database().await;

    let action = new_action();
    let principal = new_principal();
    let resource = new_resource();

    let matching_ids = [
        "/Projects/Astronaut/mission.usd",
        "/Projects/Astronaut/rover/cam.usd",
        "/Projects/Astronaut/",
    ];
    // `/Projects/Astronaut` is the parent node (not under the folder) and
    // `/Projects/Astronaut2024/...` only shares a textual prefix, so neither
    // must be returned by a `/Projects/Astronaut/*` query.
    let non_matching_ids = ["/Projects/Astronaut", "/Projects/Astronaut2024/x.usd"];

    for id in matching_ids.iter().chain(non_matching_ids.iter()) {
        let mut test_resource = resource.clone();
        test_resource.id = id.to_string();
        let policy_text = format!(
            "permit(\
                principal == Principal::\"test-user\", \
                action == Action::\"storage:read\", \
                resource == File::\"{id}\" \
            );"
        );
        policies
            .add_policy(
                &policy_text,
                Some(&action),
                Some(&principal),
                None,
                Some(&test_resource),
                None,
                0,
                DEFAULT_CREATED_BY,
                None,
            )
            .await
            .unwrap();
    }

    let prefix_resource = Resource {
        id: "/Projects/Astronaut/*".to_string(),
        kind: "File".to_string(),
        data: None,
    };

    let page_result = policies
        .get_policies_paginated(
            Some(&action),
            Some(&principal),
            None,
            Some(&prefix_resource),
            &[],
            None,
            1,
            10,
        )
        .await
        .unwrap();

    assert_eq!(page_result.items.len(), matching_ids.len());
    for policy in &page_result.items {
        let resource = policy
            .resource
            .as_ref()
            .expect("prefix query should not return policies without a resource");
        assert_eq!(resource.kind, "File");
        assert!(resource.id.starts_with("/Projects/Astronaut/"));
    }
}

#[tokio::test]
async fn test_get_policies_paginated_with_multiple_empty_params() {
    let (policies, _) = connect_database().await;

    let action = new_action();
    let principal = new_principal();

    let global_policy = "permit(principal, action, resource);";
    let global_policy_result = policies
        .add_policy(
            global_policy,
            None,
            None,
            None,
            None,
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap()
        .policy;

    let partial_policy1 = "permit(\
        principal == Principal::\"test-user\", \
        action, \
        resource \
    );";
    policies
        .add_policy(
            partial_policy1,
            None,
            Some(&principal),
            None,
            None,
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap();

    let partial_policy2 = "permit(\
        principal, \
        action == Action::\"storage:read\", \
        resource \
    );";
    policies
        .add_policy(
            partial_policy2,
            Some(&action),
            None,
            None,
            None,
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap();

    let empty_principal = Principal::default();
    let empty_action = Action::default();
    let empty_resource = Resource::default();

    let page_result = policies
        .get_policies_paginated(
            Some(&empty_action),
            Some(&empty_principal),
            None,
            Some(&empty_resource),
            &[],
            None,
            1,
            10,
        )
        .await
        .unwrap();

    assert_eq!(page_result.items.len(), 1);
    assert_eq!(page_result.items[0], global_policy_result);
    assert_eq!(page_result.items[0].principal, None);
    assert_eq!(page_result.items[0].action, None);
    assert_eq!(page_result.items[0].resource, None);
}

#[tokio::test]
async fn test_get_policies_sorted_by_order() {
    let (policies, _) = connect_database().await;

    let action = new_action();
    let principal = new_principal();

    let p_high = policies
        .add_policy(
            r#"permit(principal, action, resource) when { context.rank == 10 };"#,
            None,
            None,
            None,
            None,
            None,
            10,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap()
        .policy;
    assert_eq!(p_high.order, 10);

    let p_low = policies
        .add_policy(
            r#"permit(principal, action, resource) when { context.rank == 1 };"#,
            None,
            None,
            None,
            None,
            None,
            1,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap()
        .policy;
    assert_eq!(p_low.order, 1);

    let p_default = policies
        .add_policy(
            r#"permit(principal, action, resource) when { context.rank == 0 };"#,
            None,
            None,
            None,
            None,
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap()
        .policy;
    assert_eq!(p_default.order, 0);

    let result = policies
        .get_policies(Some(&action), Some(&principal), &[], None, &[])
        .await
        .unwrap();

    assert_eq!(result.len(), 3);
    assert_eq!(result[0].order, 0);
    assert_eq!(result[1].order, 1);
    assert_eq!(result[2].order, 10);
}

#[tokio::test]
async fn test_get_policies_paginated_sorted_by_order() {
    let (policies, _) = connect_database().await;

    policies
        .add_policy(
            r#"permit(principal, action, resource) when { context.rank == 5 };"#,
            None,
            None,
            None,
            None,
            None,
            5,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap();

    policies
        .add_policy(
            r#"permit(principal, action, resource) when { context.rank == 0 };"#,
            None,
            None,
            None,
            None,
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap();

    policies
        .add_policy(
            r#"permit(principal, action, resource) when { context.rank == 3 };"#,
            None,
            None,
            None,
            None,
            None,
            3,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap();

    let page_result = policies
        .get_policies_paginated(None, None, None, None, &[], None, 1, 10)
        .await
        .unwrap();

    assert_eq!(page_result.items.len(), 3);
    assert_eq!(page_result.items[0].order, 0);
    assert_eq!(page_result.items[1].order, 3);
    assert_eq!(page_result.items[2].order, 5);
}

#[tokio::test]
async fn test_get_policies_paginated_filtered_by_order() {
    let (policies, _) = connect_database().await;

    for (rank, order) in [(0, 10), (1, 20), (2, 20)] {
        policies
            .add_policy(
                &format!("permit(principal, action, resource) when {{ context.rank == {rank} }};"),
                None,
                None,
                None,
                None,
                None,
                order,
                DEFAULT_CREATED_BY,
                None,
            )
            .await
            .unwrap();
    }

    let page_result = policies
        .get_policies_paginated(None, None, None, None, &[], Some(20), 1, 10)
        .await
        .unwrap();
    assert_eq!(page_result.items.len(), 2);
    assert!(page_result.items.iter().all(|p| p.order == 20));

    let page_result = policies
        .get_policies_paginated(None, None, None, None, &[], Some(10), 1, 10)
        .await
        .unwrap();
    assert_eq!(page_result.items.len(), 1);
    assert_eq!(page_result.items[0].order, 10);

    let page_result = policies
        .get_policies_paginated(None, None, None, None, &[], Some(999), 1, 10)
        .await
        .unwrap();
    assert_eq!(page_result.items.len(), 0);
}

#[tokio::test]
async fn test_get_policies_excludes_scoped_when_filter_is_none() {
    let (policies, _) = connect_database().await;

    let action = new_action();
    let principal = new_principal();
    let resource = new_resource();

    let unscoped_policy = "permit(principal, action, resource);";
    policies
        .add_policy(
            unscoped_policy,
            None,
            None,
            None,
            None,
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap();

    let scoped_resource_policy = r#"permit(
            principal == Principal::"test-user",
            action == Action::"storage:read",
            resource == File::"/test.data"
        );"#;
    policies
        .add_policy(
            scoped_resource_policy,
            Some(&action),
            Some(&principal),
            None,
            Some(&resource),
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap();

    let principal_only_policy = r#"permit(
            principal == Principal::"test-user",
            action,
            resource
        );"#;
    policies
        .add_policy(
            principal_only_policy,
            None,
            Some(&principal),
            None,
            None,
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap();

    // Authorization with all scopes specified: should match all three policies
    let result = policies
        .get_policies(Some(&action), Some(&principal), &[], Some(&resource), &[])
        .await
        .unwrap();
    assert_eq!(result.len(), 3);

    // Authorization without resource: should NOT return the resource-scoped policy
    let result = policies
        .get_policies(Some(&action), Some(&principal), &[], None, &[])
        .await
        .unwrap();
    assert_eq!(result.len(), 2);
    assert!(result.iter().all(|p| p.resource.is_none()));

    // Authorization without principal: should NOT return principal-scoped policies
    let result = policies
        .get_policies(Some(&action), None, &[], Some(&resource), &[])
        .await
        .unwrap();
    assert_eq!(result.len(), 1);
    assert!(result[0].principal.is_none());
    assert!(result[0].action.is_none());

    // Authorization with no scopes: should only return the fully unscoped policy
    let result = policies
        .get_policies(None, None, &[], None, &[])
        .await
        .unwrap();
    assert_eq!(result.len(), 1);
    assert!(result[0].principal.is_none());
    assert!(result[0].action.is_none());
    assert!(result[0].resource.is_none());
}

#[tokio::test]
async fn test_get_policies_paginated_includes_all_when_filter_is_none() {
    let (policies, _) = connect_database().await;

    let action = new_action();
    let principal = new_principal();
    let resource = new_resource();

    policies
        .add_policy(
            "permit(principal, action, resource);",
            None,
            None,
            None,
            None,
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap();

    policies
        .add_policy(
            r#"permit(
            principal == Principal::"test-user",
            action == Action::"storage:read",
            resource == File::"/test.data"
        );"#,
            Some(&action),
            Some(&principal),
            None,
            Some(&resource),
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap();

    policies
        .add_policy(
            r#"permit(
            principal == Principal::"test-user",
            action,
            resource
        );"#,
            None,
            Some(&principal),
            None,
            None,
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap();

    // Search with no filters: returns ALL policies regardless of scopes
    let result = policies
        .get_policies_paginated(None, None, None, None, &[], None, 1, 50)
        .await
        .unwrap();
    assert_eq!(result.items.len(), 3);

    // Search with principal filter: exact match on principal
    let result = policies
        .get_policies_paginated(None, Some(&principal), None, None, &[], None, 1, 50)
        .await
        .unwrap();
    assert_eq!(result.items.len(), 2);

    // Search with resource filter: exact match on resource
    let result = policies
        .get_policies_paginated(None, None, None, Some(&resource), &[], None, 1, 50)
        .await
        .unwrap();
    assert_eq!(result.items.len(), 1);

    // Search with all filters: exact match on all dimensions
    let result = policies
        .get_policies_paginated(
            Some(&action),
            Some(&principal),
            None,
            Some(&resource),
            &[],
            None,
            1,
            50,
        )
        .await
        .unwrap();
    assert_eq!(result.items.len(), 1);
}

async fn create_config_file_storage(yaml: &str) -> ConfigFilePolicyStorage {
    let dir = std::env::temp_dir().join(format!("permission-db-test-{}", std::process::id()));
    tokio::fs::create_dir_all(&dir).await.unwrap();

    let path = dir.join(format!(
        "policies-{}.yaml",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    tokio::fs::write(&path, yaml).await.unwrap();

    let storage = ConfigFilePolicyStorage::new(path);
    storage.initialize().await.unwrap();
    storage
}

#[tokio::test]
async fn test_config_file_get_policies_excludes_scoped_when_filter_is_none() {
    let yaml = r#"
policies:
  - policy: 'permit(principal, action, resource);'
  - policy: 'permit(principal == Principal::"test-user", action == Action::"storage:read", resource == File::"/test.data");'
  - policy: 'permit(principal == Principal::"test-user", action, resource);'
"#;
    let policies = create_config_file_storage(yaml).await;

    let action = new_action();
    let principal = new_principal();
    let resource = new_resource();

    // All scopes specified: all three policies match
    let result = policies
        .get_policies(Some(&action), Some(&principal), &[], Some(&resource), &[])
        .await
        .unwrap();
    assert_eq!(result.len(), 3);

    // Without resource: should NOT return the resource-scoped policy
    let result = policies
        .get_policies(Some(&action), Some(&principal), &[], None, &[])
        .await
        .unwrap();
    assert_eq!(result.len(), 2);
    assert!(result.iter().all(|p| p.resource.is_none()));

    // Without principal: should NOT return principal-scoped policies
    let result = policies
        .get_policies(Some(&action), None, &[], Some(&resource), &[])
        .await
        .unwrap();
    assert_eq!(result.len(), 1);
    assert!(result[0].principal.is_none());
    assert!(result[0].action.is_none());

    // No scopes: only the fully unscoped policy
    let result = policies
        .get_policies(None, None, &[], None, &[])
        .await
        .unwrap();
    assert_eq!(result.len(), 1);
    assert!(result[0].principal.is_none());
    assert!(result[0].action.is_none());
    assert!(result[0].resource.is_none());
}

#[tokio::test]
async fn test_config_file_get_policies_paginated_includes_all_when_filter_is_none() {
    let yaml = r#"
policies:
  - policy: 'permit(principal, action, resource);'
  - policy: 'permit(principal == Principal::"test-user", action == Action::"storage:read", resource == File::"/test.data");'
  - policy: 'permit(principal == Principal::"test-user", action, resource);'
"#;
    let policies = create_config_file_storage(yaml).await;

    let principal = new_principal();
    let resource = new_resource();

    // No filters: returns ALL policies
    let result = policies
        .get_policies_paginated(None, None, None, None, &[], None, 1, 50)
        .await
        .unwrap();
    assert_eq!(result.items.len(), 3);

    // Principal filter: exact match on principal
    let result = policies
        .get_policies_paginated(None, Some(&principal), None, None, &[], None, 1, 50)
        .await
        .unwrap();
    assert_eq!(result.items.len(), 2);

    // Resource filter: exact match on resource
    let result = policies
        .get_policies_paginated(None, None, None, Some(&resource), &[], None, 1, 50)
        .await
        .unwrap();
    assert_eq!(result.items.len(), 1);
}

#[tokio::test]
async fn test_config_file_get_policies_paginated_filtered_by_order() {
    let yaml = r#"
policies:
  - policy: 'permit(principal, action, resource);'
    order: 10
  - policy: 'permit(principal == Principal::"test-user", action == Action::"storage:read", resource == File::"/test.data");'
    order: 20
  - policy: 'permit(principal == Principal::"test-user", action, resource);'
    order: 20
"#;
    let policies = create_config_file_storage(yaml).await;

    let result = policies
        .get_policies_paginated(None, None, None, None, &[], Some(20), 1, 50)
        .await
        .unwrap();
    assert_eq!(result.items.len(), 2);
    assert!(result.items.iter().all(|p| p.order == 20));

    let result = policies
        .get_policies_paginated(None, None, None, None, &[], Some(10), 1, 50)
        .await
        .unwrap();
    assert_eq!(result.items.len(), 1);
    assert_eq!(result.items[0].order, 10);

    let result = policies
        .get_policies_paginated(None, None, None, None, &[], Some(999), 1, 50)
        .await
        .unwrap();
    assert_eq!(result.items.len(), 0);
}

#[tokio::test]
async fn test_config_file_get_policies_membership_filtering() {
    let yaml = r#"
policies:
  - policy: 'permit(principal, action, resource);'
  - policy: 'permit(principal in Group::"editors", action, resource);'
"#;
    let policies = create_config_file_storage(yaml).await;

    // No memberships: the group-scoped policy is excluded.
    let result = policies
        .get_policies(None, None, &[], None, &[])
        .await
        .unwrap();
    assert_eq!(result.len(), 1);
    assert!(result[0].group.is_none());

    // Member of "editors": both the unscoped and the group-scoped policy.
    let member = vec!["editors".to_string()];
    let result = policies
        .get_policies(None, None, &member, None, &[])
        .await
        .unwrap();
    assert_eq!(result.len(), 2);
    assert!(result
        .iter()
        .any(|p| p.group.as_ref().map(|g| g.id.as_str()) == Some("editors")));

    // Member of an unrelated group: only the unscoped policy.
    let non_member = vec!["viewers".to_string()];
    let result = policies
        .get_policies(None, None, &non_member, None, &[])
        .await
        .unwrap();
    assert_eq!(result.len(), 1);
    assert!(result[0].group.is_none());
}

#[tokio::test]
async fn test_config_file_get_policies_paginated_filtered_by_group() {
    let yaml = r#"
policies:
  - policy: 'permit(principal, action, resource);'
  - policy: 'permit(principal in Group::"editors", action, resource);'
  - policy: 'permit(principal in Group::"viewers", action, resource);'
"#;
    let policies = create_config_file_storage(yaml).await;

    // Exact match: only the editors-scoped policy.
    let editors = Group {
        id: "editors".to_string(),
    };
    let result = policies
        .get_policies_paginated(None, None, Some(&editors), None, &[], None, 1, 50)
        .await
        .unwrap();
    assert_eq!(result.items.len(), 1);
    assert_eq!(result.items[0].group.as_ref().unwrap().id, "editors");

    // Empty-id sentinel (`group=NULL`): only policies without a group scope.
    let unscoped = Group { id: String::new() };
    let result = policies
        .get_policies_paginated(None, None, Some(&unscoped), None, &[], None, 1, 50)
        .await
        .unwrap();
    assert_eq!(result.items.len(), 1);
    assert!(result.items[0].group.is_none());

    // No group filter: every policy is returned.
    let result = policies
        .get_policies_paginated(None, None, None, None, &[], None, 1, 50)
        .await
        .unwrap();
    assert_eq!(result.items.len(), 3);
}

#[tokio::test]
async fn test_update_policy_updates_existing_row() {
    let (policies, _) = connect_database().await;

    let original = "permit(principal, action, resource);";
    let inserted = policies
        .add_policy(
            original,
            None,
            None,
            None,
            None,
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap()
        .policy;

    let new_text = r#"permit(principal, action, resource) when { context.updated == true };"#;
    let updated = policies
        .update_policy(
            inserted.id,
            new_text,
            None,
            None,
            None,
            None,
            None,
            7,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap();

    assert_eq!(updated.id, inserted.id);
    assert_eq!(updated.text, new_text);
    assert_eq!(updated.order, 7);

    let page = policies
        .get_policies_paginated(None, None, None, None, &[], None, 1, 50)
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].id, inserted.id);
    assert_eq!(page.items[0].text, new_text);
}

#[tokio::test]
async fn test_update_policy_inserts_when_id_does_not_exist() {
    let (policies, _) = connect_database().await;

    let text = "permit(principal, action, resource);";
    let result = policies
        .update_policy(
            42,
            text,
            None,
            None,
            None,
            None,
            None,
            3,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap();

    assert_eq!(result.id, 42);
    assert_eq!(result.text, text);
    assert_eq!(result.order, 3);

    let page = policies
        .get_policies_paginated(None, None, None, None, &[], None, 1, 50)
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].id, 42);
}

#[tokio::test]
async fn test_update_policy_returns_existing_when_hash_matches_other_row() {
    let (policies, _) = connect_database().await;

    let shared = "permit(principal, action, resource);";
    let first = policies
        .add_policy(
            shared,
            None,
            None,
            None,
            None,
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap()
        .policy;

    let other = r#"permit(principal, action, resource) when { context.other == true };"#;
    let second = policies
        .add_policy(
            other,
            None,
            None,
            None,
            None,
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap()
        .policy;
    assert_ne!(first.id, second.id);

    // Attempt to retarget `second.id` at the text already stored under `first.id`.
    let result = policies
        .update_policy(
            second.id,
            shared,
            None,
            None,
            None,
            None,
            None,
            99,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap();

    // The row at `first.id` is returned unchanged; no write happened at `second.id`.
    assert_eq!(result.id, first.id);
    assert_eq!(result.text, first.text);
    assert_eq!(result.order, first.order);

    let second_current = policies.get_policy_by_id(&second.id).await.unwrap();
    assert_eq!(second_current.text, other);
    assert_eq!(second_current.order, 0);
}

#[tokio::test]
async fn test_update_policy_whitespace_variant_matches_existing_row() {
    let (policies, _) = connect_database().await;

    let canonical = "permit(principal, action, resource);";
    let first = policies
        .add_policy(
            canonical,
            None,
            None,
            None,
            None,
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap()
        .policy;

    let placeholder = r#"permit(principal, action, resource) when { context.v == 1 };"#;
    let second = policies
        .add_policy(
            placeholder,
            None,
            None,
            None,
            None,
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap()
        .policy;

    // Whitespace-only variant of `canonical` - should canonicalize to the same hash as `first`.
    let whitespace_variant =
        "permit(\n    principal,\n\taction,\n    resource\n);\n// trailing comment\n";
    let result = policies
        .update_policy(
            second.id,
            whitespace_variant,
            None,
            None,
            None,
            None,
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap();

    assert_eq!(result.id, first.id);
    assert_eq!(result.text, first.text);
}

#[tokio::test]
async fn test_update_policy_same_id_same_text_updates_fields() {
    let (policies, _) = connect_database().await;

    let text = "permit(principal, action, resource);";
    let inserted = policies
        .add_policy(
            text,
            None,
            None,
            None,
            None,
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap()
        .policy;

    // Same id, same canonical text, different order - fields should still update.
    let updated = policies
        .update_policy(
            inserted.id,
            text,
            None,
            None,
            None,
            None,
            None,
            5,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap();

    assert_eq!(updated.id, inserted.id);
    assert_eq!(updated.text, text);
    assert_eq!(updated.order, 5);
}

#[tokio::test]
async fn test_update_policy_invalid_cedar_returns_constraint_error() {
    let (policies, _) = connect_database().await;

    let err = policies
        .update_policy(
            1,
            "not a cedar policy",
            None,
            None,
            None,
            None,
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .expect_err("invalid Cedar text must be rejected");
    assert_eq!(err.kind, StorageErrKind::Constraint);
}

#[tokio::test]
async fn test_add_policy_with_comment() {
    let (policies, _) = connect_database().await;

    let policy = "permit(principal, action, resource);";
    let comment = "owned by platform team; reviewed 2026-Q1";

    let inserted = policies
        .add_policy(
            policy,
            None,
            None,
            None,
            None,
            None,
            0,
            DEFAULT_CREATED_BY,
            Some(comment),
        )
        .await
        .unwrap()
        .policy;
    assert_eq!(inserted.comment.as_deref(), Some(comment));

    let fetched = policies.get_policy_by_id(&inserted.id).await.unwrap();
    assert_eq!(fetched.comment.as_deref(), Some(comment));

    let paginated = policies
        .get_policies_paginated(None, None, None, None, &[], None, 1, 10)
        .await
        .unwrap();
    let entry = paginated
        .items
        .iter()
        .find(|p| p.id == inserted.id)
        .expect("inserted policy must appear in paginated results");
    assert_eq!(entry.comment.as_deref(), Some(comment));
}

/// Re-PUTing the same Cedar text via `add_policy` must refresh the mutable
/// fields (`order`, `comment`) and the audit columns (`created_by`,
/// `created_at`) on the existing row rather than silently returning stale
/// values from the dedup path.
#[tokio::test]
async fn test_add_policy_conflict_updates_order_comment_and_audit() {
    let (policies, _) = connect_database().await;

    let policy = "permit(principal, action, resource);";

    let first = policies
        .add_policy(
            policy,
            None,
            None,
            None,
            None,
            None,
            1,
            DEFAULT_CREATED_BY,
            Some("initial comment"),
        )
        .await
        .unwrap();
    assert!(first.created, "first insert must report created=true");
    let first = first.policy;
    assert_eq!(first.order, 1);
    assert_eq!(first.comment.as_deref(), Some("initial comment"));
    assert_eq!(first.created_by, DEFAULT_CREATED_BY);

    let second_created_by = "other-creator";
    let second = policies
        .add_policy(
            policy,
            None,
            None,
            None,
            None,
            None,
            42,
            second_created_by,
            Some("updated comment"),
        )
        .await
        .unwrap();
    assert!(
        !second.created,
        "conflict-hit upsert must report created=false"
    );
    let second = second.policy;

    assert_eq!(second.id, first.id);
    assert_eq!(second.order, 42);
    assert_eq!(second.comment.as_deref(), Some("updated comment"));
    assert_eq!(second.created_by, second_created_by);
    assert!(!second.created_at.is_empty());
    assert!(
        second.created_at >= first.created_at,
        "created_at should be refreshed to >= original ({} vs {})",
        second.created_at,
        first.created_at,
    );

    let fetched = policies.get_policy_by_id(&first.id).await.unwrap();
    assert_eq!(fetched.order, 42);
    assert_eq!(fetched.comment.as_deref(), Some("updated comment"));
    assert_eq!(fetched.created_by, second_created_by);

    let third = policies
        .add_policy(
            policy,
            None,
            None,
            None,
            None,
            None,
            7,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap();
    assert!(
        !third.created,
        "repeat upsert after first update must remain created=false"
    );
    let third = third.policy;
    assert_eq!(third.id, first.id);
    assert_eq!(third.order, 7);
    assert_eq!(third.comment, None);
    assert_eq!(third.created_by, DEFAULT_CREATED_BY);
}

/// Same contract as `test_add_policy_conflict_updates_order_comment_and_audit`
/// but exercises the batched upsert path used by the batch PUT API.
#[tokio::test]
async fn test_add_policy_batch_conflict_updates_order_comment_and_audit() {
    let (policies, _) = connect_database().await;

    let policy = "permit(principal, action, resource);";
    let canonical_hash = test_canonical_hash(policy);

    let first_batch = vec![PolicyDbBatch {
        policy: policy.to_string(),
        action: None,
        principal: None,
        resource: None,
        group: None,
        order: 1,
        system: false,
        created_by: DEFAULT_CREATED_BY.to_string(),
        comment: Some("initial comment".to_string()),
        canonical_hash: canonical_hash.clone(),
        resource_prefix: None,
    }];
    let first = policies.add_policy_batch(first_batch).await.unwrap();
    assert_eq!(first.len(), 1);
    let first = first.into_iter().next().unwrap();
    assert!(first.created, "first batch insert must report created=true");
    let first = first.policy;
    assert_eq!(first.order, 1);
    assert_eq!(first.comment.as_deref(), Some("initial comment"));
    assert_eq!(first.created_by, DEFAULT_CREATED_BY);

    let second_created_by = "other-creator";
    let second_batch = vec![PolicyDbBatch {
        policy: policy.to_string(),
        action: None,
        principal: None,
        resource: None,
        group: None,
        order: 99,
        system: false,
        created_by: second_created_by.to_string(),
        comment: Some("updated comment".to_string()),
        canonical_hash,
        resource_prefix: None,
    }];
    let second = policies.add_policy_batch(second_batch).await.unwrap();
    assert_eq!(second.len(), 1);
    let second = second.into_iter().next().unwrap();
    assert!(
        !second.created,
        "conflict-hit batch upsert must report created=false"
    );
    let second = second.policy;

    assert_eq!(second.id, first.id);
    assert_eq!(second.order, 99);
    assert_eq!(second.comment.as_deref(), Some("updated comment"));
    assert_eq!(second.created_by, second_created_by);
    assert!(
        second.created_at >= first.created_at,
        "created_at should be refreshed to >= original ({} vs {})",
        second.created_at,
        first.created_at,
    );

    let fetched = policies.get_policy_by_id(&first.id).await.unwrap();
    assert_eq!(fetched.order, 99);
    assert_eq!(fetched.comment.as_deref(), Some("updated comment"));
    assert_eq!(fetched.created_by, second_created_by);

    let paginated = policies
        .get_policies_paginated(None, None, None, None, &[], None, 1, 10)
        .await
        .unwrap();
    assert_eq!(paginated.items.len(), 1);
    assert_eq!(paginated.items[0].id, first.id);
}

/// `add_policy_batch` must report `created=true` only for the rows that were
/// newly inserted by the current call. A mixed batch where some hashes already
/// exist in the database must return per-item flags reflecting that.
#[tokio::test]
async fn test_add_policy_batch_mixed_created_flags() {
    let (policies, _) = connect_database().await;

    let existing = "permit(principal, action, resource);";
    policies
        .add_policy(
            existing,
            None,
            None,
            None,
            None,
            None,
            0,
            DEFAULT_CREATED_BY,
            None,
        )
        .await
        .unwrap();

    let fresh =
        r#"permit(principal, action, resource) when { context.fresh == true };"#.to_string();
    let mixed = vec![
        PolicyDbBatch {
            policy: existing.to_string(),
            action: None,
            principal: None,
            resource: None,
            group: None,
            order: 0,
            system: false,
            created_by: DEFAULT_CREATED_BY.to_string(),
            comment: None,
            canonical_hash: test_canonical_hash(existing),
            resource_prefix: None,
        },
        PolicyDbBatch {
            policy: fresh.clone(),
            action: None,
            principal: None,
            resource: None,
            group: None,
            order: 0,
            system: false,
            created_by: DEFAULT_CREATED_BY.to_string(),
            comment: None,
            canonical_hash: test_canonical_hash(&fresh),
            resource_prefix: None,
        },
    ];

    let results = policies.add_policy_batch(mixed).await.unwrap();
    assert_eq!(results.len(), 2);
    assert!(
        !results[0].created,
        "row that already existed must report created=false"
    );
    assert!(
        results[1].created,
        "row inserted by this call must report created=true"
    );
}

/// Within a single `add_policy_batch` call, duplicate items collapse to a
/// single inserted row; every duplicate slot in the response must share the
/// same `created=true` flag since the underlying row was created by this call.
#[tokio::test]
async fn test_add_policy_batch_intra_batch_duplicates_share_created_flag() {
    let (policies, _) = connect_database().await;

    let policy = "permit(principal, action, resource);";
    let hash = test_canonical_hash(policy);
    let batch = vec![
        PolicyDbBatch {
            policy: policy.to_string(),
            action: None,
            principal: None,
            resource: None,
            group: None,
            order: 0,
            system: false,
            created_by: DEFAULT_CREATED_BY.to_string(),
            comment: None,
            canonical_hash: hash.clone(),
            resource_prefix: None,
        },
        PolicyDbBatch {
            policy: policy.to_string(),
            action: None,
            principal: None,
            resource: None,
            group: None,
            order: 0,
            system: false,
            created_by: DEFAULT_CREATED_BY.to_string(),
            comment: None,
            canonical_hash: hash,
            resource_prefix: None,
        },
    ];

    let results = policies.add_policy_batch(batch).await.unwrap();
    assert_eq!(results.len(), 2);
    assert!(results[0].created);
    assert!(results[1].created);
    assert_eq!(results[0].policy.id, results[1].policy.id);
}
