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

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use tokio::sync::RwLock;
use tracing::{Instrument, error, info, warn};

use crate::metrics;
use crate::models::{
    DisplayNameFilter, Group, GroupQuery, Page, User, UserQuery, parse_display_name_filter,
};
use crate::provider::{
    AppRoleAssignmentsSource, DeltaChange, DeltaSyncSource, IdentityProvider, MembershipChange,
    PageOutcome, ProviderError, ScopedDirectory,
};
use crate::snapshot::{SnapshotStore, StoredSnapshot};

const MIN_RELOAD_INTERVAL: Duration = Duration::from_millis(100);

pub struct CacheConfig {
    pub sync_interval: Duration,
    pub sync_jitter_max: Duration,
    pub users_enabled: bool,
    pub groups_enabled: bool,
    pub memberships_enabled: bool,
}

struct DirectorySnapshot {
    /// Users indexed by id, in id order.
    users: BTreeMap<String, User>,
    /// Groups indexed by id, in id order.
    groups: BTreeMap<String, Group>,
    /// Direct members of each group (group_id → set of user_ids).
    members_by_group: HashMap<String, HashSet<String>>,
    /// Direct group memberships of each user (user_id → set of group_ids).
    groups_by_user: HashMap<String, HashSet<String>>,
}

impl DirectorySnapshot {
    /// Build a snapshot from raw Vec inputs (e.g. rows loaded from Postgres).
    fn new(
        users: Vec<User>,
        groups: Vec<Group>,
        members_by_group: HashMap<String, HashSet<String>>,
    ) -> Self {
        Self::from_maps(
            users.into_iter().map(|u| (u.id.clone(), u)).collect(),
            groups.into_iter().map(|g| (g.id.clone(), g)).collect(),
            members_by_group,
        )
    }

    /// Build a snapshot from already-indexed maps (e.g. produced by sync_once).
    fn from_maps(
        users: BTreeMap<String, User>,
        groups: BTreeMap<String, Group>,
        members_by_group: HashMap<String, HashSet<String>>,
    ) -> Self {
        let groups_by_user = build_groups_by_user(&members_by_group);
        Self {
            users,
            groups,
            members_by_group,
            groups_by_user,
        }
    }
}

fn build_groups_by_user(
    members_by_group: &HashMap<String, HashSet<String>>,
) -> HashMap<String, HashSet<String>> {
    let mut index: HashMap<String, HashSet<String>> = HashMap::new();
    for (group_id, user_ids) in members_by_group {
        for user_id in user_ids {
            index
                .entry(user_id.clone())
                .or_default()
                .insert(group_id.clone());
        }
    }
    index
}

pub struct CachedProvider {
    delta_source: Arc<dyn DeltaSyncSource>,
    inner: Arc<dyn IdentityProvider>,
    snapshot: Arc<RwLock<Option<DirectorySnapshot>>>,
    snapshot_store: Option<Arc<dyn SnapshotStore>>,
    config: CacheConfig,
    /// Enterprise-application app-role assignment scope source. When set, the
    /// writer restricts synced groups/memberships to the assigned groups and
    /// surfaces assigned service principals as users instead of running the
    /// global `/groups/delta` firehose. `None` disables scoping.
    app_role_assignments: Option<Arc<dyn AppRoleAssignmentsSource>>,
    ready: Arc<AtomicBool>,
    /// Whether the in-memory snapshot came from a completed full sync. Used by
    /// readers to avoid downgrading complete data to a partial reload.
    snapshot_complete: Arc<AtomicBool>,
}

struct SyncOutcome {
    user_count: usize,
    group_count: usize,
    membership_count: usize,
    changed: bool,
}

/// State loaded from Postgres when a previous full sync was interrupted.
/// Passed to the first `sync_once` call after startup to resume where it left off.
struct ResumeState {
    /// `@odata.nextLink` URL for resuming the users phase.
    /// `None` means the users phase had already completed before the crash;
    /// the users rows in Postgres are the full current set and that phase
    /// can be skipped.
    users_next_link: Option<String>,
    /// `@odata.nextLink` URL for resuming the groups phase, or `None` if the
    /// groups phase had not started yet.
    groups_next_link: Option<String>,
    /// The users delta token saved by `commit_users_phase`. Present when
    /// `users_next_link` is `None` so `commit_sync` uses the correct token
    /// instead of the one from the previous complete cycle.
    users_delta_token: Option<String>,
}

#[derive(Default)]
struct InitialSyncState {
    users_delta_token: Option<String>,
    groups_delta_token: Option<String>,
    resume: Option<ResumeState>,
    snapshot_loaded: bool,
}

/// Result of loading a startup snapshot from the store.
enum StartupOutcome {
    /// A complete snapshot was found; apply it and mark ready.
    Ready(StoredSnapshot),
    /// An incomplete snapshot was found and the lock was acquired; resume.
    Resume(StoredSnapshot),
    /// No usable snapshot (empty or load error); proceed with a fresh full sync.
    FullSync,
    /// Another replica holds the lock; wait and retry.
    WaitForPeer,
}

