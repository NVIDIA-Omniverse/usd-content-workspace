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

use std::{fmt::Debug, path::PathBuf, sync::Arc, time::Duration};

use async_trait::async_trait;
use deadpool_postgres::Pool;
use futures::future::BoxFuture;
use snafu::IntoError;
use time::OffsetDateTime;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error};

use crate::{
    backend::metadata::postgres::error::{init_error, InitError},
    common::{
        backoff::{retry_with_backoff, Backoff},
        credentials::access_token_file,
        error::PrettyPrintError,
        secret::Secret,
    },
};

const CREDENTIALS_EXPIRY_BUFFER: Duration = Duration::from_secs(60);
const CREDENTIALS_DEFAULT_REFRESH_INTERVAL: Duration = Duration::from_secs(60);

pub type ArcCredentialsProvider = Arc<dyn CredentialsProvider>;
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Postgres client credentials.
#[derive(Debug, Clone)]
pub enum Credentials {
    Password(PasswordCredentials),
}

impl Credentials {
    fn expires_on(&self) -> Option<OffsetDateTime> {
        match self {
            Self::Password(credentials) => credentials.expires_on,
        }
    }
}

/// Password credentials.
#[derive(Debug, Clone)]
pub struct PasswordCredentials {
    pub user: String,
    pub password: Secret<String>,
    pub expires_on: Option<OffsetDateTime>,
}

/// Postgres client credentials provider.
#[async_trait]
pub trait CredentialsProvider: Debug + Send + Sync {
    async fn get_credentials(&self) -> Result<Credentials, BoxError>;
}

/// Access token file credentials provider.
#[derive(Debug)]
pub struct AccessTokenFileCredentialsProvider {
    user: String,
    provider: access_token_file::AccessTokenFileCredentialsProvider,
}

impl AccessTokenFileCredentialsProvider {
    pub fn new(user: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self {
            user: user.into(),
            provider: access_token_file::AccessTokenFileCredentialsProvider::new(path),
        }
    }
}

#[async_trait]
impl CredentialsProvider for AccessTokenFileCredentialsProvider {
    async fn get_credentials(&self) -> Result<Credentials, BoxError> {
        let data = self.provider.try_get_or_parse_token().await.map_err(Box::new)?;
        Ok(Credentials::Password(PasswordCredentials {
            user: self.user.clone(),
            password: data.access_token.clone(),
            expires_on: Some(data.expires_on),
        }))
    }
}

/// Password credentials provider.
#[derive(Debug)]
pub struct PasswordCredentialsProvider {
    user: String,
    password: Secret<String>,
}

impl PasswordCredentialsProvider {
    pub fn new(user: impl Into<String>, password: impl Into<Secret<String>>) -> Self {
        Self {
            user: user.into(),
            password: password.into(),
        }
    }
}

#[async_trait]
impl CredentialsProvider for PasswordCredentialsProvider {
    async fn get_credentials(&self) -> Result<Credentials, BoxError> {
        Ok(Credentials::Password(PasswordCredentials {
            user: self.user.clone(),
            password: self.password.clone(),
            expires_on: None,
        }))
    }
}

pub async fn create_renewable_connection_pool<F>(
    provider: ArcCredentialsProvider,
    mut create_pool: F,
    retry_backoff: Backoff,
) -> Result<(Arc<RwLock<Pool>>, CancellationToken), InitError>
where
    F: FnMut(Credentials) -> BoxFuture<'static, Result<Pool, InitError>> + Send + Sync + 'static,
{
    let credentials = get_credentials_with_retry(&retry_backoff, provider.as_ref()).await?;
    debug!(?credentials, "using postgres credentials");

    let maybe_expires_on = credentials.expires_on();
    let pool = create_pool(credentials).await?;

    let cancellation_token = CancellationToken::new();
    let shared_pool = Arc::new(RwLock::new(pool));
    if maybe_expires_on.is_some() {
        debug!("token expiration set, running a pool refresh task in the background");

        tokio::spawn(cancellation_token.clone().run_until_cancelled_owned({
            let shared_pool = shared_pool.clone();
            let mut maybe_expires_on = maybe_expires_on;
            async move {
                while let Some(expires_on) = maybe_expires_on {
                    let sleep_for = expires_on - OffsetDateTime::now_utc();
                    if sleep_for.is_negative() {
                        // Nothing to do. We must refresh the token immediately.
                    } else if let Ok(duration) = Duration::try_from(sleep_for) {
                        tokio::time::sleep(duration).await;
                    } else {
                        tokio::time::sleep(CREDENTIALS_DEFAULT_REFRESH_INTERVAL).await;
                    };

                    for backoff in retry_backoff.iter() {
                        let credentials_or_error = provider
                            .get_credentials()
                            .await
                            .inspect(|credentials| debug!(?credentials, "fetched postgres credentials"))
                            .inspect_err(|error| {
                                error!(
                                    error = %PrettyPrintError::new(&**error),
                                    "failed to fetch postgres client credentials",
                                );
                            });

                        let Ok(credentials) = credentials_or_error else {
                            tokio::time::sleep(backoff).await;
                            continue;
                        };

                        maybe_expires_on = credentials
                            .expires_on()
                            .map(|expires_on| expires_on - CREDENTIALS_EXPIRY_BUFFER);

                        let pool_or_error = create_pool(credentials)
                            .await
                            .inspect(|_| debug!("connection pool renewed"))
                            .inspect_err(|error| {
                                error!(
                                    error = %PrettyPrintError::new(error),
                                    "failed to create a connection pool",
                                );
                            });
                        let Ok(pool) = pool_or_error else {
                            tokio::time::sleep(backoff).await;
                            continue;
                        };

                        *shared_pool.write().await = pool;
                    }
                }
            }
        }));
    } else {
        debug!("token expiration was not set");
    }

    Ok((shared_pool, cancellation_token))
}

