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

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use arc_swap::ArcSwap;
use async_trait::async_trait;
use deadpool_postgres::Pool;
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::models::{Group, User};
use crate::provider::MembershipChange;

use super::{SnapshotStore, SnapshotStoreError, StoredSnapshot};

/// Maximum number of rows included in a single batched INSERT or DELETE statement.
const BATCH_SIZE: usize = 500;

/// PostgreSQL advisory lock ID used to serialize full syncs across replicas.
const FULL_SYNC_LOCK_ID: i64 = 1;

fn format_error_chain(error: &dyn std::error::Error) -> String {
    let mut msg = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        msg.push_str(": ");
        msg.push_str(&cause.to_string());
        source = cause.source();
    }
    msg
}

impl From<tokio_postgres::Error> for SnapshotStoreError {
    fn from(e: tokio_postgres::Error) -> Self {
        Self {
            msg: format_error_chain(&e),
        }
    }
}

impl From<deadpool_postgres::PoolError> for SnapshotStoreError {
    fn from(e: deadpool_postgres::PoolError) -> Self {
        Self {
            msg: format_error_chain(&e),
        }
    }
}

/// Snapshot store backed by four normalized PostgreSQL tables.
pub struct PgSnapshotStore {
    /// Held behind an `ArcSwap` so the token refresher can atomically replace
    /// the pool when a short-lived password rotates.
    pool: Arc<ArcSwap<Pool>>,
    users_enabled: bool,
    groups_enabled: bool,
    memberships_enabled: bool,
    /// Whether enterprise-application app-role assignment scoping is active.
    /// Persisted in `sync_state` so a change to this flag (a mode switch)
    /// invalidates the stored snapshot and forces a full re-sync.
    app_role_assignments_enabled: bool,
    /// Dedicated connection holding the session-level advisory lock during a
    /// full sync.  Kept alive (out of the pool) for the entire duration so
    /// that the lock is not released prematurely.  If the process crashes the
    /// connection closes and PostgreSQL auto-releases the lock.
    lock_conn: Mutex<Option<deadpool_postgres::Object>>,
}

impl PgSnapshotStore {
    pub fn new(
        pool: Arc<ArcSwap<Pool>>,
        users_enabled: bool,
        groups_enabled: bool,
        memberships_enabled: bool,
        app_role_assignments_enabled: bool,
    ) -> Self {
        let pool_ref = pool.clone();
        let meter = opentelemetry::global::meter("microsoft-graph-userinfo-service");
        meter
            .u64_observable_gauge("snapshot.postgres.pool.active_connections")
            .with_description("Number of active connections in the database pool")
            .with_callback(move |gauge| {
                let status = pool_ref.load().status();
                let active = status.size.saturating_sub(status.available);
                gauge.observe(active as u64, &[]);
            })
            .build();

        Self {
            pool,
            users_enabled,
            groups_enabled,
            memberships_enabled,
            app_role_assignments_enabled,
            lock_conn: Mutex::new(None),
        }
    }

    /// Load the current pool. A connection obtained from the returned pool stays
    /// valid even if the pool is swapped afterwards.
    fn pool(&self) -> arc_swap::Guard<Arc<Pool>> {
        self.pool.load()
    }

