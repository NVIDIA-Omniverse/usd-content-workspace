// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

pub mod nvidia {
    pub mod omniverse {
        pub mod storage {
            pub mod fileobject {
                pub mod v1alpha {
                    pub const FILE_DESCRIPTOR_SET: &[u8] =
                        tonic::include_file_descriptor_set!("fileobject-v1alpha/v1alpha");
                    tonic::include_proto!("fileobject-v1alpha/nvidia.omniverse.storage.fileobject.v1alpha");
                }

                pub mod v1beta {
                    pub const FILE_DESCRIPTOR_SET: &[u8] =
                        tonic::include_file_descriptor_set!("fileobject-v1beta/v1beta");
                    tonic::include_proto!("fileobject-v1beta/nvidia.omniverse.storage.fileobject.v1beta");
                }
            }
            pub mod filefolder {
                pub mod v1alpha {
                    pub const FILE_DESCRIPTOR_SET: &[u8] =
                        tonic::include_file_descriptor_set!("filefolder-v1alpha/v1alpha");
                    tonic::include_proto!("filefolder-v1alpha/nvidia.omniverse.storage.filefolder.v1alpha");
                }

                pub mod v1beta {
                    pub const FILE_DESCRIPTOR_SET: &[u8] =
                        tonic::include_file_descriptor_set!("filefolder-v1beta/v1beta");
                    tonic::include_proto!("filefolder-v1beta/nvidia.omniverse.storage.filefolder.v1beta");
                }
            }
            pub mod capabilities {
                pub mod v1alpha {
                    pub const FILE_DESCRIPTOR_SET: &[u8] =
                        tonic::include_file_descriptor_set!("capabilities-v1alpha/v1alpha");
                    tonic::include_proto!("capabilities-v1alpha/nvidia.omniverse.storage.capabilities.v1alpha");
                }

                pub mod v1beta {
                    pub const FILE_DESCRIPTOR_SET: &[u8] =
                        tonic::include_file_descriptor_set!("capabilities-v1beta/v1beta");
                    tonic::include_proto!("capabilities-v1beta/nvidia.omniverse.storage.capabilities.v1beta");
                }
            }
            pub mod versioning {
                pub mod v1alpha {
                    pub const FILE_DESCRIPTOR_SET: &[u8] =
                        tonic::include_file_descriptor_set!("versioning-v1alpha/v1alpha");
                    tonic::include_proto!("versioning-v1alpha/nvidia.omniverse.storage.versioning.v1alpha");
                }

                pub mod v1beta {
                    pub const FILE_DESCRIPTOR_SET: &[u8] =
                        tonic::include_file_descriptor_set!("versioning-v1beta/v1beta");
                    tonic::include_proto!("versioning-v1beta/nvidia.omniverse.storage.versioning.v1beta");
                }
            }
            pub mod metadata {
                pub mod v1alpha {
                    pub const FILE_DESCRIPTOR_SET: &[u8] =
                        tonic::include_file_descriptor_set!("metadata-v1alpha/v1alpha");
                    tonic::include_proto!("metadata-v1alpha/nvidia.omniverse.storage.metadata.v1alpha");
                }
            }
        }
    }
}

pub mod google {
    pub mod protobuf {
        tonic::include_proto!("fileobject-v1alpha/google.protobuf");
        tonic::include_proto!("metadata-v1alpha/google.protobuf");
    }
}

use serde::de::{Deserialize, Deserializer};
use serde::ser::{Serialize, SerializeStruct, Serializer};
use std::collections::HashMap;
use time::OffsetDateTime;

impl crate::nvidia::omniverse::storage::fileobject::v1alpha::DownloadPreference {
    pub const VARIANTS: &'static [&'static str] = &["body", "redirect", "unspecified"];
}

impl std::fmt::Display for crate::nvidia::omniverse::storage::fileobject::v1alpha::DownloadPreference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            crate::nvidia::omniverse::storage::fileobject::v1alpha::DownloadPreference::Body => write!(f, "body"),
            crate::nvidia::omniverse::storage::fileobject::v1alpha::DownloadPreference::Redirect => {
                write!(f, "redirect")
            },
            crate::nvidia::omniverse::storage::fileobject::v1alpha::DownloadPreference::Unspecified => {
                write!(f, "unspecified")
            },
        }
    }
}

impl crate::nvidia::omniverse::storage::fileobject::v1alpha::UploadPreference {
    pub const VARIANTS: &'static [&'static str] = &["body", "redirect", "multipart", "unspecified"];
}

