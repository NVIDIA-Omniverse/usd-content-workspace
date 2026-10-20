// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::{
    collections::HashMap,
    fmt::Debug,
    ops::{Deref, DerefMut},
    sync::Arc,
    time::Duration,
};

use aws_sdk_s3::config::ProvideCredentials;
use figment::Figment;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

use super::{
    error::Error,
    resource::{Address, AddressingStyle},
};

/// Secrets for the S3 service.
///
/// The secrets struct is a map of URL components to client credentials.
///
/// The keys can be in one of the following formats:
/// - Authority only (e.g. <mybucket.example.com>)
///   - In this case, the scheme will be assumed to be https and the addressing style will be virtual unless overridden.
/// - Authority with scheme (e.g. <https://mybucket.example.com>)
///   - In this case, the addressing style will be virtual unless overridden.
/// - Authority with scheme and path (e.g. <https://mybucket.example.com/path>)
///   - In this case, the addressing style will be path unless overridden.
#[derive(Debug, Clone, Deserialize, Serialize)]
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
    pub const CONFIG_KEY: &str = "storage-service-s3";
}

impl TryFrom<&Figment> for Secrets {
    type Error = figment::Error;

    fn try_from(figment: &Figment) -> Result<Self, Self::Error> {
        figment.focus(Self::CONFIG_KEY).extract()
    }
}

/// Client credentials for the S3 service.
///
/// These credentials are used to authenticate the client to the S3 service.
/// All fields are optional, and can be inferred in some cases, but any value specified here will always take
/// precedence over the inferred value.
///
/// If both secrets and role ARN are specified, the secrets will be used in the source provider,
/// otherwise IRSA will be used in the source provider, before assuming the role.
#[derive(Default, Clone, Deserialize, Serialize, PartialEq)]
pub struct ClientCredentials {
    /// The access key ID for the client.
    #[serde(rename = "AWS_ACCESS_KEY_ID")]
    pub access_key_id: Option<String>,
    /// The secret access key for the client.
    #[serde(rename = "AWS_SECRET_ACCESS_KEY")]
    pub secret_access_key: Option<String>,
    /// The session token for the client.
    #[serde(rename = "AWS_SESSION_TOKEN")]
    pub session_token: Option<String>,
    /// The bucket for the client. This value can be inferred from the incoming request, but can be overridden here.
    #[serde(rename = "AWS_BUCKET")]
    pub bucket: Option<String>,
    /// The region for the client. This value can be inferred from the incoming request, but can be overridden here.
    #[serde(rename = "AWS_REGION")]
    pub region: Option<String>,
    /// The endpoint URL for the client.
    #[serde(rename = "AWS_ENDPOINT_URL")]
    pub endpoint_url: Option<String>,
    /// The addressing style for the client. This value can be inferred from the secret key format,
    /// but can be overridden here.
    #[serde(rename = "AWS_ADDRESSING_STYLE")]
    pub addressing_style: Option<AddressingStyle>,
    /// The role ARN for the client. Note: we are specifially avoiding the use of the AWS_ROLE_ARN environment variable,
    /// since it is used by the AWS, see <https://docs.aws.amazon.com/cli/v1/userguide/cli-configure-envvars.html>
    #[serde(rename = "AWS_ASSUMED_ROLE_ARN")]
    pub role_arn: Option<String>,
    /// The session name for the client.
    #[serde(rename = "AWS_ASSUMED_ROLE_SESSION_NAME")]
    pub role_session_name: Option<String>,
    /// The session length for the client.
    #[serde(
        rename = "AWS_ASSUMED_ROLE_SESSION_LENGTH",
        with = "humantime_serde::option",
        default
    )]
    pub role_session_length: Option<Duration>,
    /// The external ID for the client.
    #[serde(rename = "AWS_EXTERNAL_ID")]
    pub external_id: Option<String>,
    /// The cloudfront configuration for the client.
    #[serde(rename = "CLOUDFRONT")]
    pub cloudfront: Option<Cloudfront>,
    /// Whether object versioning is enabled on this endpoint's bucket. When set,
    /// overrides the backend-wide `s3.versioning_enabled` config for this
    /// endpoint; when unset, the backend-wide value (or enabled) applies.
    #[serde(rename = "VERSIONING_ENABLED")]
    pub versioning_enabled: Option<bool>,
    /// Whether this endpoint's bucket is read-only. When set, overrides the
    /// backend-wide `s3.read_only` config for this endpoint; when unset, the
    /// backend-wide value (or writable) applies.
    #[serde(rename = "READ_ONLY")]
    pub read_only: Option<bool>,
    /// Client credentials override. The above bucket secrets will be ignored, if specified.
    #[serde(skip)]
    pub credentials_override: Option<CredentialsOverride>,
}

