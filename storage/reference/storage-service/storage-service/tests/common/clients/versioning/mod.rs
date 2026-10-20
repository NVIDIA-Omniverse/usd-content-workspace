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
        pub use storage_core::traits::versioning::v1alpha::*;
    }

    pub mod v1beta {
        pub use storage_core::traits::versioning::v1beta::*;
    }
}

pub mod v1alpha {
    use futures::{stream::BoxStream, TryStreamExt};
    use tonic::Status;

    use crate::common::clients::versioning::{grpc, rest, types::v1alpha::*};

    #[derive(Debug, Clone)]
    pub enum Client {
        Grpc(grpc::v1alpha::Client),
        Rest(rest::v1alpha::Client),
    }

    impl Client {
        pub fn enumerate_versions(
            &self,
            request: EnumerateVersionsRequest,
        ) -> BoxStream<'static, Result<EnumerateVersionsResponse, Status>> {
            match self {
                Self::Grpc(client) => client.enumerate_versions(request),
                Self::Rest(client) => client.enumerate_versions(request),
            }
        }

        pub async fn enumerate_versions_collect(
            &self,
            request: EnumerateVersionsRequest,
        ) -> Result<EnumerateVersionsResponse, Status> {
            let mut result = EnumerateVersionsResponse::default();

            let mut responses = self.enumerate_versions(request);
            while let Some(response) = responses.try_next().await? {
                result.items.extend(response.items);
                result.versions_order = response.versions_order;
            }

            Ok(result)
        }

        pub async fn is_versioned(&self, request: IsVersionedRequest) -> Result<IsVersionedResponse, Status> {
            match self {
                Self::Grpc(client) => client.is_versioned(request).await,
                Self::Rest(client) => client.is_versioned(request).await,
            }
        }
    }
}

pub mod v1beta {
    use futures::{stream::BoxStream, TryStreamExt};
    use tonic::Status;

    use crate::common::clients::versioning::{grpc, rest, types::v1beta::*};

    #[derive(Debug, Clone)]
    pub enum Client {
        Grpc(grpc::v1beta::Client),
        Rest(rest::v1beta::Client),
    }

    impl Client {
        pub fn enumerate_versions(
            &self,
            request: EnumerateVersionsRequest,
        ) -> BoxStream<'static, Result<EnumerateVersionsResponse, Status>> {
            match self {
                Self::Grpc(client) => client.enumerate_versions(request),
                Self::Rest(client) => client.enumerate_versions(request),
            }
        }

        pub async fn enumerate_versions_collect(
            &self,
            request: EnumerateVersionsRequest,
        ) -> Result<EnumerateVersionsResponse, Status> {
            let mut result = EnumerateVersionsResponse::default();

            let mut responses = self.enumerate_versions(request);
            while let Some(response) = responses.try_next().await? {
                result.items.extend(response.items);
                result.versions_order = response.versions_order;
            }

            Ok(result)
        }
    }
}
