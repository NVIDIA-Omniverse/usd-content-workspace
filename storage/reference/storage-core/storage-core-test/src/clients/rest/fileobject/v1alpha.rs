// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use bytes::Bytes;
use http::StatusCode;
use serde_json::Value;
use storage_core::traits::fileobject::v1alpha::{
    read_from_address_response, read_response, write_response::WriteResponseType, AbortMultipartUploadRequest,
    AbortMultipartUploadResponse, Chunk, CompleteMultipartUploadRequest, CompleteMultipartUploadResponse,
    CompleteRedirectUploadRequest, CompleteRedirectUploadResponse, CopyRequest, CopyResponse,
    CreateMultipartUploadResponse, DeleteRequest, DeleteResponse, DownloadPreference, EnumerateRequest,
    EnumerateResponse, FetchWriteTypeInfoRequest, FetchWriteTypeInfoResponse, GetNativeMetadataSupportRequest,
    GetNativeMetadataSupportResponse, GetOptimisticLockingSupportRequest, GetOptimisticLockingSupportResponse, Header,
    IsReadOnlyRequest, IsReadOnlyResponse, Metadata, MoveRequest, MoveResponse, ReadFromAddressRequest,
    ReadFromAddressResponse, ReadRequest, ReadResponse, Redirect, ResourceIdentity, ResourceInfo, StatRequest,
    StatResponse, UploadPartRequest, UploadPartResponse, UploadPreference, WriteParameters, WriteRedirectProperties,
    WriteResponse, WriteTypeForSizeInterval,
};
use storage_core_builder::api_services::fileobject::v1alpha::types::{
    AbortMultipartUploadRequestBody, CompleteMultipartUploadPart, CompleteMultipartUploadRequestBody,
    CompleteRedirectUploadRequestBody, CopyRequestBody, HttpHeader, MoveRequestBody, PrepareMultipartUploadRequestBody,
};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::Status;
use tracing::debug;
use urlencoding::encode;

use super::FileObjectRestClient;
use crate::clients::traits::fileobject::FileObject;
use crate::headers::HeadersConfig;

pub struct FileObjectClient {
    client: FileObjectRestClient,
}

impl FileObjectClient {
    pub fn new(port: u16, headers_config: HeadersConfig) -> Self {
        Self {
            client: FileObjectRestClient::new(port, headers_config),
        }
    }
}

#[tonic::async_trait]
impl FileObject for FileObjectClient {
    async fn stat(&mut self, request: StatRequest) -> Result<StatResponse, Status> {
        if request.resource_address.is_empty() {
            return Err(Status::invalid_argument("Resource address is required"));
        }
        let path = format!("v1alpha/fileobject/by-address/{}", encode(&request.resource_address));

        let response = self.client.stat(&path).await?;
        let headers = response.headers();

        let metadata: Metadata = serde_json::from_str(
            headers
                .get("x-nvidia-omniverse-storage-metadata")
                .unwrap()
                .to_str()
                .unwrap(),
        )
        .unwrap();

        let resource_identity = ResourceIdentity {
            encoded_identity: headers
                .get("x-nvidia-omniverse-storage-resource-identity")
                .unwrap()
                .to_str()
                .unwrap()
                .to_string(),
        };
        Ok(StatResponse {
            resource_info: Some(ResourceInfo {
                metadata: Some(metadata),
                resource_identity: Some(resource_identity),
            }),
        })
    }

