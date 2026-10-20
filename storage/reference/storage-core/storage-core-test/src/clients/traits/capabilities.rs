// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use storage_core::traits::capabilities::v1alpha::{
    ListRoutesRequest, ListRoutesResponse, ListServicesRequest, ListServicesResponse, ListTopLevelAddressesRequest,
    ListTopLevelAddressesResponse,
};
use tonic::Status;

#[tonic::async_trait]
pub trait Capabilities
where
    Self: std::marker::Send + std::marker::Sync + 'static,
{
    async fn list_services(&mut self, _request: ListServicesRequest) -> Result<ListServicesResponse, Status> {
        unimplemented!()
    }

    async fn list_top_level_addresses(
        &mut self,
        _request: ListTopLevelAddressesRequest,
    ) -> Result<ListTopLevelAddressesResponse, Status> {
        unimplemented!()
    }

    async fn list_routes(&mut self, _request: ListRoutesRequest) -> Result<ListRoutesResponse, Status> {
        unimplemented!()
    }
}
