// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;
use tracing::instrument;

use crate::traits::{
    error::Result,
    fileobject::v1alpha::FileObject,
    versioning::v1alpha::{
        EnumerateVersionsRequest, EnumerateVersionsResponse, EnumerateVersionsResponseWithContinueToken,
        IsVersionedRequest, IsVersionedResponse, Versioning, VersionsOrder,
    },
    BoxStream, Request, StreamingOptions,
};

use super::super::{AddressClass, ManifestService, ManifestState};
use crate::traits::versioning::v1alpha::VersionInfo;

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
                self.read_manifest_v1alpha(key, request.metadata().clone(), request.extensions().clone())
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

    /// Contract: `resource_address` is answered here at *object* granularity.
    /// The manifest layer only adds versioning capability for a workspace *object
    /// path* that the loaded manifest actually contains; for a genuine top-level
    /// (bucket/account root) address `classify_address` yields `NotWorkspace` and
    /// we defer to the physical backend. Callers asking "does this bucket support
    /// versioning?" from `ListTopLevelAddresses` entries therefore get the
    /// backend's answer, not a manifest override — do not expect a bucket-root
    /// address to be reported versioned just because a manifest exists beneath it.
    #[instrument(name = "manifest::is_versioned", skip_all)]
    async fn is_versioned(&self, request: Request<IsVersionedRequest>) -> Result<IsVersionedResponse> {
        // A manifest-backed workspace entry synthesizes a version from the loaded
        // manifest regardless of the physical backend's versioning state, so
        // `EnumerateVersions` succeeds even when the backend has versioning off.
        // Report `true` for such addresses; otherwise the manifest layer adds no
        // versioning capability and we defer to the physical backend.
        let resource_address = request.get_ref().resource_address.clone();
        let key = match self.classify_address(&resource_address)? {
            AddressClass::NotWorkspace | AddressClass::ManifestFile { .. } => {
                return self.inner.is_versioned(request).await
            },
            AddressClass::WorkspacePath { key } => key,
        };

        // The cold-cache manifest read carries the caller's metadata and extensions
        // (including the `AdditionalHeaders` injected above this layer), so it goes
        // through a permission-enforcing resolver chain as the caller.
        let metadata = request.metadata().clone();
        let extensions = request.extensions().clone();
        let state = self
            .get_manifest_state(&key, |key| {
                self.read_manifest_v1alpha(key, metadata.clone(), extensions.clone())
            })
            .await?;

        let manifest = match state {
            ManifestState::Loaded(manifest) => manifest,
            // No manifest at this workspace path: the manifest layer synthesizes
            // nothing, so the answer is the physical backend's.
            ManifestState::Absent => return self.inner.is_versioned(request).await,
            ManifestState::Invalid => return Err(ManifestService::<S>::invalid_manifest_error().into()),
        };

        // A manifest entry for this object path means the manifest layer can
        // answer `EnumerateVersions` itself: report versioning as enabled.
        // Otherwise defer to the physical backend.
        match key.object_path.as_ref() {
            Some(object_path) if manifest.contains_key(object_path) => Ok(IsVersionedResponse { is_versioned: true }),
            _ => self.inner.is_versioned(request).await,
        }
    }
}