impl CachedProvider {
    pub fn new(
        delta_source: Arc<dyn DeltaSyncSource>,
        inner: Arc<dyn IdentityProvider>,
        snapshot_store: Option<Arc<dyn SnapshotStore>>,
        config: CacheConfig,
        app_role_assignments: Option<Arc<dyn AppRoleAssignmentsSource>>,
    ) -> Self {
        Self {
            delta_source,
            inner,
            snapshot: Arc::new(RwLock::new(None)),
            snapshot_store,
            config,
            app_role_assignments,
            ready: Arc::new(AtomicBool::new(false)),
            snapshot_complete: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn ready(&self) -> Arc<AtomicBool> {
        self.ready.clone()
    }

    async fn apply_stored_snapshot(
        &self,
        stored: StoredSnapshot,
    ) -> (Option<String>, Option<String>) {
        let delta_tokens = (stored.users_delta_token, stored.groups_delta_token);
        let membership = stored
            .membership
            .into_iter()
            .map(|(g, us)| (g, us.into_iter().collect()))
            .collect();
        let snapshot = DirectorySnapshot::new(stored.users, stored.groups, membership);
        *self.snapshot.write().await = Some(snapshot);
        delta_tokens
    }

    /// Load a snapshot from the store and decide how startup should proceed.
    async fn load_startup_snapshot(&self, store: &Arc<dyn SnapshotStore>) -> StartupOutcome {
        let load_start = Instant::now();
        let load_result = store.load().await;
        let load_outcome = if load_result.is_ok() {
            "success"
        } else {
            "error"
        };
        metrics::METRICS.postgres_load_duration.record(
            load_start.elapsed().as_secs_f64(),
            &[opentelemetry::KeyValue::new("outcome", load_outcome)],
        );

        let stored = match load_result {
            Ok(Some(s)) => s,
            Ok(None) => {
                return if store.try_acquire_full_sync_lock().await.unwrap_or(false) {
                    info!("no snapshot in store, will perform full sync (lock acquired)");
                    StartupOutcome::FullSync
                } else {
                    info!("another replica is performing full sync, waiting for completion");
                    StartupOutcome::WaitForPeer
                };
            }
            Err(e) => {
                warn!(%e, "failed to load snapshot from store, will perform full sync");
                return StartupOutcome::FullSync;
            }
        };

        if stored.full_sync_complete {
            return StartupOutcome::Ready(stored);
        }

        if store.try_acquire_full_sync_lock().await.unwrap_or(false) {
            return StartupOutcome::Resume(stored);
        }

        info!("another replica is performing full sync, waiting for completion");
        StartupOutcome::WaitForPeer
    }

    /// Block until the full-sync lock becomes available or the running full
    /// sync completes (whichever comes first).
    async fn wait_until_sync_completes_or_lock_available(&self, store: &Arc<dyn SnapshotStore>) {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;

            match store.try_acquire_full_sync_lock().await {
                Ok(true) => return,
                Ok(false) => {}
                Err(e) => {
                    warn!(%e, "failed to check full-sync lock while waiting");
                }
            }

            match store.load().await {
                Ok(Some(stored)) if stored.full_sync_complete => return,
                Ok(_) => {
                    info!("waiting for another replica to complete full sync");
                }
                Err(e) => {
                    warn!(%e, "failed to load snapshot while waiting for full sync");
                }
            }
        }
    }

    /// Load initial sync state from the snapshot store if available.
    ///
    /// Inspects the database for an existing snapshot and returns delta tokens
    /// and optional resume state. If a complete snapshot is found it is loaded
    /// into the in-memory snapshot. Sets `snapshot_loaded` to `true` when a
    /// complete snapshot was loaded (the caller decides whether to mark the
    /// service as ready).
    async fn load_initial_state(&self) -> InitialSyncState {
        let mut state = InitialSyncState::default();

        let Some(store) = &self.snapshot_store else {
            return state;
        };

        loop {
            match self.load_startup_snapshot(store).await {
                StartupOutcome::Ready(stored) => {
                    let user_count = stored.users.len();
                    let group_count = stored.groups.len();
                    let membership_count = stored.membership.len();
                    let _ = store.release_full_sync_lock().await;
                    let (ut, gt) = self.apply_stored_snapshot(stored).await;
                    state.users_delta_token = ut;
                    state.groups_delta_token = gt;
                    state.snapshot_loaded = true;
                    info!(
                        user_count,
                        group_count,
                        membership_count,
                        has_users_delta_token = state.users_delta_token.is_some(),
                        has_groups_delta_token = state.groups_delta_token.is_some(),
                        "loaded snapshot from store"
                    );
                    return state;
                }
                StartupOutcome::Resume(stored) => {
                    let user_count = stored.users.len();
                    let group_count = stored.groups.len();
                    let has_users_resume = stored.users_next_link.is_some();
                    let has_groups_resume = stored.groups_next_link.is_some();

                    state.resume = Some(ResumeState {
                        users_next_link: stored.users_next_link,
                        groups_next_link: stored.groups_next_link,
                        users_delta_token: stored.users_delta_token,
                    });

                    let membership = stored
                        .membership
                        .into_iter()
                        .map(|(g, us)| (g, us.into_iter().collect()))
                        .collect();
                    let snapshot = DirectorySnapshot::new(stored.users, stored.groups, membership);
                    *self.snapshot.write().await = Some(snapshot);

                    info!(
                        user_count,
                        group_count,
                        resuming_users_from = has_users_resume,
                        resuming_groups_from = has_groups_resume,
                        "incomplete sync detected; will resume from last checkpoint"
                    );
                    return state;
                }
                StartupOutcome::FullSync => return state,
                StartupOutcome::WaitForPeer => {
                    self.wait_until_sync_completes_or_lock_available(store)
                        .await;
                }
            }
        }
    }

    pub async fn run_sync_loop(&self) {
        if !self.config.sync_jitter_max.is_zero() {
            let max_ms = self.config.sync_jitter_max.as_millis() as u64;
            let jitter = Duration::from_millis(rand::random::<u64>() % max_ms);
            info!(
                sync_interval = ?self.config.sync_interval,
                jitter = ?jitter,
                "delaying first sync for startup jitter"
            );
            tokio::time::sleep(jitter).await;
        }
        info!(
            sync_interval = ?self.config.sync_interval,
            "starting background directory delta-sync loop"
        );

        let initial = self.load_initial_state().await;
        let mut users_delta_token = initial.users_delta_token;
        let mut groups_delta_token = initial.groups_delta_token;
        let mut resume_state = initial.resume;
        let mut cycle: u64 = 0;

        if initial.snapshot_loaded {
            self.ready.store(true, Ordering::Release);
            info!("service is ready");
        }

        loop {
            cycle += 1;
            let is_initial = !self.ready.load(Ordering::Acquire);
            let start = Instant::now();

            info!(cycle, is_initial, "directory sync cycle starting");

            let sync_span =
                tracing::info_span!("directory.sync", cycle, is_initial, otel.kind = "internal",);

            match self
                .sync_once(
                    cycle,
                    &mut users_delta_token,
                    &mut groups_delta_token,
                    resume_state.take(),
                )
                .instrument(sync_span)
                .await
            {
                Ok(outcome) => {
                    let duration_secs = start.elapsed().as_secs_f64();
                    let elapsed = fmt_elapsed(start.elapsed());
                    metrics::record_sync_cycle("success", duration_secs);
                    metrics::record_sync_completed();
                    metrics::record_snapshot_size(
                        outcome.user_count as u64,
                        outcome.group_count as u64,
                        outcome.membership_count as u64,
                    );
                    if outcome.changed {
                        info!(
                            cycle,
                            user_count = outcome.user_count,
                            group_count = outcome.group_count,
                            elapsed,
                            "directory sync cycle completed, snapshot updated"
                        );
                    } else {
                        info!(
                            cycle,
                            user_count = outcome.user_count,
                            group_count = outcome.group_count,
                            elapsed,
                            "directory sync cycle completed, no changes detected"
                        );
                    }
                    if is_initial {
                        self.ready.store(true, Ordering::Release);
                        info!("initial sync finished, service is now ready");
                    }
                }
                Err(ProviderError::DeltaExpired) => {
                    if let Some(store) = &self.snapshot_store {
                        let _ = store.release_full_sync_lock().await;
                    }
                    metrics::record_sync_cycle("delta_expired", start.elapsed().as_secs_f64());
                    warn!(
                        cycle,
                        elapsed = fmt_elapsed(start.elapsed()),
                        "delta token expired, resetting to full sync on next cycle"
                    );
                    users_delta_token = None;
                    groups_delta_token = None;
                }
                Err(e) => {
                    if let Some(store) = &self.snapshot_store {
                        let _ = store.release_full_sync_lock().await;
                    }
                    metrics::record_sync_cycle("error", start.elapsed().as_secs_f64());
                    let elapsed = fmt_elapsed(start.elapsed());
                    let has_snapshot = self.snapshot.read().await.is_some();

                    if has_snapshot {
                        warn!(
                            cycle,
                            elapsed,
                            %e,
                            "directory sync failed, serving stale snapshot"
                        );
                    } else {
                        error!(
                            cycle,
                            elapsed,
                            %e,
                            "directory sync failed, no snapshot available — service not ready"
                        );
                    }
                }
            }

            info!(
                cycle,
                next_sync_in = humantime::format_duration(self.config.sync_interval).to_string(),
                "waiting for next sync cycle"
            );
            tokio::time::sleep(self.config.sync_interval).await;
        }
    }

    /// Periodically reload the snapshot from the store without ever syncing
    /// from the identity provider.
    ///
    /// Used by the reader startup mode: a separate writer keeps the store
    /// up to date, and readers pick up its changes by reloading at a fixed
    /// interval. The reader never acquires the full-sync lock and never calls
    /// the delta source.
    pub async fn run_reader_loop(&self, reload_interval: Duration) {
        let effective_interval = if reload_interval.is_zero() {
            warn!(
                configured = ?reload_interval,
                minimum = ?MIN_RELOAD_INTERVAL,
                "reader reload interval is zero; using minimum interval"
            );
            MIN_RELOAD_INTERVAL
        } else {
            reload_interval
        };

        info!(
            reload_interval = ?effective_interval,
            "starting reader snapshot reload loop"
        );

        loop {
            tokio::time::sleep(effective_interval).await;
            self.reload_from_store().await;
        }
    }

    /// Initialize the reader's in-memory snapshot at startup and mark it ready.
    ///
    /// Loads whatever the store holds — adopting even a partial snapshot — so a
    /// reader that starts during the writer's first full sync serves the data
    /// persisted so far. When nothing is available (empty store or load error)
    /// an empty snapshot is installed instead, so the reader serves empty results
    /// rather than errors and never waits for the writer. The reader is always
    /// ready once this returns.
    pub async fn initialize_reader(&self) {
        self.reload_from_store().await;

        if self.snapshot.read().await.is_none() {
            *self.snapshot.write().await = Some(DirectorySnapshot::new(
                Vec::new(),
                Vec::new(),
                HashMap::new(),
            ));
            info!(
                "no snapshot available at startup; serving an empty snapshot until the writer persists data"
            );
        }

        self.ready.store(true, Ordering::Release);
    }

    /// Reload the reader's in-memory snapshot from the store.
    ///
    /// Partial snapshots are adopted while the reader does not yet hold a snapshot
    /// from a completed full sync, so data becomes visible as the writer persists
    /// it. Once a complete snapshot is held, only another completed full sync
    /// replaces it — a partial reload never downgrades it. An empty store or a
    /// load error leaves the current snapshot untouched; the empty bootstrap
    /// snapshot is installed only by [`CachedProvider::initialize_reader`].
    async fn reload_from_store(&self) {
        let Some(store) = &self.snapshot_store else {
            return;
        };

        let load_start = Instant::now();
        let load_result = store.load().await;
        let load_outcome = if load_result.is_ok() {
            "success"
        } else {
            "error"
        };
        metrics::METRICS.postgres_load_duration.record(
            load_start.elapsed().as_secs_f64(),
            &[opentelemetry::KeyValue::new("outcome", load_outcome)],
        );

        let stored = match load_result {
            Ok(Some(stored)) => stored,
            Ok(None) => {
                info!("no snapshot in store yet; keeping current snapshot");
                return;
            }
            Err(e) => {
                warn!(%e, "reader failed to load snapshot from store");
                return;
            }
        };

        let complete = stored.full_sync_complete;
        if !complete && self.snapshot_complete.load(Ordering::Acquire) {
            info!("ignoring incomplete snapshot; keeping complete snapshot");
            return;
        }

        let user_count = stored.users.len();
        let group_count = stored.groups.len();
        let membership_count = stored.membership.len();
        self.apply_stored_snapshot(stored).await;
        self.snapshot_complete.store(complete, Ordering::Release);
        let was_ready = self.ready.swap(true, Ordering::Release);
        metrics::record_snapshot_size(
            user_count as u64,
            group_count as u64,
            membership_count as u64,
        );

        if was_ready {
            info!(
                user_count,
                group_count, membership_count, complete, "reader reloaded snapshot from store"
            );
        } else {
            info!(
                user_count,
                group_count,
                membership_count,
                complete,
                "reader loaded snapshot from store, service is now ready"
            );
        }
    }

    async fn sync_once(
        &self,
        cycle: u64,
        users_delta_token: &mut Option<String>,
        groups_delta_token: &mut Option<String>,
        mut resume: Option<ResumeState>,
    ) -> Result<SyncOutcome, ProviderError> {
        let sync_users = self.config.users_enabled;
        let sync_groups = self.config.groups_enabled || self.config.memberships_enabled;

        let is_resume = resume.is_some();
        let mut is_full_sync =
            is_resume || (users_delta_token.is_none() && groups_delta_token.is_none());

        // ---- Full-sync coordination ----
        // Acquire the full-sync lock before truncating shared tables.
        // If another replica holds it, wait for it to finish and load
        // the result instead of performing a redundant full sync.
        if is_full_sync && let Some(store) = &self.snapshot_store {
            let acquired = store.try_acquire_full_sync_lock().await.map_err(|e| {
                ProviderError::Internal(format!("try_acquire_full_sync_lock failed: {e}"))
            })?;
            if !acquired {
                info!(
                    cycle,
                    "another replica is performing full sync, waiting for completion"
                );
                self.wait_until_sync_completes_or_lock_available(store)
                    .await;

                match store.load().await {
                    Ok(Some(stored)) if stored.full_sync_complete => {
                        let _ = store.release_full_sync_lock().await;
                        let (ut, gt) = self.apply_stored_snapshot(stored).await;
                        *users_delta_token = ut;
                        *groups_delta_token = gt;
                        is_full_sync = false;
                        resume = None;
                    }
                    _ => {}
                }
            }
        }

        // On resume: skip begin_full_sync (the tables still hold the progress
        // we want to keep). On a fresh full sync: truncate and start over.
        if is_full_sync
            && !is_resume
            && let Some(store) = &self.snapshot_store
        {
            store
                .begin_full_sync()
                .await
                .map_err(|e| ProviderError::Internal(format!("begin_full_sync failed: {e}")))?;
        }

        // Determine which phases to run and their starting URLs.
        //
        // Resume cases:
        //  - users_next_link=Some → resume mid-users phase from that URL
        //  - users_next_link=None → users phase was already complete; skip it
        //  - groups_next_link=Some → resume mid-groups phase from that URL
        //  - groups_next_link=None → groups phase had not started yet
        let (skip_users, users_start_url) = match &resume {
            Some(r) if r.users_next_link.is_none() => (true, None),
            Some(r) => (false, r.users_next_link.clone()),
            None => (false, users_delta_token.clone()),
        };
        let groups_start_url = resume
            .as_ref()
            .and_then(|r| r.groups_next_link.clone())
            .or_else(|| groups_delta_token.clone());

        // ---- Users phase ----
        let mut all_user_upserts: Vec<User> = Vec::new();
        let mut all_user_removes: Vec<String> = Vec::new();
        let mut new_users_delta_token: Option<String> = None;

        if sync_users && !skip_users {
            info!(cycle, is_full_sync, is_resume, "fetching user deltas");
            let user_start = Instant::now();
            let mut url: Option<String> = users_start_url;
            let mut page: u32 = 0;
            let mut total_upserts: usize = 0;
            let mut total_removes: usize = 0;

            loop {
                page += 1;
                let result = self.delta_source.fetch_user_page(url.as_deref()).await?;

                let mut page_upserts: Vec<User> = Vec::new();
                let mut page_removes: Vec<String> = Vec::new();
                for change in result.changes {
                    match change {
                        DeltaChange::Upsert(u) => page_upserts.push(u),
                        DeltaChange::Remove(id) => page_removes.push(id),
                    }
                }
                let page_upsert_count = page_upserts.len();
                let page_remove_count = page_removes.len();
                total_upserts += page_upsert_count;
                total_removes += page_remove_count;

                // Write page data first, then update resume state. If the process
                // crashes between these two calls the page is re-processed on the
                // next restart (UPSERTs are idempotent).
                if let Some(store) = &self.snapshot_store {
                    store
                        .apply_user_changes(&page_upserts, &page_removes)
                        .await
                        .map_err(|e| {
                            ProviderError::Internal(format!("apply_user_changes failed: {e}"))
                        })?;

                    match &result.outcome {
                        PageOutcome::NextPage(next) => {
                            store.save_users_next_link(next).await.map_err(|e| {
                                ProviderError::Internal(format!("save_users_next_link failed: {e}"))
                            })?;
                        }
                        PageOutcome::Done(token) => {
                            store.commit_users_phase(token).await.map_err(|e| {
                                ProviderError::Internal(format!("commit_users_phase failed: {e}"))
                            })?;
                        }
                    }
                }

                all_user_upserts.extend(page_upserts);
                all_user_removes.extend(page_removes);

                info!(
                    cycle,
                    page,
                    upserts = page_upsert_count,
                    removes = page_remove_count,
                    total_upserts,
                    total_removes,
                    "user delta page fetched"
                );

                match result.outcome {
                    PageOutcome::NextPage(next) => url = Some(next),
                    PageOutcome::Done(token) => {
                        new_users_delta_token = Some(token);
                        break;
                    }
                }
            }

            info!(
                cycle,
                total_upserts,
                total_removes,
                elapsed = fmt_elapsed(user_start.elapsed()),
                "user delta fetch complete"
            );
        }

        // ---- Groups phase ----
        let mut all_group_upserts: Vec<Group> = Vec::new();
        let mut all_group_removes: Vec<String> = Vec::new();
        // Groups with many members appear on multiple pages (each page carrying a different
        // subset of members@delta). Track seen IDs so group properties are only accumulated
        // once for the in-memory snapshot; membership changes are accumulated from every page.
        let mut seen_group_ids: HashSet<String> = HashSet::new();
        let mut all_membership_changes: Vec<MembershipChange> = Vec::new();
        let mut new_groups_delta_token: Option<String> = None;

        // In scoped mode the global groups delta firehose is skipped entirely;
        // the assigned groups are fetched by id further below.
        if sync_groups && self.app_role_assignments.is_none() {
            info!(cycle, is_full_sync, is_resume, "fetching group deltas");
            let group_start = Instant::now();
            let mut url: Option<String> = groups_start_url;
            let mut page: u32 = 0;
            let mut total_upserts: usize = 0;
            let mut total_removes: usize = 0;
            let mut total_membership_items: usize = 0;

            loop {
                page += 1;
                let result = self.delta_source.fetch_group_page(url.as_deref()).await?;

                let mut page_upserts: Vec<Group> = Vec::new();
                let mut page_removes: Vec<String> = Vec::new();
                for change in result.changes {
                    match change {
                        DeltaChange::Upsert(g) => {
                            if seen_group_ids.insert(g.id.clone()) {
                                all_group_upserts.push(g.clone());
                            }
                            page_upserts.push(g);
                        }
                        DeltaChange::Remove(id) => {
                            if seen_group_ids.insert(id.clone()) {
                                all_group_removes.push(id.clone());
                            }
                            page_removes.push(id);
                        }
                    }
                }
                let page_upsert_count = page_upserts.len();
                let page_remove_count = page_removes.len();
                let page_membership_items: usize = result
                    .membership_changes
                    .iter()
                    .map(|m| m.added_user_ids.len() + m.removed_user_ids.len())
                    .sum();
                total_upserts += page_upsert_count;
                total_removes += page_remove_count;
                total_membership_items += page_membership_items;

                if let Some(store) = &self.snapshot_store {
                    store
                        .apply_group_changes(
                            &page_upserts,
                            &page_removes,
                            &result.membership_changes,
                        )
                        .await
                        .map_err(|e| {
                            ProviderError::Internal(format!("apply_group_changes failed: {e}"))
                        })?;

                    if let PageOutcome::NextPage(next) = &result.outcome {
                        store.save_groups_next_link(next).await.map_err(|e| {
                            ProviderError::Internal(format!("save_groups_next_link failed: {e}"))
                        })?;
                    }
                }

                all_membership_changes.extend(result.membership_changes);

                info!(
                    cycle,
                    page,
                    upserts = page_upsert_count,
                    removes = page_remove_count,
                    membership_items = page_membership_items,
                    distinct_groups = seen_group_ids.len(),
                    total_membership_items,
                    "group delta page fetched"
                );

                match result.outcome {
                    PageOutcome::NextPage(next) => url = Some(next),
                    PageOutcome::Done(token) => {
                        new_groups_delta_token = Some(token);
                        break;
                    }
                }
            }

            info!(
                cycle,
                distinct_groups = seen_group_ids.len(),
                total_upserts,
                total_removes,
                total_membership_items,
                elapsed = fmt_elapsed(group_start.elapsed()),
                "group delta fetch complete"
            );
        }

        // ---- App-role assignment scope (replaces the groups delta phase) ----
        //
        // No delta stream exists for `appRoleAssignedTo`, so the full assigned
        // set is re-fetched every cycle and persisted by replacement. This
        // inherently reconciles added / removed / renamed assignments without
        // any delta bookkeeping. Users are unaffected (delta path above).
        let scoped_result: Option<ScopedDirectory> = if let Some(source) =
            &self.app_role_assignments
        {
            info!(
                cycle,
                "fetching enterprise-application app-role assignments"
            );
            let scope_start = Instant::now();
            let scope = source.fetch().await?;
            if let Some(store) = &self.snapshot_store {
                store
                    .replace_scoped_groups(&scope.groups, &scope.members_by_group)
                    .await
                    .map_err(|e| {
                        ProviderError::Internal(format!("replace_scoped_groups failed: {e}"))
                    })?;
                store
                    .replace_service_principals(&scope.service_principals)
                    .await
                    .map_err(|e| {
                        ProviderError::Internal(format!("replace_service_principals failed: {e}"))
                    })?;
            }
            let membership_count: usize = scope.members_by_group.values().map(|s| s.len()).sum();
            info!(
                cycle,
                group_count = scope.groups.len(),
                membership_count,
                service_principal_count = scope.service_principals.len(),
                elapsed = fmt_elapsed(scope_start.elapsed()),
                "app-role assignment scope fetched and persisted"
            );
            Some(scope)
        } else {
            None
        };

        // ---- No-changes fast path (incremental only) ----
        // Never taken in scoped mode: the scope is re-materialized every cycle.
        if !is_full_sync
            && scoped_result.is_none()
            && all_user_upserts.is_empty()
            && all_user_removes.is_empty()
            && all_group_upserts.is_empty()
            && all_group_removes.is_empty()
            && all_membership_changes.is_empty()
        {
            if let Some(token) = new_users_delta_token {
                *users_delta_token = Some(token);
            }
            if let Some(token) = new_groups_delta_token {
                *groups_delta_token = Some(token);
            }
            self.persist_sync(users_delta_token, groups_delta_token, false)
                .await?;
            let guard = self.snapshot.read().await;
            let snap = guard.as_ref().unwrap();
            let membership_count: usize = snap.members_by_group.values().map(|s| s.len()).sum();
            return Ok(SyncOutcome {
                user_count: snap.users.len(),
                group_count: snap.groups.len(),
                membership_count,
                changed: false,
            });
        }

        // ---- Build / update in-memory snapshot ----
        //
        // Fresh full sync: build entirely from the data fetched this cycle.
        // Resume or incremental: start from the current in-memory snapshot
        // (which already holds the rows loaded from Postgres at startup for a
        // resume, or the previous complete snapshot for an incremental sync)
        // and apply only the changes from this cycle.
        let (users, mut groups, mut members_by_group) = if is_full_sync && !is_resume {
            let users = all_user_upserts
                .into_iter()
                .map(|u| (u.id.clone(), u))
                .collect();
            let groups = if self.config.groups_enabled {
                all_group_upserts
                    .into_iter()
                    .map(|g| (g.id.clone(), g))
                    .collect()
            } else {
                BTreeMap::new()
            };
            let members_by_group = if self.config.memberships_enabled {
                build_members_from_changes(all_membership_changes)
            } else {
                HashMap::new()
            };
            (users, groups, members_by_group)
        } else {
            let guard = self.snapshot.read().await;
            let current = guard.as_ref().unwrap();
            let mut users = current.users.clone();
            let mut groups = current.groups.clone();
            let mut members_by_group = current.members_by_group.clone();
            drop(guard);

            for id in &all_user_removes {
                users.remove(id);
            }
            for u in all_user_upserts {
                users.insert(u.id.clone(), u);
            }

            if self.config.memberships_enabled {
                for id in &all_group_removes {
                    members_by_group.remove(id);
                }
                apply_membership_changes(&mut members_by_group, all_membership_changes);
            }
            if self.config.groups_enabled {
                for id in &all_group_removes {
                    groups.remove(id);
                }
                for g in all_group_upserts {
                    groups.insert(g.id.clone(), g);
                }
            }

            (users, groups, members_by_group)
        };

        // In scoped mode the group set and its memberships come wholesale from
        // the freshly fetched assignment scope, replacing whatever the delta
        // path produced for groups (users are kept as synced above).
        if let Some(scope) = &scoped_result {
            groups = scope
                .groups
                .iter()
                .map(|g| (g.id.clone(), g.clone()))
                .collect();
            members_by_group = if self.config.memberships_enabled {
                scope.members_by_group.clone()
            } else {
                HashMap::new()
            };
        }

        let user_count = users.len();
        let group_count = groups.len();
        let membership_count: usize = members_by_group.values().map(|s| s.len()).sum();
        let snapshot = DirectorySnapshot::from_maps(users, groups, members_by_group);
        *self.snapshot.write().await = Some(snapshot);
        info!(
            cycle,
            user_count, group_count, membership_count, "snapshot swapped"
        );

        // Use the users delta token from this cycle's fetch when available.
        // On a resume where the users phase was skipped, fall back to the
        // token saved by commit_users_phase (carried in ResumeState).
        if let Some(token) = new_users_delta_token {
            *users_delta_token = Some(token);
        } else if let Some(r) = &resume
            && r.users_next_link.is_none()
        {
            *users_delta_token = r.users_delta_token.clone();
        }
        if let Some(token) = new_groups_delta_token {
            *groups_delta_token = Some(token);
        }

        self.persist_sync(users_delta_token, groups_delta_token, is_full_sync)
            .await?;

        Ok(SyncOutcome {
            user_count,
            group_count,
            membership_count,
            changed: true,
        })
    }

    async fn persist_sync(
        &self,
        users_delta_token: &Option<String>,
        groups_delta_token: &Option<String>,
        is_full_sync: bool,
    ) -> Result<(), ProviderError> {
        if let Some(store) = &self.snapshot_store {
            let start = Instant::now();
            store
                .commit_sync(
                    users_delta_token.as_deref(),
                    groups_delta_token.as_deref(),
                    is_full_sync,
                )
                .await
                .map_err(|e| ProviderError::Internal(format!("commit_sync failed: {e}")))?;
            metrics::METRICS
                .postgres_commit_duration
                .record(start.elapsed().as_secs_f64(), &[]);

            if is_full_sync {
                store.release_full_sync_lock().await.map_err(|e| {
                    ProviderError::Internal(format!("release_full_sync_lock failed: {e}"))
                })?;
            }
        }
        Ok(())
    }
}

fn build_members_from_changes(changes: Vec<MembershipChange>) -> HashMap<String, HashSet<String>> {
    let mut map: HashMap<String, HashSet<String>> = HashMap::new();
    for mc in changes {
        map.entry(mc.group_id)
            .or_default()
            .extend(mc.added_user_ids);
    }
    map
}

fn apply_membership_changes(
    members_by_group: &mut HashMap<String, HashSet<String>>,
    changes: Vec<MembershipChange>,
) {
    for mc in changes {
        let set = members_by_group.entry(mc.group_id).or_default();
        for id in mc.added_user_ids {
            set.insert(id);
        }
        for id in mc.removed_user_ids {
            set.remove(&id);
        }
    }
}

trait HasId {
    fn id(&self) -> &str;
}

impl HasId for User {
    fn id(&self) -> &str {
        &self.id
    }
}

impl HasId for Group {
    fn id(&self) -> &str {
        &self.id
    }
}

fn encode_cursor(last_id: &str) -> String {
    URL_SAFE_NO_PAD.encode(format!("id:{last_id}"))
}

fn decode_cursor(cursor: &str) -> Result<String, ProviderError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(cursor)
        .map_err(|e| ProviderError::InvalidCursor(format!("invalid cursor: {e}")))?;
    let s = String::from_utf8(bytes)
        .map_err(|e| ProviderError::InvalidCursor(format!("invalid cursor: {e}")))?;
    s.strip_prefix("id:")
        .map(String::from)
        .ok_or_else(|| ProviderError::InvalidCursor("invalid cursor format".into()))
}

