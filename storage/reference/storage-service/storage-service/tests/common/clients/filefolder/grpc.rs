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

pub mod v1alpha {
    use async_stream::try_stream;
    use futures::{stream::BoxStream, TryStreamExt};
    use storage_core_protos::nvidia::omniverse::storage::filefolder::v1alpha::file_folder_service_client::FileFolderServiceClient;
    use tonic::{transport::Channel, Response, Status};

    use crate::common::clients::{filefolder::types::v1alpha::*, utils::grpc::request_with_headers};

    #[derive(Debug, Clone)]
    pub struct Client {
        client: FileFolderServiceClient<Channel>,
    }

    impl Client {
        pub fn new(channel: Channel) -> Self {
            Self {
                client: FileFolderServiceClient::new(channel),
            }
        }

        pub fn list(&self, request: ListRequest) -> BoxStream<'static, Result<ListResponse, Status>> {
            let mut client = self.client.clone();
            Box::pin(try_stream! {
                let mut responses = client.list(request_with_headers(request)).await?.into_inner();
                while let Some(response) = responses.try_next().await? {
                    yield response;
                }
            })
        }

        pub fn list_stat(&self, request: ListStatRequest) -> BoxStream<'static, Result<ListStatResponse, Status>> {
            let mut client = self.client.clone();
            Box::pin(try_stream! {
                let mut responses = client.list_stat(request_with_headers(request)).await?.into_inner();
                while let Some(response) = responses.try_next().await? {
                    yield response;
                }
            })
        }

        pub async fn delete_folder(&self, request: DeleteFolderRequest) -> Result<DeleteFolderResponse, Status> {
            self.client
                .clone()
                .delete_folder(request_with_headers(request))
                .await
                .map(Response::into_inner)
        }

        pub async fn create_folder(&self, request: CreateFolderRequest) -> Result<CreateFolderResponse, Status> {
            self.client
                .clone()
                .create_folder(request_with_headers(request))
                .await
                .map(Response::into_inner)
        }

        pub async fn get_folder_mode(&self, request: GetFolderModeRequest) -> Result<GetFolderModeResponse, Status> {
            self.client
                .clone()
                .get_folder_mode(request_with_headers(request))
                .await
                .map(Response::into_inner)
        }
    }
}

pub mod v1beta {
    use async_stream::try_stream;
    use futures::{stream::BoxStream, TryStreamExt};
    use storage_core_protos::nvidia::omniverse::storage::filefolder::v1beta::file_folder_service_client::FileFolderServiceClient;
    use tonic::{transport::Channel, Response, Status};

    use crate::common::clients::{filefolder::types::v1beta::*, utils::grpc::request_with_headers};

    #[derive(Debug, Clone)]
    pub struct Client {
        client: FileFolderServiceClient<Channel>,
    }

    impl Client {
        pub fn new(channel: Channel) -> Self {
            Self {
                client: FileFolderServiceClient::new(channel),
            }
        }

        pub fn list(&self, request: ListRequest) -> BoxStream<'static, Result<ListResponse, Status>> {
            let mut client = self.client.clone();
            Box::pin(try_stream! {
                let mut responses = client.list(request_with_headers(request)).await?.into_inner();
                while let Some(response) = responses.try_next().await? {
                    yield response;
                }
            })
        }

        pub fn list_stat(&self, request: ListStatRequest) -> BoxStream<'static, Result<ListStatResponse, Status>> {
            let mut client = self.client.clone();
            Box::pin(try_stream! {
                let mut responses = client.list_stat(request_with_headers(request)).await?.into_inner();
                while let Some(response) = responses.try_next().await? {
                    yield response;
                }
            })
        }

        pub async fn delete_folder(&self, request: DeleteFolderRequest) -> Result<DeleteFolderResponse, Status> {
            self.client
                .clone()
                .delete_folder(request_with_headers(request))
                .await
                .map(Response::into_inner)
        }
    }
}
