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

use std::{borrow::Cow, collections::HashSet, num::NonZero, sync::Arc};

use async_stream::try_stream;
use async_trait::async_trait;
use bytesize::ByteSize;
use deadpool_postgres::{Object, Pool};
use futures::{
    future::{try_join, try_join_all},
    stream::BoxStream,
    TryStreamExt,
};
use prost::Message;
use snafu::IntoError;
use storage_service_metrics::{future::FutureExt, status::result::Status, utils::record_metric_value};
use storage_service_protos::google::protobuf::Value;
use tokio::sync::RwLock;
use tokio_postgres::types::ToSql;
use tokio_util::sync::DropGuard;
use tracing::{debug, instrument};
use uuid::Uuid;

use crate::{
    backend::metadata::{
        batch::create_batch_error_entry_v1alpha,
        postgres::{
            error::{Error, InitError},
            query::{QueryKeyValueRow, SliceProject},
        },
        types::{KeyValue, NamedKeyValue},
        utils::{decode_value, Digest},
        *,
    },
    common::error::PrettyPrintErrorExt,
};

pub mod config;
pub mod credentials;
pub mod error;

const DEFAULT_PORT: u16 = 5432;
const DEFAULT_CONNECTION_POOL_SIZE: NonZero<usize> = NonZero::new(30).unwrap();
const DEFAULT_SCHEMA: &str = "public";
const DEFAULT_QUERY_BATCH_SIZE: NonZero<u32> = NonZero::new(100).unwrap();
const DEFAULT_QUERY_BYTE_SIZE_BUDGET: ByteSize = ByteSize::mib(16);
const DEFAULT_MAX_METADATA_VALUE_SIZE: ByteSize = ByteSize::mib(2);

mod metrics;
mod migrations;
mod query;

/// Postgres metadata backend.
#[derive(Debug)]
pub struct PostgresMetadataBackend {
    inner: Arc<PostgresMetadataBackendInner>,
}

impl PostgresMetadataBackend {
    pub async fn try_from_config(
        backend_id: &str,
        config: config::Config,
        credentials_provider: credentials::ArcCredentialsProvider,
    ) -> Result<Self, InitError> {
        let schema = config.schema.as_deref().unwrap_or(DEFAULT_SCHEMA);
        let queries = query::Queries::new(schema);

        let (pool, cancel_refresh) = credentials::create_renewable_connection_pool(
            credentials_provider,
            {
                let config = config.pool_config;
                move |credentials| {
                    let pool_or_error = Self::create_pool(config.clone(), credentials);
                    Box::pin(async move { pool_or_error })
                }
            },
            Default::default(),
        )
        .await?;

        // Initialize the database.
        {
            debug!("initializing the database");

            let mut conn = pool
                .read()
                .await
                .get()
                .await
                .map_err(|source| error::init_error::ConnectionAcquire.into_error(source))?;

            migrations::apply_migrations(schema, &mut conn)
                .await
                .map_err(|source| error::init_error::Migrate.into_error(source))?;
        }

        Ok(Self {
            inner: Arc::new(PostgresMetadataBackendInner {
                query_batch_size: config.query_batch_size.unwrap_or(DEFAULT_QUERY_BATCH_SIZE),
                query_byte_size_budget: config.query_byte_size_budget.unwrap_or(DEFAULT_QUERY_BYTE_SIZE_BUDGET),
                max_metadata_value_size: config
                    .max_metadata_value_size
                    .unwrap_or(DEFAULT_MAX_METADATA_VALUE_SIZE),
                metrics: metrics::Metrics::new(backend_id),
                pool,
                queries,
                _drop_guard: cancel_refresh.drop_guard(),
            }),
        })
    }

