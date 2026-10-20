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
        pub use storage_core_protos::nvidia::omniverse::storage::capabilities::v1alpha::*;
    }

    pub mod v1beta {
        pub use storage_core_protos::nvidia::omniverse::storage::capabilities::v1beta::*;
    }
}

pub mod v1alpha {
    use tonic::Status;

    use crate::common::clients::capabilities::{grpc, rest, types::v1alpha::*};

    #[derive(Debug, Clone)]
    pub enum Client {
        Grpc(grpc::v1alpha::Client),
        Rest(rest::v1alpha::Client),
    }

    impl Client {
        pub async fn list_services(&self, request: ListServicesRequest) -> Result<ListServicesResponse, Status> {
            match self {
                Self::Grpc(client) => client.list_services(request).await,
                Self::Rest(client) => client.list_services(request).await,
            }
        }

        pub async fn list_routes(&self, request: ListRoutesRequest) -> Result<ListRoutesResponse, Status> {
            match self {
                Self::Grpc(client) => client.list_routes(request).await,
                Self::Rest(client) => client.list_routes(request).await,
            }
        }

        pub async fn list_top_level_addresses(
            &self,
            request: ListTopLevelAddressesRequest,
        ) -> Result<ListTopLevelAddressesResponse, Status> {
            match self {
                Self::Grpc(client) => client.list_top_level_addresses(request).await,
                Self::Rest(client) => client.list_top_level_addresses(request).await,
            }
        }

        pub async fn list_services_alphabetically(
            &self,
            request: ListServicesRequest,
        ) -> Result<ListServicesResponse, Status> {
            let mut response = self.list_services(request).await?;
            response.services.sort_by(|a, b| a.service_name.cmp(&b.service_name));

            for entry in response.services.iter_mut() {
                entry.service_versions.sort();
            }

            Ok(response)
        }
    }
}

pub mod v1beta {
    use tonic::Status;

    use crate::common::clients::capabilities::{grpc, rest, types::v1beta::*};

    #[derive(Debug, Clone)]
    pub enum Client {
        Grpc(grpc::v1beta::Client),
        Rest(rest::v1beta::Client),
    }

    impl Client {
        pub async fn list_services(&self, request: ListServicesRequest) -> Result<ListServicesResponse, Status> {
            match self {
                Self::Grpc(client) => client.list_services(request).await,
                Self::Rest(client) => client.list_services(request).await,
            }
        }

        pub async fn list_top_level_addresses(
            &self,
            request: ListTopLevelAddressesRequest,
        ) -> Result<ListTopLevelAddressesResponse, Status> {
            match self {
                Self::Grpc(client) => client.list_top_level_addresses(request).await,
                Self::Rest(client) => client.list_top_level_addresses(request).await,
            }
        }

        pub async fn list_services_alphabetically(
            &self,
            request: ListServicesRequest,
        ) -> Result<ListServicesResponse, Status> {
            let mut response = self.list_services(request).await?;
            response.services.sort_by(|a, b| a.service_name.cmp(&b.service_name));

            for entry in response.services.iter_mut() {
                entry.service_versions.sort();
            }

            Ok(response)
        }
    }
}
