// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{
    collections::HashMap,
    fmt::Debug,
    ops::{Deref, DerefMut},
    sync::Arc,
};

use figment::Figment;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Secrets for the Azure service.
///
/// The secrets struct is a map of URL components to client credentials.
///
/// The keys must be the authority of the URL to match the incoming request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Secrets(pub HashMap<String, ClientCredentials>);

impl Deref for Secrets {
    type Target = HashMap<String, ClientCredentials>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Secrets {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Secrets {
    pub const CONFIG_KEY: &str = "storage-service-azure";
}

impl TryFrom<&Figment> for Secrets {
    type Error = figment::Error;

    fn try_from(figment: &Figment) -> Result<Self, Self::Error> {
        figment.focus(Self::CONFIG_KEY).extract()
    }
}

/// Client credentials for the Azure service.
///
/// These credentials are used to authenticate the client to the Azure service.
#[derive(Default, Clone, Deserialize, Serialize)]
pub struct ClientCredentials {
    /// The storage account for the client. This value can be inferred from the incoming request, but can be overridden here.
    #[serde(rename = "AZURE_STORAGE_ACCOUNT")]
    pub storage_account: Option<String>,
    /// The storage key for the client.
    #[serde(rename = "AZURE_STORAGE_KEY")]
    pub storage_key: Option<String>,
    /// The Front Door URL for the client. If this is set, the client will issue presigned URLs
    /// through Front Door instead of directly to the storage account.
    ///
    /// This can be either:
    /// - A hostname only: `frontdoor.azurefd.net`
    /// - A full URL with optional path prefix: `https://frontdoor.azurefd.net/storage-prefix`
    ///
    /// When a path prefix is specified, it will be prepended to the blob path in presigned URLs.
    /// The Front Door/proxy must be configured to strip this prefix before forwarding to storage.
    #[serde(rename = "AZURE_FRONT_DOOR_URL")]
    pub front_door_url: Option<String>,
    /// Whether object versioning is enabled on this endpoint's account. When
    /// set, overrides the backend-wide `azure.versioning_enabled` config for
    /// this endpoint; when unset, the backend-wide value (or enabled) applies.
    #[serde(rename = "VERSIONING_ENABLED")]
    pub versioning_enabled: Option<bool>,
    /// Whether this endpoint's account is read-only. When set, overrides the
    /// backend-wide `azure.read_only` config for this endpoint; when unset, the
    /// backend-wide value (or writable) applies.
    #[serde(rename = "READ_ONLY")]
    pub read_only: Option<bool>,
    /// Azure AD tenant ID for service principal authentication.
    #[serde(rename = "AZURE_TENANT_ID")]
    pub tenant_id: Option<String>,
    /// Azure AD client ID for service principal authentication.
    #[serde(rename = "AZURE_CLIENT_ID")]
    pub client_id: Option<String>,
    /// Azure AD client secret for service principal authentication.
    #[serde(rename = "AZURE_CLIENT_SECRET")]
    pub client_secret: Option<String>,
    /// Azure AD authority host. Defaults to `https://login.microsoftonline.com`.
    #[serde(rename = "AZURE_AUTHORITY_HOST")]
    pub authority_host: Option<String>,
    /// Client credential override. The above storage account secrets will be ignored, if specified.
    #[serde(skip)]
    pub credential_override: Option<CredentialOverride>,
}

impl ClientCredentials {
    pub const DEFAULT_AUTHORITY_HOST: &str = "https://login.microsoftonline.com";

    pub fn credential_hash(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.storage_account.as_deref().unwrap_or("").as_bytes());
        hasher.update(self.storage_key.as_deref().unwrap_or("").as_bytes());
        hasher.update(self.front_door_url.as_deref().unwrap_or("").as_bytes());
        hasher.update(self.tenant_id.as_deref().unwrap_or("").as_bytes());
        hasher.update(self.client_id.as_deref().unwrap_or("").as_bytes());
        hasher.update(self.client_secret.as_deref().unwrap_or("").as_bytes());
        hasher.update(self.authority_host.as_deref().unwrap_or("").as_bytes());
        hex::encode(hasher.finalize())
    }
}

impl Debug for ClientCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientConfig")
            .field("storage_account", &self.storage_account.is_some())
            .field("storage_key", &self.storage_key.is_some())
            .field("front_door_url", &self.front_door_url)
            .field("tenant_id", &self.tenant_id.is_some())
            .field("client_id", &self.client_id.is_some())
            .field("client_secret", &self.client_secret.is_some())
            .field("authority_host", &self.authority_host)
            .field("credential_override", &self.credential_override)
            .finish()
    }
}

