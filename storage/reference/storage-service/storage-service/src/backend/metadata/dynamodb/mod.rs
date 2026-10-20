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

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use async_stream::try_stream;
use aws_config::BehaviorVersion;
use aws_sdk_dynamodb::{
    config::Builder,
    operation::{batch_get_item::BatchGetItemOutput, query::QueryOutput},
    types::{AttributeValue, KeysAndAttributes},
    Client,
};
use aws_sdk_s3::config::ProvideCredentials;
use futures::{
    stream::{select_all, BoxStream},
    TryFutureExt, TryStreamExt,
};
use prost::Message;
use serde::Deserialize;
use snafu::IntoError;
use storage_service_metrics::{
    future::FutureExt as _,
    utils::{record_metric, record_metric_value},
};
use storage_service_protos::google::protobuf::Value;
use tokio::sync::Semaphore;
use tonic::async_trait;
use tracing::{debug, instrument, warn};

use crate::{
    backend::metadata::{
        dynamodb::{
            error::{
                error::{BatchGetInvalidRowSnafu, QueryInvalidRowSnafu, UserMetadataValueMissingSnafu},
                Error,
            },
            metrics::Metrics,
            utils::{
                create_random_etag, get_metadata_key_attribute, get_resource_uri_hash_attribute,
                get_row_projection_expression, IsThrottlingErrorExt as _, Row, COL_ETAG, COL_METADATA_KEY,
                COL_METADATA_VALUE, COL_RESOURCE_URI, COL_RESOURCE_URI_HASH, PAR_ETAG, PAR_EXPECTED_ETAG,
                PAR_METADATA_VALUE, PAR_RESOURCE_URI, PAR_RESOURCE_URI_HASH,
            },
        },
        types::{KeyValue, NamedKeyValue},
        utils::Digest,
        *,
    },
    common::{
        aws::ConfigLoaderExt,
        future::WithConcurrencyLimitExt,
        iterator::{ChunksExt, TryCollectExt},
    },
};

mod error;
mod metrics;
mod utils;

const BATCH_GET_ITEM_MAX_ITEMS: usize = 100;

/// DynamoDB metadata backend configuration.
#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct Config {
    pub table: String,
    pub region: Option<String>,
    pub endpoint_url: Option<String>,
    pub max_concurrency: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            table: Default::default(),
            region: None,
            endpoint_url: None,
            max_concurrency: 1000,
        }
    }
}

/// DynamoDB metadata backend.
pub struct DynamoDBMetadataBackend {
    client: Client,
    table: String,
    metrics: Metrics,
    request_semaphore: Semaphore,
}

impl DynamoDBMetadataBackend {
    /// Create a backend from configuration.
    pub async fn from_config(backend_id: &str, config: Config, credentials: Arc<dyn ProvideCredentials>) -> Self {
        let sdk_config = aws_config::defaults(BehaviorVersion::latest())
            .maybe_endpoint_url(config.endpoint_url)
            .maybe_region(config.region)
            .credentials_provider(credentials)
            .load()
            .await;
        let client_config = Builder::from(&sdk_config).build();
        let client = Client::from_conf(client_config);

        Self {
            client,
            table: config.table,
            metrics: Metrics::new(backend_id),
            request_semaphore: Semaphore::new(config.max_concurrency),
        }
    }

    /// Get items in a batch.
    #[instrument(skip_all)]
    async fn get_metadata_through_cache(
        &self,
        resource: String,
        keys: HashSet<String>,
    ) -> Result<HashMap<String, UserMetadataValue>, Error> {
        let key_values = if keys.is_empty() {
            self.query_all_items(&resource).await?
        } else {
            self.get_selected_items(resource, keys).await?
        };

        Ok(key_values.into_iter().map(|nkv| (nkv.key, nkv.value.into())).collect())
    }

    /// Fetch all values for the specified key through cache, with a result filter.
    #[instrument(skip_all)]
    async fn query_all_items(&self, resource_uri: &str) -> Result<Vec<NamedKeyValue>, Error> {
        let mut stream = self.query_all_items_streamed(resource_uri);
        let mut results = Vec::new();
        while let Some(rows) = stream.try_next().await? {
            results.extend(rows.into_iter().filter_map(|row| {
                if row.resource_uri != resource_uri {
                    warn!(?resource_uri, ?row.resource_uri, "resource uri collision detected");
                    None
                } else {
                    Some(NamedKeyValue {
                        value: KeyValue {
                            value: row.metadata_value,
                            etag: row.etag,
                        },
                        key: row.metadata_key,
                    })
                }
            }));
        }

        Ok(results)
    }