    fn create_pool(config: config::PoolConfig, credentials: credentials::Credentials) -> Result<Pool, InitError> {
        use deadpool_postgres::{Config, ManagerConfig, PoolConfig, RecyclingMethod, Runtime, SslMode};
        use native_tls::TlsConnector;
        use postgres_native_tls::MakeTlsConnector;
        use tokio_postgres::NoTls;

        let (user, password) = match credentials {
            credentials::Credentials::Password(password) => (password.user, password.password),
        };

        // Create the connection pool.
        let ssl_mode = config.ssl_mode.unwrap_or(SslMode::Prefer);
        let pool_config = Config {
            host: Some(config.host),
            dbname: Some(config.dbname),
            user: Some(user),
            password: Some(password.into_inner()),
            ssl_mode: Some(ssl_mode),
            port: Some(config.port.unwrap_or(DEFAULT_PORT)),
            pool: Some(PoolConfig::new(
                config
                    .connection_pool_size
                    .unwrap_or(DEFAULT_CONNECTION_POOL_SIZE)
                    .get(),
            )),
            manager: Some(ManagerConfig {
                recycling_method: RecyclingMethod::Fast,
            }),
            ..Default::default()
        };

        let pool = if ssl_mode == SslMode::Disable {
            pool_config
                .create_pool(Some(Runtime::Tokio1), NoTls)
                .map_err(|source| error::init_error::PoolCreate.into_error(source))?
        } else {
            let native_tls = TlsConnector::builder()
                .build()
                .map_err(|source| error::init_error::TlsInit.into_error(source))?;
            let connector = MakeTlsConnector::new(native_tls);

            pool_config
                .create_pool(Some(Runtime::Tokio1), connector)
                .map_err(|source| error::init_error::PoolCreate.into_error(source))?
        };

        Ok(pool)
    }
}

#[async_trait]
impl Metadata for PostgresMetadataBackend {
    fn inner(&self) -> Option<&dyn Metadata> {
        None
    }

    async fn get_metadata_batch(&self, request: Request<GetMetadataBatchRequest>) -> Result<GetMetadataBatchResponse> {
        let request = request.into_inner();

        let mut errors = Vec::new();
        let mut queries = Vec::with_capacity(request.requests.len());
        for (request_index, entry) in request.requests.into_iter().enumerate() {
            match GetKeyValueRequestRow::try_from_batch_request_entry(request_index as u32, entry) {
                Ok(row) => {
                    queries.push(row);
                },
                Err(error) => {
                    errors.push(GetKeyValueRowResult {
                        request_index: request_index as u32,
                        result: Err(error),
                    });
                },
            }
        }

        let get_results = if !queries.is_empty() {
            self.inner.get_key_values_and_map_results(queries).await?
        } else {
            Default::default()
        };

        let results = errors
            .into_iter()
            .chain(get_results)
            .map(|result| {
                let outcome = match result.result {
                    Ok(value) => get_metadata_batch_response_entry::Outcome::Success(value.into()),
                    Err(error) => get_metadata_batch_response_entry::Outcome::Error(create_error_entry(&error)),
                };

                GetMetadataBatchResponseEntry {
                    request_index: result.request_index,
                    outcome: Some(outcome),
                }
            })
            .collect();

        Ok(GetMetadataBatchResponse { results })
    }

    async fn query_metadata(
        &self,
        request: Request<QueryMetadataRequest>,
        options: StreamingOptions,
    ) -> Result<BoxStream<'static, Result<QueryMetadataResponsePage>>> {
        let request = request.into_inner();

        let resource_uri = get_resource_uri_from_resource_location(request.resource_location)?;
        let (start_metadata_key, limit) = match options {
            StreamingOptions::Streamed => (None, self.inner.query_batch_size),
            StreamingOptions::Paginated(options) => (
                options.continuation_handle,
                NonZero::new(options.max_page_size.unwrap_or_default()).unwrap_or(self.inner.query_batch_size),
            ),
        };

