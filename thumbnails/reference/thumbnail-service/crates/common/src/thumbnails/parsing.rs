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

//! Thumbnail key parsing and metadata extraction - handles parsing thumbnail metadata keys and extracting resource identities from protobuf metadata values

use super::types::{ThumbnailInfo, ThumbnailMatchInfo};
use prost_types;
use tracing::info;

/// Read a string field by path from a protobuf `Struct`.
///
/// Example:
/// - `["thumbnail", "resource_identity"]`
/// - `["association_target", "resource_address"]`
fn read_struct_value_by_path<'a>(
    struct_val: &'a prost_types::Struct,
    path: &[&str],
) -> Option<&'a str> {
    let (leaf, parents) = path.split_last()?;
    let mut current = struct_val;

    for segment in parents {
        current = match current.fields.get(*segment)?.kind.as_ref()? {
            prost_types::value::Kind::StructValue(next) => next,
            _ => return None,
        };
    }

    match current.fields.get(*leaf)?.kind.as_ref()? {
        prost_types::value::Kind::StringValue(v) => Some(v.as_str()),
        _ => None,
    }
}

fn struct_top_level_string<'a>(struct_val: &'a prost_types::Struct, key: &str) -> Option<&'a str> {
    match struct_val.fields.get(key)?.kind.as_ref()? {
        prost_types::value::Kind::StringValue(v) => Some(v.as_str()),
        _ => None,
    }
}

pub fn parse_thumbnail_key(key: &str) -> Option<ThumbnailInfo> {
    if !key.starts_with("thumbnail_") {
        return None;
    }

    let remainder = key.strip_prefix("thumbnail_")?;
    let parts: Vec<&str> = remainder.split('_').collect();
    if parts.len() < 3 {
        return None;
    }

    let dimensions: Vec<&str> = parts[0].split('x').collect();
    if dimensions.len() != 2 {
        return None;
    }

    let width = dimensions[0].parse::<u32>().ok()?;
    let height = dimensions[1].parse::<u32>().ok()?;
    let format = parts[1].to_string();

    let uuid = parts[2..].join("_");

    Some(ThumbnailInfo {
        width,
        height,
        format,
        uuid,
    })
}

pub fn extract_resource_identity_from_metadata(value: &prost_types::Value) -> Option<String> {
    match value.kind.as_ref()? {
        prost_types::value::Kind::StringValue(s) => {
            info!("[PARSE] Processing string_value metadata");
            info!("[PARSE] Parsing JSON string: {}", s);
            if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(s) {
                // Nested shape: thumbnail.resource_identity (ADR 0002 §2).
                if let Some(id) = json_val
                    .get("thumbnail")
                    .and_then(|t| t.get("resource_identity"))
                    .and_then(|v| v.as_str())
                {
                    info!(
                        "[OK] Found thumbnail.resource_identity in JSON string: {}",
                        id
                    );
                    return Some(id.to_string());
                }
            }
            info!("[OK] Using string_value directly as resource identity");
            Some(s.clone())
        }
        prost_types::value::Kind::StructValue(struct_val) => {
            info!("[PARSE] Processing struct_value metadata");
            // Nested shape: thumbnail.resource_identity (ADR 0002 §2).
            if let Some(id) =
                read_struct_value_by_path(struct_val, &["thumbnail", "resource_identity"])
            {
                info!(
                    "[OK] Extracted thumbnail.resource_identity from struct: {}",
                    id
                );
                return Some(id.to_string());
            }
            info!(
                "[WARN] No thumbnail.resource_identity found in struct, available keys: {:?}",
                struct_val.fields.keys().collect::<Vec<_>>()
            );
            None
        }
        _ => {
            info!("[WARN]  Unsupported metadata value type");
            None
        }
    }
}

