// SPDX-FileCopyrightText: Copyright (c) 2024-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
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

use opentelemetry::metrics::Meter;

pub struct BogusInstrumentProvider;

impl BogusInstrumentProvider {
    pub fn meter() -> Meter {
        let instrument_provider = std::sync::Arc::new(BogusInstrumentProvider);
        Meter::new(instrument_provider)
    }
}

impl opentelemetry::metrics::InstrumentProvider for BogusInstrumentProvider {}

impl opentelemetry::metrics::MeterProvider for BogusInstrumentProvider {
    fn meter_with_scope(&self, _scope: opentelemetry::InstrumentationScope) -> Meter {
        Self::meter()
    }
}