        let inner = self.inner.clone();
        Ok(Box::pin(try_stream! {
            let resource_uri_hash = Digest::compute(&resource_uri);
            let namespace = request.namespace.as_deref().unwrap_or(DEFAULT_NAMESPACE_NAME);
            let mut start_metadata_key = start_metadata_key;
            loop {
                let results = inner
                    .query_metadata_page(
                        &resource_uri,
                        &resource_uri_hash,
                        namespace,
                        start_metadata_key.as_deref(),
                        limit.get(),
                    )
                    .await?;

                let (is_exhausted, items) = match results {
                    QueryMetadataPageResult::Exhausted(items) => (true, items),
                    QueryMetadataPageResult::NotExhausted(items) => (false, items),
                };

                let results = items
                    .into_iter()
                    .map(|nkv| {
                        QueryMetadataResponseEntry {
                            user_metadata: Some(nkv.value.into()),
                            user_metadata_key: nkv.key,
                        }
                    })
                    .collect::<Vec<_>>();

                let next_start_metadata_key = if !is_exhausted {
                    results
                        .last()
                        .map(|entry| entry.user_metadata_key.clone())
                } else {
                    None
                };

                yield QueryMetadataResponsePage {
                    response: QueryMetadataResponse { results },
                    next_continuation_handle: next_start_metadata_key.clone(),
                };

                if next_start_metadata_key.is_none() {
                    break;
                }

                start_metadata_key = next_start_metadata_key;
            }
        }))
    }

    async fn update_metadata_batch(
        &self,
        request: Request<UpdateMetadataBatchRequest>,
    ) -> Result<UpdateMetadataBatchResponse> {
        let requests = request.into_inner().requests;

        let mut update_row_count = 0usize;
        let mut upsert_row_count = 0usize;
        for entry in requests.iter() {
            if entry.expected_etag.is_some() {
                upsert_row_count += 1;
            } else {
                update_row_count += 1;
            }
        }

        let mut errors: Vec<UpdateRowResult> = Vec::new();
        let mut updates = Vec::with_capacity(update_row_count);
        let mut upserts = Vec::with_capacity(upsert_row_count);
        for (request_index, entry) in requests.into_iter().enumerate() {
            match UpdateOrUpsertRow::try_from_batch_request_entry(
                request_index as i64,
                entry,
                self.inner.max_metadata_value_size.as_u64() as usize,
            ) {
                Ok(UpdateOrUpsertRow::Update(row)) => updates.push(row),
                Ok(UpdateOrUpsertRow::Upsert(row)) => upserts.push(row),
                Err(error) => errors.push(UpdateRowResult {
                    request_index: request_index as u32,
                    result: Err(error),
                }),
            }
        }

        let update_results = if !updates.is_empty() || !upserts.is_empty() {
            self.inner.batch_update_rows(updates, upserts).await?
        } else {
            Default::default()
        };

        Ok(UpdateMetadataBatchResponse {
            results: errors
                .into_iter()
                .chain(update_results)
                .map(|result| UpdateMetadataBatchResponseEntry {
                    outcome: Some(match result.result {
                        Ok(etag) => update_metadata_batch_response_entry::Outcome::Success(
                            UpdateMetadataBatchResponseEntrySuccess { etag },
                        ),
                        Err(error) => update_metadata_batch_response_entry::Outcome::Error(create_error_entry(&error)),
                    }),
                    request_index: result.request_index,
                })
                .collect(),
        })
    }

    async fn delete_metadata_batch(
        &self,
        request: Request<DeleteMetadataBatchRequest>,
    ) -> Result<DeleteMetadataBatchResponse> {
        let requests = request.into_inner().requests;
        let requests_len = requests.len();

        let mut rows = Vec::with_capacity(requests_len);
        let mut errors: Vec<DeleteRowResult> = Vec::new();
        for (request_index, entry) in requests.into_iter().enumerate() {
            match DeleteRequestRow::try_from_batch_request_entry(request_index as i64, entry) {
                Ok(row) => rows.push(row),
                Err(error) => errors.push(DeleteRowResult {
                    request_index: request_index as u32,
                    result: Err(error),
                }),
            }
        }

        let delete_results = if !rows.is_empty() {
            self.inner.batch_delete_rows(rows).await?
        } else {
            Default::default()
        };

        Ok(DeleteMetadataBatchResponse {
            results: errors
                .into_iter()
                .chain(delete_results)
                .map(|result| DeleteMetadataBatchResponseEntry {
                    outcome: Some(match result.result {
                        Ok(_) => delete_metadata_batch_response_entry::Outcome::Success(
                            DeleteMetadataBatchResponseEntrySuccess {},
                        ),
                        Err(error) => delete_metadata_batch_response_entry::Outcome::Error(create_error_entry(&error)),
                    }),
                    request_index: result.request_index,
                })
                .collect(),
        })
    }
}

