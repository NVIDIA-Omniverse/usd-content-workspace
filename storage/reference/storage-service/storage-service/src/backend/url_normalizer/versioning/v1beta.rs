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

use crate::backend::url_normalizer::UrlNormalizerService;
use futures::future::BoxFuture;
use storage_core::traits::{
    versioning::v1beta::{EnumerateVersionsRequest, EnumerateVersionsResponseWithContinueToken, Versioning},
    BoxStream, Request, Result, StreamingOptions,
};

impl<S: Versioning> Versioning for UrlNormalizerService<S> {
    fn inner(&self) -> Option<&dyn Versioning> {
        None
    }

    fn enumerate_versions<'a: 'b, 'b>(
        &'a self,
        request: Request<EnumerateVersionsRequest>,
        streaming: StreamingOptions,
        version_id_token: Option<String>,
    ) -> BoxFuture<'b, Result<BoxStream<EnumerateVersionsResponseWithContinueToken>>> {
        self.invoke_with_resource_address(
            request,
            |message| &mut message.resource_address,
            |request| self.inner.enumerate_versions(request, streaming, version_id_token),
        )
    }
}
