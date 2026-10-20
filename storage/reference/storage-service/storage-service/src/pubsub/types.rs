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

use futures::future::BoxFuture;
use std::{error::Error, fmt::Debug, sync::Arc};
use storage_core::layer::Layer;
use time::OffsetDateTime;
use tonic::async_trait;

/// A union of storage event types.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum StorageEvent {
    ObjectCreated(ObjectCreatedEventData),
    ObjectDeleted(ObjectDeletedEventData),
    DirCreated(DirEventData),
    DirDeleted(DirEventData),
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum StorageEventSource {
    AzureBlob,
    S3,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ObjectCreatedEventData {
    pub source: StorageEventSource,
    pub resource_address: String,
    pub resource_identity: String,
    pub sequencer: String,
    pub time: OffsetDateTime,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ObjectDeletedEventData {
    pub source: StorageEventSource,
    pub resource_address: String,
    pub sequencer: String,
    pub time: OffsetDateTime,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DirEventData {
    pub resource_address: String,
    pub time: OffsetDateTime,
}

/// A logical batch of storage events passed through the consumer pipeline.
pub type ConsumeRequest = Vec<StorageEvent>;

/// A shareable error for one storage event.
pub type ConsumerError = Arc<dyn Error + Send + Sync>;

/// The outcome for one storage event.
pub type ConsumeResult = Result<(), ConsumerError>;

/// Ordered outcomes for a logical batch of storage events.
///
/// Consumers must return exactly one result for every request entry, in the
/// same order as the input events.
pub type ConsumeResults = Vec<ConsumeResult>;

/// A consumer of logical batches of storage events.
#[async_trait]
pub trait Consumer: Send + Sync + 'static {
    async fn consume(&self, events: ConsumeRequest) -> ConsumeResults;
}

/// A type-erased consumer.
#[derive(Clone)]
pub struct AnyConsumer {
    inner: Arc<dyn Consumer>,
}

impl AnyConsumer {
    pub fn noop() -> Self {
        Self::from_consumer(NoOpConsumer)
    }

    pub fn from_consumer(consumer: impl Consumer) -> Self {
        Self {
            inner: Arc::new(consumer),
        }
    }
}

impl AnyConsumer {
    pub fn decorate<L>(self, layer: L) -> Self
    where
        L: Layer<Self>,
        L::Service: Consumer,
    {
        Self {
            inner: Arc::new(layer.layer(self)),
        }
    }

    pub fn maybe_decorate<L>(self, layer: Option<L>) -> Self
    where
        L: Layer<Self>,
        L::Service: Consumer,
    {
        if let Some(layer) = layer {
            self.decorate(layer)
        } else {
            self
        }
    }
}

impl Consumer for AnyConsumer {
    fn consume<'a: 'b, 'b>(&'a self, events: ConsumeRequest) -> BoxFuture<'b, ConsumeResults> {
        self.inner.consume(events)
    }
}

pub struct NoOpConsumer;

impl Consumer for NoOpConsumer {
    fn consume<'a: 'b, 'b>(&'a self, events: ConsumeRequest) -> BoxFuture<'b, ConsumeResults> {
        let results = events.into_iter().map(|_| Ok(())).collect();
        Box::pin(async move { results })
    }
}