/// Extracts both the resource identity and specification from a metadata value.
pub fn extract_match_info_from_metadata(
    metadata_key: &str,
    value: &prost_types::Value,
) -> Option<ThumbnailMatchInfo> {
    match value.kind.as_ref()? {
        prost_types::value::Kind::StringValue(s) => {
            let Ok(json_val) = serde_json::from_str::<serde_json::Value>(s) else {
                // String metadata must be JSON-encoded association metadata.
                // Non-JSON string values are treated as malformed/stale input.
                return None;
            };

            let thumbnail = json_val.get("thumbnail");
            let resource_identity = thumbnail
                .and_then(|t| t.get("resource_identity"))
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let resource_address = thumbnail
                .and_then(|t| t.get("resource_address"))
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let target_identity = json_val
                .get("association_target")
                .and_then(|t| t.get("resource_identity"))
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let target_type = json_val
                .get("target_type")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let specification = json_val.get("specification").cloned();
            let create_time = json_val
                .get("create_time")
                .and_then(|v| v.as_str())
                .map(str::to_string);

            Some(ThumbnailMatchInfo {
                metadata_key: metadata_key.to_string(),
                thumbnail_resource_identity: resource_identity?,
                thumbnail_resource_address: resource_address,
                association_target_resource_identity: target_identity,
                target_type,
                specification,
                create_time,
            })
        }
        prost_types::value::Kind::StructValue(struct_val) => {
            let resource_identity =
                read_struct_value_by_path(struct_val, &["thumbnail", "resource_identity"])?
                    .to_string();
            let resource_address =
                read_struct_value_by_path(struct_val, &["thumbnail", "resource_address"])
                    .map(str::to_string);
            let target_identity =
                read_struct_value_by_path(struct_val, &["association_target", "resource_identity"])
                    .map(str::to_string);
            let target_type =
                struct_top_level_string(struct_val, "target_type").map(str::to_string);

            let specification = struct_val
                .fields
                .get("specification")
                .and_then(protobuf_value_to_json);
            let create_time =
                struct_top_level_string(struct_val, "create_time").map(str::to_string);

            Some(ThumbnailMatchInfo {
                metadata_key: metadata_key.to_string(),
                thumbnail_resource_identity: resource_identity,
                thumbnail_resource_address: resource_address,
                association_target_resource_identity: target_identity,
                target_type,
                specification,
                create_time,
            })
        }
        _ => None,
    }
}

/// The largest integer an `f64` represents exactly, 2^53.
const MAX_EXACT_INTEGER: f64 = 9_007_199_254_740_992.0;

/// Renders a protobuf `Struct` number, narrowing whole values back to JSON
/// integers.
///
/// A `Struct` has one numeric kind and it is a double, so `width: 256` becomes
/// `256.0` the moment it round-trips through stored metadata. That reaches
/// clients: Navigator's generated code checks the response against a `uint32`
/// field and rejects the float. Associations stored as a JSON string never had
/// the problem, because `serde_json` keeps `256` an integer — this restores the
/// same output for the `StructValue` shape that nested metadata introduced.
///
/// Above 2^53 an integral double is no longer necessarily the value that was
/// written, so those keep their float form rather than being given a precision
/// they do not have.
fn protobuf_number_to_json(n: f64) -> Option<serde_json::Value> {
    if n.fract() == 0.0 && n.abs() <= MAX_EXACT_INTEGER {
        if n.is_sign_negative() {
            return Some(serde_json::Value::Number((n as i64).into()));
        }
        return Some(serde_json::Value::Number((n as u64).into()));
    }
    serde_json::Number::from_f64(n).map(serde_json::Value::Number)
}

