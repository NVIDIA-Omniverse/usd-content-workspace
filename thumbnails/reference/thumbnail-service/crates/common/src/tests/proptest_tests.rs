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

//! Property-based tests using proptest - verifies that functions behave correctly across a wide range of inputs, helping catch edge cases that might be missed by traditional example-based tests

use crate::converters::*;
use crate::spec_enums::*;
use proptest::prelude::*;

proptest! {
    #[test]
    fn test_encoding_format_display_is_uppercase(format in "[a-zA-Z]{2,5}") {
        let resolved: EncodingFormat = format.parse().unwrap_or_default();
        let display = resolved.to_string();
        assert_eq!(display, display.to_uppercase());
    }

    #[test]
    fn test_encoding_format_known_values(
        format in prop::sample::select(vec!["JPEG", "JPG", "PNG", "GIF", "WEBP"])
    ) {
        let parsed: EncodingFormat = format.parse().unwrap();
        let expected = if format == "JPG" { "JPEG" } else { format };
        assert_eq!(parsed.to_string(), expected);
    }

    #[test]
    fn test_encoding_format_roundtrip_display_parse(
        format in prop::sample::select(vec!["JPEG", "PNG", "GIF", "WEBP"])
    ) {
        let parsed: EncodingFormat = format.parse().unwrap();
        let roundtripped: EncodingFormat = parsed.to_string().parse().unwrap();
        assert_eq!(parsed, roundtripped);
    }
}

proptest! {
    #[test]
    fn test_transform_type_proto_roundtrip(transform_code in 1i32..=6i32) {
        use thumbnail_proto::nvidia::omniverse::thumbnail::v1alpha as proto;
        if let Ok(proto_val) = proto::TransformType::try_from(transform_code) {
            let spec_val = TransformType::from(proto_val);
            let back = proto::TransformType::from(spec_val);
            assert_eq!(proto_val, back);
        }
    }

    #[test]
    fn test_transform_type_str_roundtrip(
        name in prop::sample::select(vec!["SQUARE", "SCALE_BY_WIDTH", "SCALE_BY_HEIGHT", "CROP", "FIT", "SCALE_AND_CROP"])
    ) {
        let parsed: TransformType = name.parse().unwrap();
        assert_eq!(parsed.to_string(), name);
    }
}

proptest! {
    #[test]
    fn test_crop_strategy_proto_roundtrip(crop_code in 1i32..=9i32) {
        use thumbnail_proto::nvidia::omniverse::thumbnail::v1alpha as proto;
        if let Ok(proto_val) = proto::CropStrategy::try_from(crop_code) {
            let spec_val = CropStrategy::from(proto_val);
            let back = proto::CropStrategy::from(spec_val);
            assert_eq!(proto_val, back);
        }
    }

    #[test]
    fn test_crop_strategy_str_roundtrip(
        name in prop::sample::select(vec!["CENTER", "TOP", "BOTTOM", "LEFT", "RIGHT", "TOP_LEFT", "TOP_RIGHT", "BOTTOM_LEFT", "BOTTOM_RIGHT"])
    ) {
        let parsed: CropStrategy = name.parse().unwrap();
        assert_eq!(parsed.to_string(), name);
    }
}

proptest! {
    #[test]
    fn test_current_timestamp_rfc3339_format(_unit in prop::bool::ANY) {
        let timestamp = current_timestamp_rfc3339();

        assert!(timestamp.contains('T'), "Timestamp should contain 'T' separator");
        assert!(timestamp.contains('Z') || timestamp.contains('+') || timestamp.contains('-'),
                "Timestamp should contain timezone indicator");
        assert!(timestamp.len() >= 20, "Timestamp should be at least 20 characters");

        chrono::DateTime::parse_from_rfc3339(&timestamp)
            .expect("Generated timestamp should be valid RFC3339");
    }

    #[test]
    fn test_current_timestamp_proto_validity(_unit in prop::bool::ANY) {
        let duration = current_timestamp_duration();
        let timestamp = prost_types::Timestamp {
            seconds: duration.as_secs() as i64,
            nanos: duration.subsec_nanos() as i32,
        };

        assert!(timestamp.seconds > 946_684_800, "Timestamp should be after year 2000");

        assert!(timestamp.nanos >= 0 && timestamp.nanos < 1_000_000_000);
    }

    #[test]
    fn test_current_timestamp_duration_validity(_unit in prop::bool::ANY) {
        let duration = current_timestamp_duration();

        assert!(duration.as_secs() > 946_684_800);
    }
}