#[derive(Debug)]
struct PostgresMetadataBackendInner {
    pool: Arc<RwLock<Pool>>,
    queries: query::Queries,
    query_batch_size: NonZero<u32>,
    query_byte_size_budget: ByteSize,
    max_metadata_value_size: ByteSize,
    metrics: metrics::Metrics,
    _drop_guard: DropGuard,
}

impl PostgresMetadataBackendInner {
    async fn get_key_values_and_map_results<U, K, N>(
        &self,
        requests: Vec<GetKeyValueRequestRow<U, K, N>>,
    ) -> Result<Vec<GetKeyValueRowResult>, Error>
    where
        U: ToSql + Send + Sync,
        K: ToSql + Send + Sync,
        N: ToSql + Send + Sync,
    {
        let results = self.get_key_values_batched(&requests).await?;
        debug_assert_eq!(requests.len(), results.len());

        Ok(requests
            .into_iter()
            .zip(results)
            .map(|(request, result)| GetKeyValueRowResult {
                request_index: request.request_index,
                result,
            })
            .collect())
    }

    #[instrument(skip_all, fields(requests_len = requests.len()))]
    async fn get_key_values_batched<U, K, N>(
        &self,
        requests: &[GetKeyValueRequestRow<U, K, N>],
    ) -> Result<Vec<Result<KeyValue, Error>>, Error>
    where
        U: ToSql + Send + Sync,
        K: ToSql + Send + Sync,
        N: ToSql + Send + Sync,
    {
        use error::error::{MetadataDecode, MetadataKeyNotFound};

        let mut results: Vec<Result<KeyValue, Error>> = requests.iter().map(|_| MetadataKeyNotFound.fail()).collect();

        let chunk_size = self.query_batch_size.get() as usize;
        let result_chunks = try_join_all(
            requests
                .chunks(chunk_size)
                .map(|requests| self.get_key_values_once(requests)),
        )
        .await?;

        for (idx, chunk) in result_chunks.into_iter().enumerate() {
            let chunk_result_offset = idx * chunk_size;
            for row in chunk.into_iter() {
                let result = match decode_value(&row.metadata_value) {
                    Ok(value) => Ok(KeyValue { etag: row.etag, value }),
                    Err(source) => {
                        tracing::error!(error = %source.pretty(), "failed to decode metadata value");
                        Err(MetadataDecode.into_error(source))
                    },
                };

                results[chunk_result_offset + row.request_row_index as usize] = result;
            }
        }

        Ok(results)
    }

    /// Read a batch of key values.
    #[instrument(skip_all, fields(requests_len = requests.len()))]
    async fn get_key_values_once<U, K, N>(
        &self,
        requests: &[GetKeyValueRequestRow<U, K, N>],
    ) -> Result<Vec<QueryKeyValueRow>, Error>
    where
        U: ToSql + Send + Sync,
        K: ToSql + Send + Sync,
        N: ToSql + Send + Sync,
    {
        debug!(?requests, "querying key values");

        self.metrics
            .query_metadata_batch_batch_size
            .record(requests.len() as u64);

        let conn = self.acquire_connection().await?;
        self.queries
            .query_key_values(
                &conn,
                &SliceProject::new(requests, |r| &r.resource_uri_hash),
                &SliceProject::new(requests, |r| &r.resource_uri),
                &SliceProject::new(requests, |r| &r.namespace),
                &SliceProject::new(requests, |r| &r.user_metadata_key),
            )
            .with_latency_metric(&self.metrics.query_metadata_batch_latency)
            .await
            .inspect(record_metric_value(
                &self.metrics.query_metadata_batch_count,
                Status::Ok,
            ))
            .inspect_err(record_metric_value(
                &self.metrics.query_metadata_batch_count,
                Status::Err,
            ))
            .map_err(|source| error::error::Query.into_error(source))?
            .try_collect::<Vec<QueryKeyValueRow>>()
            .await
            .inspect(|results| {
                debug!(?results, "query finished");
                self.metrics
                    .query_metadata_batch_response_size
                    .record(results.len() as u64);
            })
            .map_err(|source| error::error::Query.into_error(source))
    }

