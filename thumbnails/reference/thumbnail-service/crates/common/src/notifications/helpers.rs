// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
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

//! Helper functions for event streaming and graceful shutdown

use std::time::Duration;
use tokio::sync::watch;
use tokio::task::JoinSet;
use tracing::{error, info, warn};

pub(super) fn is_shutdown_requested(shutdown: &watch::Receiver<bool>) -> bool {
    *shutdown.borrow()
}

pub(super) async fn sleep_with_shutdown_check(
    duration: Duration,
    shutdown: &mut watch::Receiver<bool>,
    reason: &str,
) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(duration) => false,
        _ = shutdown.changed() => {
            if is_shutdown_requested(shutdown) {
                info!("[SHUTDOWN] Shutdown during {}, stopping", reason);
                true
            } else {
                false
            }
        }
    }
}

pub(super) async fn wait_for_tasks(
    task_set: &mut JoinSet<()>,
    reason: &str,
    task_cleanup_timeout_secs: u64,
) {
    let remaining_tasks = task_set.len();
    if remaining_tasks == 0 {
        return;
    }

    info!(
        "[CLEANUP] Waiting for {} in-flight tasks to complete ({})",
        remaining_tasks, reason
    );

    let deadline = tokio::time::Instant::now() + Duration::from_secs(task_cleanup_timeout_secs);
    let mut completed = 0;
    let mut timed_out = false;

    while !task_set.is_empty() {
        tokio::select! {
            Some(result) = task_set.join_next() => {
                completed += 1;
                if let Err(e) = result {
                    error!("[ERROR] Task panicked during cleanup: {}", e);
                }
            }
            _ = tokio::time::sleep_until(deadline) => {
                let remaining = task_set.len();
                warn!(
                    "[TIMEOUT] {} tasks still running after {}s, aborting them",
                    remaining, task_cleanup_timeout_secs
                );
                task_set.abort_all();
                timed_out = true;
                break;
            }
        }
    }

    if timed_out {
        warn!(
            "[CLEANUP] {} tasks completed before the {}s timeout; remaining tasks were aborted",
            completed, task_cleanup_timeout_secs
        );
    } else {
        info!("[OK] All {} tasks completed successfully", completed);
    }
}

/// Whether a gRPC status means the token should be refreshed and the stream
/// re-established.
///
/// Only `UNAUTHENTICATED` (expired token) is refreshable. `PERMISSION_DENIED`
/// is a missing queue ACL / subscribe grant — token refresh will not cure it,
/// and treating it as auth recovery produces an immediate reconnect spin at
/// info. Both durable consumers (StorageEventQueue and WorkerEventQueue) share this classifier.
pub fn is_auth_error(status: &tonic::Status) -> bool {
    matches!(status.code(), tonic::Code::Unauthenticated)
}

#[cfg(test)]
mod tests {
    use super::is_auth_error;
    use tonic::{Code, Status};

    /// Pin the classification the reconnect loops depend on: only an expired
    /// token takes the refresh+immediate-reconnect path. Permission denied
    /// (missing queue consume ACL) must not — that is the RC spin Fidot saw.
    #[test]
    fn is_auth_error_treats_only_unauthenticated_as_refreshable() {
        assert!(
            is_auth_error(&Status::new(Code::Unauthenticated, "token expired")),
            "expired-token UNAUTHENTICATED must stay on the refresh path"
        );
        assert!(
            !is_auth_error(&Status::new(
                Code::PermissionDenied,
                "The caller does not have permission"
            )),
            "PERMISSION_DENIED must not take the auth-refresh immediate-reconnect path"
        );
        assert!(
            !is_auth_error(&Status::new(Code::Unavailable, "transient")),
            "non-auth codes must not look refreshable"
        );
    }
}
