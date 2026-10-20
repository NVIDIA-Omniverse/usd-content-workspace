// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use snafu::prelude::*;

#[derive(Debug, Clone, Snafu)]
#[snafu(visibility(pub), context(suffix(false)))]
pub enum Error {
    #[snafu(transparent)]
    AddressParse {
        source: std::net::AddrParseError,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    #[snafu(transparent)]
    Figment {
        #[snafu(source(from(figment::Error, Arc::new)))]
        source: Arc<figment::Error>,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    #[snafu(display("internal error: {message}"))]
    Internal {
        message: String,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    #[snafu(transparent)]
    Io {
        #[snafu(source(from(std::io::Error, Arc::new)))]
        source: Arc<std::io::Error>,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    #[snafu(display("secrets error: {message}"))]
    Secrets {
        message: String,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    #[snafu(transparent)]
    SystemTime {
        source: std::time::SystemTimeError,
        #[snafu(implicit)]
        loc: snafu::Location,
    },
}

impl Error {
    #[track_caller]
    pub fn internal(message: impl Into<String>) -> Self {
        Internal { message }.build()
    }

    #[track_caller]
    pub fn secrets(message: impl Into<String>) -> Self {
        Secrets { message }.build()
    }

    pub fn source_location(&self) -> snafu::Location {
        match self {
            Error::AddressParse { loc, .. } => loc,
            Error::Figment { loc, .. } => loc,
            Error::Internal { loc, .. } => loc,
            Error::Io { loc, .. } => loc,
            Error::Secrets { loc, .. } => loc,
            Error::SystemTime { loc, .. } => loc,
        }
    }
}
