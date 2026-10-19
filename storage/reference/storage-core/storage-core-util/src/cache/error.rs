// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use snafu::Snafu;

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("redis: {source}"))]
    #[snafu(context(false))]
    Redis { source: fred::error::Error },

    #[snafu(display("postcard: {source}"))]
    #[snafu(context(false))]
    Postcard { source: postcard::Error },

    #[snafu(display("namespace required when L2 client is configured"))]
    EmptyNamespace,

    #[snafu(display("redis command timed out after {timeout_ms}ms"))]
    Timeout { timeout_ms: u64 },
}
