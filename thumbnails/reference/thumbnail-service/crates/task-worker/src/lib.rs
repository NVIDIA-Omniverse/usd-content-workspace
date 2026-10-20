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

//! `task-worker` — the Rust thumbnail generation worker.
//!
//! Consumes `omni.thumbnail.generate` from its own durable Notification queue
//! (WorkerEventQueue), converts, uploads, and delegates association back to
//! thumbnail-service over gRPC. It does not consume storage events, does not
//! serve an API, and does not write association metadata: planning stays on
//! `server` and association stays a service RPC.
//!
//! Event distribution is reached only through the published Notification API and
//! its configured endpoint, so a deployment can be pointed at any API-conformant
//! service with configuration alone. There is deliberately no abstraction layer
//! and no second intake path — see the conformance notes in the chart README for
//! the behaviors the worker depends on that the API surface does not express.

pub mod associate;
pub mod config;
pub mod consumer;
pub mod health;
pub mod metrics;
pub mod outcome;
pub mod pipeline;