impl std::fmt::Display for crate::nvidia::omniverse::storage::fileobject::v1alpha::UploadPreference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            crate::nvidia::omniverse::storage::fileobject::v1alpha::UploadPreference::Body => write!(f, "body"),
            crate::nvidia::omniverse::storage::fileobject::v1alpha::UploadPreference::Redirect => write!(f, "redirect"),
            crate::nvidia::omniverse::storage::fileobject::v1alpha::UploadPreference::Multipart => {
                write!(f, "multipart")
            },
            crate::nvidia::omniverse::storage::fileobject::v1alpha::UploadPreference::Unspecified => {
                write!(f, "unspecified")
            },
        }
    }
}

/// For the REST implementation, last_modified_timestamp is serialized as an ISO 8601 string.
impl Serialize for crate::nvidia::omniverse::storage::fileobject::v1alpha::Metadata {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("Metadata", 3)?;
        state.serialize_field("data_object_size", &self.data_object_size)?;

        if let Some(timestamp) = self.last_modified_timestamp {
            let last_modified_timestamp = OffsetDateTime::from_unix_timestamp(timestamp.seconds)
                .map_err(|e| serde::ser::Error::custom(e.to_string()))?
                + time::Duration::seconds(timestamp.nanos as i64);
            state.serialize_field(
                "last_modified_timestamp",
                &last_modified_timestamp
                    .format(&time::format_description::well_known::Iso8601::DEFAULT)
                    .map_err(|e| serde::ser::Error::custom(e.to_string()))?,
            )?;
        }

        // FileObject-native metadata; omitted when empty so older clients see no change.
        if !self.native_metadata.is_empty() {
            state.serialize_field("native_metadata", &self.native_metadata)?;
        }
        state.end()
    }
}

impl<'de> Deserialize<'de> for crate::nvidia::omniverse::storage::fileobject::v1alpha::Metadata {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct Helper {
            data_object_size: Option<u64>,
            last_modified_timestamp: Option<String>,
            #[serde(default)]
            native_metadata: std::collections::HashMap<String, crate::google::protobuf::Value>,
        }

        let helper = Helper::deserialize(deserializer)?;

        let last_modified_timestamp = if let Some(ts_str) = helper.last_modified_timestamp {
            let datetime = OffsetDateTime::parse(&ts_str, &time::format_description::well_known::Iso8601::DEFAULT)
                .map_err(|e| serde::de::Error::custom(e.to_string()))?;

            Some(crate::google::protobuf::Timestamp {
                seconds: datetime.unix_timestamp(),
                nanos: datetime.nanosecond() as i32,
            })
        } else {
            None
        };

        Ok(Self {
            data_object_size: helper.data_object_size,
            last_modified_timestamp,
            native_metadata: helper.native_metadata,
        })
    }
}

/// For the REST implementation, the resource identity is serialized as a string.
impl Serialize for crate::nvidia::omniverse::storage::fileobject::v1alpha::ResourceIdentity {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.encoded_identity)
    }
}

impl<'de> Deserialize<'de> for crate::nvidia::omniverse::storage::fileobject::v1alpha::ResourceIdentity {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let encoded_identity = String::deserialize(deserializer)?;
        Ok(Self { encoded_identity })
    }
}

/// For the REST implementation, the additional headers are serialized as a map of strings, and there's no method field.
impl Serialize for crate::nvidia::omniverse::storage::fileobject::v1alpha::Redirect {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Gather headers as a map of strings
        let additional_headers: HashMap<String, String> = self
            .additional_headers
            .iter()
            .map(|h| (h.name.clone(), h.value.clone()))
            .collect();
        let mut state = serializer.serialize_struct("Redirect", 2)?;
        state.serialize_field("redirect_target_url", &self.redirect_target_url)?;
        state.serialize_field("additional_headers", &additional_headers)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for crate::nvidia::omniverse::storage::fileobject::v1alpha::Redirect {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct Helper {
            redirect_target_url: String,
            additional_headers: HashMap<String, String>,
        }

        let helper = Helper::deserialize(deserializer)?;

        // This is frustrating, but deprecated fields are still required to be used and
        // will emit a warning.
        // https://github.com/rust-lang/rust/issues/102777
        #[allow(deprecated)]
        Ok(Self {
            redirect_target_url: helper.redirect_target_url,
            method: "GET".to_string(),
            additional_headers: helper
                .additional_headers
                .into_iter()
                .map(|(name, value)| crate::nvidia::omniverse::storage::fileobject::v1alpha::Header { name, value })
                .collect(),
        })
    }
}

