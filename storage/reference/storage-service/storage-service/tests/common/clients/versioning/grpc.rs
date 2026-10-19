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
    use storage_core_protos::nvidia::omniverse::storage::versioning::v1alpha::versioning_service_client::VersioningServiceClient;
    use tonic::{transport::Channel, Status};

    use crate::common::clients::{utils::grpc::request_with_headers, versioning::types::v1alpha::*};

    #[derive(Debug, Clone)]
    pub struct Client {
        client: VersioningServiceClient<Channel>,
    }

    impl Client {
        pub fn new(channel: Channel) -> Self {
            Self {
                client: VersioningServiceClient::new(channel),
            }
        }

        pub fn enumerate_versions(
            &self,
            request: EnumerateVersionsRequest,
        ) -> BoxStream<'static, Result<EnumerateVersionsResponse, Status>> {
            let mut client = self.client.clone();
            Box::pin(try_stream! {
                let mut responses = client.enumerate_versions(request_with_headers(request)).await?.into_inner();
                while let Some(response) = responses.try_next().await? {
                    yield response;
                }
            })
        }

        pub async fn is_versioned(&self, request: IsVersionedRequest) -> Result<IsVersionedResponse, Status> {
            self.client
                .clone()
                .is_versioned(request_with_headers(request))
                .await
                .map(|response| response.into_inner())
        }
    }
}

pub mod v1beta {
    use async_stream::try_stream;
    use futures::{stream::BoxStream, TryStreamExt};
    use storage_core_protos::nvidia::omniverse::storage::versioning::v1beta::versioning_service_client::VersioningServiceClient;
    use tonic::{transport::Channel, Status};

    use crate::common::clients::{utils::grpc::request_with_headers, versioning::types::v1beta::*};

    #[derive(Debug, Clone)]
    pub struct Client {
        client: VersioningServiceClient<Channel>,
    }

    impl Client {
        pub fn new(channel: Channel) -> Self {
            Self {
                client: VersioningServiceClient::new(channel),
            }
        }

        pub fn enumerate_versions(
            &self,
            request: EnumerateVersionsRequest,
        ) -> BoxStream<'static, Result<EnumerateVersionsResponse, Status>> {
            let mut client = self.client.clone();
            Box::pin(try_stream! {
                let mut responses = client.enumerate_versions(request_with_headers(request)).await?.into_inner();
                while let Some(response) = responses.try_next().await? {
                    yield response;
                }
            })
        }
    }
}
