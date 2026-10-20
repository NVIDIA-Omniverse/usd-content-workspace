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

pub mod v2alpha {
    use storage_service_protos::nvidia::omniverse::discovery::capabilities::v2alpha::capabilities_service_client::CapabilitiesServiceClient;
    use tonic::{transport::Channel, Response, Status};

    use crate::common::clients::{discovery::types::v2alpha::*, utils::grpc::request_with_headers};

    #[derive(Debug, Clone)]
    pub struct Client {
        client: CapabilitiesServiceClient<Channel>,
    }

    impl Client {
        pub fn new(channel: Channel) -> Self {
            Self {
                client: CapabilitiesServiceClient::new(channel),
            }
        }

        pub async fn list_services(&self, request: ListServicesRequest) -> Result<ListServicesResponse, Status> {
            self.client
                .clone()
                .list_services(request_with_headers(request))
                .await
                .map(Response::into_inner)
        }
    }
}
