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

pub mod v2alpha {
    use axum::{
        extract::{Query, State},
        http::StatusCode,
        response::Response,
        routing::get,
        Router,
    };
    use serde::Deserialize;
    use storage_core::ingress::{OpenAPI, ToRestService};
    use storage_service_protos::nvidia::omniverse::discovery::capabilities::v2alpha::ListServicesResponse;
    use utoipa::{IntoParams, OpenApi};

    use crate::api::{
        discovery::v2alpha::{Capabilities, LIST_SERVICES_PATH},
        rest_utils::ResponseBuilder,
    };

    impl ToRestService for Capabilities {
        fn service(&self) -> Router {
            Router::new()
                .route(LIST_SERVICES_PATH, get(list_services))
                .with_state(self.clone())
        }

        fn openapi(&self) -> Option<OpenAPI> {
            Some(OpenAPI::new(
                "discovery/v2alpha",
                "/v2alpha/discovery/openapi.json",
                false,
                ApiDoc::openapi(),
            ))
        }
    }

    #[derive(Debug, Default, Deserialize, IntoParams)]
    #[serde(default)]
    struct ListServicesParams {
        /// Maximum number of services to return. The server may return fewer.
        page_size: i32,
        /// Page token from a prior call to retrieve the next page.
        page_token: String,
    }

    #[utoipa::path(
        get,
        path = "/v2alpha/services",
        params(ListServicesParams),
        responses(
            (status = 200, description = "List services implemented by this server", body = ListServicesResponse),
            (status = 400, description = "The page token is invalid"),
        ),
        tag = "discovery",
    )]
    async fn list_services(
        Query(params): Query<ListServicesParams>,
        State(state): State<Capabilities>,
    ) -> Result<Response, StatusCode> {
        let response = state
            .list_services_page(params.page_size, &params.page_token)
            .map_err(|_| StatusCode::BAD_REQUEST)?;

        ResponseBuilder::new().status(StatusCode::OK).json(&response)
    }

    #[derive(OpenApi)]
    #[openapi(
        paths(list_services),
        components(schemas(ListServicesResponse)),
        tags((name = "discovery", description = "Discovery Capabilities API")),
    )]
    struct ApiDoc;
}
