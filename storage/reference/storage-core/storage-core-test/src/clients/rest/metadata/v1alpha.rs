// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use storage_core::traits::metadata::v1alpha::{
    update_metadata_batch_entry_error, update_metadata_batch_entry_result, DeleteMetadataRequest,
    DeleteMetadataResponse, GetMetadataRequest, GetMetadataResponse, UpdateMetadataBatchEntryError,
    UpdateMetadataBatchEntryResult, UpdateMetadataBatchRequest, UpdateMetadataBatchResponse, UpdateMetadataRequest,
    UpdateMetadataResponse,
};
use storage_core_builder::api_services::metadata::v1alpha::rest::{
    UpdateMetadataBatchEntry, UpdateMetadataBatchEntryErrorCode, UpdateMetadataBatchRequestBody,
    UpdateMetadataBatchResponseBody,
};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::Status;
use urlencoding::encode;

use super::MetadataRestClient;
use crate::clients::traits::metadata::Metadata;
use crate::headers::HeadersConfig;

pub struct MetadataClient {
    client: MetadataRestClient,
}

impl MetadataClient {
    pub fn new(port: u16, headers_config: HeadersConfig) -> Self {
        Self {
            client: MetadataRestClient::new(port, headers_config),
        }
    }
}

#[tonic::async_trait]
impl Metadata for MetadataClient {
    async fn get_metadata(&mut self, request: GetMetadataRequest) -> Result<GetMetadataResponse, Status> {
        let path = format!("v1alpha/metadata/{}", encode(&request.uri));
        let body = self.client.get_metadata(&path, request.user_metadata_keys).await?;
        let response: GetMetadataResponse = serde_json::from_slice(&body).unwrap();
        Ok(response)
    }

    async fn update_metadata(&mut self, request: UpdateMetadataRequest) -> Result<UpdateMetadataResponse, Status> {
        let path = format!(
            "v1alpha/metadata/{}/{}",
            encode(&request.uri),
            encode(&request.user_metadata_key)
        );
        let value = request.user_metadata.unwrap_or_default();
        let etag = self
            .client
            .update_metadata(&path, &value, request.expected_etag.as_deref())
            .await?;
        Ok(UpdateMetadataResponse { etag })
    }

    async fn update_metadata_batch(
        &mut self,
        request: UpdateMetadataBatchRequest,
    ) -> Result<ReceiverStream<Result<UpdateMetadataBatchResponse, Status>>, Status> {
        let path = "v1alpha/metadata";
        let request_body = UpdateMetadataBatchRequestBody {
            requests: request
                .requests
                .into_iter()
                .map(|request| UpdateMetadataBatchEntry {
                    uri: request.uri,
                    key: request.user_metadata_key,
                    value: serde_json::to_value(request.user_metadata.unwrap_or_default()).unwrap(),
                    expected_etag: request.expected_etag,
                })
                .collect(),
        };

        let body = self.client.update_metadata_batch(path, &request_body).await?;
        let response_body: UpdateMetadataBatchResponseBody = serde_json::from_slice(&body).unwrap();
        let response = UpdateMetadataBatchResponse {
            results: response_body
                .responses
                .into_iter()
                .map(|response| {
                    let outcome = if let Some(success) = response.success {
                        update_metadata_batch_entry_result::Outcome::Success(UpdateMetadataResponse {
                            etag: success.etag,
                        })
                    } else if let Some(error) = response.error {
                        update_metadata_batch_entry_result::Outcome::Error(UpdateMetadataBatchEntryError {
                            code: rest_batch_error_code(error.code) as i32,
                            message: error.message,
                        })
                    } else {
                        update_metadata_batch_entry_result::Outcome::Error(UpdateMetadataBatchEntryError {
                            code: update_metadata_batch_entry_error::Code::Internal as i32,
                            message: "missing batch entry outcome".to_string(),
                        })
                    };

                    UpdateMetadataBatchEntryResult {
                        request_index: response.request_index,
                        outcome: Some(outcome),
                    }
                })
                .collect(),
        };

        let (tx, rx) = mpsc::channel(1);
        let _ = tx.send(Ok(response)).await;
        Ok(ReceiverStream::new(rx))
    }

    async fn delete_metadata(&mut self, request: DeleteMetadataRequest) -> Result<DeleteMetadataResponse, Status> {
        let path = format!(
            "v1alpha/metadata/{}/{}",
            encode(&request.uri),
            encode(&request.user_metadata_key)
        );
        self.client
            .delete_metadata(&path, request.expected_etag.as_deref())
            .await?;
        Ok(DeleteMetadataResponse {})
    }
}

fn rest_batch_error_code(code: UpdateMetadataBatchEntryErrorCode) -> update_metadata_batch_entry_error::Code {
    match code {
        UpdateMetadataBatchEntryErrorCode::InvalidArgument => update_metadata_batch_entry_error::Code::InvalidArgument,
        UpdateMetadataBatchEntryErrorCode::NotFound => update_metadata_batch_entry_error::Code::NotFound,
        UpdateMetadataBatchEntryErrorCode::PermissionDenied => {
            update_metadata_batch_entry_error::Code::PermissionDenied
        },
        UpdateMetadataBatchEntryErrorCode::FailedPrecondition => {
            update_metadata_batch_entry_error::Code::FailedPrecondition
        },
        UpdateMetadataBatchEntryErrorCode::Internal => update_metadata_batch_entry_error::Code::Internal,
        UpdateMetadataBatchEntryErrorCode::Unavailable => update_metadata_batch_entry_error::Code::Unavailable,
    }
}
