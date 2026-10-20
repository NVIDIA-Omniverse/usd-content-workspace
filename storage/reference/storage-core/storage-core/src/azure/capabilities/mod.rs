// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

pub mod v1alpha;
pub mod v1beta;

use futures::StreamExt;
use tracing::{info_span, instrument, Instrument};
use url::Url;

use crate::azure::error::{Error, SdkError};
use crate::azure::service::Storage;
use storage_core_builder::traits::error::Result;
use storage_core_util::additional_headers::{AdditionalHeaders, RequestType};

impl Storage {
    #[instrument(name = "azure::list_top_level_addresses_internal", skip_all)]
    pub async fn list_top_level_addresses_internal(
        &self,
        additional_headers: Option<AdditionalHeaders>,
    ) -> Result<Vec<String>> {
        // Gather any configured urls from the config
        let urls = self
            .secrets
            .read()
            .await
            .as_ref()
            .map(|object| object.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default();

        // Top Level Addresses should be able to be passed to list/list_stat, so for azure we need
        // to actually get the containers from each storage account.
        let mut addresses = Vec::new();
        for url in urls {
            let url = Url::parse(&url).map_err(Error::from)?;
            let client = self.client_from_url(&url).await?;
            let mut builder = client.list_containers();
            if let Some(additional_headers) = additional_headers.clone() {
                builder = builder.context(Self::create_custom_context(additional_headers, RequestType::Api)?);
            }

            let mut stream = builder.into_stream();

            while let Some(page) = stream.next().instrument(info_span!("List Containers")).await {
                let results = page.map_err(SdkError::from).map_err(Error::from)?;
                // Make a url for each container.
                results.containers.iter().for_each(|container| {
                    let mut url = url.clone();
                    url.set_path(&container.name);
                    addresses.push(url.to_string());
                });
            }
        }

        Ok(addresses)
    }
}
