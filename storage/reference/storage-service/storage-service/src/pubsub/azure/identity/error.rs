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

use snafu::{Location, Snafu};

#[derive(Debug, Snafu)]
#[snafu(visibility(pub), context(suffix(false)))]
pub enum Error {
    #[snafu(display("resource uri is invalid"))]
    UrlParse {
        source: url::ParseError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("resource uri does not belong to a blob"))]
    AddressParse {
        source: storage_core::azure::error::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to create a client"))]
    ClientCreate {
        source: storage_core::azure::error::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("could not find object version"))]
    VersionNotFound {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("get_properties request failed"))]
    GetProperties {
        source: azure_core::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("list_blobs request failed"))]
    ListBlobs {
        source: azure_core::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },
}
