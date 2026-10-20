// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

pub mod v1alpha;
pub mod v1beta;

use super::error;
use super::resource::{Address, AddressingStyle};
use super::secrets::ClientCredentials;
use super::service::Storage;
use crate::traits::error::Result;

impl Storage {
    /// Resolves the operator-specified versioning state for the endpoint
    /// serving `resource_address`: the per-endpoint `VERSIONING_ENABLED` secrets
    /// entry wins, then the backend-wide `s3.versioning_enabled` config, then
    /// the default (enabled — the legacy behavior). There is no runtime
    /// detection and no backend call; only the in-memory secrets map is read,
    /// so a hot-reload is picked up on the next call.
    ///
    /// This is the `EnumerateVersions` short-circuit, so it does not reject an
    /// endpoint that strict mode refuses to serve: that endpoint resolves to the
    /// default and `self.client(..)` rejects it. `IsVersioned` answers through
    /// [`Self::is_versioned_for`], which rejects it up front.
    pub(super) async fn versioning_enabled_for(&self, resource_address: &Address) -> bool {
        let (matched_secret, endpoint_override) = self.versioning_secrets_entry(resource_address).await;
        self.resolve_versioning_enabled(matched_secret, endpoint_override)
    }

    /// Resolves the `IsVersioned` answer for the endpoint serving
    /// `resource_address`, with the precedence of [`Self::versioning_enabled_for`].
    ///
    /// Errors with `not_found` when strict mode (`allow_unconfigured_endpoints =
    /// false`) is on and nothing configures the endpoint, since the backend would
    /// refuse to serve that address anyway.
    pub(super) async fn is_versioned_for(&self, resource_address: &Address) -> Result<bool> {
        let (matched_secret, endpoint_override) = self.versioning_secrets_entry(resource_address).await;
        // Same reasoning as `read_only_for`: in strict mode an endpoint absent from the
        // secrets map is one this backend refuses to serve, so reporting a versioning
        // state for it would answer for an address every real operation rejects with
        // `endpoint not configured`. Reject it the same way, without a backend call. A
        // backend-wide setting is a configured answer for every endpoint.
        if !matched_secret && !self.config.allow_unconfigured_endpoints && self.config.versioning_enabled.is_none() {
            return Err(error::Error::not_found(format!(
                "endpoint not configured: {}",
                resource_address.host().unwrap_or("unknown")
            ))
            .into());
        }

        Ok(self.resolve_versioning_enabled(matched_secret, endpoint_override))
    }

    /// Looks up the endpoint serving `resource_address` in the secrets map:
    /// whether it has an entry, and that entry's `VERSIONING_ENABLED`.
    async fn versioning_secrets_entry(&self, resource_address: &Address) -> (bool, Option<bool>) {
        let secrets_guard = self.secrets.read().await;
        match secrets_guard.as_ref().and_then(|secrets| {
            secrets
                .get(resource_address.config_key(AddressingStyle::Virtual))
                .or_else(|| secrets.get(resource_address.config_key(AddressingStyle::Path)))
        }) {
            Some(creds) => (true, creds.versioning_enabled),
            None => (false, None),
        }
    }

