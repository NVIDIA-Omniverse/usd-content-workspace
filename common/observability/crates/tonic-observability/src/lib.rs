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

//! Trace and metric layers for Tonic-based gRPC clients and servers.
#![deny(missing_debug_implementations, missing_docs)]
#![warn(unnameable_types)]
pub mod client;
mod common;
pub mod rpc_map;
pub mod server;

pub mod __macro_support {
    //! Not a public API, macro only. No stability guarantees.
    #![doc(hidden)]
    pub use crate::client::trace::span::__macro_support as client_trace;
    pub use crate::server::trace::span::__macro_support as server_trace;
    pub use opentelemetry::trace as otel;
    pub use opentelemetry_semantic_conventions::attribute as sem;
    pub use tonic::codegen::http;
    pub use tracing;
}

/// Verbose tracing of the server layers.
///
/// It's intended that this is disabled by default.
macro_rules! server_verbose {
    ($fmt:literal) => {
        #[cfg(feature = "verbose")]
        eprintln!(concat!("server ", $fmt));
    };
    ($fmt:literal, $($arg:tt)+) => {
        #[cfg(feature = "verbose")]
        eprintln!(concat!("server ", $fmt), $($arg)+);
    };
}
pub(crate) use server_verbose;

/// Verbose tracing of the client layers.
///
/// It's intended that this is disabled by default.
macro_rules! client_verbose {
    ($fmt:literal) => {
        #[cfg(feature = "verbose")]
        eprintln!(concat!("client ", $fmt));
    };
    ($fmt:literal, $($arg:tt)+) => {
        #[cfg(feature = "verbose")]
        eprintln!(concat!("client ", $fmt), $($arg)+);
    };
}
pub(crate) use client_verbose;