    async fn read_from_address(
        &mut self,
        request: ReadFromAddressRequest,
    ) -> Result<ReceiverStream<Result<ReadFromAddressResponse, Status>>, Status> {
        let (tx, rx) = mpsc::channel(1);
        if request.resource_address.is_empty() {
            return Err(Status::invalid_argument("Resource address is required"));
        }
        let path = format!("v1alpha/fileobject/by-address/{}", encode(&request.resource_address));

        let download_preference = if let Some(download_preference) = request.download_preference {
            if download_preference == DownloadPreference::Body as i32 {
                Some("body")
            } else if download_preference == DownloadPreference::Redirect as i32 {
                Some("redirect")
            } else {
                return Err(Status::invalid_argument("Invalid download preference"));
            }
        } else {
            None
        };

        let response = self.client.read_from_address(&path, download_preference).await?;

        tokio::spawn(async move {
            // First send the metadata
            let headers = response.headers();
            let metadata: Metadata = serde_json::from_str(
                headers
                    .get("x-nvidia-omniverse-storage-metadata")
                    .unwrap()
                    .to_str()
                    .unwrap(),
            )
            .unwrap();

            let resource_identity = ResourceIdentity {
                encoded_identity: headers
                    .get("x-nvidia-omniverse-storage-resource-identity")
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_string(),
            };

            let _ = tx
                .send(Ok(ReadFromAddressResponse {
                    reply_type: Some(read_from_address_response::ReplyType::ResourceInfo(ResourceInfo {
                        metadata: Some(metadata),
                        resource_identity: Some(resource_identity),
                    })),
                }))
                .await;

            let status = response.status();
            let body = response.bytes().await.unwrap();
            match status {
                // Redirect response
                reqwest::StatusCode::MULTIPLE_CHOICES => {
                    let redirect: Redirect = serde_json::from_slice(&body).unwrap();
                    let _ = tx
                        .send(Ok(ReadFromAddressResponse {
                            reply_type: Some(read_from_address_response::ReplyType::Redirect(redirect)),
                        }))
                        .await;
                },
                // Data response, gather data and send it
                reqwest::StatusCode::OK => {
                    let _ = tx
                        .send(Ok(ReadFromAddressResponse {
                            reply_type: Some(read_from_address_response::ReplyType::Chunk(Chunk { chunk: body })),
                        }))
                        .await;
                },
                _ => panic!("Unexpected status code: {status}"),
            }
        });
        Ok(ReceiverStream::new(rx))
    }

    async fn read(&mut self, request: ReadRequest) -> Result<ReceiverStream<Result<ReadResponse, Status>>, Status> {
        let (tx, rx) = mpsc::channel(1);

        if request.resource_identity.is_none()
            || request.resource_identity.as_ref().unwrap().encoded_identity.is_empty()
        {
            return Err(Status::invalid_argument("Resource identity is required"));
        }

        let path = format!(
            "v1alpha/fileobject/by-identity/{}",
            encode(
                &request
                    .resource_identity
                    .ok_or(Status::invalid_argument("Resource identity is required"))?
                    .encoded_identity
            )
        );
        let download_preference = if let Some(download_preference) = request.download_preference {
            if download_preference == DownloadPreference::Body as i32 {
                Some("body")
            } else if download_preference == DownloadPreference::Redirect as i32 {
                Some("redirect")
            } else {
                return Err(Status::invalid_argument("Invalid download preference"));
            }
        } else {
            None
        };
        let response = self.client.read(&path, download_preference).await?;

        tokio::spawn(async move {
            // First send the metadata
            let headers = response.headers();
            let metadata: Metadata = serde_json::from_str(
                headers
                    .get("x-nvidia-omniverse-storage-metadata")
                    .unwrap()
                    .to_str()
                    .unwrap(),
            )
            .unwrap();
            let _ = tx
                .send(Ok(ReadResponse {
                    reply_type: Some(read_response::ReplyType::Metadata(metadata)),
                }))
                .await;

            let status = response.status();
            let body = response.bytes().await.unwrap();
            match status {
                // Redirect response
                reqwest::StatusCode::MULTIPLE_CHOICES => {
                    let redirect: Redirect = serde_json::from_slice(&body).unwrap();
                    let _ = tx
                        .send(Ok(ReadResponse {
                            reply_type: Some(read_response::ReplyType::Redirect(redirect)),
                        }))
                        .await;
                },
                // Data response, gather data and send it
                reqwest::StatusCode::OK => {
                    let _ = tx
                        .send(Ok(ReadResponse {
                            reply_type: Some(read_response::ReplyType::Chunk(Chunk { chunk: body })),
                        }))
                        .await;
                },
                _ => panic!("Unexpected status code: {status}"),
            }
        });
        Ok(ReceiverStream::new(rx))
    }