    /// Applies the precedence documented on [`Self::versioning_enabled_for`] to
    /// the result of [`Self::versioning_secrets_entry`].
    fn resolve_versioning_enabled(&self, matched_secret: bool, endpoint_override: Option<bool>) -> bool {
        if let Some(versioning_enabled) = endpoint_override.or(self.config.versioning_enabled) {
            return versioning_enabled;
        }

        // Same precedence as `read_only_for`: only an endpoint absent from the secrets
        // map is driven by env vars (that is the only case `Storage::create_client`
        // consults `ClientCredentials::from_env`), so the `VERSIONING_ENABLED` env var
        // is honored only then. Without this the env setting is parsed by `from_env`
        // and used to build the client, but never reflected in the answer this resolver
        // gives -- so `IsVersioned` would report a state the deployment did not configure.
        if !matched_secret && self.config.allow_unconfigured_endpoints {
            if let Ok(env_creds) = ClientCredentials::from_env() {
                if let Some(versioning_enabled) = env_creds.versioning_enabled {
                    return versioning_enabled;
                }
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use figment::providers::{Format, Toml};
    use figment::Figment;
    use serial_test::serial;

    use crate::s3::service::{Config, Storage};
    use crate::traits::fileobject::v1alpha::{FileObject, IsReadOnlyRequest};
    use crate::traits::versioning::v1alpha::{IsVersionedRequest, Versioning};
    use crate::traits::{Request, Result};

    /// Builds `Storage` the way an operator does: from a Figment TOML document,
    /// so the `[s3] versioning_enabled` field name and deserialization are
    /// exercised — not a struct literal.
    fn storage_from_toml(config: &str) -> Storage {
        let figment = Figment::new().merge(Toml::string(config));
        let config = Config::try_from(&figment).expect("config should deserialize");
        Storage::new(config, None).expect("storage construction should succeed")
    }

    /// Like [`storage_from_toml`], with a secrets map carrying per-endpoint entries.
    fn storage_from_toml_with_secrets(config: &str, secrets: crate::s3::secrets::Secrets) -> Storage {
        let figment = Figment::new().merge(Toml::string(config));
        let config = Config::try_from(&figment).expect("config should deserialize");
        Storage::new(config, Some(secrets)).expect("storage construction should succeed")
    }

    const ADDRESS: &str = "https://bucket.s3.us-west-2.amazonaws.com/";
    const OTHER_ADDRESS: &str = "https://other.s3.us-west-2.amazonaws.com/";

    /// Strict mode: only endpoints in the secrets map are served.
    const STRICT: &str = "[s3]\nallow_unconfigured_endpoints = false\n";

    /// Calls the public trait entry point, so a missing/miswired override can't
    /// hide behind the macro-generated default.
    async fn is_versioned(storage: &Storage, address: &str) -> Result<bool> {
        Versioning::is_versioned(
            storage,
            Request::new(IsVersionedRequest {
                resource_address: address.to_string(),
            }),
        )
        .await
        .map(|response| response.is_versioned)
    }

    async fn assert_reports(storage: &Storage, expected: bool) {
        assert_eq!(is_versioned(storage, ADDRESS).await.unwrap(), expected);
    }

    /// Secrets map with a single entry for [`ADDRESS`]'s endpoint.
    fn secrets_for_address(versioning_enabled: Option<bool>) -> crate::s3::secrets::Secrets {
        use std::collections::HashMap;

        use crate::s3::secrets::{ClientCredentials, Secrets};

        Secrets(HashMap::from([(
            "https://bucket.s3.us-west-2.amazonaws.com".to_string(),
            ClientCredentials {
                versioning_enabled,
                ..Default::default()
            },
        )]))
    }

    /// Runs `f` with the `VERSIONING_ENABLED` env var set to `value`, removing it
    /// afterwards. Callers must be `#[serial]`.
    async fn with_versioning_env<F, Fut, T>(value: &str, f: F) -> T
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = T>,
    {
        unsafe {
            std::env::set_var("VERSIONING_ENABLED", value);
        }
        let result = f().await;
        unsafe {
            std::env::remove_var("VERSIONING_ENABLED");
        }
        result
    }

    // Serial with the `VERSIONING_ENABLED` env tests: with no secrets/config, the
    // resolver falls back to the env var, so a concurrent env mutation could flip this.
    #[tokio::test]
    #[serial]
    async fn unset_config_reports_enabled() {
        let storage = storage_from_toml("[s3]\n");
        assert_reports(&storage, true).await;
    }

    #[tokio::test]
    async fn configured_true_reports_enabled() {
        let storage = storage_from_toml("[s3]\nversioning_enabled = true\n");
        assert_reports(&storage, true).await;
    }

    #[tokio::test]
    async fn configured_false_reports_disabled() {
        let storage = storage_from_toml("[s3]\nversioning_enabled = false\n");
        assert_reports(&storage, false).await;
    }

    #[tokio::test]
    async fn malformed_address_errors() {
        let storage = storage_from_toml("[s3]\n");
        assert!(is_versioned(&storage, "not a url").await.is_err());
    }

    /// Per-endpoint `VERSIONING_ENABLED` overrides the backend-wide scalar in
    /// both directions; endpoints without an override fall back to the scalar.
    // Serial: the non-overridden endpoint with the scalar unset falls back to the env.
    #[tokio::test]
    #[serial]
    async fn endpoint_override_wins_over_backend_default() {
        // Endpoint override (false) wins over the unset backend default.
        let storage = storage_from_toml_with_secrets("[s3]\n", secrets_for_address(Some(false)));
        assert_reports(&storage, false).await;
        // A different endpoint with no override falls back to the scalar.
        assert!(is_versioned(&storage, OTHER_ADDRESS).await.unwrap());

        // Endpoint override (true) wins over the backend-wide false.
        let storage =
            storage_from_toml_with_secrets("[s3]\nversioning_enabled = false\n", secrets_for_address(Some(true)));
        assert_reports(&storage, true).await;
        // ...while the non-overridden endpoint still reports the scalar (false).
        assert!(!is_versioned(&storage, OTHER_ADDRESS).await.unwrap());
    }

    /// In the unconfigured-endpoint mode (no secrets entry, no backend-wide
    /// scalar, `allow_unconfigured_endpoints`), the `VERSIONING_ENABLED` env var
    /// that `ClientCredentials::from_env` parses drives the answer.
    #[tokio::test]
    #[serial]
    async fn env_drives_answer_when_unconfigured() {
        let storage = storage_from_toml("[s3]\n");
        let result = with_versioning_env("false", || is_versioned(&storage, ADDRESS)).await;
        assert!(
            !result.unwrap(),
            "VERSIONING_ENABLED=false must disable versioning when nothing else is configured"
        );
    }

    /// An endpoint with a secrets entry is not driven by env vars, even when the
    /// entry leaves `VERSIONING_ENABLED` unset: the default (enabled) applies.
    #[tokio::test]
    #[serial]
    async fn env_ignored_for_endpoint_in_secrets() {
        let storage = storage_from_toml_with_secrets("[s3]\n", secrets_for_address(None));
        let result = with_versioning_env("false", || is_versioned(&storage, ADDRESS)).await;
        assert!(result.unwrap(), "a configured endpoint must ignore VERSIONING_ENABLED");
    }

    /// The backend-wide `s3.versioning_enabled` scalar wins over the env var.
    #[tokio::test]
    #[serial]
    async fn backend_default_wins_over_env() {
        let storage = storage_from_toml("[s3]\nversioning_enabled = true\n");
        let result = with_versioning_env("false", || is_versioned(&storage, ADDRESS)).await;
        assert!(
            result.unwrap(),
            "s3.versioning_enabled must win over VERSIONING_ENABLED"
        );
    }

    /// With `allow_unconfigured_endpoints = false` the env var is never consulted:
    /// it neither answers for an endpoint absent from the secrets map nor makes
    /// that endpoint one strict mode serves.
    #[tokio::test]
    #[serial]
    async fn env_ignored_when_unconfigured_endpoints_disallowed() {
        let storage = storage_from_toml(STRICT);
        let result = with_versioning_env("false", || is_versioned(&storage, ADDRESS)).await;
        let err =
            result.expect_err("strict mode must reject an unconfigured endpoint even with VERSIONING_ENABLED set");
        assert!(
            err.to_string().contains("endpoint not configured"),
            "unexpected error: {err}"
        );
    }

    /// Strict mode refuses to serve an endpoint absent from the secrets map, so
    /// IsVersioned rejects it with exactly the error IsReadOnly gives for the
    /// same address, and without a backend call. Covers both an absent secrets
    /// map and a map that lacks this endpoint.
    #[tokio::test]
    async fn strict_mode_rejects_unconfigured_endpoint_like_is_read_only() {
        for storage in [
            storage_from_toml(STRICT),
            storage_from_toml_with_secrets(STRICT, secrets_for_address(Some(true))),
        ] {
            let versioned_err = is_versioned(&storage, OTHER_ADDRESS)
                .await
                .expect_err("unconfigured endpoint must be rejected");
            let read_only_err = FileObject::is_read_only(
                &storage,
                Request::new(IsReadOnlyRequest {
                    resource_address: OTHER_ADDRESS.to_string(),
                }),
            )
            .await
            .expect_err("IsReadOnly rejects the same endpoint");

            assert_eq!(versioned_err.tonic_status().code(), tonic::Code::NotFound);
            assert_eq!(versioned_err.tonic_status().code(), read_only_err.tonic_status().code());
            assert_eq!(versioned_err.to_string(), read_only_err.to_string());
            assert!(
                versioned_err.to_string().contains("endpoint not configured"),
                "unexpected error: {versioned_err}"
            );
        }
    }

    /// In strict mode an endpoint with a secrets entry is served, so it gets its
    /// answer: the entry's `VERSIONING_ENABLED`, else the default (enabled).
    #[tokio::test]
    async fn strict_mode_answers_for_configured_endpoint() {
        let storage = storage_from_toml_with_secrets(STRICT, secrets_for_address(Some(false)));
        assert_reports(&storage, false).await;

        let storage = storage_from_toml_with_secrets(STRICT, secrets_for_address(None));
        assert_reports(&storage, true).await;
    }

    /// Strict mode with a backend-wide `s3.versioning_enabled` is a configured
    /// answer for every endpoint, so it is reported rather than rejected.
    #[tokio::test]
    async fn strict_mode_answers_when_backend_wide_setting_exists() {
        let storage = storage_from_toml("[s3]\nallow_unconfigured_endpoints = false\nversioning_enabled = false\n");
        assert_reports(&storage, false).await;
        assert!(!is_versioned(&storage, OTHER_ADDRESS).await.unwrap());
    }
}
