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

//! The discovery plane: what this service tells the discovery operator about
//! itself.
//!
//! The operator lists Kubernetes Services annotated
//! `discovery.omniverse.nvidia.com/enabled=true`, dials the port named `grpc`
//! with a plaintext credential-less channel, and calls `ListServices`. Whatever
//! comes back is what callers can find; a service that does not answer is a
//! service Navigator cannot route to.

use crate::http::rest_api_version;
use discovery_proto::nvidia::omniverse::discovery::capabilities::v2alpha::capabilities_service_server::CapabilitiesService;
use discovery_proto::nvidia::omniverse::discovery::capabilities::v2alpha::{
    GrpcEndpoint, GrpcService, ListServicesRequest, ListServicesResponse, RestEndpoint, RestService,
    RestVersion, RestVersionDetail, ServiceEntry,
};
use thumbnail_proto::nvidia::omniverse::thumbnail::v1alpha::thumbnail_service_server::SERVICE_NAME as THUMBNAIL_GRPC_SERVICE_NAME;
use tonic::{Request, Response, Status};

/// The discovery "type" key for this service, matching the id the
/// discovery-service registry uses for thumbnails.
const SERVICE_TYPE: &str = "thumbnail";
const REST_API_VERSION: &str = rest_api_version!();

pub struct CapabilitiesServiceImpl;

#[tonic::async_trait]
impl CapabilitiesService for CapabilitiesServiceImpl {
    /// Describe this service to the discovery operator.
    ///
    /// The payload is a fixed single entry, so `page_size` and `page_token` are
    /// no-ops and `next_page_token` is always empty. Only the thumbnail domain
    /// surface is advertised — health, reflection, and this service itself are
    /// infrastructure the operator has no reason to route to.
    async fn list_services(
        &self,
        _request: Request<ListServicesRequest>,
    ) -> Result<Response<ListServicesResponse>, Status> {
        let service = ServiceEntry {
            service_name: SERVICE_TYPE.to_owned(),
            grpc_services: vec![GrpcService {
                endpoints: vec![GrpcEndpoint {
                    service_name: THUMBNAIL_GRPC_SERVICE_NAME.to_owned(),
                }],
            }],
            rest_services: vec![RestService {
                endpoints: vec![RestEndpoint {
                    // The v2 operator reads this name and only falls back to
                    // `service_name` when it is empty, so it is the field a v2
                    // client actually sees.
                    name: SERVICE_TYPE.to_owned(),
                    versions: vec![RestVersion {
                        version: REST_API_VERSION.to_owned(),
                        url_detail: Some(RestVersionDetail {
                            path: format!("/{REST_API_VERSION}/"),
                        }),
                    }],
                }],
            }],
        };

        Ok(Response::new(ListServicesResponse {
            services: vec![service],
            next_page_token: String::new(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The operator dials with a bare plaintext channel carrying no metadata, so
    /// a request with no authorization must reach the handler and be answered.
    /// This also guards the intent should a global auth layer ever be added: it
    /// would have to exempt this service.
    #[tokio::test]
    async fn list_services_advertises_the_thumbnail_apis_unauthenticated() {
        let response = CapabilitiesServiceImpl
            .list_services(Request::new(ListServicesRequest::default()))
            .await
            .expect("list_services should succeed without authorization metadata")
            .into_inner();

        assert_eq!(response.services.len(), 1);
        assert!(response.next_page_token.is_empty());

        let entry = &response.services[0];
        assert_eq!(entry.service_name, "thumbnail");

        assert_eq!(entry.grpc_services.len(), 1);
        assert_eq!(entry.grpc_services[0].endpoints.len(), 1);
        assert_eq!(
            entry.grpc_services[0].endpoints[0].service_name,
            "nvidia.omniverse.thumbnail.v1alpha.ThumbnailService"
        );

        assert_eq!(entry.rest_services.len(), 1);
        assert_eq!(entry.rest_services[0].endpoints.len(), 1);
        let rest_endpoint = &entry.rest_services[0].endpoints[0];
        assert_eq!(rest_endpoint.name, "thumbnail");
        assert_eq!(rest_endpoint.versions.len(), 1);
        assert_eq!(rest_endpoint.versions[0].version, "v1alpha");
        assert_eq!(
            rest_endpoint.versions[0]
                .url_detail
                .as_ref()
                .expect("url_detail should be set")
                .path,
            "/v1alpha/"
        );
    }

    /// The response is a fixed payload, so paging must not change it. An operator
    /// that passes a page size should not be handed a truncated or empty list.
    #[tokio::test]
    async fn list_services_ignores_pagination() {
        let paged = CapabilitiesServiceImpl
            .list_services(Request::new(ListServicesRequest {
                page_size: 1,
                page_token: "some-token".to_owned(),
            }))
            .await
            .expect("list_services should succeed")
            .into_inner();

        let default = CapabilitiesServiceImpl
            .list_services(Request::new(ListServicesRequest::default()))
            .await
            .expect("list_services should succeed")
            .into_inner();

        assert_eq!(paged, default);
        assert!(paged.next_page_token.is_empty());
    }

    /// The advertised gRPC name has to be the generated constant rather than a
    /// string typed here, or a proto package rename would leave the operator
    /// routing to a service name that no longer exists.
    #[test]
    fn the_advertised_grpc_name_comes_from_the_generated_constant() {
        assert_eq!(
            THUMBNAIL_GRPC_SERVICE_NAME,
            "nvidia.omniverse.thumbnail.v1alpha.ThumbnailService"
        );
    }
}