    async fn enumerate(
        &mut self,
        request: EnumerateRequest,
    ) -> Result<ReceiverStream<Result<EnumerateResponse, Status>>, Status> {
        let (tx, rx) = mpsc::channel(1);
        let mut client = self.client.clone();
        tokio::spawn(async move {
            let base_path = format!("v1alpha/fileobject/data-objects/{}", encode(&request.resource_address));

            let mut continuation_handle: Option<String> = None;
            loop {
                let path = base_path.clone();
                let response = match client.enumerate(&path, continuation_handle.as_deref()).await {
                    Ok(response) => response,
                    Err(e) => {
                        let _ = tx.send(Err(e)).await;
                        return;
                    },
                };

                debug!(?path, ?response);

                let result: storage_core_builder::api_services::fileobject::v1alpha::types::EnumerateResponse =
                    serde_json::from_slice(&response).unwrap();
                let _ = tx.send(Ok(EnumerateResponse { items: result.items })).await;
                if let Some(next_continuation_handle) = result.next_continuation_handle {
                    continuation_handle = Some(next_continuation_handle);
                } else {
                    break;
                }
            }
        });
        Ok(ReceiverStream::new(rx))
    }

    async fn write(&mut self, request: WriteParameters, data: Option<Bytes>) -> Result<WriteResponse, Status> {
        let path = format!(
            "v1alpha/fileobject/by-address/{}",
            encode(&request.destination_resource_address)
        );

        let upload_preference = if let Some(upload_preference) = request.upload_preference {
            if upload_preference == UploadPreference::Body as i32 {
                Some("body")
            } else if upload_preference == UploadPreference::Redirect as i32 {
                Some("redirect")
            } else if upload_preference == UploadPreference::Multipart as i32 {
                Some("multipart")
            } else {
                None
            }
        } else {
            None
        };

        let response = self
            .client
            .write(
                &path,
                request.data_object_size,
                upload_preference,
                request.previous_version.as_ref().map(|v| v.encoded_identity.as_str()),
                request.fail_if_exists,
                data,
            )
            .await?;

        let status = response.status();
        let body = response.bytes().await.unwrap();
        match status {
            StatusCode::CREATED => {
                let response: ResourceInfo = serde_json::from_slice(&body).unwrap();
                Ok(WriteResponse {
                    write_response_type: Some(WriteResponseType::ResourceInfo(response)),
                })
            },
            StatusCode::MULTIPLE_CHOICES => {
                let response: Value = serde_json::from_slice(&body).unwrap();
                if let Some(redirect) = response.get("redirect") {
                    let redirect: storage_core_builder::api_services::fileobject::v1alpha::types::WriteRedirectProperties =
                        serde_json::from_value(redirect.clone()).unwrap();
                    return Ok(WriteResponse {
                        write_response_type: Some(WriteResponseType::WriteRedirect(WriteRedirectProperties {
                            redirect_target_url: redirect.redirect_target_url,
                            method: match redirect.method.as_str() {
                                "post" => 1,
                                "put" => 2,
                                _ => 0,
                            },
                            additional_headers: redirect
                                .additional_headers
                                .into_iter()
                                .map(|h| Header {
                                    name: h.name,
                                    value: h.value,
                                })
                                .collect(),
                            completion_header_names: redirect.completion_header_names,
                        })),
                    });
                } else if let Some(multipart) = response.get("multipart") {
                    let multipart: storage_core_builder::api_services::fileobject::v1alpha::types::CreateMultipartUploadResponse =
                        serde_json::from_value(multipart.clone()).unwrap();
                    return Ok(WriteResponse {
                        write_response_type: Some(WriteResponseType::MultipartUpload(CreateMultipartUploadResponse {
                            upload_id: multipart.upload_id,
                            first_part_write_redirect: Some(WriteRedirectProperties {
                                redirect_target_url: multipart.first_part_write_redirect.redirect_target_url,
                                method: match multipart.first_part_write_redirect.method.as_str() {
                                    "post" => 1,
                                    "put" => 2,
                                    _ => 0,
                                },
                                additional_headers: multipart
                                    .first_part_write_redirect
                                    .additional_headers
                                    .into_iter()
                                    .map(|h| Header {
                                        name: h.name,
                                        value: h.value,
                                    })
                                    .collect(),
                                completion_header_names: multipart.first_part_write_redirect.completion_header_names,
                            }),
                            maximum_parts_number: multipart.maximum_parts_number,
                            minimum_size_per_part: multipart.minimum_size_per_part,
                            maximum_size_per_part: multipart.maximum_size_per_part,
                        })),
                    });
                } else {
                    return Err(Status::internal("Unexpected status code"));
                }
            },
            _ => panic!("Unexpected status code: {status}"),
        }
    }

