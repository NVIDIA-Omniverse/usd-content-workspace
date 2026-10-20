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

//! Wait for a usable service-identity token, and probe expiry for stream reopen.
//!
//! `permission-auth`'s [`TokenRefresher`] only exposes `refresh()`. Durable
//! consumers also need (1) a startup barrier so the first connect does not race
//! an empty interceptor, and (2) an `is_expiring_soon` check so a quiet stream
//! can reopen with a fresh bearer the way permission-service does — without an
//! idle-silence teardown.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use thumbnail_auth::{AccessTokenFileProvider, TokenManager, TokenRefresher};
// Only the feature-gated test mutator plants a token directly.
#[cfg(any(test, feature = "test-utils"))]
use thumbnail_auth::Token;
use tracing::{info, warn};

/// How long startup will wait for the first usable bearer before failing.
pub const TOKEN_READY_TIMEOUT: Duration = Duration::from_secs(60);

/// How often an established durable stream checks whether its bearer is close
/// to expiry. Matches permission-service's consumer tick.
pub const TOKEN_EXPIRY_CHECK_INTERVAL: Duration = Duration::from_secs(30);

/// Holds the concrete token source so callers can wait, probe expiry, and still
/// obtain the shared [`TokenRefresher`] used for 401 recovery.
#[derive(Clone)]
pub struct TokenGate {
    inner: TokenGateInner,
}

#[derive(Clone)]
enum TokenGateInner {
    ClientCredentials(Arc<TokenManager>),
    /// The path is carried alongside the provider, which does not expose its own,
    /// so the startup wait can check for the file before asking the provider to
    /// read it. See [`wait_for_access_token_file`].
    AccessTokenFile {
        provider: Arc<AccessTokenFileProvider>,
        path: PathBuf,
    },
}

impl TokenGate {
    pub fn client_credentials(manager: Arc<TokenManager>) -> Self {
        Self {
            inner: TokenGateInner::ClientCredentials(manager),
        }
    }

    pub fn access_token_file(provider: Arc<AccessTokenFileProvider>, path: PathBuf) -> Self {
        Self {
            inner: TokenGateInner::AccessTokenFile { provider, path },
        }
    }

    pub fn refresher(&self) -> Arc<dyn TokenRefresher> {
        match &self.inner {
            TokenGateInner::ClientCredentials(manager) => manager.clone(),
            TokenGateInner::AccessTokenFile { provider, .. } => provider.clone(),
        }
    }

    /// True when the cached bearer is missing or inside the shared skew window.
    ///
    /// For the access-token file source, [`AccessTokenFileProvider::get_token`]
    /// already returns `None` when the cached token is expiring soon, which is
    /// the same probe permission-service uses.
    pub fn is_expiring_soon(&self) -> bool {
        match &self.inner {
            TokenGateInner::ClientCredentials(manager) => match manager.get_token_handle().lock() {
                Ok(guard) => guard.as_ref().is_none_or(|token| token.is_expiring_soon()),
                Err(_) => true,
            },
            TokenGateInner::AccessTokenFile { provider, .. } => provider.get_token().is_none(),
        }
    }

    /// Block until a usable bearer is cached, or fail when `timeout` elapses.
    pub async fn wait_until_ready(&self, timeout: Duration) -> Result<(), String> {
        if !self.is_expiring_soon() {
            return Ok(());
        }

        match &self.inner {
            TokenGateInner::ClientCredentials(manager) => {
                wait_for_client_credentials(manager, timeout).await
            }
            TokenGateInner::AccessTokenFile { provider, path } => {
                wait_for_access_token_file(provider, path, timeout).await
            }
        }
    }

    /// Clear the cached client-credentials token so [`Self::is_expiring_soon`] is
    /// true. Integration tests use this to force the peer-aligned reopen path
    /// without waiting for real clock skew.
    #[cfg(any(test, feature = "test-utils"))]
    #[doc(hidden)]
    pub fn clear_cached_token_for_tests(&self) {
        match &self.inner {
            TokenGateInner::ClientCredentials(manager) => {
                *manager.get_token_handle().lock().expect("token mutex") = None;
            }
            TokenGateInner::AccessTokenFile { .. } => {
                panic!("clear_cached_token_for_tests is for client-credentials fixtures");
            }
        }
    }

    /// Plant a long-lived bearer on the client-credentials handle. Paired with a
    /// test [`TokenRefresher`] so `TokenExpiring` reopen does not hang on a
    /// unreachable token endpoint.
    #[cfg(any(test, feature = "test-utils"))]
    #[doc(hidden)]
    pub fn restore_fresh_token_for_tests(&self) {
        match &self.inner {
            TokenGateInner::ClientCredentials(manager) => {
                *manager.get_token_handle().lock().expect("token mutex") =
                    Some(Token::new("test-token-refreshed".into(), 3600));
            }
            TokenGateInner::AccessTokenFile { .. } => {
                panic!("restore_fresh_token_for_tests is for client-credentials fixtures");
            }
        }
    }
}

