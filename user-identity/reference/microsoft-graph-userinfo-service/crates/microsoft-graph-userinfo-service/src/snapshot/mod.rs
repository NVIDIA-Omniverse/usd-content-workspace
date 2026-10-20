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

pub mod postgres;

use std::collections::{HashMap, HashSet};
use std::fmt;

use async_trait::async_trait;

use crate::models::{Group, User};
use crate::provider::MembershipChange;

/// Snapshot loaded from persistent storage.
///
/// `full_sync_complete` distinguishes a usable snapshot from a partial one
/// that is being resumed:
/// - `true`: the snapshot is complete and delta tokens are valid for incremental sync.
/// - `false`: a full sync was in progress when the service stopped; the snapshot
///   contains whatever rows were written before the crash and should be used as the
///   starting point for a resumed sync, not as data ready to serve.
pub struct StoredSnapshot {
    pub users: Vec<User>,
    pub groups: Vec<Group>,
    /// Direct group membership: group_id → list of user_ids.
    pub membership: HashMap<String, Vec<String>>,
    pub users_delta_token: Option<String>,
    pub groups_delta_token: Option<String>,
    /// Cache flags that were active when this snapshot was saved.
    /// Used on load to detect flag changes that require a full re-sync.
    pub users_cache_enabled: bool,
    pub groups_cache_enabled: bool,
    pub memberships_cache_enabled: bool,
    /// Whether `commit_sync` has been called for this snapshot.
    pub full_sync_complete: bool,
    /// The `@odata.nextLink` URL from the last users page write, used to
    /// resume the users fetch after a crash. `None` means either the users
    /// phase completed or it was not enabled.
    pub users_next_link: Option<String>,
    /// The `@odata.nextLink` URL from the last groups page write, used to
    /// resume the groups fetch after a crash.
    pub groups_next_link: Option<String>,
}

pub struct SnapshotStoreError {
    pub msg: String,
}

impl fmt::Debug for SnapshotStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for SnapshotStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "snapshot store error: {}", self.msg)
    }
}

impl std::error::Error for SnapshotStoreError {}

#[async_trait]
pub trait SnapshotStore: Send + Sync {
    /// Load the persisted directory state.
    ///
    /// Returns `None` when there is no row in `sync_state`, the stored cache
    /// flags differ from the current configuration, or no resume state exists
    /// for an incomplete sync (all require a fresh full sync with TRUNCATE).
    ///
    /// Returns `Some(snapshot)` in two cases:
    /// - `snapshot.full_sync_complete = true`: ready to serve; use delta tokens
    ///   for the next incremental sync.
    /// - `snapshot.full_sync_complete = false`: a full sync was interrupted;
    ///   the snapshot contains partial row data and resume links for continuing
    ///   from where it stopped.
    async fn load(&self) -> Result<Option<StoredSnapshot>, SnapshotStoreError>;

    /// Begin a fresh full sync: truncate all entity tables, clear resume state,
    /// and mark the sync incomplete.
    async fn begin_full_sync(&self) -> Result<(), SnapshotStoreError>;

    /// Persist user changes from one delta page.
    async fn apply_user_changes(
        &self,
        upserts: &[User],
        removes: &[String],
    ) -> Result<(), SnapshotStoreError>;

    /// Save the `@odata.nextLink` URL so the users fetch can be resumed after a
    /// crash. Must be called after every successful `apply_user_changes` for a
    /// `NextPage` outcome. The data write must complete before this call.
    async fn save_users_next_link(&self, next_link: &str) -> Result<(), SnapshotStoreError>;

    /// Mark the users phase as complete: clears `users_next_link` and saves
    /// the delta token. After this call, `users_next_link = NULL` combined
    /// with a non-null `users_delta_token` signals that the users phase is
    /// complete (distinguishable from "nothing fetched yet" where both are
    /// NULL, because `begin_full_sync` clears the delta token).
    async fn commit_users_phase(&self, delta_token: &str) -> Result<(), SnapshotStoreError>;

    /// Persist group property changes and membership changes from one delta page.
    async fn apply_group_changes(
        &self,
        upserts: &[Group],
        removes: &[String],
        membership_changes: &[MembershipChange],
    ) -> Result<(), SnapshotStoreError>;

    /// Save the `@odata.nextLink` URL so the groups fetch can be resumed after a
    /// crash. Must be called after every successful `apply_group_changes` for a
    /// `NextPage` outcome. The data write must complete before this call.
    async fn save_groups_next_link(&self, next_link: &str) -> Result<(), SnapshotStoreError>;

    /// Save delta tokens and cache flags, and mark the sync as complete.
    /// Clears all resume state. Either token may be `None` if that phase is
    /// disabled.
    ///
    /// When `is_full_sync` is `true`, `full_sync_complete` is set
    /// unconditionally (the caller holds the advisory lock).  When `false`
    /// (incremental sync), the write is guarded by
    /// `WHERE full_sync_complete = true` so that it becomes a no-op if
    /// another replica is mid-full-sync.
    async fn commit_sync(
        &self,
        users_delta_token: Option<&str>,
        groups_delta_token: Option<&str>,
        is_full_sync: bool,
    ) -> Result<(), SnapshotStoreError>;

    /// Replace the entire set of scoped groups and their memberships in one
    /// atomic step. Used by the enterprise-application scoping path, where the
    /// full assigned-group set is re-fetched every cycle and reconciled by
    /// replacement (removed assignments are dropped, added ones inserted).
    ///
    /// Users are left untouched — they continue to be synced via the delta
    /// path. The default implementation is a no-op for stores that do not
    /// support scoping.
    async fn replace_scoped_groups(
        &self,
        _groups: &[Group],
        _members_by_group: &HashMap<String, HashSet<String>>,
    ) -> Result<(), SnapshotStoreError> {
        Ok(())
    }

    /// Replace the entire set of service principals surfaced as users (assigned
    /// to the enterprise application) in one atomic step. Stored in a dedicated
    /// table so the `/users/delta` reconciliation sweep never deletes them. The
    /// default implementation is a no-op.
    async fn replace_service_principals(
        &self,
        _service_principals: &[User],
    ) -> Result<(), SnapshotStoreError> {
        Ok(())
    }

    /// Attempt to acquire the cross-replica full-sync advisory lock.
    ///
    /// Returns `true` if the lock was acquired (this replica should perform
    /// the full sync).  Returns `false` if another replica already holds it.
    /// The default implementation always returns `true` (no persistence,
    /// no coordination needed).
    async fn try_acquire_full_sync_lock(&self) -> Result<bool, SnapshotStoreError> {
        Ok(true)
    }

    /// Release the full-sync advisory lock previously acquired via
    /// [`try_acquire_full_sync_lock`].  No-op if no lock is held.
    async fn release_full_sync_lock(&self) -> Result<(), SnapshotStoreError> {
        Ok(())
    }
}
