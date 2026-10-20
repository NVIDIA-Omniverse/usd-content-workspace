// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use tracing::instrument;
use url::Url;

use crate::{
    azure::{
        error::Error,
        resource::{Address, ContainerAddress, WrappedResourceInfo},
        service::Storage,
    },
    traits::{
        versioning::v1alpha::{
            EnumerateVersionsRequest, EnumerateVersionsResponse, EnumerateVersionsResponseWithContinueToken,
            IsVersionedRequest, IsVersionedResponse, VersionInfo, Versioning as VersioningTrait, VersionsOrder,
        },
        BoxStream, Request, Result, StreamingOptions,
    },
};

use storage_core_util::additional_headers::AdditionalHeaders;

#[tonic::async_trait]
impl VersioningTrait for Storage {
    fn inner(&self) -> Option<&dyn VersioningTrait> {
        None
    }

    #[instrument(name = "azure::enumerate_versions", skip_all)]
    async fn enumerate_versions(
        &self,
        mut request: Request<EnumerateVersionsRequest>,
        streaming: StreamingOptions,
        _version_id_token: Option<String>,
    ) -> Result<BoxStream<EnumerateVersionsResponseWithContinueToken>> {
        let additional_headers = request.extensions_mut().remove::<AdditionalHeaders>();
        let url = Url::parse(&request.into_inner().resource_address).map_err(Error::from)?;
        let container_address = ContainerAddress::try_from_url(&url)?;
        let resource_address = Address::parse_prefix(url)?;

        // Short-circuit when the operator configured versioning off for this
        // endpoint (per-endpoint `VERSIONING_ENABLED` secrets entry, falling
        // back to the backend-wide `azure.versioning_enabled`) — structurally
        // before any client/credential resolution, so a credential problem can
        // never preempt the deterministic FAILED_PRECONDITION. There is no
        // runtime detection: unconfigured means the listing goes to the backend.
        if !self.versioning_enabled_for(&resource_address).await {
            let account = resource_address.account().ok().unwrap_or("unknown");
            let container = resource_address.container().ok().unwrap_or("unknown");
            return Err(Error::precondition_failed(format!(
                "versioning is not enabled on account {account} container {container}"
            ))
            .into());
        }

        let resource_key = resource_address.blob().to_string();

        self.list_internal(
            None,
            resource_address,
            true,
            streaming,
            additional_headers,
            move |blobs, _, next_marker, resource_address| {
                Ok(EnumerateVersionsResponseWithContinueToken {
                    response: EnumerateVersionsResponse {
                        items: blobs
                            // Filter out objects that don't have the exact object key
                            .filter(|b| b.name == resource_key)
                            .map(|b| VersionInfo {
                                resource_address: b
                                    .version_id
                                    .as_deref()
                                    .map(|v| resource_address.create_versioned_resource_address(v)),
                                resource_info: Some(
                                    WrappedResourceInfo::from((
                                        container_address.get_blob_url(&b.name),
                                        b.clone(),
                                        b.version_id.clone(),
                                    ))
                                    .0
                                    .into(),
                                ),
                                sorting_key: None,
                            })
                            .collect(),
                        versions_order: VersionsOrder::OldestFirst.into(),
                    },
                    continue_token: next_marker.map(|m| m.as_str().to_string()),
                    version_id_token: None,
                })
            },
        )
        .await
    }

    #[instrument(name = "azure::is_versioned", skip_all)]
    async fn is_versioned(&self, request: Request<IsVersionedRequest>) -> Result<IsVersionedResponse> {
        // Parse so a malformed address errors rather than silently reporting a
        // versioning state for a URL the backend could never serve. The answer is
        // the operator-specified configuration; there is no backend call. Like
        // IsReadOnly, an endpoint strict mode refuses to serve is rejected.
        let url = Url::parse(&request.get_ref().resource_address).map_err(Error::from)?;
        let resource_address = Address::parse_prefix(url)?;
        Ok(IsVersionedResponse {
            is_versioned: self.is_versioned_for(&resource_address).await?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::azure::service::Config;

    /// `azure.versioning_enabled = false` short-circuits EnumerateVersions with
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
                    resource_address: "https://account.blob.core.windows.net/container/key.txt".to_string(),
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
                    resource_address: "https://account.blob.core.windows.net/container/key.txt".to_string(),
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