// Custom serialization for the GetFolderModeResponse to serialize the folder mode as a string.
impl Serialize for crate::nvidia::omniverse::storage::filefolder::v1alpha::GetFolderModeResponse {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("GetFolderModeResponse", 1)?;
        let string_value = match self.folder_mode {
            _ if self.folder_mode
                == crate::nvidia::omniverse::storage::filefolder::v1alpha::FolderMode::Native as i32 =>
            {
                "native"
            },
            _ if self.folder_mode
                == crate::nvidia::omniverse::storage::filefolder::v1alpha::FolderMode::NoEmpty as i32 =>
            {
                "no_empty"
            },
            _ if self.folder_mode
                == crate::nvidia::omniverse::storage::filefolder::v1alpha::FolderMode::Hybrid as i32 =>
            {
                "hybrid"
            },
            _ => return Err(serde::ser::Error::custom("Invalid folder mode")),
        };
        state.serialize_field("folder_mode", &string_value)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for crate::nvidia::omniverse::storage::filefolder::v1alpha::GetFolderModeResponse {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct Helper {
            folder_mode: String,
        }

        let helper = Helper::deserialize(deserializer)?;

        let folder_mode = match helper.folder_mode.as_str() {
            "native" => crate::nvidia::omniverse::storage::filefolder::v1alpha::FolderMode::Native as i32,
            "no_empty" => crate::nvidia::omniverse::storage::filefolder::v1alpha::FolderMode::NoEmpty as i32,
            "hybrid" => crate::nvidia::omniverse::storage::filefolder::v1alpha::FolderMode::Hybrid as i32,
            _ => return Err(serde::de::Error::custom("Invalid folder mode")),
        };

        Ok(Self { folder_mode })
    }
}

impl crate::nvidia::omniverse::storage::fileobject::v1beta::DownloadPreference {
    pub const VARIANTS: &'static [&'static str] = &["body", "redirect", "unspecified"];
}

impl std::fmt::Display for crate::nvidia::omniverse::storage::fileobject::v1beta::DownloadPreference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            crate::nvidia::omniverse::storage::fileobject::v1beta::DownloadPreference::Body => write!(f, "body"),
            crate::nvidia::omniverse::storage::fileobject::v1beta::DownloadPreference::Redirect => {
                write!(f, "redirect")
            },
            crate::nvidia::omniverse::storage::fileobject::v1beta::DownloadPreference::Unspecified => {
                write!(f, "unspecified")
            },
        }
    }
}

impl crate::nvidia::omniverse::storage::fileobject::v1beta::UploadPreference {
    pub const VARIANTS: &'static [&'static str] = &["body", "redirect", "multipart", "unspecified"];
}

impl std::fmt::Display for crate::nvidia::omniverse::storage::fileobject::v1beta::UploadPreference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            crate::nvidia::omniverse::storage::fileobject::v1beta::UploadPreference::Body => write!(f, "body"),
            crate::nvidia::omniverse::storage::fileobject::v1beta::UploadPreference::Redirect => write!(f, "redirect"),
            crate::nvidia::omniverse::storage::fileobject::v1beta::UploadPreference::Multipart => {
                write!(f, "multipart")
            },
            crate::nvidia::omniverse::storage::fileobject::v1beta::UploadPreference::Unspecified => {
                write!(f, "unspecified")
            },
        }
    }
}

/// For the REST implementation, last_modified_timestamp is serialized as an ISO 8601 string.
impl Serialize for crate::nvidia::omniverse::storage::fileobject::v1beta::Metadata {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("Metadata", 2)?;
        state.serialize_field("data_object_size", &self.data_object_size)?;

        if let Some(timestamp) = self.last_modified_timestamp {
            let last_modified_timestamp = OffsetDateTime::from_unix_timestamp(timestamp.seconds)
                .map_err(|e| serde::ser::Error::custom(e.to_string()))?
                + time::Duration::seconds(timestamp.nanos as i64);
            state.serialize_field(
                "last_modified_timestamp",
                &last_modified_timestamp
                    .format(&time::format_description::well_known::Iso8601::DEFAULT)
                    .map_err(|e| serde::ser::Error::custom(e.to_string()))?,
            )?;
        }
        state.end()
    }
}

impl<'de> Deserialize<'de> for crate::nvidia::omniverse::storage::fileobject::v1beta::Metadata {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct Helper {
            data_object_size: Option<u64>,
            last_modified_timestamp: Option<String>,
        }

        let helper = Helper::deserialize(deserializer)?;

        let last_modified_timestamp = if let Some(ts_str) = helper.last_modified_timestamp {
            let datetime = OffsetDateTime::parse(&ts_str, &time::format_description::well_known::Iso8601::DEFAULT)
                .map_err(|e| serde::de::Error::custom(e.to_string()))?;

            Some(crate::google::protobuf::Timestamp {
                seconds: datetime.unix_timestamp(),
                nanos: datetime.nanosecond() as i32,
            })
        } else {
            None
        };

