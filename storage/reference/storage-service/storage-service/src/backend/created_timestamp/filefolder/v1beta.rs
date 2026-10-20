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
use storage_core::traits::{async_trait, filefolder::v1beta::*, BoxStream, Request, Result, StreamingOptions};

use crate::backend::created_timestamp::{delete_created_timestamp, folder_address_location, CreatedTimestampService};

#[async_trait]
impl<S: FileFolder> FileFolder for CreatedTimestampService<S> {
    fn inner(&self) -> Option<&dyn FileFolder> {
        None
    }

    fn list<'a: 'b, 'b>(
        &'a self,
        request: Request<ListRequest>,
        streaming: StreamingOptions,
    ) -> BoxFuture<'b, Result<BoxStream<ListResponseWithContinueToken>>> {
        self.inner.list(request, streaming)
    }

    fn list_stat<'a: 'b, 'b>(
        &'a self,
        request: Request<ListStatRequest>,
        streaming: StreamingOptions,
    ) -> BoxFuture<'b, Result<BoxStream<ListStatResponseWithContinueToken>>> {
        self.inner.list_stat(request, streaming)
    }

    // Delete the folder, then best-effort remove its `created_timestamp`. Cleanup runs only after
    // the underlying delete succeeds, so a failed delete leaves the metadata untouched; a metadata
    // failure never fails the delete. Only the directly-deleted folder address is cleaned up; the
    // created_timestamps of recursively removed children are left for the metadata backend to
    // reap, since the layer does not have the child addresses without an extra listing.
    async fn delete_folder(&self, request: Request<DeleteFolderRequest>) -> Result<DeleteFolderResponse> {
        let folder_uri = request.get_ref().folder.as_ref().map(|folder| folder.uri.clone());
        let response = self.inner.delete_folder(request).await?;
        if let Some(folder_uri) = folder_uri {
            delete_created_timestamp(&self.metadata, folder_address_location(folder_uri)).await;
        }
        Ok(response)
    }
}
