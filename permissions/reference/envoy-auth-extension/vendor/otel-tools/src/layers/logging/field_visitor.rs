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

use tracing::field::{Field, Visit};

/// A tracing visitor that records fields as JSON values.
#[derive(Debug)]
pub(crate) struct FieldVisitor<'a, S> {
    pub(crate) serializer: &'a mut S,
    pub(crate) reserved: &'a [&'a str],
}

impl<S> Visit for FieldVisitor<'_, S>
where
    S: serde::ser::SerializeMap,
{
    /// Visit a double-precision floating point value.
    fn record_f64(&mut self, field: &Field, value: f64) {
        let key = field.name();
        if !self.reserved.contains(&key) {
            let _ = self.serializer.serialize_entry(key, &value);
        }
    }

    /// Visit a signed 64-bit integer value.
    fn record_i64(&mut self, field: &Field, value: i64) {
        let key = field.name();
        if !self.reserved.contains(&key) {
            let _ = self.serializer.serialize_entry(key, &value);
        }
    }

    /// Visit an unsigned 64-bit integer value.
    fn record_u64(&mut self, field: &Field, value: u64) {
        let key = field.name();
        if !self.reserved.contains(&key) {
            let _ = self.serializer.serialize_entry(key, &value);
        }
    }

    /// Visit a signed 128-bit integer value.
    fn record_i128(&mut self, field: &Field, value: i128) {
        let key = field.name();
        if !self.reserved.contains(&key) {
            let value = format!("{value}");
            let _ = self.serializer.serialize_entry(key, &value);
        }
    }

    /// Visit an unsigned 128-bit integer value.
    fn record_u128(&mut self, field: &Field, value: u128) {
        let key = field.name();
        if !self.reserved.contains(&key) {
            let value = format!("{value}");
            let _ = self.serializer.serialize_entry(key, &value);
        }
    }

    /// Visit a boolean value.
    fn record_bool(&mut self, field: &Field, value: bool) {
        let key = field.name();
        if !self.reserved.contains(&key) {
            let _ = self.serializer.serialize_entry(key, &value);
        }
    }

    /// Visit a string value.
    fn record_str(&mut self, field: &Field, value: &str) {
        let key = field.name();
        if !self.reserved.contains(&key) {
            let _ = self.serializer.serialize_entry(key, &value);
        }
    }

    // /// Records a type implementing `Error`.
    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        let key = field.name();
        if !self.reserved.contains(&key) {
            let value = format!("{value}");
            let _ = self.serializer.serialize_entry(key, &value);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let key = field.name();
        if !self.reserved.contains(&key) {
            let value = format!("{value:?}");
            let _ = self.serializer.serialize_entry(key, &value);
        }
    }
}

#[cfg(test)]
mod tests;
