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

use storage_core::traits::{versioning::v1beta::*, BoxStream, Request, Result, StreamingOptions};
use tonic::async_trait;

use crate::backend::permissions::{action::Action, request::RequestExt, Permissions};

#[async_trait]
impl<S> Versioning for Permissions<S>
where
    S: Versioning,
{
    fn inner(&self) -> Option<&dyn Versioning> {
        None
    }

    async fn enumerate_versions(
        &self,
        request: Request<EnumerateVersionsRequest>,
        streaming: StreamingOptions,
        version_id_token: Option<String>,
    ) -> Result<BoxStream<EnumerateVersionsResponseWithContinueToken>> {
        self.validate_object_access(
            &request.get_permission_request_metadata(&self.inner.forward_headers)?,
            &request.get_ref().resource_address,
            Action::EnumerateVersions,
        )
        .await?;

        self.next.enumerate_versions(request, streaming, version_id_token).await
    }
}
