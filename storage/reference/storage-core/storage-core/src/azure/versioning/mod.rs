// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

pub mod v1alpha;
pub mod v1beta;

use url::Position;

use super::error::Error;
use super::resource::Address;
use super::service::Storage;
use crate::traits::error::Result;

impl Storage {
    /// Resolves the operator-specified versioning state for the endpoint
    /// serving `resource_address`: the per-endpoint `VERSIONING_ENABLED` secrets
    /// entry wins, then the backend-wide `azure.versioning_enabled` config, then
    /// the default (enabled — the legacy behavior). There is no runtime
    /// detection and no backend call; only the in-memory secrets map is read,
    /// so a hot-reload is picked up on the next call.
    ///
    /// This is the `EnumerateVersions` short-circuit, so it does not reject an
    /// endpoint that strict mode refuses to serve: that endpoint resolves to the
    /// default and client resolution rejects it. `IsVersioned` answers through
    /// [`Self::is_versioned_for`], which rejects it up front.
    pub(super) async fn versioning_enabled_for(&self, resource_address: &Address) -> bool {
        let (_, endpoint_override) = self.versioning_secrets_entry(resource_address).await;
        endpoint_override.or(self.config.versioning_enabled).unwrap_or(true)
    }

    /// Resolves the `IsVersioned` answer for the endpoint serving
    /// `resource_address`, with the precedence of [`Self::versioning_enabled_for`].
    ///
    /// Errors with `not_found` when strict mode (`allow_unconfigured_endpoints =
    /// false`) is on and nothing configures the endpoint, since the backend would
    /// refuse to serve that address anyway.
    pub(super) async fn is_versioned_for(&self, resource_address: &Address) -> Result<bool> {
        let lookup_key = &resource_address.resource_address[..Position::BeforePath];
        let (matched_secret, endpoint_override) = self.versioning_secrets_entry(resource_address).await;

        // Same reasoning as `read_only_for`: in strict mode an endpoint absent from the
        // secrets map is one this backend refuses to serve, so answering a versioning
        // state for it would describe an address every real operation rejects with
        // `endpoint not configured`.
        if !matched_secret && !self.config.allow_unconfigured_endpoints && self.config.versioning_enabled.is_none() {
            return Err(Error::not_found(format!("endpoint not configured: {lookup_key}")).into());
        }

        Ok(endpoint_override.or(self.config.versioning_enabled).unwrap_or(true))
    }

    /// Looks up the endpoint serving `resource_address` in the secrets map:
    /// whether it has an entry, and that entry's `VERSIONING_ENABLED`.
    async fn versioning_secrets_entry(&self, resource_address: &Address) -> (bool, Option<bool>) {
        let lookup_key = &resource_address.resource_address[..Position::BeforePath];
        let secrets_guard = self.secrets.read().await;
        match secrets_guard.as_ref().and_then(|secrets| secrets.get(lookup_key)) {
            Some(creds) => (true, creds.versioning_enabled),
            None => (false, None),
        }
    }
}

#[cfg(test)]
mod tests {
    use figment::providers::{Format, Toml};
    use figment::Figment;
    use serial_test::serial;

    use crate::azure::service::{Config, Storage};
    use crate::traits::fileobject::v1alpha::{FileObject, IsReadOnlyRequest};
    use crate::traits::versioning::v1alpha::{IsVersionedRequest, Versioning};
    use crate::traits::{Request, Result};

    /// Builds `Storage` the way an operator does: from a Figment TOML document,
    /// so the `[azure] versioning_enabled` field name and deserialization are
    /// exercised — not a struct literal.
    fn storage_from_toml(config: &str) -> Storage {
        let figment = Figment::new().merge(Toml::string(config));
        let config = Config::try_from(&figment).expect("config should deserialize");
        Storage::new(config, None).expect("storage construction should succeed")
    }

