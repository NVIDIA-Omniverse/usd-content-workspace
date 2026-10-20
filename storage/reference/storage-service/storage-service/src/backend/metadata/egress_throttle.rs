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

use async_stream::try_stream;
use storage_core::{
    egress_throttle::EgressThrottle,
    traits::{Request, Result},
};
use tokio_stream::StreamExt;
use tonic::async_trait;

use crate::backend::metadata::*;

#[async_trait]
impl<T: Metadata> Metadata for EgressThrottle<T> {
    fn inner(&self) -> Option<&dyn Metadata> {
        Some(&self.inner)
    }

    async fn get_metadata_batch(&self, request: Request<GetMetadataBatchRequest>) -> Result<GetMetadataBatchResponse> {
        let _permit = self.semaphore.acquire().await;
        self.inner.get_metadata_batch(request).await
    }

    async fn query_metadata(
        &self,
        request: Request<QueryMetadataRequest>,
        options: StreamingOptions,
    ) -> Result<BoxStream<QueryMetadataResponsePage>> {
        let mut stream = {
            let _permit = self.semaphore.acquire().await;
            self.inner.query_metadata(request, options).await?
        };

        let semaphore = self.semaphore.clone();
        Ok(Box::pin(try_stream! {
            loop {
                let permit = semaphore.acquire().await;
                if let Some(response) = stream.try_next().await? {
                    drop(permit);

                    yield response;
                } else {
                    return;
                }
            }
        }))
    }

    async fn update_metadata_batch(
        &self,
        request: Request<UpdateMetadataBatchRequest>,
    ) -> Result<UpdateMetadataBatchResponse> {
        let _permit = self.semaphore.acquire().await;
        self.inner.update_metadata_batch(request).await
    }

    async fn delete_metadata_batch(
        &self,
        request: Request<DeleteMetadataBatchRequest>,
    ) -> Result<DeleteMetadataBatchResponse> {
        let _permit = self.semaphore.acquire().await;
        self.inner.delete_metadata_batch(request).await
    }
}
