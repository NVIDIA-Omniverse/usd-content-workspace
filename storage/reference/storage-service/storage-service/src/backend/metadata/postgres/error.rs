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

use snafu::{IntoError, Location, Snafu};
use storage_core::traits::StorageErrorExt;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub), module, context(suffix(false)))]
pub enum QueryError {
    #[snafu(display("failed to execute a statement"))]
    Execute {
        source: tokio_postgres::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to read a row"))]
    RowRead {
        source: tokio_postgres::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to parse a row"))]
    RowParse {
        source: tokio_postgres::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },
}

impl QueryError {
    #[track_caller]
    pub fn execute(source: tokio_postgres::Error) -> Self {
        query_error::Execute.into_error(source)
    }

    #[track_caller]
    pub fn row_read(source: tokio_postgres::Error) -> Self {
        query_error::RowRead.into_error(source)
    }

    #[track_caller]
    pub fn row_parse(source: tokio_postgres::Error) -> Self {
        query_error::RowParse.into_error(source)
    }
}

#[derive(Debug, Snafu)]
#[snafu(visibility(pub), module, context(suffix(false)))]
pub enum MigrationError {
    #[snafu(display("failed to acquire a connection"))]
    ConnectionAcquire {
        source: deadpool_postgres::PoolError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to start a transaction"))]
    TransactionStart {
        source: tokio_postgres::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to commit a transaction"))]
    TransactionCommit {
        source: tokio_postgres::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to rollback a transaction"))]
    TransactionRollback {
        source: tokio_postgres::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to execute a query"))]
    QueryExecute {
        source: tokio_postgres::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to fetch a row"))]
    RowFetch {
        source: tokio_postgres::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("unexpected migration {actual:?} (expected {expected:?})"))]
    UnexpectedMigration {
        actual: String,
        expected: &'static str,
        #[snafu(provide, implicit)]
        location: Location,
    },
}

impl MigrationError {
    #[track_caller]
    pub fn query_execute(source: tokio_postgres::Error) -> Self {
        migration_error::QueryExecute.into_error(source)
    }

    #[track_caller]
    pub fn row_fetch(source: tokio_postgres::Error) -> Self {
        migration_error::RowFetch.into_error(source)
    }
}

#[derive(Debug, Snafu)]
#[snafu(visibility(pub), module, context(suffix(false)))]
pub enum Error {
    #[snafu(display("failed to acquire a connection"))]
    ConnectionAcquire {
        source: deadpool_postgres::PoolError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to execute a query"))]
    Query {
        source: QueryError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to encode metadata value"))]
    MetadataEncode {
        source: prost::EncodeError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to decode metadata value"))]
    MetadataDecode {
        source: prost::DecodeError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("metadata was modified"))]
    MetadataModified {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("collision detected"))]
    Collision {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("resource location is missing"))]
    ResourceLocationMissing {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("user metadata value is missing"))]
    UserMetadataValueMissing {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("metadata key not found"))]
    MetadataKeyNotFound {
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("metadata value is too large (actual {actual_len}, limit {max_len})"))]
    UserMetadataValueTooLarge {
        actual_len: usize,
        max_len: usize,
        #[snafu(provide, implicit)]
        location: Location,
    },
}

impl StorageErrorExt for Error {
    fn get_size(&self) -> usize {
        0
    }

    fn http_status(&self) -> axum::http::StatusCode {
        use axum::http::StatusCode;

        match self {
            Self::ConnectionAcquire { .. }
            | Self::Query { .. }
            | Self::MetadataEncode { .. }
            | Self::MetadataDecode { .. }
            | Self::Collision { .. } => StatusCode::INTERNAL_SERVER_ERROR,
            Self::MetadataModified { .. } => StatusCode::PRECONDITION_FAILED,
            Self::ResourceLocationMissing { .. }
            | Self::UserMetadataValueMissing { .. }
            | Self::UserMetadataValueTooLarge { .. } => StatusCode::BAD_REQUEST,
            Self::MetadataKeyNotFound { .. } => StatusCode::NOT_FOUND,
        }
    }

    fn tonic_status(&self) -> tonic::Status {
        use tonic::Status;

        match self {
            Self::ConnectionAcquire { .. }
            | Self::Query { .. }
            | Self::MetadataEncode { .. }
            | Self::MetadataDecode { .. }
            | Self::Collision { .. } => Status::internal("unexpected error"),
            Self::MetadataModified { .. } => Status::failed_precondition(self.to_string()),
            Self::ResourceLocationMissing { .. }
            | Self::UserMetadataValueMissing { .. }
            | Self::UserMetadataValueTooLarge { .. } => Status::invalid_argument(self.to_string()),
            Self::MetadataKeyNotFound { .. } => Status::not_found(self.to_string()),
        }
    }
}

#[derive(Snafu, Debug)]
#[snafu(visibility(pub), module, context(suffix(false)))]
pub enum InitError {
    #[snafu(display("failed to init the tls connector"))]
    TlsInit {
        source: native_tls::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to create the connection pool"))]
    PoolCreate {
        source: deadpool_postgres::CreatePoolError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to acquire a connection"))]
    ConnectionAcquire {
        source: deadpool_postgres::PoolError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to apply database migrations"))]
    Migrate {
        source: MigrationError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to start a transaction"))]
    TransactionStart {
        source: tokio_postgres::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to commit a transaction"))]
    TransactionCommit {
        source: tokio_postgres::Error,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to init the database"))]
    DatabaseInit {
        source: QueryError,
        #[snafu(provide, implicit)]
        location: Location,
    },

    #[snafu(display("failed to fetch client credentials"))]
    ClientCredentialsFetch {
        source: Box<dyn std::error::Error + Send + Sync>,
        #[snafu(provide, implicit)]
        location: Location,
    },
}
