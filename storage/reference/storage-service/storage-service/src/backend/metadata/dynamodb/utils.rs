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

use aws_sdk_dynamodb::{
    operation::{
        batch_get_item::BatchGetItemError, delete_item::DeleteItemError, put_item::PutItemError, query::QueryError,
        update_item::UpdateItemError,
    },
    types::AttributeValue,
};
use prost::Message as _;
use snafu::IntoError;
use storage_service_protos::google::protobuf::Value;
use tracing::warn;
use uuid::Uuid;

use crate::backend::metadata::{dynamodb::error::InvalidRow, utils::Digest};

pub const COL_RESOURCE_URI_HASH: &str = "ResourceUriHash";
pub const COL_RESOURCE_URI: &str = "ResourceUri";
pub const COL_METADATA_KEY: &str = "MetadataKey";
pub const COL_METADATA_VALUE: &str = "MetadataValue";
pub const COL_ETAG: &str = "ETag";

pub const PAR_RESOURCE_URI_HASH: &str = ":ResourceUriHash";
pub const PAR_RESOURCE_URI: &str = ":ResourceUri";
pub const PAR_METADATA_VALUE: &str = ":MetadataValue";
pub const PAR_ETAG: &str = ":ETag";
pub const PAR_EXPECTED_ETAG: &str = ":ExpectedETag";

pub trait IsThrottlingErrorExt {
    fn is_throttled(&self) -> bool;
}

impl IsThrottlingErrorExt for BatchGetItemError {
    fn is_throttled(&self) -> bool {
        matches!(
            self,
            Self::RequestLimitExceeded(_)
                | Self::ProvisionedThroughputExceededException(_)
                | Self::ThrottlingException(_)
        )
    }
}

impl IsThrottlingErrorExt for QueryError {
    fn is_throttled(&self) -> bool {
        matches!(
            self,
            Self::RequestLimitExceeded(_)
                | Self::ProvisionedThroughputExceededException(_)
                | Self::ThrottlingException(_)
        )
    }
}

impl IsThrottlingErrorExt for PutItemError {
    fn is_throttled(&self) -> bool {
        matches!(
            self,
            Self::RequestLimitExceeded(_)
                | Self::ProvisionedThroughputExceededException(_)
                | Self::ThrottlingException(_)
        )
    }
}

impl IsThrottlingErrorExt for DeleteItemError {
    fn is_throttled(&self) -> bool {
        matches!(
            self,
            Self::RequestLimitExceeded(_)
                | Self::ProvisionedThroughputExceededException(_)
                | Self::ThrottlingException(_)
        )
    }
}

impl IsThrottlingErrorExt for UpdateItemError {
    fn is_throttled(&self) -> bool {
        matches!(
            self,
            Self::RequestLimitExceeded(_)
                | Self::ProvisionedThroughputExceededException(_)
                | Self::ThrottlingException(_)
        )
    }
}

/// A query result.
pub struct Row {
    pub metadata_key: String,
    pub metadata_value: Value,
    pub etag: String,
    pub resource_uri: String,
}

impl Row {
    /// Attempt to parse a row from a query result.
    pub fn try_from_row(row: impl IntoIterator<Item = (String, AttributeValue)>) -> Result<Self, InvalidRow> {
        use super::error::invalid_row::ValueDecodeFailedSnafu;

        let mut maybe_key = None;
        let mut maybe_value = None;
        let mut maybe_etag = None;
        let mut maybe_resource = None;
        for (column_key, column_value) in row.into_iter() {
            match column_key {
                k if k == COL_METADATA_KEY => {
                    if let AttributeValue::S(v) = column_value {
                        maybe_key = Some(v);
                    } else {
                        return Err(InvalidRow::column_value_type_invalid(COL_METADATA_VALUE));
                    }
                },
                k if k == COL_METADATA_VALUE => {
                    if let AttributeValue::B(v) = column_value {
                        maybe_value = Some(
                            Value::decode(v.as_ref()).map_err(|source| ValueDecodeFailedSnafu.into_error(source))?,
                        );
                    } else {
                        return Err(InvalidRow::column_value_type_invalid(COL_METADATA_VALUE));
                    }
                },
                k if k == COL_ETAG => {
                    if let AttributeValue::S(v) = column_value {
                        maybe_etag = Some(v);
                    } else {
                        return Err(InvalidRow::column_value_type_invalid(COL_ETAG));
                    }
                },
                k if k == COL_RESOURCE_URI => {
                    if let AttributeValue::S(v) = column_value {
                        maybe_resource = Some(v);
                    } else {
                        return Err(InvalidRow::column_value_type_invalid(COL_RESOURCE_URI));
                    }
                },
                k => {
                    warn!(column = ?k, "unexpected column value returned in a response");
                },
            }
        }

        Ok(Self {
            metadata_key: maybe_key.ok_or_else(|| InvalidRow::column_missing(COL_METADATA_KEY))?,
            metadata_value: maybe_value.ok_or_else(|| InvalidRow::column_missing(COL_METADATA_VALUE))?,
            etag: maybe_etag.ok_or_else(|| InvalidRow::column_missing(COL_ETAG))?,
            resource_uri: maybe_resource.ok_or_else(|| InvalidRow::column_missing(COL_RESOURCE_URI))?,
        })
    }
}

/// Generate a random etag value.
pub fn create_random_etag() -> String {
    format!("\"{}\"", Uuid::new_v4())
}

/// Create a query attribute for the specified metadata key.
pub fn get_metadata_key_attribute(value: impl Into<String>) -> (String, AttributeValue) {
    (COL_METADATA_KEY.into(), AttributeValue::S(value.into()))
}

/// Create a query attribute for the specified resource.
pub fn get_resource_uri_hash_attribute(value: &Digest) -> (String, AttributeValue) {
    (
        COL_RESOURCE_URI_HASH.into(),
        AttributeValue::S(value.to_digest_string()),
    )
}

/// Create a projection expression for a row.
pub fn get_row_projection_expression() -> String {
    [COL_RESOURCE_URI, COL_METADATA_KEY, COL_METADATA_VALUE, COL_ETAG].join(", ")
}
