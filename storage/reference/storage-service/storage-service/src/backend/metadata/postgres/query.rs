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

use std::pin::pin;

use async_stream::try_stream;
use bytesize::ByteSize;
use futures::{stream::BoxStream, TryStreamExt};
use postgres_protocol::types::{array_to_sql, ArrayDimension};
use snafu::Snafu;
use tokio_postgres::{
    types::{to_sql_checked, IsNull, Kind, ToSql, Type},
    Client, Row,
};

use crate::backend::metadata::{postgres::error::QueryError, utils::Digest};

const TABLE_METADATA: &str = "metadata";

const COL_RESOURCE_URI_HASH: &str = "resource_uri_hash";
const COL_RESOURCE_URI: &str = "resource_uri";
const COL_METADATA_KEY: &str = "metadata_key";
const COL_METADATA_VALUE: &str = "metadata_value";
const COL_NAMESPACE: &str = "namespace";
const COL_ETAG: &str = "etag";

pub type RowStream<T> = BoxStream<'static, Result<T, QueryError>>;

#[derive(Debug)]
pub struct QueryKeyValuesForResourceRow {
    pub metadata_key: String,
    pub metadata_value: Vec<u8>,
    pub etag: String,
    pub has_next: bool,
}

impl TryFrom<Row> for QueryKeyValuesForResourceRow {
    type Error = QueryError;

    fn try_from(value: Row) -> Result<Self, Self::Error> {
        Ok(Self {
            metadata_key: value.try_get(0).map_err(QueryError::row_parse)?,
            metadata_value: value.try_get(1).map_err(QueryError::row_parse)?,
            etag: value.try_get(2).map_err(QueryError::row_parse)?,
            has_next: value.try_get(3).map_err(QueryError::row_parse)?,
        })
    }
}

#[derive(Debug)]
pub struct QueryKeyValueRow {
    pub request_row_index: i64,
    pub metadata_value: Vec<u8>,
    pub etag: String,
}

impl TryFrom<Row> for QueryKeyValueRow {
    type Error = QueryError;

    fn try_from(value: Row) -> Result<Self, Self::Error> {
        Ok(Self {
            request_row_index: value.try_get(0).map_err(QueryError::row_parse)?,
            metadata_value: value.try_get(1).map_err(QueryError::row_parse)?,
            etag: value.try_get(2).map_err(QueryError::row_parse)?,
        })
    }
}

#[derive(Debug)]
pub struct RequestIndexRow {
    pub request_index: i64,
}

impl TryFrom<Row> for RequestIndexRow {
    type Error = QueryError;

    fn try_from(value: Row) -> Result<Self, Self::Error> {
        Ok(Self {
            request_index: value.try_get(0).map_err(QueryError::row_parse)?,
        })
    }
}

#[derive(Debug)]
pub struct Queries {
    query_all_key_values_for_resource_query: String,
    query_key_values_query: String,
    upsert_metadata_batch_query: String,
    update_metadata_batch_query: String,
    delete_metadata_batch_query: String,
}

