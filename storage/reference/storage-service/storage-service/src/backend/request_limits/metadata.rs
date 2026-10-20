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

use futures::{future::BoxFuture, FutureExt};
use storage_core::traits::{BoxStream, Request, Result, StreamingOptions};

use crate::backend::{
    metadata::*,
    request_limits::{error::error, types::StreamingOptionsExt, RequestLimitsService, MAX_BATCH_SIZE},
};

impl<S: Metadata> Metadata for RequestLimitsService<S> {
    fn inner(&self) -> Option<&dyn Metadata> {
        None
    }

    fn get_metadata_batch<'a: 'b, 'b>(
        &'a self,
        request: Request<GetMetadataBatchRequest>,
    ) -> BoxFuture<'b, Result<GetMetadataBatchResponse>> {
        if request.get_ref().requests.len() > MAX_BATCH_SIZE {
            batch_size_exceeded_future()
        } else {
            self.inner.get_metadata_batch(request)
        }
    }

    fn query_metadata<'a: 'b, 'b>(
        &'a self,
        request: Request<QueryMetadataRequest>,
        options: StreamingOptions,
    ) -> BoxFuture<'b, Result<BoxStream<QueryMetadataResponsePage>>> {
        self.inner.query_metadata(request, options.with_page_size_cap())
    }

    fn update_metadata_batch<'a: 'b, 'b>(
        &'a self,
        request: Request<UpdateMetadataBatchRequest>,
    ) -> BoxFuture<'b, Result<UpdateMetadataBatchResponse>> {
        if request.get_ref().requests.len() > MAX_BATCH_SIZE {
            batch_size_exceeded_future()
        } else {
            self.inner.update_metadata_batch(request)
        }
    }

    fn delete_metadata_batch<'a: 'b, 'b>(
        &'a self,
        request: Request<DeleteMetadataBatchRequest>,
    ) -> BoxFuture<'b, Result<DeleteMetadataBatchResponse>> {
        if request.get_ref().requests.len() > MAX_BATCH_SIZE {
            batch_size_exceeded_future()
        } else {
            self.inner.delete_metadata_batch(request)
        }
    }
}

#[track_caller]
fn batch_size_exceeded_future<'a, T: Send + Sync + 'static>() -> BoxFuture<'a, Result<T>> {
    futures::future::ready(Err(error::BatchSizeExceeded.build().into())).boxed()
}
