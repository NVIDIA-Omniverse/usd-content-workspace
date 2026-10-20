// SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use storage_core_util::observability::metrics::{BoundUIntCounter, MetricBinder, UIntCounter};

use crate::utils::rate_limit_counter_base;

#[derive(Clone)]
pub(crate) struct RateLimitCounter {
    counter: BoundUIntCounter,
}

impl std::fmt::Debug for RateLimitCounter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RateLimitCounter").finish()
    }
}

impl RateLimitCounter {
    fn new(base: &UIntCounter, method: &str, storage_backend: &str) -> Self {
        let pod_name = std::env::var("STORAGE_POD_NAME").unwrap_or("pod".to_string());
        let pkg_version = env!("CARGO_PKG_VERSION").to_string();
        Self {
            counter: base.bind([
                ("sdk_operation", method.to_string()),
                ("storage_backend", storage_backend.to_string()),
                ("pod_name", pod_name),
                ("pkg_version", pkg_version),
            ]),
        }
    }

    pub(crate) fn check_error(&self, err: &azure_core::Error) {
        if let azure_storage::ErrorKind::HttpResponse { status, .. } = err.kind() {
            if *status == azure_core::StatusCode::TooManyRequests
                || *status == azure_core::StatusCode::ServiceUnavailable
            {
                self.counter.add(1);
                storage_core_util::info_limited!(%status, "Azure rate limit response detected");
            }
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RateLimitCounters {
    pub get_properties: RateLimitCounter,
    pub get_blob: RateLimitCounter,
    pub put_block_blob: RateLimitCounter,
    pub copy_blob: RateLimitCounter,
    pub delete_blob: RateLimitCounter,
    pub put_block_list: RateLimitCounter,
    pub list_blobs: RateLimitCounter,
}

impl RateLimitCounters {
    pub fn new(storage_backend: &str) -> Self {
        let base = rate_limit_counter_base();
        Self {
            get_properties: RateLimitCounter::new(base, "get_properties", storage_backend),
            get_blob: RateLimitCounter::new(base, "get_blob", storage_backend),
            put_block_blob: RateLimitCounter::new(base, "put_block_blob", storage_backend),
            copy_blob: RateLimitCounter::new(base, "copy_blob", storage_backend),
            delete_blob: RateLimitCounter::new(base, "delete_blob", storage_backend),
            put_block_list: RateLimitCounter::new(base, "put_block_list", storage_backend),
            list_blobs: RateLimitCounter::new(base, "list_blobs", storage_backend),
        }
    }
}
