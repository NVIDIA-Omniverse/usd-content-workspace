// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;

use storage_core_builder::traits::{
    error::Result,
    filefolder::v1beta::{
        DeleteFolderRequest, DeleteFolderResponse, FileFolder, ListRequest, ListResponseWithContinueToken,
        ListStatRequest, ListStatResponseWithContinueToken,
    },
    BoxStream, Request, StreamingOptions,
};

use crate::egress_throttle::EgressThrottle;

#[async_trait]
impl<S> FileFolder for EgressThrottle<S>
where
    S: FileFolder,
{
    fn inner(&self) -> Option<&dyn FileFolder> {
        Some(&self.inner)
    }

    async fn list(
        &self,
        request: Request<ListRequest>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<ListResponseWithContinueToken>> {
        let _permit = self.semaphore.acquire().await;
        self.inner.list(request, streaming).await
    }

    async fn list_stat(
        &self,
        request: Request<ListStatRequest>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<ListStatResponseWithContinueToken>> {
        let _permit = self.semaphore.acquire().await;
        self.inner.list_stat(request, streaming).await
    }

    async fn delete_folder(&self, request: Request<DeleteFolderRequest>) -> Result<DeleteFolderResponse> {
        let _permit = self.semaphore.acquire().await;
        self.inner.delete_folder(request).await
    }
}
