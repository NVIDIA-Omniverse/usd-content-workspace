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

use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;

#[derive(Parser, Debug, Clone)]
#[command(name = "microsoft-graph-userinfo-service")]
pub struct Config {
    #[arg(long, env = "STARTUP_MODE", default_value = "reader", value_enum)]
    pub startup_mode: StartupMode,

    #[arg(long, env = "PORT", default_value = "8080")]
    pub port: u16,

    #[arg(long, env = "GRPC_PORT", default_value = "50051")]
    pub grpc_port: u16,

    #[arg(long, env = "RUST_LOG", default_value = "info")]
    pub rust_log: String,

    #[arg(long, env = "STRUCTURED_LOGGING", default_value = "false")]
    pub structured_logging: bool,

    /// Path to a JSON file of additional users, groups, and memberships overlaid
    /// on the directory in addition to those synced from the identity provider.
    /// Read once at startup and applied to every read in reader mode regardless
    /// of whether caching is enabled.
    #[arg(long, env = "STATIC_DIRECTORY_FILE")]
    pub static_directory_file: Option<PathBuf>,

    #[command(flatten)]
    pub entra: EntraConfig,

    #[command(flatten)]
    pub cache: CacheEnvConfig,

    #[command(flatten)]
    pub app_role_assignments: AppRoleAssignmentsConfig,

    #[command(flatten)]
    pub postgres: PostgresConfig,

    #[command(flatten)]
    pub permission: PermissionEnvConfig,

    #[command(flatten)]
    pub otel: OtelConfig,

    #[command(flatten)]
    pub access_log: AccessLogConfig,

    /// Names of incoming request headers to forward verbatim to other Omniverse
    /// services (currently the Permission Service). Comma-separated; matched
    /// case-insensitively and stored lowercased.
    #[arg(long, env = "FORWARD_HEADERS", default_value = "")]
    pub forward_headers: ForwardHeaders,
}

impl Config {
    /// Validate cross-field invariants that clap cannot express on its own.
    ///
    /// Enterprise-application scoping filters groups/memberships at sync time and
    /// serves them from the snapshot, so it depends on caching being enabled for
    /// all three resource kinds. A contradictory setting is rejected outright
    /// rather than silently overridden.
    pub fn validate(&self) -> Result<(), String> {
        if self.app_role_assignments.enabled {
            if !self.cache.enabled {
                return Err("APP_ROLE_ASSIGNMENTS_ENABLED requires CACHE_ENABLED=true".to_string());
            }
            if !self.cache.groups_enabled {
                return Err(
                    "APP_ROLE_ASSIGNMENTS_ENABLED requires CACHE_GROUPS_ENABLED=true".to_string(),
                );
            }
            if !self.cache.memberships_enabled {
                return Err(
                    "APP_ROLE_ASSIGNMENTS_ENABLED requires CACHE_MEMBERSHIPS_ENABLED=true"
                        .to_string(),
                );
            }
        }
        Ok(())
    }
}

#[derive(clap::ValueEnum, Debug, Clone, Default)]
pub enum StartupMode {
    /// Serve the HTTP and gRPC read APIs. Reads from the PostgreSQL snapshot
    /// (periodically reloaded into memory) when caching is enabled, or live
    /// from the identity provider when caching is disabled. Never writes to the
    /// snapshot store.
    #[default]
    Reader,
    /// Continuously synchronize the directory from the identity provider into
    /// the PostgreSQL snapshot store. Exposes only health endpoints; does not
    /// serve the read APIs.
    Writer,
}

#[derive(clap::ValueEnum, Debug, Clone, Default)]
pub enum AuthMode {
    #[default]
    ClientSecret,
    WorkloadIdentity,
}

/// TLS requirement for the PostgreSQL connection.
#[derive(clap::ValueEnum, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PostgresSslMode {
    /// Never negotiate TLS (plaintext connection).
    #[default]
    Disable,
    /// Try TLS first; fall back to plaintext if the server rejects it.
    Prefer,
    /// Require a TLS connection; abort if the server does not support it.
    Require,
}

#[derive(clap::Args, Debug, Clone)]
pub struct EntraConfig {
    #[arg(long, env = "AZURE_TENANT_ID")]
    pub azure_tenant_id: String,

    #[arg(long, env = "AZURE_CLIENT_ID")]
    pub azure_client_id: String,

