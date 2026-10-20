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

use std::{num::NonZero, sync::Arc};

use azure_core::{auth::TokenCredential, prelude::IfMatchCondition, StatusCode, TransportOptions};
use azure_data_cosmos::{
    prelude::{AuthorizationToken, CloudLocation, CollectionClient, CosmosClientBuilder, GetDocumentResponse, Query},
    ConsistencyLevel,
};
use futures::{future::try_join_all, TryFutureExt, TryStreamExt};
use snafu::IntoError;
use storage_service_metrics::{
    future::FutureExt,
    utils::{record_metric, record_metric_value},
};
use storage_service_protos::google::protobuf::Value;
use tokio::sync::Semaphore;
use tonic::async_trait;
use tracing::{debug, error, instrument, warn};

use crate::{
    backend::metadata::{
        azure_cosmosdb::{
            config::Config,
            error::{
                CollisionSnafu, DeleteDocumentFailedSnafu, DocumentClientCreateFailedSnafu, Error,
                GetDocumentFailedSnafu, HttpClientBuildFailedSnafu, InitError, MetadataModifiedSnafu,
                MetadataValueEncodeFailedSnafu, QueryDocumentsFailedSnafu, UpdateDocumentFailedSnafu,
                UserMetadataValueMissingSnafu,
            },
            metrics::Metrics,
            utils::{
                create_partition_fetch_all_query, decode_user_metadata_value, encode_user_metadata_value,
                FindOneDocument, GetOneDocument, HttpErrorExt, InsertDocument, PartitionQueryDocument,
            },
        },
        types::{KeyValue, NamedKeyValue},
        utils::Digest,
        *,
    },
    common::{error::PrettyPrintErrorExt, future::WithConcurrencyLimitExt, secret::Secret},
};

pub mod config;
pub mod error;

mod metrics;
mod utils;

const DEFAULT_MAX_CONCURRENCY: NonZero<usize> = NonZero::new(1000).unwrap();

const INVALID_DOCUMENT_ETAG: &str = r#""00000000-00000000-00000000-00000000""#;

#[derive(Debug)]
pub enum Credentials {
    PrimaryKey(Secret<String>),
    TokenCredential(Arc<dyn TokenCredential>),
}

/// CosmosDB metadata backend.
#[derive(Debug)]
pub struct AzureCosmosDBMetadataBackend {
    client: CollectionClient,
    request_semaphore: Semaphore,
    metrics: Metrics,
    consistency_levels: ConsistencyLevels,
}

impl AzureCosmosDBMetadataBackend {
    /// Create a client from configuration.
    pub fn try_from_config(backend_id: &str, config: Config, credentials: Credentials) -> Result<Self, InitError> {
        let auth_token = match credentials {
            Credentials::PrimaryKey(key) => {
                debug!("using primary key authentication");

                AuthorizationToken::PrimaryKey(key.into_inner().into())
            },
            Credentials::TokenCredential(credential) => {
                debug!(?credential, "using token authentication");

                AuthorizationToken::TokenCredential(credential)
            },
        };

        let http_client = reqwest::Client::builder()
            .pool_max_idle_per_host(config.max_idle_connections_per_host.unwrap_or(usize::MAX))
            .build()
            .map_err(|source| HttpClientBuildFailedSnafu.into_error(source))?;

        let client = CosmosClientBuilder::with_location(CloudLocation::Public {
            account: config.account,
            auth_token,
        })
        .transport(TransportOptions::new(Arc::new(http_client)))
        .build()
        .database_client(config.database_id)
        .collection_client(config.container_id);

        Ok(Self {
            client,
            request_semaphore: Semaphore::new(config.max_concurrency.unwrap_or(DEFAULT_MAX_CONCURRENCY).get()),
            metrics: Metrics::new(backend_id),
            consistency_levels: ConsistencyLevels {
                query: config.consistency_levels.query.into(),
                create: config.consistency_levels.create.into(),
                delete: config.consistency_levels.delete.into(),
            },
        })
    }

