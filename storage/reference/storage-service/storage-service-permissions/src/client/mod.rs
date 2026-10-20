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

use std::num::NonZero;

use serde::Deserialize;
use tonic::metadata::MetadataMap;

use crate::{
    client::error::{Error, InitError},
    types::{Condition, DecisionResponse, ResourceAction},
};

pub mod error;
pub mod grpc;
pub mod rest;

mod utils;

/// Client configuration.
#[derive(Default, Debug, Deserialize, Clone)]
pub struct Config {
    pub rest: Option<rest::Config>,
    pub grpc: Option<grpc::Config>,
}

/// Permission service client.
#[derive(Debug)]
pub struct Client {
    inner: ClientInner,
}

impl Client {
    /// Create a permission client from the configuration.
    pub async fn try_from_config(config: Config) -> Result<Self, InitError> {
        let inner = match (config.rest, config.grpc) {
            (Some(config), None) => {
                let client = rest::Client::from_config(config);
                ClientInner::Rest(client)
            },
            (None, Some(config)) => {
                let client = grpc::Client::try_from_config(config).await?;
                ClientInner::Grpc(client)
            },
            (None, None) => return Err(InitError::endpoint_config_missing()),
            (Some(_), Some(_)) => return Err(InitError::multiple_endpoints_configured()),
        };

        Ok(Self { inner })
    }

    /// Query permission for individual resources in a batch. The results will be returned in the order, in which
    /// they are passed in the function. No resource deduplication will take place. The `headers` are attached to
    /// every request sent to the permission service.
    pub async fn query(
        &self,
        request_metadata: &MetadataMap,
        resources: impl IntoIterator<Item = ResourceAction>,
        condition: Condition,
        batch_size: NonZero<usize>,
    ) -> Result<Vec<DecisionResponse>, Error> {
        match &self.inner {
            ClientInner::Grpc(client) => client.query(request_metadata, resources, condition, batch_size).await,
            ClientInner::Rest(client) => {
                client
                    .query(request_metadata.as_ref(), resources, condition, batch_size)
                    .await
            },
        }
    }
}

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
enum ClientInner {
    Rest(rest::Client),
    Grpc(grpc::Client),
}
