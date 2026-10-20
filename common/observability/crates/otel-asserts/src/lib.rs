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

//! OpenTelemetry assertion tools, mainly test collectors.
//!
//! This crate is experimental, and may change.
#![deny(missing_docs, missing_debug_implementations)]
pub mod grpc;
pub mod http;
mod select;
#[allow(missing_docs)]
pub mod semconv;
#[cfg(feature = "with-serde")]
pub(crate) mod serde;
pub mod tracing;
pub mod types;

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

/// A TCP socket server, listening for connections.
///
/// This is basically a wrapper around [`std::net::TcpListener`], and ensures
/// that the listener can be converted into a [`tokio::net::TcpListener`].
///
/// The benefit of this is that the listener can be constructed in synchronous
/// code, and so env vars can be set from the address safely.
#[derive(Debug)]
pub struct TcpListener {
    listener: std::net::TcpListener,
    addr: SocketAddr,
}

impl TcpListener {
    /// Creates a new `TcpListener` which will be bound to a random port on
    /// localhost.
    pub fn bind_random() -> Self {
        let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let addr = SocketAddr::new(ip, 0);
        let listener = std::net::TcpListener::bind(addr).expect("failed to bind to random port");
        listener
            .set_nonblocking(true)
            .expect("failed to set non-blocking");
        let addr = listener.local_addr().expect("failed to get local address");
        Self { listener, addr }
    }

    /// Returns the local socket address of this listener.
    #[inline]
    pub fn local_addr(&self) -> &SocketAddr {
        &self.addr
    }

    /// tokio reactor must be running when this is called.
    pub fn into_tokio(self) -> tokio::net::TcpListener {
        tokio::net::TcpListener::from_std(self.listener)
            .expect("failed to convert to tokio listener")
    }
}

impl fmt::Display for TcpListener {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.addr.fmt(f)
    }
}

/// Set the environment to the specified keys only.
///
/// Other keys are removed.
///
/// # Safety
///
/// On Unix for some libc implementations, `set_env`/`remove_env` is only safe
/// in single-threaded programs. `cargo nextest` provides per-test isolation, so
/// these tests are safe only when run via `cargo nextest` and not `cargo test`:
/// <https://nexte.st/docs/configuration/env-vars/#altering-the-environment-within-tests>
pub unsafe fn set_env(vars: &[(&str, &str)]) {
    for (key, _value) in std::env::vars() {
        // don't remove `RUST_BACKTRACE`, as this prevents a backtrace from
        // being printed
        // TODO: only filter `OTEL_*` keys?
        if key == "RUST_BACKTRACE" {
            continue;
        }
        unsafe {
            std::env::remove_var(&key);
        }
    }
    for (key, value) in vars.iter().copied() {
        unsafe {
            std::env::set_var(key, value);
        }
    }
}

/// Set the environment to the specified keys only.
///
/// Other keys are removed.
///
/// # Safety
///
/// On Unix for some libc implementations, `set_env`/`remove_env` is only safe
/// in single-threaded programs. `cargo nextest` provides per-test isolation, so
/// these tests are safe only when run via `cargo nextest` and not `cargo test`:
/// <https://nexte.st/docs/configuration/env-vars/#altering-the-environment-within-tests>
#[macro_export]
macro_rules! unsafe_env {
    ($($k:literal : $v:expr),* $(,)?) => {
        unsafe {
            $crate::set_env(&[
                $(($k, &$v as &str)),*
            ])
        }
    };
}