fn paginate<T: Clone + HasId>(
    items: &[T],
    cursor: Option<&str>,
    limit: u32,
) -> Result<Page<T>, ProviderError> {
    let start = match cursor {
        Some(c) => {
            let last_id = decode_cursor(c)?;
            items
                .iter()
                .position(|item| item.id() > last_id.as_str())
                .unwrap_or(items.len())
        }
        None => 0,
    };
    let limit = limit as usize;

    if start >= items.len() {
        return Ok(Page {
            items: Vec::new(),
            next_cursor: None,
        });
    }

    let end = (start + limit).min(items.len());
    let page_items = items[start..end].to_vec();
    let next_cursor = if end < items.len() {
        Some(encode_cursor(page_items.last().unwrap().id()))
    } else {
        None
    };

    Ok(Page {
        items: page_items,
        next_cursor,
    })
}

fn filter_users<'a>(users: impl Iterator<Item = &'a User>, params: &UserQuery) -> Vec<&'a User> {
    users
        .filter(|u| {
            if let Some(ref email) = params.email
                && u.email.as_deref() != Some(email.as_str())
            {
                return false;
            }
            if let Some(ref dn) = params.display_name {
                let name = u.display_name.as_str();
                match parse_display_name_filter(dn) {
                    DisplayNameFilter::Prefix(p) => {
                        if !name.to_lowercase().starts_with(&p.to_lowercase()) {
                            return false;
                        }
                    }
                    DisplayNameFilter::Search(s) => {
                        if !name.to_lowercase().contains(&s.to_lowercase()) {
                            return false;
                        }
                    }
                }
            }
            true
        })
        .collect()
}