    /// Create a stream that queries all keys for a resource.
    fn query_all_items_streamed<'a>(&'a self, resource_uri: &'a str) -> BoxStream<'a, Result<Vec<Row>, Error>> {
        let resource_uri_hash = Digest::compute(resource_uri);
        Box::pin(try_stream! {
            let mut start_key = None;
            loop {
                let response = self.query_all_items_once(
                    resource_uri.into(),
                    resource_uri_hash,
                    start_key,
                ).await?;
                if let Some(items) = response.items {
                    yield items
                        .into_iter()
                        .map(Row::try_from_row)
                        .try_collect()
                        .map_err(|source| QueryInvalidRowSnafu.into_error(source))?;
                }

                let Some(last_evaluated_key) = response.last_evaluated_key else {
                    break;
                };

                start_key = Some(last_evaluated_key);
            }
        })
    }

    /// Query a page of key values for the specified resource.
    #[instrument(skip_all)]
    async fn query_all_items_once(
        &self,
        resource_uri: String,
        resource_uri_hash: Digest,
        start_key: Option<HashMap<String, AttributeValue>>,
    ) -> Result<QueryOutput, Error> {
        debug!(?resource_uri, ?resource_uri_hash, ?start_key, "querying items");

        self.client
            .query()
            .table_name(&self.table)
            .projection_expression(get_row_projection_expression())
            .key_condition_expression(format!("{} = {}", COL_RESOURCE_URI_HASH, PAR_RESOURCE_URI_HASH,))
            .filter_expression(format!("{} = {}", COL_RESOURCE_URI, PAR_RESOURCE_URI))
            .expression_attribute_values(
                PAR_RESOURCE_URI_HASH,
                AttributeValue::S(resource_uri_hash.to_digest_string()),
            )
            .expression_attribute_values(PAR_RESOURCE_URI, AttributeValue::S(resource_uri))
            .set_exclusive_start_key(start_key)
            .send()
            .with_latency_metric(&self.metrics.query_latency)
            .with_concurrency_limit(&self.request_semaphore)
            .inspect_ok(record_metric_value(&self.metrics.query_count, 200u16))
            .inspect_err(record_metric(&self.metrics.query_count))
            .await
            .map_err(|error| match error.as_service_error() {
                Some(e) if e.is_throttled() => Error::request_throttled(error),
                _ => Error::query_failed(error),
            })
            .inspect(|response| debug!(?response, "query finished"))
    }

    /// Get items throught cache, for a single resource and multiple keys.
    #[instrument(skip_all)]
    async fn get_selected_items(
        &self,
        resource_uri: String,
        keys: impl IntoIterator<Item = String>,
    ) -> Result<Vec<NamedKeyValue>, Error> {
        let resource_uri_hash = Digest::compute(&resource_uri);
        let mut stream = select_all(keys.into_iter().chunks::<BATCH_GET_ITEM_MAX_ITEMS>().map(|chunk| {
            self.get_items_batch_streamed(chunk.into_iter().map(|key| (resource_uri_hash, Some(key.clone()))))
        }));
        let mut results = Vec::new();
        while let Some(rows) = stream.try_next().await? {
            results.extend(rows.into_iter().filter_map(|row| {
                if row.resource_uri != resource_uri {
                    warn!(?row.resource_uri, ?resource_uri, ?resource_uri_hash, "resource uri collision detected");
                    None
                } else {
                    Some(NamedKeyValue {
                        value: KeyValue {
                            value: row.metadata_value,
                            etag: row.etag,
                        },
                        key: row.metadata_key,
                    })
                }
            }));
        }

        Ok(results)
    }