/// Client credentials override for the Azure service.
///
/// Allows to customize the acquisition of credentials for a storage account.
#[derive(Default, Clone)]
pub enum CredentialOverride {
    /// Use anonymous authentication.
    #[default]
    Anonymous,
    /// Use OAuth token authentication.
    TokenCredential(Arc<dyn azure_core::auth::TokenCredential>),
}

impl std::fmt::Debug for CredentialOverride {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Anonymous => f.debug_tuple("Anonymous").finish(),
            Self::TokenCredential(_) => f.debug_tuple("TokenCredential").finish_non_exhaustive(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use figment::{
        providers::{Format, Toml},
        Jail,
    };
    use std::collections::HashMap;

    #[test_log::test()]
    fn test_secrets_basic_parsing() {
        Jail::expect_with(|jail| {
            let data = r#"
                {
                    "storage-service-azure": {
                        "mystorageaccount.blob.core.windows.net": {
                            "AZURE_STORAGE_ACCOUNT": "mystorageaccount",
                            "AZURE_STORAGE_KEY": "dGVzdC1zdG9yYWdlLWtleS1mb3ItdGVzdGluZw==",
                            "AZURE_FRONT_DOOR_URL": "basic-parsing-abc123.z01.azurefd.net"
                        }
                    }
                }
            "#;

            jail.create_file("secrets.json", data).unwrap();
            jail.create_file(
                "config.toml",
                r#"
                [secrets]
                secrets_file = "secrets.json"
                "#,
            )
            .unwrap();

            let figment = Figment::new().merge(Toml::file("config.toml"));
            let secrets_figment = storage_core_util::secrets::from_figment(&figment).unwrap().unwrap();
            let secrets: Secrets = secrets_figment.focus("storage-service-azure").extract().unwrap();

            assert_eq!(secrets.len(), 1);
            assert!(secrets.contains_key("mystorageaccount.blob.core.windows.net"));

            let credentials = &secrets["mystorageaccount.blob.core.windows.net"];
            assert_eq!(credentials.storage_account, Some("mystorageaccount".to_string()));
            assert_eq!(
                credentials.storage_key,
                Some("dGVzdC1zdG9yYWdlLWtleS1mb3ItdGVzdGluZw==".to_string())
            );
            assert_eq!(
                credentials.front_door_url,
                Some("basic-parsing-abc123.z01.azurefd.net".to_string())
            );

            Ok(())
        });
    }

    #[test_log::test()]
    fn test_secrets_multiple_entries() {
        Jail::expect_with(|jail| {
            let data = r#"
                {
                    "storage-service-azure": {
                        "account1.blob.core.windows.net": {
                            "AZURE_STORAGE_ACCOUNT": "account1",
                            "AZURE_STORAGE_KEY": "dGVzdC1rZXktMQ==",
                            "AZURE_FRONT_DOOR_URL": "multi-entry-fd1-xyzabc.b02.azurefd.net"
                        },
                        "account2.blob.core.windows.net": {
                            "AZURE_STORAGE_ACCOUNT": "account2",
                            "AZURE_STORAGE_KEY": "dGVzdC1rZXktMg==",
                            "AZURE_FRONT_DOOR_URL": "multi-entry-fd2-def456.c03.azurefd.net"
                        },
                        "customdomain.com": {
                            "AZURE_STORAGE_ACCOUNT": "customaccount",
                            "AZURE_STORAGE_KEY": "dGVzdC1rZXktY3VzdG9t",
                            "AZURE_FRONT_DOOR_URL": "multi-entry-fd3-ghi789.d04.azurefd.net"
                        }
                    }
                }
            "#;

            jail.create_file("secrets.json", data).unwrap();
            jail.create_file(
                "config.toml",
                r#"
                [secrets]
                secrets_file = "secrets.json"
                "#,
            )
            .unwrap();

            let figment = Figment::new().merge(Toml::file("config.toml"));
            let secrets_figment = storage_core_util::secrets::from_figment(&figment).unwrap().unwrap();
            let secrets: Secrets = secrets_figment.focus("storage-service-azure").extract().unwrap();

            assert_eq!(secrets.len(), 3);

            // Test first entry
            let creds1 = &secrets["account1.blob.core.windows.net"];
            assert_eq!(creds1.storage_account, Some("account1".to_string()));
            assert_eq!(creds1.storage_key, Some("dGVzdC1rZXktMQ==".to_string()));
            assert_eq!(
                creds1.front_door_url,
                Some("multi-entry-fd1-xyzabc.b02.azurefd.net".to_string())
            );

            // Test second entry
            let creds2 = &secrets["account2.blob.core.windows.net"];
            assert_eq!(creds2.storage_account, Some("account2".to_string()));
            assert_eq!(creds2.storage_key, Some("dGVzdC1rZXktMg==".to_string()));
            assert_eq!(
                creds2.front_door_url,
                Some("multi-entry-fd2-def456.c03.azurefd.net".to_string())
            );

            // Test third entry with custom domain
            let creds3 = &secrets["customdomain.com"];
            assert_eq!(creds3.storage_account, Some("customaccount".to_string()));
            assert_eq!(creds3.storage_key, Some("dGVzdC1rZXktY3VzdG9t".to_string()));
            assert_eq!(
                creds3.front_door_url,
                Some("multi-entry-fd3-ghi789.d04.azurefd.net".to_string())
            );

            Ok(())
        });
    }

    #[test_log::test()]
    fn test_secrets_partial_credentials() {
        Jail::expect_with(|jail| {
            let data = r#"
                {
                    "storage-service-azure": {
                        "partial.blob.core.windows.net": {
                            "AZURE_STORAGE_ACCOUNT": "partialaccount"
                        },
                        "keyonly.blob.core.windows.net": {
                            "AZURE_STORAGE_KEY": "dGVzdC1rZXktb25seQ=="
                        },
                        "frontdoor.blob.core.windows.net": {
                            "AZURE_FRONT_DOOR_URL": "partial-creds-only-jkl012.e05.azurefd.net"
                        }
                    }
                }
            "#;

            jail.create_file("secrets.json", data).unwrap();
            jail.create_file(
                "config.toml",
                r#"
                [secrets]
                secrets_file = "secrets.json"
                "#,
            )
            .unwrap();

            let figment = Figment::new().merge(Toml::file("config.toml"));
            let secrets_figment = storage_core_util::secrets::from_figment(&figment).unwrap().unwrap();
            let secrets: Secrets = secrets_figment.focus("storage-service-azure").extract().unwrap();

            assert_eq!(secrets.len(), 3);

            // Test partial credentials - account only
            let creds1 = &secrets["partial.blob.core.windows.net"];
            assert_eq!(creds1.storage_account, Some("partialaccount".to_string()));
            assert!(creds1.storage_key.is_none());
            assert!(creds1.front_door_url.is_none());

            // Test partial credentials - key only
            let creds2 = &secrets["keyonly.blob.core.windows.net"];
            assert!(creds2.storage_account.is_none());
            assert_eq!(creds2.storage_key, Some("dGVzdC1rZXktb25seQ==".to_string()));
            assert!(creds2.front_door_url.is_none());

            // Test partial credentials - front door only
            let creds3 = &secrets["frontdoor.blob.core.windows.net"];
            assert!(creds3.storage_account.is_none());
            assert!(creds3.storage_key.is_none());
            assert_eq!(
                creds3.front_door_url,
                Some("partial-creds-only-jkl012.e05.azurefd.net".to_string())
            );

            Ok(())
        });
    }

    #[test_log::test()]
    fn test_client_credentials_credential_hash() {
        let credentials1 = ClientCredentials {
            storage_account: Some("testaccount".to_string()),
            storage_key: Some("dGVzdC1rZXk=".to_string()),
            ..Default::default()
        };

        let credentials2 = ClientCredentials {
            storage_account: Some("testaccount".to_string()),
            storage_key: Some("dGVzdC1rZXk=".to_string()),
            ..Default::default()
        };

        let credentials3 = ClientCredentials {
            storage_account: Some("differentaccount".to_string()),
            storage_key: Some("dGVzdC1rZXk=".to_string()),
            ..Default::default()
        };

        // Identical credentials should produce same hash
        assert_eq!(credentials1.credential_hash(), credentials2.credential_hash());

        // Different credentials should produce different hash
        assert_ne!(credentials1.credential_hash(), credentials3.credential_hash());

        // Hash should be consistent
        assert_eq!(credentials1.credential_hash(), credentials1.credential_hash());
    }

    #[test_log::test()]
    fn test_client_credentials_credential_hash_with_none_values() {
        let credentials1 = ClientCredentials::default();

        let credentials2 = ClientCredentials {
            storage_account: Some("".to_string()), // Empty string
            storage_key: Some("".to_string()),     // Empty string
            ..Default::default()
        };

        let hash1 = credentials1.credential_hash();
        let hash2 = credentials2.credential_hash();

        // None values and empty strings should produce the same hash
        assert_eq!(hash1, hash2);
    }

    #[test_log::test()]
    fn test_client_credentials_credential_hash_includes_all_fields() {
        let base = ClientCredentials {
            storage_account: Some("testaccount".to_string()),
            storage_key: Some("dGVzdC1rZXk=".to_string()),
            ..Default::default()
        };

        let with_front_door = ClientCredentials {
            front_door_url: Some("frontdoor.azurefd.net".to_string()),
            ..base.clone()
        };

        let with_authority = ClientCredentials {
            authority_host: Some("https://login.microsoftonline.us".to_string()),
            ..base.clone()
        };

        // All fields contribute to the hash
        assert_ne!(base.credential_hash(), with_front_door.credential_hash());
        assert_ne!(base.credential_hash(), with_authority.credential_hash());
    }

    #[test_log::test()]
    fn test_client_credentials_debug_format() {
        let credentials = ClientCredentials {
            storage_account: Some("testaccount".to_string()),
            storage_key: Some("dGVzdC1rZXk=".to_string()),
            front_door_url: Some("debug-format-pqr678.g07.azurefd.net".to_string()),
            tenant_id: Some("test-tenant-id".to_string()),
            client_id: Some("test-client-id".to_string()),
            client_secret: Some("test-client-secret".to_string()),
            authority_host: Some("https://login.microsoftonline.com".to_string()),
            versioning_enabled: None,
            read_only: None,
            credential_override: None,
        };

        let debug_str = format!("{:?}", credentials);

        // Should not contain actual secret values
        assert!(!debug_str.contains("testaccount"));
        assert!(!debug_str.contains("dGVzdC1rZXk="));
        assert!(!debug_str.contains("test-tenant-id"));
        assert!(!debug_str.contains("test-client-id"));
        assert!(!debug_str.contains("test-client-secret"));
        assert!(debug_str.contains("debug-format-pqr678.g07.azurefd.net"));

        // Should contain boolean indicators for presence of secrets
        assert!(debug_str.contains("storage_account: true"));
        assert!(debug_str.contains("storage_key: true"));
        assert!(debug_str.contains("tenant_id: true"));
        assert!(debug_str.contains("client_id: true"));
        assert!(debug_str.contains("client_secret: true"));
    }

    #[test_log::test()]
    fn test_client_credentials_debug_format_with_none_values() {
        let credentials = ClientCredentials::default();

        let debug_str = format!("{:?}", credentials);

        // Should contain boolean indicators for absence of secrets
        assert!(debug_str.contains("storage_account: false"));
        assert!(debug_str.contains("storage_key: false"));
        assert!(debug_str.contains("tenant_id: false"));
        assert!(debug_str.contains("client_id: false"));
        assert!(debug_str.contains("client_secret: false"));
    }

    #[test_log::test()]
    fn test_secrets_deref_traits() {
        let mut secrets = Secrets(HashMap::new());
        let credentials = ClientCredentials {
            storage_account: Some("testaccount".to_string()),
            storage_key: Some("dGVzdC1rZXk=".to_string()),
            ..Default::default()
        };

        // Test Deref
        assert_eq!(secrets.len(), 0);
        secrets.insert("test.blob.core.windows.net".to_string(), credentials.clone());
        assert_eq!(secrets.len(), 1);
        assert!(secrets.contains_key("test.blob.core.windows.net"));

        // Test DerefMut
        secrets.get_mut("test.blob.core.windows.net").unwrap().storage_account = Some("updatedaccount".to_string());
        assert_eq!(
            secrets["test.blob.core.windows.net"].storage_account,
            Some("updatedaccount".to_string())
        );
    }

    #[test_log::test()]
    fn test_secrets_config_key() {
        assert_eq!(Secrets::CONFIG_KEY, "storage-service-azure");
    }

    #[test_log::test()]
    fn test_client_credentials_default() {
        let credentials = ClientCredentials::default();

        assert!(credentials.storage_account.is_none());
        assert!(credentials.storage_key.is_none());
    }

    #[test_log::test()]
    fn test_secrets_empty_config() {
        Jail::expect_with(|jail| {
            let data = r#"
                {
                    "storage-service-azure": {}
                }
            "#;

            jail.create_file("secrets.json", data).unwrap();
            jail.create_file(
                "config.toml",
                r#"
                [secrets]
                secrets_file = "secrets.json"
                "#,
            )
            .unwrap();

            let figment = Figment::new().merge(Toml::file("config.toml"));
            let secrets_figment = storage_core_util::secrets::from_figment(&figment).unwrap().unwrap();
            let secrets: Secrets = secrets_figment.focus("storage-service-azure").extract().unwrap();

            assert_eq!(secrets.len(), 0);

            Ok(())
        });
    }

    #[test_log::test()]
    fn test_secrets_missing_config_key() {
        Jail::expect_with(|jail| {
            let data = r#"
                {
                    "some-other-service": {
                        "test.blob.core.windows.net": {
                            "AZURE_STORAGE_ACCOUNT": "testaccount",
                            "AZURE_STORAGE_KEY": "dGVzdC1rZXk="
                        }
                    }
                }
            "#;

            jail.create_file("secrets.json", data).unwrap();
            jail.create_file(
                "config.toml",
                r#"
                [secrets]
                secrets_file = "secrets.json"
                "#,
            )
            .unwrap();

            let figment = Figment::new().merge(Toml::file("config.toml"));
            let secrets_figment = storage_core_util::secrets::from_figment(&figment).unwrap().unwrap();
            let secrets: Secrets = secrets_figment.focus("storage-service-azure").extract().unwrap();

            assert_eq!(secrets.len(), 0);

            Ok(())
        });
    }

    #[test_log::test()]
    fn test_secrets_invalid_json() {
        Jail::expect_with(|jail| {
            jail.create_file("secrets.json", "This is not valid JSON").unwrap();
            jail.create_file(
                "config.toml",
                r#"
                [secrets]
                secrets_file = "secrets.json"
                "#,
            )
            .unwrap();

            let figment = Figment::new().merge(Toml::file("config.toml"));
            let secrets_figment = storage_core_util::secrets::from_figment(&figment).unwrap().unwrap();

            // This should fail when trying to extract the secrets
            assert!(secrets_figment
                .focus("storage-service-azure")
                .extract::<Secrets>()
                .is_err());

            Ok(())
        });
    }

    #[test_log::test()]
    fn test_secrets_file_not_found() {
        Jail::expect_with(|jail| {
            jail.create_file(
                "config.toml",
                r#"
                [secrets]
                secrets_file = "nonexistent.json"
                "#,
            )
            .unwrap();

            let figment = Figment::new().merge(Toml::file("config.toml"));
            let secrets_figment = storage_core_util::secrets::from_figment(&figment).unwrap();

            // Should return None when file doesn't exist
            assert!(secrets_figment.is_none());

            Ok(())
        });
    }

    #[test_log::test()]
    fn test_client_credentials_serialization() {
        let credentials = ClientCredentials {
            storage_account: Some("testaccount".to_string()),
            storage_key: Some("dGVzdC1rZXk=".to_string()),
            front_door_url: Some("serialization-stu901.h08.azurefd.net".to_string()),
            ..Default::default()
        };

        // Test serialization
        let serialized = serde_json::to_string(&credentials).unwrap();
        assert!(serialized.contains("testaccount"));
        assert!(serialized.contains("dGVzdC1rZXk="));
        assert!(serialized.contains("serialization-stu901.h08.azurefd.net"));

        // Test deserialization
        let deserialized: ClientCredentials = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized.storage_account, credentials.storage_account);
        assert_eq!(deserialized.storage_key, credentials.storage_key);
    }