    /// Get metadata items through cache.
    #[instrument(skip_all)]
    async fn get_or_query_metadata(
        &self,
        resource_uri: String,
        metadata_keys: Vec<String>,
        consistency_level: &ConsistencyLevel,
    ) -> Result<GetMetadataResponse, Error> {
        let results = if metadata_keys.is_empty() {
            self.get_all_partition_documents(resource_uri, consistency_level)
                .await?
        } else {
            self.get_selected_partition_documents(resource_uri, metadata_keys, consistency_level)
                .await?
        };

        Ok(GetMetadataResponse {
            user_metadata: results
                .into_iter()
                .map(|nkv| (nkv.key, UserMetadataValue::from(nkv.value)))
                .collect(),
        })
    }

    /// Get all documents in a partition.
    #[instrument(skip_all)]
    async fn get_all_partition_documents(
        &self,
        resource_uri: String,
        consistency_level: &ConsistencyLevel,
    ) -> Result<Vec<NamedKeyValue>, Error> {
        debug!(?resource_uri, "fetching all documents in a partition");

        let partition_key_digest = Digest::compute(&resource_uri);
        let query = create_partition_fetch_all_query(resource_uri.clone(), &partition_key_digest);
        let results = self
            .fetch_partition_documents_with_query(query, consistency_level.clone())
            .await?;

        Ok(results)
    }

    /// Get selected documents in a parition.
    #[instrument(skip_all)]
    async fn get_selected_partition_documents(
        &self,
        resource_uri: String,
        metadata_keys: Vec<String>,
        consistency_level: &ConsistencyLevel,
    ) -> Result<Vec<NamedKeyValue>, Error> {
        debug!(
            ?resource_uri,
            ?metadata_keys,
            ?consistency_level,
            "reading partition documents through cache"
        );

        let get_document_results = try_join_all(metadata_keys.iter().map(|metadata_key| async {
            self.get_document(&resource_uri, metadata_key.as_str(), consistency_level.clone())
                .await
        }))
        .await?;

        let results = metadata_keys
            .into_iter()
            .zip(get_document_results.into_iter())
            .filter_map(|(metadata_key, maybe_key_value)| {
                maybe_key_value.map(|key_value| NamedKeyValue {
                    key: metadata_key,
                    value: key_value,
                })
            })
            .collect();

        Ok(results)
    }

    /// Fetch documents in a partition given a query.
    #[instrument(skip(self, query))]
    async fn fetch_partition_documents_with_query(
        &self,
        query: Query,
        consistency_level: ConsistencyLevel,
    ) -> Result<Vec<NamedKeyValue>, Error> {
        debug!(?query, ?consistency_level, "fetching documents in a partition");

        let mut results = Vec::new();
        let mut stream = self
            .client
            .query_documents(query)
            .consistency_level(consistency_level)
            .into_stream::<PartitionQueryDocument>();
        while let Some(page) = stream
            .try_next()
            .with_latency_metric(&self.metrics.query_documents_latency)
            .with_concurrency_limit(&self.request_semaphore)
            .inspect_ok(record_metric_value(&self.metrics.query_documents_count, StatusCode::Ok))
            .inspect_err(record_metric(&self.metrics.query_documents_count))
            .await
            .map_err(|source| QueryDocumentsFailedSnafu.into_error(source))?
        {
            debug!(?page, "query result fetched");

            for document in page.results.into_iter() {
                results.push(document.try_into()?);
            }

            // Exit early. We don't want to record false metrics.
            if page.continuation_token.is_none() {
                break;
            }
        }

        Ok(results)
    }