    /// Create a stream that fetches all specified entities from the configured table.
    fn get_items_batch_streamed<'a>(
        &'a self,
        resource_keys: impl IntoIterator<Item = (Digest, Option<String>)>,
    ) -> BoxStream<'a, Result<Vec<Row>, Error>> {
        let keys = resource_keys
            .into_iter()
            .map(|(resource_uri_hash, maybe_key)| {
                if let Some(key) = maybe_key {
                    HashMap::from([
                        get_resource_uri_hash_attribute(&resource_uri_hash),
                        get_metadata_key_attribute(key),
                    ])
                } else {
                    HashMap::from([get_resource_uri_hash_attribute(&resource_uri_hash)])
                }
            })
            .collect();

        Box::pin(try_stream! {
            let keys_and_attributes =  KeysAndAttributes::builder()
                .projection_expression(get_row_projection_expression())
                .set_keys(Some(keys))
                .build()
                .map_err(Error::batch_get_build_error)?;

            let mut request_items = HashMap::from([(self.table.clone(), keys_and_attributes)]);
            while !request_items.is_empty() {
                let response = self.get_items_batch_once(request_items).await?;
                if let Some(responses) = response.responses {
                    for (table, items) in responses.into_iter() {
                        if table != self.table {
                            warn!(?table, "unexpected table name returned in a response");
                            continue;
                        }

                        yield items
                            .into_iter()
                            .map(Row::try_from_row)
                            .try_collect()
                            .map_err(|source| BatchGetInvalidRowSnafu.into_error(source))?;
                    }
                }

                request_items = response.unprocessed_keys.unwrap_or_default();
            }
        })
    }

    /// Get items in a batch once.
    #[instrument(skip_all, fields(batch_size = %request_items.len()))]
    async fn get_items_batch_once(
        &self,
        request_items: HashMap<String, KeysAndAttributes>,
    ) -> Result<BatchGetItemOutput, Error> {
        debug!(?request_items, "reading items");

        let batch_size: usize = request_items.values().map(|items| items.keys.len()).sum();
        self.metrics.batch_get_item_request_items.record(batch_size as u64);

        self.client
            .batch_get_item()
            .set_request_items(Some(request_items))
            .send()
            .with_latency_metric(&self.metrics.batch_get_item_latency)
            .with_concurrency_limit(&self.request_semaphore)
            .inspect_ok(record_metric_value(&self.metrics.batch_get_item_count, 200u16))
            .inspect_err(record_metric(&self.metrics.batch_get_item_count))
            .await
            .map_err(|error| match error.as_service_error() {
                Some(e) if e.is_throttled() => Error::request_throttled(error),
                _ => Error::batch_get_failed(error),
            })
            .inspect(|response| debug!(?response, "items read"))
    }

    /// Update a row.
    #[instrument(skip_all)]
    async fn update_item(
        &self,
        resource_uri: String,
        metadata_key: String,
        metadata_value: Value,
        expected_etag: Option<String>,
    ) -> Result<String, Error> {
        let etag = create_random_etag();
        let resource_uri_hash = Digest::compute(&resource_uri);
        let encoded_metadata_value = metadata_value.encode_to_vec();
        let response_or_error = if let Some(expected_etag) = expected_etag {
            self.update_item_with_etag_precondition(
                resource_uri.clone(),
                resource_uri_hash,
                metadata_key.clone(),
                encoded_metadata_value,
                etag.clone(),
                expected_etag,
            )
            .await
        } else {
            self.update_item_without_etag_precondition(
                resource_uri.clone(),
                resource_uri_hash,
                metadata_key.clone(),
                encoded_metadata_value,
                etag.clone(),
            )
            .await
        };

        response_or_error?;

        Ok(etag)
    }

    /// Update an item with the etag precondition.
    #[instrument(skip_all)]
    async fn update_item_with_etag_precondition(
        &self,
        resource_uri: String,
        resource_uri_hash: Digest,
        metadata_key: String,
        metadata_value: Vec<u8>,
        etag: String,
        expected_etag: String,
    ) -> Result<(), Error> {
        // todo: metrics

        debug!(
            ?resource_uri,
            ?resource_uri_hash,
            ?metadata_key,
            ?etag,
            ?expected_etag,
            "updating an item conditionally"
        );

        self.client
            .update_item()
            .table_name(&self.table)
            .key(
                COL_RESOURCE_URI_HASH,
                AttributeValue::S(resource_uri_hash.to_digest_string()),
            )
            .key(COL_METADATA_KEY, AttributeValue::S(metadata_key))
            .update_expression(format!(
                "SET {} = {}, {} = {}",
                COL_METADATA_VALUE, PAR_METADATA_VALUE, COL_ETAG, PAR_ETAG
            ))
            .condition_expression(format!(
                "{} = {} AND {} = {}",
                COL_ETAG, PAR_EXPECTED_ETAG, COL_RESOURCE_URI, PAR_RESOURCE_URI
            ))
            .expression_attribute_values(PAR_EXPECTED_ETAG, AttributeValue::S(expected_etag))
            .expression_attribute_values(PAR_RESOURCE_URI, AttributeValue::S(resource_uri))
            .expression_attribute_values(PAR_METADATA_VALUE, AttributeValue::B(metadata_value.into()))
            .expression_attribute_values(PAR_ETAG, AttributeValue::S(etag))
            .send()
            .with_latency_metric(&self.metrics.put_item_latency)
            .with_concurrency_limit(&self.request_semaphore)
            .await
            .inspect(record_metric_value(&self.metrics.put_item_count, 200))
            .inspect_err(record_metric(&self.metrics.put_item_count))
            .map_err(|error| match error.as_service_error() {
                Some(e) if e.is_conditional_check_failed_exception() => Error::metadata_modified(error),
                Some(e) if e.is_throttled() => Error::request_throttled(error),
                _ => Error::put_failed(error),
            })?;

        Ok(())
    }

    /// Update an item without the etag precondition.
    #[instrument(skip_all)]
    async fn update_item_without_etag_precondition(
        &self,
        resource_uri: String,
        resource_uri_hash: Digest,
        metadata_key: String,
        metadata_value: Vec<u8>,
        etag: String,
    ) -> Result<(), Error> {
        // todo: metrics

        debug!(
            ?resource_uri,
            ?resource_uri_hash,
            ?metadata_key,
            ?etag,
            "updating an item unconditionally"
        );

        self.client
            .update_item()
            .table_name(&self.table)
            .key(
                COL_RESOURCE_URI_HASH,
                AttributeValue::S(resource_uri_hash.to_digest_string()),
            )
            .key(COL_METADATA_KEY, AttributeValue::S(metadata_key))
            .update_expression(format!(
                "SET {} = {}, {} = {}, {} = {}",
                COL_RESOURCE_URI, PAR_RESOURCE_URI, COL_METADATA_VALUE, PAR_METADATA_VALUE, COL_ETAG, PAR_ETAG
            ))
            .condition_expression(format!(
                "{} = {} OR (attribute_not_exists({}) AND attribute_not_exists({}))",
                COL_RESOURCE_URI, PAR_RESOURCE_URI, COL_RESOURCE_URI_HASH, COL_METADATA_KEY
            ))
            .expression_attribute_values(PAR_RESOURCE_URI, AttributeValue::S(resource_uri))
            .expression_attribute_values(PAR_METADATA_VALUE, AttributeValue::B(metadata_value.into()))
            .expression_attribute_values(PAR_ETAG, AttributeValue::S(etag))
            .send()
            .with_latency_metric(&self.metrics.put_item_latency)
            .with_concurrency_limit(&self.request_semaphore)
            .await
            .inspect(record_metric_value(&self.metrics.put_item_count, 200))
            .inspect_err(record_metric(&self.metrics.put_item_count))
            .map_err(|error| match error.as_service_error() {
                Some(e) if e.is_throttled() => Error::request_throttled(error),
                _ => Error::put_failed(error),
            })?;

        Ok(())
    }

    /// Delete a row.
    #[instrument(skip_all)]
    async fn delete_item(
        &self,
        resource_uri: String,
        metadata_key: String,
        expected_etag: Option<String>,
    ) -> Result<(), Error> {
        let resource_uri_hash = Digest::compute(&resource_uri);
        let maybe_error = if let Some(expected_etag) = expected_etag {
            self.delete_item_with_etag_precondition(
                resource_uri.clone(),
                resource_uri_hash,
                metadata_key.clone(),
                expected_etag,
            )
            .await
        } else {
            self.delete_item_without_etag_precondition(resource_uri.clone(), resource_uri_hash, metadata_key.clone())
                .await
        };

        maybe_error?;

        Ok(())
    }

    /// Delete an item with the etag precondition.
    #[instrument(skip_all)]
    async fn delete_item_with_etag_precondition(
        &self,
        resource_uri: String,
        resource_uri_hash: Digest,
        metadata_key: String,
        expected_etag: String,
    ) -> Result<(), Error> {
        debug!(
            ?resource_uri,
            ?resource_uri_hash,
            ?metadata_key,
            ?expected_etag,
            "deleting an item unconditionally"
        );

        self.client
            .delete_item()
            .table_name(&self.table)
            .key(
                COL_RESOURCE_URI_HASH,
                AttributeValue::S(resource_uri_hash.to_digest_string()),
            )
            .key(COL_METADATA_KEY, AttributeValue::S(metadata_key))
            .condition_expression(format!(
                "{} = {} AND {} = {}",
                COL_ETAG, PAR_ETAG, COL_RESOURCE_URI, PAR_RESOURCE_URI
            ))
            .expression_attribute_values(PAR_ETAG, AttributeValue::S(expected_etag))
            .expression_attribute_values(PAR_RESOURCE_URI, AttributeValue::S(resource_uri))
            .send()
            .with_latency_metric(&self.metrics.delete_item_latency)
            .with_concurrency_limit(&self.request_semaphore)
            .await
            .inspect(record_metric_value(&self.metrics.delete_item_count, 200))
            .inspect_err(record_metric(&self.metrics.delete_item_count))
            .map_err(|error| match error.as_service_error() {
                Some(e) if e.is_conditional_check_failed_exception() => Error::metadata_modified(error),
                Some(e) if e.is_throttled() => Error::request_throttled(error),
                _ => Error::delete_failed(error),
            })?;

        Ok(())
    }

    /// Delete an item without the etag precondition.
    #[instrument(skip_all)]
    async fn delete_item_without_etag_precondition(
        &self,
        resource_uri: String,
        resource_uri_hash: Digest,
        metadata_key: String,
    ) -> Result<(), Error> {
        debug!(
            ?resource_uri,
            ?resource_uri_hash,
            ?metadata_key,
            "deleting an item unconditionally"
        );

        let response_or_error = self
            .client
            .delete_item()
            .table_name(&self.table)
            .key(
                COL_RESOURCE_URI_HASH,
                AttributeValue::S(resource_uri_hash.to_digest_string()),
            )
            .key(COL_METADATA_KEY, AttributeValue::S(metadata_key))
            .condition_expression(format!("{} = {}", COL_RESOURCE_URI, PAR_RESOURCE_URI))
            .expression_attribute_values(PAR_RESOURCE_URI, AttributeValue::S(resource_uri))
            .send()
            .with_latency_metric(&self.metrics.delete_item_latency)
            .with_concurrency_limit(&self.request_semaphore)
            .await
            .inspect(record_metric_value(&self.metrics.delete_item_count, 200))
            .inspect_err(record_metric(&self.metrics.delete_item_count));

        if let Err(error) = response_or_error {
            match error.as_service_error() {
                Some(e) if e.is_conditional_check_failed_exception() => Ok(()),
                Some(e) if e.is_throttled() => Err(Error::request_throttled(error)),
                _ => Err(Error::delete_failed(error)),
            }
        } else {
            Ok(())
        }
    }
}

#[async_trait]
impl Metadata for DynamoDBMetadataBackend {
    fn inner(&self) -> Option<&dyn Metadata> {
        None
    }

    async fn get_metadata_batch(&self, request: Request<GetMetadataBatchRequest>) -> Result<GetMetadataBatchResponse> {
        todo!()
    }

    async fn query_metadata(
        &self,
        request: Request<QueryMetadataRequest>,
        options: StreamingOptions,
    ) -> Result<BoxStream<'static, Result<QueryMetadataResponsePage>>> {
        todo!()
    }

    async fn update_metadata_batch(
        &self,
        request: Request<UpdateMetadataBatchRequest>,
    ) -> Result<UpdateMetadataBatchResponse> {
        todo!()
    }

    async fn delete_metadata_batch(
        &self,
        request: Request<DeleteMetadataBatchRequest>,
    ) -> Result<DeleteMetadataBatchResponse> {
        todo!()
    }
}
