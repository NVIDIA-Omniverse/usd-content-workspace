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
use serde::de;
use std::fmt;

struct Visitor;

impl de::Visitor<'_> for Visitor {
    type Value = TraceId;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a trace ID")
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        TraceId::from_hex(v).map_err(E::custom)
    }
}

struct OptVisitor;

impl<'a> de::Visitor<'a> for OptVisitor {
    type Value = Option<TraceId>;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a trace ID or none")
    }

    fn visit_some<D: de::Deserializer<'a>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_any(Visitor).map(Some)
    }

    fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(None)
    }

    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(None)
    }
}

pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<TraceId>, D::Error>
where
    D: de::Deserializer<'de>,
{
    deserializer.deserialize_option(OptVisitor)
}