    #[instrument(skip_all)]
    async fn batch_update_rows(
        &self,
        updates: Vec<UpdateRequestRow>,
        upserts: Vec<UpsertRequestRow>,
    ) -> Result<Vec<UpdateRowResult>, Error> {
        use error::error::{Collision, MetadataModified};

        // TODO: transient failures must be reported on per-entry level.
        // TODO: failure in either of the operations doesn't mean the request failure.
        let (update_results, upsert_results) =
            try_join(self.apply_update_batch(&updates), self.apply_upsert_batch(&upserts)).await?;
        let results = updates
            .into_iter()
            .map(|request| {
                let result = if update_results.contains(&request.request_index) {
                    Ok(request.etag)
                } else {
                    MetadataModified.fail()
                };

                UpdateRowResult {
                    request_index: request.request_index as u32,
                    result,
                }
            })
            .chain(upserts.into_iter().map(|request| {
                let result = if upsert_results.contains(&request.request_index) {
                    Ok(request.etag)
                } else {
                    Collision.fail()
                };

                UpdateRowResult {
                    request_index: request.request_index as u32,
                    result,
                }
            }))
            .collect();

        Ok(results)
    }

    #[instrument(skip_all)]
    async fn apply_upsert_batch(&self, requests: &[UpsertRequestRow]) -> Result<HashSet<i64>, Error> {
        if requests.is_empty() {
            return Ok(Default::default());
        }

        debug!(?requests, "applying batch upsert");

        self.metrics.upsert_metadata_batch_size.record(requests.len() as u64);

        let conn = self.acquire_connection().await?;
        let rows = self
            .queries
            .upsert_metadata_batch(
                &conn,
                &SliceProject::new(requests, |r| &r.request_index),
                &SliceProject::new(requests, |r| &r.resource_uri_hash),
                &SliceProject::new(requests, |r| &r.resource_uri),
                &SliceProject::new(requests, |r| &r.namespace),
                &SliceProject::new(requests, |r| &r.metadata_key),
                &SliceProject::new(requests, |r| &r.encoded_value),
                &SliceProject::new(requests, |r| &r.etag),
            )
            .with_latency_metric(&self.metrics.upsert_metadata_batch_latency)
            .await
            .inspect(record_metric_value(
                &self.metrics.upsert_metadata_batch_count,
                Status::Ok,
            ))
            .inspect_err(record_metric_value(
                &self.metrics.upsert_metadata_batch_count,
                Status::Err,
            ))
            .map_err(|source| error::error::Query.into_error(source))?;

        rows.map_ok(|row| row.request_index)
            .try_collect::<HashSet<i64>>()
            .await
            .inspect(|result| {
                debug!(?result, "batch upsert finished");
                self.metrics
                    .upsert_metadata_batch_response_size
                    .record(result.len() as u64)
            })
            .map_err(|source| error::error::Query.into_error(source))
    }

    #[instrument(skip_all)]
    async fn apply_update_batch(&self, requests: &[UpdateRequestRow]) -> Result<HashSet<i64>, Error> {
        if requests.is_empty() {
            return Ok(Default::default());
        }

        debug!(?requests, "applying batch update");

        self.metrics.update_metadata_batch_size.record(requests.len() as u64);

        let conn = self.acquire_connection().await?;
        let rows = self
            .queries
            .update_metadata_batch(
                &conn,
                &SliceProject::new(requests, |r| &r.request_index),
                &SliceProject::new(requests, |r| &r.resource_uri_hash),
                &SliceProject::new(requests, |r| &r.resource_uri),
                &SliceProject::new(requests, |r| &r.namespace),
                &SliceProject::new(requests, |r| &r.metadata_key),
                &SliceProject::new(requests, |r| &r.encoded_value),
                &SliceProject::new(requests, |r| &r.expected_etag),
                &SliceProject::new(requests, |r| &r.etag),
            )
            .with_latency_metric(&self.metrics.update_metadata_batch_latency)
            .await
            .inspect(record_metric_value(
                &self.metrics.update_metadata_batch_count,
                Status::Ok,
            ))
            .inspect_err(record_metric_value(
                &self.metrics.update_metadata_batch_count,
                Status::Err,
            ))
            .map_err(|source| error::error::Query.into_error(source))?;

        rows.map_ok(|row| row.request_index)
            .try_collect::<HashSet<i64>>()
            .await
            .inspect(|results| {
                debug!(?results, "batch update finished");
                self.metrics
                    .update_metadata_batch_response_size
                    .record(results.len() as u64)
            })
            .map_err(|source| error::error::Query.into_error(source))
    }

