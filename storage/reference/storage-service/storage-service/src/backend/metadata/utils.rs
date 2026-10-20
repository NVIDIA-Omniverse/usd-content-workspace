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

use arrayvec::ArrayString;
use base64::{display::Base64Display, engine::general_purpose::STANDARD, Engine};
use bytes::{Bytes, BytesMut};
use percent_encoding::percent_decode_str;
use prost::{DecodeError, EncodeError, Message};
use serde::Serialize;
use snafu::IntoError;
use std::str::FromStr;
use storage_service_protos::google::protobuf::Value;
use url::Url;

use crate::{
    backend::metadata::error::GetMetadataKeyError, common::header::is_valid_etag, error::InvalidArgumentError,
};

const DIGEST_BYTE_STRING_PREFIX_LEN: usize = 7;
pub const DIGEST_BYTE_STRING_LEN: usize = DIGEST_BYTE_STRING_PREFIX_LEN + blake3::OUT_LEN;

pub const QUERY_PARAMETER_ETAG: &str = "etag";

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq)]
pub struct Digest(blake3::Hash);

impl Digest {
    pub fn compute(value: impl AsRef<[u8]>) -> Self {
        Self(blake3::hash(value.as_ref()))
    }

    #[allow(clippy::wrong_self_convention)]
    pub fn to_digest_string(&self) -> String {
        self.to_digest_array_string().as_str().into()
    }

    pub fn to_digest_byte_string(&self) -> [u8; DIGEST_BYTE_STRING_LEN] {
        let mut bytes = [0u8; DIGEST_BYTE_STRING_LEN];
        bytes[..DIGEST_BYTE_STRING_PREFIX_LEN].copy_from_slice(b"blake3:");
        bytes[DIGEST_BYTE_STRING_PREFIX_LEN..].copy_from_slice(self.0.as_bytes());

        bytes
    }

    #[allow(clippy::wrong_self_convention)]
    fn to_digest_array_string(&self) -> ArrayString<71> {
        let digest = self.0.to_hex();

        let mut result = ArrayString::new();
        result.push_str("blake3:");
        result.push_str(&digest);

        result
    }
}

impl Serialize for Digest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let digest = self.to_digest_array_string();
        serializer.serialize_str(&digest)
    }
}

pub fn get_object_key(key_prefix: &str, key: &str, entry_name: &str) -> String {
    format!(
        "{}{}/{}",
        key_prefix,
        Base64Display::new(key.as_bytes(), &STANDARD),
        Base64Display::new(entry_name.as_bytes(), &STANDARD)
    )
}

pub fn get_object_key_prefix(key_prefix: &str, key: &str) -> String {
    format!("{}{}/", key_prefix, Base64Display::new(key.as_bytes(), &STANDARD))
}

pub fn get_entry_name(object_key_prefix: &str, object_key: &str) -> Option<String> {
    let key_suffix = object_key.strip_prefix(object_key_prefix)?;

    let Ok(name_bytes) = STANDARD.decode(key_suffix) else {
        return None;
    };

    let Ok(name) = String::from_utf8(name_bytes) else {
        return None;
    };

    Some(name)
}

/// Encode a metadata value into a byte buffer.
#[track_caller]
pub fn encode_value(value: &Value) -> Result<Bytes, EncodeError> {
    let mut buffer = BytesMut::with_capacity(value.encoded_len());
    value.encode(&mut buffer)?;

    Ok(buffer.into())
}

/// Decode a metadata value from a byte buffer.
#[track_caller]
pub fn decode_value(data: &[u8]) -> Result<Value, DecodeError> {
    Value::decode(data)
}

/// Create a metadata key for the specified uri.
#[track_caller]
pub fn get_metadata_key(uri: &str) -> Result<String, GetMetadataKeyError> {
    use crate::backend::metadata::error::get_metadata_key_error::*;

    let url = Url::from_str(uri).map_err(|source| UrlDecodeSnafu.into_error(source))?;
    let decoded_path = percent_decode_str(url.path())
        .decode_utf8()
        .map_err(|source| PathDecodeSnafu.into_error(source))?;
    let maybe_etag = url
        .query_pairs()
        .filter_map(|(key, value)| if key == QUERY_PARAMETER_ETAG { Some(value) } else { None })
        .next();
    let maybe_decoded_etag = if let Some(value) = maybe_etag.as_ref() {
        Some(
            percent_decode_str(value)
                .decode_utf8()
                .map_err(|source| ETagDecodeSnafu.into_error(source))?,
        )
    } else {
        None
    };

    Ok(format!(
        "{}",
        MetadataKeyRef {
            scheme: url.scheme(),
            authority: url.authority(),
            path: &decoded_path,
            etag: maybe_decoded_etag.as_ref().map(|etag| etag.as_ref())
        }
    ))
}

/// Validate an expected etag value.
pub fn into_valid_etag(value: Option<String>) -> Result<Option<String>, InvalidArgumentError> {
    if let Some(value) = value.as_deref() {
        if !is_valid_etag(value) {
            return Err(InvalidArgumentError::new("expected etag value is invalid"));
        }
    }

    Ok(value)
}

struct MetadataKeyRef<'a> {
    scheme: &'a str,
    authority: &'a str,
    path: &'a str,
    etag: Option<&'a str>,
}

impl std::fmt::Display for MetadataKeyRef<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:02x}{}{:04x}{}{:04x}{}",
            self.scheme.len(),
            self.scheme,
            self.authority.len(),
            self.authority,
            self.path.len(),
            self.path
        )?;
        if let Some(etag) = self.etag {
            write!(f, "{:04x}{}", etag.len(), etag)?;
        }

        Ok(())
    }
}