async fn wait_for_client_credentials(
    manager: &Arc<TokenManager>,
    timeout: Duration,
) -> Result<(), String> {
    let received = manager.token_received.clone();
    let failed = manager.token_failure.clone();
    let manager = manager.clone();

    // The first exchange must sit inside the same timeout as the wait loop: a
    // hung DNS/TCP/TLS/HTTP call otherwise outruns TOKEN_READY_TIMEOUT.
    let ready = tokio::time::timeout(timeout, async move {
        // Kick an exchange immediately so we do not wait a full refresh-loop tick.
        manager.refresh().await;
        loop {
            if client_credentials_ready(&manager) {
                return;
            }
            tokio::select! {
                _ = received.notified() => {}
                _ = failed.notified() => {
                    warn!("[AUTH] Client-credentials token exchange failed while waiting for first token");
                }
                // `notify_waiters()` does not store a permit for a future waiter.
                // A refresh that lands between the ready check and this select
                // registering would otherwise wait for the next refresh-loop tick
                // and burn TOKEN_READY_TIMEOUT despite a usable token in cache.
                _ = tokio::time::sleep(Duration::from_millis(250)) => {}
            }
        }
    })
    .await;

    match ready {
        Ok(()) => {
            info!("[AUTH] Client-credentials token ready before outbound connect");
            Ok(())
        }
        Err(_) => Err(format!(
            "Timed out after {}s waiting for the first client-credentials token",
            timeout.as_secs()
        )),
    }
}

fn client_credentials_ready(manager: &TokenManager) -> bool {
    manager
        .get_token_handle()
        .lock()
        .ok()
        .and_then(|guard| {
            guard
                .as_ref()
                // Same skew window as `TokenGate::is_expiring_soon` / reopen path.
                .filter(|token| !token.is_expiring_soon())
                .map(|_| ())
        })
        .is_some()
}

