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

use azure_core::StatusCode;
use azure_data_cosmos::{
    prelude::{Param, Query},
    CosmosEntity,
};
use base64::{prelude::BASE64_STANDARD_NO_PAD, Engine};
use prost::Message;
use serde::{Deserialize, Serialize};
use snafu::IntoError;
use storage_service_protos::google::protobuf::Value;

use crate::backend::metadata::{
    azure_cosmosdb::error::{
        metadata_value_decode_error, Error, MetadataValueDecodeError, MetadataValueDecodeFailedSnafu,
    },
    types::{KeyValue, NamedKeyValue},
    utils::Digest,
};

const PROPERTY_RESOURCE_URI_HASH: &str = "resourceUriHash";
const PROPERTY_RESOURCE_URI: &str = "resourceUri";
const PROPERTY_METADATA_KEY: &str = "metadataKey";
const PROPERTY_METADATA_VALUE: &str = "metadataValue";
const PROPERTY_ETAG: &str = "_etag";

/// An object to insert in a table.
#[derive(Debug, Serialize)]
pub struct InsertDocument {
    #[serde(rename = "resourceUriHash")]
    pub resource_uri_hash: String,
    #[serde(rename = "id")]
    pub metadata_key_hash: String,
    #[serde(rename = "resourceUri")]
    pub resource_uri: String,
    #[serde(rename = "metadataKey")]
    pub metadata_key: String,
    #[serde(rename = "metadataValue")]
    pub metadata_value: String,
}

impl CosmosEntity for InsertDocument {
    type Entity = String;

    fn partition_key(&self) -> Self::Entity {
        self.resource_uri_hash.clone()
    }
}

/// A document returned from a partition query call.
#[derive(Debug, Deserialize)]
pub struct PartitionQueryDocument {
    #[serde(rename = "metadataKey")]
    pub metadata_key: String,
    #[serde(rename = "metadataValue")]
    pub metadata_value: String,
    #[serde(rename = "_etag")]
    pub etag: String,
}

impl TryFrom<PartitionQueryDocument> for NamedKeyValue {
    type Error = Error;

    #[track_caller]
    fn try_from(value: PartitionQueryDocument) -> Result<Self, Self::Error> {
        Ok(NamedKeyValue {
            key: value.metadata_key,
            value: KeyValue {
                etag: value.etag,
                value: decode_user_metadata_value(&value.metadata_value)
                    .map_err(|source| MetadataValueDecodeFailedSnafu.into_error(source))?,
            },
        })
    }
}

/// A document returned from a get call.
#[derive(Debug, Deserialize)]
pub struct GetOneDocument {
    #[serde(rename = "resourceUri")]
    pub resource_uri: String,
    #[serde(rename = "metadataKey")]
    pub metadata_key: String,
    #[serde(rename = "metadataValue")]
    pub metadata_value: String,
}

/// A document returned from a find one call.
#[derive(Debug, Deserialize)]
pub struct FindOneDocument {
    #[serde(rename = "resourceUri")]
    pub resource_uri: String,
    #[serde(rename = "metadataKey")]
    pub metadata_key: String,
}

/// HttpError extension methods.
pub trait HttpErrorExt {
    fn is_failed_precondition(&self) -> bool;

    fn is_not_found(&self) -> bool;

    fn is_conflict(&self) -> bool;
}

impl HttpErrorExt for azure_core::error::HttpError {
    fn is_failed_precondition(&self) -> bool {
        self.status() == StatusCode::PreconditionFailed
    }

    fn is_not_found(&self) -> bool {
        self.status() == StatusCode::NotFound
    }

    fn is_conflict(&self) -> bool {
        self.status() == StatusCode::Conflict
    }
}

/// Encode a metadata entry value.
#[track_caller]
pub fn encode_user_metadata_value(value: &Value) -> Result<String, prost::EncodeError> {
    let mut buffer = Vec::with_capacity(value.encoded_len());
    value.encode(&mut buffer)?;

    Ok(BASE64_STANDARD_NO_PAD.encode(buffer.as_slice()))
}

/// Decode a metadata entry value.
#[track_caller]
pub fn decode_user_metadata_value(value: &str) -> Result<Value, MetadataValueDecodeError> {
    let decoded_value = BASE64_STANDARD_NO_PAD
        .decode(value.as_bytes())
        .map_err(|source| metadata_value_decode_error::Base64DecodeFailedSnafu.into_error(source))?;

    Value::decode(decoded_value.as_slice())
        .map_err(|source| metadata_value_decode_error::ProstDecodeFailedSnafu.into_error(source))
}

/// Create a query to fetch all documents from a partition.
pub fn create_partition_fetch_all_query(resource_uri: String, resource_uri_hash: &Digest) -> Query {
    Query::with_params(
        format!(
            "SELECT d.{}, d.{}, d.{} FROM d WHERE d.{} = @u AND d.{} = @uh",
            PROPERTY_METADATA_KEY,
            PROPERTY_METADATA_VALUE,
            PROPERTY_ETAG,
            PROPERTY_RESOURCE_URI,
            PROPERTY_RESOURCE_URI_HASH,
        ),
        [
            create_param("@u", resource_uri),
            create_param("@uh", resource_uri_hash.to_digest_string()),
        ],
    )
}

/// Create a query parameter.
fn create_param(name: impl Into<String>, value: impl Into<serde_json::Value>) -> Param {
    Param::new(name.into(), value.into())
}