    pub async fn initialize(&self) -> Result<(), SnapshotStoreError> {
        let client = self.pool().get().await?;

        // Create tables on a fresh deployment. The IF NOT EXISTS guard makes
        // all of these no-ops when upgrading an existing deployment.
        client
            .batch_execute(
                r#"
            CREATE TABLE IF NOT EXISTS users (
                id           TEXT PRIMARY KEY,
                display_name TEXT,
                email        TEXT,
                username     TEXT,
                given_name   TEXT,
                family_name  TEXT,
                active       BOOLEAN,
                job_title    TEXT
            );

            CREATE TABLE IF NOT EXISTS groups (
                id           TEXT PRIMARY KEY,
                display_name TEXT,
                description  TEXT
            );

            CREATE TABLE IF NOT EXISTS group_members (
                group_id TEXT NOT NULL,
                user_id  TEXT NOT NULL,
                PRIMARY KEY (group_id, user_id)
            );

            CREATE INDEX IF NOT EXISTS group_members_user_id_idx ON group_members (user_id);

            CREATE TABLE IF NOT EXISTS sync_state (
                id                        INTEGER PRIMARY KEY DEFAULT 1 CHECK (id = 1),
                users_delta_token         TEXT,
                groups_delta_token        TEXT,
                full_sync_complete        BOOLEAN NOT NULL DEFAULT false,
                users_cache_enabled       BOOLEAN NOT NULL DEFAULT false,
                groups_cache_enabled      BOOLEAN NOT NULL DEFAULT false,
                memberships_cache_enabled BOOLEAN NOT NULL DEFAULT false,
                users_next_link           TEXT,
                groups_next_link          TEXT,
                updated_at                TIMESTAMPTZ NOT NULL DEFAULT now()
            );

            ALTER TABLE sync_state
                ADD COLUMN IF NOT EXISTS app_role_assignments_enabled BOOLEAN NOT NULL DEFAULT false;

            CREATE TABLE IF NOT EXISTS app_role_assignment_service_principals (
                id           TEXT PRIMARY KEY,
                display_name TEXT,
                email        TEXT,
                username     TEXT,
                given_name   TEXT,
                family_name  TEXT,
                active       BOOLEAN,
                job_title    TEXT
            );

            ALTER TABLE group_members SET (
                autovacuum_vacuum_scale_factor  = 0.01,
                autovacuum_analyze_scale_factor = 0.005
            );

            ALTER TABLE users SET (
                autovacuum_vacuum_scale_factor  = 0.05,
                autovacuum_analyze_scale_factor = 0.02
            );
        "#,
            )
            .await?;

        info!("snapshot store initialized");
        Ok(())
    }
}