    async fn batch_delete_rows(&self, requests: Vec<DeleteRequestRow>) -> Result<Vec<DeleteRowResult>, Error> {
        use error::error::MetadataModified;

        let request_indices = self.apply_delete_batch(&requests).await?;
        let results = requests
            .into_iter()
            .map(|request| {
                let result = if request_indices.contains(&request.request_index) {
                    Ok(())
                } else {
                    if request.expected_etag.is_some() {
                        MetadataModified.fail()
                    } else {
                        Ok(())
                    }
                };

                DeleteRowResult {
                    request_index: request.request_index as u32,
                    result,
                }
            })
            .collect();

        Ok(results)
    }

    #[instrument(skip_all)]
    async fn apply_delete_batch(&self, requests: &[DeleteRequestRow]) -> Result<HashSet<i64>, Error> {
        debug!(?requests, "applying batch delete");

        self.metrics.delete_metadata_batch_size.record(requests.len() as u64);

        let conn = self.acquire_connection().await?;
        let rows = self
            .queries
            .delete_metadata_batch(
                &conn,
                &SliceProject::new(requests, |r| &r.request_index),
                &SliceProject::new(requests, |r| &r.resource_uri_hash),
                &SliceProject::new(requests, |r| &r.resource_uri),
                &SliceProject::new(requests, |r| &r.namespace),
                &SliceProject::new(requests, |r| &r.metadata_key),
                &SliceProject::new(requests, |r| &r.expected_etag),
            )
            .with_latency_metric(&self.metrics.delete_metadata_batch_latency)
            .await
            .inspect(record_metric_value(
                &self.metrics.delete_metadata_batch_count,
                Status::Ok,
            ))
            .inspect_err(record_metric_value(
                &self.metrics.delete_metadata_batch_count,
                Status::Err,
            ))
            .map_err(|source| error::error::Query.into_error(source))?;

        rows.map_ok(|row| row.request_index)
            .try_collect::<HashSet<i64>>()
            .await
            .inspect(|results| {
                debug!(?results, "batch delete finished");
                self.metrics
                    .delete_metadata_batch_response_size
                    .record(results.len() as u64)
            })
            .map_err(|source| error::error::Query.into_error(source))
    }

