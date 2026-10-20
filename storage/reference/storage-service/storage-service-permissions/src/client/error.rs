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

use std::fmt::Debug;

use snafu::{IntoError, Location, Snafu};

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("failed to encode a request header"))]
    RequestHeaderEncodeFailed {
        #[snafu(source)]
        source: http::header::InvalidHeaderValue,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to encode request metadata"))]
    RequestMetadataEncodeFailed {
        #[snafu(source)]
        source: tonic::metadata::errors::InvalidMetadataValue,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to encode a request"))]
    RequestEncodeFailed {
        #[snafu(source)]
        source: serde_json::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("request failed"))]
    RequestFailed {
        #[snafu(source)]
        source: Box<dyn std::error::Error + Send + Sync>,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("unexpected response status {status_code}"))]
    UnexpectedResponseStatus {
        status_code: http::StatusCode,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to decode the response"))]
    ResponseDecodeFailed {
        #[snafu(source)]
        source: Box<dyn std::error::Error + Send + Sync>,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("the decision is missing from the response"))]
    ResponseDecisionMissing {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("request principal is invalid"))]
    InvalidPrincipal {
        #[snafu(source)]
        source: MaybeTonicStatus,
        #[snafu(provide, implicit)]
        location: Location,
    },
}

impl Error {
    #[track_caller]
    pub fn request_header_encode_failed(source: http::header::InvalidHeaderValue) -> Self {
        RequestHeaderEncodeFailedSnafu.into_error(source)
    }

    #[track_caller]
    pub fn request_metadata_encode_failed(source: tonic::metadata::errors::InvalidMetadataValue) -> Self {
        RequestMetadataEncodeFailedSnafu.into_error(source)
    }

    #[track_caller]
    pub fn request_failed(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        RequestFailedSnafu.into_error(Box::new(source))
    }

    #[track_caller]
    pub fn unexpected_response_status(status_code: http::StatusCode) -> Self {
        UnexpectedResponseStatusSnafu { status_code }.build()
    }

    #[track_caller]
    pub fn request_encode_failed(source: serde_json::Error) -> Self {
        RequestEncodeFailedSnafu.into_error(source)
    }

    #[track_caller]
    pub fn response_decode_failed(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        ResponseDecodeFailedSnafu.into_error(Box::new(source))
    }

    #[track_caller]
    pub fn response_decision_missing() -> Self {
        ResponseDecisionMissingSnafu.build()
    }

    #[track_caller]
    pub fn invalid_principal() -> Self {
        InvalidPrincipalSnafu.into_error(MaybeTonicStatus::None)
    }

    #[track_caller]
    pub fn invalid_principal_source(source: tonic::Status) -> Self {
        InvalidPrincipalSnafu.into_error(MaybeTonicStatus::Some { source })
    }
}

#[derive(Debug, Snafu)]
pub enum InitError {
    #[snafu(display("endpoint configuration is missing"))]
    EndpointConfigMissing {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("only one endpoint can be configured simultaneously"))]
    MultipleEndpointsConfigured {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("grpc endpoint url is invalid"))]
    InvalidEndpoint {
        #[snafu(source)]
        source: tonic::transport::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to connect to the grpc endpoint"))]
    ConnectFailed {
        #[snafu(source)]
        source: tonic::transport::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("grpc endpoint uri is invalid"))]
    InvalidEndpointUri {
        #[snafu(source)]
        source: http::uri::InvalidUri,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("grpc endpoint uri is missing a host"))]
    EndpointHostMissing {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("grpc endpoint uri is missing a port"))]
    EndpointPortMissing {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to resolve the grpc endpoint dns"))]
    ResolutionFailed {
        #[snafu(source)]
        source: std::io::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("grpc endpoint dns resolved no addresses"))]
    NoEndpointsResolved {
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
    pub fn endpoint_config_missing() -> Self {
        EndpointConfigMissingSnafu.build()
    }

    #[track_caller]
    pub fn multiple_endpoints_configured() -> Self {
        MultipleEndpointsConfiguredSnafu.build()
    }

    #[track_caller]
    pub fn invalid_endpoint(source: tonic::transport::Error) -> Self {
        InvalidEndpointSnafu.into_error(source)
    }

    #[track_caller]
    pub fn connect_failed(source: tonic::transport::Error) -> Self {
        ConnectFailedSnafu.into_error(source)
    }

    #[track_caller]
    pub fn invalid_endpoint_uri(source: http::uri::InvalidUri) -> Self {
        InvalidEndpointUriSnafu.into_error(source)
    }

    #[track_caller]
    pub fn endpoint_host_missing() -> Self {
        EndpointHostMissingSnafu.build()
    }

    #[track_caller]
    pub fn endpoint_port_missing() -> Self {
        EndpointPortMissingSnafu.build()
    }

    #[track_caller]
    pub fn resolution_failed(source: std::io::Error) -> Self {
        ResolutionFailedSnafu.into_error(source)
    }

    #[track_caller]
    pub fn no_endpoints_resolved() -> Self {
        NoEndpointsResolvedSnafu.build()
    }

    #[track_caller]
    pub fn channel_create(source: storage_service_grpc::balancer::Error) -> Self {
        ChannelCreateSnafu.into_error(source)
    }
}

#[derive(Debug, Snafu)]
pub enum MaybeTonicStatus {
    #[snafu(display("no additional error context provided"))]
    None,

    #[snafu(transparent)]
    Some {
        #[snafu(source)]
        source: tonic::Status,
    },
}
