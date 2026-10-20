// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

// Standard library imports
use std::error::Error;
use std::fmt;

// Third-party imports
use http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

// Local imports
use crate::traits::{
    error::status_with_default_error_info,
    fileobject::v1alpha::{AddressInfo, DownloadPreference, UploadPreference},
    StorageErrorExt,
};

pub struct InternalError {
    pub message: String,
}

impl fmt::Debug for InternalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InternalError").field("message", &self.message).finish()
    }
}

impl fmt::Display for InternalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl Error for InternalError {}

impl StorageErrorExt for InternalError {
    fn tonic_status(&self) -> tonic::Status {
        status_with_default_error_info(tonic::Code::Internal, &self.message, "Internal")
    }

    fn http_status(&self) -> StatusCode {
        StatusCode::INTERNAL_SERVER_ERROR
    }

    fn get_size(&self) -> usize {
        std::mem::size_of::<Self>() + self.message.capacity()
    }
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ReadByAddressQueryParams {
    pub download_preference: Option<DownloadPreference>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ReadByIdentityQueryParams {
    pub download_preference: Option<DownloadPreference>,
}

// Only used in the OpenAPI schema
#[allow(dead_code)]
#[derive(ToSchema)]
pub enum WriteResponse {
    Redirect(WriteRedirectResponse),
    Multipart(CreateMultipartUploadResponse),
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct WriteByAddressQueryParams {
    pub upload_preference: Option<String>,
    pub previous_version: Option<String>,
    pub fail_if_exists: Option<bool>,
    pub data_object_size: u64,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct MultipartUploadResponse {
    pub multipart: CreateMultipartUploadResponse,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateMultipartUploadResponse {
    pub upload_id: String,
    pub first_part_write_redirect: WriteRedirectProperties,
    pub maximum_parts_number: Option<u32>,
    pub minimum_size_per_part: Option<u64>,
    pub maximum_size_per_part: Option<u64>,
}

impl TryFrom<crate::traits::fileobject::v1alpha::CreateMultipartUploadResponse> for CreateMultipartUploadResponse {
    type Error = StatusCode;

    fn try_from(
        response: crate::traits::fileobject::v1alpha::CreateMultipartUploadResponse,
    ) -> Result<Self, Self::Error> {
        Ok(Self {
            upload_id: response.upload_id,
            first_part_write_redirect: response
                .first_part_write_redirect
                .map(|w| w.into())
                .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?,
            maximum_parts_number: response.maximum_parts_number,
            minimum_size_per_part: response.minimum_size_per_part,
            maximum_size_per_part: response.maximum_size_per_part,
        })
    }
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct WriteRedirectProperties {
    pub redirect_target_url: String,
    pub method: String,
    pub additional_headers: Vec<HttpHeader>,
    pub completion_header_names: Vec<String>,
}

impl From<crate::traits::fileobject::v1alpha::WriteRedirectProperties> for WriteRedirectProperties {
    fn from(response: crate::traits::fileobject::v1alpha::WriteRedirectProperties) -> Self {
        Self {
            redirect_target_url: response.redirect_target_url,
            method: match response.method {
                1 => "post".to_string(),
                2 => "put".to_string(),
                _ => "unspecified".to_string(),
            },
            additional_headers: response
                .additional_headers
                .into_iter()
                .map(|h| HttpHeader {
                    name: h.name,
                    value: h.value,
                })
                .collect(),
            completion_header_names: response.completion_header_names,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct UploadPartResponse {
    pub part_write_redirects: Vec<WriteRedirectProperties>,
}

impl From<crate::traits::fileobject::v1alpha::UploadPartResponse> for UploadPartResponse {
    fn from(response: crate::traits::fileobject::v1alpha::UploadPartResponse) -> Self {
        Self {
            part_write_redirects: response.part_write_redirects.into_iter().map(|w| w.into()).collect(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct WriteRedirectResponse {
    pub redirect: WriteRedirectProperties,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct DeleteByAddressQueryParams {
    pub previous_version: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AbortMultipartUploadRequestBody {
    pub upload_id: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CompleteMultipartUploadRequestBody {
    pub upload_id: String,
    pub parts: Vec<CompleteMultipartUploadPart>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CompleteMultipartUploadPart {
    pub part_number: u32,
    pub additional_headers: Vec<HttpHeader>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct HttpHeader {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct PrepareMultipartUploadRequestBody {
    pub upload_id: String,
    pub part_number: u32,
    pub part_count: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CompleteRedirectUploadRequestBody {
    pub additional_headers: Vec<HttpHeader>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct EnumerateQueryParams {
    pub continuation_handle: Option<String>,
    pub max_page_size: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct EnumerateResponse {
    pub items: Vec<AddressInfo>,
    pub next_continuation_handle: Option<String>,
}

// Need to convert the UploadPreference enum to a string to serialize it
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct WriteTypeInfo {
    pub minimum_data_object_size: u64,
    pub maximum_data_object_size: u64,
    pub preferred_upload_method: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct FetchWriteTypeInfoResponse {
    pub write_type_intervals: Vec<WriteTypeInfo>,
}

impl From<crate::traits::fileobject::v1alpha::FetchWriteTypeInfoResponse> for FetchWriteTypeInfoResponse {
    fn from(response: crate::traits::fileobject::v1alpha::FetchWriteTypeInfoResponse) -> Self {
        Self {
            write_type_intervals: response.write_type_intervals.into_iter().map(|w| w.into()).collect(),
        }
    }
}

impl From<crate::traits::fileobject::v1alpha::WriteTypeForSizeInterval> for WriteTypeInfo {
    fn from(write_type_info: crate::traits::fileobject::v1alpha::WriteTypeForSizeInterval) -> Self {
        let upload_method = match write_type_info.preferred_upload_method {
            _ if write_type_info.preferred_upload_method == UploadPreference::Body as i32 => "body",
            _ if write_type_info.preferred_upload_method == UploadPreference::Redirect as i32 => "redirect",
            _ if write_type_info.preferred_upload_method == UploadPreference::Multipart as i32 => "multipart",
            _ => "unspecified",
        };
        Self {
            minimum_data_object_size: write_type_info.minimum_data_object_size,
            maximum_data_object_size: write_type_info.maximum_data_object_size,
            preferred_upload_method: upload_method.to_string(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CopyRequestBody {
    pub destination_resource_address: String,
    pub previous_version: Option<String>,
    pub fail_if_exists: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct MoveRequestBody {
    pub destination_resource_address: String,
    pub destination_previous_version: Option<String>,
    pub fail_if_exists: Option<bool>,
    pub source_previous_version: Option<String>,
}