#[async_trait]
impl SnapshotStore for PgSnapshotStore {
    async fn load(&self) -> Result<Option<StoredSnapshot>, SnapshotStoreError> {
        let mut client = self.pool().get().await?;

        // Read all four tables under one repeatable-read snapshot so a concurrent
        // writer commit (including a full-sync TRUNCATE) cannot tear the result
        // across statements. Read-only lets PostgreSQL skip serialization tracking.
        let tx = client
            .build_transaction()
            .isolation_level(tokio_postgres::IsolationLevel::RepeatableRead)
            .read_only(true)
            .start()
            .await?;

        let state_row = tx
            .query_opt(
                "SELECT users_delta_token, groups_delta_token, full_sync_complete, \
                        users_cache_enabled, groups_cache_enabled, \
                        memberships_cache_enabled, users_next_link, groups_next_link, \
                        app_role_assignments_enabled \
                 FROM sync_state WHERE id = 1",
                &[],
            )
            .await?;

        let Some(row) = state_row else {
            return Ok(None);
        };

        let stored_users_enabled: bool = row.get(3);
        let stored_groups_enabled: bool = row.get(4);
        let stored_memberships_enabled: bool = row.get(5);
        let stored_app_role_assignments_enabled: bool = row.get(8);

        if stored_users_enabled != self.users_enabled
            || stored_groups_enabled != self.groups_enabled
            || stored_memberships_enabled != self.memberships_enabled
            || stored_app_role_assignments_enabled != self.app_role_assignments_enabled
        {
            warn!(
                stored_users_enabled,
                stored_groups_enabled,
                stored_memberships_enabled,
                stored_app_role_assignments_enabled,
                current_users_enabled = self.users_enabled,
                current_groups_enabled = self.groups_enabled,
                current_memberships_enabled = self.memberships_enabled,
                current_app_role_assignments_enabled = self.app_role_assignments_enabled,
                "cache flags changed; discarding snapshot and performing full sync"
            );
            return Ok(None);
        }

        let full_sync_complete: bool = row.get(2);
        let users_delta_token: Option<String> = row.get(0);
        let groups_delta_token: Option<String> = row.get(1);
        let users_next_link: Option<String> = row.get(6);
        let groups_next_link: Option<String> = row.get(7);

        // begin_full_sync clears both delta tokens and both next-links.
        // This makes the combination of users_next_link and users_delta_token
        // sufficient to determine the resume state without any extra flag:
        //   users_next_link=Some → mid-users phase, resume from that URL
        //   users_next_link=None, users_delta_token=Some → users phase complete
        //   users_next_link=None, users_delta_token=None → nothing fetched yet
        // Return None only when there is nothing to resume.
        if !full_sync_complete
            && users_next_link.is_none()
            && groups_next_link.is_none()
            && users_delta_token.is_none()
        {
            return Ok(None);
        }

        let user_rows = tx
            .query(
                "SELECT id, display_name, email, username, given_name, family_name, \
                        active, job_title FROM users",
                &[],
            )
            .await?;

        let mut users: Vec<User> = user_rows
            .into_iter()
            .map(|r| User {
                id: r.get(0),
                display_name: r.get(1),
                email: r.get(2),
                username: r.get(3),
                given_name: r.get(4),
                family_name: r.get(5),
                active: r.get(6),
                job_title: r.get(7),
            })
            .collect();

        // Merge service principals surfaced as users. They live in a dedicated
        // table so the /users/delta reconciliation sweep never deletes them.
        // Real synced users take precedence on the (disjoint in practice) id
        // space, so skip any service principal whose id already exists.
        let sp_rows = tx
            .query(
                "SELECT id, display_name, email, username, given_name, family_name, \
                        active, job_title FROM app_role_assignment_service_principals",
                &[],
            )
            .await?;
        if !sp_rows.is_empty() {
            let existing_ids: HashSet<String> = users.iter().map(|u| u.id.clone()).collect();
            for r in sp_rows {
                let id: String = r.get(0);
                if existing_ids.contains(&id) {
                    warn!(
                        service_principal_id = %id,
                        "service principal id collides with a synced user; keeping the user"
                    );
                    continue;
                }
                users.push(User {
                    id,
                    display_name: r.get(1),
                    email: r.get(2),
                    username: r.get(3),
                    given_name: r.get(4),
                    family_name: r.get(5),
                    active: r.get(6),
                    job_title: r.get(7),
                });
            }
        }

        let group_rows = tx
            .query("SELECT id, display_name, description FROM groups", &[])
            .await?;

        let groups: Vec<Group> = group_rows
            .into_iter()
            .map(|r| Group {
                id: r.get(0),
                display_name: r.get(1),
                description: r.get(2),
            })
            .collect();

        let member_rows = tx
            .query("SELECT group_id, user_id FROM group_members", &[])
            .await?;

        let mut membership: HashMap<String, Vec<String>> = HashMap::new();
        for r in member_rows {
            let group_id: String = r.get(0);
            let user_id: String = r.get(1);
            membership.entry(group_id).or_default().push(user_id);
        }

        tx.commit().await?;

        Ok(Some(StoredSnapshot {
            users,
            groups,
            membership,
            users_delta_token,
            groups_delta_token,
            users_cache_enabled: stored_users_enabled,
            groups_cache_enabled: stored_groups_enabled,
            memberships_cache_enabled: stored_memberships_enabled,
            full_sync_complete,
            users_next_link,
            groups_next_link,
        }))
    }

    async fn begin_full_sync(&self) -> Result<(), SnapshotStoreError> {
        let client = self.pool().get().await?;
        client
            .execute(
                "TRUNCATE app_role_assignment_service_principals, \
                          group_members, groups, users RESTART IDENTITY",
                &[],
            )
            .await?;
        client
            .execute(
                "INSERT INTO sync_state \
                     (id, full_sync_complete, \
                      users_delta_token, groups_delta_token, \
                      users_cache_enabled, groups_cache_enabled, memberships_cache_enabled, \
                      app_role_assignments_enabled, \
                      users_next_link, groups_next_link, updated_at) \
                 VALUES (1, false, NULL, NULL, $1, $2, $3, $4, NULL, NULL, now()) \
                 ON CONFLICT (id) DO UPDATE SET \
                     full_sync_complete           = false, \
                     users_delta_token            = NULL, \
                     groups_delta_token           = NULL, \
                     users_cache_enabled          = EXCLUDED.users_cache_enabled, \
                     groups_cache_enabled         = EXCLUDED.groups_cache_enabled, \
                     memberships_cache_enabled    = EXCLUDED.memberships_cache_enabled, \
                     app_role_assignments_enabled = EXCLUDED.app_role_assignments_enabled, \
                     users_next_link              = NULL, \
                     groups_next_link             = NULL, \
                     updated_at                   = now()",
                &[
                    &self.users_enabled,
                    &self.groups_enabled,
                    &self.memberships_enabled,
                    &self.app_role_assignments_enabled,
                ],
            )
            .await?;
        Ok(())
    }

