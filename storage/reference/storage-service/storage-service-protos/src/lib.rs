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

pub mod google {
    pub mod protobuf {
        tonic::include_proto!("event-publisher-v1beta/google.protobuf");
    }
}

pub mod nvidia {
    pub mod omniverse {
        pub mod discovery {
            pub mod capabilities {
                pub mod v2alpha {
                    pub const FILE_DESCRIPTOR_SET: &[u8] =
                        tonic::include_file_descriptor_set!("discovery-capabilities-v2alpha/v2alpha");
                    tonic::include_proto!(
                        "discovery-capabilities-v2alpha/nvidia.omniverse.discovery.capabilities.v2alpha"
                    );
                }
            }
        }
        pub mod permission {
            pub mod v1beta {
                tonic::include_proto!("permissions-v1beta/nvidia.omniverse.permission.v1beta");
            }
        }
        pub mod storage {
            pub use storage_core_protos::nvidia::omniverse::storage::filefolder;
            pub use storage_core_protos::nvidia::omniverse::storage::fileobject;

            pub mod metadata {

                pub mod v1alpha {
                    pub const FILE_DESCRIPTOR_SET: &[u8] =
                        tonic::include_file_descriptor_set!("metadata-v1alpha/v1alpha");
                    tonic::include_proto!("metadata-v1alpha/nvidia.omniverse.storage.metadata.v1alpha");
                }

                pub mod v1testmetadata {
                    pub const FILE_DESCRIPTOR_SET: &[u8] =
                        tonic::include_file_descriptor_set!("metadata-v1testmetadata/v1testmetadata");
                    tonic::include_proto!("metadata-v1testmetadata/nvidia.omniverse.storage.metadata.v1testmetadata");
                }
            }
        }
        pub mod notifications {
            pub mod publisher {
                pub mod v1beta {
                    pub const FILE_DESCRIPTOR_SET: &[u8] =
                        tonic::include_file_descriptor_set!("event-publisher-v1beta/v1beta");
                    tonic::include_proto!("event-publisher-v1beta/nvidia.omniverse.notifications.publisher.v1beta");
                }
            }
        }
        pub mod metadata {
            pub mod v1beta {
                pub const FILE_DESCRIPTOR_SET: &[u8] =
                    tonic::include_file_descriptor_set!("metadata-api-v1beta/v1beta");
                tonic::include_proto!("metadata-api-v1beta/nvidia.omniverse.metadata.v1beta");
            }
        }
    }
}

use serde::{
    de::Deserializer,
    ser::{Error, Serializer},
    Deserialize, Serialize,
};
use utoipa::{PartialSchema, ToSchema};

use std::collections::HashMap;

impl Serialize for crate::google::protobuf::Value {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use crate::google::protobuf::value::Kind;
        match &self.kind {
            Some(Kind::NullValue(_)) => serializer.serialize_none(),
            Some(Kind::NumberValue(n)) => serializer.serialize_f64(*n),
            Some(Kind::StringValue(s)) => serializer.serialize_str(s),
            Some(Kind::BoolValue(b)) => serializer.serialize_bool(*b),
            Some(Kind::StructValue(s)) => s.serialize(serializer),
            Some(Kind::ListValue(l)) => l.serialize(serializer),
            None => serializer.serialize_none(),
        }
    }
}

impl<'de> Deserialize<'de> for crate::google::protobuf::Value {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use crate::google::protobuf::value::Kind;
        use serde::de::Error;

        struct ValueVisitor;

        impl<'de> serde::de::Visitor<'de> for ValueVisitor {
            type Value = crate::google::protobuf::Value;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a valid JSON value")
            }

            fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E>
            where
                E: Error,
            {
                Ok(crate::google::protobuf::Value {
                    kind: Some(Kind::BoolValue(v)),
                })
            }

            fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
            where
                E: Error,
            {
                Ok(crate::google::protobuf::Value {
                    kind: Some(Kind::NumberValue(v as f64)),
                })
            }

            fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
            where
                E: Error,
            {
                Ok(crate::google::protobuf::Value {
                    kind: Some(Kind::NumberValue(v as f64)),
                })
            }

            fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E>
            where
                E: Error,
            {
                Ok(crate::google::protobuf::Value {
                    kind: Some(Kind::NumberValue(v)),
                })
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: Error,
            {
                Ok(crate::google::protobuf::Value {
                    kind: Some(Kind::StringValue(v.to_owned())),
                })
            }

            fn visit_none<E>(self) -> Result<Self::Value, E>
            where
                E: Error,
            {
                Ok(crate::google::protobuf::Value {
                    kind: Some(Kind::NullValue(0)), // 0 is the default enum value for NullValue
                })
            }

            fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
            where
                D: Deserializer<'de>,
            {
                Deserialize::deserialize(deserializer)
            }

            fn visit_seq<A>(self, seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let list =
                    crate::google::protobuf::ListValue::deserialize(serde::de::value::SeqAccessDeserializer::new(seq))?;
                Ok(crate::google::protobuf::Value {
                    kind: Some(Kind::ListValue(list)),
                })
            }

            fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let struct_value =
                    crate::google::protobuf::Struct::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;
                Ok(crate::google::protobuf::Value {
                    kind: Some(Kind::StructValue(struct_value)),
                })
            }
        }

        deserializer.deserialize_any(ValueVisitor)
    }
}

impl Serialize for crate::google::protobuf::Struct {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(self.fields.len()))?;
        for (k, v) in &self.fields {
            map.serialize_entry(k, v)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for crate::google::protobuf::Struct {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let fields = HashMap::<String, crate::google::protobuf::Value>::deserialize(deserializer)?;
        Ok(Self { fields })
    }
}

impl Serialize for crate::google::protobuf::ListValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeSeq;
        let mut seq = serializer.serialize_seq(Some(self.values.len()))?;
        for value in &self.values {
            seq.serialize_element(value)?;
        }
        seq.end()
    }
}

impl<'de> Deserialize<'de> for crate::google::protobuf::ListValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let values = Vec::<crate::google::protobuf::Value>::deserialize(deserializer)?;
        Ok(Self { values })
    }
}

impl get_size2::GetSize for crate::google::protobuf::Value {
    fn get_heap_size(&self) -> usize {
        self.kind.get_heap_size()
    }
}

impl get_size2::GetSize for crate::google::protobuf::ListValue {
    fn get_heap_size(&self) -> usize {
        self.values.get_heap_size()
    }
}

impl get_size2::GetSize for crate::google::protobuf::Struct {
    fn get_heap_size(&self) -> usize {
        self.fields.get_heap_size()
    }
}

impl get_size2::GetSize for crate::google::protobuf::value::Kind {
    fn get_heap_size(&self) -> usize {
        match self {
            Self::ListValue(value) => value.get_heap_size(),
            Self::StructValue(value) => value.get_heap_size(),
            Self::StringValue(value) => value.get_heap_size(),
            _ => 0,
        }
    }
}

#[derive(Serialize, Deserialize, ToSchema)]
enum SerializableResourceLocation<T> {
    #[serde(rename = "resource_address")]
    ResourceAddress(T),
    #[serde(rename = "resource_identity")]
    ResourceIdentity(T),
    #[serde(rename = "folder_address")]
    FolderAddress(T),
}

impl Serialize for nvidia::omniverse::storage::metadata::v1alpha::ResourceLocation {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use crate::{
            nvidia::omniverse::storage::metadata::v1alpha::resource_location::Resource, SerializableResourceLocation::*,
        };

        let wrapper = match self
            .resource
            .as_ref()
            .ok_or_else(|| S::Error::custom("resource cannot be null"))?
        {
            Resource::ResourceAddress(value) => ResourceAddress(value.as_str()),
            Resource::ResourceIdentity(value) => ResourceIdentity(value.encoded_identity.as_str()),
            Resource::FolderAddress(value) => FolderAddress(value.uri.as_str()),
        };

        wrapper.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for nvidia::omniverse::storage::metadata::v1alpha::ResourceLocation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use crate::nvidia::omniverse::storage::{
            filefolder::v1alpha::FolderAddress, fileobject::v1alpha::ResourceIdentity,
            metadata::v1alpha::resource_location::Resource,
        };

        let wrapper = SerializableResourceLocation::<String>::deserialize(deserializer)?;
        match wrapper {
            SerializableResourceLocation::ResourceAddress(value) => Ok(Self {
                resource: Some(Resource::ResourceAddress(value)),
            }),
            SerializableResourceLocation::ResourceIdentity(value) => Ok(Self {
                resource: Some(Resource::ResourceIdentity(ResourceIdentity {
                    encoded_identity: value,
                })),
            }),
            SerializableResourceLocation::FolderAddress(value) => Ok(Self {
                resource: Some(Resource::FolderAddress(FolderAddress { uri: value })),
            }),
        }
    }
}

