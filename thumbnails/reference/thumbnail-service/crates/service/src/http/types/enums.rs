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

//! Enumeration types for HTTP API — serialized values match the REST API spec (no prefixes)

use serde::{Deserialize, Serialize};
use std::fmt;
use utoipa::ToSchema;

#[derive(Deserialize, Serialize, Debug, Clone, ToSchema, PartialEq)]
pub enum TargetType {
    #[serde(rename = "FILE")]
    File,
    #[serde(rename = "FOLDER")]
    Folder,
}

impl TargetType {
    /// Whether a resource of this target type must carry a `resource_identity`.
    /// Files have identity; folders do not.
    pub fn requires_identity(&self) -> bool {
        matches!(self, TargetType::File)
    }
}

impl From<Option<&str>> for TargetType {
    fn from(stored: Option<&str>) -> Self {
        match stored {
            Some("FOLDER") => TargetType::Folder,
            Some("FILE") => TargetType::File,
            // Keep FILE fallback for metadata entries without `target_type`.
            _ => TargetType::File,
        }
    }
}

/// Thumbnail specification variants supported by `GET /v1alpha/thumbnails`.
///
/// Add variants here as their query filters become implemented.
#[derive(Deserialize, Serialize, Debug, Clone, ToSchema, PartialEq, Eq)]
pub enum ThumbnailSpecificationType {
    #[serde(rename = "IMAGE_TO_IMAGE")]
    ImageToImage,
}

impl ThumbnailSpecificationType {
    /// The `"type"` discriminant value stored in the association metadata.
    pub fn storage_key(&self) -> &'static str {
        match self {
            Self::ImageToImage => "IMAGE_TO_IMAGE",
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, ToSchema)]
pub enum FrameSelectStrategy {
    #[serde(rename = "AUTO")]
    Auto,
    #[serde(rename = "TIMECODE")]
    Timecode,
}

impl FrameSelectStrategy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Auto => "AUTO",
            Self::Timecode => "TIMECODE",
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, Copy, Default, ToSchema, PartialEq)]
pub enum TransformType {
    #[default]
    #[serde(rename = "SQUARE")]
    Square,
    #[serde(rename = "SCALE_BY_WIDTH")]
    ScaleByWidth,
    #[serde(rename = "SCALE_BY_HEIGHT")]
    ScaleByHeight,
    #[serde(rename = "CROP")]
    Crop,
    #[serde(rename = "FIT")]
    Fit,
    #[serde(rename = "SCALE_AND_CROP")]
    ScaleAndCrop,
    #[serde(rename = "THUMBNAIL")]
    Thumbnail,
    #[serde(rename = "RESIZE")]
    Resize,
}

impl TransformType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Square => "SQUARE",
            Self::ScaleByWidth => "SCALE_BY_WIDTH",
            Self::ScaleByHeight => "SCALE_BY_HEIGHT",
            Self::Crop => "CROP",
            Self::Fit => "FIT",
            Self::ScaleAndCrop => "SCALE_AND_CROP",
            Self::Thumbnail => "THUMBNAIL",
            Self::Resize => "RESIZE",
        }
    }
}

impl fmt::Display for TransformType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<thumbnail_common::TransformType> for TransformType {
    fn from(t: thumbnail_common::TransformType) -> Self {
        match t {
            thumbnail_common::TransformType::Square => Self::Square,
            thumbnail_common::TransformType::ScaleByWidth => Self::ScaleByWidth,
            thumbnail_common::TransformType::ScaleByHeight => Self::ScaleByHeight,
            thumbnail_common::TransformType::Crop => Self::Crop,
            thumbnail_common::TransformType::Fit => Self::Fit,
            thumbnail_common::TransformType::ScaleAndCrop => Self::ScaleAndCrop,
            thumbnail_common::TransformType::Thumbnail => Self::Thumbnail,
            thumbnail_common::TransformType::Resize => Self::Resize,
        }
    }
}

