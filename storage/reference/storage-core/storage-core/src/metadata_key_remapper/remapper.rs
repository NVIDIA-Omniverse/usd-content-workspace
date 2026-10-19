// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;

use figment::Figment;
use serde::{Deserialize, Serialize};
use storage_core_builder::layer::Layer;
use storage_core_util::info_limited;

/// Configuration for the metadata key remapper.
///
/// The configuration key is `metadata_key_remapper` when extracting from a Figment struct.
///
/// Each entry in `mappings` maps a `client_key` (the key the client uses in API requests
/// and sees in responses) to a `storage_key` (the key actually stored in the backend).
/// Two `client_key`s mapping to the same `storage_key` is a configuration error because
/// reverse translation would be ambiguous.
///
/// Example configuration:
/// ```toml
/// [metadata_key_remapper]
/// enabled = true
///
/// [metadata_key_remapper.mappings]
/// "nv-omniverse-nucleus-hash" = "nv_omniverse_nucleus_hash"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Enable the metadata key remapper layer.
    /// Default is true.
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Map from `client_key` (request/response side) to `storage_key` (backend side).
    /// Default is an empty map (layer is a no-op).
    #[serde(default)]
    pub mappings: HashMap<String, String>,
}

fn default_enabled() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            mappings: HashMap::new(),
        }
    }
}

impl Config {
    pub const CONFIG_KEY: &str = "metadata_key_remapper";
}

impl TryFrom<&Figment> for Config {
    type Error = figment::Error;

    fn try_from(figment: &Figment) -> std::result::Result<Self, Self::Error> {
        figment.focus(Self::CONFIG_KEY).extract()
    }
}

/// Layer that translates metadata keys between a client-facing namespace and a
/// backend-facing namespace.
///
/// On requests, `client_key` references in the request are rewritten to `storage_key`s
/// before being passed to the inner service. On responses, `storage_key`s in the
/// response map are rewritten back to `client_key`s. Keys that do not appear in the
/// configured `mappings` pass through untouched.
///
/// Supports the following traits:
/// - `v1alpha::Metadata`
pub struct MetadataKeyRemapperLayer {
    pub forward: HashMap<String, String>,
    pub reverse: HashMap<String, String>,
}

impl MetadataKeyRemapperLayer {
    pub fn new(config: Config) -> std::result::Result<Option<Self>, figment::Error> {
        if !config.enabled {
            return Ok(None);
        }

        let mut reverse: HashMap<String, String> = HashMap::with_capacity(config.mappings.len());
        for (client_key, storage_key) in &config.mappings {
            if let Some(existing) = reverse.insert(storage_key.clone(), client_key.clone()) {
                return Err(figment::Error::from(format!(
                    "metadata_key_remapper: storage_key {storage_key:?} is mapped from multiple \
                     client_keys ({existing:?} and {client_key:?}); reverse translation would be ambiguous"
                )));
            }
        }

        info_limited!(mappings = config.mappings.len(), "Metadata key remapper config");

        Ok(Some(Self {
            forward: config.mappings,
            reverse,
        }))
    }

    /// Creates a new MetadataKeyRemapperLayer from a Figment struct.
    ///
    /// The config will be extracted from the Figment struct, looking for the
    /// `metadata_key_remapper` key.
    pub fn from_figment(figment: &Figment) -> std::result::Result<Option<Self>, figment::Error> {
        let config = Config::try_from(figment)?;
        Self::new(config)
    }
}

impl<S> Layer<S> for MetadataKeyRemapperLayer {
    type Service = MetadataKeyRemapper<S>;
    fn layer(&self, inner: S) -> Self::Service {
        MetadataKeyRemapper {
            inner,
            forward: self.forward.clone(),
            reverse: self.reverse.clone(),
        }
    }
}

pub struct MetadataKeyRemapper<S> {
    pub inner: S,
    pub forward: HashMap<String, String>,
    pub reverse: HashMap<String, String>,
}

impl<S> MetadataKeyRemapper<S> {
    /// Translate a single key client → storage. Returns the original key if no mapping exists.
    pub(crate) fn to_storage_key(&self, client_key: &str) -> String {
        self.forward
            .get(client_key)
            .cloned()
            .unwrap_or_else(|| client_key.to_string())
    }

    /// Translate a single key storage → client. Returns the original key if no mapping exists.
    pub(crate) fn to_client_key(&self, storage_key: String) -> String {
        match self.reverse.get(&storage_key) {
            Some(client) => client.clone(),
            None => storage_key,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with(mappings: &[(&str, &str)]) -> Config {
        Config {
            enabled: true,
            mappings: mappings.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect(),
        }
    }

    #[test]
    fn disabled_returns_none() {
        let config = Config {
            enabled: false,
            ..Default::default()
        };
        assert!(MetadataKeyRemapperLayer::new(config).unwrap().is_none());
    }

    #[test]
    fn empty_mappings_is_ok() {
        let layer = MetadataKeyRemapperLayer::new(Config::default()).unwrap().unwrap();
        assert!(layer.forward.is_empty());
        assert!(layer.reverse.is_empty());
    }

    #[test]
    fn duplicate_storage_key_is_rejected() {
        let config = config_with(&[("client-a", "shared"), ("client-b", "shared")]);
        let err = match MetadataKeyRemapperLayer::new(config) {
            Err(e) => e,
            Ok(_) => panic!("expected duplicate-storage-key error"),
        };
        let msg = err.to_string();
        assert!(msg.contains("shared"), "unexpected error: {msg}");
    }

    #[test]
    fn reverse_map_is_built_from_forward() {
        let config = config_with(&[("nv-omniverse-nucleus-hash", "nv_omniverse_nucleus_hash")]);
        let layer = MetadataKeyRemapperLayer::new(config).unwrap().unwrap();
        assert_eq!(
            layer.forward.get("nv-omniverse-nucleus-hash").map(String::as_str),
            Some("nv_omniverse_nucleus_hash"),
        );
        assert_eq!(
            layer.reverse.get("nv_omniverse_nucleus_hash").map(String::as_str),
            Some("nv-omniverse-nucleus-hash"),
        );
    }
}