impl Queries {
    pub fn new(schema: &str) -> Self {
        Self {
            query_all_key_values_for_resource_query: format!(
                "WITH candidates AS (\
                    SELECT {COL_RESOURCE_URI_HASH}, {COL_NAMESPACE}, {COL_METADATA_KEY}, {COL_METADATA_VALUE}, {COL_ETAG}, \
                        octet_length({COL_METADATA_KEY}) + octet_length({COL_METADATA_VALUE}) + octet_length({COL_ETAG}) AS row_len \
                        FROM {schema}.{TABLE_METADATA} \
                            WHERE {COL_RESOURCE_URI_HASH} = $1 \
                                AND {COL_RESOURCE_URI} = $2 \
                                AND {COL_NAMESPACE} = $3 \
                                AND (\
                                    $4::text IS NULL \
                                    OR ({COL_RESOURCE_URI_HASH}, {COL_NAMESPACE}, {COL_METADATA_KEY}) > ($1, $3, $4)\
                                ) \
                        ORDER BY {COL_RESOURCE_URI_HASH}, {COL_NAMESPACE}, {COL_METADATA_KEY}
                        LIMIT $5\
                ), \
                sized AS (\
                    SELECT {COL_RESOURCE_URI_HASH}, {COL_NAMESPACE}, {COL_METADATA_KEY}, {COL_METADATA_VALUE}, {COL_ETAG}, \
                        row_number() OVER(\
                            ORDER BY {COL_RESOURCE_URI_HASH}, {COL_NAMESPACE}, {COL_METADATA_KEY}\
                        ) AS row_idx, \
                        sum(row_len) OVER(\
                            ORDER BY {COL_RESOURCE_URI_HASH}, {COL_NAMESPACE}, {COL_METADATA_KEY} \
                            ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW\
                        ) AS cumulative_row_len \
                        FROM candidates\
                ), \
                selected AS MATERIALIZED(\
                    SELECT {COL_METADATA_KEY}, {COL_METADATA_VALUE}, {COL_ETAG}, row_idx \
                        FROM sized \
                        WHERE row_idx = 1 OR cumulative_row_len <= $6 \
                        ORDER BY {COL_RESOURCE_URI_HASH}, {COL_NAMESPACE}, {COL_METADATA_KEY}\
                ), \
                last_selected AS (\
                    SELECT {COL_METADATA_KEY} FROM selected ORDER BY row_idx DESC LIMIT 1\
                ), \
                next_page_status AS (\
                    SELECT EXISTS(\
                        SELECT 1 \
                            FROM {schema}.{TABLE_METADATA} m \
                            CROSS JOIN last_selected ls \
                            WHERE m.{COL_RESOURCE_URI_HASH} = $1 \
                                AND m.{COL_RESOURCE_URI} = $2 \
                                AND m.{COL_NAMESPACE} = $3 \
                                AND (m.{COL_RESOURCE_URI_HASH}, m.{COL_NAMESPACE}, m.{COL_METADATA_KEY}) > ($1, $3, ls.{COL_METADATA_KEY})
                    ) as has_more\
                ) \
                SELECT {COL_METADATA_KEY}, {COL_METADATA_VALUE}, {COL_ETAG}, has_more \
                    FROM selected \
                    CROSS JOIN next_page_status \
                    ORDER BY row_idx"
            ),
            // The returned request_index value stores the zero-based index of the corresponding
            // request in the batch.
            query_key_values_query: format!(
                "WITH input AS(\
                    SELECT resource_uri_hash, resource_uri, namespace, metadata_key, ord - 1 as request_index \
                    FROM UNNEST($1::bytea[], $2::text[], $3::text[], $4::text[]) \
                        WITH ORDINALITY AS tmp(resource_uri_hash, resource_uri, namespace, metadata_key, ord)\
                ) \
                SELECT i.request_index, m.metadata_value, m.etag \
                    FROM {schema}.{TABLE_METADATA} AS m \
                        JOIN input AS i \
                            ON i.resource_uri_hash = m.{COL_RESOURCE_URI_HASH} \
                            AND i.namespace = m.{COL_NAMESPACE} \
                            AND i.metadata_key = m.{COL_METADATA_KEY} \
                            AND i.resource_uri = m.{COL_RESOURCE_URI}"
            ),
            upsert_metadata_batch_query: format!(
                "WITH input AS(\
                    SELECT request_index, resource_uri_hash, resource_uri, namespace, metadata_key, metadata_value, etag \
                    FROM UNNEST($1::bigint[], $2::bytea[], $3::text[], $4::text[], $5::text[], $6::bytea[], $7::text[]) \
                        AS tmp(request_index, resource_uri_hash, resource_uri, namespace, metadata_key, metadata_value, etag)\
                ), \
                ordered AS MATERIALIZED(\
                    SELECT * FROM input ORDER BY resource_uri_hash, namespace, metadata_key \
                ), \
                upserted AS(\
                    INSERT INTO {schema}.{TABLE_METADATA}(\
                        {COL_RESOURCE_URI_HASH}, \
                        {COL_RESOURCE_URI}, \
                        {COL_NAMESPACE}, \
                        {COL_METADATA_KEY}, \
                        {COL_METADATA_VALUE}, \
                        {COL_ETAG}) \
                    SELECT resource_uri_hash, resource_uri, namespace, metadata_key, metadata_value, etag \
                    FROM ordered \
                    ON CONFLICT({COL_RESOURCE_URI_HASH}, {COL_NAMESPACE}, {COL_METADATA_KEY}) \
                        DO UPDATE SET {COL_METADATA_VALUE} = EXCLUDED.{COL_METADATA_VALUE}, \
                            {COL_ETAG} = EXCLUDED.{COL_ETAG} \
                        WHERE {schema}.{TABLE_METADATA}.{COL_RESOURCE_URI} = EXCLUDED.{COL_RESOURCE_URI} \
                    RETURNING {COL_RESOURCE_URI_HASH}, {COL_NAMESPACE}, {COL_METADATA_KEY} \
                ) \
                SELECT ordered.request_index \
                FROM ordered JOIN upserted \
                    ON upserted.{COL_RESOURCE_URI_HASH} = ordered.resource_uri_hash \
                        AND upserted.{COL_NAMESPACE} = ordered.namespace \
                        AND upserted.{COL_METADATA_KEY} = ordered.metadata_key"
            ),
            update_metadata_batch_query: format!(
                "WITH input AS(\
                    SELECT request_index, resource_uri_hash, resource_uri, namespace, metadata_key, metadata_value, expected_etag, etag \
                    FROM UNNEST($1::bigint[], $2::bytea[], $3::text[], $4::text[], $5::text[], $6::bytea[], $7::text[], $8::text[]) \
                        AS tmp(request_index, resource_uri_hash, resource_uri, namespace, metadata_key, metadata_value, expected_etag, etag)\
                ), locked AS MATERIALIZED(\
                    SELECT m.ctid, i.metadata_value, i.etag, i.request_index \
                        FROM {schema}.{TABLE_METADATA} AS m \
                        JOIN input AS i \
                            ON m.{COL_RESOURCE_URI_HASH} = i.resource_uri_hash \
                                AND m.{COL_NAMESPACE} = i.namespace \
                                AND m.{COL_METADATA_KEY} = i.metadata_key \
                                AND m.{COL_RESOURCE_URI} = i.resource_uri \
                                AND m.{COL_ETAG} = i.expected_etag \
                        ORDER BY m.{COL_RESOURCE_URI_HASH}, m.{COL_NAMESPACE}, m.{COL_METADATA_KEY} \
                        FOR UPDATE OF m\
                ) \
                UPDATE {schema}.{TABLE_METADATA} AS d \
                    SET {COL_METADATA_VALUE} = l.metadata_value, \
                        {COL_ETAG} = l.etag \
                    FROM locked AS l \
                        WHERE d.ctid = l.ctid \
                    RETURNING l.request_index"
            ),
            delete_metadata_batch_query: format!(
                "WITH input AS (\
                    SELECT request_index, resource_uri_hash, resource_uri, namespace, metadata_key, expected_etag \
                    FROM UNNEST($1::bigint[], $2::bytea[], $3::text[], $4::text[], $5::text[], $6::text[]) \
                        AS tmp(request_index, resource_uri_hash, resource_uri, namespace, metadata_key, expected_etag)
                ), locked AS MATERIALIZED(\
                    SELECT m.ctid, i.request_index \
                        FROM input AS i \
                        JOIN {schema}.{TABLE_METADATA} AS m \
                            ON m.{COL_RESOURCE_URI_HASH} = i.resource_uri_hash \
                                AND m.{COL_RESOURCE_URI} = i.resource_uri \
                                AND m.{COL_NAMESPACE} = i.namespace \
                                AND m.{COL_METADATA_KEY} = i.metadata_key \
                                AND (i.expected_etag IS NULL OR m.{COL_ETAG} = i.expected_etag)
                        ORDER BY m.{COL_RESOURCE_URI_HASH}, m.{COL_NAMESPACE}, m.{COL_METADATA_KEY} \
                        FOR UPDATE OF m\
                ) \
                DELETE FROM {schema}.{TABLE_METADATA} AS m \
                    USING locked AS l \
                        WHERE m.ctid = l.ctid \
                    RETURNING l.request_index"
            ),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn query_all_key_values_for_resource(
        &self,
        conn: &Client,
        resource_uri: &(impl ToSql + Sync),
        resource_uri_hash: &(impl ToSql + Sync),
        namespace: &(impl ToSql + Sync),
        start_metadata_key: &(impl ToSql + Sync),
        page_len_limit: u32,
        page_size_budget: ByteSize,
    ) -> Result<RowStream<QueryKeyValuesForResourceRow>, QueryError> {
        query_all(
            conn,
            &self.query_all_key_values_for_resource_query,
            &[
                &resource_uri_hash,
                &resource_uri,
                &namespace,
                &start_metadata_key,
                &(page_len_limit as i64),
                &(page_size_budget.as_u64() as i64),
            ],
        )
        .await
    }

