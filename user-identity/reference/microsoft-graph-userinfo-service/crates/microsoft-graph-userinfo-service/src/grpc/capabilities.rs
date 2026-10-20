// SPDX-FileCopyrightText: Copyright 2026 NVIDIA CORPORATION & AFFILIATES
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

use crate::api::{rest_api_prefix, rest_api_version};
use crate::grpc::proto::user_info_service_server::SERVICE_NAME as USERINFO_GRPC_SERVICE_NAME;
use microsoft_graph_userinfo_capabilities_proto::nvidia::omniverse::discovery::capabilities::v2alpha::capabilities_service_server::CapabilitiesService;
use microsoft_graph_userinfo_capabilities_proto::nvidia::omniverse::discovery::capabilities::v2alpha::{
    GrpcEndpoint, GrpcService, ListServicesRequest, ListServicesResponse, RestEndpoint,
    RestService, RestVersion, RestVersionDetail, ServiceEntry,
};
use tonic::{Request, Response, Status};

const SERVICE_TYPE: &str = "userinfo";
const REST_API_VERSION: &str = rest_api_version!();
const REST_API_PREFIX: &str = rest_api_prefix!();

#[derive(Default)]
pub struct CapabilitiesServiceImpl;

#[tonic::async_trait]
impl CapabilitiesService for CapabilitiesServiceImpl {
    async fn list_services(
        &self,
        _request: Request<ListServicesRequest>,
    ) -> Result<Response<ListServicesResponse>, Status> {
        let service = ServiceEntry {
            service_name: SERVICE_TYPE.to_owned(),
            grpc_services: vec![GrpcService {
                endpoints: vec![GrpcEndpoint {
                    service_name: USERINFO_GRPC_SERVICE_NAME.to_owned(),
                }],
            }],
            rest_services: vec![RestService {
                endpoints: vec![RestEndpoint {
                    name: SERVICE_TYPE.to_owned(),
                    versions: vec![RestVersion {
                        version: REST_API_VERSION.to_owned(),
                        url_detail: Some(RestVersionDetail {
                            path: format!("{REST_API_PREFIX}/"),
                        }),
                    }],
                }],
            }],
        };

        Ok(Response::new(ListServicesResponse {
            services: vec![service],
            next_page_token: String::new(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn list_services_advertises_userinfo_apis() {
        let service = CapabilitiesServiceImpl;
        let response = service
            .list_services(Request::new(ListServicesRequest::default()))
            .await
            .expect("list_services should succeed")
            .into_inner();

        assert_eq!(response.services.len(), 1);
        assert!(response.next_page_token.is_empty());

        let entry = &response.services[0];
        assert_eq!(entry.service_name, "userinfo");

        assert_eq!(entry.grpc_services.len(), 1);
        assert_eq!(entry.grpc_services[0].endpoints.len(), 1);
        assert_eq!(
            entry.grpc_services[0].endpoints[0].service_name,
            "nvidia.omniverse.userinfo.v1alpha.UserInfoService"
        );

        assert_eq!(entry.rest_services.len(), 1);
        assert_eq!(entry.rest_services[0].endpoints.len(), 1);
        let rest_endpoint = &entry.rest_services[0].endpoints[0];
        assert_eq!(rest_endpoint.name, "userinfo");
        assert_eq!(rest_endpoint.versions.len(), 1);
        assert_eq!(rest_endpoint.versions[0].version, "v1alpha");
        assert_eq!(
            rest_endpoint.versions[0]
                .url_detail
                .as_ref()
                .expect("url_detail should be set")
                .path,
            "/api/v1alpha/"
        );
    }
}