    async fn apply_user_changes(
        &self,
        upserts: &[User],
        removes: &[String],
    ) -> Result<(), SnapshotStoreError> {
        if upserts.is_empty() && removes.is_empty() {
            return Ok(());
        }
        let client = self.pool().get().await?;

        for chunk in upserts.chunks(BATCH_SIZE) {
            if chunk.is_empty() {
                continue;
            }
            let mut query = String::from(
                "INSERT INTO users \
                 (id, display_name, email, username, given_name, family_name, active, job_title) \
                 VALUES ",
            );
            let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = Vec::new();
            for (i, u) in chunk.iter().enumerate() {
                if i > 0 {
                    query.push(',');
                }
                let base = i * 8 + 1;
                query.push_str(&format!(
                    "(${},${},${},${},${},${},${},${})",
                    base,
                    base + 1,
                    base + 2,
                    base + 3,
                    base + 4,
                    base + 5,
                    base + 6,
                    base + 7
                ));
                params.push(&u.id);
                params.push(&u.display_name);
                params.push(&u.email);
                params.push(&u.username);
                params.push(&u.given_name);
                params.push(&u.family_name);
                params.push(&u.active);
                params.push(&u.job_title);
            }
            query.push_str(
                " ON CONFLICT (id) DO UPDATE SET \
                 display_name = EXCLUDED.display_name, \
                 email = EXCLUDED.email, \
                 username = EXCLUDED.username, \
                 given_name = EXCLUDED.given_name, \
                 family_name = EXCLUDED.family_name, \
                 active = EXCLUDED.active, \
                 job_title = EXCLUDED.job_title",
            );
            client.execute(query.as_str(), &params).await?;
        }

        for chunk in removes.chunks(BATCH_SIZE) {
            let ids: Vec<&str> = chunk.iter().map(String::as_str).collect();
            client
                .execute("DELETE FROM users WHERE id = ANY($1::text[])", &[&ids])
                .await?;
        }

        Ok(())
    }

    async fn save_users_next_link(&self, next_link: &str) -> Result<(), SnapshotStoreError> {
        let client = self.pool().get().await?;
        client
            .execute(
                "UPDATE sync_state SET users_next_link = $1 WHERE id = 1",
                &[&next_link],
            )
            .await?;
        Ok(())
    }

    async fn commit_users_phase(&self, delta_token: &str) -> Result<(), SnapshotStoreError> {
        let client = self.pool().get().await?;
        client
            .execute(
                "UPDATE sync_state SET \
                     users_next_link   = NULL, \
                     users_delta_token = $1 \
                 WHERE id = 1",
                &[&delta_token],
            )
            .await?;
        Ok(())
    }

    async fn apply_group_changes(
        &self,
        upserts: &[Group],
        removes: &[String],
        membership_changes: &[MembershipChange],
    ) -> Result<(), SnapshotStoreError> {
        if upserts.is_empty() && removes.is_empty() && membership_changes.is_empty() {
            return Ok(());
        }
        let client = self.pool().get().await?;

        for chunk in upserts.chunks(BATCH_SIZE) {
            if chunk.is_empty() {
                continue;
            }
            let mut query =
                String::from("INSERT INTO groups (id, display_name, description) VALUES ");
            let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = Vec::new();
            for (i, g) in chunk.iter().enumerate() {
                if i > 0 {
                    query.push(',');
                }
                let base = i * 3 + 1;
                query.push_str(&format!("(${},${},${})", base, base + 1, base + 2));
                params.push(&g.id);
                params.push(&g.display_name);
                params.push(&g.description);
            }
            query.push_str(
                " ON CONFLICT (id) DO UPDATE SET \
                 display_name = EXCLUDED.display_name, \
                 description = EXCLUDED.description",
            );
            client.execute(query.as_str(), &params).await?;
        }

        for chunk in removes.chunks(BATCH_SIZE) {
            let ids: Vec<&str> = chunk.iter().map(String::as_str).collect();
            client
                .execute(
                    "DELETE FROM group_members WHERE group_id = ANY($1::text[])",
                    &[&ids],
                )
                .await?;
            client
                .execute("DELETE FROM groups WHERE id = ANY($1::text[])", &[&ids])
                .await?;
        }

        if self.memberships_enabled {
            for change in membership_changes {
                if !change.added_user_ids.is_empty() {
                    for chunk in change.added_user_ids.chunks(BATCH_SIZE) {
                        let mut query =
                            String::from("INSERT INTO group_members (group_id, user_id) VALUES ");
                        let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
                            Vec::new();
                        params.push(&change.group_id);
                        for (i, user_id) in chunk.iter().enumerate() {
                            if i > 0 {
                                query.push(',');
                            }
                            query.push_str(&format!("($1,${})", i + 2));
                            params.push(user_id);
                        }
                        query.push_str(" ON CONFLICT DO NOTHING");
                        client.execute(query.as_str(), &params).await?;
                    }
                }

                if !change.removed_user_ids.is_empty() {
                    for chunk in change.removed_user_ids.chunks(BATCH_SIZE) {
                        let ids: Vec<&str> = chunk.iter().map(String::as_str).collect();
                        client
                            .execute(
                                "DELETE FROM group_members \
                                 WHERE group_id = $1 AND user_id = ANY($2::text[])",
                                &[&change.group_id, &ids],
                            )
                            .await?;
                    }
                }
            }
        }

        Ok(())
    }

