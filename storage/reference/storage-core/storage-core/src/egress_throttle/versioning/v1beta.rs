// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;

use storage_core_builder::traits::{
    error::Result,
    versioning::v1beta::{EnumerateVersionsRequest, EnumerateVersionsResponseWithContinueToken, Versioning},
    BoxStream, Request, StreamingOptions,
};

use crate::egress_throttle::EgressThrottle;

#[async_trait]
impl<S> Versioning for EgressThrottle<S>
where
    S: Versioning,
{
    fn inner(&self) -> Option<&dyn Versioning> {
        Some(&self.inner)
    }

    async fn enumerate_versions(
        &self,
        request: Request<EnumerateVersionsRequest>,
        streaming: StreamingOptions,
        version_id_token: Option<String>,
    ) -> Result<BoxStream<EnumerateVersionsResponseWithContinueToken>> {
        let _permit = self.semaphore.acquire().await;
        self.inner
            .enumerate_versions(request, streaming, version_id_token)
            .await
    }
}
