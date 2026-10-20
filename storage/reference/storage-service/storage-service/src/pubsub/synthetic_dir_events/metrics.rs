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

use storage_core_util::observability::metrics::{
    metric_base, Attribute, BoundTimerMetric, BoundUIntCounter, BoundUIntHistogram,
};
use storage_service_metrics::{
    batch_size_metric_base, response_count_metric_base,
    status::result::ResultMetrics,
    utils::{BindExt, BATCH_SIZE_BUCKETS_FINE},
};

pub struct Metrics {
    pub accumulated_created: BoundUIntCounter,
    pub accumulated_deleted: BoundUIntCounter,
    pub publish_results_created: ResultMetrics,
    pub publish_results_deleted: ResultMetrics,
    pub flush_batch_size_created: BoundUIntHistogram,
    pub flush_batch_size_deleted: BoundUIntHistogram,
    pub flush_duration: BoundTimerMetric,
}

impl Metrics {
    pub fn new() -> Self {
        let accumulated = {
            metric_base!(
                static ACCUMULATED: UIntCounter,
                name = "storage_synthetic_dir_events_accumulated",
                description = "Number of possible dir paths accumulated (per-insertion, pre-dedup)",
                { pod, kind }
            );
            &*ACCUMULATED
        };

        let publish_results = response_count_metric_base!(
            "storage_synthetic_dir_events_publish_results",
            "Per-entry batch publish outcome (Ok/Err) for synthetic dir events",
            { kind },
        );

        let flush_batch_size = batch_size_metric_base!(
            "storage_synthetic_dir_events_flush_batch_size",
            "Size of dir event batches at each flush (zero-size flushes included)",
            &BATCH_SIZE_BUCKETS_FINE as &'static [f64],
            { kind },
        );

        let flush_duration = {
            metric_base!(
                static FLUSH_DURATION: TimerMetric,
                name = "storage_synthetic_dir_events_flush_duration",
                { pod }
            );
            (*FLUSH_DURATION).bind_default_attributes()
        };

        Self {
            accumulated_created: accumulated.bind_with_default_attributes([Attribute::new("kind", "created")]),
            accumulated_deleted: accumulated.bind_with_default_attributes([Attribute::new("kind", "deleted")]),
            publish_results_created: ResultMetrics::new_bind(|result| {
                publish_results
                    .bind_with_default_attributes([Attribute::new("kind", "created"), Attribute::new("result", result)])
            }),
            publish_results_deleted: ResultMetrics::new_bind(|result| {
                publish_results
                    .bind_with_default_attributes([Attribute::new("kind", "deleted"), Attribute::new("result", result)])
            }),
            flush_batch_size_created: flush_batch_size
                .bind_with_default_attributes([Attribute::new("kind", "created")]),
            flush_batch_size_deleted: flush_batch_size
                .bind_with_default_attributes([Attribute::new("kind", "deleted")]),
            flush_duration,
        }
    }
}