    #[arg(
        long,
        env = "AZURE_AUTH_MODE",
        default_value = "client-secret",
        value_enum
    )]
    pub azure_auth_mode: AuthMode,

    #[arg(long, env = "AZURE_CLIENT_SECRET")]
    pub azure_client_secret: Option<String>,

    #[arg(
        long,
        env = "AZURE_FEDERATED_TOKEN_FILE",
        default_value = "/var/run/secrets/azure/tokens/azure-identity-token"
    )]
    pub azure_federated_token_file: String,

    #[arg(long, env = "GRAPH_MAX_RETRIES", default_value = "3")]
    pub graph_max_retries: u32,
}

#[derive(clap::Args, Debug, Clone)]
pub struct CacheEnvConfig {
    #[arg(long, env = "CACHE_ENABLED", default_value = "true")]
    pub enabled: bool,

    #[arg(long, env = "CACHE_USERS_ENABLED", default_value = "true")]
    pub users_enabled: bool,

    #[arg(long, env = "CACHE_GROUPS_ENABLED", default_value = "true")]
    pub groups_enabled: bool,

    /// WARNING: Enabling membership caching can drastically increase both the initial
    /// synchronization time and memory usage. For a large Active Directory (e.g. ~100,000 users
    /// and ~170,000 groups), a full sync can take several hours, delta syncs can take tens of
    /// minutes, and the in-memory membership index can consume up to 2.5 GB of RAM. Ensure the
    /// readiness probe's failureThreshold and container memory limits are sized accordingly.
    #[arg(long, env = "CACHE_MEMBERSHIPS_ENABLED", default_value = "false")]
    pub memberships_enabled: bool,

    #[arg(long, env = "SYNC_INTERVAL", default_value = "5m", value_parser = humantime::parse_duration)]
    pub sync_interval: Duration,

    #[arg(long, env = "SYNC_JITTER_MAX", default_value = "0s", value_parser = humantime::parse_duration)]
    pub sync_jitter_max: Duration,

    /// How often a reader reloads the snapshot from the store. Only used in
    /// reader startup mode with caching enabled.
    #[arg(long, env = "READER_RELOAD_INTERVAL", default_value = "60s", value_parser = humantime::parse_duration)]
    pub reader_reload_interval: Duration,
}

/// Enterprise-application app-role assignment scoping.
///
/// When enabled, the writer reads the enterprise application's own
/// `appRoleAssignedTo` collection to restrict synced groups/memberships to the
/// groups assigned to that application, and surfaces the application's assigned
/// service principals as additional users. Requires the cache to be enabled for
/// groups and memberships (see [`Config::validate`]).
#[derive(clap::Args, Debug, Clone)]
pub struct AppRoleAssignmentsConfig {
    #[arg(
        id = "app-role-assignments-enabled",
        long = "app-role-assignments-enabled",
        env = "APP_ROLE_ASSIGNMENTS_ENABLED",
        default_value = "false"
    )]
    pub enabled: bool,

    /// Object id of the enterprise application's service principal to query.
    /// When unset, the authenticating application's own service principal is
    /// resolved from `AZURE_CLIENT_ID`, which works without the
    /// `Application.Read.All` permission. Pointing this at a different
    /// application's service principal requires `Application.Read.All`.
    #[arg(
        id = "app-role-assignments-service-principal-object-id",
        long = "app-role-assignments-service-principal-object-id",
        env = "APP_ROLE_ASSIGNMENTS_SERVICE_PRINCIPAL_OBJECT_ID"
    )]
    pub service_principal_object_id: Option<String>,

    #[arg(
        id = "app-role-assignments-include-service-principals",
        long = "app-role-assignments-include-service-principals",
        env = "APP_ROLE_ASSIGNMENTS_INCLUDE_SERVICE_PRINCIPALS",
        default_value = "true"
    )]
    pub include_service_principals: bool,
}

