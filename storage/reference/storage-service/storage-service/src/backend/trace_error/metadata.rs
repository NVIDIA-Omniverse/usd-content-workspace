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

use crate::backend::{
    metadata::*,
    trace_error::{log_storage_error, TraceErrorService},
};
use futures::{StreamExt, TryStreamExt};
use storage_core::traits::{async_trait, BoxStream, Request, Result, StreamingOptions};

#[async_trait]
impl<S: Metadata> Metadata for TraceErrorService<S> {
    fn inner(&self) -> Option<&dyn Metadata> {
        None
    }

    async fn get_metadata_batch(&self, request: Request<GetMetadataBatchRequest>) -> Result<GetMetadataBatchResponse> {
        self.inner
            .get_metadata_batch(request)
            .await
            .inspect_err(|error| log_storage_error(error, "get_metadata_batch"))
    }

    async fn query_metadata(
        &self,
        request: Request<QueryMetadataRequest>,
        options: StreamingOptions,
    ) -> Result<BoxStream<QueryMetadataResponsePage>> {
        self.inner
            .query_metadata(request, options)
            .await
            .inspect_err(|error| log_storage_error(error, "query_metadata"))
            .map(|stream| {
                stream
                    .map_err(|error| {
                        log_storage_error(&error, "query_metadata_page");
                        error
                    })
                    .boxed()
            })
    }

    async fn update_metadata_batch(
        &self,
        request: Request<UpdateMetadataBatchRequest>,
    ) -> Result<UpdateMetadataBatchResponse> {
        self.inner
            .update_metadata_batch(request)
            .await
            .inspect_err(|error| log_storage_error(error, "update_metadata_batch"))
    }

    async fn delete_metadata_batch(
        &self,
        request: Request<DeleteMetadataBatchRequest>,
    ) -> Result<DeleteMetadataBatchResponse> {
        self.inner
            .delete_metadata_batch(request)
            .await
            .inspect_err(|error| log_storage_error(error, "delete_metadata_batch"))
    }
}
