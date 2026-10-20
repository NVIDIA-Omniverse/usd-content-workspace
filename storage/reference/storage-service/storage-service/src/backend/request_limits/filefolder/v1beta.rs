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
use storage_core::traits::{filefolder::v1beta::*, BoxStream, Request, Result, StreamingOptions};

use crate::backend::request_limits::{types::StreamingOptionsExt, RequestLimitsService};

impl<S: FileFolder> FileFolder for RequestLimitsService<S> {
    fn inner(&self) -> Option<&dyn FileFolder> {
        None
    }

    fn list<'a: 'b, 'b>(
        &'a self,
        request: Request<ListRequest>,
        streaming: StreamingOptions,
    ) -> BoxFuture<'b, Result<BoxStream<ListResponseWithContinueToken>>> {
        self.inner.list(request, streaming.with_page_size_cap())
    }

    fn list_stat<'a: 'b, 'b>(
        &'a self,
        request: Request<ListStatRequest>,
        streaming: StreamingOptions,
    ) -> BoxFuture<'b, Result<BoxStream<ListStatResponseWithContinueToken>>> {
        self.inner.list_stat(request, streaming.with_page_size_cap())
    }

    fn delete_folder<'a: 'b, 'b>(
        &'a self,
        request: Request<DeleteFolderRequest>,
    ) -> BoxFuture<'b, Result<DeleteFolderResponse>> {
        self.inner.delete_folder(request)
    }
}
