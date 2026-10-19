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

use storage_core::traits::{generate_storage_trait, BoxStream, Request, Result, StreamingOptions};
pub use storage_service_protos::nvidia::omniverse::storage::metadata::v1alpha::*;

pub mod any;
pub mod batch;
pub mod cache;
pub mod canonicalization;
pub mod egress_throttle;
pub mod error;
pub mod metrics;
pub mod postgres;
pub mod reject_duplicates;
pub mod router;
pub mod types;
pub mod utils;

#[cfg(feature = "dynamodb")]
pub mod dynamodb;

#[cfg(feature = "cosmosdb")]
pub mod azure_cosmosdb;

pub const DEFAULT_NAMESPACE_NAME: &str = "default";

pub struct QueryMetadataResponsePage {
    pub response: QueryMetadataResponse,
    pub next_continuation_handle: Option<String>,
}

generate_storage_trait! {
    Metadata {
        async fn get_metadata_batch(&self, request: Request<GetMetadataBatchRequest>) -> Result<GetMetadataBatchResponse>;
        async fn query_metadata(&self, request: Request<QueryMetadataRequest>, options: StreamingOptions) -> Result<BoxStream<QueryMetadataResponsePage>>;
        async fn update_metadata_batch(&self, request: Request<UpdateMetadataBatchRequest>) -> Result<UpdateMetadataBatchResponse>;
        async fn delete_metadata_batch(&self, request: Request<DeleteMetadataBatchRequest>) -> Result<DeleteMetadataBatchResponse>;
    }
}
