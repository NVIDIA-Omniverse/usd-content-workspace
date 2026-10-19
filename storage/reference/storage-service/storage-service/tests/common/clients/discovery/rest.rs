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
    use reqwest::StatusCode;
    use tonic::Status;

    use crate::common::clients::{
        discovery::types::v2alpha::*,
        utils::{
            get_headers,
            rest::{RequestBuilderExt, ResponseExt},
        },
    };

    pub mod models {
        use serde::{Deserialize, Serialize};

        use crate::common::clients::discovery::types::v2alpha;

        #[derive(Serialize, Deserialize, Debug)]
        pub struct ListServicesResponse {
            pub services: Vec<ServiceEntry>,
            pub next_page_token: String,
        }

        impl From<ListServicesResponse> for v2alpha::ListServicesResponse {
            fn from(value: ListServicesResponse) -> Self {
                Self {
                    services: value.services.into_iter().map(Into::into).collect(),
                    next_page_token: value.next_page_token,
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct ServiceEntry {
            pub service_name: String,
            pub grpc_services: Vec<GrpcService>,
            pub rest_services: Vec<RestService>,
        }

        impl From<ServiceEntry> for v2alpha::ServiceEntry {
            fn from(value: ServiceEntry) -> Self {
                Self {
                    service_name: value.service_name,
                    grpc_services: value.grpc_services.into_iter().map(Into::into).collect(),
                    rest_services: value.rest_services.into_iter().map(Into::into).collect(),
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct GrpcService {
            pub endpoints: Vec<GrpcEndpoint>,
        }

        impl From<GrpcService> for v2alpha::GrpcService {
            fn from(value: GrpcService) -> Self {
                Self {
                    endpoints: value.endpoints.into_iter().map(Into::into).collect(),
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct GrpcEndpoint {
            pub service_name: String,
        }

        impl From<GrpcEndpoint> for v2alpha::GrpcEndpoint {
            fn from(value: GrpcEndpoint) -> Self {
                Self {
                    service_name: value.service_name,
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct RestService {
            pub endpoints: Vec<RestEndpoint>,
        }

        impl From<RestService> for v2alpha::RestService {
            fn from(value: RestService) -> Self {
                Self {
                    endpoints: value.endpoints.into_iter().map(Into::into).collect(),
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct RestEndpoint {
            pub name: String,
            pub versions: Vec<RestVersion>,
        }

        impl From<RestEndpoint> for v2alpha::RestEndpoint {
            fn from(value: RestEndpoint) -> Self {
                Self {
                    name: value.name,
                    versions: value.versions.into_iter().map(Into::into).collect(),
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct RestVersion {
            pub version: String,
            pub url_detail: Option<RestVersionDetail>,
        }

        impl From<RestVersion> for v2alpha::RestVersion {
            fn from(value: RestVersion) -> Self {
                Self {
                    version: value.version,
                    url_detail: value.url_detail.map(Into::into),
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct RestVersionDetail {
            pub path: String,
        }

        impl From<RestVersionDetail> for v2alpha::RestVersionDetail {
            fn from(value: RestVersionDetail) -> Self {
                Self { path: value.path }
            }
        }
    }

    #[derive(Debug, Clone)]
    pub struct Client {
        client: reqwest::Client,
        base_url: String,
    }

    impl Client {
        pub fn new(client: reqwest::Client, base_url: impl Into<String>) -> Self {
            Self {
                client,
                base_url: base_url.into(),
            }
        }

        pub async fn list_services(&self, request: ListServicesRequest) -> Result<ListServicesResponse, Status> {
            let mut query = Vec::new();
            if request.page_size != 0 {
                query.push(("page_size", request.page_size.to_string()));
            }
            if !request.page_token.is_empty() {
                query.push(("page_token", request.page_token));
            }

            let response = self
                .client
                .get(format!("{}/v2alpha/services", self.base_url))
                .query(&query)
                .headers(get_headers())
                .send_with_retry()
                .await
                .expect("list services request failed");

            if response.status() != StatusCode::OK {
                return Err(response.to_tonic_status());
            }

            let response: models::ListServicesResponse = response.json().await.expect("response body read failed");
            Ok(response.into())
        }
    }
}