#[derive(clap::Args, Debug, Clone)]
pub struct PermissionEnvConfig {
    #[arg(
        id = "permission-enabled",
        long = "permission-enabled",
        env = "PERMISSION_ENABLED",
        default_value = "false"
    )]
    pub enabled: bool,

    #[arg(
        id = "permission-url",
        long = "permission-url",
        env = "PERMISSION_URL",
        default_value = ""
    )]
    pub url: String,

    #[arg(
        id = "permission-service-name",
        long = "permission-service-name",
        env = "PERMISSION_SERVICE_NAME",
        default_value = "userinfo"
    )]
    pub service_name: String,

    #[arg(id = "permission-cache-ttl", long = "permission-cache-ttl", env = "PERMISSION_CACHE_TTL", default_value = "5m", value_parser = humantime::parse_duration)]
    pub cache_ttl: Duration,

    #[arg(
        id = "permission-cache-max-capacity",
        long = "permission-cache-max-capacity",
        env = "PERMISSION_CACHE_MAX_CAPACITY",
        default_value = "10000"
    )]
    pub cache_max_capacity: u64,
}

#[derive(clap::Args, Debug, Clone)]
pub struct OtelConfig {
    #[arg(
        id = "otel-traces-enabled",
        long = "otel-traces-enabled",
        env = "OTEL_TRACES_ENABLED",
        default_value = "false"
    )]
    pub traces_enabled: bool,

    #[arg(
        id = "otel-traces-endpoint",
        long = "otel-traces-endpoint",
        env = "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
        default_value = "http://localhost:4317"
    )]
    pub traces_endpoint: String,

    #[arg(
        id = "otel-metrics-enabled",
        long = "otel-metrics-enabled",
        env = "OTEL_METRICS_ENABLED",
        default_value = "false"
    )]
    pub metrics_enabled: bool,

    #[arg(
        id = "otel-metrics-endpoint",
        long = "otel-metrics-endpoint",
        env = "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT",
        default_value = "http://localhost:4317"
    )]
    pub metrics_endpoint: String,

    #[arg(
        id = "otel-service-name",
        long = "otel-service-name",
        env = "OTEL_SERVICE_NAME",
        default_value = "microsoft-graph-userinfo-service"
    )]
    pub service_name: String,
}

/// API access logging. Disabled by default; when enabled, one structured
/// record is emitted per business REST/gRPC call.
#[derive(clap::Args, Debug, Clone)]
pub struct AccessLogConfig {
    #[arg(
        id = "access-log-enabled",
        long = "access-log-enabled",
        env = "ACCESS_LOG_ENABLED",
        default_value = "false"
    )]
    pub enabled: bool,

    /// Map of access-record field name to request header name, given as a JSON
    /// object (e.g. `{"ip":"x-forwarded-for","ua":"user-agent"}`). Each present
    /// header is copied as a top-level field on the record. Credential headers
    /// (`authorization`, `cookie`) and the reserved field names `transport`,
    /// `operation`, `request`, and `response` are dropped at load. Header names
    /// are matched case-insensitively and stored lowercased.
    #[arg(
        id = "access-log-headers",
        long = "access-log-headers",
        env = "ACCESS_LOG_HEADERS",
        default_value = "{}"
    )]
    pub headers: AccessLogHeaders,
}

/// Parsed `ACCESS_LOG_HEADERS` map of access-record field name to lowercased
/// header name, with credential and reserved-field entries removed.
#[derive(Debug, Clone)]
pub struct AccessLogHeaders(pub Vec<(String, String)>);

impl std::str::FromStr for AccessLogHeaders {
    type Err = String;

    /// Parses a JSON object of access-record field name to header name,
    /// lowercasing header names and dropping credential headers and
    /// reserved-field entries.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let map: std::collections::BTreeMap<String, String> =
            serde_json::from_str(s).map_err(|e| format!("invalid ACCESS_LOG_HEADERS JSON: {e}"))?;
        let headers = map
            .into_iter()
            .filter_map(|(field, header)| {
                let header = header.trim().to_ascii_lowercase();
                if header.is_empty()
                    || RESERVED_ACCESS_FIELDS.contains(&field.as_str())
                    || CREDENTIAL_HEADERS.contains(&header.as_str())
                {
                    return None;
                }
                Some((field, header))
            })
            .collect();
        Ok(AccessLogHeaders(headers))
    }
}

const RESERVED_ACCESS_FIELDS: [&str; 4] = ["transport", "operation", "request", "response"];

const CREDENTIAL_HEADERS: [&str; 2] = ["authorization", "cookie"];

/// Parsed `FORWARD_HEADERS` list of lowercased header names to forward to other
/// Omniverse services.
#[derive(Debug, Clone)]
pub struct ForwardHeaders(pub Vec<String>);

