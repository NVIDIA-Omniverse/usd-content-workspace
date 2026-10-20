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
    use storage_core::{ingress::ToGrpcService, traits::async_trait};
    use storage_service_protos::nvidia::omniverse::discovery::capabilities::v2alpha::{
        capabilities_service_server::{CapabilitiesService, CapabilitiesServiceServer},
        ListServicesRequest, ListServicesResponse, FILE_DESCRIPTOR_SET,
    };
    use tonic::{Request, Response, Status};

    use crate::api::discovery::v2alpha::Capabilities;

    impl ToGrpcService for Capabilities {
        type Service = CapabilitiesServiceServer<Capabilities>;

        fn file_descriptor_set(&self) -> Option<&[u8]> {
            Some(FILE_DESCRIPTOR_SET)
        }

        fn service(&self) -> Self::Service {
            CapabilitiesServiceServer::new(self.clone())
        }
    }

    #[async_trait]
    impl CapabilitiesService for Capabilities {
        async fn list_services(
            &self,
            request: Request<ListServicesRequest>,
        ) -> Result<Response<ListServicesResponse>, Status> {
            let request = request.into_inner();
            let response = self
                .list_services_page(request.page_size, &request.page_token)
                .map_err(|_| Status::invalid_argument("page token is invalid"))?;

            Ok(Response::new(response))
        }
    }
}
