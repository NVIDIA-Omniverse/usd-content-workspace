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

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use thumbnail_proto::nvidia::omniverse::thumbnail::v1alpha as proto;
use utoipa::ToSchema;

// ---------------------------------------------------------------------------
// EncodingFormat
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EncodingFormat {
    #[default]
    Png,
    #[serde(alias = "JPG")]
    Jpeg,
    Gif,
    Webp,
}

impl EncodingFormat {
    /// Whether encode quality must be in the lossy `1..=100` range.
    pub fn requires_lossy_quality_range(self) -> bool {
        match self {
            Self::Jpeg | Self::Webp => true,
            Self::Png | Self::Gif => false,
        }
    }
}

#[cfg(test)]
mod quality_tests {
    use super::EncodingFormat;

    #[test]
    fn quality_range_is_format_dependent() {
        for (format, bounded) in [
            (EncodingFormat::Jpeg, true),
            (EncodingFormat::Webp, true),
            (EncodingFormat::Png, false),
            (EncodingFormat::Gif, false),
        ] {
            assert_eq!(format.requires_lossy_quality_range(), bounded, "{format}");
        }
    }
}

impl fmt::Display for EncodingFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Png => write!(f, "PNG"),
            Self::Jpeg => write!(f, "JPEG"),
            Self::Gif => write!(f, "GIF"),
            Self::Webp => write!(f, "WEBP"),
        }
    }
}

impl FromStr for EncodingFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "PNG" => Ok(Self::Png),
            "JPEG" | "JPG" => Ok(Self::Jpeg),
            "GIF" => Ok(Self::Gif),
            "WEBP" => Ok(Self::Webp),
            _ => Err(format!("Unknown encoding_format: '{s}'")),
        }
    }
}

impl From<proto::ImageEncodingFormat> for EncodingFormat {
    fn from(p: proto::ImageEncodingFormat) -> Self {
        match p {
            proto::ImageEncodingFormat::Jpeg => Self::Jpeg,
            proto::ImageEncodingFormat::Gif => Self::Gif,
            proto::ImageEncodingFormat::Webp => Self::Webp,
            _ => Self::Png,
        }
    }
}

impl From<EncodingFormat> for proto::ImageEncodingFormat {
    fn from(e: EncodingFormat) -> Self {
        match e {
            EncodingFormat::Png => Self::Png,
            EncodingFormat::Jpeg => Self::Jpeg,
            EncodingFormat::Gif => Self::Gif,
            EncodingFormat::Webp => Self::Webp,
        }
    }
}

// ---------------------------------------------------------------------------
// TransformType
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TransformType {
    #[default]
    Square,
    ScaleByWidth,
    ScaleByHeight,
    Crop,
    Fit,
    ScaleAndCrop,
    Thumbnail,
    Resize,
}

impl fmt::Display for TransformType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Square => write!(f, "SQUARE"),
            Self::ScaleByWidth => write!(f, "SCALE_BY_WIDTH"),
            Self::ScaleByHeight => write!(f, "SCALE_BY_HEIGHT"),
            Self::Crop => write!(f, "CROP"),
            Self::Fit => write!(f, "FIT"),
            Self::ScaleAndCrop => write!(f, "SCALE_AND_CROP"),
            Self::Thumbnail => write!(f, "THUMBNAIL"),
            Self::Resize => write!(f, "RESIZE"),
        }
    }
}

impl FromStr for TransformType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "SQUARE" => Ok(Self::Square),
            "SCALE_BY_WIDTH" => Ok(Self::ScaleByWidth),
            "SCALE_BY_HEIGHT" => Ok(Self::ScaleByHeight),
            "CROP" => Ok(Self::Crop),
            "FIT" => Ok(Self::Fit),
            "SCALE_AND_CROP" => Ok(Self::ScaleAndCrop),
            "THUMBNAIL" => Ok(Self::Thumbnail),
            "RESIZE" => Ok(Self::Resize),
            _ => Err(format!("Unknown transform_type: '{s}'")),
        }
    }
}

