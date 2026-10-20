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

//! Notification system tests - contains comprehensive tests for the notification system, organized into logical categories for better maintainability

pub mod config_tests;
pub mod debug_tests;
pub mod durable_consumer_housekeeping_tests;
pub mod event_processing_tests;
// heartbeat_tests.rs is gone deliberately. Its six tests hand-wrote
// `tokio::select!` loops and asserted wall-clock tick counts — 8-12 ticks in a
// second, fewer than 5 in another — importing no crate function, so they measured
// tokio's `interval` rather than anything this service does. Four of the six had
// tight upper bounds, which is a flake on a loaded runner for no coverage.
//
// The property they were reaching for is real: the durable consumer's
// housekeeping must be a persistent interval, not a sleep rebuilt each loop
// iteration. It is covered twice over by tests that drive real code —
// `durable_consumer_housekeeping_tests` below, against a real `TokenGate`, and
// `token_expiry_is_checked_while_saturated` in `task-worker/tests/consumer.rs`,
// against the real consume loop at its concurrency bound.
pub mod provisioning_glue_tests;
pub mod provisioning_tests;
pub mod stream_establishment_timeout_tests;
pub mod types_tests;
