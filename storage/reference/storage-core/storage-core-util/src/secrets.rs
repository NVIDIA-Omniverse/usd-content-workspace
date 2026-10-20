// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{collections::HashMap, future::Future, path::PathBuf, pin::Pin, sync::Arc, time::Duration};

use crate::{error_limited, info_limited, warn_limited};
use figment::{
    providers::{Format, Json, Toml, Yaml},
    Figment,
};
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;
use url::Url;

use super::error::Error;

/// Configuration for the secrets.
///
/// The configuration key is `secrets` when extracting from a Figment struct.
///
/// Example configuration:
/// ```toml
/// [secrets]
/// secrets_file = "/var/secrets/secrets.json"
/// ```
///
/// OR
///
/// ```toml
/// [secrets]
/// secrets = '{"key": "value"}'
/// ```
#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    /// File path for the secrets file. This defaults to the secrets file path on NVCF so it will work automatically
    /// when users specify secrets, but can be overridden to a different path. If this file does not exist,
    /// the storage service will run without secrets. If the storage service cannot open this file for
    /// another reason, or if this file does not contain valid JSON, the storage service will exit immediately.
    #[serde(default = "default_secrets_file")]
    secrets_file: PathBuf,

    /// JSON blob to use as the secrets. If both secrets_file and secrets options are provided,
    /// this value will be used.
    #[serde(default)]
    secrets: Option<String>,

    /// How often to poll the secrets file for changes. Set to "0s" to disable file watching.
    /// Only applies when using file-based secrets (not inline JSON).
    #[serde(default = "default_poll_interval", with = "humantime_serde")]
    poll_interval: Duration,
}

fn default_secrets_file() -> PathBuf {
    PathBuf::from("/var/secrets/secrets.json")
}

fn default_poll_interval() -> Duration {
    Duration::from_secs(10)
}

impl Config {
    pub const CONFIG_KEY: &str = "secrets";
}

impl TryFrom<&Figment> for Config {
    type Error = figment::Error;

    fn try_from(figment: &Figment) -> Result<Self, Self::Error> {
        figment.focus(Self::CONFIG_KEY).extract()
    }
}

/// Creates a new Secrets Figment from the provided configuration.
pub fn generate_figment(config: &Config) -> Result<Option<Figment>, Error> {
    let mut figment = Figment::new();
    if let Some(secrets) = &config.secrets {
        info_limited!("Reading secrets from environment/cli");
        figment = figment.merge(Json::string(secrets));
    } else {
        info_limited!("Reading secrets from file: {}", config.secrets_file.display());
        if std::fs::metadata(&config.secrets_file).is_ok() {
            match config.secrets_file.extension() {
                Some(ext) if ext.eq_ignore_ascii_case("json") => {
                    figment = figment.merge(Json::file(&config.secrets_file));
                },
                Some(ext) if ext.eq_ignore_ascii_case("toml") => {
                    figment = figment.merge(Toml::file(&config.secrets_file));
                },
                Some(ext) if ext.eq_ignore_ascii_case("yaml") || ext.eq_ignore_ascii_case("yml") => {
                    figment = figment.merge(Yaml::file(&config.secrets_file));
                },
                _ => {
                    warn_limited!(
                        path = %config.secrets_file.display(),
                        "Secrets file has unrecognized extension, ignoring"
                    );
                    return Ok(None);
                },
            }
        } else {
            info_limited!("Secrets file does not exist: {}", config.secrets_file.display());
            return Ok(None);
        }
    };

    Ok(Some(figment))
}

/// Creates a new Secrets instance from a Figment configuration.
///
/// The configuration will be extracted from the Figment struct, looking for the `secrets` key.
pub fn from_figment(figment: &Figment) -> Result<Option<Figment>, Error> {
    let config = Config::try_from(figment)?;
    generate_figment(&config)
}

/// Creates a Secrets Figment from raw bytes, using the file extension from `config` to determine format.
///
/// This avoids the TOCTOU race in `generate_figment` where the file could change between
/// the content-comparison read and the Figment parse read. Instead, we parse directly from
/// the already-read bytes.
pub(crate) fn generate_figment_from_bytes(config: &Config, bytes: &[u8]) -> Result<Option<Figment>, Error> {
    let contents =
        std::str::from_utf8(bytes).map_err(|e| Error::secrets(format!("secrets file is not valid UTF-8: {e}")))?;

    let figment = Figment::new();
    let figment = match config.secrets_file.extension() {
        Some(ext) if ext.eq_ignore_ascii_case("json") => figment.merge(Json::string(contents)),
        Some(ext) if ext.eq_ignore_ascii_case("toml") => figment.merge(Toml::string(contents)),
        Some(ext) if ext.eq_ignore_ascii_case("yaml") || ext.eq_ignore_ascii_case("yml") => {
            figment.merge(Yaml::string(contents))
        },
        _ => return Ok(None),
    };

    Ok(Some(figment))
}

/// Normalize secret keys by adding `https://` prefix to keys without a URL scheme.
///
/// This is shared between S3 and Azure backends to avoid duplication.
pub fn normalize_url_keys<V>(map: &mut HashMap<String, V>) {
    let keys: Vec<String> = map.keys().cloned().collect();
    for key in keys {
        if !key.contains("://") {
            let https_key = format!("https://{key}");
            match Url::parse(&https_key) {
                Ok(_) => {
                    if let Some(value) = map.remove(&key) {
                        map.insert(https_key, value);
                    }
                },
                Err(e) => {
                    error_limited!(?e, ?key, "Config key is not a valid URL");
                },
            }
        }
    }
}

