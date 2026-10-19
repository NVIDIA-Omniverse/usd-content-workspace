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
        pub use storage_service_protos::nvidia::omniverse::storage::metadata::v1alpha::*;

        pub trait ResourceLocationExt {
            fn resource_address(uri: impl Into<String>) -> Self;

            fn resource_identity(uri: impl Into<String>) -> Self;

            fn folder_address(uri: impl Into<String>) -> Self;

            fn uri(&self) -> &str;
        }

        impl ResourceLocationExt for ResourceLocation {
            fn resource_address(uri: impl Into<String>) -> Self {
                Self {
                    resource: Some(resource_location::Resource::ResourceAddress(uri.into())),
                }
            }

            fn resource_identity(uri: impl Into<String>) -> Self {
                use storage_service_protos::nvidia::omniverse::storage::fileobject::v1alpha::ResourceIdentity;

                Self {
                    resource: Some(resource_location::Resource::ResourceIdentity(ResourceIdentity {
                        encoded_identity: uri.into(),
                    })),
                }
            }

            fn folder_address(uri: impl Into<String>) -> Self {
                use storage_service_protos::nvidia::omniverse::storage::filefolder::v1alpha::FolderAddress;

                Self {
                    resource: Some(resource_location::Resource::FolderAddress(FolderAddress {
                        uri: uri.into(),
                    })),
                }
            }

            #[track_caller]
            fn uri(&self) -> &str {
                use resource_location::Resource::*;

                match self.resource.as_ref().unwrap() {
                    ResourceAddress(value) => value,
                    ResourceIdentity(value) => &value.encoded_identity,
                    FolderAddress(value) => &value.uri,
                }
            }
        }

        pub trait ResponseEntryExt {
            type Success;
            type Error;

            fn as_success(&self) -> &Self::Success;

            fn as_error(&self) -> &Self::Error;
        }

        impl ResponseEntryExt for UpdateMetadataBatchResponseEntry {
            type Success = UpdateMetadataBatchResponseEntrySuccess;
            type Error = BatchErrorEntry;

            #[track_caller]
            fn as_success(&self) -> &Self::Success {
                if let Some(update_metadata_batch_response_entry::Outcome::Success(ref value)) = self.outcome {
                    value
                } else {
                    panic!("success outcome expected, got {:?}", self.outcome);
                }
            }

            #[track_caller]
            fn as_error(&self) -> &Self::Error {
                if let Some(update_metadata_batch_response_entry::Outcome::Error(ref value)) = self.outcome {
                    value
                } else {
                    panic!("error outcome expected, got {:?}", self.outcome);
                }
            }
        }
    }

    pub mod v1beta {
        pub use storage_service_protos::nvidia::omniverse::metadata::v1beta::*;

        pub trait ResourceLocationExt {
            fn resource_address(uri: impl Into<String>) -> Self;

            fn resource_identity(uri: impl Into<String>) -> Self;

            fn folder_address(uri: impl Into<String>) -> Self;

            fn uri(&self) -> &str;
        }

        impl ResourceLocationExt for ResourceLocation {
            fn resource_address(uri: impl Into<String>) -> Self {
                Self {
                    resource: Some(resource_location::Resource::ResourceAddress(uri.into())),
                }
            }

            fn resource_identity(uri: impl Into<String>) -> Self {
                Self {
                    resource: Some(resource_location::Resource::ResourceIdentity(ResourceIdentity {
                        encoded_identity: uri.into(),
                    })),
                }
            }

            fn folder_address(uri: impl Into<String>) -> Self {
                Self {
                    resource: Some(resource_location::Resource::FolderAddress(FolderAddress {
                        uri: uri.into(),
                    })),
                }
            }

            #[track_caller]
            fn uri(&self) -> &str {
                use resource_location::Resource::*;

                match self.resource.as_ref().unwrap() {
                    ResourceAddress(value) => value,
                    ResourceIdentity(value) => &value.encoded_identity,
                    FolderAddress(value) => &value.uri,
                }
            }
        }

        pub trait ResponseEntryExt {
            type Success;
            type Error;

            fn as_success(&self) -> &Self::Success;

            fn as_error(&self) -> &Self::Error;
        }

        impl ResponseEntryExt for UpdateMetadataBatchResponseEntry {
            type Success = UpdateMetadataBatchResponseEntrySuccess;
            type Error = BatchErrorEntry;

            #[track_caller]
            fn as_success(&self) -> &Self::Success {
                if let Some(update_metadata_batch_response_entry::Outcome::Success(ref value)) = self.outcome {
                    value
                } else {
                    panic!("success outcome expected, got {:?}", self.outcome);
                }
            }

            #[track_caller]
            fn as_error(&self) -> &Self::Error {
                if let Some(update_metadata_batch_response_entry::Outcome::Error(ref value)) = self.outcome {
                    value
                } else {
                    panic!("error outcome expected, got {:?}", self.outcome);
                }
            }
        }
    }
}

