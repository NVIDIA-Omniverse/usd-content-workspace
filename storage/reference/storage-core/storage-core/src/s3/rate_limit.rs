// SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use crate::utils::rate_limit_counter_base;
use aws_smithy_runtime_api::box_error::BoxError;
use aws_smithy_runtime_api::client::interceptors::context::BeforeDeserializationInterceptorContextRef;
use aws_smithy_runtime_api::client::interceptors::Intercept;
use aws_smithy_runtime_api::client::runtime_components::RuntimeComponents;
use aws_smithy_types::config_bag::ConfigBag;
use storage_core_util::observability::metrics::{BoundUIntCounter, MetricBinder, UIntCounter};

#[derive(Clone)]
pub(crate) struct RateLimitInterceptor {
    counter: BoundUIntCounter,
}

impl std::fmt::Debug for RateLimitInterceptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RateLimitInterceptor").finish()
    }
}

impl RateLimitInterceptor {
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
}

impl Intercept for RateLimitInterceptor {
    fn name(&self) -> &'static str {
        "RateLimitInterceptor"
    }

    fn read_after_transmit(
        &self,
        context: &BeforeDeserializationInterceptorContextRef<'_>,
        _runtime_components: &RuntimeComponents,
        _cfg: &mut ConfigBag,
    ) -> Result<(), BoxError> {
        let status = context.response().status().as_u16();
        if status == 429 || status == 503 {
            self.counter.add(1);
            storage_core_util::info_limited!(status, "S3 rate limit response detected");
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RateLimitInterceptors {
    pub head_object: RateLimitInterceptor,
    pub get_object: RateLimitInterceptor,
    pub put_object: RateLimitInterceptor,
    pub list_objects_v2: RateLimitInterceptor,
    pub list_object_versions: RateLimitInterceptor,
    pub delete_object: RateLimitInterceptor,
    pub copy_object: RateLimitInterceptor,
    pub create_multipart_upload: RateLimitInterceptor,
    pub complete_multipart_upload: RateLimitInterceptor,
    pub abort_multipart_upload: RateLimitInterceptor,
}

impl RateLimitInterceptors {
    pub fn new(storage_backend: &str) -> Self {
        let base = rate_limit_counter_base();
        Self {
            head_object: RateLimitInterceptor::new(base, "head_object", storage_backend),
            get_object: RateLimitInterceptor::new(base, "get_object", storage_backend),
            put_object: RateLimitInterceptor::new(base, "put_object", storage_backend),
            list_objects_v2: RateLimitInterceptor::new(base, "list_objects_v2", storage_backend),
            list_object_versions: RateLimitInterceptor::new(base, "list_object_versions", storage_backend),
            delete_object: RateLimitInterceptor::new(base, "delete_object", storage_backend),
            copy_object: RateLimitInterceptor::new(base, "copy_object", storage_backend),
            create_multipart_upload: RateLimitInterceptor::new(base, "create_multipart_upload", storage_backend),
            complete_multipart_upload: RateLimitInterceptor::new(base, "complete_multipart_upload", storage_backend),
            abort_multipart_upload: RateLimitInterceptor::new(base, "abort_multipart_upload", storage_backend),
        }
    }
}
