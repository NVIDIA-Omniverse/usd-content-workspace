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

    use reqwest::StatusCode;
    use tonic::Status;

    use crate::common::clients::{
        capabilities::types::v1alpha::*,
        utils::{
            get_headers,
            rest::{RequestBuilderExt, ResponseExt},
        },
    };

    pub mod models {
        use crate::common::clients::capabilities::types::v1alpha;
        use serde::{Deserialize, Serialize};
        use storage_core::traits::capabilities::v1alpha::route::RouteType;

        #[derive(Serialize, Deserialize, Debug)]
        pub enum Route {
            #[serde(rename = "wildcard_pattern")]
            WildcardPattern(String),
        }

        impl From<Route> for v1alpha::Route {
            fn from(value: Route) -> Self {
                Self {
                    route_type: Some(match value {
                        Route::WildcardPattern(value) => RouteType::WildcardPattern(value),
                    }),
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct TopLevelAddressEntry {
            pub top_level_address: String,
        }

        impl From<TopLevelAddressEntry> for v1alpha::TopLevelAddressEntry {
            fn from(value: TopLevelAddressEntry) -> Self {
                Self {
                    top_level_address: value.top_level_address,
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct ListRoutesResponse {
            pub items: Vec<Route>,
        }

        impl From<ListRoutesResponse> for v1alpha::ListRoutesResponse {
            fn from(value: ListRoutesResponse) -> Self {
                Self {
                    items: value.items.into_iter().map(Into::into).collect(),
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct ListTopLevelAddressesResponse {
            pub items: Vec<TopLevelAddressEntry>,
        }

        impl From<ListTopLevelAddressesResponse> for v1alpha::ListTopLevelAddressesResponse {
            fn from(value: ListTopLevelAddressesResponse) -> Self {
                Self {
                    items: value.items.into_iter().map(Into::into).collect(),
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct ListServicesResponse {
            pub services: Vec<ServiceEntry>,
        }

        impl From<ListServicesResponse> for v1alpha::ListServicesResponse {
            fn from(value: ListServicesResponse) -> Self {
                Self {
                    services: value.services.into_iter().map(Into::into).collect(),
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct ServiceEntry {
            pub service_name: String,
            pub service_versions: Vec<String>,
        }

        impl From<ServiceEntry> for v1alpha::ServiceEntry {
            fn from(value: ServiceEntry) -> Self {
                Self {
                    service_name: value.service_name,
                    service_versions: value.service_versions,
                }
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

        pub async fn list_services(&self, _request: ListServicesRequest) -> Result<ListServicesResponse, Status> {
            let response = self
                .client
                .get(format!("{}/v1alpha/capabilities/services", self.base_url))
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

        pub async fn list_routes(&self, _request: ListRoutesRequest) -> Result<ListRoutesResponse, Status> {
            let response = self
                .client
                .get(format!("{}/v1alpha/capabilities/routes", self.base_url))
                .headers(get_headers())
                .send_with_retry()
                .await
                .expect("list routes request failed");

            if response.status() != StatusCode::OK {
                return Err(response.to_tonic_status());
            }

            let response: models::ListRoutesResponse = response.json().await.expect("response body read failed");

            Ok(response.into())
        }

        pub async fn list_top_level_addresses(
            &self,
            _request: ListTopLevelAddressesRequest,
        ) -> Result<ListTopLevelAddressesResponse, Status> {
            let response = self
                .client
                .get(format!("{}/v1alpha/capabilities/top-level-addresses", self.base_url))
                .headers(get_headers())
                .send_with_retry()
                .await
                .expect("list top level addresses request failed");

            if response.status() != StatusCode::OK {
                return Err(response.to_tonic_status());
            }

            let response: models::ListTopLevelAddressesResponse =
                response.json().await.expect("response body read failed");

            Ok(response.into())
        }
    }
}

pub mod v1beta {

    use reqwest::StatusCode;
    use tonic::Status;

    use crate::common::clients::{
        capabilities::types::v1beta::*,
        utils::{
            get_headers,
            rest::{RequestBuilderExt, ResponseExt},
        },
    };

    pub mod models {
        use crate::common::clients::capabilities::types::v1beta;
        use serde::{Deserialize, Serialize};

        #[derive(Serialize, Deserialize, Debug)]
        pub struct TopLevelAddressEntry {
            pub top_level_address: String,
        }

        impl From<TopLevelAddressEntry> for v1beta::TopLevelAddressEntry {
            fn from(value: TopLevelAddressEntry) -> Self {
                Self {
                    top_level_address: value.top_level_address,
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct ListTopLevelAddressesResponse {
            pub items: Vec<TopLevelAddressEntry>,
        }

        impl From<ListTopLevelAddressesResponse> for v1beta::ListTopLevelAddressesResponse {
            fn from(value: ListTopLevelAddressesResponse) -> Self {
                Self {
                    items: value.items.into_iter().map(Into::into).collect(),
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct ListServicesResponse {
            pub services: Vec<ServiceEntry>,
        }

        impl From<ListServicesResponse> for v1beta::ListServicesResponse {
            fn from(value: ListServicesResponse) -> Self {
                Self {
                    services: value.services.into_iter().map(Into::into).collect(),
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct ServiceEntry {
            pub service_name: String,
            pub service_versions: Vec<String>,
        }

        impl From<ServiceEntry> for v1beta::ServiceEntry {
            fn from(value: ServiceEntry) -> Self {
                Self {
                    service_name: value.service_name,
                    service_versions: value.service_versions,
                }
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

        pub async fn list_services(&self, _request: ListServicesRequest) -> Result<ListServicesResponse, Status> {
            let response = self
                .client
                .get(format!("{}/v1alpha/capabilities/services", self.base_url))
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

        pub async fn list_top_level_addresses(
            &self,
            _request: ListTopLevelAddressesRequest,
        ) -> Result<ListTopLevelAddressesResponse, Status> {
            let response = self
                .client
                .get(format!("{}/v1alpha/capabilities/top-level-addresses", self.base_url))
                .headers(get_headers())
                .send_with_retry()
                .await
                .expect("list top level addresses request failed");

            if response.status() != StatusCode::OK {
                return Err(response.to_tonic_status());
            }

            let response: models::ListTopLevelAddressesResponse =
                response.json().await.expect("response body read failed");

            Ok(response.into())
        }
    }
}