    async fn save_groups_next_link(&self, next_link: &str) -> Result<(), SnapshotStoreError> {
        let client = self.pool().get().await?;
        client
            .execute(
                "UPDATE sync_state SET groups_next_link = $1 WHERE id = 1",
                &[&next_link],
            )
            .await?;
        Ok(())
    }

    async fn commit_sync(
        &self,
        users_delta_token: Option<&str>,
        groups_delta_token: Option<&str>,
        is_full_sync: bool,
    ) -> Result<(), SnapshotStoreError> {
        let client = self.pool().get().await?;

        if is_full_sync {
            client
                .execute(
                    "INSERT INTO sync_state \
                         (id, users_delta_token, groups_delta_token, full_sync_complete, \
                          users_cache_enabled, groups_cache_enabled, memberships_cache_enabled, \
                          app_role_assignments_enabled, \
                          users_next_link, groups_next_link, updated_at) \
                     VALUES (1, $1, $2, true, $3, $4, $5, $6, NULL, NULL, now()) \
                     ON CONFLICT (id) DO UPDATE SET \
                         users_delta_token            = EXCLUDED.users_delta_token, \
                         groups_delta_token           = EXCLUDED.groups_delta_token, \
                         full_sync_complete           = true, \
                         users_cache_enabled          = EXCLUDED.users_cache_enabled, \
                         groups_cache_enabled         = EXCLUDED.groups_cache_enabled, \
                         memberships_cache_enabled    = EXCLUDED.memberships_cache_enabled, \
                         app_role_assignments_enabled = EXCLUDED.app_role_assignments_enabled, \
                         users_next_link              = NULL, \
                         groups_next_link             = NULL, \
                         updated_at                   = EXCLUDED.updated_at",
                    &[
                        &users_delta_token,
                        &groups_delta_token,
                        &self.users_enabled,
                        &self.groups_enabled,
                        &self.memberships_enabled,
                        &self.app_role_assignments_enabled,
                    ],
                )
                .await?;
        } else {
            let rows = client
                .execute(
                    "UPDATE sync_state SET \
                         users_delta_token            = $1, \
                         groups_delta_token           = $2, \
                         users_cache_enabled          = $3, \
                         groups_cache_enabled         = $4, \
                         memberships_cache_enabled    = $5, \
                         app_role_assignments_enabled = $6, \
                         users_next_link              = NULL, \
                         groups_next_link             = NULL, \
                         updated_at                   = now() \
                     WHERE id = 1 AND full_sync_complete = true",
                    &[
                        &users_delta_token,
                        &groups_delta_token,
                        &self.users_enabled,
                        &self.groups_enabled,
                        &self.memberships_enabled,
                        &self.app_role_assignments_enabled,
                    ],
                )
                .await?;
            if rows == 0 {
                warn!("incremental commit_sync skipped: full sync in progress on another replica");
            }
        }

        Ok(())
    }