    pub async fn query_key_values(
        &self,
        conn: &Client,
        resource_uri_hashes: &(impl ToSql + Sync),
        resource_uris: &(impl ToSql + Sync),
        namespaces: &(impl ToSql + Sync),
        metadata_keys: &(impl ToSql + Sync),
    ) -> Result<RowStream<QueryKeyValueRow>, QueryError> {
        query_all(
            conn,
            &self.query_key_values_query,
            &[&resource_uri_hashes, &resource_uris, &namespaces, &metadata_keys],
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn upsert_metadata_batch(
        &self,
        conn: &Client,
        request_indices: &(impl ToSql + Sync),
        resource_uri_hashes: &(impl ToSql + Sync),
        resource_uris: &(impl ToSql + Sync),
        namespaces: &(impl ToSql + Sync),
        metadata_keys: &(impl ToSql + Sync),
        metadata_values: &(impl ToSql + Sync),
        etags: &(impl ToSql + Sync),
    ) -> Result<RowStream<RequestIndexRow>, QueryError> {
        query_all(
            conn,
            &self.upsert_metadata_batch_query,
            &[
                &request_indices,
                &resource_uri_hashes,
                &resource_uris,
                &namespaces,
                &metadata_keys,
                &metadata_values,
                &etags,
            ],
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn update_metadata_batch(
        &self,
        conn: &Client,
        request_indices: &(impl ToSql + Sync),
        resource_uri_hashes: &(impl ToSql + Sync),
        resource_uris: &(impl ToSql + Sync),
        namespaces: &(impl ToSql + Sync),
        metadata_keys: &(impl ToSql + Sync),
        metadata_values: &(impl ToSql + Sync),
        expected_etags: &(impl ToSql + Sync),
        etags: &(impl ToSql + Sync),
    ) -> Result<RowStream<RequestIndexRow>, QueryError> {
        query_all(
            conn,
            &self.update_metadata_batch_query,
            &[
                &request_indices,
                &resource_uri_hashes,
                &resource_uris,
                &namespaces,
                &metadata_keys,
                &metadata_values,
                &expected_etags,
                &etags,
            ],
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn delete_metadata_batch(
        &self,
        conn: &Client,
        request_indices: &(impl ToSql + Sync),
        resource_uri_hashes: &(impl ToSql + Sync),
        resource_uris: &(impl ToSql + Sync),
        namespaces: &(impl ToSql + Sync),
        metadata_keys: &(impl ToSql + Sync),
        expected_etags: &(impl ToSql + Sync),
    ) -> Result<RowStream<RequestIndexRow>, QueryError> {
        query_all(
            conn,
            &self.delete_metadata_batch_query,
            &[
                &request_indices,
                &resource_uri_hashes,
                &resource_uris,
                &namespaces,
                &metadata_keys,
                &expected_etags,
            ],
        )
        .await
    }
}

pub struct SliceProject<'a, T, P> {
    slice: &'a [T],
    project: P,
}

impl<'a, T, P, V> SliceProject<'a, T, P>
where
    P: for<'b> Fn(&'b T) -> &'b V,
{
    pub fn new(slice: &'a [T], project: P) -> Self {
        Self { slice, project }
    }
}

impl<'a, T, P> std::fmt::Debug for SliceProject<'a, T, P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SliceProject").finish_non_exhaustive()
    }
}

impl<'a, V: ToSql, T, P> ToSql for SliceProject<'a, T, P>
where
    P: for<'b> Fn(&'b T) -> &'b V,
{
    fn to_sql(
        &self,
        ty: &tokio_postgres::types::Type,
        out: &mut bytes::BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>>
    where
        Self: Sized,
    {
        let member_type = match *ty.kind() {
            Kind::Array(ref member) => member,
            _ => return Err(Box::new(SliceProjectError::ArrayTypeExpected)),
        };

        let lower_bound = match *ty {
            Type::OID_VECTOR | Type::INT2_VECTOR => 0,
            _ => 1,
        };

        let dimension = ArrayDimension {
            len: self
                .slice
                .len()
                .try_into()
                .map_err(|_| Box::new(SliceProjectError::SliceIsTooLong))?,
            lower_bound,
        };

        array_to_sql(
            Some(dimension),
            member_type.oid(),
            self.slice.iter().map(|item| (self.project)(item)),
            |e, out| match e.to_sql(member_type, out)? {
                IsNull::No => Ok(postgres_protocol::IsNull::No),
                IsNull::Yes => Ok(postgres_protocol::IsNull::Yes),
            },
            out,
        )?;

        Ok(IsNull::No)
    }

    fn accepts(ty: &tokio_postgres::types::Type) -> bool
    where
        Self: Sized,
    {
        match *ty.kind() {
            Kind::Array(ref member) => V::accepts(member),
            _ => false,
        }
    }

    to_sql_checked!();
}

impl ToSql for Digest {
    fn to_sql(
        &self,
        ty: &tokio_postgres::types::Type,
        out: &mut bytes::BytesMut,
    ) -> Result<tokio_postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>> {
        self.to_digest_byte_string().as_slice().to_sql(ty, out)
    }

    fn accepts(ty: &tokio_postgres::types::Type) -> bool {
        matches!(ty, &tokio_postgres::types::Type::BYTEA)
    }

    to_sql_checked!();
}

#[derive(Snafu, Debug)]
#[snafu(module, context(suffix(false)))]
enum SliceProjectError {
    #[snafu(display("array type expected"))]
    ArrayTypeExpected,

    #[snafu(display("slice is too long"))]
    SliceIsTooLong,
}

async fn query_raw(
    conn: &Client,
    statement: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Result<tokio_postgres::RowStream, tokio_postgres::Error> {
    conn.query_raw(statement, slice_iter(params)).await
}

async fn query_all<T>(
    conn: &Client,
    statement: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Result<RowStream<T>, QueryError>
where
    T: TryFrom<Row, Error = QueryError>,
    T: Send + 'static,
{
    let rows = query_raw(conn, statement, params).await.map_err(QueryError::execute)?;

    Ok(Box::pin(try_stream! {
        let mut rows = pin!(rows);
        while let Some(row) = rows.try_next().await.map_err(QueryError::row_read)? {
            yield T::try_from(row)?;
        }
    }))
}

fn slice_iter<'a>(s: &'a [&'a (dyn ToSql + Sync)]) -> impl ExactSizeIterator<Item = &'a dyn ToSql> + 'a {
    s.iter().map(|s| *s as _)
}
