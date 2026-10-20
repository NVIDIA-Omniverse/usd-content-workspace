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

use crate::types::metrics::AggregationTemporality;

pub(crate) fn serialize<S>(
    aggregation_temporality: &Result<AggregationTemporality, i32>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let v = match aggregation_temporality {
        Ok(at) => *at as i32,
        Err(at) => *at,
    };
    serializer.serialize_i32(v)
}

pub(crate) fn deserialize<'de, D>(
    deserializer: D,
) -> Result<Result<AggregationTemporality, i32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize as _;
    <i32>::deserialize(deserializer).map(crate::types::metrics::agg_temp)
}
