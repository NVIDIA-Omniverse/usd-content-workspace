// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

pub mod v1alpha;
pub mod v1beta;

use std::fmt::Debug;

use azure_core::prelude::{IfMatchCondition, NextMarker};
use azure_storage_blobs::blob::Blob;
use azure_storage_blobs::container::operations::BlobPrefix;
use bytes::Bytes;
use futures::StreamExt;
use storage_core_util::debug_limited;
use tonic::Code;
use tracing::{info_span, instrument, Instrument};
use url::Url;

use crate::{
    azure::{
        error::{Error, SdkError},
        resource::{create_folder_placeholder, Address, FOLDER_PLACEHOLDER},
        service::Storage,
    },
    traits::{Result, StreamingOptions},
};

use storage_core_util::additional_headers::{AdditionalHeaders, RequestType};

impl Storage {
    #[instrument(name = "azure::list_internal", skip_all)]
    pub async fn list_internal<F, T>(
        &self,
        delimiter: Option<&str>,
        resource_address: Address,
        include_versions: bool,
        streaming: StreamingOptions,
        additional_headers: Option<AdditionalHeaders>,
        response_function: F,
    ) -> Result<crate::traits::BoxStream<T>>
    where
        F: Fn(
                Box<dyn Iterator<Item = &Blob> + '_>,
                Box<dyn Iterator<Item = &BlobPrefix> + '_>,
                Option<NextMarker>,
                &Address,
            ) -> Result<T>
            + std::marker::Send
            + 'static,
        T: std::marker::Send + 'static,
    {
        if resource_address.is_versioned() {
            return Err(Error::invalid_argument(format!(
                "Versioned resource addresses are not supported for listing: {}",
                resource_address
            ))
            .into());
        }

        let prefix = resource_address.prefix();
        debug_limited!(?prefix);

        // If the request is for the top level of a bucket, we should return an empty response
        // instead of a NOT_FOUND error
        let is_top_level = prefix.is_empty();

        // Fetch blob metadata inline when the operator enabled native metadata (no extra round-trip on Azure).
        // WrappedResourceInfo then maps it into native_metadata on each listed item; when disabled we skip it so
        // pre-existing metadata that was never exposed through this API stays hidden. v1beta list_stat drops the
        // map on conversion, so the only cost there is a slightly larger list response.
        let include_metadata = self.config.native_metadata_enabled.unwrap_or(false);
        let mut blob_list_options = self
            .client(&resource_address)
            .await?
            .container_client(resource_address.container()?)
            .list_blobs()
            .prefix(prefix.into_owned())
            .include_metadata(include_metadata)
            .include_versions(include_versions);

        if let Some(additional_headers) = additional_headers {
            blob_list_options =
                blob_list_options.context(Self::create_custom_context(additional_headers, RequestType::Api)?);
        }

        if let Some(delim) = delimiter {
            blob_list_options = blob_list_options.delimiter(delim.to_owned());
        }

        let mut has_continue_token = false;
        let mut no_stream = false;
        if let StreamingOptions::Paginated(pagination) = streaming {
            no_stream = true;
            if let Some(max_keys) = pagination.max_page_size {
                if let Some(max_keys) = std::num::NonZero::new(max_keys) {
                    blob_list_options = blob_list_options.max_results(max_keys);
                }
            }
            if let Some(continue_token) = pagination.continuation_handle {
                has_continue_token = true;
                blob_list_options = blob_list_options.marker(continue_token);
            }
        }
        let stream = blob_list_options.into_stream();
        let rate_limit_counter = self.rate_limit_counters.list_blobs.clone();

        Ok(Box::pin(async_stream::stream! {
            let mut stream = stream;
            while let Some(page) = stream.next().instrument(info_span!("List Blob Page")).await {
                let results = match page
                    .inspect_err(|e| rate_limit_counter.check_error(e))
                    .map_err(SdkError::from)
                {
                    Ok(r) => r,
                    Err(e) => {
                        yield Err(Error::from(e).into());
                        return;
                    },
                };

                // need to track if next marker is present when streaming
                // to determine if we should return a NOT_FOUND error as azure can
                // return a empty response as the last page and we do not want to mistake
                // that last empty page for totally empty list operation
                if !has_continue_token && results.next_marker.is_some() {
                    has_continue_token = true;
                }

                // If no results are returned, return a NOT_FOUND error instead of an empty response
                // blobs.items contains both blobs and prefixes
                //
                // Azure Blob Store can return a continue token even if there are no results, and if you issue a
                // request with that token, you will get an empty response. In that scenario, we want to actually
                // return the empty response back to the client, not NOT_FOUND. So we check for the continue_token
                // in the request to determine if we should return NOT_FOUND.
                if !is_top_level && results.blobs.items.is_empty() && !has_continue_token {
                    yield Err(SdkError::not_found("No elements found for folder address"))
                        .map_err(|e| Error::from(e).into());
                    return;
                }

                // Filter out folder placeholders
                let response = response_function(
                    Box::new(results.blobs.blobs().filter(|b| !b.name.ends_with(FOLDER_PLACEHOLDER))),
                    Box::new(results.blobs.prefixes()),
                    results.next_marker,
                    &resource_address,
                );
                yield response;

                // Break for REST requests which don't support streaming
                if no_stream {
                    break;
                }
            }
        }))
    }