    #[instrument(skip_all, fields(resource_uri, offset, limit))]
    async fn query_metadata_page(
        &self,
        resource_uri: &str,
        resource_uri_hash: &Digest,
        namespace: &str,
        start_metadata_key: Option<&str>,
        limit: u32,
    ) -> Result<QueryMetadataPageResult, Error> {
        use error::error::{MetadataDecode, Query};

        debug!(
            ?resource_uri,
            ?namespace,
            ?start_metadata_key,
            ?limit,
            budget = ?self.query_byte_size_budget,
            "querying key-value pairs for a resource uri"
        );

        let conn = self.acquire_connection().await?;

        let rows = self
            .queries
            .query_all_key_values_for_resource(
                &conn,
                &resource_uri,
                resource_uri_hash,
                &namespace,
                &start_metadata_key,
                limit,
                self.query_byte_size_budget,
            )
            .with_latency_metric(&self.metrics.query_all_metadata_for_resource_latency)
            .await
            .inspect(record_metric_value(
                &self.metrics.query_all_metadata_for_resource_count,
                Status::Ok,
            ))
            .inspect_err(record_metric_value(
                &self.metrics.query_all_metadata_for_resource_count,
                Status::Err,
            ))
            .map_err(|source| Query.into_error(source))?;

        let mut last_has_next = false;
        let rows = rows
            .map_err(|source| Query.into_error(source))
            .and_then(|row| {
                last_has_next = row.has_next;
                async move {
                    Ok(NamedKeyValue {
                        key: row.metadata_key,
                        value: KeyValue {
                            etag: row.etag,
                            value: decode_value(&row.metadata_value)
                                .map_err(|source| MetadataDecode.into_error(source))?,
                        },
                    })
                }
            })
            .try_collect::<Vec<_>>()
            .await
            .inspect(|results| {
                debug!(results_len = ?results.len(), ?last_has_next, "query finished");
                self.metrics
                    .query_all_metadata_for_resource_response_size
                    .record(results.len() as u64)
            })?;

        if last_has_next {
            Ok(QueryMetadataPageResult::NotExhausted(rows))
        } else {
            Ok(QueryMetadataPageResult::Exhausted(rows))
        }
    }

    #[instrument(skip_all)]
    async fn acquire_connection(&self) -> Result<Object, Error> {
        async { self.pool.read().await.get().await }
            .with_latency_metric(&self.metrics.connection_acquire_latency)
            .await
            .inspect(record_metric_value(&self.metrics.connection_acquire_count, Status::Ok))
            .inspect_err(record_metric_value(&self.metrics.connection_acquire_count, Status::Err))
            .map_err(|source| error::error::ConnectionAcquire.into_error(source))
    }
}

#[derive(Debug)]
struct UpdateRequestRow {
    request_index: i64,
    resource_uri_hash: Digest,
    resource_uri: String,
    namespace: Cow<'static, str>,
    metadata_key: String,
    encoded_value: Vec<u8>,
    expected_etag: String,
    etag: String,
}

#[derive(Debug)]
struct UpsertRequestRow {
    request_index: i64,
    resource_uri_hash: Digest,
    resource_uri: String,
    namespace: Cow<'static, str>,
    metadata_key: String,
    encoded_value: Vec<u8>,
    etag: String,
}

enum UpdateOrUpsertRow {
    Update(UpdateRequestRow),
    Upsert(UpsertRequestRow),
}

impl UpdateOrUpsertRow {
    fn try_from_batch_request_entry(
        request_index: i64,
        entry: UpdateMetadataBatchRequestEntry,
        max_encoded_value_len: usize,
    ) -> Result<Self, Error> {
        use error::error::UserMetadataValueMissing;

        let resource_uri = get_resource_uri_from_resource_location(entry.resource_location)?;
        let encoded_value = encode_value(
            entry
                .user_metadata
                .as_ref()
                .ok_or_else(|| UserMetadataValueMissing.build())?,
            max_encoded_value_len,
        )?;

        let row = if let Some(expected_etag) = entry.expected_etag {
            Self::Update(UpdateRequestRow {
                resource_uri_hash: Digest::compute(&resource_uri),
                namespace: unwrap_namespace_or_default(entry.namespace),
                metadata_key: entry.user_metadata_key,
                etag: create_random_etag(),
                request_index,
                encoded_value,
                expected_etag,
                resource_uri,
            })
        } else {
            Self::Upsert(UpsertRequestRow {
                resource_uri_hash: Digest::compute(&resource_uri),
                namespace: unwrap_namespace_or_default(entry.namespace),
                metadata_key: entry.user_metadata_key,
                etag: create_random_etag(),
                request_index,
                encoded_value,
                resource_uri,
            })
        };

        Ok(row)
    }
}

#[derive(Debug)]
struct DeleteRequestRow {
    request_index: i64,
    resource_uri: String,
    resource_uri_hash: Digest,
    namespace: Cow<'static, str>,
    metadata_key: String,
    expected_etag: Option<String>,
}

