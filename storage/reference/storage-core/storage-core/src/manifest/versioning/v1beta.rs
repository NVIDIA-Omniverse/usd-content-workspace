// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;
use tracing::instrument;

use crate::traits::{
    error::Result,
    fileobject::v1beta::FileObject,
    versioning::v1beta::{
        EnumerateVersionsRequest, EnumerateVersionsResponse, EnumerateVersionsResponseWithContinueToken, Versioning,
        VersionsOrder,
    },
    BoxStream, Request, StreamingOptions,
};

use super::super::{AddressClass, ManifestService, ManifestState};
use crate::traits::versioning::v1beta::VersionInfo;

#[async_trait]
impl<S, R> Versioning for ManifestService<S, R>
where
    S: Versioning,
    R: FileObject,
{
    fn inner(&self) -> Option<&dyn Versioning> {
        Some(&self.inner)
    }

    #[instrument(name = "manifest::enumerate_versions", skip_all)]
    async fn enumerate_versions(
        &self,
        request: Request<EnumerateVersionsRequest>,
        streaming: StreamingOptions,
        version_id_token: Option<String>,
    ) -> Result<BoxStream<EnumerateVersionsResponseWithContinueToken>> {
        let resource_address = request.get_ref().resource_address.clone();
        let key = match self.classify_address(&resource_address)? {
            AddressClass::NotWorkspace | AddressClass::ManifestFile { .. } => {
                return self
                    .inner
                    .enumerate_versions(request, streaming, version_id_token)
                    .await
            },
            AddressClass::WorkspacePath { key } => key,
        };

        let state = self
            .get_manifest_state(&key, |key| {
                self.read_manifest_v1beta(key, request.metadata().clone(), request.extensions().clone())
            })
            .await?;

        let manifest = match state {
            ManifestState::Loaded(manifest) => manifest,
            ManifestState::Absent => {
                return self
                    .inner
                    .enumerate_versions(request, streaming, version_id_token)
                    .await
            },
            ManifestState::Invalid => return Err(ManifestService::<S>::invalid_manifest_error().into()),
        };

        // Find the resource identity for the object path.
        let Some(object_path) = key.object_path.as_ref() else {
            return self
                .inner
                .enumerate_versions(request, streaming, version_id_token)
                .await;
        };

        let Some(entry_value) = manifest.get(object_path) else {
            return self
                .inner
                .enumerate_versions(request, streaming, version_id_token)
                .await;
        };

        let items = vec![VersionInfo::try_from(entry_value)?];
        let versions_order = VersionsOrder::NewestFirst as i32;

        Ok(self.stream_enumerate_versions_response(
            items,
            versions_order,
            version_id_token,
            streaming,
            |items, versions_order, continue_token, version_id_token| EnumerateVersionsResponseWithContinueToken {
                response: EnumerateVersionsResponse { items, versions_order },
                continue_token,
                version_id_token,
            },
        ))
    }
}