/// Wait for the credential sidecar to publish a usable bearer.
///
/// The file is absent for the first moments of a cold start, and
/// `AccessTokenFileProvider::load` logs a read failure at ERROR every time it is
/// called before then. Polling it directly turned an ordinary startup into a burst
/// of ERROR lines that read as a failure to connect — they were reported as one —
/// when in fact the wait was doing its job. So the poll checks for the file first
/// and only asks the provider to read one that exists. A read that then fails is a
/// genuine problem (unreadable, or malformed) and keeps its ERROR.
///
/// Every filesystem touch happens inside the timeout, including the first. The file
/// arrives on a projected volume, and a stuck mount that blocks the very first read
/// would otherwise hang startup past the deadline this function exists to enforce.
async fn wait_for_access_token_file(
    provider: &Arc<AccessTokenFileProvider>,
    path: &Path,
    timeout: Duration,
) -> Result<(), String> {
    let provider = provider.clone();
    let path = path.to_path_buf();
    let poll = Duration::from_millis(200);
    let ready = tokio::time::timeout(timeout, async move {
        let mut announced_wait = false;
        loop {
            // `tokio::fs::try_exists`, not `Path::exists`: the latter is a blocking
            // syscall, and on a wedged mount it would stall the runtime thread
            // rather than the timeout.
            if tokio::fs::try_exists(&path).await.unwrap_or(false) {
                provider.load().await;
                if provider.get_token().is_some() {
                    return;
                }
            }

            // Only once the first look has come up empty, so a warm restart that
            // finds the file immediately stays quiet.
            if !announced_wait {
                info!(
                    path = %path.display(),
                    timeout_secs = timeout.as_secs(),
                    "[AUTH] Waiting for a usable access-token file before connecting"
                );
                announced_wait = true;
            }

            tokio::time::sleep(poll).await;
        }
    })
    .await;

    match ready {
        Ok(()) => {
            info!("[AUTH] Access-token file ready before outbound connect");
            Ok(())
        }
        Err(_) => Err(format!(
            "Timed out after {}s waiting for a usable token in the access-token file",
            timeout.as_secs()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use thumbnail_auth::Token;

    /// A gate whose client-credentials handle already holds a long-lived token
    /// must report ready without waiting.
    #[test]
    fn client_credentials_with_fresh_token_is_not_expiring_soon() {
        let manager = Arc::new(TokenManager::new(
            Arc::new(reqwest::Client::new()),
            "id".into(),
            "secret".into(),
            "scope".into(),
            "http://127.0.0.1/token".into(),
            Default::default(),
            Default::default(),
        ));
        {
            let handle: Arc<Mutex<Option<Token>>> = manager.get_token_handle();
            *handle.lock().unwrap() = Some(Token::new("tok".into(), 3600));
        }
        let gate = TokenGate::client_credentials(manager);
        assert!(!gate.is_expiring_soon());
    }

    #[test]
    fn client_credentials_without_token_is_expiring_soon() {
        let manager = Arc::new(TokenManager::new(
            Arc::new(reqwest::Client::new()),
            "id".into(),
            "secret".into(),
            "scope".into(),
            "http://127.0.0.1/token".into(),
            Default::default(),
            Default::default(),
        ));
        let gate = TokenGate::client_credentials(manager);
        assert!(gate.is_expiring_soon());
    }

    /// A bearer already inside the shared skew window must not count as ready
    /// for first connect — the reopen path would tear the stream down immediately.
    #[test]
    fn client_credentials_inside_skew_window_is_not_ready() {
        let manager = Arc::new(TokenManager::new(
            Arc::new(reqwest::Client::new()),
            "id".into(),
            "secret".into(),
            "scope".into(),
            "http://127.0.0.1/token".into(),
            Default::default(),
            Default::default(),
        ));
        {
            let handle = manager.get_token_handle();
            // expires_in=3 is inside the ~120s skew used by is_expiring_soon.
            *handle.lock().unwrap() = Some(Token::new("tok".into(), 3));
        }
        assert!(
            !super::client_credentials_ready(&manager),
            "tokens already in the skew window must not satisfy wait_until_ready"
        );
        let gate = TokenGate::client_credentials(manager);
        assert!(gate.is_expiring_soon());
    }

    #[tokio::test]
    async fn wait_until_ready_succeeds_when_a_fresh_token_is_already_cached() {
        let manager = Arc::new(TokenManager::new(
            Arc::new(reqwest::Client::new()),
            "id".into(),
            "secret".into(),
            "scope".into(),
            "http://127.0.0.1/token".into(),
            Default::default(),
            Default::default(),
        ));
        {
            let handle = manager.get_token_handle();
            *handle.lock().unwrap() = Some(Token::new("tok".into(), 3600));
        }
        let gate = TokenGate::client_credentials(manager);
        gate.wait_until_ready(Duration::from_millis(50))
            .await
            .expect("already-cached token must not wait on the network");
    }

    #[tokio::test]
    async fn wait_until_ready_fails_fast_when_no_token_arrives() {
        let manager = Arc::new(TokenManager::new(
            Arc::new(reqwest::Client::new()),
            "id".into(),
            "secret".into(),
            "scope".into(),
            "http://127.0.0.1:9/token".into(),
            Default::default(),
            Default::default(),
        ));
        let gate = TokenGate::client_credentials(manager);
        let err = gate
            .wait_until_ready(Duration::from_millis(80))
            .await
            .expect_err("empty cache with unreachable token endpoint must time out");
        assert!(
            err.contains("Timed out"),
            "error should name the timeout, got: {err}"
        );
    }

    fn access_token_file_contents() -> String {
        let expires_on = (chrono::Utc::now() + chrono::Duration::hours(1))
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        format!(r#"{{"access_token":"tok","expires_on":"{expires_on}"}}"#)
    }

    fn access_token_file_gate(path: &std::path::Path) -> TokenGate {
        let provider = Arc::new(AccessTokenFileProvider::new(
            path.to_path_buf(),
            Duration::from_millis(50),
        ));
        TokenGate::access_token_file(provider, path.to_path_buf())
    }

    /// The credential sidecar writes the token file some time after the process
    /// starts, so the wait has to survive the file simply not being there yet —
    /// which is the whole reason the gate exists.
    #[tokio::test]
    async fn wait_until_ready_waits_for_a_token_file_that_appears_late() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("access-token");

        let writer_path = path.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            std::fs::write(&writer_path, access_token_file_contents()).expect("write token file");
        });

        access_token_file_gate(&path)
            .wait_until_ready(Duration::from_secs(5))
            .await
            .expect("a token file written during the wait must satisfy the gate");
    }

    /// A file already in place is the warm-restart case and must not cost a poll
    /// interval.
    #[tokio::test]
    async fn wait_until_ready_returns_immediately_for_a_token_file_already_present() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("access-token");
        std::fs::write(&path, access_token_file_contents()).expect("write token file");

        access_token_file_gate(&path)
            .wait_until_ready(Duration::from_millis(50))
            .await
            .expect("an existing token file must be picked up on the first check");
    }

    /// A token file that never arrives has to fail startup rather than let the
    /// process connect without a bearer.
    #[tokio::test]
    async fn wait_until_ready_times_out_when_the_token_file_never_appears() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("never-written");

        let err = access_token_file_gate(&path)
            .wait_until_ready(Duration::from_millis(120))
            .await
            .expect_err("a missing token file must time out rather than proceed");
        assert!(
            err.contains("Timed out"),
            "error should name the timeout, got: {err}"
        );
    }
}