impl From<proto::TransformType> for TransformType {
    fn from(p: proto::TransformType) -> Self {
        match p {
            proto::TransformType::Square => Self::Square,
            proto::TransformType::ScaleByWidth => Self::ScaleByWidth,
            proto::TransformType::ScaleByHeight => Self::ScaleByHeight,
            proto::TransformType::Crop => Self::Crop,
            proto::TransformType::Fit => Self::Fit,
            proto::TransformType::ScaleAndCrop => Self::ScaleAndCrop,
            proto::TransformType::Thumbnail => Self::Thumbnail,
            proto::TransformType::Resize => Self::Resize,
            _ => Self::default(),
        }
    }
}

impl From<TransformType> for proto::TransformType {
    fn from(t: TransformType) -> Self {
        match t {
            TransformType::Square => Self::Square,
            TransformType::ScaleByWidth => Self::ScaleByWidth,
            TransformType::ScaleByHeight => Self::ScaleByHeight,
            TransformType::Crop => Self::Crop,
            TransformType::Fit => Self::Fit,
            TransformType::ScaleAndCrop => Self::ScaleAndCrop,
            TransformType::Thumbnail => Self::Thumbnail,
            TransformType::Resize => Self::Resize,
        }
    }
}

// ---------------------------------------------------------------------------
// CropStrategy
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CropStrategy {
    Center,
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    /// Content-aware crop using smartcrop2: analyzes the image for visual
    /// saliency and selects the most interesting crop region automatically.
    #[default]
    Smart,
}

impl fmt::Display for CropStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Center => write!(f, "CENTER"),
            Self::Top => write!(f, "TOP"),
            Self::Bottom => write!(f, "BOTTOM"),
            Self::Left => write!(f, "LEFT"),
            Self::Right => write!(f, "RIGHT"),
            Self::TopLeft => write!(f, "TOP_LEFT"),
            Self::TopRight => write!(f, "TOP_RIGHT"),
            Self::BottomLeft => write!(f, "BOTTOM_LEFT"),
            Self::BottomRight => write!(f, "BOTTOM_RIGHT"),
            Self::Smart => write!(f, "SMART"),
        }
    }
}

impl FromStr for CropStrategy {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "CENTER" => Ok(Self::Center),
            "TOP" => Ok(Self::Top),
            "BOTTOM" => Ok(Self::Bottom),
            "LEFT" => Ok(Self::Left),
            "RIGHT" => Ok(Self::Right),
            "TOP_LEFT" => Ok(Self::TopLeft),
            "TOP_RIGHT" => Ok(Self::TopRight),
            "BOTTOM_LEFT" => Ok(Self::BottomLeft),
            "BOTTOM_RIGHT" => Ok(Self::BottomRight),
            "SMART" => Ok(Self::Smart),
            _ => Err(format!("Unknown crop_strategy: '{s}'")),
        }
    }
}

impl From<proto::CropStrategy> for CropStrategy {
    fn from(p: proto::CropStrategy) -> Self {
        match p {
            proto::CropStrategy::Center => Self::Center,
            proto::CropStrategy::Top => Self::Top,
            proto::CropStrategy::Bottom => Self::Bottom,
            proto::CropStrategy::Left => Self::Left,
            proto::CropStrategy::Right => Self::Right,
            proto::CropStrategy::TopLeft => Self::TopLeft,
            proto::CropStrategy::TopRight => Self::TopRight,
            proto::CropStrategy::BottomLeft => Self::BottomLeft,
            proto::CropStrategy::BottomRight => Self::BottomRight,
            proto::CropStrategy::Smart => Self::Smart,
            _ => Self::default(),
        }
    }
}

impl From<CropStrategy> for proto::CropStrategy {
    fn from(c: CropStrategy) -> Self {
        match c {
            CropStrategy::Center => Self::Center,
            CropStrategy::Top => Self::Top,
            CropStrategy::Bottom => Self::Bottom,
            CropStrategy::Left => Self::Left,
            CropStrategy::Right => Self::Right,
            CropStrategy::TopLeft => Self::TopLeft,
            CropStrategy::TopRight => Self::TopRight,
            CropStrategy::BottomLeft => Self::BottomLeft,
            CropStrategy::BottomRight => Self::BottomRight,
            CropStrategy::Smart => Self::Smart,
        }
    }
}
