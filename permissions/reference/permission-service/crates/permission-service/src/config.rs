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

use crate::cache::CacheArgs;
use crate::events::consumer::EventConsumerArgs;
use crate::events::publisher::EventPublishingArgs;
use crate::grpc::GrpcArgs;
use crate::metrics::MetricsArgs;
use crate::traces::TracingArgs;
use clap::{ArgAction, Args, Parser, ValueEnum};
use permission_auth::config::AuthConfig;
use permission_auth::interceptors::access_token_file::AccessTokenFileArgs;
use permission_auth::interceptors::client_credentials::ClientCredentialsArgs;
use permission_db::postgres::PostgresArgs;
use std::time::Duration;

/// Defines the database backend type used by the service.
#[derive(Clone, Debug, Default, ValueEnum)]
pub enum DatabaseType {
    /// Use PostgreSQL as the storage backend.
    #[default]
    Postgres,
    /// Use read-only YAML configuration files as the storage backend.
    ConfigFile,
}

#[derive(Clone, Debug, Parser)]
#[command(version, about, long_about = None)]
pub struct ServiceConfig {
    #[command(flatten)]
    pub access_log: crate::access_log::AccessLogArgs,

    #[command(flatten)]
    pub auth: AuthConfig,

    #[command(flatten)]
    pub cache: CacheArgs,

    #[command(flatten)]
    pub access_token_file: AccessTokenFileArgs,

    #[command(flatten)]
    pub client_credentials: ClientCredentialsArgs,

    #[command(flatten)]
    pub database: DatabaseArgs,

    #[command(flatten)]
    pub client_timeouts: ClientTimeoutsArgs,

    #[command(flatten)]
    pub grpc: GrpcArgs,

    #[command(flatten)]
    pub metrics: MetricsArgs,

    #[command(flatten)]
    pub publishing: EventPublishingArgs,

    #[command(flatten)]
    pub consumer: EventConsumerArgs,

    #[command(flatten)]
    pub policies: PolicyArgs,

    #[command(flatten)]
    pub postgres: PostgresArgs,

    #[command(flatten)]
    pub service: ServiceArgs,

    #[command(flatten)]
    pub tracing: TracingArgs,

    #[command(flatten)]
    pub url_remapper: UrlRemapperArgs,
}

/// Timeouts applied to outbound HTTP clients constructed by the service.
///
/// Each field corresponds to a distinct client type the service uses for
/// outbound traffic. Bounding these timeouts prevents a slow or unreachable
/// upstream from indefinitely blocking the request path and wedging worker
/// threads (security review issue #4). New client types should be added as
/// additional fields here so that operators have a single, predictable
/// place to tune all outbound HTTP behavior.
///
/// Mirrors the `config.clientTimeouts` section in the Helm chart.
#[derive(Args, Clone, Debug)]
pub struct ClientTimeoutsArgs {
    #[command(flatten)]
    pub idp: IdpClientTimeoutsArgs,
}

/// HTTP client timeouts for outbound calls to the identity provider
/// (OpenID discovery, JWKS, userinfo, client-credentials token endpoints).
///
/// All IdP-bound traffic from `permission-auth` and the client-credentials
/// `TokenManager` flows through a single `reqwest::Client` built from these
/// values.
#[derive(Args, Clone, Debug)]
pub struct IdpClientTimeoutsArgs {
    /// Maximum time to wait for a TCP connection to the identity provider.
    #[arg(
        id = "idp-connect-timeout",
        long = "idp-connect-timeout",
        env = "IDP_CONNECT_TIMEOUT",
        default_value = "5s",
        value_parser = humantime::parse_duration,
    )]
    pub connect: Duration,

    /// Maximum total time for a single HTTP request to the identity provider
    /// (connect + send + receive).
    #[arg(
        id = "idp-request-timeout",
        long = "idp-request-timeout",
        env = "IDP_REQUEST_TIMEOUT",
        default_value = "15s",
        value_parser = humantime::parse_duration,
    )]
    pub request: Duration,

    /// How long an idle keep-alive HTTP connection to the identity provider
    /// is kept in the connection pool before being closed.
    #[arg(
        id = "idp-pool-idle-timeout",
        long = "idp-pool-idle-timeout",
        env = "IDP_POOL_IDLE_TIMEOUT",
        default_value = "90s",
        value_parser = humantime::parse_duration,
    )]
    pub idle: Duration,
}

impl IdpClientTimeoutsArgs {
    /// Builds a `reqwest::Client` configured with the IdP-bound timeouts.
    pub fn build_client(&self) -> Result<reqwest::Client, reqwest::Error> {
        reqwest::Client::builder()
            .connect_timeout(self.connect)
            .timeout(self.request)
            .pool_idle_timeout(self.idle)
            .build()
    }
}

#[derive(Args, Clone, Debug)]
pub struct DatabaseArgs {
    /// The database backend type used by the service.
    /// Set to "postgres" to use PostgreSQL or "config-file" to use read-only YAML configuration files.
    #[arg(
        id = "database-type",
        long = "database-type",
        env = "DATABASE_TYPE",
        value_enum,
        default_value_t = DatabaseType::Postgres
    )]
    pub database_type: DatabaseType,

    #[command(flatten)]
    pub init: DatabaseInitArgs,
}

