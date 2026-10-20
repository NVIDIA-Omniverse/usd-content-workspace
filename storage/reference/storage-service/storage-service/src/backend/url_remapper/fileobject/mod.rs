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

use crate::{
    backend::metadata::Metadata,
    common::url_remapper::{error::UrlRewriteError, RewriteMode, RewriteResult, UrlRemapper},
    error::InvalidArgumentError,
};
use storage_core::traits::Result;

pub mod v1alpha;
pub mod v1beta;

impl<T: Metadata> UrlRemapper<T> {
    /// Rewrite a resource address and a resource identity associated with it.
    pub(super) async fn request_rewrite_associated_address_identity(
        &self,
        address: &str,
        previous_version: Option<&str>,
    ) -> Result<RewriteResult<(String, Option<String>)>> {
        let RewriteResult::Mapped(resource_address, rule_tag) =
            self.fwd_rewrite_unversioned_url(address, RewriteMode::Precise)
        else {
            return Ok(RewriteResult::Unchanged);
        };

        let Some(previous_version) = previous_version else {
            return Ok(RewriteResult::Mapped((resource_address, None), rule_tag));
        };

        let RewriteResult::Mapped(resource_identity, _) = self
            .fwd_rewrite_versioned_url(previous_version, RewriteMode::Precise)
            .await
            .map_err(UrlRewriteError::into_invalid_argument_error)?
        else {
            return Err(InvalidArgumentError::new("Resource identity belongs to a different object").into());
        };

        Ok(RewriteResult::Mapped(
            (resource_address, Some(resource_identity)),
            rule_tag,
        ))
    }
}