impl PartialSchema for nvidia::omniverse::storage::metadata::v1alpha::ResourceLocation {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        SerializableResourceLocation::<String>::schema()
    }
}

impl ToSchema for nvidia::omniverse::storage::metadata::v1alpha::ResourceLocation {}

impl Serialize for nvidia::omniverse::metadata::v1beta::ResourceLocation {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use crate::{
            nvidia::omniverse::metadata::v1beta::resource_location::Resource, SerializableResourceLocation::*,
        };

        let wrapper = match self
            .resource
            .as_ref()
            .ok_or_else(|| S::Error::custom("resource cannot be null"))?
        {
            Resource::ResourceAddress(value) => ResourceAddress(value.as_str()),
            Resource::ResourceIdentity(value) => ResourceIdentity(value.encoded_identity.as_str()),
            Resource::FolderAddress(value) => FolderAddress(value.uri.as_str()),
        };

        wrapper.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for nvidia::omniverse::metadata::v1beta::ResourceLocation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use crate::nvidia::omniverse::metadata::v1beta::{
            resource_location::Resource, FolderAddress, ResourceIdentity,
        };

        let wrapper = SerializableResourceLocation::<String>::deserialize(deserializer)?;
        match wrapper {
            SerializableResourceLocation::ResourceAddress(value) => Ok(Self {
                resource: Some(Resource::ResourceAddress(value)),
            }),
            SerializableResourceLocation::ResourceIdentity(value) => Ok(Self {
                resource: Some(Resource::ResourceIdentity(ResourceIdentity {
                    encoded_identity: value,
                })),
            }),
            SerializableResourceLocation::FolderAddress(value) => Ok(Self {
                resource: Some(Resource::FolderAddress(FolderAddress { uri: value })),
            }),
        }
    }
}

impl PartialSchema for nvidia::omniverse::metadata::v1beta::ResourceLocation {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        SerializableResourceLocation::<String>::schema()
    }
}

impl ToSchema for nvidia::omniverse::metadata::v1beta::ResourceLocation {}

pub(crate) mod batch_entry_error_code_serde {
    use serde::{ser::Error, Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(code: &i32, serializer: S) -> Result<S::Ok, S::Error> {
        use crate::nvidia::omniverse::storage::metadata::v1alpha::batch_error_entry::Code::*;

        let wrapper = match *code {
            c if c == InvalidArgument as i32 => Wrapper::InvalidArgument,
            c if c == NotFound as i32 => Wrapper::NotFound,
            c if c == PermissionDenied as i32 => Wrapper::PermissionDenied,
            c if c == FailedPrecondition as i32 => Wrapper::FailedPrecondition,
            c if c == Internal as i32 => Wrapper::Internal,
            c if c == ResourceExhausted as i32 => Wrapper::ResourceExhausted,
            c if c == Unavailable as i32 => Wrapper::Unavailable,
            c => {
                return Err(S::Error::custom(format!(
                    "unexpected batch entry error code value {c:?}"
                )))
            },
        };

        wrapper.serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<i32, D::Error> {
        use crate::nvidia::omniverse::storage::metadata::v1alpha::batch_error_entry::Code::*;

        let wrapper = Wrapper::deserialize(deserializer)?;
        Ok(match wrapper {
            Wrapper::InvalidArgument => InvalidArgument as i32,
            Wrapper::NotFound => NotFound as i32,
            Wrapper::PermissionDenied => PermissionDenied as i32,
            Wrapper::FailedPrecondition => FailedPrecondition as i32,
            Wrapper::Internal => Internal as i32,
            Wrapper::ResourceExhausted => ResourceExhausted as i32,
            Wrapper::Unavailable => Unavailable as i32,
        })
    }

    #[derive(Serialize, Deserialize)]
    enum Wrapper {
        #[serde(rename = "invalid_argument")]
        InvalidArgument,
        #[serde(rename = "not_found")]
        NotFound,
        #[serde(rename = "permission_denied")]
        PermissionDenied,
        #[serde(rename = "failed_precondition")]
        FailedPrecondition,
        #[serde(rename = "internal")]
        Internal,
        #[serde(rename = "resource_exhausted")]
        ResourceExhausted,
        #[serde(rename = "unavailable")]
        Unavailable,
    }
}
