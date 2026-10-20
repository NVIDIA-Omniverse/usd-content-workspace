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
    use crate::common::clients::metadata::types::v1alpha::*;
    use async_stream::try_stream;
    use futures::{stream::BoxStream, TryStreamExt};
    use storage_service_protos::nvidia::omniverse::storage::metadata::v1alpha::metadata_service_client::MetadataServiceClient;
    use tonic::{transport::Channel, Response, Status};

    use crate::common::clients::utils::grpc::request_with_headers;

    #[derive(Debug, Clone)]
    pub struct Client {
        client: MetadataServiceClient<Channel>,
    }

    impl Client {
        pub fn new(channel: Channel) -> Self {
            Self {
                client: MetadataServiceClient::new(channel),
            }
        }
    }

    impl Client {
        pub async fn get_metadata(&self, request: GetMetadataRequest) -> Result<GetMetadataResponse, Status> {
            self.client
                .clone()
                .get_metadata(request_with_headers(request))
                .await
                .map(Response::into_inner)
        }

        pub async fn update_metadata(&self, request: UpdateMetadataRequest) -> Result<UpdateMetadataResponse, Status> {
            self.client
                .clone()
                .update_metadata(request_with_headers(request))
                .await
                .map(Response::into_inner)
        }

        pub async fn delete_metadata(&self, request: DeleteMetadataRequest) -> Result<DeleteMetadataResponse, Status> {
            self.client
                .clone()
                .delete_metadata(request_with_headers(request))
                .await
                .map(Response::into_inner)
        }

        pub fn get_metadata_batch(
            &self,
            request: GetMetadataBatchRequest,
        ) -> BoxStream<'static, Result<GetMetadataBatchResponse, Status>> {
            let mut client = self.client.clone();
            Box::pin(try_stream! {
                let mut responses = client.get_metadata_batch(request_with_headers(request)).await?.into_inner();
                while let Some(response) = responses.try_next().await? {
                    yield response;
                }
            })
        }

        pub fn query_metadata(
            &self,
            request: QueryMetadataRequest,
        ) -> BoxStream<'static, Result<QueryMetadataResponse, Status>> {
            let mut client = self.client.clone();
            Box::pin(try_stream! {
                let mut responses = client.query_metadata(request_with_headers(request)).await?.into_inner();
                while let Some(response) = responses.try_next().await? {
                    yield response;
                }
            })
        }

        pub fn update_metadata_batch(
            &self,
            request: UpdateMetadataBatchRequest,
        ) -> BoxStream<'static, Result<UpdateMetadataBatchResponse, Status>> {
            let mut client = self.client.clone();
            Box::pin(try_stream! {
                let mut responses = client.update_metadata_batch(request_with_headers(request)).await?.into_inner();
                while let Some(response) = responses.try_next().await? {
                    yield response;
                }
            })
        }

        pub fn delete_metadata_batch(
            &self,
            request: DeleteMetadataBatchRequest,
        ) -> BoxStream<'static, Result<DeleteMetadataBatchResponse, Status>> {
            let mut client = self.client.clone();
            Box::pin(try_stream! {
                let mut responses = client.delete_metadata_batch(request_with_headers(request)).await?.into_inner();
                while let Some(response) = responses.try_next().await? {
                    yield response;
                }
            })
        }
    }
}

pub mod v1beta {
    use crate::common::clients::metadata::types::v1beta::*;
    use async_stream::try_stream;
    use futures::{stream::BoxStream, TryStreamExt};
    use storage_service_protos::nvidia::omniverse::metadata::v1beta::metadata_service_client::MetadataServiceClient;
    use tonic::{transport::Channel, Status};

    use crate::common::clients::utils::grpc::request_with_headers;

    #[derive(Debug, Clone)]
    pub struct Client {
        client: MetadataServiceClient<Channel>,
    }

    impl Client {
        pub fn new(channel: Channel) -> Self {
            Self {
                client: MetadataServiceClient::new(channel),
            }
        }

        pub fn get_metadata_batch(
            &self,
            request: GetMetadataBatchRequest,
        ) -> BoxStream<'static, Result<GetMetadataBatchResponse, Status>> {
            let mut client = self.client.clone();
            Box::pin(try_stream! {
                let mut responses = client.get_metadata_batch(request_with_headers(request)).await?.into_inner();
                while let Some(response) = responses.try_next().await? {
                    yield response;
                }
            })
        }

        pub fn query_metadata(
            &self,
            request: QueryMetadataRequest,
        ) -> BoxStream<'static, Result<QueryMetadataResponse, Status>> {
            let mut client = self.client.clone();
            Box::pin(try_stream! {
                let mut responses = client.query_metadata(request_with_headers(request)).await?.into_inner();
                while let Some(response) = responses.try_next().await? {
                    yield response;
                }
            })
        }

        pub fn update_metadata_batch(
            &self,
            request: UpdateMetadataBatchRequest,
        ) -> BoxStream<'static, Result<UpdateMetadataBatchResponse, Status>> {
            let mut client = self.client.clone();
            Box::pin(try_stream! {
                let mut responses = client.update_metadata_batch(request_with_headers(request)).await?.into_inner();
                while let Some(response) = responses.try_next().await? {
                    yield response;
                }
            })
        }

        pub fn delete_metadata_batch(
            &self,
            request: DeleteMetadataBatchRequest,
        ) -> BoxStream<'static, Result<DeleteMetadataBatchResponse, Status>> {
            let mut client = self.client.clone();
            Box::pin(try_stream! {
                let mut responses = client.delete_metadata_batch(request_with_headers(request)).await?.into_inner();
                while let Some(response) = responses.try_next().await? {
                    yield response;
                }
            })
        }
    }
}