    /// Point-read a document.
    #[instrument(skip_all)]
    async fn get_document(
        &self,
        resource_uri: &str,
        metadata_key: &str,
        consistency_level: ConsistencyLevel,
    ) -> Result<Option<KeyValue>, Error> {
        let resource_uri_hash = Digest::compute(resource_uri);
        let metadata_key_hash = Digest::compute(metadata_key);

        debug!(
            ?resource_uri,
            ?metadata_key,
            ?resource_uri_hash,
            ?metadata_key_hash,
            "reading a document"
        );

        let builder = self
            .client
            .document_client(metadata_key_hash.to_digest_string(), &resource_uri_hash)
            .map_err(|source| DocumentClientCreateFailedSnafu.into_error(source))?
            .get_document::<GetOneDocument>()
            .consistency_level(consistency_level);

        let maybe_response = async move { builder.await }
            .with_latency_metric(&self.metrics.get_document_latency)
            .with_concurrency_limit(&self.request_semaphore)
            .await
            .inspect(record_metric_value(&self.metrics.get_document_count, StatusCode::Ok))
            .inspect_err(record_metric(&self.metrics.get_document_count));

        let response = match maybe_response {
            Ok(GetDocumentResponse::Found(document)) => document,
            Ok(GetDocumentResponse::NotFound(_)) => return Ok(None),
            Err(error) => match error.as_http_error() {
                Some(e) if e.is_not_found() => return Ok(None),
                _ => return Err(GetDocumentFailedSnafu.into_error(error)),
            },
        };

        let document = response.document.document;
        if document.resource_uri != resource_uri || document.metadata_key != metadata_key {
            warn!(
                ?resource_uri,
                ?metadata_key,
                actual_resource_uri = ?document.resource_uri,
                actual_metadata_key = ?document.metadata_key,
                ?resource_uri_hash,
                ?metadata_key_hash,
                "expectation mismatch while reading a document"
            );

            return Ok(None);
        }

        let Ok(value) = decode_user_metadata_value(&document.metadata_value)
            .inspect_err(|error| error!(error = %error.pretty(), "failed to decode user metadata value"))
        else {
            return Ok(None);
        };

        Ok(Some(KeyValue {
            etag: response.etag,
            value,
        }))
    }

    /// Update a document.
    #[instrument(skip_all)]
    async fn update_document(
        &self,
        resource_uri: String,
        metadata_key: String,
        metadata_value: Value,
        expected_etag: Option<String>,
        consistency_level: &ConsistencyLevel,
    ) -> Result<String, Error> {
        let resource_uri_hash = Digest::compute(&resource_uri);
        let metadata_key_hash = Digest::compute(&metadata_key);

        let encode_metadata_value = || {
            encode_user_metadata_value(&metadata_value)
                .map_err(|source| MetadataValueEncodeFailedSnafu.into_error(source))
        };

        let etag = if let Some(expected_etag) = expected_etag {
            debug!("expected etag value specified, selecting fast insert");

            self.update_existing_document(
                &resource_uri,
                &resource_uri_hash,
                &metadata_key,
                &metadata_key_hash,
                encode_metadata_value()?,
                expected_etag,
                consistency_level.clone(),
            )
            .await
            .map_err(|source| match source.as_http_error() {
                Some(e) if e.is_failed_precondition() => MetadataModifiedSnafu.into_error(source),
                _ => UpdateDocumentFailedSnafu.into_error(source),
            })?
        } else {
            debug!("no expected etag value has been specified, selecting slow insert");

            let maybe_existing_etag = self
                .find_document_etag(
                    &resource_uri,
                    &resource_uri_hash,
                    &metadata_key,
                    &metadata_key_hash,
                    consistency_level.clone(),
                )
                .await?;
            if let Some(existing_etag) = maybe_existing_etag {
                let etag_or_error = self
                    .update_existing_document(
                        &resource_uri,
                        &resource_uri_hash,
                        &metadata_key,
                        &metadata_key_hash,
                        encode_metadata_value()?,
                        existing_etag,
                        consistency_level.clone(),
                    )
                    .await;

                match etag_or_error {
                    Err(error) => match error.as_http_error() {
                        Some(e) if e.is_failed_precondition() => {
                            warn!(
                                ?resource_uri,
                                ?metadata_key,
                                ?resource_uri_hash,
                                ?metadata_key_hash,
                                ?error,
                                "race condition while updating a document"
                            );

                            INVALID_DOCUMENT_ETAG.into()
                        },
                        _ => return Err(UpdateDocumentFailedSnafu.into_error(error)),
                    },
                    Ok(etag) => etag,
                }
            } else {
                let etag_or_error = self
                    .create_new_document(
                        &resource_uri,
                        &resource_uri_hash,
                        &metadata_key,
                        &metadata_key_hash,
                        encode_metadata_value()?,
                        consistency_level.clone(),
                    )
                    .await;

                match etag_or_error {
                    Err(error) => match error.as_http_error() {
                        Some(e) if e.is_conflict() => {
                            warn!(
                                ?resource_uri,
                                ?metadata_key,
                                ?resource_uri_hash,
                                ?metadata_key_hash,
                                ?error,
                                "race condition while creating a document"
                            );

                            INVALID_DOCUMENT_ETAG.into()
                        },
                        _ => return Err(UpdateDocumentFailedSnafu.into_error(error)),
                    },
                    Ok(etag) => etag,
                }
            }
        };

        Ok(etag)
    }

