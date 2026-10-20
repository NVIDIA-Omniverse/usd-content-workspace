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

use crate::pool::ConnectionPool;
use crate::MetadataConfig;
use crate::{
    EvaluationPriority, Meta, PageResult, PolicyQueryBatch, ResourceTypeMeta, ServiceMeta,
};
use crate::{
    MetadataStorage, Policy, PolicyDbBatch, PolicyStorage, PolicyUpsertResult, StorageErr,
    StorageErrKind,
};
use async_trait::async_trait;
use clap::Args;
use deadpool_postgres::{GenericClient, SslMode};
use permission_core::{canonicalize_policy, Action, Group, PolicyId, Principal, Resource};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::runtime::Runtime;
use tokio_postgres::error::SqlState;
use tokio_postgres::types::ToSql;
use tokio_postgres::Row;

const SYSTEM_CREATED_BY: &str = "SYSTEM";

/// Maps a Postgres unique-violation on `metadata_resource_types` to a
/// `Conflict` error, so a resource type kind already owned by another service
/// surfaces as a client conflict rather than an internal error. Other errors
/// are converted verbatim.
fn map_resource_type_conflict(err: tokio_postgres::Error) -> StorageErr {
    if let Some(db_err) = err.as_db_error() {
        if db_err.code() == &SqlState::UNIQUE_VIOLATION {
            return StorageErr {
                msg: format!(
                    "resource type kind is already registered under another service: {}",
                    db_err.detail().unwrap_or_else(|| db_err.message())
                ),
                kind: StorageErrKind::Conflict,
            };
        }
    }
    StorageErr::from(err)
}

#[derive(Args, Clone, Debug)]
pub struct PostgresArgs {
    #[arg(
        id = "postgres-host",
        long = "postgres-host",
        env = "POSTGRES_HOST",
        required_if_eq("database-type", "postgres")
    )]
    pub host: Option<String>,

    #[arg(
        id = "postgres-port",
        long = "postgres-port",
        env = "POSTGRES_PORT",
        default_value_t = 5432
    )]
    pub port: u16,

    #[arg(
        id = "postgres-dbname",
        long = "postgres-dbname",
        env = "POSTGRES_DBNAME",
        required_if_eq("database-type", "postgres")
    )]
    pub dbname: Option<String>,

    #[arg(
        id = "postgres-user",
        long = "postgres-user",
        env = "POSTGRES_USER",
        required_if_eq("database-type", "postgres")
    )]
    pub user: Option<String>,

    #[arg(
        id = "postgres-password",
        long = "postgres-password",
        env = "POSTGRES_PASSWORD",
        group = "password_group"
    )]
    pub password: Option<String>,

    #[arg(
        id = "postgres-password-file",
        long = "postgres-password-file",
        env = "POSTGRES_PASSWORD_FILE",
        group = "password_group"
    )]
    pub password_file: Option<PathBuf>,

    #[arg(
        id = "postgres-token-file",
        long = "postgres-token-file",
        env = "POSTGRES_TOKEN_FILE",
        group = "password_group"
    )]
    pub token_file: Option<PathBuf>,

    #[arg(
        id = "postgres-token-file-poll-interval",
        long = "postgres-token-file-poll-interval",
        env = "POSTGRES_TOKEN_FILE_POLL_INTERVAL",
        default_value = "15s",
        value_parser = humantime::parse_duration,
    )]
    pub token_file_poll_interval: Duration,

    /// How long to wait at startup for a sidecar-mounted credential file
    /// (token or password file) to become readable before failing closed.
    #[arg(
        id = "postgres-credential-wait-timeout",
        long = "postgres-credential-wait-timeout",
        env = "POSTGRES_CREDENTIAL_WAIT_TIMEOUT",
        default_value = "30s",
        value_parser = humantime::parse_duration,
    )]
    pub credential_wait_timeout: Duration,

    /// Interval between attempts to read a not-yet-ready credential file at
    /// startup.
    #[arg(
        id = "postgres-credential-wait-interval",
        long = "postgres-credential-wait-interval",
        env = "POSTGRES_CREDENTIAL_WAIT_INTERVAL",
        default_value = "2s",
        value_parser = humantime::parse_duration,
    )]
    pub credential_wait_interval: Duration,

    #[arg(
        id = "postgres-ssl-mode",
        long = "postgres-ssl-mode",
        env = "POSTGRES_SSL_MODE",
        value_parser = parse_ssl_mode,
    )]
    pub ssl_mode: Option<SslMode>,

    #[arg(
        id = "postgres-connections",
        long = "postgres-connections",
        env = "POSTGRES_CONNECTIONS",
        default_value_t = 16
    )]
    pub connections: usize,

    /// Amount of worker threads specified for a dedicated Postgres runtime.
    #[arg(
        id = "postgres-worker-threads",
        long = "postgres-worker-threads",
        env = "POSTGRES_WORKER_THREADS",
        default_value_t = 32
    )]
    pub worker_threads: usize,
}

impl PostgresArgs {
    pub fn get_password(&self) -> Result<String, Box<dyn std::error::Error>> {
        if let Some(password) = &self.password {
            Ok(password.clone())
        } else if let Some(path) = &self.password_file {
            let password = fs::read_to_string(path)?.trim().to_string();
            Ok(password)
        } else {
            Err("no static Postgres password configured".into())
        }
    }
}

fn parse_ssl_mode(s: &str) -> Result<SslMode, String> {
    match s {
        "" => Ok(SslMode::Disable),
        "disable" => Ok(SslMode::Disable),
        "require" => Ok(SslMode::Require),
        "prefer" => Ok(SslMode::Prefer),
        s => Err(format!("Invalid Postgres SSL mode: {}", s)),
    }
}

#[derive(Clone, Debug)]
pub struct PostgresDatabaseOptions {
    pub temporary: bool,
}

#[derive(Clone)]
pub struct PostgresPolicyStorage {
    pool: Arc<dyn ConnectionPool>,
    runtime: Arc<Runtime>,
    options: PostgresDatabaseOptions,
    db_name: String,
}

impl PostgresPolicyStorage {
    pub fn new(
        pool: Arc<dyn ConnectionPool>,
        runtime: Arc<Runtime>,
        db_name: &str,
        options: PostgresDatabaseOptions,
    ) -> Self {
        Self {
            pool,
            runtime,
            db_name: db_name.to_string(),
            options,
        }
    }

    async fn create_table(&self) -> Result<(), StorageErr> {
        let table_stmt = if self.options.temporary {
            "TEMP TABLE"
        } else {
            "TABLE"
        };

        let client = self.pool.get().await?;
        client
            .execute(
                &format!(
                    r#"CREATE {table_stmt} IF NOT EXISTS policies (
                        id BIGSERIAL NOT NULL PRIMARY KEY,
                        principal VARCHAR(255) NULL,
                        action VARCHAR(255) NULL,
                        resource VARCHAR(4096) NULL,
                        policy TEXT,
                        created_at TIMESTAMP NOT NULL DEFAULT timezone('UTC', now()),
                        created_by VARCHAR(255) NOT NULL DEFAULT ''
                    )"#
                ),
                &[],
            )
            .await?;