/// Trait for types that can reload their credentials when the secrets file changes.
///
/// Backend implementers only need to write the reload logic — the `SecretsWatcher` handles
/// polling, change detection, and invoking this method automatically.
pub trait SecretsReloadable: Send + Sync + 'static {
    /// Reload credentials from the given secrets figment.
    ///
    /// Implementations should handle errors internally (log and keep existing credentials)
    /// rather than propagating them — there is no caller above the watcher's poll loop
    /// that can act on a reload failure.
    fn reload_secrets(&self, secrets: Option<&Figment>) -> impl Future<Output = ()> + Send;
}

/// Combined secrets loader and optional file watcher.
///
/// Parses the secrets configuration once and provides both the initial secrets
/// figment and the ability to start a file watcher, eliminating the boilerplate
/// of calling [`from_figment`] and [`SecretsWatcher::new`] separately.
///
/// # Single backend (common case)
///
/// ```ignore
/// let secrets = Secrets::from_figment(&figment)?;
/// let backend = generate_backend(&figment, secrets.figment())?;
/// let _handle = secrets.watch(backend.clone());
/// ```
///
/// # Multiple backends
///
/// ```ignore
/// let mut secrets = Secrets::from_figment(&figment)?;
/// let s3 = generate_s3(&figment, secrets.figment())?;
/// let az = generate_az(&figment, secrets.figment())?;
/// secrets.register(s3.clone());
/// secrets.register(az.clone());
/// let _handle = secrets.start();
/// ```
pub struct Secrets {
    figment: Option<Figment>,
    watcher: Option<SecretsWatcher>,
}

impl Secrets {
    /// Load secrets from the given application figment.
    ///
    /// Parses the secrets [`Config`] once, generates the secrets figment, and
    /// prepares a [`SecretsWatcher`] (if file-based secrets with a nonzero poll interval).
    pub fn from_figment(figment: &Figment) -> Result<Self, Error> {
        let config = Config::try_from(figment)?;
        let secrets_figment = generate_figment(&config)?;
        let watcher = SecretsWatcher::from_config(config)?;
        Ok(Self {
            figment: secrets_figment,
            watcher,
        })
    }

    /// Returns a reference to the secrets figment, compatible with existing
    /// `&Option<Figment>` parameters such as `Storage::from_figment`.
    pub fn figment(&self) -> &Option<Figment> {
        &self.figment
    }

    /// Register a backend to be notified when the secrets file changes.
    ///
    /// This is a no-op if no watcher is configured (inline secrets or zero poll interval).
    /// Call [`start`](Self::start) after registering all backends.
    pub fn register<T: SecretsReloadable>(&mut self, target: Arc<T>) {
        if let Some(watcher) = &mut self.watcher {
            watcher.register(target);
        }
    }

    /// Start the background polling task.
    ///
    /// Returns `Some(JoinHandle)` if a watcher was configured, `None` otherwise.
    /// Consumes `self` because the watcher's poll loop takes ownership of its state.
    pub fn start(self) -> Option<JoinHandle<()>> {
        self.watcher.map(|w| w.start())
    }

    /// Convenience method: register a single backend and start the watcher.
    ///
    /// Equivalent to calling [`register`](Self::register) then [`start`](Self::start).
    pub fn watch<T: SecretsReloadable>(mut self, target: Arc<T>) -> Option<JoinHandle<()>> {
        self.register(target);
        self.start()
    }
}

/// Type-erased callback for storing heterogeneous `SecretsReloadable` subscribers.
type ReloadFn = Box<dyn Fn(Option<Arc<Figment>>) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Watches the secrets file for changes and notifies registered backends.
///
/// Uses periodic polling with content comparison rather than filesystem notifications (inotify).
/// Kubernetes updates mounted secrets via atomic symlink rotation (`..data` → new timestamped
/// directory), which causes inotify to fire `IN_DELETE_SELF` instead of `IN_MODIFY`. This
/// destroys the inotify watch, requiring it to be re-established after every rotation — with a
/// race-condition window where events can be missed. The `notify` crate's `RecommendedWatcher`
/// (inotify on Linux) does not re-establish watches after `DELETE_SELF`, so it silently stops
/// watching after the first secret rotation. Multiple projects (Thanos, Vault Agent) have hit
/// this and switched to polling. Given that K8s secret propagation already takes 60-90s, polling
/// every few seconds adds negligible latency while being fully reliable across all mount types.
pub struct SecretsWatcher {
    config: Config,
    subscribers: Vec<ReloadFn>,
    last_contents: Option<Vec<u8>>,
}

impl SecretsWatcher {
    /// Creates a new `SecretsWatcher` if file-based secrets are configured and polling is enabled.
    ///
    /// Returns `None` if:
    /// - Inline secrets are configured (nothing to watch)
    /// - `poll_interval` is zero (watching disabled)
    ///
    /// The watcher starts regardless of whether the secrets file currently exists.
    /// If the file appears later (e.g., K8s hasn't finished mounting yet), the poll
    /// loop will pick it up. This avoids a startup race where the pod starts before
    /// the secrets volume is mounted.
    pub fn new(figment: &Figment) -> Result<Option<Self>, Error> {
        let config = Config::try_from(figment)?;
        Self::from_config(config)
    }

    pub(crate) fn from_config(config: Config) -> Result<Option<Self>, Error> {
        // Only watch file-based secrets, not inline
        if config.secrets.is_some() {
            info_limited!("Secrets provided inline, file watching disabled");
            return Ok(None);
        }

        if config.poll_interval.is_zero() {
            info_limited!("Secrets file polling disabled (poll_interval = 0)");
            return Ok(None);
        }

        let last_contents = std::fs::read(&config.secrets_file).ok();

        info_limited!(
            path = %config.secrets_file.display(),
            poll_interval = ?config.poll_interval,
            file_exists = last_contents.is_some(),
            "Secrets file watcher initialized"
        );

        Ok(Some(Self {
            config,
            subscribers: Vec::new(),
            last_contents,
        }))
    }

