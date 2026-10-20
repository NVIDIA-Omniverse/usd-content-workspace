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

use opentelemetry::TraceId;

pub(crate) fn serialize<S>(trace_id: &TraceId, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let bytes = trace_id.to_bytes();
    serializer.serialize_bytes(&bytes)
}

pub(crate) fn deserialize<'de, D>(deserializer: D) -> Result<TraceId, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize as _;
    let trace_id = <&[u8]>::deserialize(deserializer)?;
    crate::types::span_context::try_make_trace_id(trace_id).map_err(serde::de::Error::custom)
}