impl From<TransformType> for thumbnail_common::TransformType {
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

#[derive(Deserialize, Serialize, Debug, Clone, Copy, Default, ToSchema, PartialEq)]
pub enum CropStrategy {
    #[serde(rename = "CENTER")]
    Center,
    #[serde(rename = "TOP")]
    Top,
    #[serde(rename = "BOTTOM")]
    Bottom,
    #[serde(rename = "LEFT")]
    Left,
    #[serde(rename = "RIGHT")]
    Right,
    #[serde(rename = "TOP_LEFT")]
    TopLeft,
    #[serde(rename = "TOP_RIGHT")]
    TopRight,
    #[serde(rename = "BOTTOM_LEFT")]
    BottomLeft,
    #[serde(rename = "BOTTOM_RIGHT")]
    BottomRight,
    /// Content-aware crop using smartcrop2.
    #[default]
    #[serde(rename = "SMART")]
    Smart,
}

impl CropStrategy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Center => "CENTER",
            Self::Top => "TOP",
            Self::Bottom => "BOTTOM",
            Self::Left => "LEFT",
            Self::Right => "RIGHT",
            Self::TopLeft => "TOP_LEFT",
            Self::TopRight => "TOP_RIGHT",
            Self::BottomLeft => "BOTTOM_LEFT",
            Self::BottomRight => "BOTTOM_RIGHT",
            Self::Smart => "SMART",
        }
    }
}

impl fmt::Display for CropStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<thumbnail_common::CropStrategy> for CropStrategy {
    fn from(c: thumbnail_common::CropStrategy) -> Self {
        match c {
            thumbnail_common::CropStrategy::Center => Self::Center,
            thumbnail_common::CropStrategy::Top => Self::Top,
            thumbnail_common::CropStrategy::Bottom => Self::Bottom,
            thumbnail_common::CropStrategy::Left => Self::Left,
            thumbnail_common::CropStrategy::Right => Self::Right,
            thumbnail_common::CropStrategy::TopLeft => Self::TopLeft,
            thumbnail_common::CropStrategy::TopRight => Self::TopRight,
            thumbnail_common::CropStrategy::BottomLeft => Self::BottomLeft,
            thumbnail_common::CropStrategy::BottomRight => Self::BottomRight,
            thumbnail_common::CropStrategy::Smart => Self::Smart,
        }
    }
}

impl From<CropStrategy> for thumbnail_common::CropStrategy {
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

#[derive(Deserialize, Serialize, Debug, Clone, Copy, Default, ToSchema, PartialEq)]
pub enum ImageEncodingFormat {
    #[serde(rename = "JPEG", alias = "JPG")]
    Jpeg,
    #[default]
    #[serde(rename = "PNG")]
    Png,
    #[serde(rename = "GIF")]
    Gif,
    #[serde(rename = "WEBP")]
    Webp,
}

impl ImageEncodingFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Jpeg => "JPEG",
            Self::Png => "PNG",
            Self::Gif => "GIF",
            Self::Webp => "WEBP",
        }
    }
}

impl fmt::Display for ImageEncodingFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<thumbnail_common::EncodingFormat> for ImageEncodingFormat {
    fn from(e: thumbnail_common::EncodingFormat) -> Self {
        match e {
            thumbnail_common::EncodingFormat::Jpeg => Self::Jpeg,
            thumbnail_common::EncodingFormat::Png => Self::Png,
            thumbnail_common::EncodingFormat::Gif => Self::Gif,
            thumbnail_common::EncodingFormat::Webp => Self::Webp,
        }
    }
}

impl From<ImageEncodingFormat> for thumbnail_common::EncodingFormat {
    fn from(e: ImageEncodingFormat) -> Self {
        match e {
            ImageEncodingFormat::Jpeg => Self::Jpeg,
            ImageEncodingFormat::Png => Self::Png,
            ImageEncodingFormat::Gif => Self::Gif,
            ImageEncodingFormat::Webp => Self::Webp,
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, ToSchema, PartialEq)]
pub enum VideoEncodingFormat {
    #[serde(rename = "MP4_H264")]
    Mp4H264,
    #[serde(rename = "MP4_H265")]
    Mp4H265,
    #[serde(rename = "MP4_AV1")]
    Mp4Av1,
    #[serde(rename = "WEBM_VP9")]
    WebmVp9,
    #[serde(rename = "WEBM_AV1")]
    WebmAv1,
    #[serde(rename = "ANIMATED_GIF")]
    AnimatedGif,
}

impl VideoEncodingFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Mp4H264 => "MP4_H264",
            Self::Mp4H265 => "MP4_H265",
            Self::Mp4Av1 => "MP4_AV1",
            Self::WebmVp9 => "WEBM_VP9",
            Self::WebmAv1 => "WEBM_AV1",
            Self::AnimatedGif => "ANIMATED_GIF",
        }
    }
}