pub mod v1alpha {
    use std::collections::HashMap;

    use crate::common::{clients::metadata::types::v1alpha::*, random::get_random_alphanumeric_string};
    use futures::{stream::BoxStream, TryStreamExt};
    use storage_service_protos::google::protobuf::Value;
    use tonic::Status;

    #[derive(Debug, Clone)]
    pub enum Client {
        Grpc(super::grpc::v1alpha::Client),
        Rest(super::rest::v1alpha::Client),
    }

    impl Client {
        pub async fn get_metadata(&self, request: GetMetadataRequest) -> Result<GetMetadataResponse, Status> {
            match self {
                Self::Grpc(client) => client.get_metadata(request).await,
                Self::Rest(client) => client.get_metadata(request).await,
            }
        }

        pub async fn update_metadata(&self, request: UpdateMetadataRequest) -> Result<UpdateMetadataResponse, Status> {
            match self {
                Self::Grpc(client) => client.update_metadata(request).await,
                Self::Rest(client) => client.update_metadata(request).await,
            }
        }

        pub async fn delete_metadata(&self, request: DeleteMetadataRequest) -> Result<DeleteMetadataResponse, Status> {
            match self {
                Self::Grpc(client) => client.delete_metadata(request).await,
                Self::Rest(client) => client.delete_metadata(request).await,
            }
        }

        pub fn get_metadata_batch(
            &self,
            request: GetMetadataBatchRequest,
        ) -> BoxStream<'static, Result<GetMetadataBatchResponse, Status>> {
            match self {
                Self::Grpc(client) => client.get_metadata_batch(request),
                Self::Rest(client) => client.get_metadata_batch(request),
            }
        }