fn protobuf_value_to_json(value: &prost_types::Value) -> Option<serde_json::Value> {
    match value.kind.as_ref()? {
        prost_types::value::Kind::NullValue(_) => Some(serde_json::Value::Null),
        prost_types::value::Kind::NumberValue(n) => protobuf_number_to_json(*n),
        prost_types::value::Kind::StringValue(s) => Some(serde_json::Value::String(s.clone())),
        prost_types::value::Kind::BoolValue(b) => Some(serde_json::Value::Bool(*b)),
        prost_types::value::Kind::ListValue(list) => {
            let items: Vec<serde_json::Value> = list
                .values
                .iter()
                .filter_map(protobuf_value_to_json)
                .collect();
            Some(serde_json::Value::Array(items))
        }
        prost_types::value::Kind::StructValue(s) => {
            let map: serde_json::Map<String, serde_json::Value> = s
                .fields
                .iter()
                .filter_map(|(k, v)| protobuf_value_to_json(v).map(|jv| (k.clone(), jv)))
                .collect();
            Some(serde_json::Value::Object(map))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_thumbnail_key_valid() {
        let key = "thumbnail_256x256_PNG_abc123";
        let info = parse_thumbnail_key(key).unwrap();
        assert_eq!(info.width, 256);
        assert_eq!(info.height, 256);
        assert_eq!(info.format, "PNG");
        assert_eq!(info.uuid, "abc123");
    }

    #[test]
    fn test_parse_thumbnail_key_with_uuid_underscores() {
        let key = "thumbnail_64x64_JPEG_9766778a-d92b-4463-92a7-8c1a93ea2e63";
        let info = parse_thumbnail_key(key).unwrap();
        assert_eq!(info.width, 64);
        assert_eq!(info.height, 64);
        assert_eq!(info.format, "JPEG");
        assert_eq!(info.uuid, "9766778a-d92b-4463-92a7-8c1a93ea2e63");
    }

    #[test]
    fn test_parse_thumbnail_key_invalid_prefix() {
        let key = "invalid_256x256_PNG_abc123";
        assert!(parse_thumbnail_key(key).is_none());
    }

    #[test]
    fn test_parse_thumbnail_key_invalid_dimensions() {
        let key = "thumbnail_invalid_PNG_abc123";
        assert!(parse_thumbnail_key(key).is_none());
    }

    #[test]
    fn extract_match_info_returns_stored_thumbnail_address_and_target_identity() {
        // Nested association metadata (ADR 0002 §2) as stored on the target's address.
        let stored = serde_json::json!({
            "thumbnail": {
                "resource_identity": "https://store.test/thumb.png?version-id=thumb",
                "resource_address": "https://store.test/thumb.png",
            },
            "association_target": {
                "resource_address": "https://store.test/source.usd",
                "resource_identity": "https://store.test/source.usd?version-id=src",
            },
            "target_type": "FILE",
            "create_time": "2026-06-01T12:00:00Z",
        })
        .to_string();
        let value = prost_types::Value {
            kind: Some(prost_types::value::Kind::StringValue(stored)),
        };

        let info = extract_match_info_from_metadata("thumbnail_test_key", &value)
            .expect("should parse nested metadata");
        // The thumbnail address comes straight from the stored metadata, not the identity.
        assert_eq!(
            info.thumbnail_resource_address.as_deref(),
            Some("https://store.test/thumb.png")
        );
        assert_eq!(
            info.thumbnail_resource_identity,
            "https://store.test/thumb.png?version-id=thumb"
        );
        assert_eq!(
            info.association_target_resource_identity.as_deref(),
            Some("https://store.test/source.usd?version-id=src")
        );
        assert_eq!(info.target_type.as_deref(), Some("FILE"));
        assert_eq!(info.create_time.as_deref(), Some("2026-06-01T12:00:00Z"));
    }

    #[test]
    fn extract_match_info_rejects_plain_string_identity() {
        let value = prost_types::Value {
            kind: Some(prost_types::value::Kind::StringValue(
                "https://store.test/thumb.png?version-id=thumb".to_string(),
            )),
        };

        assert!(
            extract_match_info_from_metadata("thumbnail_test_key", &value).is_none(),
            "plain-string metadata should be rejected; only structured metadata is supported"
        );
    }

    #[test]
    fn extract_match_info_rejects_json_without_thumbnail_identity() {
        let value = prost_types::Value {
            kind: Some(prost_types::value::Kind::StringValue(
                serde_json::json!({
                    "thumbnail": {
                        "resource_address": "https://store.test/thumb.png"
                    },
                    "target_type": "FILE"
                })
                .to_string(),
            )),
        };

        assert!(
            extract_match_info_from_metadata("thumbnail_test_key", &value).is_none(),
            "nested metadata without thumbnail.resource_identity is malformed"
        );
    }

    fn number(n: f64) -> prost_types::Value {
        prost_types::Value {
            kind: Some(prost_types::value::Kind::NumberValue(n)),
        }
    }

    fn string(s: &str) -> prost_types::Value {
        prost_types::Value {
            kind: Some(prost_types::value::Kind::StringValue(s.to_string())),
        }
    }

    fn structure(fields: &[(&str, prost_types::Value)]) -> prost_types::Value {
        prost_types::Value {
            kind: Some(prost_types::value::Kind::StructValue(prost_types::Struct {
                fields: fields
                    .iter()
                    .map(|(k, v)| ((*k).to_string(), v.clone()))
                    .collect(),
            })),
        }
    }

    /// The regression Navigator reported. `width` and `height` are `uint32` on
    /// the wire, but a protobuf `Struct` can only carry them as doubles, so this
    /// path handed back `256.0` and Navigator's generated client rejected it
    /// against the integer field.
    #[test]
    fn struct_metadata_returns_whole_specification_numbers_as_integers() {
        let value = structure(&[
            (
                "thumbnail",
                structure(&[
                    (
                        "resource_identity",
                        string("https://store.test/thumb.png?version-id=thumb"),
                    ),
                    ("resource_address", string("https://store.test/thumb.png")),
                ]),
            ),
            ("target_type", string("FILE")),
            (
                "specification",
                structure(&[
                    ("width", number(256.0)),
                    ("height", number(256.0)),
                    ("quality", number(85.0)),
                ]),
            ),
        ]);

        let info = extract_match_info_from_metadata("thumbnail_test_key", &value)
            .expect("struct metadata should parse");
        let spec = info.specification.expect("specification should be present");

        for field in ["width", "height", "quality"] {
            assert!(
                spec[field].is_u64(),
                "{field} should come back as an integer, got {}",
                spec[field]
            );
        }

        // The rendered text is what a client actually parses, and the only place
        // the integer/float distinction is observable from outside this process.
        let rendered = spec.to_string();
        assert!(
            !rendered.contains("256.0") && !rendered.contains("85.0"),
            "whole numbers should not render as floats: {rendered}"
        );
    }

    #[test]
    fn a_genuinely_fractional_number_keeps_its_float_form() {
        let value = structure(&[
            (
                "thumbnail",
                structure(&[(
                    "resource_identity",
                    string("https://store.test/thumb.png?version-id=thumb"),
                )]),
            ),
            ("specification", structure(&[("ratio", number(1.5))])),
        ]);

        let spec = extract_match_info_from_metadata("thumbnail_test_key", &value)
            .expect("struct metadata should parse")
            .specification
            .expect("specification should be present");

        assert_eq!(spec["ratio"].as_f64(), Some(1.5));
        assert!(
            spec.to_string().contains("1.5"),
            "narrowing must not round a fractional value away"
        );
    }

    /// Past 2^53 a whole-looking double is not reliably the value that was
    /// written, so it keeps the float form rather than gaining false precision.
    #[test]
    fn an_integer_beyond_exact_double_range_is_left_alone() {
        let beyond = MAX_EXACT_INTEGER * 4.0;
        assert!(
            !protobuf_number_to_json(beyond)
                .expect("a finite double converts")
                .is_u64(),
            "a value past 2^53 should not be presented as an exact integer"
        );
        assert!(
            protobuf_number_to_json(MAX_EXACT_INTEGER)
                .expect("a finite double converts")
                .is_u64(),
            "the 2^53 boundary itself is exactly representable"
        );
    }

    #[test]
    fn non_finite_numbers_are_still_dropped() {
        assert!(protobuf_number_to_json(f64::NAN).is_none());
        assert!(protobuf_number_to_json(f64::INFINITY).is_none());
    }
}
