// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;

use crate::traits::{
    error::Result,
    versioning::v1beta::{EnumerateVersionsRequest, EnumerateVersionsResponseWithContinueToken, Versioning},
    BoxStream, Request, StreamingOptions,
};

use super::super::AdditionalHeaderManager;

#[async_trait]
impl<S> Versioning for AdditionalHeaderManager<S>
where
    S: Versioning + Send + Sync,
{
    fn inner(&self) -> Option<&dyn Versioning> {
        Some(&self.inner)
    }

    async fn enumerate_versions(
        &self,
        mut request: Request<EnumerateVersionsRequest>,
        streaming: StreamingOptions,
        version_id_token: Option<String>,
    ) -> Result<BoxStream<EnumerateVersionsResponseWithContinueToken>> {
        let additional_headers = self.parse(&request)?;
        request.extensions_mut().insert(additional_headers);

        self.inner
            .enumerate_versions(request, streaming, version_id_token)
            .await
    }
}
