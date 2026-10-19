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

use snafu::IntoError;
use storage_core::traits::Request;
use tonic::metadata::{Ascii, MetadataMap, MetadataValue};

use crate::{
    backend::permissions::error::{
        Error, RequestCredentialsBearerExpectedSnafu, RequestCredentialsDecodeSnafu, RequestCredentialsMissingSnafu,
    },
    common::forward_headers::ForwardedHeaderNames,
};

const METADATA_KEY_AUTHORIZATION: &str = "authorization";

/// Provides extension methods for the request type.
pub trait RequestExt {
    /// Extract the principal and the headers to forward to the permission service.
    fn get_permission_request_metadata(
        &self,
        forward: &ForwardedHeaderNames,
    ) -> Result<PermissionRequestMetadata, Error>;
}

impl<T> RequestExt for Request<T> {
    fn get_permission_request_metadata(
        &self,
        forward: &ForwardedHeaderNames,
    ) -> Result<PermissionRequestMetadata, Error> {
        self.metadata().get_permission_request_metadata(forward)
    }
}

impl RequestExt for MetadataMap {
    fn get_permission_request_metadata(
        &self,
        forward: &ForwardedHeaderNames,
    ) -> Result<PermissionRequestMetadata, Error> {
        let metadata_value = self
            .get(METADATA_KEY_AUTHORIZATION)
            .ok_or_else(|| RequestCredentialsMissingSnafu.build())?;

        get_bearer_permission_request_metadata(
            metadata_value,
            MetadataMap::from_headers(forward.collect(self.as_ref())),
        )
    }
}

/// What a request carries into a permission query: the authenticated principal and the incoming
/// headers configured for forwarding.
#[derive(Debug, Clone)]
pub struct PermissionRequestMetadata(pub(super) MetadataMap);

fn get_bearer_permission_request_metadata(
    authorization_metadata_value: &MetadataValue<Ascii>,
    mut forwarded_metadata: MetadataMap,
) -> Result<PermissionRequestMetadata, Error> {
    const TOKEN_TYPE_BEARER_PREFIX: &str = "bearer ";

    let metadata_value_str = authorization_metadata_value
        .to_str()
        .map_err(|source| RequestCredentialsDecodeSnafu.into_error(source))?;

    let Some((prefix, _)) = metadata_value_str.split_at_checked(TOKEN_TYPE_BEARER_PREFIX.len()) else {
        return RequestCredentialsBearerExpectedSnafu.fail();
    };

    if !prefix.eq_ignore_ascii_case(TOKEN_TYPE_BEARER_PREFIX) {
        return RequestCredentialsBearerExpectedSnafu.fail();
    }

    forwarded_metadata.insert(METADATA_KEY_AUTHORIZATION, authorization_metadata_value.clone());

    Ok(PermissionRequestMetadata(forwarded_metadata))
}
