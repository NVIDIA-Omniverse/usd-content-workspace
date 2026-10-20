// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use tracing::instrument;

use crate::{
    azure::service::Storage,
    traits::capabilities::v1beta::{
        Capabilities, ListTopLevelAddressesRequest, ListTopLevelAddressesResponse, TopLevelAddressEntry,
    },
    traits::{Request, Result},
};

use storage_core_util::additional_headers::AdditionalHeaders;

#[tonic::async_trait]
impl Capabilities for Storage {
    fn inner(&self) -> Option<&dyn Capabilities> {
        None
    }

    #[instrument(name = "azure::list_top_level_addresses", skip_all)]
    async fn list_top_level_addresses(
        &self,
        mut request: Request<ListTopLevelAddressesRequest>,
    ) -> Result<ListTopLevelAddressesResponse> {
        let additional_headers = request.extensions_mut().remove::<AdditionalHeaders>();

        let addresses = self.list_top_level_addresses_internal(additional_headers).await?;

        Ok(ListTopLevelAddressesResponse {
            items: addresses
                .into_iter()
                .map(|address| TopLevelAddressEntry {
                    top_level_address: address,
                })
                .collect(),
        })
    }
}