    #[test_log::test()]
    fn test_client_credentials_clone() {
        let credentials1 = ClientCredentials {
            storage_account: Some("testaccount".to_string()),
            storage_key: Some("dGVzdC1rZXk=".to_string()),
            front_door_url: Some("clone-test-vwx234.i09.azurefd.net".to_string()),
            ..Default::default()
        };

        let credentials2 = credentials1.clone();

        assert_eq!(credentials1.storage_account, credentials2.storage_account);
        assert_eq!(credentials1.storage_key, credentials2.storage_key);
        assert_eq!(credentials1.front_door_url, credentials2.front_door_url);
        assert_eq!(credentials1.credential_hash(), credentials2.credential_hash());
    }

    #[test_log::test()]
    fn test_secrets_service_principal_parsing() {
        Jail::expect_with(|jail| {
            let data = r#"
                {
                    "storage-service-azure": {
                        "mystorageaccount.blob.core.windows.net": {
                            "AZURE_STORAGE_ACCOUNT": "mystorageaccount",
                            "AZURE_TENANT_ID": "tenant-123",
                            "AZURE_CLIENT_ID": "client-456",
                            "AZURE_CLIENT_SECRET": "secret-789",
                            "AZURE_AUTHORITY_HOST": "https://login.microsoftonline.us"
                        }
                    }
                }
            "#;

            jail.create_file("secrets.json", data).unwrap();
            jail.create_file(
                "config.toml",
                r#"
                [secrets]
                secrets_file = "secrets.json"
                "#,
            )
            .unwrap();

            let figment = Figment::new().merge(Toml::file("config.toml"));
            let secrets_figment = storage_core_util::secrets::from_figment(&figment).unwrap().unwrap();
            let secrets: Secrets = secrets_figment.focus("storage-service-azure").extract().unwrap();

            let credentials = &secrets["mystorageaccount.blob.core.windows.net"];
            assert_eq!(credentials.storage_account, Some("mystorageaccount".to_string()));
            assert!(credentials.storage_key.is_none());
            assert_eq!(credentials.tenant_id, Some("tenant-123".to_string()));
            assert_eq!(credentials.client_id, Some("client-456".to_string()));
            assert_eq!(credentials.client_secret, Some("secret-789".to_string()));
            assert_eq!(
                credentials.authority_host,
                Some("https://login.microsoftonline.us".to_string())
            );

            Ok(())
        });
    }