    /// Find a docment and return its etag.
    #[instrument(skip_all)]
    async fn find_document_etag(
        &self,
        resource_uri: &str,
        resource_uri_hash: &Digest,
        metadata_key: &str,
        metadata_key_hash: &Digest,
        consistency_level: ConsistencyLevel,
    ) -> Result<Option<String>, Error> {
        debug!(
            ?resource_uri,
            ?metadata_key,
            ?resource_uri_hash,
            ?metadata_key_hash,
            ?consistency_level,
            "finding a document"
        );

        let builder = self
            .client
            .document_client(metadata_key_hash.to_digest_string(), resource_uri_hash)
            .map_err(|source| DocumentClientCreateFailedSnafu.into_error(source))?
            .get_document::<FindOneDocument>()
            .consistency_level(consistency_level);

        let maybe_response = async move { builder.await }
            .with_latency_metric(&self.metrics.get_document_latency)
            .with_concurrency_limit(&self.request_semaphore)
            .await
            .inspect(record_metric_value(&self.metrics.get_document_count, StatusCode::Ok))
            .inspect_err(record_metric(&self.metrics.get_document_count));

        let response = match maybe_response {
            Ok(GetDocumentResponse::Found(document)) => document,
            Ok(GetDocumentResponse::NotFound(_)) => return Ok(None),
            Err(error) => match error.as_http_error() {
                Some(e) if e.is_not_found() => return Ok(None),
                _ => return Err(GetDocumentFailedSnafu.into_error(error)),
            },
        };

        let document = response.document.document;
        if document.resource_uri != resource_uri || document.metadata_key != metadata_key {
            return CollisionSnafu.fail();
        }

        Ok(Some(response.etag))
    }

    /// Update an existing document and invalidate caches.
    #[allow(clippy::too_many_arguments)]
    #[instrument(skip_all)]
    async fn update_existing_document(
        &self,
        resource_uri: &str,
        resource_uri_hash: &Digest,
        metadata_key: &str,
        metadata_key_hash: &Digest,
        metadata_value: String,
        expected_etag: String,
        consistency_level: ConsistencyLevel,
    ) -> Result<String, azure_core::Error> {
        debug!(
            ?resource_uri,
            ?resource_uri_hash,
            ?metadata_key,
            ?metadata_key_hash,
            ?expected_etag,
            ?consistency_level,
            "upserting a document"
        );

        let builder = self
            .client
            .create_document(InsertDocument {
                resource_uri_hash: resource_uri_hash.to_digest_string(),
                metadata_key_hash: metadata_key_hash.to_digest_string(),
                resource_uri: resource_uri.into(),
                metadata_key: metadata_key.into(),
                metadata_value,
            })
            .is_upsert(true)
            .if_match_condition(IfMatchCondition::Match(expected_etag))
            .consistency_level(consistency_level);

        let response_or_error = async move { builder.await }
            .with_latency_metric(&self.metrics.create_document_latency)
            .with_concurrency_limit(&self.request_semaphore)
            .await
            .inspect(record_metric_value(
                &self.metrics.create_document_count,
                StatusCode::Created,
            ))
            .inspect_err(record_metric(&self.metrics.create_document_count));

        response_or_error.map(|response| response.etag)
    }

    /// Create a new document and invalidate caches.
    #[instrument(skip_all)]
    async fn create_new_document(
        &self,
        resource_uri: &str,
        resource_uri_hash: &Digest,
        metadata_key: &str,
        metadata_key_hash: &Digest,
        metadata_value: String,
        consistency_level: ConsistencyLevel,
    ) -> Result<String, azure_core::Error> {
        debug!(
            ?resource_uri,
            ?metadata_key,
            ?resource_uri_hash,
            ?metadata_key_hash,
            ?consistency_level,
            "creating a document"
        );

        let builder = self
            .client
            .create_document(InsertDocument {
                resource_uri_hash: resource_uri_hash.to_digest_string(),
                metadata_key_hash: metadata_key_hash.to_digest_string(),
                resource_uri: resource_uri.into(),
                metadata_key: metadata_key.into(),
                metadata_value,
            })
            .consistency_level(consistency_level);

        let response_or_error = async move { builder.await }
            .with_latency_metric(&self.metrics.create_document_latency)
            .with_concurrency_limit(&self.request_semaphore)
            .await
            .inspect(record_metric_value(
                &self.metrics.create_document_count,
                StatusCode::Created,
            ))
            .inspect_err(record_metric(&self.metrics.create_document_count));

        response_or_error.map(|response| response.etag)
    }

