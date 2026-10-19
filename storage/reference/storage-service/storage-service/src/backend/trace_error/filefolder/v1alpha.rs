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

use futures::StreamExt;
use storage_core::traits::{async_trait, filefolder::v1alpha::*, BoxStream, Request, Result, StreamingOptions};

use crate::backend::trace_error::{log_storage_error, TraceErrorService};

#[async_trait]
impl<S: FileFolder> FileFolder for TraceErrorService<S> {
    fn inner(&self) -> Option<&dyn FileFolder> {
        None
    }

    async fn list(
        &self,
        request: Request<ListRequest>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<ListResponseWithContinueToken>> {
        self.inner
            .list(request, streaming)
            .await
            .inspect_err(|error| log_storage_error(error, "list"))
            .map(|stream| {
                Box::pin(stream.then(async |item_or_error| {
                    item_or_error.inspect_err(|error| log_storage_error(error, "list_fetch"))
                })) as BoxStream<ListResponseWithContinueToken>
            })
    }

    async fn list_stat(
        &self,
        request: Request<ListStatRequest>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<ListStatResponseWithContinueToken>> {
        self.inner
            .list_stat(request, streaming)
            .await
            .inspect_err(|error| log_storage_error(error, "list_stat"))
            .map(|stream| {
                Box::pin(stream.then(async |item_or_error| {
                    item_or_error.inspect_err(|error| log_storage_error(error, "list_stat_fetch"))
                })) as BoxStream<ListStatResponseWithContinueToken>
            })
    }

    async fn delete_folder(&self, request: Request<DeleteFolderRequest>) -> Result<DeleteFolderResponse> {
        self.inner
            .delete_folder(request)
            .await
            .inspect_err(|error| log_storage_error(error, "delete_folder"))
    }

    async fn create_folder(&self, request: Request<CreateFolderRequest>) -> Result<CreateFolderResponse> {
        self.inner
            .create_folder(request)
            .await
            .inspect_err(|error| log_storage_error(error, "create_folder"))
    }

    async fn get_folder_mode(&self, request: Request<GetFolderModeRequest>) -> Result<GetFolderModeResponse> {
        self.inner
            .get_folder_mode(request)
            .await
            .inspect_err(|error| log_storage_error(error, "get_folder_mode"))
    }
}
