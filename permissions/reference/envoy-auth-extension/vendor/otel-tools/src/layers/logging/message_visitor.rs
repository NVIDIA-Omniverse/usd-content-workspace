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

const MESSAGE: &str = "message";

/// A tracing visitor that only records the "message" field.
#[derive(Debug, Default)]
pub(crate) struct MessageVisitor {
    pub(crate) message: Option<String>,
}

impl Visit for MessageVisitor {
    /// Visit a double-precision floating point value.
    fn record_f64(&mut self, field: &Field, value: f64) {
        if field.name() == MESSAGE {
            self.message = Some(format!("{value}"));
        }
    }

    /// Visit a signed 64-bit integer value.
    fn record_i64(&mut self, field: &Field, value: i64) {
        if field.name() == MESSAGE {
            self.message = Some(format!("{value}"));
        }
    }

    /// Visit an unsigned 64-bit integer value.
    fn record_u64(&mut self, field: &Field, value: u64) {
        if field.name() == MESSAGE {
            self.message = Some(format!("{value}"));
        }
    }

    /// Visit a signed 128-bit integer value.
    fn record_i128(&mut self, field: &Field, value: i128) {
        if field.name() == MESSAGE {
            self.message = Some(format!("{value}"));
        }
    }

    /// Visit an unsigned 128-bit integer value.
    fn record_u128(&mut self, field: &Field, value: u128) {
        if field.name() == MESSAGE {
            self.message = Some(format!("{value}"));
        }
    }

    /// Visit a boolean value.
    fn record_bool(&mut self, field: &Field, value: bool) {
        if field.name() == MESSAGE {
            self.message = Some(format!("{value}"));
        }
    }

    /// Visit a string value.
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == MESSAGE {
            self.message = Some(value.to_string());
        }
    }

    // /// Records a type implementing `Error`.
    fn record_error(&mut self, field: &Field, value: &(dyn std::error::Error + 'static)) {
        if field.name() == MESSAGE {
            self.message = Some(format!("{value}"));
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == MESSAGE {
            self.message = Some(format!("{value:?}"));
        }
    }
}

#[cfg(test)]
mod tests;