impl DeleteRequestRow {
    fn try_from_batch_request_entry(request_index: i64, entry: DeleteMetadataBatchRequestEntry) -> Result<Self, Error> {
        let resource_uri = get_resource_uri_from_resource_location(entry.resource_location)?;
        Ok(Self {
            resource_uri_hash: Digest::compute(&resource_uri),
            namespace: unwrap_namespace_or_default(entry.namespace),
            metadata_key: entry.user_metadata_key,
            expected_etag: entry.expected_etag,
            request_index,
            resource_uri,
        })
    }
}

struct UpdateRowResult {
    request_index: u32,
    result: Result<String, Error>,
}

struct DeleteRowResult {
    request_index: u32,
    result: Result<(), Error>,
}

#[derive(Debug)]
struct GetKeyValueRequestRow<U, K, N> {
    request_index: u32,
    resource_uri: U,
    resource_uri_hash: Digest,
    namespace: N,
    user_metadata_key: K,
}

impl GetKeyValueRequestRow<String, String, Cow<'static, str>> {
    fn try_from_batch_request_entry(request_index: u32, entry: GetMetadataBatchRequestEntry) -> Result<Self, Error> {
        let resource_uri = get_resource_uri_from_resource_location(entry.resource_location)?;
        Ok(Self {
            resource_uri_hash: Digest::compute(&resource_uri),
            namespace: unwrap_namespace_or_default(entry.namespace),
            user_metadata_key: entry.user_metadata_key,
            request_index,
            resource_uri,
        })
    }
}

struct GetKeyValueRowResult {
    request_index: u32,
    result: Result<KeyValue, Error>,
}

enum QueryMetadataPageResult {
    Exhausted(Vec<NamedKeyValue>),
    NotExhausted(Vec<NamedKeyValue>),
}

fn create_error_entry(error: &Error) -> BatchErrorEntry {
    use crate::backend::metadata::batch_error_entry::Code;

    match error {
        Error::ConnectionAcquire { .. }
        | Error::Query { .. }
        | Error::MetadataEncode { .. }
        | Error::MetadataDecode { .. }
        | Error::Collision { .. } => create_batch_error_entry_v1alpha(Code::Internal, "unexpected error"),
        Error::MetadataModified { .. } => create_batch_error_entry_v1alpha(Code::FailedPrecondition, error.to_string()),
        Error::ResourceLocationMissing { .. }
        | Error::UserMetadataValueMissing { .. }
        | Error::UserMetadataValueTooLarge { .. } => {
            create_batch_error_entry_v1alpha(Code::InvalidArgument, error.to_string())
        },
        Error::MetadataKeyNotFound { .. } => create_batch_error_entry_v1alpha(Code::NotFound, error.to_string()),
    }
}

#[track_caller]
fn encode_value(value: &Value, max_len: usize) -> Result<Vec<u8>, Error> {
    let encoded_len = value.encoded_len();
    if encoded_len > max_len {
        return error::error::UserMetadataValueTooLarge {
            actual_len: encoded_len,
            max_len,
        }
        .fail();
    }

    let mut buffer = Vec::with_capacity(encoded_len);
    value
        .encode(&mut buffer)
        .map_err(|source| error::error::MetadataEncode.into_error(source))?;

    Ok(buffer)
}

#[track_caller]
fn get_resource_uri_from_resource_location(resource_location: Option<ResourceLocation>) -> Result<String, Error> {
    use error::error::ResourceLocationMissing;
    use resource_location::Resource;

    match resource_location
        .ok_or_else(|| ResourceLocationMissing.build())?
        .resource
        .ok_or_else(|| ResourceLocationMissing.build())?
    {
        Resource::ResourceAddress(value) => Ok(value),
        Resource::ResourceIdentity(value) => Ok(value.encoded_identity),
        Resource::FolderAddress(value) => Ok(value.uri),
    }
}

fn create_random_etag() -> String {
    format!(r#""{}""#, Uuid::new_v4())
}

fn unwrap_namespace_or_default(value: Option<String>) -> Cow<'static, str> {
    if let Some(namespace) = value {
        Cow::Owned(namespace)
    } else {
        Cow::Borrowed(DEFAULT_NAMESPACE_NAME)
    }
}
