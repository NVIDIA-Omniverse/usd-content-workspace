// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use storage_core_util::{debug_limited, trace_limited};
use tracing::instrument;

use crate::traits::{
    async_trait,
    versioning::v1alpha::{
        EnumerateVersionsRequest, EnumerateVersionsResponse, EnumerateVersionsResponseWithContinueToken,
        IsVersionedRequest, IsVersionedResponse, VersionInfo, Versioning as VersioningTrait, VersionsOrder,
    },
    BoxStream, Request, Result, StreamingOptions,
};

use storage_core_util::additional_headers::AdditionalHeaders;

use super::super::{
    fileobject::{ListObjectVersionsPageResponse, ListVersionsMarkerMode},
    resource::{Address, Identity, WrappedResourceInfo},
    service::Storage,
};

use super::super::error::Error;

#[async_trait]
impl VersioningTrait for Storage {
    fn inner(&self) -> Option<&dyn VersioningTrait> {
        None
    }

    #[instrument(name = "s3::enumerate_versions", skip_all)]
    async fn enumerate_versions(
        &self,
        mut request: Request<EnumerateVersionsRequest>,
        streaming: StreamingOptions,
        version_id_token: Option<String>,
    ) -> Result<BoxStream<EnumerateVersionsResponseWithContinueToken>> {
        let additional_headers = request.extensions_mut().remove::<AdditionalHeaders>();
        debug_limited!(?request, ?additional_headers, ?streaming);

        let request = request.into_inner();

        let mut resource_address = Address::parse(&request.resource_address)?;

        // Short-circuit when the operator configured versioning off for this
        // endpoint (per-endpoint `VERSIONING_ENABLED` secrets entry, falling
        // back to the backend-wide `s3.versioning_enabled`) — structurally
        // before any client/credential resolution, so a credential problem can
        // never preempt the deterministic FAILED_PRECONDITION. There is no
        // runtime detection: unconfigured means the listing goes to the backend.
        if !self.versioning_enabled_for(&resource_address).await {
            let bucket = resource_address.host().unwrap_or("unknown");
            return Err(Error::precondition_failed(format!("versioning is not enabled on bucket {bucket}")).into());
        }

        let client = self.client(&mut resource_address).await?;

        let resource_key = resource_address.key()?.to_string();

        self.list_versions_sdk(
            &client,
            None,
            resource_address,
            streaming,
            ListVersionsMarkerMode::EnumerateSplit { version_id_token },
            false,
            additional_headers,
            move |output, resource_address, next_markers| {
                let items: Vec<_> = output
                    .versions()
                    .iter()
                    .filter(|object| object.key().map(|k| k == resource_key).unwrap_or(false))
                    .map(|object| {
                        let identity: Identity = (
                            resource_address.clone(),
                            object.e_tag().map(String::from),
                            object.version_id().map(String::from),
                        )
                            .into();
                        let data_object_size = object.size().map(|s| s as u64);
                        let last_modified_timestamp =
                            object
                                .last_modified()
                                .map(|ts| storage_core_protos::google::protobuf::Timestamp {
                                    seconds: ts.secs(),
                                    nanos: ts.subsec_nanos() as i32,
                                });
                        trace_limited!(?object);
                        VersionInfo {
                            resource_address: object
                                .version_id()
                                .map(|v| resource_address.create_versioned_resource_address(v)),
                            resource_info: Some(
                                WrappedResourceInfo::from((&identity, data_object_size, last_modified_timestamp))
                                    .0
                                    .into(),
                            ),
                            sorting_key: None,
                        }
                    })
                    .collect();
                let is_empty = items.is_empty();

                Ok(ListObjectVersionsPageResponse {
                    response: EnumerateVersionsResponseWithContinueToken {
                        response: EnumerateVersionsResponse {
                            items,
                            versions_order: VersionsOrder::NewestFirst.into(),
                        },
                        continue_token: next_markers.key_marker,
                        version_id_token: next_markers.version_id_marker,
                    },
                    is_empty,
                })
            },
        )
        .await
    }

    #[instrument(name = "s3::is_versioned", skip_all)]
    async fn is_versioned(&self, request: Request<IsVersionedRequest>) -> Result<IsVersionedResponse> {
        // Parse so a malformed address errors rather than silently reporting a
        // versioning state for a URL the backend could never serve. The answer is
        // the operator-specified configuration; there is no backend call. Like
        // IsReadOnly, an endpoint strict mode refuses to serve is rejected.
        let resource_address = Address::parse(&request.get_ref().resource_address)?;
        Ok(IsVersionedResponse {
            is_versioned: self.is_versioned_for(&resource_address).await?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::s3::service::Config;

    /// `s3.versioning_enabled = false` short-circuits EnumerateVersions with
    /// FAILED_PRECONDITION before any backend call.
    #[tokio::test]
    async fn enumerate_versions_short_circuits_when_versioning_configured_off() {
        let storage = Storage::new(
            Config {
                versioning_enabled: Some(false),
                ..Config::default()
            },
            None,
        )
        .expect("storage construction should succeed");

        let result = storage
            .enumerate_versions(
                Request::new(EnumerateVersionsRequest {
                    resource_address: "https://bucket.s3.us-west-2.amazonaws.com/key.txt".to_string(),
                }),
                StreamingOptions::Streamed,
                None,
            )
            .await;
        let Err(err) = result else {
            panic!("versioning configured off must short-circuit");
        };
        // The client-visible classification is the contract: gRPC surfaces this
        // as Code::FailedPrecondition (HTTP 412 via the REST mapping).
        assert_eq!(err.tonic_status().code(), tonic::Code::FailedPrecondition);
        assert!(
            err.to_string().contains("versioning is not enabled"),
            "unexpected error: {err}"
        );
    }

    /// IsVersioned rejects an endpoint strict mode refuses to serve, but
    /// EnumerateVersions keeps its own path: no secrets entry and no backend-wide
    /// setting resolve to "enabled", so the request goes on to client resolution,
    /// which rejects the endpoint with NOT_FOUND before any backend call.
    #[tokio::test]
    async fn enumerate_versions_in_strict_mode_rejects_unconfigured_endpoint_at_client_resolution() {
        let storage = Storage::new(
            Config {
                allow_unconfigured_endpoints: false,
                ..Config::default()
            },
            None,
        )
        .expect("storage construction should succeed");

        let result = storage
            .enumerate_versions(
                Request::new(EnumerateVersionsRequest {
                    resource_address: "https://bucket.s3.us-west-2.amazonaws.com/key.txt".to_string(),
                }),
                StreamingOptions::Streamed,
                None,
            )
            .await;
        let Err(err) = result else {
            panic!("strict mode must reject an unconfigured endpoint");
        };
        assert_eq!(err.tonic_status().code(), tonic::Code::NotFound);
        assert!(
            err.to_string().contains("endpoint not configured"),
            "unexpected error: {err}"
        );
    }
}
