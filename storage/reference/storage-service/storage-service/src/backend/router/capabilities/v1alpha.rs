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

use crate::backend::router::{Dummy, Match, Service, ServiceRoute};
use futures::try_join;
use storage_core::traits::{capabilities::v1alpha::*, Request, Result};
use tonic::async_trait;

#[async_trait]
impl<A: Capabilities, SA: Match, B: Capabilities> Capabilities for Service<ServiceRoute<A, SA>, B> {
    fn inner(&self) -> Option<&dyn Capabilities> {
        None
    }

    async fn list_top_level_addresses(
        &self,
        _request: Request<ListTopLevelAddressesRequest>,
    ) -> Result<ListTopLevelAddressesResponse> {
        let head_response_future = self
            .head
            .service
            .list_top_level_addresses(Request::new(Default::default()));
        let tail_response_future = self.tail.list_top_level_addresses(Request::new(Default::default()));
        let (mut head_response, tail_response) = try_join!(head_response_future, tail_response_future)?;

        // The selector advertises the endpoints; the backend only expands them into listable
        // roots (e.g. Azure containers), so keep the expansion of the advertised endpoints.
        head_response
            .items
            .retain(|item| self.head.selector.filter_top_level_address(&item.top_level_address));

        if tail_response.items.is_empty() {
            return Ok(head_response);
        }

        Ok(ListTopLevelAddressesResponse {
            items: head_response.items.into_iter().chain(tail_response.items).collect(),
        })
    }

    async fn list_routes(&self, _request: Request<ListRoutesRequest>) -> Result<ListRoutesResponse> {
        use route::RouteType;

        let tail_response = self.tail.list_routes(Request::new(Default::default())).await?;

        Ok(ListRoutesResponse {
            items: self
                .head
                .selector
                .iter_wildcard_routes()
                .map(|pattern| Route {
                    route_type: Some(RouteType::WildcardPattern(pattern.to_owned())),
                })
                .chain(tail_response.items)
                .collect(),
        })
    }
}

#[async_trait]
impl Capabilities for Dummy {
    fn inner(&self) -> Option<&dyn Capabilities> {
        None
    }

    async fn list_top_level_addresses(
        &self,
        _request: Request<ListTopLevelAddressesRequest>,
    ) -> Result<ListTopLevelAddressesResponse> {
        Ok(Default::default())
    }

    async fn list_routes(&self, _request: Request<ListRoutesRequest>) -> Result<ListRoutesResponse> {
        Ok(Default::default())
    }
}