    #[test_log::test()]
    fn test_credential_hash_differs_with_sp_fields() {
        let creds_key = ClientCredentials {
            storage_key: Some("key".to_string()),
            ..Default::default()
        };

        let creds_sp = ClientCredentials {
            tenant_id: Some("tenant".to_string()),
            client_id: Some("client".to_string()),
            client_secret: Some("secret".to_string()),
            ..Default::default()
        };

        assert_ne!(creds_key.credential_hash(), creds_sp.credential_hash());
    }

    #[test_log::test()]
    fn test_credential_override_debug_format_anonymous() {
        let credential = CredentialOverride::Anonymous;
        let debug_str = format!("{:?}", credential);
        assert_eq!(debug_str, "Anonymous");
    }

    #[test_log::test()]
    fn test_credential_override_debug_format_token() {
        #[derive(Debug)]
        struct MockTokenCredential;

        #[async_trait::async_trait]
        impl azure_core::auth::TokenCredential for MockTokenCredential {
            async fn get_token(&self, _scopes: &[&str]) -> azure_core::Result<azure_core::auth::AccessToken> {
                unimplemented!()
            }

            async fn clear_cache(&self) -> azure_core::Result<()> {
                unimplemented!()
            }
        }

        let credential = CredentialOverride::TokenCredential(Arc::new(MockTokenCredential));
        let debug_str = format!("{:?}", credential);
        assert_eq!(debug_str, "TokenCredential(..)");
    }
}
