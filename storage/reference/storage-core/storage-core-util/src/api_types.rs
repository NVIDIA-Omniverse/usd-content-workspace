// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use crate::error::Error;
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use storage_core_protos::google;

/// Build a FileObject-native-metadata map from raw backend string key/values.
/// Backend object metadata (S3 `x-amz-meta-*`, Azure blob metadata) is a string
/// map already present on stat/read/write responses; each value is wrapped as a
/// protobuf string `Value`. Callers pass metadata the backend already returned,
/// so this adds no extra request.
/// Approximate serialized byte size of a native-metadata map, for cache weighing. Values are
/// client-controlled, so this must be counted against cache byte budgets.
pub fn native_metadata_size(map: &std::collections::HashMap<String, google::protobuf::Value>) -> usize {
    use google::protobuf::value::Kind;
    map.iter()
        .map(|(k, v)| {
            let value_len = match &v.kind {
                Some(Kind::StringValue(s)) => s.len(),
                _ => std::mem::size_of::<google::protobuf::Value>(),
            };
            k.len() + value_len
        })
        .sum()
}

pub fn native_metadata_from_strings<I>(entries: I) -> std::collections::HashMap<String, google::protobuf::Value>
where
    I: IntoIterator<Item = (String, String)>,
{
    entries
        .into_iter()
        .map(|(k, v)| {
            (
                k,
                google::protobuf::Value {
                    kind: Some(google::protobuf::value::Kind::StringValue(v)),
                },
            )
        })
        .collect()
}

/// The ResourceInfo message contains the information about a single data object
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceInfo {
    /// Encoded identity of the data object
    pub encoded_identity: String,
    /// Size of the data object
    pub data_object_size: Option<u64>,
    /// Last modified timestamp of the data object
    #[serde(with = "crate::serde_helpers::option_timestamp")]
    pub last_modified_timestamp: Option<google::protobuf::Timestamp>,
    /// FileObject-native metadata (backend key/values, e.g. `x-amz-meta-*` /
    /// blob metadata) returned on the free stat/read/write paths. Empty unless
    /// the backend has native metadata enabled and the object carries entries.
    #[serde(default, with = "crate::serde_helpers::native_metadata_map")]
    pub native_metadata: std::collections::HashMap<String, google::protobuf::Value>,
}

#[derive(Debug, Clone)]
pub struct AddressInfo {
    /// Resource address of the  object
    pub resource_address: String,
    /// Size of the data object
    pub data_object_size: Option<u64>,
    /// Last modified timestamp of the data object
    pub last_modified_timestamp: Option<google::protobuf::Timestamp>,
}

/// Information about a redirect
#[derive(Debug, Clone)]
pub struct Redirect {
    /// URL that can be used to retrieve the data object
    pub redirect_target_url: String,
    /// Method type to access, case-insensitive
    pub method: String,
    /// Pack of headers that client should use to access cloud storage
    pub additional_headers: Vec<Header>,
}

/// HTTP header
#[derive(Debug, Clone)]
pub struct Header {
    /// Name of the header
    pub name: String,
    /// Value of the header
    pub value: String,
}

#[derive(Debug, Clone)]
pub enum ReadPart {
    // Metadata
    ResourceInfo(ResourceInfo),
    // Chunk of data to send
    Chunk(Bytes),
    // Redirect for presigned URL
    Redirect(Redirect),
}

#[derive(Debug, Clone)]
pub struct CompletedUploadPart {
    pub part_number: u32,
    pub headers: Vec<Header>,
}

#[derive(Debug, Clone)]
pub enum WritePart {
    // Write Chunks Accepted
    WriteChunksAccepted,
    // Resource Identity and Metadata
    ResourceInfo(ResourceInfo),
    // Redirect for presigned URL
    Redirect(WriteRedirect),
    // Multipart Upload
    MultipartUpload(MultipartUpload),
}