        Ok(Self {
            data_object_size: helper.data_object_size,
            last_modified_timestamp,
        })
    }
}

/// For the REST implementation, the resource identity is serialized as a string.
impl Serialize for crate::nvidia::omniverse::storage::fileobject::v1beta::ResourceIdentity {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.encoded_identity)
    }
}

impl<'de> Deserialize<'de> for crate::nvidia::omniverse::storage::fileobject::v1beta::ResourceIdentity {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let encoded_identity = String::deserialize(deserializer)?;
        Ok(Self { encoded_identity })
    }
}

/// For the REST implementation, the additional headers are serialized as a map of strings, and there's no method field.
impl Serialize for crate::nvidia::omniverse::storage::fileobject::v1beta::Redirect {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Gather headers as a map of strings
        let additional_headers: HashMap<String, String> = self
            .additional_headers
            .iter()
            .map(|h| (h.name.clone(), h.value.clone()))
            .collect();
        let mut state = serializer.serialize_struct("Redirect", 2)?;
        state.serialize_field("redirect_target_url", &self.redirect_target_url)?;
        state.serialize_field("additional_headers", &additional_headers)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for crate::nvidia::omniverse::storage::fileobject::v1beta::Redirect {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct Helper {
            redirect_target_url: String,
            additional_headers: HashMap<String, String>,
        }

        let helper = Helper::deserialize(deserializer)?;

        // This is frustrating, but deprecated fields are still required to be used and
        // will emit a warning.
        // https://github.com/rust-lang/rust/issues/102777
        #[allow(deprecated)]
        Ok(Self {
            redirect_target_url: helper.redirect_target_url,
            method: "GET".to_string(),
            additional_headers: helper
                .additional_headers
                .into_iter()
                .map(|(name, value)| crate::nvidia::omniverse::storage::fileobject::v1beta::Header { name, value })
                .collect(),
        })
    }
}

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

            // Note: `as f64` loses precision for integers outside the range -(2^53)..2^53.
            // This matches protobuf `Value` semantics where `number_value` is always a double.
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
                    kind: Some(Kind::NullValue(0)),
                })
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: Error,
            {
                Ok(crate::google::protobuf::Value {
                    kind: Some(Kind::NullValue(0)),
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

/// Custom deserializer for `Option<google::protobuf::Value>` fields that preserves
/// the distinction between a missing field (`None`) and an explicit JSON `null`
/// (`Some(NullValue)`). Serde's default `Option<T>` deserialization maps both cases
/// to `None`, which loses explicitly-set NullValue payloads during JSON round-trips.
pub fn deserialize_optional_value<'de, D>(deserializer: D) -> Result<Option<crate::google::protobuf::Value>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    crate::google::protobuf::Value::deserialize(deserializer).map(Some)
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

#[cfg(test)]
mod native_metadata_serde_tests {
    use crate::google::protobuf::{value::Kind, Value};
    use crate::nvidia::omniverse::storage::fileobject::v1alpha::Metadata;

    #[test]
    fn metadata_native_metadata_round_trips_through_json() {
        let metadata = Metadata {
            data_object_size: Some(7),
            last_modified_timestamp: None,
            native_metadata: std::collections::HashMap::from([(
                "source".to_string(),
                Value {
                    kind: Some(Kind::StringValue("pipeline-a".to_string())),
                },
            )]),
        };

        let json = serde_json::to_string(&metadata).expect("serialize");
        assert!(
            json.contains("native_metadata"),
            "native_metadata must be emitted: {json}"
        );
        let round: Metadata = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(round.native_metadata.len(), 1);
        assert_eq!(
            round.native_metadata.get("source").and_then(|v| v.kind.clone()),
            Some(Kind::StringValue("pipeline-a".to_string()))
        );
    }

    #[test]
    fn metadata_empty_native_metadata_is_omitted_and_defaults_on_parse() {
        let metadata = Metadata {
            data_object_size: Some(1),
            last_modified_timestamp: None,
            native_metadata: std::collections::HashMap::new(),
        };
        let json = serde_json::to_string(&metadata).expect("serialize");
        assert!(!json.contains("native_metadata"), "empty map must be omitted: {json}");
        // Older payloads without the field still parse.
        let round: Metadata = serde_json::from_str(r#"{"data_object_size":1}"#).expect("deserialize legacy");
        assert!(round.native_metadata.is_empty());
    }
}
