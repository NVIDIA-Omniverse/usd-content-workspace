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
    use futures::stream::BoxStream;
    use reqwest::StatusCode;
    use tonic::Status;

    use crate::common::clients::{
        utils::{
            get_headers,
            rest::{encode_path_segment, encode_query_params, RequestBuilderExt, ResponseExt},
        },
        versioning::types::v1alpha::*,
    };

    pub mod models {
        use serde::{Deserialize, Serialize};

        use crate::common::clients::{fileobject::rest::v1alpha::models::ResourceInfo, versioning::types::v1alpha};

        #[derive(Serialize, Deserialize, Debug)]
        pub enum VersionsOrder {
            #[serde(rename = "newest_first")]
            NewestFirst,
            #[serde(rename = "oldest_first")]
            OldestFirst,
            #[serde(rename = "by_key")]
            ByKey,
        }

        impl From<VersionsOrder> for i32 {
            fn from(value: VersionsOrder) -> Self {
                match value {
                    VersionsOrder::ByKey => v1alpha::VersionsOrder::ByKey as i32,
                    VersionsOrder::OldestFirst => v1alpha::VersionsOrder::OldestFirst as i32,
                    VersionsOrder::NewestFirst => v1alpha::VersionsOrder::NewestFirst as i32,
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct VersionInfo {
            pub resource_info: ResourceInfo,
            pub resource_address: Option<String>,
            pub sorting_key: Option<String>,
        }

        impl From<VersionInfo> for v1alpha::VersionInfo {
            fn from(value: VersionInfo) -> Self {
                Self {
                    resource_info: Some(value.resource_info.into()),
                    resource_address: value.resource_address,
                    sorting_key: value.sorting_key,
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct VersionInfoPage {
            pub items: Vec<VersionInfo>,
            pub versions_order: VersionsOrder,
            pub next_continuation_handle: Option<String>,
        }

        impl From<VersionInfoPage> for v1alpha::EnumerateVersionsResponse {
            fn from(value: VersionInfoPage) -> Self {
                Self {
                    items: value.items.into_iter().map(Into::into).collect(),
                    versions_order: value.versions_order.into(),
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct IsVersionedResponse {
            pub is_versioned: bool,
        }

        impl From<IsVersionedResponse> for v1alpha::IsVersionedResponse {
            fn from(value: IsVersionedResponse) -> Self {
                Self {
                    is_versioned: value.is_versioned,
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

        pub fn enumerate_versions(
            &self,
            request: EnumerateVersionsRequest,
        ) -> BoxStream<'static, Result<EnumerateVersionsResponse, Status>> {
            let client = self.clone();
            Box::pin(try_stream! {
                let mut continuation_handle: Option<String> = None;
                loop {
                    let (response, next_continuation_handle) = client.enumerate_versions_once(
                        &request,
                        continuation_handle.as_deref(),
                    ).await?;
                    yield response;

                    if next_continuation_handle.is_some() {
                        continuation_handle = next_continuation_handle;
                    } else {
                        break;
                    }
                }
            })
        }

        pub async fn enumerate_versions_once(
            &self,
            request: &EnumerateVersionsRequest,
            continuation_handle: Option<&str>,
        ) -> Result<(EnumerateVersionsResponse, Option<String>), Status> {
            let mut query_params = Vec::new();
            if let Some(continuation_handle) = continuation_handle {
                query_params.push(format!("continuation_handle={}", continuation_handle));
            }

            let response = self
                .client
                .get(format!(
                    "{}/v1alpha/versioning/{}/versions{}",
                    self.base_url,
                    encode_path_segment(&request.resource_address),
                    encode_query_params(&query_params),
                ))
                .headers(get_headers())
                .send_with_retry()
                .await
                .expect("list request failed");

            if response.status() != StatusCode::OK {
                return Err(response.to_tonic_status());
            }

            let mut response: models::VersionInfoPage = response.json().await.expect("response body read failed");
            let continuation_handle = std::mem::take(&mut response.next_continuation_handle);

            Ok((response.into(), continuation_handle))
        }

        pub async fn is_versioned(&self, request: IsVersionedRequest) -> Result<IsVersionedResponse, Status> {
            let response = self
                .client
                .get(format!(
                    "{}/v1alpha/versioning/is-versioned/{}",
                    self.base_url,
                    encode_path_segment(&request.resource_address),
                ))
                .headers(get_headers())
                .send_with_retry()
                .await
                .expect("is-versioned request failed");

            if response.status() != StatusCode::OK {
                return Err(response.to_tonic_status());
            }

            let response: models::IsVersionedResponse = response.json().await.expect("response body read failed");

            Ok(response.into())
        }
    }
}

pub mod v1beta {
    use async_stream::try_stream;
    use futures::stream::BoxStream;
    use reqwest::StatusCode;
    use tonic::Status;

    use crate::common::clients::{
        utils::{
            get_headers,
            rest::{encode_path_segment, encode_query_params, RequestBuilderExt, ResponseExt},
        },
        versioning::types::v1beta::*,
    };

    pub mod models {
        use serde::{Deserialize, Serialize};

        use crate::common::clients::{fileobject::rest::v1beta::models::ResourceInfo, versioning::types::v1beta};

        #[derive(Serialize, Deserialize, Debug)]
        pub enum VersionsOrder {
            #[serde(rename = "newest_first")]
            NewestFirst,
            #[serde(rename = "oldest_first")]
            OldestFirst,
            #[serde(rename = "by_key")]
            ByKey,
        }

        impl From<VersionsOrder> for i32 {
            fn from(value: VersionsOrder) -> Self {
                match value {
                    VersionsOrder::ByKey => v1beta::VersionsOrder::ByKey as i32,
                    VersionsOrder::OldestFirst => v1beta::VersionsOrder::OldestFirst as i32,
                    VersionsOrder::NewestFirst => v1beta::VersionsOrder::NewestFirst as i32,
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct VersionInfo {
            pub resource_info: ResourceInfo,
            pub resource_address: Option<String>,
            pub sorting_key: Option<String>,
        }

        impl From<VersionInfo> for v1beta::VersionInfo {
            fn from(value: VersionInfo) -> Self {
                Self {
                    resource_info: Some(value.resource_info.into()),
                    sorting_key: value.sorting_key,
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct VersionInfoPage {
            pub items: Vec<VersionInfo>,
            pub versions_order: VersionsOrder,
            pub next_continuation_handle: Option<String>,
        }

        impl From<VersionInfoPage> for v1beta::EnumerateVersionsResponse {
            fn from(value: VersionInfoPage) -> Self {
                Self {
                    items: value.items.into_iter().map(Into::into).collect(),
                    versions_order: value.versions_order.into(),
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

        pub fn enumerate_versions(
            &self,
            request: EnumerateVersionsRequest,
        ) -> BoxStream<'static, Result<EnumerateVersionsResponse, Status>> {
            let client = self.clone();
            Box::pin(try_stream! {
                let mut continuation_handle: Option<String> = None;
                loop {
                    let (response, next_continuation_handle) = client.enumerate_versions_once(
                        &request,
                        continuation_handle.as_deref(),
                    ).await?;
                    yield response;

                    if next_continuation_handle.is_some() {
                        continuation_handle = next_continuation_handle;
                    } else {
                        break;
                    }
                }
            })
        }

        pub async fn enumerate_versions_once(
            &self,
            request: &EnumerateVersionsRequest,
            continuation_handle: Option<&str>,
        ) -> Result<(EnumerateVersionsResponse, Option<String>), Status> {
            let mut query_params = Vec::new();
            if let Some(continuation_handle) = continuation_handle {
                query_params.push(format!("continuation_handle={}", continuation_handle));
            }

            let response = self
                .client
                .get(format!(
                    "{}/v1alpha/versioning/{}/versions{}",
                    self.base_url,
                    encode_path_segment(&request.resource_address),
                    encode_query_params(&query_params),
                ))
                .headers(get_headers())
                .send_with_retry()
                .await
                .expect("list request failed");

            if response.status() != StatusCode::OK {
                return Err(response.to_tonic_status());
            }

            let mut response: models::VersionInfoPage = response.json().await.expect("response body read failed");
            let continuation_handle = std::mem::take(&mut response.next_continuation_handle);

            Ok((response.into(), continuation_handle))
        }
    }
}
