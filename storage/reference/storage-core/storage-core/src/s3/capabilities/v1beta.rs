// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use tracing::instrument;

use crate::{
    traits::capabilities::v1beta::{
        Capabilities, ListTopLevelAddressesRequest, ListTopLevelAddressesResponse, TopLevelAddressEntry,
    },
    traits::{async_trait, Request, Result},
};

use super::super::service::Storage;

#[async_trait]
impl Capabilities for Storage {
    fn inner(&self) -> Option<&dyn Capabilities> {
        None
    }

    #[instrument(name = "s3::list_top_level_addresses", skip_all)]
    async fn list_top_level_addresses(
        &self,
        _request: Request<ListTopLevelAddressesRequest>,
    ) -> Result<ListTopLevelAddressesResponse> {
        // Gather any configured hosts from the config and return them as top level addresses.
        Ok(ListTopLevelAddressesResponse {
            items: self
                .secrets
                .read()
                .await
                .as_ref()
                .map(|object| {
                    object
                        .keys()
                        .map(|key| TopLevelAddressEntry {
                            top_level_address: key.to_string(),
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
        })
    }
}