    async fn delete(&mut self, request: DeleteRequest) -> Result<DeleteResponse, Status> {
        let path = format!("v1alpha/fileobject/by-address/{}", encode(&request.resource_address));

        self.client
            .delete(
                &path,
                request.previous_version.as_ref().map(|v| v.encoded_identity.as_str()),
            )
            .await?;

        Ok(DeleteResponse {})
    }

    async fn abort_multipart_upload(
        &mut self,
        request: AbortMultipartUploadRequest,
    ) -> Result<AbortMultipartUploadResponse, Status> {
        let path = format!(
            "v1alpha/fileobject/by-address/{}/multipart/abort",
            encode(&request.destination_resource_address)
        );

        let body = AbortMultipartUploadRequestBody {
            upload_id: request.upload_id,
        };
        self.client.abort_multipart_upload(&path, body).await?;

        Ok(AbortMultipartUploadResponse {})
    }

    async fn complete_multipart_upload(
        &mut self,
        request: CompleteMultipartUploadRequest,
    ) -> Result<CompleteMultipartUploadResponse, Status> {
        let path = format!(
            "v1alpha/fileobject/by-address/{}/multipart/complete",
            encode(&request.destination_resource_address)
        );

        let body = CompleteMultipartUploadRequestBody {
            upload_id: request.upload_id,
            parts: request
                .parts
                .into_iter()
                .map(|p| CompleteMultipartUploadPart {
                    part_number: p.part_number,
                    additional_headers: p
                        .headers
                        .into_iter()
                        .map(|h| HttpHeader {
                            name: h.name,
                            value: h.value,
                        })
                        .collect(),
                })
                .collect(),
        };

        let response = self.client.complete_multipart_upload(&path, body).await?;

        let resource_info: ResourceInfo = serde_json::from_slice(&response).unwrap();
        Ok(CompleteMultipartUploadResponse {
            resource_info: Some(resource_info),
        })
    }

    async fn upload_part(&mut self, request: UploadPartRequest) -> Result<UploadPartResponse, Status> {
        let path = format!(
            "v1alpha/fileobject/by-address/{}/multipart/prepare",
            encode(&request.destination_resource_address)
        );

        let body = PrepareMultipartUploadRequestBody {
            upload_id: request.upload_id,
            part_number: request.part_number,
            part_count: request.part_count,
        };

        let response = self.client.upload_part(&path, body).await?;

        let response: storage_core_builder::api_services::fileobject::v1alpha::types::UploadPartResponse =
            serde_json::from_slice(&response).unwrap();
        Ok(UploadPartResponse {
            part_write_redirects: response
                .part_write_redirects
                .into_iter()
                .map(|w| WriteRedirectProperties {
                    redirect_target_url: w.redirect_target_url,
                    method: match w.method.as_str() {
                        "post" => 1,
                        "put" => 2,
                        _ => 0,
                    },
                    additional_headers: w
                        .additional_headers
                        .into_iter()
                        .map(|h| Header {
                            name: h.name,
                            value: h.value,
                        })
                        .collect(),
                    completion_header_names: w.completion_header_names,
                })
                .collect(),
        })
    }

