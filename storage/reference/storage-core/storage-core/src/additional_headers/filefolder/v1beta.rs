// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;

use crate::traits::{
    error::Result,
    filefolder::v1beta::{
        DeleteFolderRequest, DeleteFolderResponse, FileFolder, ListRequest, ListResponseWithContinueToken,
        ListStatRequest, ListStatResponseWithContinueToken,
    },
    BoxStream, Request, StreamingOptions,
};

use super::super::AdditionalHeaderManager;

#[async_trait]
impl<S> FileFolder for AdditionalHeaderManager<S>
where
    S: FileFolder + Send + Sync,
{
    fn inner(&self) -> Option<&dyn FileFolder> {
        Some(&self.inner)
    }

    async fn list(
        &self,
        mut request: Request<ListRequest>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<ListResponseWithContinueToken>> {
        let additional_headers = self.parse(&request)?;
        request.extensions_mut().insert(additional_headers);

        self.inner.list(request, streaming).await
    }

    async fn list_stat(
        &self,
        mut request: Request<ListStatRequest>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<ListStatResponseWithContinueToken>> {
        let additional_headers = self.parse(&request)?;
        request.extensions_mut().insert(additional_headers);

        self.inner.list_stat(request, streaming).await
    }

    async fn delete_folder(&self, mut request: Request<DeleteFolderRequest>) -> Result<DeleteFolderResponse> {
        let additional_headers = self.parse(&request)?;
        request.extensions_mut().insert(additional_headers);

        self.inner.delete_folder(request).await
    }
}
