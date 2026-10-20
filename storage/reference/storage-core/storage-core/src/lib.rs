// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

// Core modules - always available
pub mod api_services;
pub mod ingress;
pub mod layer;
pub mod service_registry;
pub mod storage;
pub mod traits;
pub mod utils;

// Backend modules - conditionally compiled
#[cfg(feature = "azure")]
pub mod azure;

#[cfg(feature = "s3")]
pub mod s3;

// Middleware modules - conditionally compiled
#[cfg(feature = "additional_headers")]
pub mod additional_headers;

#[cfg(feature = "egress_throttle")]
pub mod egress_throttle;

#[cfg(feature = "list_cache")]
pub mod list_cache;

#[cfg(feature = "manifest")]
pub mod manifest;

#[cfg(feature = "metrics")]
pub mod metrics;

#[cfg(feature = "small_object_cache")]
pub mod small_object_cache;

#[cfg(feature = "metadata_cache")]
pub mod metadata_cache;

#[cfg(feature = "metadata_key_remapper")]
pub mod metadata_key_remapper;

#[cfg(feature = "metadata_key_router")]
pub mod metadata_key_router;

#[cfg(feature = "stat_cache")]
pub mod stat_cache;

#[cfg(test)]
mod tests {
    macro_rules! l2_config_tests {
        ($name:ident, $config:path) => {
            #[test]
            fn $name() {
                let default = <$config>::default();
                assert!(!default.l2_enabled);

                let enabled: $config = serde_json::from_str(r#"{"l2_enabled":true}"#).expect("deserialize config");
                assert!(enabled.l2_enabled);

                let args: storage_core_util::cache::LayerCacheArgs = enabled.into();
                assert!(args.l2_enabled);
            }
        };
    }

    #[cfg(feature = "list_cache")]
    l2_config_tests!(
        list_cache_l2_config_defaults_and_deserializes,
        crate::list_cache::Config
    );

    #[cfg(feature = "manifest")]
    l2_config_tests!(manifest_l2_config_defaults_and_deserializes, crate::manifest::Config);

    #[cfg(feature = "metadata_cache")]
    l2_config_tests!(
        metadata_cache_l2_config_defaults_and_deserializes,
        crate::metadata_cache::Config
    );

    #[cfg(feature = "small_object_cache")]
    l2_config_tests!(
        small_object_cache_l2_config_defaults_and_deserializes,
        crate::small_object_cache::Config
    );

    #[cfg(feature = "stat_cache")]
    l2_config_tests!(
        stat_cache_l2_config_defaults_and_deserializes,
        crate::stat_cache::Config
    );
}
