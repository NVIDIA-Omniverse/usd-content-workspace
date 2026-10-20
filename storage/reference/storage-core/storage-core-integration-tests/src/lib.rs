// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

pub mod mocks;
pub mod names;
pub mod observability_harness;
pub mod service;
#[macro_use]
pub mod fixture;
pub mod utils;

use std::sync::OnceLock;

use fixture::StorageFixture;
use rstest::*;
use storage_core_test::metrics::InMemoryMetrics;

static GLOBAL_METRICS_EXPORTER: OnceLock<InMemoryMetrics> = OnceLock::new();

pub fn global_metrics_exporter() -> &'static InMemoryMetrics {
    GLOBAL_METRICS_EXPORTER.get_or_init(InMemoryMetrics::new)
}

#[fixture]
#[once]
pub fn storage_fixture() -> StorageFixture {
    StorageFixture::new()
}

#[fixture]
#[once]
pub fn assume_role_storage_fixture() -> StorageFixture {
    let role_arn = std::env::var("OMNI_STORAGE_SERVICE_ROLE_ARN")
        .expect("OMNI_STORAGE_SERVICE_ROLE_ARN not set. Assume role tests required this to be set. Please run 'source tools/vault/local_env.sh'");
    StorageFixture::with_assumed_role(Some(role_arn))
}

#[fixture]
#[once]
pub fn cdn_storage_fixture() -> StorageFixture {
    StorageFixture::with_cdn()
}