    /// Like [`storage_from_toml`], with a secrets map carrying per-endpoint entries.
    fn storage_from_toml_with_secrets(config: &str, secrets: crate::azure::secrets::Secrets) -> Storage {
        let figment = Figment::new().merge(Toml::string(config));
        let config = Config::try_from(&figment).expect("config should deserialize");
        Storage::new(config, Some(secrets)).expect("storage construction should succeed")
    }

    const ADDRESS: &str = "https://account.blob.core.windows.net/container/";
    const OTHER_ADDRESS: &str = "https://other.blob.core.windows.net/container/";

    /// Strict mode: only endpoints in the secrets map are served.
    const STRICT: &str = "[azure]\nallow_unconfigured_endpoints = false\n";

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
    fn secrets_for_address(versioning_enabled: Option<bool>) -> crate::azure::secrets::Secrets {
        use std::collections::HashMap;

        use crate::azure::secrets::{ClientCredentials, Secrets};

        Secrets(HashMap::from([(
            "https://account.blob.core.windows.net".to_string(),
            ClientCredentials {
                versioning_enabled,
                ..Default::default()
            },
        )]))
    }

    #[tokio::test]
    async fn unset_config_reports_enabled() {
        let storage = storage_from_toml("[azure]\n");
        assert_reports(&storage, true).await;
    }

    #[tokio::test]
    async fn configured_true_reports_enabled() {
        let storage = storage_from_toml("[azure]\nversioning_enabled = true\n");
        assert_reports(&storage, true).await;
    }

    #[tokio::test]
    async fn configured_false_reports_disabled() {
        let storage = storage_from_toml("[azure]\nversioning_enabled = false\n");
        assert_reports(&storage, false).await;
    }

    #[tokio::test]
    async fn malformed_address_errors() {
        let storage = storage_from_toml("[azure]\n");
        assert!(is_versioned(&storage, "not a url").await.is_err());
    }

    /// Per-endpoint `VERSIONING_ENABLED` overrides the backend-wide scalar in
    /// both directions; endpoints without an override fall back to the scalar.
    #[tokio::test]
    async fn endpoint_override_wins_over_backend_default() {
        // Endpoint override (false) wins over the unset backend default.
        let storage = storage_from_toml_with_secrets("[azure]\n", secrets_for_address(Some(false)));
        assert_reports(&storage, false).await;
        // A different endpoint with no override falls back to the scalar.
        assert!(is_versioned(&storage, OTHER_ADDRESS).await.unwrap());

        // Endpoint override (true) wins over the backend-wide false.
        let storage =
            storage_from_toml_with_secrets("[azure]\nversioning_enabled = false\n", secrets_for_address(Some(true)));
        assert_reports(&storage, true).await;
        // ...while the non-overridden endpoint still reports the scalar (false).
        assert!(!is_versioned(&storage, OTHER_ADDRESS).await.unwrap());
    }

    /// The `VERSIONING_ENABLED` environment variable fallback is S3-only
    /// (docs/secrets.md): Azure ignores it even for an unconfigured endpoint.
    #[tokio::test]
    #[serial]
    async fn env_var_is_ignored() {
        let storage = storage_from_toml("[azure]\n");
        unsafe {
            std::env::set_var("VERSIONING_ENABLED", "false");
        }
        let result = is_versioned(&storage, ADDRESS).await;
        unsafe {
            std::env::remove_var("VERSIONING_ENABLED");
        }
        assert!(
            result.unwrap(),
            "Azure must ignore the VERSIONING_ENABLED environment variable"
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

    /// Strict mode with a backend-wide `azure.versioning_enabled` is a configured
    /// answer for every endpoint, so it is reported rather than rejected.
    #[tokio::test]
    async fn strict_mode_answers_when_backend_wide_setting_exists() {
        let storage = storage_from_toml("[azure]\nallow_unconfigured_endpoints = false\nversioning_enabled = false\n");
        assert_reports(&storage, false).await;
        assert!(!is_versioned(&storage, OTHER_ADDRESS).await.unwrap());
    }
}