fn filter_groups<'a>(
    groups: impl Iterator<Item = &'a Group>,
    display_name: Option<&str>,
) -> Vec<&'a Group> {
    groups
        .filter(|g| {
            if let Some(dn) = display_name {
                let name = g.display_name.as_str();
                match parse_display_name_filter(dn) {
                    DisplayNameFilter::Prefix(p) => {
                        name.to_lowercase().starts_with(&p.to_lowercase())
                    }
                    DisplayNameFilter::Search(s) => name.to_lowercase().contains(&s.to_lowercase()),
                }
            } else {
                true
            }
        })
        .collect()
}

#[async_trait]
impl IdentityProvider for CachedProvider {
    async fn list_users(&self, params: &UserQuery) -> Result<Page<User>, ProviderError> {
        if !self.config.users_enabled {
            return self.inner.list_users(params).await;
        }
        let guard = self.snapshot.read().await;
        let snapshot = guard
            .as_ref()
            .ok_or_else(|| ProviderError::Internal("directory not yet synced".into()))?;
        let filtered: Vec<User> = filter_users(snapshot.users.values(), params)
            .into_iter()
            .cloned()
            .collect();
        paginate(
            &filtered,
            params.cursor.as_deref(),
            params.effective_limit(),
        )
    }

    async fn get_user(&self, user_id: &str) -> Result<User, ProviderError> {
        if !self.config.users_enabled {
            return self.inner.get_user(user_id).await;
        }
        let guard = self.snapshot.read().await;
        let snapshot = guard
            .as_ref()
            .ok_or_else(|| ProviderError::Internal("directory not yet synced".into()))?;
        snapshot
            .users
            .get(user_id)
            .cloned()
            .ok_or_else(|| ProviderError::NotFound(format!("user {user_id} not found")))
    }

