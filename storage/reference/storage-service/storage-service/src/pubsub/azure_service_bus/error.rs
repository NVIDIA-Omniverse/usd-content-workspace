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

use crate::error::BoxError;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum SubscriberError {
    #[snafu(display("failed to create a connection"))]
    ConnectFailed {
        #[snafu(source)]
        source: BoxError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to create a receiver"))]
    ReceiverCreateFailed {
        #[snafu(source)]
        source: BoxError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to receive messages"))]
    ReceiveMessagesFailed {
        #[snafu(source)]
        source: BoxError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to complete a message"))]
    CompleteMessageFailed {
        #[snafu(source)]
        source: BoxError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to resolve resource identity from a message"))]
    MessageIdentityResolve {
        source: super::types::BoxError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to parse an event"))]
    EventParseFailed {
        #[snafu(source)]
        source: serde_json::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("blob address is invalid: {message}"))]
    InvalidBlobAddress {
        message: String,
        #[snafu(provide, implicit)]
        location: Location,
    },
}

impl SubscriberError {
    #[track_caller]
    pub fn connect_failed(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        ConnectFailedSnafu.into_error(Box::new(source))
    }

    #[track_caller]
    pub fn receiver_create_failed(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        ReceiverCreateFailedSnafu.into_error(Box::new(source))
    }

    #[track_caller]
    pub fn receive_messages_failed(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        ReceiveMessagesFailedSnafu.into_error(Box::new(source))
    }

    #[track_caller]
    pub fn complete_message_failed(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        CompleteMessageFailedSnafu.into_error(Box::new(source))
    }

    #[track_caller]
    pub fn invalid_blob_address(message: impl Into<String>) -> Self {
        InvalidBlobAddressSnafu { message }.build()
    }
}
