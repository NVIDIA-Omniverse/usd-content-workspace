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
        versioning::v1beta::{
            EnumerateVersionsRequest, EnumerateVersionsResponse, EnumerateVersionsResponseWithContinueToken,
            VersionInfo, Versioning as VersioningTrait, VersionsOrder,
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
            move |blobs, _, next_marker, _| {
                Ok(EnumerateVersionsResponseWithContinueToken {
                    response: EnumerateVersionsResponse {
                        items: blobs
                            // Filter out objects that don't have the exact object key
                            .filter(|b| b.name == resource_key)
                            .map(|b| VersionInfo {
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
}