#[derive(Args, Clone, Debug)]
pub struct DatabaseInitArgs {
    /// Path to a YAML file containing policies to initialize in the database on startup.
    /// In config-file mode, this file is used as the policy storage backend.
    /// In postgres mode, policies from this file are loaded as system policies.
    #[arg(
        id = "init-policies-file",
        long = "init-policies-file",
        env = "INIT_POLICIES_FILE",
        default_value = ""
    )]
    pub policies_file: String,

    /// Path to a YAML file containing metadata to initialize in the database on startup.
    /// In config-file mode, this file is used as the metadata storage backend.
    #[arg(
        id = "init-metadata-file",
        long = "init-metadata-file",
        env = "INIT_METADATA_FILE",
        default_value = ""
    )]
    pub metadata_file: String,
}

#[derive(Clone, Debug, Parser)]
pub struct ServiceArgs {
    #[arg(long, env = "HOSTNAME", default_value = "127.0.0.1")]
    pub hostname: String,

    /// Service name defined in metrics and traces.
    #[arg(
        id = "service-name",
        long = "service-name",
        env = "SERVICE_NAME",
        default_value = "permission-service"
    )]
    pub name: String,

    /// Service scope defined in metrics and traces.
    #[arg(
        id = "service-scope",
        long = "service-scope",
        env = "SERVICE_SCOPE",
        default_value = "permission_service"
    )]
    pub scope: String,

    /// Amount of worker threads specified for the service runtime.
    /// This should not include worker threads for postgres dedicated runtime
    /// that are specified in `postgres_worker_threads`.
    #[arg(
        id = "service-worker-threads",
        long = "service-worker-threads",
        env = "SERVICE_WORKER_THREADS",
        default_value_t = 32
    )]
    pub worker_threads: usize,

    /// Stack size (in bytes) for each worker thread in the service runtime.
    #[arg(
        id = "service-thread-stack-size",
        long = "service-thread-stack-size",
        env = "SERVICE_THREAD_STACK_SIZE",
        default_value_t = 4 * 1024 * 1024
    )]
    pub thread_stack_size: usize,

    /// Number of freshly evaluated (cache-miss) resources in a request above
    /// which evaluation is spread across CPU cores in parallel. Requests at or
    /// below this size are evaluated sequentially on the current task, avoiding
    /// a blocking-pool handoff that yields no parallelism benefit for a single
    /// resource.
    #[arg(
        id = "parallel-evaluation-threshold",
        long = "parallel-evaluation-threshold",
        env = "PARALLEL_EVALUATION_THRESHOLD",
        default_value_t = 1
    )]
    pub parallel_evaluation_threshold: usize,
}

/// Arguments controlling forward URL-prefix remapping of resource IDs.
///
/// Mirrors the `config.urlRemapper` block in the Helm chart and matches the
/// naming used by storage-service so a single Helm values fragment can drive
/// both services. When `enabled` is true the service loads the YAML file
/// pointed to by `file` at startup and applies the longest-matching rule to
/// every resource id flowing through the policy, authorization, and
/// diagnostics APIs.
#[derive(Args, Clone, Debug)]
pub struct UrlRemapperArgs {
    /// Enables forward URL-prefix remapping of resource IDs.
    #[arg(
        id = "enable-url-remapper",
        long = "enable-url-remapper",
        env = "ENABLE_URL_REMAPPER",
        action = ArgAction::Set,
        default_value_t = false,
        default_missing_value = "true",
        num_args = 0..=1,
        require_equals = false,
        requires_ifs = [("true", "url-remapper-file")],
    )]
    pub enabled: bool,

    /// Path to the YAML file that defines URL-prefix rewrite rules.
    /// Required when `enable-url-remapper` is true.
    #[arg(
        id = "url-remapper-file",
        long = "url-remapper-file",
        env = "URL_REMAPPER_FILE",
        default_value = ""
    )]
    pub file: String,
}

#[derive(Args, Clone, Debug)]
pub struct PolicyArgs {
    /// Toggles returning the "reason" field for denied authorization requests.
    #[arg(long,
        env = "ENABLE_DENY_REASON",
        action = ArgAction::Set,
        default_value_t = true,
        default_missing_value = "true",
        num_args = 0..=1,
        require_equals = false
    )]
    pub enable_deny_reason: bool,

    /// Determines if submitted policies need to be validated
    /// before inserted to the database ("true" by default).
    #[arg(long,
        env = "POLICY_VALIDATION",
        action = ArgAction::Set,
        default_value_t = true,
        default_missing_value = "true",
        num_args = 0..=1,
        require_equals = false
    )]
    pub policy_validation: bool,

    /// Default order assigned to policies when no explicit order is specified.
    /// Lower values are evaluated first (firewall-style ordering).
    #[arg(long, env = "DEFAULT_POLICY_ORDER", default_value_t = 0)]
    pub default_policy_order: i32,
}

impl ServiceConfig {
    pub fn opentelemetry_resource(&self) -> opentelemetry_sdk::Resource {
        opentelemetry_sdk::Resource::builder()
            .with_service_name(self.service.name.clone())
            .with_attribute(opentelemetry::KeyValue::new(
                opentelemetry_semantic_conventions::resource::SERVICE_INSTANCE_ID,
                self.service.hostname.clone(),
            ))
            .with_attribute(opentelemetry::KeyValue::new(
                opentelemetry_semantic_conventions::resource::SERVICE_VERSION,
                env!("CARGO_PKG_VERSION"),
            ))
            .build()
    }

    pub fn opentelemetry_scope(&self) -> opentelemetry::InstrumentationScope {
        opentelemetry::InstrumentationScope::builder(self.service.name.clone())
            .with_version(env!("CARGO_PKG_VERSION"))
            .build()
    }
}
