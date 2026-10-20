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

pub mod grpc;
pub mod rest;

pub mod types {
    pub mod v1alpha {
        pub use storage_core_protos::nvidia::omniverse::storage::filefolder::v1alpha::*;
    }

    pub mod v1beta {
        pub use storage_core_protos::nvidia::omniverse::storage::filefolder::v1beta::*;
    }
}

pub mod v1alpha {
    use futures::{stream::BoxStream, TryStreamExt};
    use tonic::Status;

    use crate::common::clients::filefolder::{grpc, rest, types::v1alpha::*};

    #[derive(Debug, Clone)]
    pub enum Client {
        Grpc(grpc::v1alpha::Client),
        Rest(rest::v1alpha::Client),
    }

    impl Client {
        pub fn list(&self, request: ListRequest) -> BoxStream<'static, Result<ListResponse, Status>> {
            match self {
                Self::Grpc(client) => client.list(request),
                Self::Rest(client) => client.list(request),
            }
        }

        pub fn list_stat(&self, request: ListStatRequest) -> BoxStream<'static, Result<ListStatResponse, Status>> {
            match self {
                Self::Grpc(client) => client.list_stat(request),
                Self::Rest(client) => client.list_stat(request),
            }
        }

        pub async fn delete_folder(&self, request: DeleteFolderRequest) -> Result<DeleteFolderResponse, Status> {
            match self {
                Self::Grpc(client) => client.delete_folder(request).await,
                Self::Rest(client) => client.delete_folder(request).await,
            }
        }

        pub async fn create_folder(&self, request: CreateFolderRequest) -> Result<CreateFolderResponse, Status> {
            match self {
                Self::Grpc(client) => client.create_folder(request).await,
                Self::Rest(client) => client.create_folder(request).await,
            }
        }

        pub async fn get_folder_mode(&self, request: GetFolderModeRequest) -> Result<GetFolderModeResponse, Status> {
            match self {
                Self::Grpc(client) => client.get_folder_mode(request).await,
                Self::Rest(client) => client.get_folder_mode(request).await,
            }
        }

        pub async fn list_collect(&self, request: ListRequest) -> Result<ListResponse, Status> {
            let mut result = ListResponse::default();
            let mut responses = self.list(request);
            while let Some(response) = responses.try_next().await? {
                result.sub_resource_addresses.extend(response.sub_resource_addresses);
                result.subfolder_addresses.extend(response.subfolder_addresses);
            }

            Ok(result)
        }

        pub async fn list_stat_collect(&self, request: ListStatRequest) -> Result<ListStatResponse, Status> {
            let mut result = ListStatResponse::default();
            let mut responses = self.list_stat(request);
            while let Some(response) = responses.try_next().await? {
                result.entries.extend(response.entries);
                result.subfolder_addresses.extend(response.subfolder_addresses);
            }

            Ok(result)
        }
    }
}

pub mod v1beta {
    use futures::{stream::BoxStream, TryStreamExt};
    use tonic::Status;

    use crate::common::clients::filefolder::{grpc, rest, types::v1beta::*};

    #[derive(Debug, Clone)]
    pub enum Client {
        Grpc(grpc::v1beta::Client),
        Rest(rest::v1beta::Client),
    }

    impl Client {
        pub fn list(&self, request: ListRequest) -> BoxStream<'static, Result<ListResponse, Status>> {
            match self {
                Self::Grpc(client) => client.list(request),
                Self::Rest(client) => client.list(request),
            }
        }

        pub fn list_stat(&self, request: ListStatRequest) -> BoxStream<'static, Result<ListStatResponse, Status>> {
            match self {
                Self::Grpc(client) => client.list_stat(request),
                Self::Rest(client) => client.list_stat(request),
            }
        }

        pub async fn delete_folder(&self, request: DeleteFolderRequest) -> Result<DeleteFolderResponse, Status> {
            match self {
                Self::Grpc(client) => client.delete_folder(request).await,
                Self::Rest(client) => client.delete_folder(request).await,
            }
        }

        pub async fn list_collect(&self, request: ListRequest) -> Result<ListResponse, Status> {
            let mut result = ListResponse::default();
            let mut responses = self.list(request);
            while let Some(response) = responses.try_next().await? {
                result.sub_resource_addresses.extend(response.sub_resource_addresses);
                result.subfolder_addresses.extend(response.subfolder_addresses);
            }

            Ok(result)
        }

        pub async fn list_stat_collect(&self, request: ListStatRequest) -> Result<ListStatResponse, Status> {
            let mut result = ListStatResponse::default();
            let mut responses = self.list_stat(request);
            while let Some(response) = responses.try_next().await? {
                result.entries.extend(response.entries);
                result.subfolder_addresses.extend(response.subfolder_addresses);
            }

            Ok(result)
        }
    }
}
