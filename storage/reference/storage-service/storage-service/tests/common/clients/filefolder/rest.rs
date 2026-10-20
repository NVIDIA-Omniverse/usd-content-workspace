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
        filefolder::types::v1alpha::*,
        utils::{
            get_headers,
            rest::{encode_path_segment, encode_query_params, RequestBuilderExt, ResponseExt},
        },
    };

    pub mod models {
        use serde::{Deserialize, Serialize};
        use storage_core::traits::filefolder::v1alpha::FolderAddress;

        use crate::common::clients::{
            filefolder::types::v1alpha, fileobject::rest::v1alpha::models::Metadata,
            fileobject::types::v1alpha as fileobject_v1alpha,
        };

        #[derive(Serialize, Deserialize, Debug)]
        pub struct ListResult {
            pub subfolder_addresses: Vec<String>,
            pub sub_resource_addresses: Vec<String>,
            pub next_continuation_handle: Option<String>,
        }

        impl From<ListResult> for v1alpha::ListResponse {
            fn from(value: ListResult) -> Self {
                Self {
                    subfolder_addresses: value
                        .subfolder_addresses
                        .into_iter()
                        .map(|uri| FolderAddress { uri })
                        .collect(),
                    sub_resource_addresses: value.sub_resource_addresses,
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct ListStatResult {
            pub subfolder_addresses: Vec<String>,
            pub entries: Vec<ListItem>,
            pub next_continuation_handle: Option<String>,
        }

        impl From<ListStatResult> for v1alpha::ListStatResponse {
            fn from(value: ListStatResult) -> Self {
                Self {
                    subfolder_addresses: value
                        .subfolder_addresses
                        .into_iter()
                        .map(|uri| FolderAddress { uri })
                        .collect(),
                    entries: value.entries.into_iter().map(Into::into).collect(),
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct ListItem {
            pub resource_address: String,
            pub resource_identity: String,
            pub metadata: Metadata,
        }

        impl From<ListItem> for v1alpha::ListItem {
            fn from(value: ListItem) -> Self {
                Self {
                    resource_address: value.resource_address,
                    resource_info: Some(fileobject_v1alpha::ResourceInfo {
                        metadata: Some(value.metadata.into()),
                        resource_identity: Some(fileobject_v1alpha::ResourceIdentity {
                            encoded_identity: value.resource_identity,
                        }),
                    }),
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

        pub fn list(&self, request: ListRequest) -> BoxStream<'static, Result<ListResponse, Status>> {
            self.list_with_page_size(request, None)
        }

        /// Same as [`Self::list`] but lets callers cap the per-page size via the
        /// `max_page_size` query parameter, the only wire-level knob that
        /// exercises the synthetic-folder page-size contract end-to-end.
        pub fn list_with_page_size(
            &self,
            request: ListRequest,
            max_page_size: Option<u32>,
        ) -> BoxStream<'static, Result<ListResponse, Status>> {
            let client = self.clone();
            Box::pin(try_stream! {
                let mut continuation_handle: Option<String> = None;
                loop {
                    let (response, next_continuation_handle) = client.list_once(&request, continuation_handle.as_deref(), max_page_size).await?;
                    yield response;

                    if next_continuation_handle.is_some() {
                        continuation_handle = next_continuation_handle;
                    } else {
                        break;
                    }
                }
            })
        }

        pub fn list_stat(&self, request: ListStatRequest) -> BoxStream<'static, Result<ListStatResponse, Status>> {
            self.list_stat_with_page_size(request, None)
        }

        pub fn list_stat_with_page_size(
            &self,
            request: ListStatRequest,
            max_page_size: Option<u32>,
        ) -> BoxStream<'static, Result<ListStatResponse, Status>> {
            let client = self.clone();
            Box::pin(try_stream! {
                let mut continuation_handle: Option<String> = None;
                loop {
                    let (response, next_continuation_handle) = client.list_stat_once(&request, continuation_handle.as_deref(), max_page_size).await?;
                    yield response;

                    if next_continuation_handle.is_some() {
                        continuation_handle = next_continuation_handle;
                    } else {
                        break;
                    }
                }
            })
        }

        pub async fn delete_folder(&self, request: DeleteFolderRequest) -> Result<DeleteFolderResponse, Status> {
            let response = self
                .client
                .delete(format!(
                    "{}/v1alpha/filefolder/list/{}",
                    self.base_url,
                    encode_path_segment(&request.folder.expect("foled address is missing").uri)
                ))
                .headers(get_headers())
                .send_with_retry()
                .await
                .expect("delete request failed");

            if response.status() != StatusCode::NO_CONTENT {
                return Err(response.to_tonic_status());
            }

            Ok(Default::default())
        }

        pub async fn create_folder(&self, request: CreateFolderRequest) -> Result<CreateFolderResponse, Status> {
            let response = self
                .client
                .put(format!(
                    "{}/v1alpha/filefolder/{}",
                    self.base_url,
                    encode_path_segment(&request.folder.expect("foled address is missing").uri)
                ))
                .headers(get_headers())
                .send_with_retry()
                .await
                .expect("create folder request failed");

            if response.status() != StatusCode::NO_CONTENT {
                return Err(response.to_tonic_status());
            }

            Ok(Default::default())
        }

        pub async fn get_folder_mode(&self, request: GetFolderModeRequest) -> Result<GetFolderModeResponse, Status> {
            let response = self
                .client
                .get(format!(
                    "{}/v1alpha/filefolder/get-folder-mode/{}",
                    self.base_url,
                    encode_path_segment(&request.folder.expect("foled address is missing").uri)
                ))
                .headers(get_headers())
                .send_with_retry()
                .await
                .expect("get folder mode request failed");

            if response.status() != StatusCode::OK {
                return Err(response.to_tonic_status());
            }

            let result: GetFolderModeResponse = response.json().await.expect("response body read failed");

            Ok(result)
        }

        pub async fn list_once(
            &self,
            request: &ListRequest,
            continuation_handle: Option<&str>,
            max_page_size: Option<u32>,
        ) -> Result<(ListResponse, Option<String>), Status> {
            let mut query_params = Vec::new();
            if let Some(continuation_handle) = continuation_handle {
                query_params.push(format!("continuation_handle={}", continuation_handle));
            }
            if let Some(max_page_size) = max_page_size {
                query_params.push(format!("max_page_size={}", max_page_size));
            }

            let response = self
                .client
                .get(format!(
                    "{}/v1alpha/filefolder/list/{}{}",
                    self.base_url,
                    encode_path_segment(request.folder.as_ref().expect("folder address is missing").uri.as_str()),
                    encode_query_params(&query_params),
                ))
                .headers(get_headers())
                .send_with_retry()
                .await
                .expect("list request failed");

            if response.status() != StatusCode::OK {
                return Err(response.to_tonic_status());
            }

            let mut response: models::ListResult = response.json().await.expect("response body read failed");
            let continuation_handle = std::mem::take(&mut response.next_continuation_handle);

            Ok((response.into(), continuation_handle))
        }

        pub async fn list_stat_once(
            &self,
            request: &ListStatRequest,
            continuation_handle: Option<&str>,
            max_page_size: Option<u32>,
        ) -> Result<(ListStatResponse, Option<String>), Status> {
            let mut query_params = Vec::new();
            if let Some(continuation_handle) = continuation_handle {
                query_params.push(format!("continuation_handle={}", continuation_handle));
            }
            if let Some(max_page_size) = max_page_size {
                query_params.push(format!("max_page_size={}", max_page_size));
            }

            let response = self
                .client
                .get(format!(
                    "{}/v1alpha/filefolder/liststat/{}{}",
                    self.base_url,
                    encode_path_segment(request.folder.as_ref().expect("folder address is missing").uri.as_str()),
                    encode_query_params(&query_params),
                ))
                .headers(get_headers())
                .send_with_retry()
                .await
                .expect("list request failed");

            if response.status() != StatusCode::OK {
                return Err(response.to_tonic_status());
            }

            let mut response: models::ListStatResult = response.json().await.expect("response body read failed");
            let continuation_handle = std::mem::take(&mut response.next_continuation_handle);

            Ok((response.into(), continuation_handle))
        }
    }
}

pub mod v1beta {
    use async_stream::try_stream;
    use futures::stream::BoxStream;
    use reqwest::StatusCode;
    use tonic::Status;

    use crate::common::clients::{
        filefolder::types::v1beta::*,
        utils::{
            get_headers,
            rest::{encode_path_segment, encode_query_params, RequestBuilderExt, ResponseExt},
        },
    };

    pub mod models {
        use serde::{Deserialize, Serialize};
        use storage_core::traits::filefolder::v1beta::FolderAddress;

        use crate::common::clients::{
            filefolder::types::v1beta, fileobject::rest::v1beta::models::Metadata,
            fileobject::types::v1beta as fileobject_v1beta,
        };

        #[derive(Serialize, Deserialize, Debug)]
        pub struct ListResult {
            pub subfolder_addresses: Vec<String>,
            pub sub_resource_addresses: Vec<String>,
            pub next_continuation_handle: Option<String>,
        }

        impl From<ListResult> for v1beta::ListResponse {
            fn from(value: ListResult) -> Self {
                Self {
                    subfolder_addresses: value
                        .subfolder_addresses
                        .into_iter()
                        .map(|uri| FolderAddress { uri })
                        .collect(),
                    sub_resource_addresses: value.sub_resource_addresses,
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct ListStatResult {
            pub subfolder_addresses: Vec<String>,
            pub entries: Vec<ListItem>,
            pub next_continuation_handle: Option<String>,
        }

        impl From<ListStatResult> for v1beta::ListStatResponse {
            fn from(value: ListStatResult) -> Self {
                Self {
                    subfolder_addresses: value
                        .subfolder_addresses
                        .into_iter()
                        .map(|uri| FolderAddress { uri })
                        .collect(),
                    entries: value.entries.into_iter().map(Into::into).collect(),
                }
            }
        }

        #[derive(Serialize, Deserialize, Debug)]
        pub struct ListItem {
            pub resource_address: String,
            pub resource_identity: String,
            pub metadata: Metadata,
        }

        impl From<ListItem> for v1beta::ListItem {
            fn from(value: ListItem) -> Self {
                Self {
                    resource_address: value.resource_address,
                    resource_info: Some(fileobject_v1beta::ResourceInfo {
                        metadata: Some(value.metadata.into()),
                        resource_identity: Some(fileobject_v1beta::ResourceIdentity {
                            encoded_identity: value.resource_identity,
                        }),
                    }),
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

        pub fn list(&self, request: ListRequest) -> BoxStream<'static, Result<ListResponse, Status>> {
            self.list_with_page_size(request, None)
        }

        pub fn list_with_page_size(
            &self,
            request: ListRequest,
            max_page_size: Option<u32>,
        ) -> BoxStream<'static, Result<ListResponse, Status>> {
            let client = self.clone();
            Box::pin(try_stream! {
                let mut continuation_handle: Option<String> = None;
                loop {
                    let (response, next_continuation_handle) = client.list_once(&request, continuation_handle.as_deref(), max_page_size).await?;
                    yield response;

                    if next_continuation_handle.is_some() {
                        continuation_handle = next_continuation_handle;
                    } else {
                        break;
                    }
                }
            })
        }

        pub fn list_stat(&self, request: ListStatRequest) -> BoxStream<'static, Result<ListStatResponse, Status>> {
            self.list_stat_with_page_size(request, None)
        }

        pub fn list_stat_with_page_size(
            &self,
            request: ListStatRequest,
            max_page_size: Option<u32>,
        ) -> BoxStream<'static, Result<ListStatResponse, Status>> {
            let client = self.clone();
            Box::pin(try_stream! {
                let mut continuation_handle: Option<String> = None;
                loop {
                    let (response, next_continuation_handle) = client.list_stat_once(&request, continuation_handle.as_deref(), max_page_size).await?;
                    yield response;

                    if next_continuation_handle.is_some() {
                        continuation_handle = next_continuation_handle;
                    } else {
                        break;
                    }
                }
            })
        }

        pub async fn delete_folder(&self, request: DeleteFolderRequest) -> Result<DeleteFolderResponse, Status> {
            let response = self
                .client
                .delete(format!(
                    "{}/v1alpha/filefolder/list/{}",
                    self.base_url,
                    encode_path_segment(&request.folder.expect("foled address is missing").uri)
                ))
                .headers(get_headers())
                .send_with_retry()
                .await
                .expect("delete request failed");

            if response.status() != StatusCode::NO_CONTENT {
                return Err(response.to_tonic_status());
            }

            Ok(Default::default())
        }

        pub async fn list_once(
            &self,
            request: &ListRequest,
            continuation_handle: Option<&str>,
            max_page_size: Option<u32>,
        ) -> Result<(ListResponse, Option<String>), Status> {
            let mut query_params = Vec::new();
            if let Some(continuation_handle) = continuation_handle {
                query_params.push(format!("continuation_handle={}", continuation_handle));
            }
            if let Some(max_page_size) = max_page_size {
                query_params.push(format!("max_page_size={}", max_page_size));
            }

            let response = self
                .client
                .get(format!(
                    "{}/v1beta/filefolder/list/{}{}",
                    self.base_url,
                    encode_path_segment(request.folder.as_ref().expect("folder address is missing").uri.as_str()),
                    encode_query_params(&query_params),
                ))
                .headers(get_headers())
                .send_with_retry()
                .await
                .expect("list request failed");

            if response.status() != StatusCode::OK {
                return Err(response.to_tonic_status());
            }

            let mut response: models::ListResult = response.json().await.expect("response body read failed");
            let continuation_handle = std::mem::take(&mut response.next_continuation_handle);

            Ok((response.into(), continuation_handle))
        }

        pub async fn list_stat_once(
            &self,
            request: &ListStatRequest,
            continuation_handle: Option<&str>,
            max_page_size: Option<u32>,
        ) -> Result<(ListStatResponse, Option<String>), Status> {
            let mut query_params = Vec::new();
            if let Some(continuation_handle) = continuation_handle {
                query_params.push(format!("continuation_handle={}", continuation_handle));
            }
            if let Some(max_page_size) = max_page_size {
                query_params.push(format!("max_page_size={}", max_page_size));
            }

            let response = self
                .client
                .get(format!(
                    "{}/v1beta/filefolder/liststat/{}{}",
                    self.base_url,
                    encode_path_segment(request.folder.as_ref().expect("folder address is missing").uri.as_str()),
                    encode_query_params(&query_params),
                ))
                .headers(get_headers())
                .send_with_retry()
                .await
                .expect("list request failed");

            if response.status() != StatusCode::OK {
                return Err(response.to_tonic_status());
            }

            let mut response: models::ListStatResult = response.json().await.expect("response body read failed");
            let continuation_handle = std::mem::take(&mut response.next_continuation_handle);

            Ok((response.into(), continuation_handle))
        }
    }
}
