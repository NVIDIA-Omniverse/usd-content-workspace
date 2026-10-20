// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use storage_core::traits::filefolder::v1alpha::{
    CreateFolderRequest, CreateFolderResponse, DeleteFolderRequest, DeleteFolderResponse, GetFolderModeRequest,
    GetFolderModeResponse, ListRequest, ListResponse, ListStatRequest, ListStatResponse,
};
use tokio_stream::wrappers::ReceiverStream;
use tonic::Status;
#[tonic::async_trait]
pub trait FileFolder
where
    Self: std::marker::Send + std::marker::Sync + 'static,
{
    async fn list(&mut self, _request: ListRequest) -> Result<ReceiverStream<Result<ListResponse, Status>>, Status> {
        unimplemented!()
    }

    async fn list_stat(
        &mut self,
        _request: ListStatRequest,
    ) -> Result<ReceiverStream<Result<ListStatResponse, Status>>, Status> {
        unimplemented!()
    }

    async fn delete_folder(&mut self, _request: DeleteFolderRequest) -> Result<DeleteFolderResponse, Status> {
        unimplemented!()
    }

    async fn create_folder(&mut self, _request: CreateFolderRequest) -> Result<CreateFolderResponse, Status> {
        unimplemented!()
    }

    async fn get_folder_mode(&mut self, _request: GetFolderModeRequest) -> Result<GetFolderModeResponse, Status> {
        unimplemented!()
    }
}
