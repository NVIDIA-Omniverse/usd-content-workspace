// SPDX-FileCopyrightText: Copyright (c) 2024 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Serde helpers for types that don't derive `Serialize` / `Deserialize`
//! upstream (e.g., prost-generated protobuf types).

/// Serde helper for `Option<storage_core_protos::google::protobuf::Timestamp>`.
///
/// Use as `#[serde(with = "storage_core_util::serde_helpers::option_timestamp")]`.
/// Round-trips through `(seconds: i64, nanos: i32)` so cached values can be
/// JSON-serialized for L2 storage.
pub mod option_timestamp {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use storage_core_protos::google::protobuf::Timestamp;

    #[derive(Serialize, Deserialize)]
    struct TimestampRepr {
        seconds: i64,
        nanos: i32,
    }

    pub fn serialize<S: Serializer>(value: &Option<Timestamp>, serializer: S) -> Result<S::Ok, S::Error> {
        match value {
            Some(ts) => Some(TimestampRepr {
                seconds: ts.seconds,
                nanos: ts.nanos,
            }),
            None => None,
        }
        .serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Timestamp>, D::Error> {
        let repr = Option::<TimestampRepr>::deserialize(deserializer)?;
        Ok(repr.map(|r| Timestamp {
            seconds: r.seconds,
            nanos: r.nanos,
        }))
    }
}

/// Serde helper for `Option<storage_core_protos::google::protobuf::Value>`.
///
/// Use as `#[serde(with = "storage_core_util::serde_helpers::option_proto_value")]`.
/// Round-trips through prost wire bytes so cached metadata values can be
/// serialized for L2 (Redis) storage.
pub mod option_proto_value {
    use prost::Message;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use storage_core_protos::google::protobuf::Value;

    pub fn serialize<S: Serializer>(value: &Option<Value>, serializer: S) -> Result<S::Ok, S::Error> {
        value.as_ref().map(Message::encode_to_vec).serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Value>, D::Error> {
        let bytes = Option::<Vec<u8>>::deserialize(deserializer)?;
        bytes
            .map(|b| Value::decode(b.as_slice()).map_err(serde::de::Error::custom))
            .transpose()
    }
}

/// Postcard-safe serde for a `HashMap<String, Value>` native-metadata map.
///
/// `google.protobuf.Value`'s `Deserialize` uses `deserialize_any`, which non-self-describing
/// formats (the postcard L2 cache tier) cannot support. Encode each value to protobuf bytes so the
/// serialized shape is `HashMap<String, Vec<u8>>`, then decode back on read — mirrors
/// [`option_proto_value`].
pub mod native_metadata_map {
    use std::collections::HashMap;

    use prost::Message;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use storage_core_protos::google::protobuf::Value;

    pub fn serialize<S: Serializer>(map: &HashMap<String, Value>, serializer: S) -> Result<S::Ok, S::Error> {
        let encoded: HashMap<&String, Vec<u8>> = map.iter().map(|(k, v)| (k, v.encode_to_vec())).collect();
        encoded.serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<HashMap<String, Value>, D::Error> {
        HashMap::<String, Vec<u8>>::deserialize(deserializer)?
            .into_iter()
            .map(|(k, bytes)| {
                Value::decode(bytes.as_slice())
                    .map(|v| (k, v))
                    .map_err(serde::de::Error::custom)
            })
            .collect()
    }
}

#[cfg(test)]
mod native_metadata_map_tests {
    use std::collections::HashMap;

    use serde::{Deserialize, Serialize};
    use storage_core_protos::google::protobuf::{value::Kind, Value};

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct Envelope {
        #[serde(default, with = "super::native_metadata_map")]
        native_metadata: HashMap<String, Value>,
    }

    fn string_value(s: &str) -> Value {
        Value {
            kind: Some(Kind::StringValue(s.to_string())),
        }
    }

    #[test]
    fn survives_postcard_round_trip() {
        // The L2 cache tier encodes with postcard (non-self-describing); Value's own Deserialize uses
        // deserialize_any and would fail there. The helper must make a metadata-bearing envelope survive.
        let env = Envelope {
            native_metadata: HashMap::from([
                ("source".to_string(), string_value("pipeline-a")),
                ("owner".to_string(), string_value("team-x")),
            ]),
        };
        let bytes = postcard::to_allocvec(&env).expect("postcard encode");
        let round: Envelope = postcard::from_bytes(&bytes).expect("postcard decode");
        assert_eq!(round, env);
    }

    #[test]
    fn empty_map_survives_postcard() {
        let env = Envelope {
            native_metadata: HashMap::new(),
        };
        let bytes = postcard::to_allocvec(&env).expect("postcard encode");
        let round: Envelope = postcard::from_bytes(&bytes).expect("postcard decode");
        assert!(round.native_metadata.is_empty());
    }
}
