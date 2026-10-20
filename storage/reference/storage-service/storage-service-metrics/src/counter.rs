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

use std::sync::atomic::{AtomicU64, Ordering};

use storage_core_util::observability::metrics::BoundUIntGauge;

pub struct UpDownCounter {
    instrument: BoundUIntGauge,
    value: AtomicU64,
}

impl UpDownCounter {
    pub fn new(instrument: BoundUIntGauge) -> Self {
        Self {
            instrument,
            value: Default::default(),
        }
    }

    pub fn inc_scoped<'a>(&'a self) -> UpDownCounterIncGuard<'a> {
        self.inc();
        UpDownCounterIncGuard { counter: self }
    }

    pub fn inc(&self) {
        let value = self.value.fetch_add(1, Ordering::Relaxed) + 1;
        self.instrument.record(value);
    }

    pub fn dec(&self) {
        let value = self.value.fetch_sub(1, Ordering::Relaxed) - 1;
        self.instrument.record(value);
    }
}

pub struct UpDownCounterIncGuard<'a> {
    counter: &'a UpDownCounter,
}

impl Drop for UpDownCounterIncGuard<'_> {
    fn drop(&mut self) {
        self.counter.dec();
    }
}
