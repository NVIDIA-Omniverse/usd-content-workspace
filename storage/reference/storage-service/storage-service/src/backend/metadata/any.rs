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

use std::sync::Arc;

use futures::future::BoxFuture;
use storage_core::{
    layer::Layer,
    traits::{Request, Result},
};

use crate::backend::metadata::*;

pub trait Decorate {
    fn decorate(&self, inner: AnyMetadata) -> AnyMetadata;
}

impl<L: Layer<AnyMetadata>> Decorate for L
where
    L::Service: Metadata,
{
    fn decorate(&self, inner: AnyMetadata) -> AnyMetadata {
        AnyMetadata::from_metadata(self.layer(inner))
    }
}

impl<L: Layer<AnyMetadata>> Decorate for Option<L>
where
    L::Service: Metadata,
{
    fn decorate(&self, inner: AnyMetadata) -> AnyMetadata {
        if let Some(ref value) = self {
            value.decorate(inner)
        } else {
            inner
        }
    }
}

#[derive(Clone)]
pub struct AnyMetadata {
    pub(in crate::backend::metadata) inner: Arc<dyn Metadata>,
}

impl AnyMetadata {
    pub fn from_metadata(inner: impl Metadata) -> Self {
        Self { inner: Arc::new(inner) }
    }

    pub fn decorate(self, decorator: &impl Decorate) -> Self {
        Self {
            inner: decorator.decorate(self).inner,
        }
    }
}

impl Metadata for AnyMetadata {
    fn inner(&self) -> Option<&dyn Metadata> {
        None
    }

    fn get_metadata_batch<'a: 'b, 'b>(
        &'a self,
        request: Request<GetMetadataBatchRequest>,
    ) -> BoxFuture<'b, Result<GetMetadataBatchResponse>> {
        self.inner.get_metadata_batch(request)
    }

    fn query_metadata<'a: 'b, 'b>(
        &'a self,
        request: Request<QueryMetadataRequest>,
        options: StreamingOptions,
    ) -> BoxFuture<'b, Result<BoxStream<QueryMetadataResponsePage>>> {
        self.inner.query_metadata(request, options)
    }

    fn update_metadata_batch<'a: 'b, 'b>(
        &'a self,
        request: Request<UpdateMetadataBatchRequest>,
    ) -> BoxFuture<'b, Result<UpdateMetadataBatchResponse>> {
        self.inner.update_metadata_batch(request)
    }

    fn delete_metadata_batch<'a: 'b, 'b>(
        &'a self,
        request: Request<DeleteMetadataBatchRequest>,
    ) -> BoxFuture<'b, Result<DeleteMetadataBatchResponse>> {
        self.inner.delete_metadata_batch(request)
    }
}
