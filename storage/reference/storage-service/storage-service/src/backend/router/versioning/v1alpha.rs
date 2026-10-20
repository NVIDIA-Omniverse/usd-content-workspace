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

use futures::future::BoxFuture;
use storage_core::traits::{versioning::v1alpha::*, BoxStream, Request, Result, StreamingOptions};
use tonic::async_trait;

use crate::backend::router::{error::Error, Dummy, Match, Service, ServiceRoute};

#[async_trait]
impl<A: Versioning, SA: Match, B: Versioning> Versioning for Service<ServiceRoute<A, SA>, B> {
    fn inner(&self) -> Option<&dyn Versioning> {
        None
    }

    fn enumerate_versions<'a: 'r, 'r>(
        &'a self,
        request: Request<EnumerateVersionsRequest>,
        streaming: StreamingOptions,
        version_id_token: Option<String>,
    ) -> BoxFuture<'r, Result<BoxStream<EnumerateVersionsResponseWithContinueToken>>> {
        if self
            .head
            .selector
            .match_resource_address(&request.get_ref().resource_address)
        {
            self.head
                .service
                .enumerate_versions(request, streaming, version_id_token)
        } else {
            self.tail.enumerate_versions(request, streaming, version_id_token)
        }
    }

    fn is_versioned<'a: 'r, 'r>(
        &'a self,
        request: Request<IsVersionedRequest>,
    ) -> BoxFuture<'r, Result<IsVersionedResponse>> {
        // Versioning is a per-top-level-address capability, so route like a folder address: any
        // route kind (object or folder) that covers the address answers for it.
        if self
            .head
            .selector
            .match_folder_address(&request.get_ref().resource_address)
        {
            self.head.service.is_versioned(request)
        } else {
            self.tail.is_versioned(request)
        }
    }
}

#[async_trait]
impl Versioning for Dummy {
    fn inner(&self) -> Option<&dyn Versioning> {
        None
    }

    async fn enumerate_versions(
        &self,
        _request: Request<EnumerateVersionsRequest>,
        _streaming: StreamingOptions,
        _version_id_token: Option<String>,
    ) -> Result<BoxStream<EnumerateVersionsResponseWithContinueToken>> {
        Err(Error::UnsupportedAddress.into())
    }

    async fn is_versioned(&self, _request: Request<IsVersionedRequest>) -> Result<IsVersionedResponse> {
        Err(Error::UnsupportedAddress.into())
    }
}
