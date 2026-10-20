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

//! HTTP API types module
//!
//! This module contains all request/response types, enums, and specifications
//! for the HTTP thumbnail service API.

mod enums;
mod requests;
mod responses;
mod specifications;

// Re-export all types for backwards compatibility
pub use enums::*;
pub use requests::*;
pub use responses::*;
pub use specifications::*;