    /// Register a backend to be notified when the secrets file changes.
    ///
    /// The backend is type-erased internally so the watcher can hold multiple heterogeneous backends.
    pub fn register<T: SecretsReloadable>(&mut self, target: Arc<T>) {
        self.subscribers.push(Box::new(move |figment| {
            let target = target.clone();
            Box::pin(async move {
                target.reload_secrets(figment.as_deref()).await;
            })
        }));
    }

    /// Start the background polling task.
    ///
    /// Spawns a tokio task that periodically reads the secrets file, compares contents,
    /// and notifies all registered backends on change. The task runs until the tokio
    /// runtime shuts down.
    ///
    /// Polling does not begin until this method is called; secrets read at construction
    /// time remain in effect until the first poll cycle fires.
    ///
    /// Returns the `JoinHandle` so callers can hold it for graceful shutdown.
    pub fn start(self) -> JoinHandle<()> {
        let Self {
            config,
            subscribers,
            mut last_contents,
        } = self;

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(config.poll_interval);
            // First tick completes immediately; skip it since we just read the file in new()
            interval.tick().await;

            // Track whether the file was missing on the previous poll so we only
            // log once per state transition instead of every poll cycle.
            let mut file_was_missing = last_contents.is_none();

            loop {
                interval.tick().await;

                let new_contents = match tokio::fs::read(&config.secrets_file).await {
                    Ok(contents) => {
                        if file_was_missing {
                            info_limited!(
                                path = %config.secrets_file.display(),
                                "Secrets file now available"
                            );
                            file_was_missing = false;
                        }
                        contents
                    },
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        if !file_was_missing {
                            info_limited!(
                                path = %config.secrets_file.display(),
                                "Secrets file not found, will keep checking"
                            );
                            file_was_missing = true;
                        }
                        continue;
                    },
                    Err(e) => {
                        warn_limited!(
                            path = %config.secrets_file.display(),
                            error = %e,
                            "Failed to read secrets file, keeping existing secrets"
                        );
                        continue;
                    },
                };

                if Some(&new_contents) == last_contents.as_ref() {
                    continue;
                }

                info_limited!(
                    path = %config.secrets_file.display(),
                    "Secrets file changed, reloading"
                );

                // Parse from the already-read bytes to avoid TOCTOU race
                // (file could change between our read and a second read by generate_figment)
                match generate_figment_from_bytes(&config, &new_contents) {
                    Ok(Some(figment)) => {
                        last_contents = Some(new_contents);
                        let figment = Some(Arc::new(figment));
                        for subscriber in &subscribers {
                            subscriber(figment.clone()).await;
                        }
                    },
                    Ok(None) => {
                        // Unknown file extension — cannot parse, skip notification.
                        // Update last_contents to avoid re-logging on every poll cycle.
                        last_contents = Some(new_contents);
                        warn_limited!(
                            path = %config.secrets_file.display(),
                            "Secrets file has unrecognized extension, keeping existing secrets"
                        );
                    },
                    Err(e) => {
                        error_limited!(
                            path = %config.secrets_file.display(),
                            error = %e,
                            "Failed to parse secrets file after change, keeping existing secrets"
                        );
                    },
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use figment::{
        providers::{Format, Toml},
        Jail,
    };
    use std::io::Write;

    use serde_json::json;
    use std::collections::HashMap;
    use std::fs::File;

    /// Simple struct for S3 secrets that can be extracted using Figment
    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct S3Secrets(HashMap<String, String>);

    impl S3Secrets {
        const CONFIG_KEY: &str = "storage-service-s3";
    }

    impl TryFrom<&Figment> for S3Secrets {
        type Error = figment::Error;

        fn try_from(figment: &Figment) -> Result<Self, Self::Error> {
            figment.focus(Self::CONFIG_KEY).extract()
        }
    }

    #[test_log::test()]
    fn test_basic_file_reading() {
        Jail::expect_with(|jail| {
            let tmp_dir = tempfile::tempdir().unwrap();
            let file_path = tmp_dir.path().join("secrets.json");
            let mut secrets_file = File::create(file_path.clone()).unwrap();
            let data = r#"
                {
                    "storage-service-s3": {
                        "secret1": "value1",
                        "secret2": "value2"
                    }
                }
            "#;
            secrets_file.write_all(data.as_bytes()).unwrap();

            jail.create_file(
                "config.toml",
                format!(
                    r#"
                [secrets]
                secrets_file = "{}"
                "#,
                    file_path.to_str().unwrap()
                )
                .as_str(),
            )
            .unwrap();

            let figment = Figment::new().merge(Toml::file("config.toml"));

            let secrets = from_figment(&figment).unwrap().unwrap();
            let s3_secrets: S3Secrets = secrets.focus("storage-service-s3").extract().unwrap();
            assert_eq!(s3_secrets.0.get("secret1").unwrap(), "value1");
            assert_eq!(s3_secrets.0.get("secret2").unwrap(), "value2");
            Ok(())
        });
    }

    #[test_log::test()]
    fn test_basic_json_reading() {
        Jail::expect_with(|jail| {
            let json = json!({
                "storage-service-s3": {
                    "secret1": "value1",
                    "secret2": "value2"
                }
            })
            .to_string();

            let json_str = format!("{json:?}");
            jail.create_file("config.toml", format!("[secrets]\nsecrets = {json_str}").as_str())
                .unwrap();

            let figment = Figment::new().merge(Toml::file("config.toml"));
            let secrets = from_figment(&figment).unwrap().unwrap();
            let s3_secrets: S3Secrets = secrets.focus("storage-service-s3").extract().unwrap();
            assert_eq!(s3_secrets.0.get("secret1").unwrap(), "value1");
            assert_eq!(s3_secrets.0.get("secret2").unwrap(), "value2");
            Ok(())
        });
    }

    #[test_log::test()]
    fn test_parsing_file_doesnt_exist() {
        Jail::expect_with(|jail| {
            let tmp_dir = tempfile::tempdir().unwrap();
            let file_path = tmp_dir.path().join("secrets.json");
            let config = Config {
                secrets_file: file_path,
                secrets: None,
                poll_interval: default_poll_interval(),
            };

            jail.create_file(
                "config.toml",
                format!(
                    r#"
                [secrets]
                secrets_file = "{}"
                "#,
                    config.secrets_file.to_str().unwrap()
                )
                .as_str(),
            )
            .unwrap();

            let figment = Figment::new().merge(Toml::file("config.toml"));
            assert!(from_figment(&figment).unwrap().is_none());
            Ok(())
        });
    }

    #[test_log::test()]
    fn test_parsing_file_not_json() {
        Jail::expect_with(|jail| {
            let tmp_dir = tempfile::tempdir().unwrap();
            let file_path = tmp_dir.path().join("secrets.json");
            let mut secrets_file = File::create(file_path.clone()).unwrap();
            let data = "This is not json";
            secrets_file.write_all(data.as_bytes()).unwrap();

            let config = Config {
                secrets_file: file_path,
                secrets: None,
                poll_interval: default_poll_interval(),
            };

            jail.create_file(
                "config.toml",
                format!(
                    r#"
                [secrets]
                secrets_file = "{}"
                "#,
                    config.secrets_file.to_str().unwrap()
                )
                .as_str(),
            )
            .unwrap();

            let figment = Figment::new().merge(Toml::file("config.toml"));
            let secrets = from_figment(&figment).unwrap().unwrap();
            assert!(secrets.focus("storage-service-s3").extract::<S3Secrets>().is_err());
            Ok(())
        });
    }

    #[test_log::test()]
    fn test_parsing_file_no_key() {
        Jail::expect_with(|jail| {
            let tmp_dir = tempfile::tempdir().unwrap();
            let file_path = tmp_dir.path().join("secrets.json");
            let mut secrets_file = File::create(file_path.clone()).unwrap();
            let data = r#"
                {
                    "some-other-service": {
                        "secret1": "value1",
                        "secret2": "value2"
                    }
                }
            "#;
            secrets_file.write_all(data.as_bytes()).unwrap();

            let config = Config {
                secrets_file: file_path,
                secrets: None,
                poll_interval: default_poll_interval(),
            };

            jail.create_file(
                "config.toml",
                format!(
                    r#"
                [secrets]
                secrets_file = "{}"
                "#,
                    config.secrets_file.to_str().unwrap()
                )
                .as_str(),
            )
            .unwrap();

            let figment = Figment::new().merge(Toml::file("config.toml"));
            let secrets = from_figment(&figment).unwrap().unwrap();
            let s3_secrets: S3Secrets = secrets.focus("storage-service-s3").extract().unwrap();
            assert!(s3_secrets.0.is_empty());
            Ok(())
        });
    }

    #[test_log::test()]
    fn test_json_not_json() {
        Jail::expect_with(|jail| {
            let config = Config {
                secrets_file: "/var/secrets/secrets.json".into(),
                secrets: Some("this is not json".to_string()),
                poll_interval: default_poll_interval(),
            };

            jail.create_file(
                "config.toml",
                format!(
                    r#"
                [secrets]
                secrets = "{}"
                "#,
                    config.secrets.unwrap()
                )
                .as_str(),
            )
            .unwrap();

            let figment = Figment::new().merge(Toml::file("config.toml"));
            let secrets = from_figment(&figment).unwrap().unwrap();
            assert!(secrets.focus("storage-service-s3").extract::<S3Secrets>().is_err());
            Ok(())
        });
    }

    #[test_log::test()]
    fn test_json_no_key() {
        Jail::expect_with(|jail| {
            let json = json!({
                    "some-other-service": {
                        "secret1": "value1",
                        "secret2": "value2"
                }
            })
            .to_string();
            let json_str = format!("{json:?}");

            jail.create_file("config.toml", format!("[secrets]\nsecrets = {json_str}").as_str())
                .unwrap();

            let figment = Figment::new().merge(Toml::file("config.toml"));
            let secrets = from_figment(&figment).unwrap().unwrap();
            let s3_secrets: S3Secrets = secrets.focus("storage-service-s3").extract().unwrap();
            assert!(s3_secrets.0.is_empty());
            Ok(())
        });
    }

    // =========================================================================
    // SecretsWatcher tests
    // =========================================================================

    #[test_log::test()]
    fn test_watcher_none_for_inline_secrets() {
        Jail::expect_with(|jail| {
            let json = json!({"key": "value"}).to_string();
            let json_str = format!("{json:?}");
            jail.create_file("config.toml", format!("[secrets]\nsecrets = {json_str}").as_str())
                .unwrap();

            let figment = Figment::new().merge(Toml::file("config.toml"));
            let watcher = SecretsWatcher::new(&figment).unwrap();
            assert!(watcher.is_none(), "watcher should be None for inline secrets");
            Ok(())
        });
    }

    #[test_log::test()]
    fn unit_watcher_none_for_zero_poll_interval() {
        Jail::expect_with(|jail| {
            jail.create_file(
                "config.toml",
                r#"
                [secrets]
                secrets_file = "/tmp/test-secrets.json"
                poll_interval = "0s"
                "#,
            )
            .unwrap();

            let figment = Figment::new().merge(Toml::file("config.toml"));
            let watcher = SecretsWatcher::new(&figment).unwrap();
            assert!(watcher.is_none(), "watcher should be None when poll_interval is 0");
            Ok(())
        });
    }

    #[test_log::test()]
    fn test_watcher_some_when_file_missing() {
        Jail::expect_with(|jail| {
            jail.create_file(
                "config.toml",
                r#"
                [secrets]
                secrets_file = "/tmp/nonexistent-secrets-file-12345.json"
                "#,
            )
            .unwrap();

            let figment = Figment::new().merge(Toml::file("config.toml"));
            let watcher = SecretsWatcher::new(&figment).unwrap();
            assert!(
                watcher.is_some(),
                "watcher should be Some even when file doesn't exist (startup race fix)"
            );
            Ok(())
        });
    }

    #[test_log::test()]
    fn test_watcher_some_when_file_exists() {
        Jail::expect_with(|jail| {
            let tmp_dir = tempfile::tempdir().unwrap();
            let file_path = tmp_dir.path().join("secrets.json");
            std::fs::write(&file_path, r#"{"key": "value"}"#).unwrap();

            jail.create_file(
                "config.toml",
                format!(
                    r#"
                [secrets]
                secrets_file = "{}"
                "#,
                    file_path.to_str().unwrap()
                )
                .as_str(),
            )
            .unwrap();

            let figment = Figment::new().merge(Toml::file("config.toml"));
            let watcher = SecretsWatcher::new(&figment).unwrap();
            assert!(watcher.is_some());
            Ok(())
        });
    }

    // =========================================================================
    // Secrets struct tests
    // =========================================================================

    #[test_log::test()]
    fn test_secrets_from_figment_with_file() {
        Jail::expect_with(|jail| {
            let tmp_dir = tempfile::tempdir().unwrap();
            let file_path = tmp_dir.path().join("secrets.json");
            std::fs::write(&file_path, r#"{"storage-service-s3": {"key": "val"}}"#).unwrap();

            jail.create_file(
                "config.toml",
                format!(
                    r#"
                [secrets]
                secrets_file = "{}"
                "#,
                    file_path.to_str().unwrap()
                )
                .as_str(),
            )
            .unwrap();

            let figment = Figment::new().merge(Toml::file("config.toml"));
            let secrets = Secrets::from_figment(&figment).unwrap();
            assert!(
                secrets.figment().is_some(),
                "should have a figment for file-based secrets"
            );
            Ok(())
        });
    }

    #[test_log::test()]
    fn test_secrets_from_figment_inline() {
        Jail::expect_with(|jail| {
            let json = json!({"key": "value"}).to_string();
            let json_str = format!("{json:?}");
            jail.create_file("config.toml", format!("[secrets]\nsecrets = {json_str}").as_str())
                .unwrap();

            let figment = Figment::new().merge(Toml::file("config.toml"));
            let secrets = Secrets::from_figment(&figment).unwrap();
            assert!(secrets.figment().is_some(), "should have a figment for inline secrets");
            Ok(())
        });
    }

    #[test_log::test()]
    fn test_secrets_from_figment_no_file() {
        Jail::expect_with(|jail| {
            jail.create_file(
                "config.toml",
                r#"
                [secrets]
                secrets_file = "/tmp/nonexistent-secrets-file-99999.json"
                "#,
            )
            .unwrap();

            let figment = Figment::new().merge(Toml::file("config.toml"));
            let secrets = Secrets::from_figment(&figment).unwrap();
            assert!(secrets.figment().is_none(), "should be None when file does not exist");
            Ok(())
        });
    }

    #[test_log::test()]
    fn test_secrets_watch_returns_none_inline() {
        Jail::expect_with(|jail| {
            let json = json!({"key": "value"}).to_string();
            let json_str = format!("{json:?}");
            jail.create_file("config.toml", format!("[secrets]\nsecrets = {json_str}").as_str())
                .unwrap();

            let figment = Figment::new().merge(Toml::file("config.toml"));
            let secrets = Secrets::from_figment(&figment).unwrap();

            struct Dummy;
            impl SecretsReloadable for Dummy {
                async fn reload_secrets(&self, _: Option<&Figment>) {}
            }

            let handle = secrets.watch(Arc::new(Dummy));
            assert!(handle.is_none(), "watch should return None for inline secrets");
            Ok(())
        });
    }

    #[test_log::test(tokio::test)]
    async fn test_secrets_watch_returns_handle() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let file_path = tmp_dir.path().join("secrets.json");
        std::fs::write(&file_path, r#"{"key": "value"}"#).unwrap();

        let figment = watcher_figment(&file_path, 50);
        let secrets = Secrets::from_figment(&figment).unwrap();

        struct Dummy;
        impl SecretsReloadable for Dummy {
            async fn reload_secrets(&self, _: Option<&Figment>) {}
        }

        let handle = secrets.watch(Arc::new(Dummy));
        assert!(handle.is_some(), "watch should return Some for file-based secrets");
    }

    #[test_log::test(tokio::test)]
    async fn test_secrets_register_and_start() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let file_path = tmp_dir.path().join("secrets.json");
        std::fs::write(&file_path, r#"{"key": "v1"}"#).unwrap();

        let figment = watcher_figment(&file_path, 50);
        let mut secrets = Secrets::from_figment(&figment).unwrap();

        let count_a = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let count_b = Arc::new(std::sync::atomic::AtomicU32::new(0));

        struct CountingSub {
            count: Arc<std::sync::atomic::AtomicU32>,
        }
        impl SecretsReloadable for CountingSub {
            async fn reload_secrets(&self, _: Option<&Figment>) {
                self.count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }

        secrets.register(Arc::new(CountingSub { count: count_a.clone() }));
        secrets.register(Arc::new(CountingSub { count: count_b.clone() }));
        let _handle = secrets.start();

        tokio::time::sleep(Duration::from_millis(30)).await;
        std::fs::write(&file_path, r#"{"key": "v2"}"#).unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;

        assert!(
            count_a.load(std::sync::atomic::Ordering::Relaxed) >= 1,
            "subscriber A should be notified"
        );
        assert!(
            count_b.load(std::sync::atomic::Ordering::Relaxed) >= 1,
            "subscriber B should be notified"
        );
    }

    // =========================================================================
    // normalize_url_keys tests
    // =========================================================================

    #[test_log::test()]
    fn test_normalize_url_keys_adds_scheme() {
        let mut map = HashMap::new();
        map.insert("mybucket.s3.amazonaws.com".to_string(), "value1".to_string());
        map.insert("https://already-has-scheme.com".to_string(), "value2".to_string());

        normalize_url_keys(&mut map);

        assert!(map.contains_key("https://mybucket.s3.amazonaws.com"));
        assert!(!map.contains_key("mybucket.s3.amazonaws.com"));
        assert!(map.contains_key("https://already-has-scheme.com"));
        assert_eq!(map.len(), 2);
    }

    #[test_log::test()]
    fn test_normalize_url_keys_preserves_other_schemes() {
        let mut map = HashMap::new();
        map.insert("http://insecure.example.com".to_string(), "val".to_string());

        normalize_url_keys(&mut map);

        assert!(map.contains_key("http://insecure.example.com"));
        assert_eq!(map.len(), 1);
    }

    #[test_log::test()]
    fn test_normalize_url_keys_invalid_url() {
        let mut map = HashMap::new();
        map.insert(":not-valid:".to_string(), "val".to_string());

        normalize_url_keys(&mut map);

        assert!(map.contains_key(":not-valid:"));
        assert_eq!(map.len(), 1);
    }

    #[test_log::test()]
    fn test_normalize_url_keys_empty_map() {
        let mut map: HashMap<String, String> = HashMap::new();
        normalize_url_keys(&mut map);
        assert!(map.is_empty());
    }

    // =========================================================================
    // generate_figment_from_bytes tests
    // =========================================================================

    #[test_log::test()]
    fn test_generate_figment_from_bytes_json() {
        let config = Config {
            secrets_file: PathBuf::from("/tmp/test.json"),
            secrets: None,
            poll_interval: default_poll_interval(),
        };
        let json_bytes = br#"{"storage-service-s3": {"key": "val"}}"#;

        let figment = generate_figment_from_bytes(&config, json_bytes).unwrap().unwrap();
        let s3: S3Secrets = figment.focus("storage-service-s3").extract().unwrap();
        assert_eq!(s3.0.get("key").unwrap(), "val");
    }

    #[test_log::test()]
    fn test_generate_figment_from_bytes_toml() {
        let config = Config {
            secrets_file: PathBuf::from("/tmp/test.toml"),
            secrets: None,
            poll_interval: default_poll_interval(),
        };
        let toml_bytes = br#"
[storage-service-s3]
key = "val"
"#;

        let figment = generate_figment_from_bytes(&config, toml_bytes).unwrap().unwrap();
        let s3: S3Secrets = figment.focus("storage-service-s3").extract().unwrap();
        assert_eq!(s3.0.get("key").unwrap(), "val");
    }

    #[test_log::test()]
    fn test_generate_figment_from_bytes_yaml() {
        for ext in &["yaml", "yml", "YAML", "YML"] {
            let config = Config {
                secrets_file: PathBuf::from(format!("/tmp/test.{ext}")),
                secrets: None,
                poll_interval: default_poll_interval(),
            };
            let yaml_bytes = b"storage-service-s3:\n  key: val\n";

            let figment = generate_figment_from_bytes(&config, yaml_bytes).unwrap().unwrap();
            let s3: S3Secrets = figment.focus("storage-service-s3").extract().unwrap();
            assert_eq!(s3.0.get("key").unwrap(), "val", "failed for extension: {ext}");
        }
    }

    #[test_log::test()]
    fn test_generate_figment_from_bytes_invalid_utf8() {
        let config = Config {
            secrets_file: PathBuf::from("/tmp/test.json"),
            secrets: None,
            poll_interval: default_poll_interval(),
        };
        let invalid_bytes: &[u8] = &[0xFF, 0xFE, 0xFD];

        let result = generate_figment_from_bytes(&config, invalid_bytes);
        assert!(result.is_err());
    }

    #[test_log::test()]
    fn test_generate_figment_from_bytes_unknown_extension() {
        let config = Config {
            secrets_file: PathBuf::from("/tmp/test.txt"),
            secrets: None,
            poll_interval: default_poll_interval(),
        };
        let bytes = b"some data";

        // Unknown extension returns None (no parser for this format)
        assert!(generate_figment_from_bytes(&config, bytes).unwrap().is_none());
    }

    #[test_log::test()]
    fn test_generate_figment_from_bytes_no_extension() {
        let config = Config {
            secrets_file: PathBuf::from("/tmp/secrets"),
            secrets: None,
            poll_interval: default_poll_interval(),
        };
        let bytes = b"some data";

        // No extension returns None
        assert!(generate_figment_from_bytes(&config, bytes).unwrap().is_none());
    }

    // =========================================================================
    // Config deserialization tests
    // =========================================================================

    #[test_log::test()]
    fn test_config_default_values() {
        Jail::expect_with(|jail| {
            jail.create_file("config.toml", "[secrets]\n").unwrap();

            let figment = Figment::new().merge(Toml::file("config.toml"));
            let config = Config::try_from(&figment).unwrap();
            assert_eq!(config.secrets_file, PathBuf::from("/var/secrets/secrets.json"));
            assert_eq!(config.poll_interval, Duration::from_secs(10));
            assert!(config.secrets.is_none());
            Ok(())
        });
    }

    #[test_log::test()]
    fn test_config_custom_poll_interval() {
        Jail::expect_with(|jail| {
            jail.create_file(
                "config.toml",
                r#"
                [secrets]
                poll_interval = "30s"
                "#,
            )
            .unwrap();

            let figment = Figment::new().merge(Toml::file("config.toml"));
            let config = Config::try_from(&figment).unwrap();
            assert_eq!(config.poll_interval, Duration::from_secs(30));
            Ok(())
        });
    }

    #[test_log::test()]
    fn test_config_disable_polling() {
        Jail::expect_with(|jail| {
            jail.create_file(
                "config.toml",
                r#"
                [secrets]
                poll_interval = "0s"
                "#,
            )
            .unwrap();

            let figment = Figment::new().merge(Toml::file("config.toml"));
            let config = Config::try_from(&figment).unwrap();
            assert!(config.poll_interval.is_zero());
            Ok(())
        });
    }

    // =========================================================================
    // SecretsWatcher poll loop integration tests
    // =========================================================================

    /// Helper to build a figment with custom secrets Config nested under the "secrets" key.
    fn watcher_figment(secrets_file: &std::path::Path, poll_millis: u64) -> Figment {
        use figment::providers::Serialized;
        #[derive(Serialize)]
        struct Wrapper {
            secrets: WrapperInner,
        }
        #[derive(Serialize)]
        struct WrapperInner {
            secrets_file: String,
            poll_interval: String,
        }
        Figment::new().merge(Serialized::defaults(Wrapper {
            secrets: WrapperInner {
                secrets_file: secrets_file.to_str().unwrap().to_string(),
                poll_interval: format!("{poll_millis}ms"),
            },
        }))
    }

    #[test_log::test(tokio::test)]
    async fn test_watcher_detects_file_change() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let file_path = tmp_dir.path().join("secrets.json");
        std::fs::write(&file_path, r#"{"storage-service-s3": {"key": "original"}}"#).unwrap();

        let figment = watcher_figment(&file_path, 50);

        let (tx, rx) = tokio::sync::oneshot::channel::<Option<String>>();
        let tx = Arc::new(std::sync::Mutex::new(Some(tx)));

        struct TestSubscriber {
            tx: Arc<std::sync::Mutex<Option<tokio::sync::oneshot::Sender<Option<String>>>>>,
        }

        impl SecretsReloadable for TestSubscriber {
            async fn reload_secrets(&self, secrets: Option<&Figment>) {
                let value = secrets.and_then(|f| {
                    f.focus("storage-service-s3")
                        .extract::<HashMap<String, String>>()
                        .ok()
                        .and_then(|m| m.get("key").cloned())
                });
                if let Some(tx) = self.tx.lock().unwrap().take() {
                    let _ = tx.send(value);
                }
            }
        }

        let subscriber = Arc::new(TestSubscriber { tx });

        let mut watcher = SecretsWatcher::new(&figment).unwrap().unwrap();
        watcher.register(subscriber);
        let _handle = watcher.start();

        // Give the watcher time to complete its first (no-op) tick
        tokio::time::sleep(Duration::from_millis(30)).await;

        // Change the file
        std::fs::write(&file_path, r#"{"storage-service-s3": {"key": "updated"}}"#).unwrap();

        // Wait for notification
        let result = tokio::time::timeout(Duration::from_secs(5), rx).await;
        let value = result.expect("timed out waiting for reload").unwrap();
        assert_eq!(value, Some("updated".to_string()));
    }

    #[test_log::test(tokio::test)]
    async fn test_watcher_no_notification_when_file_unchanged() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let file_path = tmp_dir.path().join("secrets.json");
        std::fs::write(&file_path, r#"{"key": "value"}"#).unwrap();

        let figment = watcher_figment(&file_path, 50);

        let call_count = Arc::new(std::sync::atomic::AtomicU32::new(0));

        struct CountingSubscriber {
            count: Arc<std::sync::atomic::AtomicU32>,
        }

        impl SecretsReloadable for CountingSubscriber {
            async fn reload_secrets(&self, _secrets: Option<&Figment>) {
                self.count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }

        let subscriber = Arc::new(CountingSubscriber {
            count: call_count.clone(),
        });

        let mut watcher = SecretsWatcher::new(&figment).unwrap().unwrap();
        watcher.register(subscriber);
        let _handle = watcher.start();

        // Wait for several poll cycles
        tokio::time::sleep(Duration::from_millis(300)).await;

        assert_eq!(
            call_count.load(std::sync::atomic::Ordering::Relaxed),
            0,
            "subscriber should not be called when file is unchanged"
        );
    }

    #[test_log::test(tokio::test)]
    async fn test_watcher_handles_missing_file_gracefully() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let file_path = tmp_dir.path().join("nonexistent.json");

        let figment = watcher_figment(&file_path, 50);

        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let tx = Arc::new(std::sync::Mutex::new(Some(tx)));

        struct NotifySubscriber {
            tx: Arc<std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
        }

        impl SecretsReloadable for NotifySubscriber {
            async fn reload_secrets(&self, _secrets: Option<&Figment>) {
                if let Some(tx) = self.tx.lock().unwrap().take() {
                    let _ = tx.send(());
                }
            }
        }

        let subscriber = Arc::new(NotifySubscriber { tx });

        let mut watcher = SecretsWatcher::new(&figment).unwrap().unwrap();
        watcher.register(subscriber);
        let _handle = watcher.start();

        // Wait a bit, then create the file (simulating K8s late mount)
        tokio::time::sleep(Duration::from_millis(100)).await;
        std::fs::write(&file_path, r#"{"key": "value"}"#).unwrap();

        let result = tokio::time::timeout(Duration::from_secs(5), rx).await;
        assert!(result.is_ok(), "subscriber should be notified when file appears");
    }

    #[test_log::test(tokio::test)]
    async fn test_watcher_notifies_multiple_subscribers() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let file_path = tmp_dir.path().join("secrets.json");
        std::fs::write(&file_path, r#"{"key": "v1"}"#).unwrap();

        let figment = watcher_figment(&file_path, 50);

        let count_a = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let count_b = Arc::new(std::sync::atomic::AtomicU32::new(0));

        struct CountingSubscriber {
            count: Arc<std::sync::atomic::AtomicU32>,
        }

        impl SecretsReloadable for CountingSubscriber {
            async fn reload_secrets(&self, _secrets: Option<&Figment>) {
                self.count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }

        let sub_a = Arc::new(CountingSubscriber { count: count_a.clone() });
        let sub_b = Arc::new(CountingSubscriber { count: count_b.clone() });

        let mut watcher = SecretsWatcher::new(&figment).unwrap().unwrap();
        watcher.register(sub_a);
        watcher.register(sub_b);
        let _handle = watcher.start();

        tokio::time::sleep(Duration::from_millis(30)).await;
        std::fs::write(&file_path, r#"{"key": "v2"}"#).unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;

        assert!(
            count_a.load(std::sync::atomic::Ordering::Relaxed) >= 1,
            "subscriber A should be notified"
        );
        assert!(
            count_b.load(std::sync::atomic::Ordering::Relaxed) >= 1,
            "subscriber B should be notified"
        );
    }

    #[test_log::test(tokio::test)]
    async fn test_watcher_keeps_old_secrets_on_parse_failure() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let file_path = tmp_dir.path().join("secrets.json");
        std::fs::write(&file_path, r#"{"key": "v1"}"#).unwrap();

        let figment = watcher_figment(&file_path, 50);

        let call_count = Arc::new(std::sync::atomic::AtomicU32::new(0));

        struct CountingSubscriber {
            count: Arc<std::sync::atomic::AtomicU32>,
        }

        impl SecretsReloadable for CountingSubscriber {
            async fn reload_secrets(&self, _secrets: Option<&Figment>) {
                self.count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }

        let subscriber = Arc::new(CountingSubscriber {
            count: call_count.clone(),
        });

        let mut watcher = SecretsWatcher::new(&figment).unwrap().unwrap();
        watcher.register(subscriber);
        let _handle = watcher.start();

        tokio::time::sleep(Duration::from_millis(30)).await;

        // First, write valid changed content to prove the subscriber works
        std::fs::write(&file_path, r#"{"key": "v2"}"#).unwrap();
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(
            call_count.load(std::sync::atomic::Ordering::Relaxed),
            1,
            "subscriber should be called for a valid change"
        );

        // Now write invalid UTF-8 to trigger a parse failure
        std::fs::write(&file_path, [0xFF, 0xFE, 0xFD]).unwrap();

        // Wait for several poll cycles — subscriber should NOT be notified again
        tokio::time::sleep(Duration::from_millis(300)).await;

        assert_eq!(
            call_count.load(std::sync::atomic::Ordering::Relaxed),
            1,
            "subscriber should not be called again when parse fails"
        );
    }

    #[test_log::test(tokio::test)]
    async fn test_watcher_recovers_after_parse_failure() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let file_path = tmp_dir.path().join("secrets.json");
        std::fs::write(&file_path, r#"{"key": "v1"}"#).unwrap();

        let figment = watcher_figment(&file_path, 50);

        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let tx = Arc::new(std::sync::Mutex::new(Some(tx)));

        struct NotifySubscriber {
            tx: Arc<std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
        }

        impl SecretsReloadable for NotifySubscriber {
            async fn reload_secrets(&self, _secrets: Option<&Figment>) {
                if let Some(tx) = self.tx.lock().unwrap().take() {
                    let _ = tx.send(());
                }
            }
        }

        let subscriber = Arc::new(NotifySubscriber { tx });

        let mut watcher = SecretsWatcher::new(&figment).unwrap().unwrap();
        watcher.register(subscriber);
        let _handle = watcher.start();

        tokio::time::sleep(Duration::from_millis(30)).await;

        // Write invalid data — parse fails, last_contents NOT updated
        std::fs::write(&file_path, [0xFF, 0xFE, 0xFD]).unwrap();
        tokio::time::sleep(Duration::from_millis(150)).await;

        // Write valid data — should trigger notification since last_contents still has "v1"
        std::fs::write(&file_path, r#"{"key": "v2"}"#).unwrap();

        let result = tokio::time::timeout(Duration::from_secs(5), rx).await;
        assert!(
            result.is_ok(),
            "subscriber should be notified after recovery from parse failure"
        );
    }
}