    async fn replace_scoped_groups(
        &self,
        groups: &[Group],
        members_by_group: &HashMap<String, HashSet<String>>,
    ) -> Result<(), SnapshotStoreError> {
        let mut client = self.pool().get().await?;
        let tx = client.transaction().await?;

        // Full replacement so removed assignments are dropped and renames /
        // additions applied. Users are deliberately left untouched.
        tx.execute("DELETE FROM group_members", &[]).await?;
        tx.execute("DELETE FROM groups", &[]).await?;

        for chunk in groups.chunks(BATCH_SIZE) {
            if chunk.is_empty() {
                continue;
            }
            let mut query =
                String::from("INSERT INTO groups (id, display_name, description) VALUES ");
            let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = Vec::new();
            for (i, g) in chunk.iter().enumerate() {
                if i > 0 {
                    query.push(',');
                }
                let base = i * 3 + 1;
                query.push_str(&format!("(${},${},${})", base, base + 1, base + 2));
                params.push(&g.id);
                params.push(&g.display_name);
                params.push(&g.description);
            }
            tx.execute(query.as_str(), &params).await?;
        }

        if self.memberships_enabled {
            for (group_id, user_ids) in members_by_group {
                let user_ids: Vec<&String> = user_ids.iter().collect();
                for chunk in user_ids.chunks(BATCH_SIZE) {
                    if chunk.is_empty() {
                        continue;
                    }
                    let mut query =
                        String::from("INSERT INTO group_members (group_id, user_id) VALUES ");
                    let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = Vec::new();
                    params.push(group_id);
                    for (i, user_id) in chunk.iter().enumerate() {
                        if i > 0 {
                            query.push(',');
                        }
                        query.push_str(&format!("($1,${})", i + 2));
                        params.push(*user_id);
                    }
                    query.push_str(" ON CONFLICT DO NOTHING");
                    tx.execute(query.as_str(), &params).await?;
                }
            }
        }

        tx.commit().await?;
        Ok(())
    }

    async fn replace_service_principals(
        &self,
        service_principals: &[User],
    ) -> Result<(), SnapshotStoreError> {
        let mut client = self.pool().get().await?;
        let tx = client.transaction().await?;

        tx.execute("DELETE FROM app_role_assignment_service_principals", &[])
            .await?;

        for chunk in service_principals.chunks(BATCH_SIZE) {
            if chunk.is_empty() {
                continue;
            }
            let mut query = String::from(
                "INSERT INTO app_role_assignment_service_principals \
                 (id, display_name, email, username, given_name, family_name, active, job_title) \
                 VALUES ",
            );
            let mut params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = Vec::new();
            for (i, u) in chunk.iter().enumerate() {
                if i > 0 {
                    query.push(',');
                }
                let base = i * 8 + 1;
                query.push_str(&format!(
                    "(${},${},${},${},${},${},${},${})",
                    base,
                    base + 1,
                    base + 2,
                    base + 3,
                    base + 4,
                    base + 5,
                    base + 6,
                    base + 7
                ));
                params.push(&u.id);
                params.push(&u.display_name);
                params.push(&u.email);
                params.push(&u.username);
                params.push(&u.given_name);
                params.push(&u.family_name);
                params.push(&u.active);
                params.push(&u.job_title);
            }
            tx.execute(query.as_str(), &params).await?;
        }

        tx.commit().await?;
        Ok(())
    }

    async fn try_acquire_full_sync_lock(&self) -> Result<bool, SnapshotStoreError> {
        let mut guard = self.lock_conn.lock().await;
        if guard.is_some() {
            return Ok(true);
        }

        let conn = self.pool().get().await?;
        let row = conn
            .query_one("SELECT pg_try_advisory_lock($1)", &[&FULL_SYNC_LOCK_ID])
            .await?;
        let acquired: bool = row.get(0);

        if acquired {
            info!("acquired full-sync advisory lock");
            *guard = Some(conn);
        }

        Ok(acquired)
    }

    async fn release_full_sync_lock(&self) -> Result<(), SnapshotStoreError> {
        let mut guard = self.lock_conn.lock().await;
        if let Some(conn) = guard.take() {
            conn.execute("SELECT pg_advisory_unlock($1)", &[&FULL_SYNC_LOCK_ID])
                .await?;
            info!("released full-sync advisory lock");
        }
        Ok(())
    }
}