        client
            .execute(
                r#"ALTER TABLE policies ADD COLUMN IF NOT EXISTS system BOOLEAN NOT NULL DEFAULT 'FALSE'"#,
                &[],
            )
            .await?;
        client
            .execute(
                r#"ALTER TABLE policies ADD COLUMN IF NOT EXISTS created_at TIMESTAMP NOT NULL DEFAULT timezone('UTC', now())"#,
                &[],
            )
            .await?;
        client
            .execute(
                r#"ALTER TABLE policies ADD COLUMN IF NOT EXISTS created_by VARCHAR(255) NOT NULL DEFAULT ''"#,
                &[],
            )
            .await?;
        client
            .execute(
                r#"ALTER TABLE policies ADD COLUMN IF NOT EXISTS "order" INTEGER NOT NULL DEFAULT 0"#,
                &[],
            )
            .await?;
        client
            .execute(
                r#"ALTER TABLE policies ADD COLUMN IF NOT EXISTS policy_hash CHAR(32)"#,
                &[],
            )
            .await?;
        client
            .execute(
                r#"ALTER TABLE policies ADD COLUMN IF NOT EXISTS comment TEXT"#,
                &[],
            )
            .await?;
        client
            .execute(
                r#"ALTER TABLE policies ADD COLUMN IF NOT EXISTS principal_group VARCHAR(255) NULL"#,
                &[],
            )
            .await?;
        client
            .execute(
                r#"ALTER TABLE policies ADD COLUMN IF NOT EXISTS resource_prefix VARCHAR(4096) NULL"#,
                &[],
            )
            .await?;
        Ok(())
    }

    async fn create_indexes(&self) -> Result<(), StorageErr> {
        let client = self.pool.get().await?;

        client
            .execute(
                r#"CREATE INDEX IF NOT EXISTS policies_action_idx ON policies (action)"#,
                &[],
            )
            .await?;

        // `varchar_pattern_ops` orders keys by code point rather than the
        // database collation, which is what both exact (`=`) lookups and the
        // prefix range scan (`~>=~`/`~<~`) used by wildcard resource queries
        // rely on. It supersedes the default-collation index, so drop the old
        // one if a previous deployment created it.
        client
            .execute(r#"DROP INDEX IF EXISTS policies_resource_idx"#, &[])
            .await?;

        client
            .execute(
                r#"CREATE INDEX IF NOT EXISTS policies_resource_pattern_idx ON policies (resource varchar_pattern_ops)"#,
                &[],
            )
            .await?;

        client
            .execute(
                r#"CREATE INDEX IF NOT EXISTS policies_system_true_idx ON policies (id) WHERE system = TRUE"#,
                &[],
            )
            .await?;

        client
            .execute(
                r#"CREATE UNIQUE INDEX IF NOT EXISTS policies_policy_hash_unique_idx ON policies (policy_hash)"#,
                &[],
            )
            .await?;

        client
            .execute(
                r#"CREATE INDEX IF NOT EXISTS policies_order_id_idx ON policies ("order", id)"#,
                &[],
            )
            .await?;

        client
            .execute(
                r#"CREATE INDEX IF NOT EXISTS policies_principal_group_idx ON policies (principal_group)"#,
                &[],
            )
            .await?;

        client
            .execute(
                r#"CREATE INDEX IF NOT EXISTS policies_resource_prefix_idx ON policies (resource_prefix varchar_pattern_ops)"#,
                &[],
            )
            .await?;

        Ok(())
    }

    async fn drop_table(&self) -> Result<(), StorageErr> {
        let client = self.pool.get().await?;
        client
            .execute(r#"DROP TABLE policies IF EXISTS"#, &[])
            .await?;
        Ok(())
    }

    fn read_row(row: &Row) -> Policy {
        let id: i64 = row.get(0);
        let text: &str = row.get(1);
        let sub: Option<String> = row.get(2);
        let principal = sub.map(|id| Principal { id, info: None });
        let group_id: Option<String> = row.get(3);
        let group = group_id.map(|id| Group { id });

        let action_str: Option<String> = row.get(4);
        let action = action_str.map(|a| a.parse().unwrap());

        let resource_str: Option<String> = row.get(5);
        let resource = resource_str.map(|r| r.parse().unwrap());
        let created_at: String = row.get(6);
        let created_by: String = row.get(7);
        let order: i32 = row.get(8);
        let comment: Option<String> = row.get(9);
        let resource_prefix_str: Option<String> = row.get(10);
        let resource_prefix = resource_prefix_str.map(|r| r.parse().unwrap());
        let policy_hash: String = row.get(11);

        Policy {
            id,
            order,
            text: text.to_string(),
            policy_hash,
            resource,
            resource_prefix,
            action,
            principal,
            group,
            created_at,
            created_by,
            comment,
        }
    }

    /// Reads a row produced by an `INSERT ... ON CONFLICT DO UPDATE` query
    /// whose `RETURNING` clause appends `(xmax = 0) AS created` after the
    /// columns parsed by [`Self::read_row`]. `xmax = 0` is `true` only for
    /// rows freshly inserted by the current statement; conflict-hit rows
    /// carry the updating transaction's xmax and therefore return `false`.
    fn read_upsert_row(row: &Row) -> PolicyUpsertResult {
        let policy = Self::read_row(row);
        let created: bool = row.get(12);
        PolicyUpsertResult { policy, created }
    }

    fn read_row_for_evaluation(row: &Row) -> Policy {
        let id: i64 = row.get(0);
        let text: &str = row.get(1);
        let sub: Option<String> = row.get(2);
        let principal = sub.map(|id| Principal { id, info: None });
        let group_id: Option<String> = row.get(3);
        let group = group_id.map(|id| Group { id });

        let action_str: Option<String> = row.get(4);
        let action = action_str.map(|a| a.parse().unwrap());

        let resource_str: Option<String> = row.get(5);
        let resource = resource_str.map(|r| r.parse().unwrap());
        let order: i32 = row.get(6);
        let resource_prefix_str: Option<String> = row.get(7);
        let resource_prefix = resource_prefix_str.map(|r| r.parse().unwrap());
        let policy_hash: String = row.get(8);

        Policy {
            id,
            order,
            text: text.to_string(),
            policy_hash,
            resource,
            resource_prefix,
            action,
            principal,
            group,
            created_at: String::new(),
            created_by: String::new(),
            comment: None,
        }
    }
}

#[async_trait]
impl PolicyStorage for PostgresPolicyStorage {
    async fn initialize(&self) -> Result<(), StorageErr> {
        self.create_table().await?;
        self.create_indexes().await?;
        Ok(())
    }

    async fn destroy(&self) -> Result<(), StorageErr> {
        self.drop_table().await?;
        Ok(())
    }

    #[tracing::instrument(level = "info", skip(self), fields(otel.kind = "client", db.name = self.db_name))]
    async fn set_system_policies(
        &self,
        mut policies: Vec<PolicyDbBatch>,
    ) -> Result<Vec<Policy>, StorageErr> {
        let pool = self.pool.clone();

        self.runtime
            .spawn(async move {
                let client = pool.get().await?;
                let stmt = client
                    .prepare_cached("DELETE FROM policies WHERE system = TRUE;")
                    .await?;

                client.execute(&stmt, &[]).await?;
                Ok::<(), StorageErr>(())
            })
            .await??;

        for policy in policies.iter_mut() {
            policy.system = true;
            policy.created_by = SYSTEM_CREATED_BY.to_string();
        }
        let results = self.add_policy_batch(policies).await?;
        Ok(results.into_iter().map(|r| r.policy).collect())
    }

    #[tracing::instrument(level = "info", skip(self), fields(otel.kind = "client", db.name = self.db_name))]
    async fn add_policy(
        &self,
        policy: &str,
        action: Option<&Action>,
        principal: Option<&Principal>,
        group: Option<&Group>,
        resource: Option<&Resource>,
        resource_prefix: Option<&Resource>,
        order: i32,
        created_by: &str,
        comment: Option<&str>,
    ) -> Result<PolicyUpsertResult, StorageErr> {
        let policy = policy.trim().to_string();
        let canonical = canonicalize_policy(&policy).map_err(|err| StorageErr {
            msg: err.msg,
            kind: StorageErrKind::Constraint,
        })?;
        let action = action.cloned();
        let principal = principal.cloned();
        let group = group.cloned();
        let resource = resource.cloned();
        let resource_prefix = resource_prefix.cloned();
        let created_by = created_by.to_string();
        let comment = comment.map(|c| c.to_string());
        let pool = self.pool.clone();

        let result = self
            .runtime
            .spawn(async move {
                let action_str = action.as_ref().map(|a| a.entity_id());
                let sub = principal.as_ref().map(|p| p.id.as_str());
                let group_id = group.as_ref().map(|g| g.id.as_str());
                let resource_str = resource.as_ref().map(|r| r.to_string());
                let resource_prefix_str = resource_prefix.as_ref().map(|r| r.to_string());

                let client = pool.get().await?;
                // On `policy_hash` conflict, refresh the mutable fields
                // (`order`, `comment`) and the audit columns (`created_at`,
                // `created_by`) so a re-PUT of the same Cedar text behaves
                // like a full overwrite of the existing row. The
                // `WHERE system = FALSE` guard skips the update when the
                // conflicting row is a system policy, so the statement returns
                // no row and the upsert leaves the system policy untouched.
                let stmt = client
                    .prepare_cached(
                        r#"
                            INSERT INTO policies (action, principal, principal_group, resource, resource_prefix, policy, "order", created_by, policy_hash, comment)
                            VALUES ($1, $2, $3, $4, $10, $5, $6, $7, md5($8), $9)
                            ON CONFLICT (policy_hash) DO UPDATE
                                SET "order" = EXCLUDED."order",
                                    comment = EXCLUDED.comment,
                                    created_by = EXCLUDED.created_by,
                                    created_at = timezone('UTC', now())
                                WHERE policies.system = FALSE
                            RETURNING id, policy, principal, principal_group, action, resource,
                                      to_char(created_at, 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"'),
                                      created_by, "order", comment, resource_prefix, policy_hash,
                                      (xmax = 0) AS created;
                        "#,
                    )
                    .await?;

                let row = client
                    .query_opt(
                        &stmt,
                        &[&action_str, &sub, &group_id, &resource_str, &policy, &order, &created_by, &canonical, &comment, &resource_prefix_str],
                    )
                    .await
                    .map_err(|err| StorageErr {
                        msg: format!("Failed to insert policy to the database: {}", err),
                        kind: StorageErrKind::Internal,
                    })?;

                match row {
                    Some(row) => Ok::<PolicyUpsertResult, StorageErr>(
                        PostgresPolicyStorage::read_upsert_row(&row),
                    ),
                    None => Err(StorageErr {
                        msg: "A system policy with the same text already exists.".to_string(),
                        kind: StorageErrKind::Forbidden,
                    }),
                }
            })
            .await??;

        Ok(result)
    }

    #[tracing::instrument(level = "info", skip(self), fields(otel.kind = "client", db.name = self.db_name))]
    async fn add_policy_batch(
        &self,
        batches: Vec<PolicyDbBatch>,
    ) -> Result<Vec<PolicyUpsertResult>, StorageErr> {
        if batches.is_empty() {
            return Ok(vec![]);
        }

        // Deduplicate by `canonical_hash` before sending: Postgres rejects an
        // ON CONFLICT command that touches the same conflict target twice.
        // Each input batch maps to the index of the row that will actually be
        // sent to the database, so duplicates in the input share the same
        // result row, preserving the original sequential behavior where ON
        // CONFLICT returned the existing row for each duplicate.
        let mut hash_to_dedup: std::collections::HashMap<String, usize> =
            std::collections::HashMap::with_capacity(batches.len());
        let mut input_to_dedup: Vec<usize> = Vec::with_capacity(batches.len());
        let mut actions: Vec<Option<String>> = Vec::with_capacity(batches.len());
        let mut principals: Vec<Option<String>> = Vec::with_capacity(batches.len());
        let mut groups: Vec<Option<String>> = Vec::with_capacity(batches.len());
        let mut resources: Vec<Option<String>> = Vec::with_capacity(batches.len());
        let mut resource_prefixes: Vec<Option<String>> = Vec::with_capacity(batches.len());
        let mut policy_texts: Vec<String> = Vec::with_capacity(batches.len());
        let mut systems: Vec<bool> = Vec::with_capacity(batches.len());
        let mut orders: Vec<i32> = Vec::with_capacity(batches.len());
        let mut created_bys: Vec<String> = Vec::with_capacity(batches.len());
        let mut hashes: Vec<String> = Vec::with_capacity(batches.len());
        let mut comments: Vec<Option<String>> = Vec::with_capacity(batches.len());

        for batch in batches {
            if batch.canonical_hash.is_empty() {
                return Err(StorageErr {
                    msg: "PolicyDbBatch.canonical_hash must be set by the caller; this is a service bug.".to_string(),
                    kind: StorageErrKind::Internal,
                });
            }

            if let Some(&idx) = hash_to_dedup.get(&batch.canonical_hash) {
                input_to_dedup.push(idx);
                continue;
            }

            let idx = hashes.len();
            hash_to_dedup.insert(batch.canonical_hash.clone(), idx);
            actions.push(batch.action.as_ref().map(|a| a.entity_id()));
            principals.push(batch.principal.map(|p| p.id));
            groups.push(batch.group.map(|g| g.id));
            resources.push(batch.resource.as_ref().map(|r| r.to_string()));
            resource_prefixes.push(batch.resource_prefix.as_ref().map(|r| r.to_string()));
            policy_texts.push(batch.policy);
            systems.push(batch.system);
            orders.push(batch.order);
            created_bys.push(batch.created_by);
            hashes.push(batch.canonical_hash);
            comments.push(batch.comment);
            input_to_dedup.push(idx);
        }

        let pool = self.pool.clone();

        let result = self
            .runtime
            .spawn(async move {
                let client = pool.get().await?;

                // Single batched upsert. `unnest WITH ORDINALITY` tags each row with its
                // input position; the top-level `ORDER BY idx` forces Postgres to assign
                // ids in input order. The `policy_hash` is supplied directly by the
                // caller - no md5() on the server, no canonicalize_policy() on the client.
                //
                // The `WHERE NOT EXISTS (... system policy ...)` guard makes the whole
                // statement atomic with respect to system policies: if any input hash
                // matches an existing system policy, the SELECT yields no rows, so the
                // batch inserts nothing and `RETURNING` is empty. This rejects the entire
                // batch without partially committing the non-colliding rows.
                let stmt = client
                    .prepare_cached(
                        r#"
                            WITH input AS (
                                SELECT action, principal, principal_group, resource, resource_prefix, policy,
                                       sys, ord, created_by, hash, comment, idx
                                FROM unnest(
                                    $1::text[], $2::text[], $3::text[], $4::text[],
                                    $5::bool[], $6::int4[], $7::text[], $8::text[], $9::text[],
                                    $10::text[], $11::text[]
                                ) WITH ORDINALITY AS t(
                                    action, principal, resource, policy,
                                    sys, ord, created_by, hash, comment, principal_group, resource_prefix, idx
                                )
                            )
                            INSERT INTO policies
                                (action, principal, principal_group, resource, resource_prefix, policy, system, "order", created_by, policy_hash, comment)
                            SELECT action, principal, principal_group, resource, resource_prefix, policy, sys, ord, created_by, hash, comment
                            FROM input
                            WHERE NOT EXISTS (
                                SELECT 1 FROM policies p
                                WHERE p.system = TRUE
                                  AND p.policy_hash IN (SELECT hash FROM input)
                            )
                            ORDER BY idx
                            ON CONFLICT (policy_hash) DO UPDATE
                                SET "order" = EXCLUDED."order",
                                    comment = EXCLUDED.comment,
                                    created_by = EXCLUDED.created_by,
                                    created_at = timezone('UTC', now())
                            RETURNING id, policy, principal, principal_group, action, resource,
                                      to_char(created_at, 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"'),
                                      created_by, "order", comment, resource_prefix, policy_hash,
                                      (xmax = 0) AS created;
                        "#,
                    )
                    .await?;

                let rows = client
                    .query(
                        &stmt,
                        &[
                            &actions,
                            &principals,
                            &resources,
                            &policy_texts,
                            &systems,
                            &orders,
                            &created_bys,
                            &hashes,
                            &comments,
                            &groups,
                            &resource_prefixes,
                        ],
                    )
                    .await
                    .map_err(|err| StorageErr {
                        msg: format!("Failed to insert policy batch to the database: {}", err),
                        kind: StorageErrKind::Internal,
                    })?;

                // System policies are skipped by the `WHERE system = FALSE`
                // guard, so they never appear in `RETURNING`. Map results back
                // to inputs by canonical hash; a missing hash means the input
                // collided with a system policy and the whole batch is rejected.
                let mut by_hash: std::collections::HashMap<String, PolicyUpsertResult> =
                    std::collections::HashMap::with_capacity(rows.len());
                for row in &rows {
                    let result = PostgresPolicyStorage::read_upsert_row(row);
                    by_hash.insert(result.policy.policy_hash.clone(), result);
                }

                let mut results: Vec<PolicyUpsertResult> = Vec::with_capacity(input_to_dedup.len());
                for &dedup_idx in &input_to_dedup {
                    match by_hash.get(&hashes[dedup_idx]) {
                        Some(result) => results.push(result.clone()),
                        None => {
                            return Err(StorageErr {
                                msg: "A system policy with the same text already exists."
                                    .to_string(),
                                kind: StorageErrKind::Forbidden,
                            });
                        }
                    }
                }
                Ok::<Vec<PolicyUpsertResult>, StorageErr>(results)
            })
            .await??;

        Ok(result)
    }

    #[tracing::instrument(level = "info", skip(self), fields(otel.kind = "client", db.name = self.db_name))]
    async fn update_policy(
        &self,
        policy_id: PolicyId,
        policy: &str,
        action: Option<&Action>,
        principal: Option<&Principal>,
        group: Option<&Group>,
        resource: Option<&Resource>,
        resource_prefix: Option<&Resource>,
        order: i32,
        created_by: &str,
        comment: Option<&str>,
    ) -> Result<Policy, StorageErr> {
        let policy = policy.trim().to_string();
        let canonical = canonicalize_policy(&policy).map_err(|err| StorageErr {
            msg: err.msg,
            kind: StorageErrKind::Constraint,
        })?;
        let action = action.cloned();
        let principal = principal.cloned();
        let group = group.cloned();
        let resource = resource.cloned();
        let resource_prefix = resource_prefix.cloned();
        let created_by = created_by.to_string();
        let comment = comment.map(|c| c.to_string());
        let pool = self.pool.clone();

        let result = self
            .runtime
            .spawn(async move {
                let action_str = action.as_ref().map(|a| a.entity_id());
                let sub = principal.as_ref().map(|p| p.id.as_str());
                let group_id = group.as_ref().map(|g| g.id.as_str());
                let resource_str = resource.as_ref().map(|r| r.to_string());
                let resource_prefix_str = resource_prefix.as_ref().map(|r| r.to_string());

                let client = pool.get().await?;

                // Single-statement upsert: if another row (id <> $1) already
                // carries the same canonical hash, its row is returned and
                // no write happens. Otherwise we upsert the target id. Both
                // branches share one snapshot, so we cannot produce a stale
                // lookup followed by a mutating write. In the narrow window
                // where a concurrent writer inserts a conflicting hash after
                // our CTE read, Postgres' unique index on `policy_hash`
                // rejects the INSERT with a constraint error instead of
                // corrupting data.
                let stmt = client
                    .prepare_cached(
                        r#"
                            WITH existing AS (
                                SELECT id, policy, principal, principal_group, action, resource,
                                       created_at, created_by, "order", comment, resource_prefix, policy_hash
                                FROM policies
                                WHERE policy_hash = md5($9) AND id <> $1
                                LIMIT 1
                            ),
                            upserted AS (
                                INSERT INTO policies
                                    (id, action, principal, principal_group, resource, resource_prefix, policy, "order", created_by, policy_hash, comment)
                                SELECT $1, $2, $3, $4, $5, $11, $6, $7, $8, md5($9), $10
                                WHERE NOT EXISTS (SELECT 1 FROM existing)
                                ON CONFLICT (id) DO UPDATE
                                    SET action = EXCLUDED.action,
                                        principal = EXCLUDED.principal,
                                        principal_group = EXCLUDED.principal_group,
                                        resource = EXCLUDED.resource,
                                        resource_prefix = EXCLUDED.resource_prefix,
                                        policy = EXCLUDED.policy,
                                        "order" = EXCLUDED."order",
                                        policy_hash = EXCLUDED.policy_hash,
                                        comment = EXCLUDED.comment
                                RETURNING id, policy, principal, principal_group, action, resource,
                                          created_at, created_by, "order", comment, resource_prefix, policy_hash
                            )
                            SELECT id, policy, principal, principal_group, action, resource,
                                   to_char(created_at, 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"'),
                                   created_by, "order", comment, resource_prefix, policy_hash
                            FROM upserted
                            UNION ALL
                            SELECT id, policy, principal, principal_group, action, resource,
                                   to_char(created_at, 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"'),
                                   created_by, "order", comment, resource_prefix, policy_hash
                            FROM existing
                            LIMIT 1;
                        "#,
                    )
                    .await?;

                let row = client
                    .query_one(
                        &stmt,
                        &[&policy_id, &action_str, &sub, &group_id, &resource_str, &policy, &order, &created_by, &canonical, &comment, &resource_prefix_str],
                    )
                    .await
                    .map_err(|err| StorageErr {
                        msg: format!("Failed to update policy in the database: {}", err),
                        kind: StorageErrKind::Internal,
                    })?;

                Ok::<Policy, StorageErr>(PostgresPolicyStorage::read_row(&row))
            })
            .await??;

        Ok(result)
    }

    #[tracing::instrument(level = "info", skip(self), fields(otel.kind = "client", db.name = self.db_name))]
    async fn get_policies(
        &self,
        action: Option<&Action>,
        principal: Option<&Principal>,
        groups: &[String],
        resource: Option<&Resource>,
        resource_prefixes: &[Resource],
    ) -> Result<Vec<Policy>, StorageErr> {
        let action = action.cloned();
        let principal = principal.cloned();
        let groups = groups.to_vec();
        let resource = resource.cloned();
        let prefixes: Vec<String> = resource_prefixes.iter().map(|r| r.to_string()).collect();
        let pool = self.pool.clone();

        let result = self
            .runtime
            .spawn(async move {
                let action_str = action.as_ref().map(|a| a.entity_id());
                let sub = principal.as_ref().map(|p| p.id.as_str());
                let resource_str = resource.as_ref().map(|r| r.to_string());

                let client = pool.get().await?;
                let stmt = client
                    .prepare_cached(
                        r#"
                            SELECT id, policy, principal, principal_group, action, resource, "order", resource_prefix, policy_hash
                            FROM policies
                            WHERE
                                (principal = $1 OR principal IS NULL) AND
                                (principal_group IS NULL OR principal_group = ANY($4)) AND
                                (action = $2 OR action IS NULL) AND
                                (resource = $3 OR resource IS NULL) AND
                                (resource_prefix IS NULL OR resource_prefix = ANY($5))
                            ORDER BY "order" ASC, id ASC;
                        "#,
                    )
                    .await?;

                let rows = client
                    .query(&stmt, &[&sub, &action_str, &resource_str, &groups, &prefixes])
                    .await?;

                let mut results = Vec::new();
                for row in rows.iter() {
                    let policy = PostgresPolicyStorage::read_row_for_evaluation(row);
                    results.push(policy);
                }
                Ok::<Vec<Policy>, StorageErr>(results)
            })
            .await??;

        Ok(result)
    }

    #[tracing::instrument(level = "info", skip(self), fields(otel.kind = "client", db.name = self.db_name))]
    async fn get_policies_batched(
        &self,
        principal: Option<&Principal>,
        groups: &[String],
        batches: &[PolicyQueryBatch],
        resource_prefixes: &[Resource],
    ) -> Result<Vec<Policy>, StorageErr> {
        if batches.is_empty() {
            return Ok(vec![]);
        }

        let principal = principal.cloned();
        let groups = groups.to_vec();
        let batches = batches.to_vec();
        let prefixes: Vec<String> = resource_prefixes.iter().map(|r| r.to_string()).collect();
        let pool = self.pool.clone();

        let result = self
            .runtime
            .spawn(async move {
                let client = pool.get().await?;

                let actions: Vec<String> = batches.iter().map(|b| b.action.entity_id()).collect();
                let resources: Vec<Option<String>> = batches
                    .iter()
                    .map(|b| b.resource.clone().map(|r| r.to_string()))
                    .collect();

                let stmt = client
                    .prepare_cached(
                        r#"
                            SELECT id, policy, principal, principal_group, action, resource, "order", resource_prefix, policy_hash
                            FROM policies
                            WHERE
                                (principal = $1 OR principal IS NULL) AND
                                (principal_group IS NULL OR principal_group = ANY($4)) AND
                                (resource IS NULL OR resource = ANY($2)) AND
                                (action IS NULL OR action = ANY($3)) AND
                                (resource_prefix IS NULL OR resource_prefix = ANY($5))
                            ORDER BY "order" ASC, id ASC
                        "#,
                    )
                    .await?;

                let rows = client
                    .query(
                        &stmt,
                        &[
                            &principal.map(|p| p.id.clone()),
                            &resources,
                            &actions,
                            &groups,
                            &prefixes,
                        ],
                    )
                    .await?;

                let mut results = Vec::new();
                for row in rows.iter() {
                    let policy = PostgresPolicyStorage::read_row_for_evaluation(row);
                    results.push(policy);
                }
                Ok::<Vec<Policy>, StorageErr>(results)
            })
            .await??;

        Ok(result)
    }

    #[tracing::instrument(level = "info", skip(self), fields(otel.kind = "client", db.name = self.db_name))]
    async fn get_policies_paginated(
        &self,
        action: Option<&Action>,
        principal: Option<&Principal>,
        group: Option<&Group>,
        resource: Option<&Resource>,
        resource_prefixes: &[Resource],
        order: Option<i32>,
        page: i64,
        limit: i64,
    ) -> Result<PageResult<Policy>, StorageErr> {
        if page < 1 {
            return Err(StorageErr {
                msg: "`page` must be greater than 1.".to_string(),
                kind: StorageErrKind::InvalidPagination,
            });
        }

        if limit <= 0 || limit > 50 {
            return Err(StorageErr {
                msg: "`limit` must be in range between 0 and 50.".to_string(),
                kind: StorageErrKind::InvalidPagination,
            });
        }

        let sub = principal.as_ref().map(|p| p.id.clone());
        let group_id = group.map(|g| g.id.clone());
        let action = action.cloned();
        let resource = resource.cloned();
        let prefixes: Vec<String> = resource_prefixes.iter().map(|r| r.to_string()).collect();
        let offset = (page - 1) * limit;
        let pool = self.pool.clone();

        let result = self
            .runtime
            .spawn(async move {
                let client = pool.get().await?;
                let mut conditions = vec![];
                let mut params: Vec<Box<dyn ToSql + Sync + Send>> = vec![];

                match sub {
                    Some(sub) if sub.is_empty() => {
                        conditions.push("(principal IS NULL)".into());
                    }
                    Some(sub) => {
                        params.push(Box::new(sub));
                        conditions.push(format!("(principal = ${})", params.len()));
                    }
                    _ => {}
                };

                match group_id {
                    Some(group_id) if group_id.is_empty() => {
                        conditions.push("(principal_group IS NULL)".into());
                    }
                    Some(group_id) => {
                        params.push(Box::new(group_id));
                        conditions.push(format!("(principal_group = ${})", params.len()));
                    }
                    _ => {}
                };

                match action {
                    Some(action) if action.name == "*" => {
                        params.push(Box::new(format!("{}:%", action.service)));
                        conditions.push(format!("(action LIKE ${})", params.len()));
                    }
                    Some(action) if action.name.is_empty() => {
                        conditions.push("(action IS NULL)".into());
                    }
                    Some(action) => {
                        params.push(Box::new(action.entity_id()));
                        conditions.push(format!("(action = ${})", params.len()));
                    }
                    _ => {}
                }

                match resource {
                    Some(resource) if resource.id.ends_with('*') => {
                        // A trailing `*` requests every resource whose stored
                        // `kind::"id"` form starts with the literal prefix. We
                        // express that as a half-open code-point range so the
                        // `varchar_pattern_ops` index can serve it: `lo` is the
                        // prefix, `hi` is the smallest string greater than every
                        // string starting with `lo`. The pattern operators
                        // `~>=~`/`~<~` (not ordinary `>=`/`<`) are what that
                        // index supports and they work with bound parameters.
                        // The same range is matched against `resource_prefix`
                        // so a prefix listing also surfaces hierarchy nodes.
                        let id_prefix = resource.id.strip_suffix('*').unwrap_or_default();
                        let lo = format!(r#"{}::"{}"#, resource.kind, id_prefix);
                        let hi = prefix_upper_bound(&lo);
                        params.push(Box::new(lo));
                        let lo_idx = params.len();
                        match hi {
                            Some(hi) => {
                                params.push(Box::new(hi));
                                let hi_idx = params.len();
                                conditions.push(format!(
                                    "((resource ~>=~ ${lo_idx} AND resource ~<~ ${hi_idx}) OR \
                                      (resource_prefix ~>=~ ${lo_idx} AND resource_prefix ~<~ ${hi_idx}))"
                                ));
                            }
                            None => {
                                conditions.push(format!(
                                    "(resource ~>=~ ${lo_idx} OR resource_prefix ~>=~ ${lo_idx})"
                                ));
                            }
                        }
                    }
                    Some(resource) if resource.id.is_empty() => {
                        conditions
                            .push("(resource IS NULL AND resource_prefix IS NULL)".into());
                    }
                    Some(resource) if !prefixes.is_empty() => {
                        // Inherited scope (`scope=inherited`): return the exact
                        // policy on this resource plus every hierarchy policy
                        // whose prefix is an ancestor of it.
                        params.push(Box::new(resource.to_string()));
                        let exact_idx = params.len();
                        params.push(Box::new(prefixes.clone()));
                        let prefixes_idx = params.len();
                        conditions.push(format!(
                            "(resource = ${exact_idx} OR resource_prefix = ANY(${prefixes_idx}))"
                        ));
                    }
                    Some(resource) => {
                        // Exact listing is inclusive of a hierarchy node defined
                        // on exactly this resource URL.
                        params.push(Box::new(resource.to_string()));
                        let idx = params.len();
                        conditions.push(format!(
                            "(resource = ${idx} OR resource_prefix = ${idx})"
                        ));
                    }
                    _ => {}
                }

                if let Some(order) = order {
                    params.push(Box::new(order));
                    conditions.push(format!(r#"("order" = ${})"#, params.len()));
                }

                let where_stmt = if conditions.is_empty() {
                    String::new()
                } else {
                    format!("WHERE {}", conditions.join(" AND "))
                };
                let sql = format!(
                    r#"
                        SELECT id, policy, principal, principal_group, action, resource, to_char(created_at, 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), created_by, "order", comment, resource_prefix, policy_hash
                        FROM policies
                        {}
                        ORDER BY "order" ASC, id ASC
                        LIMIT {}
                        OFFSET {}
                    "#,
                    where_stmt,
                    limit,
                    offset
                );

                let stmt = client.prepare_cached(&sql).await?;
                let params_refs: Vec<&(dyn ToSql + Sync)> =
                    params.iter().map(|p| p.as_ref() as &(dyn ToSql + Sync)).collect();
                let result = client.query(&stmt, &params_refs).await;

                match result {
                    Ok(rows) => {
                        let mut policies = Vec::new();
                        for row in rows.iter() {
                            let policy = PostgresPolicyStorage::read_row(row);
                            policies.push(policy);
                        }
                        Ok(PageResult {
                            page,
                            page_size: policies.len(),
                            page_count: None,
                            items: policies,
                        })
                    }
                    Err(err) => Err(StorageErr::from(err)),
                }
            })
            .await??;

        Ok(result)
    }

    #[tracing::instrument(level = "info", skip(self), fields(otel.kind = "client", db.name = self.db_name))]
    async fn get_policy_by_id(&self, policy_id: &PolicyId) -> Result<Policy, StorageErr> {
        let policy_id = *policy_id;
        let pool = self.pool.clone();

        let result = self
            .runtime
            .spawn(async move {
                let client = pool.get().await?;
                let stmt = client
                    .prepare_cached(
                        r#"
                            SELECT id, policy, principal, principal_group, action, resource, to_char(created_at, 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"'), created_by, "order", comment, resource_prefix, policy_hash
                            FROM policies
                            WHERE id = $1
                        "#,
                    )
                    .await?;

                let result = client.query_one(&stmt, &[&policy_id]).await;
                match result {
                    Ok(row) => Ok(PostgresPolicyStorage::read_row(&row)),
                    Err(_) => Err(StorageErr {
                        msg: "Policy not found.".to_string(),
                        kind: StorageErrKind::DoesNotExist,
                    }),
                }
            })
            .await??;

        Ok(result)
    }

    #[tracing::instrument(level = "info", skip(self), fields(otel.kind = "client", db.name = self.db_name))]
    async fn remove_policy(&self, policy_id: &PolicyId) -> Result<(), StorageErr> {
        let policy_id = *policy_id;
        let pool = self.pool.clone();

        let result = self
            .runtime
            .spawn(async move {
                let client = pool.get().await?;
                // Delete only non-system rows in a single statement and report
                // whether the target was a system policy, so a concurrent change
                // to the row cannot slip a system policy past the guard.
                let stmt = client
                    .prepare_cached(
                        r#"
                            WITH existing AS (
                                SELECT system FROM policies WHERE id = $1
                            ),
                            deleted AS (
                                DELETE FROM policies WHERE id = $1 AND system = FALSE
                                RETURNING id
                            )
                            SELECT COALESCE((SELECT system FROM existing), FALSE) AS is_system;
                        "#,
                    )
                    .await?;

                let is_system: bool = client
                    .query_one(&stmt, &[&policy_id])
                    .await
                    .map_err(|err| StorageErr {
                        msg: format!("Failed to remove policy from the database: {}", err),
                        kind: StorageErrKind::Internal,
                    })?
                    .get(0);

                if is_system {
                    return Err(StorageErr {
                        msg: "System policies cannot be deleted.".to_string(),
                        kind: StorageErrKind::Forbidden,
                    });
                }

                Ok::<(), StorageErr>(())
            })
            .await??;

        Ok(result)
    }
}

/// Smallest string strictly greater than every string starting with `prefix`,
/// turning a prefix match into the half-open range `[prefix, upper)`. Returns
/// `None` when no such bound exists - an empty prefix, or one whose trailing
/// scalar is already the maximum - in which case only the lower bound applies.
pub(crate) fn prefix_upper_bound(prefix: &str) -> Option<String> {
    let mut chars: Vec<char> = prefix.chars().collect();
    while let Some(last) = chars.pop() {
        let mut next = last as u32 + 1;
        if next == 0xD800 {
            next = 0xE000;
        }
        if let Some(next) = char::from_u32(next) {
            let mut upper: String = chars.iter().collect();
            upper.push(next);
            return Some(upper);
        }
    }
    None
}

pub struct PostgresMetadataStorage {
    pool: Arc<dyn ConnectionPool>,
    runtime: Arc<Runtime>,
    options: PostgresDatabaseOptions,
    db_name: String,
    default_id_claim: String,
}

impl PostgresMetadataStorage {
    pub fn new(
        pool: Arc<dyn ConnectionPool>,
        runtime: Arc<Runtime>,
        db_name: &str,
        options: PostgresDatabaseOptions,
        default_id_claim: String,
    ) -> Self {
        Self {
            pool,
            runtime,
            db_name: db_name.to_string(),
            options,
            default_id_claim,
        }
    }

    pub async fn create_tables(&self) -> Result<(), StorageErr> {
        let table_stmt = if self.options.temporary {
            "TEMP TABLE"
        } else {
            "TABLE"
        };

        let client = self.pool.get().await?;
        client
            .execute(
                &format!(
                    r#"CREATE {table_stmt} IF NOT EXISTS metadata_resource_types (
                        service VARCHAR(100) NOT NULL,
                        type VARCHAR(100) NOT NULL,
                        evaluation_priority VARCHAR(30) NOT NULL,
                        PRIMARY KEY(service, type)
                    )"#
                ),
                &[],
            )
            .await?;

        client
            .execute(
                &format!(
                    r#"CREATE {table_stmt} IF NOT EXISTS metadata_services (
                        service VARCHAR(100) NOT NULL PRIMARY KEY,
                        id_claim VARCHAR(20) NOT NULL
                    )"#
                ),
                &[],
            )
            .await?;

        client
            .execute(
                &format!(
                    r#"CREATE {table_stmt} IF NOT EXISTS metadata_actions (
                        service VARCHAR(100) NOT NULL,
                        action VARCHAR(100) NOT NULL,
                        PRIMARY KEY(service, action)
                    )"#
                ),
                &[],
            )
            .await?;

        client
            .execute(
                r#"ALTER TABLE metadata_services ADD COLUMN IF NOT EXISTS system BOOLEAN NOT NULL DEFAULT FALSE"#,
                &[],
            )
            .await?;

        client
            .execute(
                r#"ALTER TABLE metadata_actions ADD COLUMN IF NOT EXISTS system BOOLEAN NOT NULL DEFAULT FALSE"#,
                &[],
            )
            .await?;

        client
            .execute(
                r#"ALTER TABLE metadata_resource_types ADD COLUMN IF NOT EXISTS system BOOLEAN NOT NULL DEFAULT FALSE"#,
                &[],
            )
            .await?;

        client
            .execute(
                r#"ALTER TABLE metadata_resource_types ADD COLUMN IF NOT EXISTS parent_type VARCHAR(100) NULL"#,
                &[],
            )
            .await?;

        // Enforces global resource type uniqueness. Creation fails with a
        // unique-violation if existing rows register the same kind under more
        // than one service; operators must de-duplicate those kinds before the
        // upgrade can complete.
        client
            .execute(
                r#"CREATE UNIQUE INDEX IF NOT EXISTS ux_metadata_resource_types_type ON metadata_resource_types (type)"#,
                &[],
            )
            .await
            .map_err(|err| {
                if let Some(db_err) = err.as_db_error() {
                    if db_err.code() == &SqlState::UNIQUE_VIOLATION {
                        return StorageErr {
                            msg: format!(
                                "Cannot enforce global resource type uniqueness; de-duplicate resource type kinds registered under multiple services before upgrading: {}",
                                err
                            ),
                            kind: StorageErrKind::Constraint,
                        };
                    }
                }
                StorageErr::from(err)
            })?;

        Ok(())
    }

    pub async fn drop_tables(&self) -> Result<(), StorageErr> {
        let client = self.pool.get().await?;
        client
            .execute(r#"DROP TABLE IF EXISTS metadata_resource_types;"#, &[])
            .await?;

        client
            .execute(r#"DROP TABLE IF EXISTS metadata_services;"#, &[])
            .await?;

        client
            .execute(r#"DROP TABLE IF EXISTS metadata_actions;"#, &[])
            .await?;

        Ok(())
    }
}

#[async_trait]
impl MetadataStorage for PostgresMetadataStorage {
    async fn initialize(&self) -> Result<(), StorageErr> {
        self.create_tables().await?;
        Ok(())
    }

    async fn destroy(&self) -> Result<(), StorageErr> {
        self.drop_tables().await?;
        Ok(())
    }

    #[tracing::instrument(level = "info", skip(self, metadata), fields(otel.kind = "client", db.name = self.db_name))]
    async fn set_system_metadata(&self, metadata: MetadataConfig) -> Result<(), StorageErr> {
        let pool = self.pool.clone();
        let default_id_claim = self.default_id_claim.clone();

        self.runtime
            .spawn(async move {
                let mut client = pool.get().await?;
                let tx = client.transaction().await?;

                tx.execute("DELETE FROM metadata_actions WHERE system = TRUE", &[])
                    .await?;
                tx.execute("DELETE FROM metadata_resource_types WHERE system = TRUE", &[])
                    .await?;
                tx.execute("DELETE FROM metadata_services WHERE system = TRUE", &[])
                    .await?;

                let svc_stmt = tx
                    .prepare_cached(
                        r#"INSERT INTO metadata_services (service, id_claim, system)
                           VALUES ($1, $2, TRUE)
                           ON CONFLICT (service) DO UPDATE SET id_claim = $2, system = TRUE"#,
                    )
                    .await?;

                let action_stmt = tx
                    .prepare_cached(
                        r#"INSERT INTO metadata_actions (service, action, system)
                           VALUES ($1, $2, TRUE)
                           ON CONFLICT (service, action) DO UPDATE SET system = TRUE"#,
                    )
                    .await?;

                let rt_stmt = tx
                    .prepare_cached(
                        r#"INSERT INTO metadata_resource_types (service, type, evaluation_priority, parent_type, system)
                           VALUES ($1, $2, $3, $4, TRUE)
                           ON CONFLICT (service, type) DO UPDATE SET evaluation_priority = $3, parent_type = $4, system = TRUE"#,
                    )
                    .await?;

                for entry in &metadata.services {
                    let id_claim = if entry.principal.id_claim.is_empty() {
                        default_id_claim.as_str()
                    } else {
                        entry.principal.id_claim.as_str()
                    };
                    tx.execute(&svc_stmt, &[&entry.name, &id_claim]).await?;

                    for action in &entry.actions {
                        tx.execute(&action_stmt, &[&entry.name, action]).await?;
                    }

                    for rt in &entry.resource_types {
                        let priority = rt.evaluation_priority.to_string();
                        tx.execute(
                            &rt_stmt,
                            &[&entry.name, &rt.kind, &priority, &rt.parent_type],
                        )
                        .await
                        .map_err(map_resource_type_conflict)?;
                    }
                }

                tx.commit().await?;
                Ok::<(), StorageErr>(())
            })
            .await??;

        Ok(())
    }

    #[tracing::instrument(level = "info", skip(self), fields(otel.kind = "client", db.name = self.db_name))]
    async fn get_meta(
        &self,
        action: &Action,
        resource: Option<&Resource>,
    ) -> Result<Meta, StorageErr> {
        let action_service = action.service.clone();
        let resource_type = resource.map(|r| r.kind.clone());
        let default_id_claim = self.default_id_claim.clone();
        let pool = self.pool.clone();

        let result = self
            .runtime
            .spawn(async move {
                let client = pool.get().await?;
                let stmt = client
                    .prepare_cached(
                        r#"
                            SELECT r.evaluation_priority, p.id_claim, r.parent_type
                            FROM metadata_resource_types as r
                            FULL JOIN metadata_services as p ON r.service = p.service
                            WHERE (p.service = $1 OR r.service = $1) AND (r IS NULL OR r.type = $2 OR $2 IS NULL)
                            LIMIT 1;
                        "#,
                    )
                    .await?;

                if let Some(row) = client
                    .query_opt(&stmt, &[&action_service, &resource_type])
                    .await?
                {
                    let evaluation_priority: Option<&str> = row.get(0);
                    let evaluation_priority: EvaluationPriority = evaluation_priority
                        .unwrap_or("forbid")
                        .try_into()?;

                    let id_claim = row
                        .get::<usize, Option<&str>>(1)
                        .filter(|claim| !claim.is_empty())
                        .unwrap_or(&default_id_claim);

                    let parent_type: Option<String> = row.get(2);
                    let has_resource = resource_type.is_some();

                    Ok::<Meta, StorageErr>(Meta {
                        service: ServiceMeta {
                            service: action_service.clone(),
                            id_claim: id_claim.to_string(),
                        },
                        resource: ResourceTypeMeta {
                            service: action_service,
                            evaluation_priority: if has_resource {
                                evaluation_priority
                            } else {
                                EvaluationPriority::default()
                            },
                            kind: resource_type.unwrap_or_default(),
                            parent_type: if has_resource { parent_type } else { None },
                        },
                    })
                } else {
                    Ok::<Meta, StorageErr>(Meta {
                        service: ServiceMeta {
                            service: action_service.clone(),
                            id_claim: default_id_claim,
                        },
                        resource: ResourceTypeMeta::new(
                            &action_service,
                            resource_type.as_deref().unwrap_or(""),
                        ),
                    })
                }
            })
            .await??;

        Ok(result)
    }

    #[tracing::instrument(level = "info", skip(self), fields(otel.kind = "client", db.name = self.db_name))]
    async fn get_services(&self) -> Result<Vec<ServiceMeta>, StorageErr> {
        let pool = self.pool.clone();
        let default_id_claim = self.default_id_claim.clone();

        let result = self
            .runtime
            .spawn(async move {
                let client = pool.get().await?;
                let stmt = client
                    .prepare_cached(r#"SELECT service, id_claim FROM metadata_services"#)
                    .await?;

                let rows = client.query(&stmt, &[]).await?;
                let services: Vec<ServiceMeta> = rows
                    .iter()
                    .map(|row| {
                        let id_claim: String = row.get(1);
                        ServiceMeta {
                            service: row.get(0),
                            id_claim: if id_claim.is_empty() {
                                default_id_claim.clone()
                            } else {
                                id_claim
                            },
                        }
                    })
                    .collect();

                Ok::<Vec<ServiceMeta>, StorageErr>(services)
            })
            .await??;

        Ok(result)
    }

    #[tracing::instrument(level = "info", skip(self), fields(otel.kind = "client", db.name = self.db_name))]
    async fn get_service_meta(&self, service: &str) -> Result<ServiceMeta, StorageErr> {
        let action = Action {
            name: String::new(),
            service: service.to_string(),
        };
        let meta = self.get_meta(&action, None).await;
        Ok(meta?.service)
    }

    #[tracing::instrument(level = "info", skip(self), fields(otel.kind = "client", db.name = self.db_name))]
    async fn set_service_meta(&self, principal: &ServiceMeta) -> Result<(), StorageErr> {
        let service = principal.service.clone();
        let id_claim = principal.id_claim.clone();
        let pool = self.pool.clone();

        let result = self
            .runtime
            .spawn(async move {
                let client = pool.get().await?;
                let stmt = client
                    .prepare_cached(
                        r#"
                            INSERT INTO metadata_services (service, id_claim)
                            VALUES ($1, $2)
                            ON CONFLICT (service) DO UPDATE
                                SET id_claim = $2;
                        "#,
                    )
                    .await?;

                client.execute(&stmt, &[&service, &id_claim]).await?;

                Ok::<(), StorageErr>(())
            })
            .await??;

        Ok(result)
    }

    #[tracing::instrument(level = "info", skip(self), fields(otel.kind = "client", db.name = self.db_name))]
    async fn delete_service_meta(&self, service: &str) -> Result<(), StorageErr> {
        let service = service.to_string();
        let pool = self.pool.clone();

        let result = self
            .runtime
            .spawn(async move {
                let client = pool.get().await?;
                let stmt = client
                    .prepare_cached(
                        r#"
                            DELETE FROM metadata_services
                            WHERE service = $1;
                        "#,
                    )
                    .await?;
                client.execute(&stmt, &[&service]).await?;

                let stmt = client
                    .prepare_cached(
                        r#"
                            DELETE FROM metadata_actions
                            WHERE service = $1;
                        "#,
                    )
                    .await?;
                client.execute(&stmt, &[&service]).await?;

                let stmt = client
                    .prepare_cached(
                        r#"
                            DELETE FROM metadata_resource_types
                            WHERE service = $1;
                        "#,
                    )
                    .await?;
                client.execute(&stmt, &[&service]).await?;
                Ok::<(), StorageErr>(())
            })
            .await??;

        Ok(result)
    }

    #[tracing::instrument(level = "info", skip(self), fields(otel.kind = "client", db.name = self.db_name))]
    async fn get_service_actions(&self, service: &str) -> Result<Vec<Action>, StorageErr> {
        let service = service.to_string();
        let pool = self.pool.clone();

        let result = self
            .runtime
            .spawn(async move {
                let client = pool.get().await?;
                let stmt = client
                    .prepare_cached(
                        r#"SELECT service, action FROM metadata_actions WHERE service = $1"#,
                    )
                    .await?;

                let rows = client.query(&stmt, &[&service]).await?;
                let actions: Vec<Action> = rows
                    .iter()
                    .map(|row| Action {
                        service: row.get(0),
                        name: row.get(1),
                    })
                    .collect();

                Ok::<Vec<Action>, StorageErr>(actions)
            })
            .await??;

        Ok(result)
    }

    #[tracing::instrument(level = "info", skip(self), fields(otel.kind = "client", db.name = self.db_name))]
    async fn replace_service_actions(
        &self,
        service: &str,
        actions: Vec<Action>,
    ) -> Result<(), StorageErr> {
        let service = service.to_string();
        let pool = self.pool.clone();

        self.runtime
            .spawn(async move {
                let client = pool.get().await?;
                let stmt = client
                    .prepare_cached(r#"DELETE FROM metadata_actions WHERE service = $1"#)
                    .await?;

                client.execute(&stmt, &[&service]).await?;

                let stmt = client
                    .prepare_cached(
                        r#"
                            INSERT INTO metadata_actions (service, action)
                            SELECT unnest($1::text[]), unnest($2::text[])"#,
                    )
                    .await?;

                let services = actions
                    .iter()
                    .map(|_| service.clone())
                    .collect::<Vec<String>>();
                let actions = actions
                    .iter()
                    .map(|a| a.name.clone())
                    .collect::<Vec<String>>();

                client.execute(&stmt, &[&services, &actions]).await?;
                Ok::<(), StorageErr>(())
            })
            .await??;

        Ok(())
    }

    #[tracing::instrument(level = "info", skip(self), fields(otel.kind = "client", db.name = self.db_name))]
    async fn set_service_action(&self, service: &str, action: &str) -> Result<Action, StorageErr> {
        let service = service.to_string();
        let action = action.to_string();
        let pool = self.pool.clone();

        let result = self
            .runtime
            .spawn(async move {
                let client = pool.get().await?;
                let stmt = client
                    .prepare_cached(
                        r#"INSERT INTO metadata_actions (service, action) 
                            VALUES ($1, $2) 
                            ON CONFLICT (service, action) DO NOTHING"#,
                    )
                    .await?;

                client.execute(&stmt, &[&service, &action]).await?;

                let action = Action {
                    service,
                    name: action,
                };

                Ok::<Action, StorageErr>(action)
            })
            .await??;

        Ok(result)
    }

    #[tracing::instrument(level = "info", skip(self), fields(otel.kind = "client", db.name = self.db_name))]
    async fn delete_service_action(&self, service: &str, action: &str) -> Result<(), StorageErr> {
        let service = service.to_string();
        let action = action.to_string();
        let pool = self.pool.clone();

        let result = self
            .runtime
            .spawn(async move {
                let client = pool.get().await?;
                let stmt = client
                    .prepare_cached(
                        r#"DELETE FROM metadata_actions WHERE service = $1 AND action = $2"#,
                    )
                    .await?;

                client.execute(&stmt, &[&service, &action]).await?;
                Ok::<(), StorageErr>(())
            })
            .await??;

        Ok(result)
    }

    #[tracing::instrument(level = "info", skip(self), fields(otel.kind = "client", db.name = self.db_name))]
    async fn get_service_resource_types(
        &self,
        service: &str,
    ) -> Result<Vec<ResourceTypeMeta>, StorageErr> {
        let service = service.to_string();
        let pool = self.pool.clone();

        let result = self
            .runtime
            .spawn(async move {
                let client = pool.get().await?;
                let stmt = client
                    .prepare_cached(r#"SELECT type, evaluation_priority, parent_type FROM metadata_resource_types WHERE service = $1"#)
                    .await?;


                let results = client.query(&stmt, &[&service]).await?;
                let mut resource_types: Vec<ResourceTypeMeta> = Vec::with_capacity(results.len());
                for row in results.iter() {
                    let kind = row.get(0);
                    let evaluation_priority: EvaluationPriority = row.get::<usize, &str>(1).try_into()?;
                    let parent_type: Option<String> = row.get(2);
                    resource_types.push(ResourceTypeMeta {
                        service: service.clone(),
                        kind,
                        evaluation_priority,
                        parent_type,
                    });
                }
                Ok::<Vec<ResourceTypeMeta>, StorageErr>(resource_types)
            })
            .await??;

        Ok(result)
    }

    #[tracing::instrument(level = "info", skip(self), fields(otel.kind = "client", db.name = self.db_name))]
    async fn get_resource_type(
        &self,
        service: &str,
        kind: &str,
    ) -> Result<ResourceTypeMeta, StorageErr> {
        let action = Action {
            name: String::new(),
            service: service.to_string(),
        };
        let resource = Resource {
            id: String::new(),
            kind: kind.to_string(),
            data: None,
        };
        let meta = self.get_meta(&action, Some(&resource)).await;
        Ok(meta?.resource)
    }

    #[tracing::instrument(level = "info", skip(self), fields(otel.kind = "client", db.name = self.db_name))]
    async fn get_resource_type_by_kind(
        &self,
        kind: &str,
    ) -> Result<Option<ResourceTypeMeta>, StorageErr> {
        let kind = kind.to_string();
        let pool = self.pool.clone();

        let result = self
            .runtime
            .spawn(async move {
                let client = pool.get().await?;
                let stmt = client
                    .prepare_cached(
                        r#"SELECT service, evaluation_priority, parent_type
                           FROM metadata_resource_types
                           WHERE type = $1
                           LIMIT 1"#,
                    )
                    .await?;

                let row = client.query_opt(&stmt, &[&kind]).await?;
                let resource_type = match row {
                    Some(row) => {
                        let evaluation_priority: EvaluationPriority =
                            row.get::<usize, &str>(1).try_into()?;
                        Some(ResourceTypeMeta {
                            service: row.get(0),
                            kind: kind.clone(),
                            evaluation_priority,
                            parent_type: row.get(2),
                        })
                    }
                    None => None,
                };

                Ok::<Option<ResourceTypeMeta>, StorageErr>(resource_type)
            })
            .await??;

        Ok(result)
    }

    #[tracing::instrument(level = "info", skip(self), fields(otel.kind = "client", db.name = self.db_name))]
    async fn replace_resource_types(
        &self,
        service: &str,
        resource_types: Vec<ResourceTypeMeta>,
    ) -> Result<(), StorageErr> {
        let service = service.to_string();
        let pool = self.pool.clone();

        self.runtime
            .spawn(async move {
                let mut client = pool.get().await?;
                let tx = client.transaction().await?;

                let delete_stmt = tx
                    .prepare_cached(r#"DELETE FROM metadata_resource_types WHERE service = $1"#)
                    .await?;

                tx.execute(&delete_stmt, &[&service]).await?;

                let insert_stmt = tx
                    .prepare_cached(
                        r#"
                            INSERT INTO metadata_resource_types (service, type, evaluation_priority, parent_type) 
                            SELECT unnest($1::text[]), unnest($2::text[]), unnest($3::text[]), unnest($4::text[])
                        "#,
                    )
                    .await?;

                let services = resource_types.iter().map(|_| service.clone()).collect::<Vec<String>>();
                let types = resource_types.iter().map(|r| r.kind.clone()).collect::<Vec<String>>();
                let evaluation_priorities = resource_types.iter().map(|r| r.evaluation_priority.to_string()).collect::<Vec<String>>();
                let parent_types = resource_types.iter().map(|r| r.parent_type.clone()).collect::<Vec<Option<String>>>();

                tx.execute(&insert_stmt, &[&services, &types, &evaluation_priorities, &parent_types])
                    .await
                    .map_err(map_resource_type_conflict)?;
                tx.commit().await?;
                Ok::<(), StorageErr>(())
            })
            .await??;

        Ok(())
    }

    #[tracing::instrument(level = "info", skip(self), fields(otel.kind = "client", db.name = self.db_name))]
    async fn set_resource_type_meta(&self, resource: &ResourceTypeMeta) -> Result<(), StorageErr> {
        let service = resource.service.clone();
        let kind = resource.kind.clone();
        let evaluation_priority = resource.evaluation_priority.to_string();
        let parent_type = resource.parent_type.clone();
        let pool = self.pool.clone();

        let result = self
            .runtime
            .spawn(async move {
                let client = pool.get().await?;
                let stmt = client
                    .prepare_cached(
                        r#"
                            INSERT INTO metadata_resource_types (service, type, evaluation_priority, parent_type)
                            VALUES ($1, $2, $3, $4)
                            ON CONFLICT (service, type) DO UPDATE
                                SET evaluation_priority = $3, parent_type = $4;
                        "#,
                    )
                    .await?;

                client
                    .execute(&stmt, &[&service, &kind, &evaluation_priority, &parent_type])
                    .await
                    .map_err(map_resource_type_conflict)?;

                Ok::<(), StorageErr>(())
            })
            .await??;

        Ok(result)
    }

    #[tracing::instrument(level = "info", skip(self), fields(otel.kind = "client", db.name = self.db_name))]
    async fn delete_resource_type_meta(&self, service: &str, kind: &str) -> Result<(), StorageErr> {
        let service = service.to_string();
        let kind = kind.to_string();
        let pool = self.pool.clone();

        let result = self
            .runtime
            .spawn(async move {
                let client = pool.get().await?;
                let stmt = client
                    .prepare_cached(
                        r#"
                            DELETE FROM metadata_resource_types
                            WHERE service = $1 AND type = $2;
                        "#,
                    )
                    .await?;

                client.execute(&stmt, &[&service, &kind]).await?;

                Ok::<(), StorageErr>(())
            })
            .await??;

        Ok(result)
    }
}
