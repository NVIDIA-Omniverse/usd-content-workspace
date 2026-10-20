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

use std::error::Error as StdError;

use snafu::{IntoError, Location, Snafu};

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("{message}"))]
    Message {
        message: String,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("{message}"))]
    WithSource {
        message: String,
        source: Box<dyn StdError>,
        #[snafu(provide, implicit)]
        location: Location,
    },
}

impl Error {
    #[track_caller]
    pub fn new(message: impl Into<String>) -> Self {
        MessageSnafu {
            message: message.into(),
        }
        .build()
    }

    #[track_caller]
    pub fn new_with_source(message: impl Into<String>, source: impl Into<Box<dyn StdError>>) -> Self {
        WithSourceSnafu {
            message: message.into(),
        }
        .into_error(source.into())
    }
}
