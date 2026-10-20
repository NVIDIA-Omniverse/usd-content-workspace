// SPDX-FileCopyrightText: Copyright (c) 2024 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Shared `BuildError` for the per-layer caches. Each layer (`stat_cache`,
//! `list_cache`, `small_object_cache`, `metadata_cache`, `manifest`) re-exports
//! this type so callers see one error shape across layers.

use snafu::Snafu;

use super::error::Error as LowLevelError;
use super::redis_config::BuildClientError;

#[derive(Debug, Snafu)]
pub enum LayerBuildError {
    #[snafu(display("layer figment config error: {source}"))]
    Figment { source: figment::Error },

    #[snafu(display("layer cache backend error: {source}"))]
    Cache { source: LowLevelError },

    #[snafu(display("layer redis client error: {source}"))]
    Redis { source: BuildClientError },
}

impl From<figment::Error> for LayerBuildError {
    fn from(source: figment::Error) -> Self {
        LayerBuildError::Figment { source }
    }
}

impl From<LowLevelError> for LayerBuildError {
    fn from(source: LowLevelError) -> Self {
        LayerBuildError::Cache { source }
    }
}

impl From<BuildClientError> for LayerBuildError {
    fn from(source: BuildClientError) -> Self {
        LayerBuildError::Redis { source }
    }
}