        pub fn query_metadata(
            &self,
            request: QueryMetadataRequest,
        ) -> BoxStream<'static, Result<QueryMetadataResponse, Status>> {
            match self {
                Self::Grpc(client) => client.query_metadata(request),
                Self::Rest(client) => client.query_metadata(request, None),
            }
        }

        pub fn query_metadata_with_page_size_hint(
            &self,
            request: QueryMetadataRequest,
            page_size: usize,
        ) -> BoxStream<'static, Result<QueryMetadataResponse, Status>> {
            match self {
                Self::Grpc(client) => client.query_metadata(request),
                Self::Rest(client) => client.query_metadata(request, Some(page_size)),
            }
        }

        pub fn update_metadata_batch(
            &self,
            request: UpdateMetadataBatchRequest,
        ) -> BoxStream<'static, Result<UpdateMetadataBatchResponse, Status>> {
            match self {
                Self::Grpc(client) => client.update_metadata_batch(request),
                Self::Rest(client) => client.update_metadata_batch(request),
            }
        }

        pub fn delete_metadata_batch(
            &self,
            request: DeleteMetadataBatchRequest,
        ) -> BoxStream<'static, Result<DeleteMetadataBatchResponse, Status>> {
            match self {
                Self::Grpc(client) => client.delete_metadata_batch(request),
                Self::Rest(client) => client.delete_metadata_batch(request),
            }
        }

        pub async fn set_checkpoint_metadata(
            &self,
            address: impl Into<String>,
            checkpoints: impl IntoIterator<Item = (u64, impl Into<String>)>,
        ) -> Result<(), Status> {
            use storage_service_protos::google::protobuf::{value, ListValue, Struct, Value};

            self.update_metadata(UpdateMetadataRequest {
                uri: address.into(),
                user_metadata_key: "checkpoints".into(),
                user_metadata: Some(Value {
                    kind: Some(value::Kind::ListValue(ListValue {
                        values: checkpoints
                            .into_iter()
                            .map(|(id, resource_identity)| Value {
                                kind: Some(value::Kind::StructValue(Struct {
                                    fields: HashMap::from([
                                        (
                                            "id".to_string(),
                                            Value {
                                                kind: Some(value::Kind::NumberValue(id as f64)),
                                            },
                                        ),
                                        (
                                            "resource_identity".to_string(),
                                            Value {
                                                kind: Some(value::Kind::StringValue(resource_identity.into())),
                                            },
                                        ),
                                    ]),
                                })),
                            })
                            .collect(),
                    })),
                }),
                ..Default::default()
            })
            .await?;

            Ok(())
        }

        pub async fn set_metadata_value(
            &self,
            location: impl Into<ResourceLocation>,
            key: impl Into<String>,
            value: Value,
        ) -> Result<String, tonic::Status> {
            use batch_error_entry::Code;
            use update_metadata_batch_response_entry::Outcome;

            let response = self
                .update_metadata_batch_sorted(UpdateMetadataBatchRequest {
                    requests: vec![UpdateMetadataBatchRequestEntry {
                        resource_location: Some(location.into()),
                        user_metadata_key: key.into(),
                        user_metadata: Some(value),
                        expected_etag: None,
                        namespace: None,
                    }],
                })
                .await?;

            match response
                .results
                .into_iter()
                .next()
                .expect("unexpected end of the response")
                .outcome
                .expect("outcome is missing from the response")
            {
                Outcome::Success(value) => Ok(value.etag),
                Outcome::Error(value) => Err(match value.code {
                    c if c == Code::InvalidArgument as i32 => Status::invalid_argument(value.message),
                    c if c == Code::NotFound as i32 => Status::not_found(value.message),
                    c if c == Code::PermissionDenied as i32 => Status::permission_denied(value.message),
                    c if c == Code::FailedPrecondition as i32 => Status::failed_precondition(value.message),
                    c if c == Code::Internal as i32 => Status::internal(value.message),
                    c if c == Code::ResourceExhausted as i32 => Status::resource_exhausted(value.message),
                    c if c == Code::Unavailable as i32 => Status::unavailable(value.message),
                    c => panic!("unexpected error code {c}"),
                }),
            }
        }

        pub async fn set_random_metadata_value(
            &self,
            location: impl Into<ResourceLocation>,
            key: impl Into<String>,
        ) -> Result<(Value, String), tonic::Status> {
            use storage_service_protos::google::protobuf::value::Kind;

            let value = Value {
                kind: Some(Kind::StringValue(get_random_alphanumeric_string(32))),
            };
            let etag = self.set_metadata_value(location, key, value.clone()).await?;

            Ok((value, etag))
        }

        pub async fn get_metadata_batch_sorted(
            &self,
            request: GetMetadataBatchRequest,
        ) -> Result<GetMetadataBatchResponse, Status> {
            let requests_len = request.requests.len();

            let mut stream = self.get_metadata_batch(request);
            let mut results = Vec::new();
            while let Some(mut page) = stream.try_next().await? {
                results.append(&mut page.results);
            }

            results.sort_unstable_by_key(|e| e.request_index);

            assert_eq!(results.len(), requests_len);
            assert!(results.iter().enumerate().all(|(idx, e)| e.request_index == idx as u32));

            Ok(GetMetadataBatchResponse { results })
        }

        pub async fn query_metadata_sorted(
            &self,
            request: QueryMetadataRequest,
        ) -> Result<QueryMetadataResponse, Status> {
            let mut stream = self.query_metadata(request);
            let mut results = Vec::new();
            while let Some(mut page) = stream.try_next().await? {
                results.append(&mut page.results);
            }

            results.sort_unstable_by(|a, b| a.user_metadata_key.cmp(&b.user_metadata_key));

            Ok(QueryMetadataResponse { results })
        }

        pub async fn query_metadata_with_page_size_hint_sorted(
            &self,
            request: QueryMetadataRequest,
            max_page_size: usize,
        ) -> Result<QueryMetadataResponse, Status> {
            let mut stream = self.query_metadata_with_page_size_hint(request, max_page_size);
            let mut results = Vec::new();
            while let Some(mut page) = stream.try_next().await? {
                results.append(&mut page.results);
            }

            results.sort_unstable_by(|a, b| a.user_metadata_key.cmp(&b.user_metadata_key));

            Ok(QueryMetadataResponse { results })
        }

        pub async fn update_metadata_batch_sorted(
            &self,
            request: UpdateMetadataBatchRequest,
        ) -> Result<UpdateMetadataBatchResponse, Status> {
            let requests_len = request.requests.len();

            let mut stream = self.update_metadata_batch(request);
            let mut results = Vec::new();
            while let Some(mut page) = stream.try_next().await? {
                results.append(&mut page.results);
            }

            results.sort_unstable_by_key(|e| e.request_index);

            assert_eq!(results.len(), requests_len);
            assert!(results.iter().enumerate().all(|(idx, e)| e.request_index == idx as u32));

            Ok(UpdateMetadataBatchResponse { results })
        }

        pub async fn delete_metadata_batch_sorted(
            &self,
            request: DeleteMetadataBatchRequest,
        ) -> Result<DeleteMetadataBatchResponse, Status> {
            let requests_len = request.requests.len();

            let mut stream = self.delete_metadata_batch(request);
            let mut results = Vec::new();
            while let Some(mut page) = stream.try_next().await? {
                results.append(&mut page.results);
            }

            results.sort_unstable_by_key(|e| e.request_index);

            assert_eq!(results.len(), requests_len);
            assert!(results.iter().enumerate().all(|(idx, e)| e.request_index == idx as u32));

            Ok(DeleteMetadataBatchResponse { results })
        }
    }
}