#[derive(Default, Clone, Deserialize, Serialize, PartialEq)]
pub struct Cloudfront {
    /// The cloudfront distribution url. e.g. `https://d6x43ndjxh9bo.cloudfront.net`
    #[serde(rename = "DISTRIBUTION_URL")]
    pub distribution_url: Option<Url>,
    /// The cloudfront key pair ID used for signing requests.
    #[serde(rename = "KEY_PAIR_ID")]
    pub key_pair_id: Option<String>,
    /// Private key for signing. Cloudfront supports RSA and ECDSA keys.
    #[serde(rename = "PRIVATE_KEY_PEM")]
    pub private_key_pem: Option<String>,
}

impl ClientCredentials {
    pub fn from_secrets(secrets: &Secrets, resource_address: &Address) -> Result<Option<Self>, Error> {
        // Check both addressing styles for a config value.
        let (config_value, addressing_style, bucket) =
            match secrets.get(resource_address.config_key(AddressingStyle::Virtual)) {
                Some(config_value) => (Some(config_value), AddressingStyle::Virtual, None),
                None => match secrets.get(resource_address.config_key(AddressingStyle::Path)) {
                    Some(config_value) => (
                        Some(config_value),
                        AddressingStyle::Path,
                        Some(resource_address.path_style_bucket_name()?.to_string()),
                    ),
                    None => (None, AddressingStyle::default(), None),
                },
            };

        let mut config_value = match config_value {
            Some(config_value) => config_value.clone(),
            None => return Ok(None),
        };

        // Use bucket name if specified, otherwise take the bucket name from the resource address.
        config_value.bucket = config_value.bucket.or_else(|| bucket.map(|b| b.to_string()));

        // Use addressing style if specified, otherwise use the addressing style from the resource address.
        config_value.addressing_style = config_value.addressing_style.or(Some(addressing_style));

        // Use region if specified, if path style, use us-east-1 as the default region.
        config_value.region = config_value.region.or_else(|| match addressing_style {
            AddressingStyle::Virtual => None,
            AddressingStyle::Path => Some("us-east-1".to_string()),
        });

        Ok(Some(config_value))
    }

    pub fn from_env() -> Result<Self, Error> {
        let access_key_id = std::env::var("AWS_ACCESS_KEY_ID").ok();
        let secret_access_key = std::env::var("AWS_SECRET_ACCESS_KEY").ok();
        let session_token = std::env::var("AWS_SESSION_TOKEN").ok();
        let bucket = std::env::var("AWS_BUCKET").ok();
        let region = std::env::var("AWS_REGION").ok();
        let endpoint_url = std::env::var("AWS_ENDPOINT_URL").ok();
        let addressing_style = std::env::var("AWS_ADDRESSING_STYLE")
            .map(|s| serde_json::from_str(&s))
            .ok()
            .and_then(|r| r.ok());
        let role_arn = std::env::var("AWS_ASSUMED_ROLE_ARN").ok();
        let role_session_name = std::env::var("AWS_ASSUMED_ROLE_SESSION_NAME").ok();
        let role_session_length = std::env::var("AWS_ASSUMED_ROLE_SESSION_LENGTH")
            .ok()
            .and_then(|s| humantime::parse_duration(&s).ok());
        let external_id = std::env::var("AWS_EXTERNAL_ID").ok();

        // cloudfront configuration is not supported from env vars
        let cloudfront = None;
        let versioning_enabled = std::env::var("VERSIONING_ENABLED").ok().and_then(|s| s.parse().ok());
        let read_only = std::env::var("READ_ONLY").ok().and_then(|s| s.parse().ok());

        Ok(Self {
            access_key_id,
            secret_access_key,
            session_token,
            bucket,
            region,
            endpoint_url,
            addressing_style,
            role_arn,
            role_session_name,
            role_session_length,
            external_id,
            cloudfront,
            versioning_enabled,
            read_only,
            credentials_override: None,
        })
    }