    async fn get_me(&self, bearer_token: &str) -> Result<User, ProviderError> {
        if self.config.users_enabled {
            let oid = crate::http::decode_token_oid(bearer_token).ok_or_else(|| {
                ProviderError::Unauthorized("missing or invalid oid claim".into())
            })?;
            return self.get_user(&oid).await;
        }
        self.inner.get_me(bearer_token).await
    }

    async fn list_groups(&self, params: &GroupQuery) -> Result<Page<Group>, ProviderError> {
        if !self.config.groups_enabled {
            return self.inner.list_groups(params).await;
        }
        let guard = self.snapshot.read().await;
        let snapshot = guard
            .as_ref()
            .ok_or_else(|| ProviderError::Internal("directory not yet synced".into()))?;
        let filtered: Vec<Group> =
            filter_groups(snapshot.groups.values(), params.display_name.as_deref())
                .into_iter()
                .cloned()
                .collect();
        paginate(
            &filtered,
            params.cursor.as_deref(),
            params.effective_limit(),
        )
    }

    async fn get_group(&self, group_id: &str) -> Result<Group, ProviderError> {
        if !self.config.groups_enabled {
            return self.inner.get_group(group_id).await;
        }
        let guard = self.snapshot.read().await;
        let snapshot = guard
            .as_ref()
            .ok_or_else(|| ProviderError::Internal("directory not yet synced".into()))?;
        snapshot
            .groups
            .get(group_id)
            .cloned()
            .ok_or_else(|| ProviderError::NotFound(format!("group {group_id} not found")))
    }

    async fn list_group_members(
        &self,
        group_id: &str,
        params: &UserQuery,
    ) -> Result<Page<User>, ProviderError> {
        if !self.config.memberships_enabled || !self.config.users_enabled {
            return self.inner.list_group_members(group_id, params).await;
        }
        let guard = self.snapshot.read().await;
        let snapshot = guard
            .as_ref()
            .ok_or_else(|| ProviderError::Internal("directory not yet synced".into()))?;

        if !snapshot.groups.contains_key(group_id) {
            return Err(ProviderError::NotFound(format!(
                "group {group_id} not found"
            )));
        }

        let member_ids = snapshot.members_by_group.get(group_id);
        let mut members: Vec<User> = member_ids
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| snapshot.users.get(id).cloned())
                    .collect()
            })
            .unwrap_or_default();
        members.sort_by(|a, b| a.id.cmp(&b.id));

        let filtered: Vec<User> = filter_users(members.iter(), params)
            .into_iter()
            .cloned()
            .collect();
        paginate(
            &filtered,
            params.cursor.as_deref(),
            params.effective_limit(),
        )
    }

    async fn list_user_groups(
        &self,
        user_id: &str,
        params: &GroupQuery,
    ) -> Result<Page<Group>, ProviderError> {
        if !self.config.memberships_enabled || !self.config.groups_enabled {
            return self.inner.list_user_groups(user_id, params).await;
        }
        let guard = self.snapshot.read().await;
        let snapshot = guard
            .as_ref()
            .ok_or_else(|| ProviderError::Internal("directory not yet synced".into()))?;

        let group_ids = snapshot.groups_by_user.get(user_id);
        let mut groups: Vec<Group> = group_ids
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| snapshot.groups.get(id).cloned())
                    .collect()
            })
            .unwrap_or_default();
        groups.sort_by(|a, b| a.id.cmp(&b.id));

        let filtered: Vec<Group> = filter_groups(groups.iter(), params.display_name.as_deref())
            .into_iter()
            .cloned()
            .collect();
        paginate(
            &filtered,
            params.cursor.as_deref(),
            params.effective_limit(),
        )
    }

    async fn get_user_group(&self, user_id: &str, group_id: &str) -> Result<Group, ProviderError> {
        if !self.config.memberships_enabled || !self.config.groups_enabled {
            return self.inner.get_user_group(user_id, group_id).await;
        }
        let guard = self.snapshot.read().await;
        let snapshot = guard
            .as_ref()
            .ok_or_else(|| ProviderError::Internal("directory not yet synced".into()))?;

        let is_member = snapshot
            .groups_by_user
            .get(user_id)
            .map(|ids| ids.contains(group_id))
            .unwrap_or(false);

        if !is_member {
            return Err(ProviderError::NotFound(format!(
                "user {user_id} is not a member of group {group_id}"
            )));
        }

        snapshot
            .groups
            .get(group_id)
            .cloned()
            .ok_or_else(|| ProviderError::NotFound(format!("group {group_id} not found")))
    }

    async fn get_group_member(&self, group_id: &str, user_id: &str) -> Result<User, ProviderError> {
        if !self.config.memberships_enabled || !self.config.users_enabled {
            return self.inner.get_group_member(group_id, user_id).await;
        }
        let guard = self.snapshot.read().await;
        let snapshot = guard
            .as_ref()
            .ok_or_else(|| ProviderError::Internal("directory not yet synced".into()))?;

        if !snapshot.groups.contains_key(group_id) {
            return Err(ProviderError::NotFound(format!(
                "group {group_id} not found"
            )));
        }

        let is_member = snapshot
            .members_by_group
            .get(group_id)
            .map(|ids| ids.contains(user_id))
            .unwrap_or(false);

        if !is_member {
            return Err(ProviderError::NotFound(format!(
                "user {user_id} is not a member of group {group_id}"
            )));
        }

        snapshot
            .users
            .get(user_id)
            .cloned()
            .ok_or_else(|| ProviderError::NotFound(format!("user {user_id} not found")))
    }
}

