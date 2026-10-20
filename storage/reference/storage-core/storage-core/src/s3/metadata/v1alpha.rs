// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use storage_core_util::debug_limited;
use tracing::instrument;

use crate::{
    s3::error::Error,
    traits::{
        async_trait,
        metadata::v1alpha::{
            update_metadata_batch_fanout, DeleteMetadataRequest, DeleteMetadataResponse, GetMetadataRequest,
            GetMetadataResponse, Metadata, UpdateMetadataBatchRequest, UpdateMetadataBatchResponse,
            UpdateMetadataRequest, UpdateMetadataResponse, UserMetadataValue,
        },
        Request, Result,
    },
};
use storage_core_util::additional_headers::AdditionalHeaders;

use super::super::service::Storage;

#[async_trait]
impl Metadata for Storage {
    fn inner(&self) -> Option<&dyn Metadata> {
        None
    }

    #[instrument(name = "s3::get_metadata", skip_all)]
    async fn get_metadata(&self, mut request: Request<GetMetadataRequest>) -> Result<GetMetadataResponse> {
        debug_limited!(?request);
        let additional_headers = request.extensions_mut().remove::<AdditionalHeaders>();
        let request = request.into_inner();

        let metadata = self
            .get_metadata_sdk(&request.uri, request.user_metadata_keys, additional_headers)
            .await?;

        let user_metadata = metadata
            .into_iter()
            .map(|(key, value)| {
                (
                    key,
                    UserMetadataValue {
                        value: Some(value),
                        etag: String::new(),
                    },
                )
            })
            .collect();

        Ok(GetMetadataResponse { user_metadata })
    }

    #[instrument(name = "s3::update_metadata", skip_all)]
    async fn update_metadata(&self, mut request: Request<UpdateMetadataRequest>) -> Result<UpdateMetadataResponse> {
        debug_limited!(?request);
        let additional_headers = request.extensions_mut().remove::<AdditionalHeaders>();
        let request = request.into_inner();

        if request.expected_etag.is_some() {
            return Err(Error::invalid_argument("expected_etag is not supported for metadata operations").into());
        }

        if request.user_metadata_key.is_empty() {
            return Err(Error::invalid_argument("user_metadata_key is required").into());
        }

        let user_metadata = request
            .user_metadata
            .ok_or_else(|| Error::invalid_argument("User metadata is required"))?;

        self.update_metadata_sdk(
            &request.uri,
            request.user_metadata_key,
            user_metadata,
            additional_headers,
        )
        .await?;

        Ok(UpdateMetadataResponse { etag: String::new() })
    }

    #[instrument(name = "s3::update_metadata_batch", skip_all)]
    async fn update_metadata_batch(
        &self,
        request: Request<UpdateMetadataBatchRequest>,
    ) -> Result<UpdateMetadataBatchResponse> {
        debug_limited!(?request);
        update_metadata_batch_fanout(self, request).await
    }

    #[instrument(name = "s3::delete_metadata", skip_all)]
    async fn delete_metadata(&self, mut request: Request<DeleteMetadataRequest>) -> Result<DeleteMetadataResponse> {
        debug_limited!(?request);
        let additional_headers = request.extensions_mut().remove::<AdditionalHeaders>();
        let request = request.into_inner();

        if request.expected_etag.is_some() {
            return Err(Error::invalid_argument("expected_etag is not supported for metadata operations").into());
        }

        if request.user_metadata_key.is_empty() {
            return Err(Error::invalid_argument("user_metadata_key is required").into());
        }

        self.delete_metadata_sdk(&request.uri, request.user_metadata_key, additional_headers)
            .await?;

        Ok(DeleteMetadataResponse {})
    }
}