    pub fn credential_hash(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.access_key_id.as_deref().unwrap_or("").as_bytes());
        hasher.update(self.secret_access_key.as_deref().unwrap_or("").as_bytes());
        hasher.update(self.session_token.as_deref().unwrap_or("").as_bytes());
        hex::encode(hasher.finalize())
    }
}

impl Debug for ClientCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientConfig")
            .field("access_key_id", &self.access_key_id.is_some())
            .field("secret_access_key", &self.secret_access_key.is_some())
            .field("session_token", &self.session_token.is_some())
            .field("bucket", &self.bucket)
            .field("region", &self.region)
            .field("endpoint_url", &self.endpoint_url)
            .field("addressing_style", &self.addressing_style)
            .field("role_arn", &self.role_arn.is_some())
            .field("role_session_name", &self.role_session_name.is_some())
            .field("role_session_length", &self.role_session_length)
            .field("external_id", &self.external_id.is_some())
            .field("cloudfront", &self.cloudfront)
            .field("credentials_override", &self.credentials_override)
            .finish()
    }
}

impl Debug for Cloudfront {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // hash the private keys to avoid logging the actual values
        use sha2::{Digest, Sha256};
        let private_key_pem = self
            .private_key_pem
            .as_ref()
            .map(|pem| hex::encode(Sha256::digest(pem.as_bytes())));

        f.debug_struct("Cloudfront")
            .field("distribution_url", &self.distribution_url)
            .field("key_pair_id", &self.key_pair_id)
            .field("private_key_pem", &private_key_pem)
            .finish()
    }
}

/// Client credentials override for the S3 service.
///
/// Allows to customize the acquisition of credentials for a bucket.
#[derive(Default, Clone)]
pub enum CredentialsOverride {
    /// Use anonymous authentication.
    #[default]
    Anonymous,
    /// Use a credentials provider to obtain AWS credentials.
    CredentialsProvider(Arc<dyn ProvideCredentials>),
}

impl std::fmt::Debug for CredentialsOverride {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Anonymous => f.debug_tuple("Anonymous").finish(),
            Self::CredentialsProvider(_) => f.debug_tuple("CredentialsProvider").finish_non_exhaustive(),
        }
    }
}