async fn get_credentials_with_retry(
    retry_backoff: &Backoff,
    provider: &dyn CredentialsProvider,
) -> Result<Credentials, InitError> {
    retry_with_backoff(retry_backoff, || provider.get_credentials())
        .await
        .map_err(|source| init_error::ClientCredentialsFetch.into_error(source))
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::Arc, time::Duration};

    use deadpool_postgres::{Config, Pool, Runtime};
    use futures::future::BoxFuture;
    use parking_lot::Mutex;
    use snafu::Snafu;
    use time::OffsetDateTime;
    use tokio::sync::oneshot::channel;
    use tokio_postgres::NoTls;

    use crate::common::{backoff::Backoff, secret::Secret};

    use super::{
        super::error::InitError, create_renewable_connection_pool, BoxError, Credentials, CredentialsProvider,
        PasswordCredentials,
    };

    #[derive(Debug, Snafu)]
    struct MockCredentialsProviderError;

    #[derive(Default)]
    struct MockCredentialsProvider {
        inner: Mutex<VecDeque<GetCredentialsMock>>,
    }

    impl MockCredentialsProvider {
        fn queue_password_credentials(
            &self,
            user: impl Into<String>,
            password: impl Into<String>,
            expires_on: OffsetDateTime,
        ) {
            self.queue_response(Credentials::Password(PasswordCredentials {
                user: user.into(),
                password: Secret::new(password),
                expires_on: Some(expires_on),
            }));
        }

        fn queue_response(&self, credentials: Credentials) {
            self.queue_callback(move || Ok(credentials));
        }

        fn queue_error(&self, error: impl std::error::Error + Send + Sync + 'static) {
            self.queue_callback(move || Err(Box::new(error)));
        }

        fn queue_callback(&self, then: impl FnOnce() -> Result<Credentials, BoxError> + Send + Sync + 'static) {
            self.inner.lock().push_back(GetCredentialsMock {
                next_response: Box::new({
                    let mut then = Some(then);
                    move || (std::mem::take(&mut then).unwrap())()
                }),
            });
        }
    }

    impl std::fmt::Debug for MockCredentialsProvider {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("MockCredentialsProvider").finish_non_exhaustive()
        }
    }

    struct GetCredentialsMock {
        next_response: Box<dyn FnMut() -> Result<Credentials, BoxError> + Send + Sync>,
    }

    #[async_trait::async_trait]
    impl CredentialsProvider for MockCredentialsProvider {
        async fn get_credentials(&self) -> Result<Credentials, BoxError> {
            if let Some(mut mock) = self.inner.lock().pop_front() {
                (mock.next_response)()
            } else {
                Err(Box::new(MockCredentialsProviderError))
            }
        }
    }

    #[test_log::test(tokio::test)]
    async fn test_create_renewable_connection_pool_renews_credentials() {
        let provider = Arc::new(MockCredentialsProvider::default());
        provider.queue_password_credentials(
            "user-1",
            "password-1",
            OffsetDateTime::now_utc() - Duration::from_secs(10),
        );

        let (last_tx, last_rx) = channel();
        provider.queue_callback(move || {
            let _ = last_tx.send(());
            Ok(Credentials::Password(PasswordCredentials {
                user: "user-2".into(),
                password: Secret::new("password-2"),
                expires_on: Some(OffsetDateTime::now_utc() + Duration::from_hours(24)),
            }))
        });

        let (_pool, cancel) = create_renewable_connection_pool(provider.clone(), create_pool, create_backoff())
            .await
            .unwrap();

        let _drop_guard = cancel.drop_guard();

        tokio::time::timeout(Duration::from_secs(5), last_rx)
            .await
            .expect("credentials provider wasn't called in time")
            .expect("send half has unexpectedly closed");
    }

    #[test_log::test(tokio::test)]
    async fn test_create_renewable_connection_pool_doesnt_renew_durable_credentials() {
        let provider = Arc::new(MockCredentialsProvider::default());

        let (last_tx, last_rx) = channel();
        provider.queue_callback(move || {
            let _ = last_tx.send(());
            Ok(Credentials::Password(PasswordCredentials {
                user: "user-a".into(),
                password: Secret::new("password-a"),
                expires_on: None,
            }))
        });

        let (_pool, cancel) = create_renewable_connection_pool(provider.clone(), create_pool, create_backoff())
            .await
            .unwrap();

        let _drop_guard = cancel.drop_guard();

        tokio::time::timeout(Duration::from_secs(5), last_rx)
            .await
            .expect("credentials provider wasn't called in time")
            .expect("send half has unexpectedly closed");

        // The subroutine has never been scheduled. Hence, the provider never escapes to another thread.
        assert_eq!(Arc::strong_count(&provider), 1);
    }

    #[test_log::test(tokio::test)]
    async fn test_create_renewable_connection_pool_doesnt_renew_durable_credentials_in_loop() {
        let provider = Arc::new(MockCredentialsProvider::default());
        provider.queue_password_credentials("user", "password", OffsetDateTime::now_utc() - Duration::from_secs(10));

        let (last_tx, last_rx) = channel();
        provider.queue_callback(move || {
            let _ = last_tx.send(());
            Ok(Credentials::Password(PasswordCredentials {
                user: "user-a".into(),
                password: Secret::new("password-a"),
                expires_on: None,
            }))
        });

        let (_pool, cancel) = create_renewable_connection_pool(provider.clone(), create_pool, create_backoff())
            .await
            .unwrap();

        let _drop_guard = cancel.drop_guard();

        tokio::time::timeout(Duration::from_secs(5), last_rx)
            .await
            .expect("subroutine didn't call the provider in time")
            .expect("send half has unexpectedly closed");

        // Receiving a notification doesn't guarantee that the subroutine has been terminated.
        tokio::time::timeout(Duration::from_secs(5), async {
            while Arc::strong_count(&provider) != 1 {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .expect("credentials provider wasn't dereferenced in time");
    }

    #[test_log::test(tokio::test)]
    async fn test_create_renewable_connection_pool_doesnt_trigger_renew_early() {
        let provider = Arc::new(MockCredentialsProvider::default());
        provider.queue_password_credentials(
            "user-1",
            "password-1",
            OffsetDateTime::now_utc() + Duration::from_millis(60001),
        );
        provider.queue_password_credentials(
            "user-2",
            "password-2",
            OffsetDateTime::now_utc() + Duration::from_secs(120),
        );

        let (last_tx, last_rx) = channel();
        provider.queue_callback(move || {
            let _ = last_tx.send(());
            Ok(Credentials::Password(PasswordCredentials {
                user: "user-3".into(),
                password: Secret::new("password-3"),
                expires_on: None,
            }))
        });

        let (_pool, cancel) = create_renewable_connection_pool(provider.clone(), create_pool, create_backoff())
            .await
            .unwrap();

        let _drop_guard = cancel.drop_guard();

        tokio::time::timeout(Duration::from_secs(5), last_rx)
            .await
            .expect_err("credentials provider wasn't expected to be called");
    }

    #[test_log::test(tokio::test)]
    async fn test_create_renewable_connection_pool_retries_failed_get_token_calls() {
        let provider = Arc::new(MockCredentialsProvider::default());
        provider.queue_password_credentials(
            "user-1",
            "password-1",
            OffsetDateTime::now_utc() - Duration::from_secs(10),
        );
        provider.queue_error(MockCredentialsProviderError);

        let (last_tx, last_rx) = channel();
        provider.queue_callback(move || {
            let _ = last_tx.send(());
            Ok(Credentials::Password(PasswordCredentials {
                user: "user-3".into(),
                password: Secret::new("password-3"),
                expires_on: None,
            }))
        });

        let (_pool, cancel) = create_renewable_connection_pool(provider.clone(), create_pool, create_backoff())
            .await
            .unwrap();

        let _drop_guard = cancel.drop_guard();

        tokio::time::timeout(Duration::from_secs(5), last_rx)
            .await
            .expect("credentials provider wasn't called in time")
            .expect("send half has unexpectedly closed");
    }

    fn create_pool(credentials: Credentials) -> BoxFuture<'static, Result<Pool, InitError>> {
        let (user, password) = match credentials {
            Credentials::Password(credentials) => (credentials.user, credentials.password),
        };

        let pool_config = Config {
            host: Some("localhost".into()),
            dbname: Some("postgres".into()),
            user: Some(user),
            password: Some(password.into_inner()),
            ..Default::default()
        };
        let pool = pool_config.create_pool(Some(Runtime::Tokio1), NoTls).unwrap();

        Box::pin(async move { Ok(pool) })
    }

    fn create_backoff() -> Backoff {
        Backoff {
            min: 0f64,
            max: 0.01f64,
            exp: 1f64,
            jitter: 0f64,
            retries: 1,
        }
    }
}
