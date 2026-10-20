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

pub(crate) mod error_chain;
#[doc(hidden)]
pub mod extract;

/// Convert [`usize`] values to [`u64`].
///
/// Otel can only record `u64` values, however all our payload sizes are
/// `usize`. For 64-bit platforms (i.e. where `usize` == `u64`), this is a
/// no-op. For 32-bit platforms (i.e. where `usize` == `u32`), this is a
/// widening. For 128-bit platforms (that don't exist yet), this would be a
/// truncation, but likely acceptable given that `u64::MAX` is rather large for
/// a payload.
macro_rules! as_u64 {
    ($v:expr) => {{
        let v: usize = $v;
        v as u64
    }};
}
pub(crate) use as_u64;