impl std::cmp::PartialEq for CredentialsOverride {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Anonymous, Self::Anonymous) => true,
            (Self::CredentialsProvider(a), Self::CredentialsProvider(b)) => Arc::ptr_eq(a, b),
            _ => false,
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
                    "storage-service-s3": {
                        "mybucket.example.com": {
                            "AWS_ACCESS_KEY_ID": "AKIAIOSFODNN7EXAMPLE",
                            "AWS_SECRET_ACCESS_KEY": "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
                            "AWS_REGION": "us-west-2"
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
            let secrets: Secrets = secrets_figment.focus("storage-service-s3").extract().unwrap();

            assert_eq!(secrets.len(), 1);
            assert!(secrets.contains_key("mybucket.example.com"));

            let credentials = &secrets["mybucket.example.com"];
            assert_eq!(credentials.access_key_id, Some("AKIAIOSFODNN7EXAMPLE".to_string()));
            assert_eq!(
                credentials.secret_access_key,
                Some("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string())
            );
            assert_eq!(credentials.region, Some("us-west-2".to_string()));
            assert!(credentials.session_token.is_none());
            assert!(credentials.bucket.is_none());
            assert!(credentials.endpoint_url.is_none());
            assert!(credentials.addressing_style.is_none());
            assert!(credentials.role_arn.is_none());
            assert!(credentials.role_session_name.is_none());
            assert!(credentials.role_session_length.is_none());
            assert!(credentials.external_id.is_none());
            assert!(credentials.cloudfront.is_none());

            Ok(())
        });
    }

    #[test_log::test()]
    fn test_secrets_multiple_entries() {
        Jail::expect_with(|jail| {
            let data = r#"
                {
                    "storage-service-s3": {
                        "bucket1.example.com": {
                            "AWS_ACCESS_KEY_ID": "AKIAIOSFODNN7EXAMPLE1",
                            "AWS_SECRET_ACCESS_KEY": "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY1",
                            "AWS_REGION": "us-east-1"
                        },
                        "https://bucket2.example.com": {
                            "AWS_ACCESS_KEY_ID": "AKIAIOSFODNN7EXAMPLE2",
                            "AWS_SECRET_ACCESS_KEY": "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY2",
                            "AWS_REGION": "us-west-2",
                            "AWS_ENDPOINT_URL": "https://custom-endpoint.com"
                        },
                        "https://bucket3.example.com/path": {
                            "AWS_ACCESS_KEY_ID": "AKIAIOSFODNN7EXAMPLE3",
                            "AWS_SECRET_ACCESS_KEY": "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY3",
                            "AWS_ADDRESSING_STYLE": "path"
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
            let secrets: Secrets = secrets_figment.focus("storage-service-s3").extract().unwrap();

            assert_eq!(secrets.len(), 3);

            // Test first entry
            let creds1 = &secrets["bucket1.example.com"];
            assert_eq!(creds1.access_key_id, Some("AKIAIOSFODNN7EXAMPLE1".to_string()));
            assert_eq!(creds1.region, Some("us-east-1".to_string()));

            // Test second entry with scheme
            let creds2 = &secrets["https://bucket2.example.com"];
            assert_eq!(creds2.access_key_id, Some("AKIAIOSFODNN7EXAMPLE2".to_string()));
            assert_eq!(creds2.endpoint_url, Some("https://custom-endpoint.com".to_string()));

            // Test third entry with path and addressing style
            let creds3 = &secrets["https://bucket3.example.com/path"];
            assert_eq!(creds3.access_key_id, Some("AKIAIOSFODNN7EXAMPLE3".to_string()));
            assert_eq!(creds3.addressing_style, Some(AddressingStyle::Path));

            Ok(())
        });
    }

    #[test_log::test()]
    fn test_client_credentials_from_secrets_virtual_addressing() {
        Jail::expect_with(|jail| {
            let data = r#"
                {
                    "storage-service-s3": {
                        "https://mybucket.example.com": {
                            "AWS_ACCESS_KEY_ID": "AKIAIOSFODNN7EXAMPLE",
                            "AWS_SECRET_ACCESS_KEY": "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
                            "AWS_REGION": "us-west-2"
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
            let secrets: Secrets = secrets_figment.focus("storage-service-s3").extract().unwrap();

            // Create a virtual addressing style address
            let address = Address::parse("https://mybucket.example.com/path/to/object").unwrap();
            let credentials = ClientCredentials::from_secrets(&secrets, &address).unwrap().unwrap();

            assert_eq!(credentials.access_key_id, Some("AKIAIOSFODNN7EXAMPLE".to_string()));
            assert_eq!(
                credentials.secret_access_key,
                Some("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string())
            );
            assert_eq!(credentials.region, Some("us-west-2".to_string()));
            assert_eq!(credentials.addressing_style, Some(AddressingStyle::Virtual));
            assert!(credentials.bucket.is_none()); // Virtual addressing doesn't extract bucket from path

            Ok(())
        });
    }

    #[test_log::test()]
    fn test_client_credentials_from_secrets_path_addressing() {
        Jail::expect_with(|jail| {
            let data = r#"
                {
                    "storage-service-s3": {
                        "https://example.com/mybucket": {
                            "AWS_ACCESS_KEY_ID": "AKIAIOSFODNN7EXAMPLE",
                            "AWS_SECRET_ACCESS_KEY": "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
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
            let secrets: Secrets = secrets_figment.focus("storage-service-s3").extract().unwrap();

            // Create a path addressing style address
            let address = Address::parse("https://example.com/mybucket/path/to/object").unwrap();
            let credentials = ClientCredentials::from_secrets(&secrets, &address).unwrap().unwrap();

            assert_eq!(credentials.access_key_id, Some("AKIAIOSFODNN7EXAMPLE".to_string()));
            assert_eq!(
                credentials.secret_access_key,
                Some("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string())
            );
            assert_eq!(credentials.addressing_style, Some(AddressingStyle::Path));
            assert_eq!(credentials.bucket, Some("mybucket".to_string())); // Path addressing extracts bucket from path
            assert_eq!(credentials.region, Some("us-east-1".to_string())); // Default region for path style

            Ok(())
        });
    }

    #[test_log::test()]
    fn test_client_credentials_from_secrets_no_match() {
        Jail::expect_with(|jail| {
            let data = r#"
                {
                    "storage-service-s3": {
                        "other.example.com": {
                            "AWS_ACCESS_KEY_ID": "AKIAIOSFODNN7EXAMPLE",
                            "AWS_SECRET_ACCESS_KEY": "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
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
            let secrets: Secrets = secrets_figment.focus("storage-service-s3").extract().unwrap();

            // Create an address that doesn't match any secrets
            let address = Address::parse("https://mybucket.example.com/path/to/object").unwrap();
            let credentials = ClientCredentials::from_secrets(&secrets, &address).unwrap();

            assert!(credentials.is_none());

            Ok(())
        });
    }

    #[test_log::test()]
    fn test_client_credentials_from_env() {
        Jail::expect_with(|jail| {
            jail.set_env("AWS_ACCESS_KEY_ID", "AKIAIOSFODNN7EXAMPLE");
            jail.set_env("AWS_SECRET_ACCESS_KEY", "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY");
            jail.set_env("AWS_SESSION_TOKEN", "test-session-token");
            jail.set_env("AWS_BUCKET", "test-bucket");
            jail.set_env("AWS_REGION", "us-west-2");
            jail.set_env("AWS_ENDPOINT_URL", "https://custom-endpoint.com");
            jail.set_env("AWS_ADDRESSING_STYLE", "\"path\"");
            jail.set_env("AWS_ASSUMED_ROLE_ARN", "arn:aws:iam::123456789012:role/test-role");
            jail.set_env("AWS_ASSUMED_ROLE_SESSION_NAME", "test-session-name");
            jail.set_env("AWS_ASSUMED_ROLE_SESSION_LENGTH", "1h");
            jail.set_env("AWS_EXTERNAL_ID", "test-external-id");

            let credentials = ClientCredentials::from_env().unwrap();

            assert_eq!(credentials.access_key_id, Some("AKIAIOSFODNN7EXAMPLE".to_string()));
            assert_eq!(
                credentials.secret_access_key,
                Some("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string())
            );
            assert_eq!(credentials.session_token, Some("test-session-token".to_string()));
            assert_eq!(credentials.bucket, Some("test-bucket".to_string()));
            assert_eq!(credentials.region, Some("us-west-2".to_string()));
            assert_eq!(
                credentials.endpoint_url,
                Some("https://custom-endpoint.com".to_string())
            );
            assert_eq!(credentials.addressing_style, Some(AddressingStyle::Path));
            assert_eq!(
                credentials.role_arn,
                Some("arn:aws:iam::123456789012:role/test-role".to_string())
            );
            assert_eq!(credentials.role_session_name, Some("test-session-name".to_string()));
            assert_eq!(credentials.role_session_length, Some(Duration::from_hours(1)));
            assert_eq!(credentials.external_id, Some("test-external-id".to_string()));

            Ok(())
        });
    }

    #[test_log::test()]
    fn unit_client_credentials_from_env_partial() {
        Jail::expect_with(|jail| {
            jail.set_env("AWS_ACCESS_KEY_ID", "AKIAIOSFODNN7EXAMPLE");
            jail.set_env("AWS_SECRET_ACCESS_KEY", "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY");
            // Other env vars not set

            let credentials = ClientCredentials::from_env().unwrap();

            assert_eq!(credentials.access_key_id, Some("AKIAIOSFODNN7EXAMPLE".to_string()));
            assert_eq!(
                credentials.secret_access_key,
                Some("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string())
            );
            assert!(credentials.session_token.is_none());
            assert!(credentials.bucket.is_none());
            assert!(credentials.region.is_none());
            assert!(credentials.endpoint_url.is_none());
            assert!(credentials.addressing_style.is_none());
            assert!(credentials.role_arn.is_none());
            assert!(credentials.role_session_name.is_none());
            assert!(credentials.role_session_length.is_none());
            assert!(credentials.external_id.is_none());
            assert!(credentials.cloudfront.is_none());

            Ok(())
        });
    }

    #[test_log::test()]
    fn test_client_credentials_credential_hash() {
        let credentials1 = ClientCredentials {
            access_key_id: Some("AKIAIOSFODNN7EXAMPLE".to_string()),
            secret_access_key: Some("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string()),
            session_token: Some("test-token".to_string()),
            bucket: None,
            region: None,
            endpoint_url: None,
            addressing_style: None,
            role_arn: None,
            role_session_name: None,
            role_session_length: None,
            external_id: None,
            cloudfront: None,
            versioning_enabled: None,
            read_only: None,
            credentials_override: None,
        };

        let credentials2 = ClientCredentials {
            access_key_id: Some("AKIAIOSFODNN7EXAMPLE".to_string()),
            secret_access_key: Some("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string()),
            session_token: Some("test-token".to_string()),
            bucket: Some("different-bucket".to_string()), // Different bucket
            region: Some("different-region".to_string()), // Different region
            endpoint_url: Some("different-endpoint".to_string()), // Different endpoint
            addressing_style: Some(AddressingStyle::Path), // Different addressing style
            role_arn: None,
            role_session_name: None,
            role_session_length: None,
            external_id: None,
            cloudfront: None,
            versioning_enabled: None,
            read_only: None,
            credentials_override: None,
        };

        let credentials3 = ClientCredentials {
            access_key_id: Some("DIFFERENT_KEY".to_string()), // Different access key
            secret_access_key: Some("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string()),
            session_token: Some("test-token".to_string()),
            bucket: None,
            region: None,
            endpoint_url: None,
            addressing_style: None,
            role_arn: None,
            role_session_name: None,
            role_session_length: None,
            external_id: None,
            cloudfront: None,
            versioning_enabled: None,
            read_only: None,
            credentials_override: None,
        };

        let hash1 = credentials1.credential_hash();
        let hash2 = credentials2.credential_hash();
        let hash3 = credentials3.credential_hash();

        // Same credentials should produce same hash
        assert_eq!(hash1, hash2);

        // Different credentials should produce different hash
        assert_ne!(hash1, hash3);
        assert_ne!(hash2, hash3);

        // Hash should be consistent
        assert_eq!(credentials1.credential_hash(), credentials1.credential_hash());
    }

    #[test_log::test()]
    fn test_client_credentials_credential_hash_with_none_values() {
        let credentials1 = ClientCredentials {
            access_key_id: None,
            secret_access_key: None,
            session_token: None,
            bucket: None,
            region: None,
            endpoint_url: None,
            addressing_style: None,
            role_arn: None,
            role_session_name: None,
            role_session_length: None,
            external_id: None,
            cloudfront: None,
            versioning_enabled: None,
            read_only: None,
            credentials_override: None,
        };

        let credentials2 = ClientCredentials {
            access_key_id: Some("".to_string()),     // Empty string
            secret_access_key: Some("".to_string()), // Empty string
            session_token: Some("".to_string()),     // Empty string
            bucket: None,
            region: None,
            endpoint_url: None,
            addressing_style: None,
            role_arn: None,
            role_session_name: None,
            role_session_length: None,
            external_id: None,
            cloudfront: None,
            versioning_enabled: None,
            read_only: None,
            credentials_override: None,
        };

        let hash1 = credentials1.credential_hash();
        let hash2 = credentials2.credential_hash();

        // None values and empty strings should produce the same hash
        assert_eq!(hash1, hash2);
    }

    #[test_log::test()]
    fn test_client_credentials_debug_format() {
        let credentials = ClientCredentials {
            access_key_id: Some("AKIAIOSFODNN7EXAMPLE".to_string()),
            secret_access_key: Some("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string()),
            session_token: Some("test-token".to_string()),
            bucket: Some("test-bucket".to_string()),
            region: Some("us-west-2".to_string()),
            endpoint_url: Some("https://endpoint.com".to_string()),
            addressing_style: Some(AddressingStyle::Virtual),
            role_arn: None,
            role_session_name: None,
            role_session_length: None,
            external_id: None,
            cloudfront: None,
            versioning_enabled: None,
            read_only: None,
            credentials_override: None,
        };

        let debug_str = format!("{:?}", credentials);

        // Should not contain actual secret values
        assert!(!debug_str.contains("AKIAIOSFODNN7EXAMPLE"));
        assert!(!debug_str.contains("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"));
        assert!(!debug_str.contains("test-token"));

        // Should contain boolean indicators for presence of secrets
        assert!(debug_str.contains("access_key_id: true"));
        assert!(debug_str.contains("secret_access_key: true"));
        assert!(debug_str.contains("session_token: true"));

        // Should contain non-secret values
        assert!(debug_str.contains("test-bucket"));
        assert!(debug_str.contains("us-west-2"));
        assert!(debug_str.contains("https://endpoint.com"));
        assert!(debug_str.contains("Virtual"));
    }

    #[test_log::test()]
    fn test_secrets_deref_traits() {
        let mut secrets = Secrets(HashMap::new());
        let credentials = ClientCredentials {
            access_key_id: Some("AKIAIOSFODNN7EXAMPLE".to_string()),
            secret_access_key: Some("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string()),
            session_token: None,
            bucket: None,
            region: None,
            endpoint_url: None,
            addressing_style: None,
            role_arn: None,
            role_session_name: None,
            role_session_length: None,
            external_id: None,
            cloudfront: None,
            versioning_enabled: None,
            read_only: None,
            credentials_override: None,
        };

        // Test Deref
        assert_eq!(secrets.len(), 0);
        secrets.insert("test.example.com".to_string(), credentials.clone());
        assert_eq!(secrets.len(), 1);
        assert!(secrets.contains_key("test.example.com"));

        // Test DerefMut
        secrets.get_mut("test.example.com").unwrap().region = Some("us-west-2".to_string());
        assert_eq!(secrets["test.example.com"].region, Some("us-west-2".to_string()));
    }

    #[test_log::test()]
    fn test_secrets_config_key() {
        assert_eq!(Secrets::CONFIG_KEY, "storage-service-s3");
    }

    #[test_log::test()]
    fn test_client_credentials_default() {
        let credentials = ClientCredentials::default();

        assert!(credentials.access_key_id.is_none());
        assert!(credentials.secret_access_key.is_none());
        assert!(credentials.session_token.is_none());
        assert!(credentials.bucket.is_none());
        assert!(credentials.region.is_none());
        assert!(credentials.endpoint_url.is_none());
        assert!(credentials.addressing_style.is_none());
        assert!(credentials.cloudfront.is_none());
    }

    #[test_log::test()]
    fn test_client_credentials_override_bucket_and_region() {
        Jail::expect_with(|jail| {
            let data = r#"
                {
                    "storage-service-s3": {
                        "https://example.com/mybucket": {
                            "AWS_ACCESS_KEY_ID": "AKIAIOSFODNN7EXAMPLE",
                            "AWS_SECRET_ACCESS_KEY": "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
                            "AWS_BUCKET": "override-bucket",
                            "AWS_REGION": "override-region"
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
            let secrets: Secrets = secrets_figment.focus("storage-service-s3").extract().unwrap();

            let address = Address::parse("https://example.com/mybucket/path/to/object").unwrap();
            let credentials = ClientCredentials::from_secrets(&secrets, &address).unwrap().unwrap();

            // Override values should take precedence
            assert_eq!(credentials.bucket, Some("override-bucket".to_string()));
            assert_eq!(credentials.region, Some("override-region".to_string()));
            assert_eq!(credentials.addressing_style, Some(AddressingStyle::Path));

            Ok(())
        });
    }

    #[test_log::test()]
    fn test_client_credentials_override_addressing_style() {
        Jail::expect_with(|jail| {
            let data = r#"
                {
                    "storage-service-s3": {
                        "https://example.com/mybucket": {
                            "AWS_ACCESS_KEY_ID": "AKIAIOSFODNN7EXAMPLE",
                            "AWS_SECRET_ACCESS_KEY": "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
                            "AWS_ADDRESSING_STYLE": "virtual"
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
            let secrets: Secrets = secrets_figment.focus("storage-service-s3").extract().unwrap();

            let address = Address::parse("https://example.com/mybucket/path/to/object").unwrap();
            let credentials = ClientCredentials::from_secrets(&secrets, &address).unwrap().unwrap();

            // Override addressing style should take precedence
            assert_eq!(credentials.addressing_style, Some(AddressingStyle::Virtual));
            assert_eq!(credentials.bucket, Some("mybucket".to_string())); // Still extracted from path
            assert_eq!(credentials.region, Some("us-east-1".to_string())); // Default for path style

            Ok(())
        });
    }

    #[test_log::test()]
    fn test_secrets_cloudfront() {
        Jail::expect_with(|jail| {
            let data = r#"
                {
                    "storage-service-s3": {
                        "mybucket.example.com": {
                            "AWS_ACCESS_KEY_ID": "AKIAIOSFODNN7EXAMPLE",
                            "AWS_SECRET_ACCESS_KEY": "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
                            "AWS_REGION": "us-west-2",
                            "CLOUDFRONT": {
                                "DISTRIBUTION_URL": "https://d654rnexample.cloudfront.net",
                                "KEY_PAIR_ID": "K2EXAMPLEAMO0T",
                                "PRIVATE_KEY_PEM": "-----BEGIN EC PRIVATE KEY-----\nMIIEpAIBAAKCAQEXAMPLEKEY..."
                            }
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
            let secrets: Secrets = secrets_figment.focus("storage-service-s3").extract().unwrap();

            assert_eq!(secrets.len(), 1);
            assert!(secrets.contains_key("mybucket.example.com"));

            let credentials = &secrets["mybucket.example.com"];

            assert_eq!(
                credentials.cloudfront,
                Some(Cloudfront {
                    distribution_url: Some(Url::parse("https://d654rnexample.cloudfront.net").unwrap()),
                    key_pair_id: Some("K2EXAMPLEAMO0T".to_string()),
                    private_key_pem: Some("-----BEGIN EC PRIVATE KEY-----\nMIIEpAIBAAKCAQEXAMPLEKEY...".to_string()),
                })
            );

            Ok(())
        });
    }

    #[test_log::test()]
    fn test_credentials_override_debug_format_anonymous() {
        let credentials = CredentialsOverride::Anonymous;
        let debug_str = format!("{:?}", credentials);
        assert_eq!(debug_str, "Anonymous");
    }

    #[test_log::test()]
    fn test_credentials_override_debug_format_credentials_provider() {
        use aws_credential_types::provider;

        #[derive(Debug)]
        struct MockProvider;

        impl provider::ProvideCredentials for MockProvider {
            fn provide_credentials<'a>(&'a self) -> provider::future::ProvideCredentials<'a>
            where
                Self: 'a,
            {
                unimplemented!()
            }
        }

        let credentials = CredentialsOverride::CredentialsProvider(Arc::new(MockProvider));
        let debug_str = format!("{:?}", credentials);
        assert_eq!(debug_str, "CredentialsProvider(..)");
    }
}
