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

use std::future::Future;

use storage_core_util::observability::metrics::BoundTimerMetric;

use crate::counter::UpDownCounter;

pub trait FutureExt {
    type Output;

    fn with_active_call_count_metric<'a>(self, metric: &'a UpDownCounter) -> impl Future<Output = Self::Output> + 'a
    where
        Self: 'a;

    fn with_latency_metric<'a>(self, metric: &'a BoundTimerMetric) -> impl Future<Output = Self::Output> + 'a
    where
        Self: 'a;
}

impl<Fut> FutureExt for Fut
where
    Fut: Future,
{
    type Output = Fut::Output;

    async fn with_active_call_count_metric<'a>(self, metric: &'a UpDownCounter) -> Self::Output
    where
        Self: 'a,
    {
        let _guard = metric.inc_scoped();
        self.await
    }

    async fn with_latency_metric<'a>(self, metric: &'a BoundTimerMetric) -> Self::Output
    where
        Self: 'a,
    {
        let _guard = metric.start();
        self.await
    }
}
