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

use time::format_description::well_known::Rfc3339;

pub fn deserialize<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::de::Deserializer<'de>,
{
    use serde::Deserialize as _;
    let value = <&str>::deserialize(deserializer)?;
    if !value.ends_with('Z') {
        let msg = format!("datetime does not end with `Z`: `{value}`");
        return Err(serde::de::Error::custom(msg));
    }
    let dt = time::OffsetDateTime::parse(value, &Rfc3339).map_err(serde::de::Error::custom)?;
    u64::try_from(dt.unix_timestamp_nanos()).map_err(|_e| {
        let msg = format!("datetime out of range: `{value}`");
        serde::de::Error::custom(msg)
    })
}