impl ForwardHeaders {
    /// Selects the configured header names from incoming REST headers into a
    /// header map for an outgoing request to another Omniverse service.
    /// Multi-valued headers are preserved.
    pub fn select_http(&self, headers: &http::HeaderMap) -> reqwest::header::HeaderMap {
        let mut out = reqwest::header::HeaderMap::new();
        for name in &self.0 {
            let Ok(header_name) = reqwest::header::HeaderName::from_bytes(name.as_bytes()) else {
                continue;
            };
            for value in headers.get_all(name.as_str()).iter() {
                out.append(header_name.clone(), value.clone());
            }
        }
        out
    }

    /// Selects the configured header names from incoming gRPC metadata into a
    /// header map for an outgoing request to another Omniverse service.
    /// Multi-valued keys are preserved; binary metadata is ignored.
    pub fn select_grpc(
        &self,
        metadata: &tonic::metadata::MetadataMap,
    ) -> reqwest::header::HeaderMap {
        let mut out = reqwest::header::HeaderMap::new();
        for name in &self.0 {
            let Ok(header_name) = reqwest::header::HeaderName::from_bytes(name.as_bytes()) else {
                continue;
            };
            for value in metadata.get_all(name.as_str()).iter() {
                if let Ok(value) = value.to_str()
                    && let Ok(value) = reqwest::header::HeaderValue::from_str(value)
                {
                    out.append(header_name.clone(), value);
                }
            }
        }
        out
    }
}

impl std::str::FromStr for ForwardHeaders {
    type Err = std::convert::Infallible;

    /// Parses a comma-separated list of header names, trimming and lowercasing
    /// each and dropping empty entries.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let headers = s
            .split(',')
            .map(|h| h.trim().to_ascii_lowercase())
            .filter(|h| !h.is_empty())
            .collect();
        Ok(ForwardHeaders(headers))
    }
}