#[derive(Debug, Clone)]
pub enum UploadMethod {
    Unspecified,
    Post,
    Put,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadPreference {
    Unspecified,
    Body,
    Redirect,
}

impl From<i32> for DownloadPreference {
    fn from(value: i32) -> Self {
        match value {
            0 => DownloadPreference::Unspecified,
            1 => DownloadPreference::Body,
            2 => DownloadPreference::Redirect,
            _ => DownloadPreference::Unspecified,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UploadPreference {
    Unspecified,
    Body,
    Redirect,
    Multipart,
}

impl From<i32> for UploadPreference {
    fn from(value: i32) -> Self {
        match value {
            0 => UploadPreference::Unspecified,
            1 => UploadPreference::Body,
            2 => UploadPreference::Redirect,
            3 => UploadPreference::Multipart,
            _ => UploadPreference::Unspecified,
        }
    }
}

#[derive(Debug, Clone)]
pub struct WriteRedirect {
    pub redirect_target_url: String,
    pub method: UploadMethod,
    pub additional_headers: Vec<Header>,
    pub completion_header_names: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct MultipartUpload {
    pub upload_id: String,
    pub first_part_write_redirect: WriteRedirect,
    pub maximum_parts_number: Option<u32>,
    pub minimum_size_per_part: Option<u64>,
    pub maximum_size_per_part: Option<u64>,
}

impl From<UploadMethod> for i32 {
    fn from(value: UploadMethod) -> Self {
        match value {
            UploadMethod::Unspecified => 0,
            UploadMethod::Post => 1,
            UploadMethod::Put => 2,
        }
    }
}

pub mod v1alpha {
    use storage_core_protos::nvidia::omniverse::storage::fileobject::v1alpha::{
        AddressInfo, CompletedUploadPart, CreateMultipartUploadResponse, Header, Metadata, Redirect, ResourceIdentity,
        ResourceInfo, WriteRedirectProperties,
    };

    impl From<super::ResourceInfo> for ResourceInfo {
        fn from(value: super::ResourceInfo) -> Self {
            Self {
                resource_identity: Some(ResourceIdentity {
                    encoded_identity: value.encoded_identity,
                }),
                metadata: Some(Metadata {
                    data_object_size: value.data_object_size,
                    last_modified_timestamp: value.last_modified_timestamp,
                    native_metadata: value.native_metadata,
                }),
            }
        }
    }

    impl TryFrom<ResourceInfo> for super::ResourceInfo {
        type Error = super::Error;
        fn try_from(value: ResourceInfo) -> Result<Self, Self::Error> {
            let encoded_identity = value
                .resource_identity
                .map(|r| r.encoded_identity)
                .ok_or_else(|| super::Error::internal("Resource info has no resource identity"))?;
            let (data_object_size, last_modified_timestamp, native_metadata) = match value.metadata {
                Some(m) => (m.data_object_size, m.last_modified_timestamp, m.native_metadata),
                None => (None, None, Default::default()),
            };
            Ok(Self {
                encoded_identity,
                data_object_size,
                last_modified_timestamp,
                native_metadata,
            })
        }
    }

    impl From<super::AddressInfo> for AddressInfo {
        fn from(value: super::AddressInfo) -> Self {
            Self {
                resource_address: value.resource_address,
                metadata: Some(Metadata {
                    data_object_size: value.data_object_size,
                    last_modified_timestamp: value.last_modified_timestamp,
                    native_metadata: Default::default(),
                }),
            }
        }
    }

    impl From<super::ResourceInfo> for Metadata {
        fn from(value: super::ResourceInfo) -> Self {
            Self {
                data_object_size: value.data_object_size,
                last_modified_timestamp: value.last_modified_timestamp,
                native_metadata: value.native_metadata,
            }
        }
    }

    #[allow(deprecated)]
    impl From<super::Redirect> for Redirect {
        fn from(value: super::Redirect) -> Self {
            Self {
                redirect_target_url: value.redirect_target_url,
                method: value.method,
                additional_headers: value
                    .additional_headers
                    .into_iter()
                    .map(|h| Header {
                        name: h.name,
                        value: h.value,
                    })
                    .collect(),
            }
        }
    }

    impl From<super::Header> for Header {
        fn from(value: super::Header) -> Self {
            Self {
                name: value.name,
                value: value.value,
            }
        }
    }

    impl From<Header> for super::Header {
        fn from(value: Header) -> Self {
            Self {
                name: value.name,
                value: value.value,
            }
        }
    }

    impl From<super::WriteRedirect> for WriteRedirectProperties {
        fn from(value: super::WriteRedirect) -> Self {
            Self {
                redirect_target_url: value.redirect_target_url,
                method: value.method.into(),
                additional_headers: value.additional_headers.into_iter().map(|h| h.into()).collect(),
                completion_header_names: value.completion_header_names,
            }
        }
    }

    impl From<super::MultipartUpload> for CreateMultipartUploadResponse {
        fn from(value: super::MultipartUpload) -> Self {
            Self {
                upload_id: value.upload_id,
                first_part_write_redirect: Some(value.first_part_write_redirect.into()),
                maximum_parts_number: value.maximum_parts_number,
                minimum_size_per_part: value.minimum_size_per_part,
                maximum_size_per_part: value.maximum_size_per_part,
            }
        }
    }

    impl From<CompletedUploadPart> for super::CompletedUploadPart {
        fn from(value: CompletedUploadPart) -> Self {
            Self {
                part_number: value.part_number,
                headers: value.headers.into_iter().map(|h| h.into()).collect(),
            }
        }
    }
}

pub mod v1beta {
    use storage_core_protos::nvidia::omniverse::storage::fileobject::v1beta::{
        AddressInfo, CompletedUploadPart, CreateMultipartUploadResponse, Header, Metadata, Redirect, ResourceIdentity,
        ResourceInfo, WriteRedirectProperties,
    };

    impl From<super::ResourceInfo> for ResourceInfo {
        fn from(value: super::ResourceInfo) -> Self {
            Self {
                resource_identity: Some(ResourceIdentity {
                    encoded_identity: value.encoded_identity,
                }),
                metadata: Some(Metadata {
                    data_object_size: value.data_object_size,
                    last_modified_timestamp: value.last_modified_timestamp,
                }),
            }
        }
    }

    impl TryFrom<ResourceInfo> for super::ResourceInfo {
        type Error = super::Error;
        fn try_from(value: ResourceInfo) -> Result<Self, Self::Error> {
            Ok(Self {
                encoded_identity: value
                    .resource_identity
                    .map(|r| r.encoded_identity)
                    .ok_or_else(|| super::Error::internal("Resource info has no resource identity"))?,
                data_object_size: value.metadata.and_then(|m| m.data_object_size),
                last_modified_timestamp: value.metadata.and_then(|m| m.last_modified_timestamp),
                native_metadata: Default::default(),
            })
        }
    }

    impl From<super::AddressInfo> for AddressInfo {
        fn from(value: super::AddressInfo) -> Self {
            Self {
                resource_address: value.resource_address,
                metadata: Some(Metadata {
                    data_object_size: value.data_object_size,
                    last_modified_timestamp: value.last_modified_timestamp,
                }),
            }
        }
    }

    impl From<super::ResourceInfo> for Metadata {
        fn from(value: super::ResourceInfo) -> Self {
            Self {
                data_object_size: value.data_object_size,
                last_modified_timestamp: value.last_modified_timestamp,
            }
        }
    }

    #[allow(deprecated)]
    impl From<super::Redirect> for Redirect {
        fn from(value: super::Redirect) -> Self {
            Self {
                redirect_target_url: value.redirect_target_url,
                method: value.method,
                additional_headers: value
                    .additional_headers
                    .into_iter()
                    .map(|h| Header {
                        name: h.name,
                        value: h.value,
                    })
                    .collect(),
            }
        }
    }

    impl From<super::Header> for Header {
        fn from(value: super::Header) -> Self {
            Self {
                name: value.name,
                value: value.value,
            }
        }
    }

    impl From<Header> for super::Header {
        fn from(value: Header) -> Self {
            Self {
                name: value.name,
                value: value.value,
            }
        }
    }

    impl From<super::WriteRedirect> for WriteRedirectProperties {
        fn from(value: super::WriteRedirect) -> Self {
            Self {
                redirect_target_url: value.redirect_target_url,
                method: value.method.into(),
                additional_headers: value.additional_headers.into_iter().map(|h| h.into()).collect(),
                completion_header_names: value.completion_header_names,
            }
        }
    }

    impl From<super::MultipartUpload> for CreateMultipartUploadResponse {
        fn from(value: super::MultipartUpload) -> Self {
            Self {
                upload_id: value.upload_id,
                first_part_write_redirect: Some(value.first_part_write_redirect.into()),
                maximum_parts_number: value.maximum_parts_number,
                minimum_size_per_part: value.minimum_size_per_part,
                maximum_size_per_part: value.maximum_size_per_part,
            }
        }
    }

    impl From<CompletedUploadPart> for super::CompletedUploadPart {
        fn from(value: CompletedUploadPart) -> Self {
            Self {
                part_number: value.part_number,
                headers: value.headers.into_iter().map(|h| h.into()).collect(),
            }
        }
    }
}