    pub async fn create_folder_internal(
        &self,
        folder_address: impl AsRef<str> + Debug,
        additional_headers: Option<AdditionalHeaders>,
    ) -> Result<()> {
        // In azure, folders are created by creating a '.folder' file in the folder.

        // See if there's an object at the folder address. If there is, we can't create the folder.
        if self
            .check_for_object_at_folder_address(folder_address.as_ref(), additional_headers.clone())
            .await?
        {
            return Err(Error::from(SdkError::precondition_failed(format!(
                "Object already exists at folder address: {}",
                folder_address.as_ref()
            )))
            .into());
        }

        // Take the folder address and add '.folder' to the end.
        let placeholder_address = create_folder_placeholder(folder_address.as_ref());
        let resource_address = Address::parse_blob(Url::parse(&placeholder_address).map_err(Error::from)?)?;

        // Create the placeholder blob only if it doesn't exist.
        let mut builder = self
            .client(&resource_address)
            .await?
            .container_client(resource_address.container()?)
            .blob_client(resource_address.blob())
            .put_block_blob(Bytes::new())
            .if_match(IfMatchCondition::NotMatch("*".to_string()));

        if let Some(additional_headers) = additional_headers {
            builder = builder.context(Self::create_custom_context(additional_headers, RequestType::Data)?);
        }

        match builder
            .into_future()
            .instrument(info_span!("Create Folder Put Placeholder Blob"))
            .await
            .inspect_err(|e| self.rate_limit_counters.put_block_blob.check_error(e))
            .map_err(SdkError::from)
        {
            Ok(_) => Ok(()),
            // If the placeholder blob already exists, we can just return success.
            Err(e) => match e {
                SdkError::PreconditionFailed { .. } => Ok(()),
                _ => Err(Error::from(e).into()),
            },
        }
    }

    pub async fn delete_folder_internal(
        &self,
        folder_address: impl AsRef<str> + Debug,
        additional_headers: Option<AdditionalHeaders>,
    ) -> Result<()> {
        // In azure, folders are deleted by deleting the '.folder' file in the folder.

        // See if there's an object at the folder address. If there is, we can't delete the folder.
        if self
            .check_for_object_at_folder_address(folder_address.as_ref(), additional_headers.clone())
            .await?
        {
            return Err(Error::invalid_argument(format!(
                "Attempting to delete a non-folder object: {}",
                folder_address.as_ref()
            ))
            .into());
        }

        let placeholder_address = create_folder_placeholder(folder_address.as_ref());

        self.delete_internal(&placeholder_address, None::<&str>, additional_headers)
            .await
            .map_err(|e| e.into())
    }

    pub async fn check_for_object_at_folder_address(
        &self,
        folder_address: impl AsRef<str>,
        additional_headers: Option<AdditionalHeaders>,
    ) -> Result<bool> {
        // Remove the trailing slash from the folder address if it exists.
        let mut object_address = folder_address.as_ref().to_string();
        if object_address.ends_with('/') {
            object_address.pop();
        }
        match self.stat_internal(&object_address, additional_headers.clone()).await {
            Ok(_) => Ok(true),
            Err(e) => match e.tonic_status().code() {
                Code::NotFound => Ok(false),
                _ => Err(e),
            },
        }
    }
}