pub mod v1beta {
    use crate::common::{clients::metadata::types::v1beta::*, random::get_random_alphanumeric_string};
    use futures::{stream::BoxStream, TryStreamExt};
    use storage_service_protos::google::protobuf::Value;
    use tonic::Status;

    #[derive(Debug, Clone)]
    pub enum Client {
        Grpc(super::grpc::v1beta::Client),
        Rest(super::rest::v1beta::Client),
    }

    impl Client {
        pub fn get_metadata_batch(
            &self,
            request: GetMetadataBatchRequest,
        ) -> BoxStream<'static, Result<GetMetadataBatchResponse, Status>> {
            match self {
                Self::Grpc(client) => client.get_metadata_batch(request),
                Self::Rest(client) => client.get_metadata_batch(request),
            }
        }

        pub fn query_metadata(
            &self,
            request: QueryMetadataRequest,
        ) -> BoxStream<'static, Result<QueryMetadataResponse, Status>> {
            match self {
                Self::Grpc(client) => client.query_metadata(request),
                Self::Rest(client) => client.query_metadata(request, None),
            }
        }

        pub fn query_metadata_with_page_size_hint(
            &self,
            request: QueryMetadataRequest,
            page_size: usize,
        ) -> BoxStream<'static, Result<QueryMetadataResponse, Status>> {
            match self {
                Self::Grpc(client) => client.query_metadata(request),
                Self::Rest(client) => client.query_metadata(request, Some(page_size)),
            }
        }

        pub fn update_metadata_batch(
            &self,
            request: UpdateMetadataBatchRequest,
        ) -> BoxStream<'static, Result<UpdateMetadataBatchResponse, Status>> {
            match self {
                Self::Grpc(client) => client.update_metadata_batch(request),
                Self::Rest(client) => client.update_metadata_batch(request),
            }
        }

        pub fn delete_metadata_batch(
            &self,
            request: DeleteMetadataBatchRequest,
        ) -> BoxStream<'static, Result<DeleteMetadataBatchResponse, Status>> {
            match self {
                Self::Grpc(client) => client.delete_metadata_batch(request),
                Self::Rest(client) => client.delete_metadata_batch(request),
            }
        }

        pub async fn set_metadata_value(
            &self,
            location: impl Into<ResourceLocation>,
            key: impl Into<String>,
            value: Value,
        ) -> Result<String, Status> {
            use batch_error_entry::Code;
            use update_metadata_batch_response_entry::Outcome;

            let response = self
                .update_metadata_batch_sorted(UpdateMetadataBatchRequest {
                    requests: vec![UpdateMetadataBatchRequestEntry {
                        resource_location: Some(location.into()),
                        user_metadata_key: key.into(),
                        user_metadata: Some(value),
                        expected_etag: None,
                        namespace: None,
                    }],
                })
                .await?;

            match response
                .results
                .into_iter()
                .next()
                .expect("unexpected end of the response")
                .outcome
                .expect("outcome is missing from the response")
            {
                Outcome::Success(value) => Ok(value.etag),
                Outcome::Error(value) => Err(match value.code {
                    c if c == Code::InvalidArgument as i32 => Status::invalid_argument(value.message),
                    c if c == Code::NotFound as i32 => Status::not_found(value.message),
                    c if c == Code::PermissionDenied as i32 => Status::permission_denied(value.message),
                    c if c == Code::FailedPrecondition as i32 => Status::failed_precondition(value.message),
                    c if c == Code::Internal as i32 => Status::internal(value.message),
                    c if c == Code::ResourceExhausted as i32 => Status::resource_exhausted(value.message),
                    c if c == Code::Unavailable as i32 => Status::unavailable(value.message),
                    c => panic!("unexpected error code {c}"),
                }),
            }
        }

        pub async fn set_random_metadata_value(
            &self,
            location: impl Into<ResourceLocation>,
            key: impl Into<String>,
        ) -> Result<(Value, String), Status> {
            use storage_service_protos::google::protobuf::value::Kind;

            let value = Value {
                kind: Some(Kind::StringValue(get_random_alphanumeric_string(32))),
            };
            let etag = self.set_metadata_value(location, key, value.clone()).await?;

            Ok((value, etag))
        }

        pub async fn get_metadata_batch_sorted(
            &self,
            request: GetMetadataBatchRequest,
        ) -> Result<GetMetadataBatchResponse, Status> {
            let requests_len = request.requests.len();

            let mut stream = self.get_metadata_batch(request);
            let mut results = Vec::new();
            while let Some(mut page) = stream.try_next().await? {
                results.append(&mut page.results);
            }

            results.sort_unstable_by_key(|e| e.request_index);

            assert_eq!(results.len(), requests_len);
            assert!(results.iter().enumerate().all(|(idx, e)| e.request_index == idx as u32));

            Ok(GetMetadataBatchResponse { results })
        }

        pub async fn query_metadata_sorted(
            &self,
            request: QueryMetadataRequest,
        ) -> Result<QueryMetadataResponse, Status> {
            let mut stream = self.query_metadata(request);
            let mut results = Vec::new();
            while let Some(mut page) = stream.try_next().await? {
                results.append(&mut page.results);
            }

            results.sort_unstable_by(|a, b| a.user_metadata_key.cmp(&b.user_metadata_key));

            Ok(QueryMetadataResponse { results })
        }

        pub async fn update_metadata_batch_sorted(
            &self,
            request: UpdateMetadataBatchRequest,
        ) -> Result<UpdateMetadataBatchResponse, Status> {
            let requests_len = request.requests.len();

            let mut stream = self.update_metadata_batch(request);
            let mut results = Vec::new();
            while let Some(mut page) = stream.try_next().await? {
                results.append(&mut page.results);
            }

            results.sort_unstable_by_key(|e| e.request_index);

            assert_eq!(results.len(), requests_len);
            assert!(results.iter().enumerate().all(|(idx, e)| e.request_index == idx as u32));

            Ok(UpdateMetadataBatchResponse { results })
        }

        pub async fn delete_metadata_batch_sorted(
            &self,
            request: DeleteMetadataBatchRequest,
        ) -> Result<DeleteMetadataBatchResponse, Status> {
            let requests_len = request.requests.len();

            let mut stream = self.delete_metadata_batch(request);
            let mut results = Vec::new();
            while let Some(mut page) = stream.try_next().await? {
                results.append(&mut page.results);
            }

            results.sort_unstable_by_key(|e| e.request_index);

            assert_eq!(results.len(), requests_len);
            assert!(results.iter().enumerate().all(|(idx, e)| e.request_index == idx as u32));

            Ok(DeleteMetadataBatchResponse { results })
        }
    }
}
