// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;

use crate::metrics::utils::TimedStream;
use crate::traits::{
    filefolder::v1alpha::{
        CreateFolderRequest, CreateFolderResponse, DeleteFolderRequest, DeleteFolderResponse,
        FileFolder as FileFolderTrait, GetFolderModeRequest, GetFolderModeResponse, ListRequest,
        ListResponseWithContinueToken, ListStatRequest, ListStatResponseWithContinueToken,
    },
    BoxStream, Request, Result, StreamingOptions,
};

use super::super::{OperationMetrics, SdkMetrics};
use crate::metrics::utils::{EntryCount, EntryCountStream};

#[async_trait]
impl<S> FileFolderTrait for OperationMetrics<S>
where
    S: FileFolderTrait + Send + Sync,
{
    fn inner(&self) -> Option<&dyn FileFolderTrait> {
        Some(&self.inner)
    }

    async fn list(
        &self,
        request: Request<ListRequest>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<ListResponseWithContinueToken>> {
        let timer = self.list_metrics_v1alpha.latency.start();
        match self.inner.list(request, streaming).await {
            Ok(stream) => {
                let timed = TimedStream::new(stream, timer, self.list_metrics_v1alpha.clone());
                Ok(Box::pin(EntryCountStream::new(
                    timed,
                    self.list_item_count_v1alpha.clone(),
                )))
            },
            Err(e) => {
                increment_error!(self.list_metrics_v1alpha, e);
                Err(e)
            },
        }
    }

    async fn list_stat(
        &self,
        request: Request<ListStatRequest>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<ListStatResponseWithContinueToken>> {
        let timer = self.list_stat_metrics_v1alpha.latency.start();
        match self.inner.list_stat(request, streaming).await {
            Ok(stream) => {
                let timed = TimedStream::new(stream, timer, self.list_stat_metrics_v1alpha.clone());
                Ok(Box::pin(EntryCountStream::new(
                    timed,
                    self.list_stat_item_count_v1alpha.clone(),
                )))
            },
            Err(e) => {
                increment_error!(self.list_stat_metrics_v1alpha, e);
                Err(e)
            },
        }
    }

    async fn delete_folder(&self, request: Request<DeleteFolderRequest>) -> Result<DeleteFolderResponse> {
        let _timer = self.delete_folder_metrics_v1alpha.latency.start();
        record_operation!(
            self.delete_folder_metrics_v1alpha,
            self.inner.delete_folder(request).await
        )
    }

    async fn create_folder(&self, request: Request<CreateFolderRequest>) -> Result<CreateFolderResponse> {
        let _timer = self.create_folder_metrics_v1alpha.latency.start();
        record_operation!(
            self.create_folder_metrics_v1alpha,
            self.inner.create_folder(request).await
        )
    }

    async fn get_folder_mode(&self, request: Request<GetFolderModeRequest>) -> Result<GetFolderModeResponse> {
        let _timer = self.get_folder_mode_metrics_v1alpha.latency.start();
        record_operation!(
            self.get_folder_mode_metrics_v1alpha,
            self.inner.get_folder_mode(request).await
        )
    }
}

#[async_trait]
impl<S> FileFolderTrait for SdkMetrics<S>
where
    S: FileFolderTrait + Send + Sync,
{
    fn inner(&self) -> Option<&dyn FileFolderTrait> {
        Some(&self.inner)
    }

    async fn list(
        &self,
        request: Request<ListRequest>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<ListResponseWithContinueToken>> {
        let timer = self.list_metrics_v1alpha.latency.start();

        match self.inner.list(request, streaming).await {
            Ok(stream) => Ok(Box::pin(TimedStream::new(
                stream,
                timer,
                self.list_metrics_v1alpha.clone(),
            ))),
            Err(e) => {
                increment_error!(self.list_metrics_v1alpha, e);
                Err(e)
            },
        }
    }

    async fn list_stat(
        &self,
        request: Request<ListStatRequest>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<ListStatResponseWithContinueToken>> {
        let timer = self.list_stat_metrics_v1alpha.latency.start();

        match self.inner.list_stat(request, streaming).await {
            Ok(stream) => Ok(Box::pin(TimedStream::new(
                stream,
                timer,
                self.list_stat_metrics_v1alpha.clone(),
            ))),
            Err(e) => {
                increment_error!(self.list_stat_metrics_v1alpha, e);
                Err(e)
            },
        }
    }

    async fn delete_folder(&self, request: Request<DeleteFolderRequest>) -> Result<DeleteFolderResponse> {
        let _timer = self.delete_folder_metrics_v1alpha.latency.start();

        record_operation!(
            self.delete_folder_metrics_v1alpha,
            self.inner.delete_folder(request).await
        )
    }

    async fn create_folder(&self, request: Request<CreateFolderRequest>) -> Result<CreateFolderResponse> {
        let _timer = self.create_folder_metrics_v1alpha.latency.start();
        record_operation!(
            self.create_folder_metrics_v1alpha,
            self.inner.create_folder(request).await
        )
    }

    async fn get_folder_mode(&self, request: Request<GetFolderModeRequest>) -> Result<GetFolderModeResponse> {
        let _timer = self.get_folder_mode_metrics_v1alpha.latency.start();
        record_operation!(
            self.get_folder_mode_metrics_v1alpha,
            self.inner.get_folder_mode(request).await
        )
    }
}

impl EntryCount for ListResponseWithContinueToken {
    fn entry_count(&self) -> u64 {
        self.response.subfolder_addresses.len() as u64 + self.response.sub_resource_addresses.len() as u64
    }
}

impl EntryCount for ListStatResponseWithContinueToken {
    fn entry_count(&self) -> u64 {
        self.response.entries.len() as u64 + self.response.subfolder_addresses.len() as u64
    }
}