    async fn complete_redirect_upload(
        &mut self,
        request: CompleteRedirectUploadRequest,
    ) -> Result<CompleteRedirectUploadResponse, Status> {
        let path = format!(
            "v1alpha/fileobject/by-address/{}/redirect/complete",
            encode(&request.destination_resource_address)
        );

        let body = CompleteRedirectUploadRequestBody {
            additional_headers: request
                .additional_headers
                .into_iter()
                .map(|h| HttpHeader {
                    name: h.name,
                    value: h.value,
                })
                .collect(),
        };

        let response = self.client.complete_redirect_upload(&path, body).await?;

        let resource_info: ResourceInfo = serde_json::from_slice(&response).unwrap();
        Ok(CompleteRedirectUploadResponse {
            resource_info: Some(resource_info),
        })
    }

    async fn fetch_write_type_info(
        &mut self,
        request: FetchWriteTypeInfoRequest,
    ) -> Result<FetchWriteTypeInfoResponse, Status> {
        let path = format!(
            "v1alpha/fileobject/upload-options/by-address/{}",
            encode(&request.destination_resource_address)
        );

        let body = self.client.fetch_write_type_info(&path).await?;
        let response: storage_core_builder::api_services::fileobject::v1alpha::types::FetchWriteTypeInfoResponse =
            serde_json::from_slice(&body).unwrap();
        Ok(FetchWriteTypeInfoResponse {
            write_type_intervals: response
                .write_type_intervals
                .into_iter()
                .map(|w| WriteTypeForSizeInterval {
                    minimum_data_object_size: w.minimum_data_object_size,
                    maximum_data_object_size: w.maximum_data_object_size,
                    preferred_upload_method: match w.preferred_upload_method.as_str() {
                        "body" => UploadPreference::Body as i32,
                        "redirect" => UploadPreference::Redirect as i32,
                        "multipart" => UploadPreference::Multipart as i32,
                        _ => UploadPreference::Unspecified as i32,
                    },
                })
                .collect(),
        })
    }

    async fn get_optimistic_locking_support(
        &mut self,
        request: GetOptimisticLockingSupportRequest,
    ) -> Result<GetOptimisticLockingSupportResponse, Status> {
        let path = format!(
            "v1alpha/fileobject/optimistic-locking-support/{}",
            encode(&request.resource_address)
        );

        let body = self.client.get_optimistic_locking_support(&path).await?;
        Ok(serde_json::from_slice(&body).unwrap())
    }

    async fn get_native_metadata_support(
        &mut self,
        request: GetNativeMetadataSupportRequest,
    ) -> Result<GetNativeMetadataSupportResponse, Status> {
        let path = format!(
            "v1alpha/fileobject/native-metadata-support/{}",
            encode(&request.resource_address)
        );

        let body = self.client.get_native_metadata_support(&path).await?;
        Ok(serde_json::from_slice(&body).unwrap())
    }

    async fn is_read_only(&mut self, request: IsReadOnlyRequest) -> Result<IsReadOnlyResponse, Status> {
        let path = format!("v1alpha/fileobject/is-read-only/{}", encode(&request.resource_address));

        let body = self.client.is_read_only(&path).await?;
        Ok(serde_json::from_slice(&body).unwrap())
    }

    async fn copy(&mut self, request: CopyRequest) -> Result<CopyResponse, Status> {
        let path = format!(
            "v1alpha/fileobject/by-identity/{}/copy",
            encode(&request.source_resource_identity.unwrap().encoded_identity)
        );

        let body = CopyRequestBody {
            destination_resource_address: request.destination_resource_address,
            previous_version: request.previous_version.map(|v| v.encoded_identity),
            fail_if_exists: request.fail_if_exists,
        };

        let body = self.client.copy(&path, body).await?;

        Ok(serde_json::from_slice(&body).unwrap())
    }

    async fn move_object(&mut self, request: MoveRequest) -> Result<MoveResponse, Status> {
        let path = format!(
            "v1alpha/fileobject/by-address/{}/move",
            encode(&request.source_resource_address)
        );

        let body = MoveRequestBody {
            destination_resource_address: request.destination_resource_address,
            destination_previous_version: request.destination_previous_version.map(|v| v.encoded_identity),
            fail_if_exists: request.fail_if_exists,
            source_previous_version: request.source_previous_version.map(|v| v.encoded_identity),
        };

        let body = self.client.move_object(&path, body).await?;

        Ok(serde_json::from_slice(&body).unwrap())
    }
}