#[derive(clap::Args, Debug, Clone)]
pub struct PostgresConfig {
    #[arg(
        id = "postgres-enabled",
        long = "postgres-enabled",
        env = "POSTGRES_ENABLED",
        default_value = "true"
    )]
    pub enabled: bool,

    #[arg(
        id = "postgres-host",
        long = "postgres-host",
        env = "POSTGRES_HOST",
        default_value = "localhost"
    )]
    pub host: String,

    #[arg(
        id = "postgres-port",
        long = "postgres-port",
        env = "POSTGRES_PORT",
        default_value_t = 5432
    )]
    pub port: u16,

    #[arg(
        id = "postgres-dbname",
        long = "postgres-dbname",
        env = "POSTGRES_DBNAME",
        default_value = "graph-userinfo"
    )]
    pub dbname: String,

    #[arg(
        id = "postgres-user",
        long = "postgres-user",
        env = "POSTGRES_USER",
        default_value = "microsoft_graph_userinfo_service"
    )]
    pub user: String,

    #[arg(
        id = "postgres-password",
        long = "postgres-password",
        env = "POSTGRES_PASSWORD"
    )]
    pub password: Option<String>,

    /// Path to a JSON token file holding a short-lived PostgreSQL password.
    /// When set, takes precedence over `password` and the connection pool is
    /// rebuilt whenever the file rotates.
    #[arg(
        id = "postgres-token-file",
        long = "postgres-token-file",
        env = "POSTGRES_TOKEN_FILE"
    )]
    pub token_file: Option<PathBuf>,

    /// How often the token file is checked for rotation.
    #[arg(
        id = "postgres-token-file-poll-interval",
        long = "postgres-token-file-poll-interval",
        env = "POSTGRES_TOKEN_FILE_POLL_INTERVAL",
        default_value = "15s",
        value_parser = humantime::parse_duration
    )]
    pub token_file_poll_interval: Duration,

    /// How long to wait at startup for the token file to appear and hold a
    /// valid credential. A sidecar may mount the file a few seconds after the
    /// service starts, so startup retries until this deadline before failing.
    #[arg(
        id = "postgres-token-file-wait-timeout",
        long = "postgres-token-file-wait-timeout",
        env = "POSTGRES_TOKEN_FILE_WAIT_TIMEOUT",
        default_value = "30s",
        value_parser = humantime::parse_duration
    )]
    pub token_file_wait_timeout: Duration,

    /// How often the token file is re-checked while waiting for it at startup.
    #[arg(
        id = "postgres-token-file-wait-interval",
        long = "postgres-token-file-wait-interval",
        env = "POSTGRES_TOKEN_FILE_WAIT_INTERVAL",
        default_value = "1s",
        value_parser = humantime::parse_duration
    )]
    pub token_file_wait_interval: Duration,

    #[arg(
        id = "postgres-connections",
        long = "postgres-connections",
        env = "POSTGRES_CONNECTIONS",
        default_value_t = 4
    )]
    pub connections: usize,

    /// TLS requirement for the PostgreSQL connection.
    #[arg(
        id = "postgres-ssl-mode",
        long = "postgres-ssl-mode",
        env = "POSTGRES_SSL_MODE",
        default_value = "prefer",
        value_enum
    )]
    pub ssl_mode: PostgresSslMode,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Config {
        Config::parse_from(["bin", "--azure-tenant-id=t", "--azure-client-id=c"])
    }

    #[test]
    fn validate_allows_app_role_assignments_with_full_cache() {
        let mut config = base();
        config.cache.enabled = true;
        config.cache.groups_enabled = true;
        config.cache.memberships_enabled = true;
        config.app_role_assignments.enabled = true;
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_rejects_app_role_assignments_without_memberships() {
        let mut config = base();
        config.cache.enabled = true;
        config.cache.groups_enabled = true;
        config.cache.memberships_enabled = false;
        config.app_role_assignments.enabled = true;
        let err = config.validate().unwrap_err();
        assert!(err.contains("CACHE_MEMBERSHIPS_ENABLED"), "{err}");
    }

    #[test]
    fn validate_rejects_app_role_assignments_without_groups() {
        let mut config = base();
        config.cache.enabled = true;
        config.cache.groups_enabled = false;
        config.cache.memberships_enabled = true;
        config.app_role_assignments.enabled = true;
        let err = config.validate().unwrap_err();
        assert!(err.contains("CACHE_GROUPS_ENABLED"), "{err}");
    }

    #[test]
    fn validate_rejects_app_role_assignments_without_cache() {
        let mut config = base();
        config.cache.enabled = false;
        config.cache.groups_enabled = true;
        config.cache.memberships_enabled = true;
        config.app_role_assignments.enabled = true;
        let err = config.validate().unwrap_err();
        assert!(err.contains("CACHE_ENABLED"), "{err}");
    }

    #[test]
    fn validate_passes_when_feature_disabled() {
        let mut config = base();
        config.app_role_assignments.enabled = false;
        config.cache.memberships_enabled = false;
        assert!(config.validate().is_ok());
    }

    #[test]
    fn access_log_headers_default_is_empty() {
        let config = base();
        assert!(!config.access_log.enabled);
        assert!(config.access_log.headers.0.is_empty());
    }

    #[test]
    fn access_log_headers_parses_json_and_lowercases_header_names() {
        let headers: AccessLogHeaders =
            r#"{"ip":"X-Forwarded-For","ua":"User-Agent"}"#.parse().unwrap();
        assert_eq!(
            headers.0,
            vec![
                ("ip".to_owned(), "x-forwarded-for".to_owned()),
                ("ua".to_owned(), "user-agent".to_owned()),
            ]
        );
    }

    #[test]
    fn access_log_headers_drops_credentials_and_reserved_fields() {
        let headers: AccessLogHeaders =
            r#"{"ip":"x-forwarded-for","secret":"Authorization","session":"Cookie","transport":"x-transport","empty":""}"#
                .parse()
                .unwrap();
        assert_eq!(
            headers.0,
            vec![("ip".to_owned(), "x-forwarded-for".to_owned())]
        );
    }

    #[test]
    fn access_log_headers_rejects_invalid_json() {
        assert!("not json".parse::<AccessLogHeaders>().is_err());
    }

    #[test]
    fn forward_headers_splits_trims_and_lowercases() {
        let headers: ForwardHeaders = " X-Request-Id , X-Forwarded-For ,, ".parse().unwrap();
        assert_eq!(
            headers.0,
            vec!["x-request-id".to_owned(), "x-forwarded-for".to_owned()]
        );
    }

    #[test]
    fn forward_headers_default_is_empty() {
        assert!(base().forward_headers.0.is_empty());
    }
}