fn fmt_elapsed(d: Duration) -> String {
    humantime::format_duration(Duration::from_millis(d.as_millis() as u64)).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{GroupPageResult, UserPageResult};

    fn test_user(id: &str, name: &str, email: &str) -> User {
        User {
            id: id.into(),
            display_name: name.into(),
            given_name: None,
            family_name: None,
            email: Some(email.into()),
            username: String::new(),
            active: Some(true),
            job_title: None,
        }
    }

    fn test_group(id: &str, name: &str) -> Group {
        Group {
            id: id.into(),
            display_name: name.into(),
            description: None,
        }
    }

    fn test_config() -> CacheConfig {
        CacheConfig {
            sync_interval: Duration::from_secs(3600),
            sync_jitter_max: Duration::ZERO,
            users_enabled: true,
            groups_enabled: true,
            memberships_enabled: true,
        }
    }

    fn sample_users() -> Vec<User> {
        vec![
            test_user("u1", "Alice Smith", "alice@example.com"),
            test_user("u2", "Bob Jones", "bob@example.com"),
            test_user("u3", "Jane Doe", "jane@example.com"),
            test_user("u4", "Janet Smith", "janet@example.com"),
            test_user("u5", "Charlie Brown", "charlie@example.com"),
        ]
    }

    fn sample_groups() -> Vec<Group> {
        vec![
            test_group("g1", "Admins"),
            test_group("g2", "Engineering"),
            test_group("g3", "Admin-Support"),
        ]
    }

    fn sample_membership() -> HashMap<String, HashSet<String>> {
        let mut m = HashMap::new();
        m.insert(
            "g1".into(),
            ["u1", "u2"].iter().map(|s| s.to_string()).collect(),
        );
        m.insert(
            "g2".into(),
            ["u2", "u3"].iter().map(|s| s.to_string()).collect(),
        );
        m
    }

    impl CachedProvider {
        async fn set_snapshot_for_test(
            &self,
            users: Vec<User>,
            groups: Vec<Group>,
            membership: HashMap<String, HashSet<String>>,
        ) {
            let snapshot = DirectorySnapshot::new(users, groups, membership);
            *self.snapshot.write().await = Some(snapshot);
            self.snapshot_complete.store(true, Ordering::Release);
        }
    }

    struct MockDeltaSource;

    #[async_trait]
    impl DeltaSyncSource for MockDeltaSource {
        async fn fetch_user_page(
            &self,
            _url: Option<&str>,
        ) -> Result<UserPageResult, ProviderError> {
            Ok(UserPageResult {
                changes: Vec::new(),
                outcome: PageOutcome::Done("mock-token".into()),
            })
        }

        async fn fetch_group_page(
            &self,
            _url: Option<&str>,
        ) -> Result<GroupPageResult, ProviderError> {
            Ok(GroupPageResult {
                changes: Vec::new(),
                membership_changes: Vec::new(),
                outcome: PageOutcome::Done("mock-token".into()),
            })
        }
    }

    struct MockProvider;

    impl MockProvider {
        fn new() -> Self {
            Self
        }
    }

    #[async_trait]
    impl IdentityProvider for MockProvider {
        async fn list_users(&self, _params: &UserQuery) -> Result<Page<User>, ProviderError> {
            unimplemented!()
        }
        async fn get_user(&self, _user_id: &str) -> Result<User, ProviderError> {
            unimplemented!()
        }
        async fn get_me(&self, _bearer_token: &str) -> Result<User, ProviderError> {
            unimplemented!()
        }
        async fn list_groups(&self, _params: &GroupQuery) -> Result<Page<Group>, ProviderError> {
            unimplemented!()
        }
        async fn get_group(&self, _group_id: &str) -> Result<Group, ProviderError> {
            unimplemented!()
        }
        async fn list_group_members(
            &self,
            _group_id: &str,
            _params: &UserQuery,
        ) -> Result<Page<User>, ProviderError> {
            unimplemented!()
        }
        async fn list_user_groups(
            &self,
            _user_id: &str,
            _params: &GroupQuery,
        ) -> Result<Page<Group>, ProviderError> {
            unimplemented!()
        }
        async fn get_user_group(
            &self,
            _user_id: &str,
            _group_id: &str,
        ) -> Result<Group, ProviderError> {
            unimplemented!()
        }
        async fn get_group_member(
            &self,
            _group_id: &str,
            _user_id: &str,
        ) -> Result<User, ProviderError> {
            unimplemented!()
        }
    }

    fn make_cached_provider(inner: MockProvider) -> CachedProvider {
        let delta_source: Arc<dyn DeltaSyncSource> = Arc::new(MockDeltaSource);
        let inner: Arc<dyn IdentityProvider> = Arc::new(inner);
        CachedProvider::new(delta_source, inner, None, test_config(), None)
    }

    struct MockSnapshotStore {
        present: bool,
        full_sync_complete: bool,
        users: Vec<User>,
        groups: Vec<Group>,
    }

    impl MockSnapshotStore {
        fn build_stored(&self) -> StoredSnapshot {
            StoredSnapshot {
                users: self.users.clone(),
                groups: self.groups.clone(),
                membership: HashMap::new(),
                users_delta_token: Some("ut".into()),
                groups_delta_token: Some("gt".into()),
                users_cache_enabled: true,
                groups_cache_enabled: true,
                memberships_cache_enabled: false,
                full_sync_complete: self.full_sync_complete,
                users_next_link: None,
                groups_next_link: None,
            }
        }
    }

    #[async_trait]
    impl SnapshotStore for MockSnapshotStore {
        async fn load(
            &self,
        ) -> Result<Option<StoredSnapshot>, crate::snapshot::SnapshotStoreError> {
            Ok(self.present.then(|| self.build_stored()))
        }
        async fn begin_full_sync(&self) -> Result<(), crate::snapshot::SnapshotStoreError> {
            Ok(())
        }
        async fn apply_user_changes(
            &self,
            _upserts: &[User],
            _removes: &[String],
        ) -> Result<(), crate::snapshot::SnapshotStoreError> {
            Ok(())
        }
        async fn save_users_next_link(
            &self,
            _next_link: &str,
        ) -> Result<(), crate::snapshot::SnapshotStoreError> {
            Ok(())
        }
        async fn commit_users_phase(
            &self,
            _delta_token: &str,
        ) -> Result<(), crate::snapshot::SnapshotStoreError> {
            Ok(())
        }
        async fn apply_group_changes(
            &self,
            _upserts: &[Group],
            _removes: &[String],
            _membership_changes: &[MembershipChange],
        ) -> Result<(), crate::snapshot::SnapshotStoreError> {
            Ok(())
        }
        async fn save_groups_next_link(
            &self,
            _next_link: &str,
        ) -> Result<(), crate::snapshot::SnapshotStoreError> {
            Ok(())
        }
        async fn commit_sync(
            &self,
            _users_delta_token: Option<&str>,
            _groups_delta_token: Option<&str>,
            _is_full_sync: bool,
        ) -> Result<(), crate::snapshot::SnapshotStoreError> {
            Ok(())
        }
    }

    fn make_reader_provider(store: MockSnapshotStore) -> CachedProvider {
        let delta_source: Arc<dyn DeltaSyncSource> = Arc::new(MockDeltaSource);
        let inner: Arc<dyn IdentityProvider> = Arc::new(MockProvider::new());
        let store: Arc<dyn SnapshotStore> = Arc::new(store);
        CachedProvider::new(delta_source, inner, Some(store), test_config(), None)
    }

    // ========== DirectorySnapshot ==========

    #[test]
    fn snapshot_indexes_users_by_id() {
        let users = sample_users();
        let snapshot = DirectorySnapshot::new(users.clone(), Vec::new(), HashMap::new());
        for user in &users {
            let found = snapshot.users.get(&user.id).unwrap();
            assert_eq!(found.id, user.id);
        }
    }

    #[test]
    fn snapshot_indexes_groups_by_id() {
        let groups = sample_groups();
        let snapshot = DirectorySnapshot::new(Vec::new(), groups.clone(), HashMap::new());
        for group in &groups {
            let found = snapshot.groups.get(&group.id).unwrap();
            assert_eq!(found.id, group.id);
        }
    }

    #[test]
    fn snapshot_sorts_users_by_id() {
        let users = vec![
            test_user("z1", "Zara", "z@e.com"),
            test_user("a1", "Alice", "a@e.com"),
            test_user("m1", "Mike", "m@e.com"),
        ];
        let snapshot = DirectorySnapshot::new(users, Vec::new(), HashMap::new());
        let ids: Vec<&str> = snapshot.users.keys().map(String::as_str).collect();
        assert_eq!(ids, vec!["a1", "m1", "z1"]);
    }

    #[test]
    fn snapshot_builds_groups_by_user_index() {
        let membership = sample_membership();
        let snapshot = DirectorySnapshot::new(sample_users(), sample_groups(), membership);
        let g1_members = snapshot.members_by_group.get("g1").unwrap();
        assert!(g1_members.contains("u1"));
        assert!(g1_members.contains("u2"));
        let u2_groups = snapshot.groups_by_user.get("u2").unwrap();
        assert!(u2_groups.contains("g1"));
        assert!(u2_groups.contains("g2"));
    }

    #[test]
    fn filter_users_exact_email_match() {
        let snapshot = DirectorySnapshot::new(sample_users(), Vec::new(), HashMap::new());
        let params = UserQuery {
            email: Some("jane@example.com".into()),
            display_name: None,
            limit: 20,
            cursor: None,
        };
        let filtered = filter_users(snapshot.users.values(), &params);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "u3");
    }

    #[test]
    fn filter_users_email_no_match() {
        let snapshot = DirectorySnapshot::new(sample_users(), Vec::new(), HashMap::new());
        let params = UserQuery {
            email: Some("nobody@example.com".into()),
            display_name: None,
            limit: 20,
            cursor: None,
        };
        let filtered = filter_users(snapshot.users.values(), &params);
        assert!(filtered.is_empty());
    }

    #[test]
    fn filter_users_display_name_prefix() {
        let snapshot = DirectorySnapshot::new(sample_users(), Vec::new(), HashMap::new());
        let params = UserQuery {
            email: None,
            display_name: Some("Jan*".into()),
            limit: 20,
            cursor: None,
        };
        let filtered = filter_users(snapshot.users.values(), &params);
        assert_eq!(filtered.len(), 2);
        let ids: Vec<&str> = filtered.iter().map(|u| u.id.as_str()).collect();
        assert!(ids.contains(&"u3"));
        assert!(ids.contains(&"u4"));
    }

    #[test]
    fn filter_users_display_name_substring() {
        let snapshot = DirectorySnapshot::new(sample_users(), Vec::new(), HashMap::new());
        let params = UserQuery {
            email: None,
            display_name: Some("Smith".into()),
            limit: 20,
            cursor: None,
        };
        let filtered = filter_users(snapshot.users.values(), &params);
        assert_eq!(filtered.len(), 2);
        let ids: Vec<&str> = filtered.iter().map(|u| u.id.as_str()).collect();
        assert!(ids.contains(&"u1"));
        assert!(ids.contains(&"u4"));
    }

    #[test]
    fn filter_groups_display_name_prefix_match() {
        let groups = sample_groups();
        let filtered = filter_groups(groups.iter(), Some("Adm*"));
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn filter_groups_display_name_search_match() {
        let groups = sample_groups();
        let filtered = filter_groups(groups.iter(), Some("engineer"));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "g2");
    }

    // ========== CachedProvider: get_user ==========

    #[tokio::test]
    async fn get_user_returns_user_from_snapshot() {
        let provider = make_cached_provider(MockProvider::new());
        provider
            .set_snapshot_for_test(sample_users(), sample_groups(), HashMap::new())
            .await;

        let user = provider.get_user("u1").await.unwrap();
        assert_eq!(user.id, "u1");
        assert_eq!(user.display_name, "Alice Smith");
    }

    #[tokio::test]
    async fn get_user_not_found_for_absent_id() {
        let provider = make_cached_provider(MockProvider::new());
        provider
            .set_snapshot_for_test(sample_users(), sample_groups(), HashMap::new())
            .await;

        let err = provider.get_user("nonexistent").await.unwrap_err();
        assert!(matches!(err, ProviderError::NotFound(_)));
    }

    // ========== CachedProvider: list_users ==========

    #[tokio::test]
    async fn list_users_no_filters_paginated() {
        let provider = make_cached_provider(MockProvider::new());
        provider
            .set_snapshot_for_test(sample_users(), sample_groups(), HashMap::new())
            .await;

        let params = UserQuery {
            email: None,
            display_name: None,
            limit: 3,
            cursor: None,
        };
        let page = provider.list_users(&params).await.unwrap();
        assert_eq!(page.items.len(), 3);
        assert!(page.next_cursor.is_some());
    }

    #[tokio::test]
    async fn list_users_email_filter() {
        let provider = make_cached_provider(MockProvider::new());
        provider
            .set_snapshot_for_test(sample_users(), sample_groups(), HashMap::new())
            .await;

        let params = UserQuery {
            email: Some("alice@example.com".into()),
            display_name: None,
            limit: 20,
            cursor: None,
        };
        let page = provider.list_users(&params).await.unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].id, "u1");
    }

    #[tokio::test]
    async fn list_users_display_name_prefix_filter() {
        let provider = make_cached_provider(MockProvider::new());
        provider
            .set_snapshot_for_test(sample_users(), sample_groups(), HashMap::new())
            .await;

        let params = UserQuery {
            email: None,
            display_name: Some("Jan*".into()),
            limit: 20,
            cursor: None,
        };
        let page = provider.list_users(&params).await.unwrap();
        assert_eq!(page.items.len(), 2);
    }

    #[tokio::test]
    async fn list_users_display_name_search_filter() {
        let provider = make_cached_provider(MockProvider::new());
        provider
            .set_snapshot_for_test(sample_users(), sample_groups(), HashMap::new())
            .await;

        let params = UserQuery {
            email: None,
            display_name: Some("Smith".into()),
            limit: 20,
            cursor: None,
        };
        let page = provider.list_users(&params).await.unwrap();
        assert_eq!(page.items.len(), 2);
    }

    #[tokio::test]
    async fn keyed_cursor_advances_through_pages() {
        let provider = make_cached_provider(MockProvider::new());
        provider
            .set_snapshot_for_test(sample_users(), sample_groups(), HashMap::new())
            .await;

        let params = UserQuery {
            email: None,
            display_name: None,
            limit: 2,
            cursor: None,
        };
        let page1 = provider.list_users(&params).await.unwrap();
        assert_eq!(page1.items.len(), 2);
        let cursor = page1.next_cursor.unwrap();

        let params = UserQuery {
            email: None,
            display_name: None,
            limit: 2,
            cursor: Some(cursor),
        };
        let page2 = provider.list_users(&params).await.unwrap();
        assert_eq!(page2.items.len(), 2);
        let cursor = page2.next_cursor.unwrap();

        let params = UserQuery {
            email: None,
            display_name: None,
            limit: 2,
            cursor: Some(cursor),
        };
        let page3 = provider.list_users(&params).await.unwrap();
        assert_eq!(page3.items.len(), 1);
        assert!(page3.next_cursor.is_none());

        let all_ids: Vec<String> = page1
            .items
            .iter()
            .chain(page2.items.iter())
            .chain(page3.items.iter())
            .map(|u| u.id.clone())
            .collect();
        assert_eq!(all_ids.len(), 5);
    }

    #[tokio::test]
    async fn out_of_range_cursor_returns_empty_page() {
        let provider = make_cached_provider(MockProvider::new());
        provider
            .set_snapshot_for_test(sample_users(), sample_groups(), HashMap::new())
            .await;

        let params = UserQuery {
            email: None,
            display_name: None,
            limit: 20,
            cursor: Some(encode_cursor("zzz_nonexistent")),
        };
        let page = provider.list_users(&params).await.unwrap();
        assert!(page.items.is_empty());
        assert!(page.next_cursor.is_none());
    }

    #[tokio::test]
    async fn keyed_cursor_stable_across_snapshot_swap() {
        let provider = make_cached_provider(MockProvider::new());
        provider
            .set_snapshot_for_test(sample_users(), sample_groups(), HashMap::new())
            .await;

        let params = UserQuery {
            email: None,
            display_name: None,
            limit: 2,
            cursor: None,
        };
        let page1 = provider.list_users(&params).await.unwrap();
        assert_eq!(page1.items.len(), 2);
        let cursor = page1.next_cursor.clone().unwrap();
        let last_id_page1 = page1.items.last().unwrap().id.clone();

        let mut users = sample_users();
        users.push(test_user("u0", "New First User", "u0@example.com"));
        provider
            .set_snapshot_for_test(users, sample_groups(), HashMap::new())
            .await;

        let params = UserQuery {
            email: None,
            display_name: None,
            limit: 2,
            cursor: Some(cursor),
        };
        let page2 = provider.list_users(&params).await.unwrap();
        assert!(!page2.items.is_empty());
        assert!(page2.items[0].id.as_str() > last_id_page1.as_str());
    }

    // ========== CachedProvider: get_me from cache ==========

    fn jwt_with_oid(oid: &str) -> String {
        #[derive(serde::Serialize)]
        struct Claims<'a> {
            oid: &'a str,
        }
        jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            &Claims { oid },
            &jsonwebtoken::EncodingKey::from_secret(b"test"),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn get_me_returns_user_from_cache() {
        let provider = make_cached_provider(MockProvider::new());
        provider
            .set_snapshot_for_test(sample_users(), sample_groups(), HashMap::new())
            .await;

        let token = jwt_with_oid("u1");
        let result = provider.get_me(&token).await.unwrap();
        assert_eq!(result.id, "u1");
        assert_eq!(result.display_name, "Alice Smith");
    }

    // ========== CachedProvider: membership from snapshot ==========

    #[tokio::test]
    async fn list_user_groups_returns_groups_from_snapshot() {
        let provider = make_cached_provider(MockProvider::new());
        provider
            .set_snapshot_for_test(sample_users(), sample_groups(), sample_membership())
            .await;

        let params = GroupQuery {
            display_name: None,
            limit: 20,
            cursor: None,
        };
        let page = provider.list_user_groups("u2", &params).await.unwrap();
        let mut ids: Vec<&str> = page.items.iter().map(|g| g.id.as_str()).collect();
        ids.sort_unstable();
        assert_eq!(ids, vec!["g1", "g2"]);
    }

    #[tokio::test]
    async fn list_user_groups_paginates_locally() {
        let provider = make_cached_provider(MockProvider::new());
        provider
            .set_snapshot_for_test(sample_users(), sample_groups(), sample_membership())
            .await;

        let params = GroupQuery {
            display_name: None,
            limit: 1,
            cursor: None,
        };
        let page1 = provider.list_user_groups("u2", &params).await.unwrap();
        assert_eq!(page1.items.len(), 1);
        assert!(page1.next_cursor.is_some());

        let params = GroupQuery {
            display_name: None,
            limit: 1,
            cursor: page1.next_cursor,
        };
        let page2 = provider.list_user_groups("u2", &params).await.unwrap();
        assert_eq!(page2.items.len(), 1);
        assert!(page2.next_cursor.is_none());
    }

    #[tokio::test]
    async fn list_user_groups_filters_by_display_name() {
        let provider = make_cached_provider(MockProvider::new());
        provider
            .set_snapshot_for_test(sample_users(), sample_groups(), sample_membership())
            .await;

        let params = GroupQuery {
            display_name: Some("Eng*".into()),
            limit: 20,
            cursor: None,
        };
        let page = provider.list_user_groups("u2", &params).await.unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].id, "g2");
    }

    #[tokio::test]
    async fn list_user_groups_empty_for_non_member() {
        let provider = make_cached_provider(MockProvider::new());
        provider
            .set_snapshot_for_test(sample_users(), sample_groups(), sample_membership())
            .await;

        let params = GroupQuery {
            display_name: None,
            limit: 20,
            cursor: None,
        };
        let page = provider.list_user_groups("u5", &params).await.unwrap();
        assert!(page.items.is_empty());
    }

    #[tokio::test]
    async fn get_user_group_found() {
        let provider = make_cached_provider(MockProvider::new());
        provider
            .set_snapshot_for_test(sample_users(), sample_groups(), sample_membership())
            .await;

        let group = provider.get_user_group("u1", "g1").await.unwrap();
        assert_eq!(group.id, "g1");
        assert_eq!(group.display_name, "Admins");
    }

    #[tokio::test]
    async fn get_user_group_not_found() {
        let provider = make_cached_provider(MockProvider::new());
        provider
            .set_snapshot_for_test(sample_users(), sample_groups(), sample_membership())
            .await;

        let err = provider.get_user_group("u1", "g2").await.unwrap_err();
        assert!(matches!(err, ProviderError::NotFound(_)));
    }

    #[tokio::test]
    async fn list_group_members_returns_users_from_snapshot() {
        let provider = make_cached_provider(MockProvider::new());
        provider
            .set_snapshot_for_test(sample_users(), sample_groups(), sample_membership())
            .await;

        let params = UserQuery {
            email: None,
            display_name: None,
            limit: 20,
            cursor: None,
        };
        let page = provider.list_group_members("g1", &params).await.unwrap();
        let mut ids: Vec<&str> = page.items.iter().map(|u| u.id.as_str()).collect();
        ids.sort_unstable();
        assert_eq!(ids, vec!["u1", "u2"]);
    }

    #[tokio::test]
    async fn list_group_members_filters_by_email() {
        let provider = make_cached_provider(MockProvider::new());
        provider
            .set_snapshot_for_test(sample_users(), sample_groups(), sample_membership())
            .await;

        let params = UserQuery {
            email: Some("alice@example.com".into()),
            display_name: None,
            limit: 20,
            cursor: None,
        };
        let page = provider.list_group_members("g1", &params).await.unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].id, "u1");
    }

    #[tokio::test]
    async fn list_group_members_empty_for_group_without_members() {
        let provider = make_cached_provider(MockProvider::new());
        provider
            .set_snapshot_for_test(sample_users(), sample_groups(), sample_membership())
            .await;

        let params = UserQuery {
            email: None,
            display_name: None,
            limit: 20,
            cursor: None,
        };
        let page = provider.list_group_members("g3", &params).await.unwrap();
        assert!(page.items.is_empty());
    }

    #[tokio::test]
    async fn list_group_members_not_found_for_unknown_group() {
        let provider = make_cached_provider(MockProvider::new());
        provider
            .set_snapshot_for_test(sample_users(), sample_groups(), sample_membership())
            .await;

        let params = UserQuery {
            email: None,
            display_name: None,
            limit: 20,
            cursor: None,
        };
        let err = provider
            .list_group_members("g99", &params)
            .await
            .unwrap_err();
        assert!(matches!(err, ProviderError::NotFound(_)));
    }

    #[tokio::test]
    async fn get_group_member_found() {
        let provider = make_cached_provider(MockProvider::new());
        provider
            .set_snapshot_for_test(sample_users(), sample_groups(), sample_membership())
            .await;

        let user = provider.get_group_member("g1", "u2").await.unwrap();
        assert_eq!(user.id, "u2");
    }

    #[tokio::test]
    async fn get_group_member_not_found() {
        let provider = make_cached_provider(MockProvider::new());
        provider
            .set_snapshot_for_test(sample_users(), sample_groups(), sample_membership())
            .await;

        let err = provider.get_group_member("g1", "u3").await.unwrap_err();
        assert!(matches!(err, ProviderError::NotFound(_)));
    }

    // ========== Readiness gating ==========

    #[tokio::test]
    async fn snapshot_none_returns_internal_error() {
        let provider = make_cached_provider(MockProvider::new());

        let err = provider.get_user("u1").await.unwrap_err();
        assert!(matches!(err, ProviderError::Internal(_)));

        let params = UserQuery {
            email: None,
            display_name: None,
            limit: 20,
            cursor: None,
        };
        let err = provider.list_users(&params).await.unwrap_err();
        assert!(matches!(err, ProviderError::Internal(_)));

        let params = GroupQuery {
            display_name: None,
            limit: 20,
            cursor: None,
        };
        let err = provider.list_groups(&params).await.unwrap_err();
        assert!(matches!(err, ProviderError::Internal(_)));
    }

    // ========== Reader reload ==========

    #[tokio::test]
    async fn reader_reload_applies_complete_snapshot_and_becomes_ready() {
        let provider = make_reader_provider(MockSnapshotStore {
            present: true,
            full_sync_complete: true,
            users: sample_users(),
            groups: sample_groups(),
        });
        assert!(!provider.ready.load(Ordering::Acquire));

        provider.reload_from_store().await;

        assert!(provider.ready.load(Ordering::Acquire));
        let user = provider.get_user("u1").await.unwrap();
        assert_eq!(user.display_name, "Alice Smith");
    }

    #[tokio::test]
    async fn reader_reload_accepts_partial_snapshot_when_none_loaded() {
        let provider = make_reader_provider(MockSnapshotStore {
            present: true,
            full_sync_complete: false,
            users: sample_users(),
            groups: sample_groups(),
        });
        assert!(!provider.ready.load(Ordering::Acquire));

        provider.reload_from_store().await;

        assert!(provider.ready.load(Ordering::Acquire));
        let user = provider.get_user("u1").await.unwrap();
        assert_eq!(user.display_name, "Alice Smith");
    }

    #[tokio::test]
    async fn reader_reload_keeps_existing_snapshot_over_partial() {
        let partial_user = User {
            id: "partial".into(),
            display_name: "Partial User".into(),
            ..sample_users()[0].clone()
        };
        let provider = make_reader_provider(MockSnapshotStore {
            present: true,
            full_sync_complete: false,
            users: vec![partial_user],
            groups: Vec::new(),
        });
        provider
            .set_snapshot_for_test(sample_users(), sample_groups(), sample_membership())
            .await;

        provider.reload_from_store().await;

        let user = provider.get_user("u1").await.unwrap();
        assert_eq!(user.display_name, "Alice Smith");
        let err = provider.get_user("partial").await.unwrap_err();
        assert!(matches!(err, ProviderError::NotFound(_)));
    }

    #[tokio::test]
    async fn reader_initialize_bootstraps_empty_when_store_absent() {
        let provider = make_reader_provider(MockSnapshotStore {
            present: false,
            full_sync_complete: false,
            users: Vec::new(),
            groups: Vec::new(),
        });

        provider.initialize_reader().await;

        assert!(provider.ready.load(Ordering::Acquire));
        let err = provider.get_user("u1").await.unwrap_err();
        assert!(matches!(err, ProviderError::NotFound(_)));
        let params = UserQuery {
            email: None,
            display_name: None,
            limit: 20,
            cursor: None,
        };
        let page = provider.list_users(&params).await.unwrap();
        assert!(page.items.is_empty());
    }

    #[tokio::test]
    async fn reader_initialize_adopts_partial_snapshot() {
        let provider = make_reader_provider(MockSnapshotStore {
            present: true,
            full_sync_complete: false,
            users: sample_users(),
            groups: sample_groups(),
        });

        provider.initialize_reader().await;

        assert!(provider.ready.load(Ordering::Acquire));
        let user = provider.get_user("u1").await.unwrap();
        assert_eq!(user.display_name, "Alice Smith");
    }

    #[tokio::test]
    async fn reader_adopts_partial_after_empty_bootstrap() {
        let provider = make_reader_provider(MockSnapshotStore {
            present: false,
            full_sync_complete: false,
            users: Vec::new(),
            groups: Vec::new(),
        });
        provider.initialize_reader().await;
        assert!(matches!(
            provider.get_user("u1").await.unwrap_err(),
            ProviderError::NotFound(_)
        ));

        let partial_store: Arc<dyn SnapshotStore> = Arc::new(MockSnapshotStore {
            present: true,
            full_sync_complete: false,
            users: sample_users(),
            groups: sample_groups(),
        });
        let provider = CachedProvider {
            snapshot_store: Some(partial_store),
            ..provider
        };
        provider.reload_from_store().await;

        let user = provider.get_user("u1").await.unwrap();
        assert_eq!(user.display_name, "Alice Smith");
    }

    // ========== app-role assignment scoping ==========

    type ScopeTuple = (Vec<Group>, HashMap<String, HashSet<String>>, Vec<User>);

    struct MockAppRoleSource {
        scopes: std::sync::Mutex<std::collections::VecDeque<ScopeTuple>>,
    }

    impl MockAppRoleSource {
        fn new(scopes: Vec<ScopeTuple>) -> Self {
            Self {
                scopes: std::sync::Mutex::new(scopes.into()),
            }
        }
    }

    #[async_trait]
    impl AppRoleAssignmentsSource for MockAppRoleSource {
        async fn fetch(&self) -> Result<ScopedDirectory, ProviderError> {
            let mut queue = self.scopes.lock().unwrap();
            // Repeat the final scope once exhausted so multi-cycle tests can
            // keep re-fetching a stable set.
            let (groups, members_by_group, service_principals) = if queue.len() > 1 {
                queue.pop_front().unwrap()
            } else {
                queue.front().unwrap().clone()
            };
            Ok(ScopedDirectory {
                groups,
                members_by_group,
                service_principals,
            })
        }

        async fn probe(&self) -> Result<(), ProviderError> {
            Ok(())
        }
    }

    fn scoped_provider(source: MockAppRoleSource) -> CachedProvider {
        let store: Arc<dyn SnapshotStore> = Arc::new(MockSnapshotStore {
            present: false,
            full_sync_complete: false,
            users: Vec::new(),
            groups: Vec::new(),
        });
        CachedProvider::new(
            Arc::new(MockDeltaSource),
            Arc::new(MockProvider::new()),
            Some(store),
            test_config(),
            Some(Arc::new(source)),
        )
    }

    fn members(pairs: &[(&str, &[&str])]) -> HashMap<String, HashSet<String>> {
        pairs
            .iter()
            .map(|(g, us)| (g.to_string(), us.iter().map(|u| u.to_string()).collect()))
            .collect()
    }

    #[tokio::test]
    async fn scoped_sync_populates_only_assigned_groups() {
        let provider = scoped_provider(MockAppRoleSource::new(vec![(
            vec![test_group("g1", "Admins")],
            members(&[("g1", &["u1"])]),
            Vec::new(),
        )]));

        let mut ut = None;
        let mut gt = None;
        provider.sync_once(1, &mut ut, &mut gt, None).await.unwrap();

        assert!(provider.get_group("g1").await.is_ok());
        assert!(matches!(
            provider.get_group("g2").await.unwrap_err(),
            ProviderError::NotFound(_)
        ));
        // The scoped path never populates a groups delta token.
        assert!(gt.is_none());
    }

    #[tokio::test]
    async fn scoped_sync_reconciles_removed_assignment() {
        let provider = scoped_provider(MockAppRoleSource::new(vec![
            (
                vec![test_group("g1", "Admins"), test_group("g2", "Engineering")],
                members(&[("g1", &["u1"]), ("g2", &["u2"])]),
                Vec::new(),
            ),
            (
                vec![test_group("g1", "Admins")],
                members(&[("g1", &["u1"])]),
                Vec::new(),
            ),
        ]));

        let mut ut = None;
        let mut gt = None;
        provider.sync_once(1, &mut ut, &mut gt, None).await.unwrap();
        assert!(provider.get_group("g1").await.is_ok());
        assert!(provider.get_group("g2").await.is_ok());

        // Second cycle drops g2 from the assigned set; it must disappear.
        provider.sync_once(2, &mut ut, &mut gt, None).await.unwrap();
        assert!(provider.get_group("g1").await.is_ok());
        assert!(matches!(
            provider.get_group("g2").await.unwrap_err(),
            ProviderError::NotFound(_)
        ));
    }
}
