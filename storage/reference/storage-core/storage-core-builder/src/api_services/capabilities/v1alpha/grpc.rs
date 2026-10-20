// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::atomic::Ordering;

use super::super::v1alpha::{list_service_response, Capabilities};
use crate::api_services::error::log_grpc_error;
use crate::traits::capabilities::v1alpha::Capabilities as CapabilitiesTrait;
use crate::traits::capabilities::v1alpha::{
    capabilities_service_server::{CapabilitiesService, CapabilitiesServiceServer},
    ListRoutesRequest, ListRoutesResponse, ListServicesRequest, ListServicesResponse, ListTopLevelAddressesRequest,
    ListTopLevelAddressesResponse, FILE_DESCRIPTOR_SET,
};
use storage_core_util::log_if_slow;
use tonic::{Request, Response, Status};
use tracing::instrument;

use super::super::super::REQUEST_ID;
use crate::{ingress::grpc::ToGrpcService, traits::async_trait};

impl<L> ToGrpcService for Capabilities<L>
where
    L: CapabilitiesTrait,
{
    type Service = CapabilitiesServiceServer<Capabilities<L>>;

    fn service(&self) -> Self::Service {
        CapabilitiesServiceServer::new(self.clone())
    }

    fn file_descriptor_set(&self) -> Option<&[u8]> {
        Some(FILE_DESCRIPTOR_SET)
    }
}

#[async_trait]
impl<T: CapabilitiesTrait> CapabilitiesService for Capabilities<T> {
    #[instrument(name = "grpc::list_services", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
    async fn list_services(
        &self,
        _request: Request<ListServicesRequest>,
    ) -> Result<Response<ListServicesResponse>, Status> {
        let _slow = log_if_slow!(guard: "grpc::list_services");
        Ok(Response::new(list_service_response()))
    }

    #[instrument(name = "grpc::list_top_level_addresses", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
    async fn list_top_level_addresses(
        &self,
        request: Request<ListTopLevelAddressesRequest>,
    ) -> Result<Response<ListTopLevelAddressesResponse>, Status> {
        let _slow = log_if_slow!(guard: "grpc::list_top_level_addresses");
        Ok(Response::new(
            self.capabilities
                .list_top_level_addresses(request.into())
                .await
                .map_err(|e| log_grpc_error(e.as_ref(), "list_top_level_addresses"))?,
        ))
    }

    #[instrument(name = "grpc::list_routes", skip_all, fields(request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed)))]
    async fn list_routes(&self, request: Request<ListRoutesRequest>) -> Result<Response<ListRoutesResponse>, Status> {
        let _slow = log_if_slow!(guard: "grpc::list_routes");
        Ok(Response::new(
            self.capabilities
                .list_routes(request.into())
                .await
                .map_err(|e| log_grpc_error(e.as_ref(), "list_routes"))?,
        ))
    }
}
