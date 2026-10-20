// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use figment::Figment;
use serde::{Deserialize, Serialize};
use storage_core_util::{error_limited, info_limited, warn_limited};
use tonic::metadata::KeyAndValueRef;

use storage_core_builder::{layer::Layer, traits::Request};
use storage_core_util::additional_headers::AdditionalHeaders;

pub use crate::additional_headers::error::Error;

/// Configuration for the additional headers.
///
/// The configuration key is `additional_headers` when extracting from a Figment struct.
///
/// Example configuration:
/// ```toml
/// [additional_headers]
/// enabled = true
/// cache_control = "no-store"
/// cache_control_api = "no-cache, no-store"
/// tracing_header_names = ["x-request-id"]
/// env_header_names = ["MY_CUSTOM_HEADER", "DEPLOYMENT_ID"]
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Enable the additional headers.
    /// Default is true
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Add http header Cache-Control to be applied to all 'get' requests, either made by the storage service
    /// or returned as a presigned url.
    /// The Cache-Control HTTP header field holds directives (instructions)
    /// that control caching in shared caches (e.g. Proxies, CDNs).
    #[serde(default = "default_cache_control")]
    pub cache_control: Option<String>,

    /// Add http header Cache-Control to all requests made by the storage service to the storage backend.
    /// The Cache-Control HTTP header field holds directives (instructions)
    /// that control caching in shared caches (e.g. Proxies, CDNs).
    #[serde(default = "default_cache_control_api")]
    pub cache_control_api: Option<String>,

    /// Additional header names to extract from incoming requests and add to all outgoing requests and
    /// returned presigned urls.
    /// Default is ["x-request-id", "traceparent", "tracestate", "baggage"]
    ///
    /// The traceparent and tracestate headers are used to propagate the trace context.
    /// These are the standard W3C Trace Context headers <https://www.w3.org/TR/trace-context/>
    ///
    #[serde(default = "default_tracing_header_names")]
    pub tracing_header_names: Vec<String>,

    /// Environment variable names to resolve at startup and add as HTTP headers to all outgoing requests.
    /// Each entry is an environment variable name. The env var name (lowercased) is used as the header
    /// name, and the env var value is used as the header value. Variables that are not set will be
    /// skipped with a warning.
    /// Default is []
    #[serde(default = "default_env_header_names")]
    pub env_header_names: Vec<String>,
}

fn default_enabled() -> bool {
    true
}

fn default_cache_control() -> Option<String> {
    None
}

fn default_cache_control_api() -> Option<String> {
    Some("no-cache, no-store".to_string())
}

fn default_tracing_header_names() -> Vec<String> {
    vec![
        "x-request-id".to_string(),
        "traceparent".to_string(),
        "tracestate".to_string(),
        "baggage".to_string(),
    ]
}

fn default_env_header_names() -> Vec<String> {
    vec![]
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            cache_control: default_cache_control(),
            cache_control_api: default_cache_control_api(),
            tracing_header_names: default_tracing_header_names(),
            env_header_names: default_env_header_names(),
        }
    }
}

impl Config {
    pub const CONFIG_KEY: &str = "additional_headers";
}

impl TryFrom<&Figment> for Config {
    type Error = figment::Error;

    fn try_from(figment: &Figment) -> Result<Self, Self::Error> {
        figment.focus(Self::CONFIG_KEY).extract()
    }
}

/// Layer for parsing additional headers from the request and adding them to the response.
///
/// This layer is used to parse additional headers from the request and add them to the response.
/// It will also add the configured cache control headers to all outgoing requests and returned presigned urls.
///
/// Supports the following traits:
/// - `v1alpha::FileObject`
/// - `v1alpha::FileFolder`
/// - `v1alpha::Capabilities`
/// - `v1alpha::Versioning`
/// - `v1alpha::Metadata`
/// - `v1beta::FileObject`
/// - `v1beta::FileFolder`
/// - `v1beta::Capabilities`
/// - `v1beta::Versioning`
pub struct AdditionalHeadersLayer {
    pub config: Config,
}

impl AdditionalHeadersLayer {
    pub fn new(config: Config) -> Option<Self> {
        info_limited!(?config, "Additional headers config");
        if config.enabled {
            Some(Self { config })
        } else {
            None
        }
    }

    /// Creates a new AdditionalHeadersLayer from a Figment struct.
    ///
    /// The config will be extracted from the Figment struct, looking for the `additional_headers` key.
    pub fn from_figment(figment: &Figment) -> Result<Option<Self>, figment::Error> {
        let config = Config::try_from(figment)?;
        Ok(Self::new(config))
    }
}

impl<S> Layer<S> for AdditionalHeadersLayer {
    type Service = AdditionalHeaderManager<S>;
    fn layer(&self, inner: S) -> Self::Service {
        AdditionalHeaderManager::new(inner, self.config.clone())
    }
}

/// Manages parsing of request context information
#[derive(Debug)]
pub struct AdditionalHeaderManager<S> {
    pub inner: S,
    pub cache_control: Option<String>,
    pub cache_control_api: Option<String>,
    pub tracing_header_names: Vec<String>,
    pub env_headers: Vec<(String, String)>,
}

impl<S> AdditionalHeaderManager<S> {
    pub fn new(inner: S, config: Config) -> Self {
        let env_headers: Vec<(String, String)> = config
            .env_header_names
            .iter()
            .filter_map(|name| match std::env::var(name) {
                Ok(value) => {
                    info_limited!(env_var = %name, "Resolved env var for additional header");
                    Some((name.to_lowercase(), value))
                },
                Err(_) => {
                    warn_limited!(env_var = %name, "Environment variable not set, skipping header");
                    None
                },
            })
            .collect();

        Self {
            inner,
            cache_control: config.cache_control,
            cache_control_api: config.cache_control_api,
            tracing_header_names: config.tracing_header_names,
            env_headers,
        }
    }

    pub fn parse<T>(&self, request: &Request<T>) -> Result<AdditionalHeaders, Error> {
        let mut headers = Vec::with_capacity(request.metadata().len() + 1);

        // Add the tracing headers to the header map
        for h in request.metadata().iter() {
            match h {
                KeyAndValueRef::Ascii(key, value) => {
                    if self.tracing_header_names.iter().any(|k| k == key.as_str()) {
                        match value.to_str() {
                            Ok(v) => {
                                headers.push((key.to_string(), v.to_string()));
                            },
                            Err(e) => {
                                error_limited!(?value, error=?e, "Error parsing tracing header value");
                                return Err(e.into());
                            },
                        }
                    }
                },
                KeyAndValueRef::Binary(key, value) => {
                    if self.tracing_header_names.iter().any(|k| k == key.as_str()) {
                        match value.to_bytes() {
                            Ok(v) => match std::str::from_utf8(&v) {
                                Ok(s) => {
                                    headers.push((key.to_string(), s.to_string()));
                                },
                                Err(e) => {
                                    error_limited!(?value, error=?e, "Error parsing tracing header value");
                                    return Err(e.into());
                                },
                            },
                            Err(e) => {
                                error_limited!(?value, error=?e, "Error parsing tracing header value");
                                return Err(e.into());
                            },
                        }
                    }
                },
            }
        }

        Ok(AdditionalHeaders::new(
            headers,
            self.cache_control.clone(),
            self.cache_control_api.clone(),
            self.env_headers.clone(),
        ))
    }
}