    /// Delete a document.
    #[instrument(skip_all)]
    async fn delete_document(
        &self,
        resource_uri: String,
        metadata_key: String,
        expected_etag: Option<String>,
        consistency_level: &ConsistencyLevel,
    ) -> Result<(), Error> {
        let resource_uri_hash = Digest::compute(&resource_uri);
        let metadata_key_hash = Digest::compute(&metadata_key);

        if let Some(expected_etag) = expected_etag {
            debug!("expected etag value specified, selecting fast delete");

            let maybe_error = self
                .delete_existing_document(
                    &resource_uri,
                    &resource_uri_hash,
                    &metadata_key,
                    &metadata_key_hash,
                    expected_etag,
                    consistency_level.clone(),
                )
                .await;

            if let Err(error) = maybe_error {
                match error.as_http_error() {
                    Some(e) if e.is_not_found() => return Ok(()),
                    Some(e) if e.is_failed_precondition() => return Err(MetadataModifiedSnafu.into_error(error)),
                    _ => return Err(DeleteDocumentFailedSnafu.into_error(error)),
                }
            }
        } else {
            debug!("expected etag value not specified, selecting slow delete");

            let maybe_existing_etag = self
                .find_document_etag(
                    &resource_uri,
                    &resource_uri_hash,
                    &metadata_key,
                    &metadata_key_hash,
                    consistency_level.clone(),
                )
                .await?;
            let Some(existing_etag) = maybe_existing_etag else {
                debug!("document not found, nothing to delete");

                return Ok(());
            };

            let maybe_error = self
                .delete_existing_document(
                    &resource_uri,
                    &resource_uri_hash,
                    &metadata_key,
                    &metadata_key_hash,
                    existing_etag,
                    consistency_level.clone(),
                )
                .await;
            if let Err(error) = maybe_error {
                match error.as_http_error() {
                    Some(e) if e.is_not_found() => return Ok(()),
                    Some(e) if e.is_failed_precondition() => {
                        warn!(
                            ?resource_uri,
                            ?metadata_key,
                            ?resource_uri_hash,
                            ?metadata_key_hash,
                            "race condition while deleting a document"
                        );

                        return Ok(());
                    },
                    _ => return Err(DeleteDocumentFailedSnafu.into_error(error)),
                }
            }
        }

        Ok(())
    }

    /// Delete an existing document and invalidate caches.
    #[instrument(skip_all)]
    async fn delete_existing_document(
        &self,
        resource_uri: &str,
        resource_uri_hash: &Digest,
        metadata_key: &str,
        metadata_key_hash: &Digest,
        expected_etag: String,
        consistency_level: ConsistencyLevel,
    ) -> Result<(), azure_core::Error> {
        debug!(
            ?resource_uri,
            ?metadata_key,
            ?resource_uri_hash,
            ?metadata_key_hash,
            ?expected_etag,
            ?consistency_level,
            "deleting a document"
        );

        let builder = self
            .client
            .document_client(metadata_key_hash.to_digest_string(), resource_uri_hash)?
            .delete_document()
            .if_match_condition(IfMatchCondition::Match(expected_etag))
            .consistency_level(consistency_level);

        let response_or_error = async move { builder.await }
            .with_latency_metric(&self.metrics.delete_document_latency)
            .with_concurrency_limit(&self.request_semaphore)
            .await
            .inspect(record_metric_value(
                &self.metrics.delete_document_count,
                StatusCode::NoContent,
            ))
            .inspect_err(record_metric(&self.metrics.delete_document_count));

        response_or_error.map(|_| ())
    }
}

#[async_trait]
impl Metadata for AzureCosmosDBMetadataBackend {
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
    ) -> Result<BoxStream<QueryMetadataResponsePage>> {
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

#[derive(Debug, Clone)]
struct ConsistencyLevels {
    query: ConsistencyLevel,
    create: ConsistencyLevel,
    delete: ConsistencyLevel,
}
