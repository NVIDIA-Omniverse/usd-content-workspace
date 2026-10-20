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

use snafu::{IntoError, Location, Snafu};
use tonic::{metadata::errors::InvalidMetadataValue, Status};

use crate::{common::credentials, error::BoxError};

/// Client initialization errors.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub(super)))]
pub enum InitError {
    #[snafu(display("notification service endpoint is missing"))]
    EndpointMissing {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to create a credentials provider"))]
    CredentialsProviderInit {
        #[snafu(source)]
        source: credentials::oauth2::error::InitError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("notification service endpoint uri is invalid"))]
    InvalidEndpointUri {
        #[snafu(source)]
        source: BoxError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to create a channel"))]
    ChannelCreate {
        source: storage_service_grpc::balancer::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },
}

impl InitError {
    #[track_caller]
    pub(super) fn endpoint_missing() -> Self {
        EndpointMissingSnafu.build()
    }

    #[track_caller]
    pub(super) fn invalid_endpoint_uri(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        InvalidEndpointUriSnafu.into_error(Box::new(source))
    }

    #[track_caller]
    pub(super) fn channel_create(source: storage_service_grpc::balancer::Error) -> Self {
        ChannelCreateSnafu.into_error(source)
    }
}

/// Client operation errors.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub(super)))]
pub enum PublishError {
    #[snafu(display("batch publish request failed"))]
    UnexpectedStatus {
        #[snafu(source)]
        source: Status,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("batch publish entry rejected: {message}"))]
    Rejected {
        message: String,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to fetch client credentials"))]
    ClientCredentialsFetchFailed {
        #[snafu(source)]
        source: credentials::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to update request metadata"))]
    MetadataSetFailed {
        #[snafu(source)]
        source: InvalidMetadataValue,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("batch publish response contained {actual} results for {expected} events"))]
    InvalidResponseCardinality {
        expected: usize,
        actual: usize,
        #[snafu(provide, implicit)]
        location: Location,
    },
}

impl PublishError {
    #[track_caller]
    pub(super) fn unexpected_status(source: Status) -> Self {
        UnexpectedStatusSnafu.into_error(source)
    }

    #[track_caller]
    pub(super) fn rejected(message: impl Into<String>) -> Self {
        RejectedSnafu { message }.build()
    }
}
