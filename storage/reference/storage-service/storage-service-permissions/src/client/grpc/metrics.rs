// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
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

use std::fmt::Debug;
use storage_core_util::observability::metrics::{BoundTimerMetric, BoundUIntHistogram};
use storage_service_metrics::{
    batch_size_metric, call_latency_metric, grpc_operation_result_metrics, status::grpc::ResultMetrics,
    utils::BATCH_SIZE_BUCKETS_FINE,
};

pub struct Metrics {
    pub check_permission_batch_latency: BoundTimerMetric,
    pub check_permission_batch_count: ResultMetrics,
    pub check_permission_batch_batch_size: BoundUIntHistogram,
}

impl Metrics {
    pub fn new() -> Self {
        Self {
            check_permission_batch_latency: call_latency_metric!(
                "storage_permission_grpc_check_permission_batch_latency",
                "Permission Service CheckPermissionBatch operation latency",
            ),
            check_permission_batch_count: grpc_operation_result_metrics!(
                "storage_permission_grpc_check_permission_batch_count",
                "Permission Service CheckPermissionBatch operation results",
            ),
            check_permission_batch_batch_size: batch_size_metric!(
                "storage_permission_grpc_check_permission_batch_batch_size",
                "Permission Service CheckPermissionBatch request size",
                &BATCH_SIZE_BUCKETS_FINE as &[f64],
            ),
        }
    }
}

impl Debug for Metrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Metrics").finish_non_exhaustive()
    }
}
