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

//! Shared library used by both service frontends.
//!
//! The `http` and `grpc` crates should keep transport-specific concerns local and depend on this
//! crate for the actual thumbnail, storage, auth, and notification workflows.

pub mod auth;
pub mod config;
pub mod converters;
pub mod errors;
pub mod generation;
pub mod logging;
pub mod notifications;
pub mod outbound;
pub mod resource;
pub mod runtime;
pub mod source_formats;
pub mod spec_enums;
pub mod storage;
pub mod thumbnail_path;
pub mod thumbnail_sizes;
pub mod thumbnails;
pub mod types;
pub mod utils;
pub mod work_events;

// Behind the `test-utils` feature, not `#[cfg(test)]`. A `cfg(test)` gate here
// would compile this module only when `thumbnail-common` itself is under test —
// never when a downstream crate's own tests run, which is most of its callers.
// The `test` arm covers this crate's own unit tests without them having to enable
// the feature; downstream crates enable it from `[dev-dependencies]`, so release
// builds never compile it.
#[cfg(any(test, feature = "test-utils"))]
pub mod test_fixtures;

#[cfg(test)]
mod tests;

// Re-export the common modules so service crates can depend on a single shared API surface.
pub use auth::*;
pub use config::*;
pub use converters::*;
pub use errors::*;
pub use generation::{
    create_thumbnails, CreateThumbnailsError, CreateThumbnailsOutcome, FailedSubmission,
};
pub use logging::{env_log_filter, init_tracing};
pub use notifications::*;
pub use outbound::{
    build_outbound_clients, OutboundAuthArgs, OutboundClients, OutboundClientsConfig,
};
pub use resource::*;
pub use runtime::{
    initialize_thumbnail_runtime, shutdown_receiver_with_signals, spawn_event_consumer_supervisor,
    start_event_consumer_service, EventConsumerRetryPolicy, EventConsumerSupervisorConfig,
};
pub use source_formats::{extract_extension, ALLOWED_EXTENSIONS};
pub use spec_enums::*;
pub use storage::*;
pub use thumbnail_path::*;
pub use thumbnail_sizes::{ThumbnailSizes, ThumbnailSizesError};
pub use thumbnails::*;
pub use types::*;
pub use utils::*;
pub use work_events::{
    build_generate_message, dispatch_generate_event, dispatch_generate_for_configured_sizes,
    dispatch_generate_per_specification, publish_generate_event, resolve_generate_spec,
    GenerateContractError, GenerateDispatchError, GenerateFanOutFailure, GenerateMessageParams,
    GenerateMessageRejection, GeneratePublishError, GenerateSource, GenerateWorkMessage,
    ResolvedGenerateSpec, WorkResource, GENERATE_EVENT_TYPE, GENERATE_SCHEMA_VERSION,
};
