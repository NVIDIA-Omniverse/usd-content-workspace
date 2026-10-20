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
    pub mod v2alpha {
        pub use storage_service_protos::nvidia::omniverse::discovery::capabilities::v2alpha::*;
    }
}

pub mod v2alpha {
    use tonic::Status;

    use crate::common::clients::discovery::{grpc, rest, types::v2alpha::*};

    #[derive(Debug, Clone)]
    pub enum Client {
        Grpc(grpc::v2alpha::Client),
        Rest(rest::v2alpha::Client),
    }

    impl Client {
        pub async fn list_services(&self, request: ListServicesRequest) -> Result<ListServicesResponse, Status> {
            match self {
                Self::Grpc(client) => client.list_services(request).await,
                Self::Rest(client) => client.list_services(request).await,
            }
        }
    }
}
