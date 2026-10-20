// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

pub mod v1alpha;
pub mod v1beta;

use std::sync::Arc;

use aws_sdk_s3::primitives::ByteStream;
use snafu::prelude::*;
use tracing::{info_span, Instrument};

use crate::traits::{error::Result, Bytes};

use storage_core_util::additional_headers::{AdditionalHeaders, RequestType};
use tonic::Code;

use super::{
    error,
    resource::Address,
    service::{Client, Storage},
};

impl Storage {
    pub async fn create_folder_sdk(
        &self,
        mut folder_address: Address,
        additional_headers: Option<AdditionalHeaders>,
    ) -> Result<()> {
        // In S3, folders are created by writing a zero-byte object to the folder address.
        let client = self.client(&mut folder_address).await?;

        // See if there's an object at the folder address. If there is, we can't create the folder.
        // Note that someone could still create this object after we check.
        if self
            .check_for_object_at_folder_address(&client, &folder_address, additional_headers.clone())
            .await?
        {
            return Err(error::SdkError::precondition_failed(format!(
                "Object already exists at folder address: {}",
                folder_address.as_ref()
            )))
            .context(error::Sdk {})?;
        }

        let bucket = &client.bucket;
        let key = folder_address.key()?;

        // Only write the object if it doesn't exist.
        let mut builder = client
            .put_object()
            .bucket(bucket)
            .key(key)
            .body(ByteStream::from(Bytes::new()))
            .if_none_match("*")
            .customize();

        builder = Self::apply_request_context(
            builder,
            additional_headers,
            RequestType::Data,
            client.proxy_interceptor.clone(),
            None,
            client.rate_limit_interceptors.put_object.clone(),
        );

        // If the folder already exists, we can just return success.
        match builder
            .send()
            .instrument(info_span!("create_folder put_object"))
            .await
            .map_err(error::SdkError::from)
        {
            Ok(_) => Ok(()),
            Err(e) => match e {
                error::SdkError::PreconditionFailed { .. } => Ok(()),
                _ => Err(e).context(error::Sdk {})?,
            },
        }
    }

    pub async fn delete_folder_sdk(
        &self,
        mut folder_address: Address,
        additional_headers: Option<AdditionalHeaders>,
    ) -> Result<()> {
        let client = self.client(&mut folder_address).await?;

        if self
            .check_for_object_at_folder_address(&client, &folder_address, additional_headers.clone())
            .await?
        {
            return Err(error::SdkError::invalid_argument(format!(
                "Attempting to delete a non-folder object: {}",
                folder_address.url
            )))
            .context(error::Sdk {})?;
        }

        // In S3, folders are deleted by deleting the zero-byte object in the folder.
        Ok(self
            .delete_sdk(folder_address, None::<String>, additional_headers)
            .await?)
    }

    pub async fn check_for_object_at_folder_address(
        &self,
        client: &Arc<Client>,
        folder_address: &Address,
        additional_headers: Option<AdditionalHeaders>,
    ) -> Result<bool> {
        let mut object_address = folder_address.clone();
        let path = object_address.url.path().to_string();
        if let Some(path_without_slash) = path.strip_suffix('/') {
            object_address.url.set_path(path_without_slash);
        }
        match self.stat_sdk(client, object_address, additional_headers.clone()).await {
            Ok(_) => Ok(true),
            Err(e) => match e.tonic_status().code() {
                Code::NotFound => Ok(false),
                _ => Err(e),
            },
        }
    }
}
