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

use std::{future::Future, pin::pin, time::Duration};

use deadpool_postgres::{Client, Transaction};
use futures::{
    future::{self, Either},
    TryStreamExt,
};
use snafu::IntoError;
use tracing::{debug, info};

use crate::backend::metadata::postgres::error::{migration_error, MigrationError};

const TABLE_MIGRATIONS: &str = "migrations";

const COL_NAME: &str = "name";

/// Apply database migrations using a connection.
pub async fn apply_migrations(schema: impl Into<String>, conn: &mut Client) -> Result<(), MigrationError> {
    let migrate_future = async { Migrator::try_new(schema.into(), conn).await?.apply_all().await };
    // Announce progress periodically: obtaining the migration lock can block for
    // as long as another instance is migrating.
    let announce_future = async {
        loop {
            tokio::time::sleep(Duration::from_secs(10)).await;
            info!("database migration is still running");
        }
    };

    match future::select(pin!(migrate_future), pin!(announce_future)).await {
        Either::Left((result, _)) => result,
        Either::Right(_) => unreachable!("the announce future never finishes"),
    }
}

#[derive(Debug)]
struct Migrator<'a> {
    schema: String,
    tx: Transaction<'a>,
}

impl<'a> Migrator<'a> {
    async fn try_new(schema: String, conn: &'a mut Client) -> Result<Self, MigrationError> {
        debug!("starting a transaction");
        let tx = conn
            .transaction()
            .await
            .map_err(|source| migration_error::TransactionStart.into_error(source))?;

        Ok(Self { tx, schema })
    }

    async fn apply_all(self) -> Result<(), MigrationError> {
        self.try_lock_migrations().await?;
        self.try_create_migrations_table_if_needed().await?;

        let mut has_new_migrations = false;
        let mut migration_names_iter = self.try_read_migrations_table().await?.into_iter();
        has_new_migrations |= self.try_apply_migration_0001(migration_names_iter.next()).await?;
        has_new_migrations |= self.try_apply_migration_0002(migration_names_iter.next()).await?;

        if has_new_migrations {
            debug!("all migrations applied, committing the transaction");
            self.tx
                .commit()
                .await
                .map_err(|source| migration_error::TransactionCommit.into_error(source))?;
        } else {
            debug!("no migrations to apply, discarding the transaction");
            self.tx
                .rollback()
                .await
                .map_err(|source| migration_error::TransactionRollback.into_error(source))?;
        }

        Ok(())
    }

    async fn try_lock_migrations(&self) -> Result<(), MigrationError> {
        debug!("obtaining the migration lock");
        self.tx
            .execute("SELECT pg_advisory_xact_lock(0)", &[])
            .await
            .map_err(MigrationError::query_execute)?;

        Ok(())
    }

    async fn try_create_migrations_table_if_needed(&self) -> Result<(), MigrationError> {
        debug!("creating the migrations table");
        self.tx
            .execute(
                &format!(
                    "CREATE TABLE IF NOT EXISTS {}.{TABLE_MIGRATIONS}({COL_NAME} TEXT PRIMARY KEY)",
                    self.schema
                ),
                &[],
            )
            .await
            .map_err(MigrationError::query_execute)?;

        Ok(())
    }

    async fn try_read_migrations_table(&self) -> Result<Vec<String>, MigrationError> {
        debug!("reading the migration state");
        let rows = self
            .tx
            .query_raw(
                &format!(
                    "SELECT {COL_NAME} FROM {}.{TABLE_MIGRATIONS} ORDER BY {COL_NAME} ASC",
                    self.schema
                ),
                [] as [i32; 0],
            )
            .await
            .map_err(MigrationError::query_execute)?;

        rows.map_err(MigrationError::row_fetch)
            .and_then(|row| async move { row.try_get(0).map_err(MigrationError::row_fetch) })
            .try_collect::<Vec<String>>()
            .await
            .inspect(|migrations| debug!(?migrations, "finished reading the migrations table"))
    }

    async fn try_record_migration(&self, name: &str) -> Result<(), MigrationError> {
        debug!(?name, "inserting a migration record");
        self.tx
            .execute(
                &format!("INSERT INTO {}.{TABLE_MIGRATIONS}({COL_NAME}) VALUES ($1)", self.schema),
                &[&name],
            )
            .await
            .map_err(MigrationError::query_execute)?;

        Ok(())
    }

    async fn try_apply_migration_0001(&self, current: Option<String>) -> Result<bool, MigrationError> {
        self.try_apply_migration(current, "0001", async {
            self.tx
                .execute(
                    &format!(
                        "CREATE TABLE IF NOT EXISTS {}.metadata(\
                            resource_uri_hash BYTEA NOT NULL, \
                            resource_uri TEXT NOT NULL, \
                            metadata_key TEXT NOT NULL, \
                            metadata_value BYTEA NOT NULL, \
                            etag TEXT NOT NULL, \
                            PRIMARY KEY (resource_uri_hash, metadata_key)\
                        )",
                        self.schema
                    ),
                    &[],
                )
                .await
                .map_err(MigrationError::query_execute)?;

            Ok(())
        })
        .await
    }

    async fn try_apply_migration_0002(&self, current: Option<String>) -> Result<bool, MigrationError> {
        self.try_apply_migration(current, "0002", async {
            self.tx
                .execute(
                    &format!("ALTER TABLE {}.metadata ADD COLUMN namespace TEXT NOT NULL DEFAULT 'default'", self.schema),
                    &[],
                )
                .await
                .map_err(MigrationError::query_execute)?;
            self.tx
                .execute(
                    &format!("ALTER TABLE {}.metadata ADD CONSTRAINT metadata_unique_resource_namespace_key UNIQUE (resource_uri_hash, namespace, metadata_key)", self.schema),
                    &[]
                )
                .await
                .map_err(MigrationError::query_execute)?;
            self.tx
                .execute(
                    &format!("ALTER TABLE {}.metadata DROP CONSTRAINT metadata_pkey", self.schema),
                    &[],
                )
                .await
                .map_err(MigrationError::query_execute)?;

            Ok(())
        })
        .await
    }

    async fn try_apply_migration<F>(
        &self,
        actual: Option<String>,
        name: &'static str,
        fut: F,
    ) -> Result<bool, MigrationError>
    where
        F: Future<Output = Result<(), MigrationError>>,
    {
        if let Some(actual) = actual {
            return if actual != name {
                migration_error::UnexpectedMigration { actual, expected: name }.fail()
            } else {
                Ok(false)
            };
        }

        debug!(?name, "applying a migration");
        fut.await?;

        self.try_record_migration(name).await?;

        debug!(?name, "migration has been applied");

        Ok(true)
    }
}
