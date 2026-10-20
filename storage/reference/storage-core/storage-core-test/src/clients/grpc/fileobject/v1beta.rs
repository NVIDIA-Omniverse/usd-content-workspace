// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use async_stream::stream;
use bytes::Bytes;
use storage_core::traits::fileobject::{
    v1alpha::{
        read_from_address_response, read_response, write_response::WriteResponseType, AbortMultipartUploadRequest,
        AbortMultipartUploadResponse, AddressInfo, Chunk, CompleteMultipartUploadRequest,
        CompleteMultipartUploadResponse, CompleteRedirectUploadRequest, CompleteRedirectUploadResponse,
        CreateMultipartUploadResponse, DeleteRequest, DeleteResponse, EnumerateRequest, EnumerateResponse,
        FetchWriteTypeInfoRequest, FetchWriteTypeInfoResponse, Header, Metadata, ReadFromAddressRequest,
        ReadFromAddressResponse, ReadRequest, ReadResponse, Redirect, ResourceIdentity, ResourceInfo, StatRequest,
        StatResponse, UploadPartRequest, UploadPartResponse, WriteChunksAccepted, WriteParameters,
        WriteRedirectProperties, WriteResponse, WriteTypeForSizeInterval,
    },
    v1beta,
};
use tokio::sync::{mpsc, oneshot};
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::Channel;
use tonic::Status;

use super::super::{connect_with_retries, request_with_headers_config};
use crate::clients::traits::fileobject::FileObject;
use crate::headers::HeadersConfig;

pub struct FileObjectClient {
    client: v1beta::file_object_service_client::FileObjectServiceClient<Channel>,
    headers_config: HeadersConfig,
}

impl FileObjectClient {
    pub async fn new(address: impl AsRef<str>, headers_config: HeadersConfig) -> Self {
        let client = connect_with_retries(|| {
            v1beta::file_object_service_client::FileObjectServiceClient::connect(address.as_ref().to_owned())
        })
        .await;
        Self { client, headers_config }
    }
}

#[tonic::async_trait]
impl FileObject for FileObjectClient {
    async fn stat(&mut self, request: StatRequest) -> Result<StatResponse, Status> {
        // Convert v1alpha request to v1beta request
        let v1beta_request = v1beta::StatRequest {
            resource_address: request.resource_address,
        };

        let response = self
            .client
            .stat(request_with_headers_config(v1beta_request, &self.headers_config))
            .await
            .map(|response| response.into_inner())?;

        // Convert v1beta response back to v1alpha response
        Ok(StatResponse {
            resource_info: response.resource_info.map(|r| ResourceInfo {
                resource_identity: r.resource_identity.map(|r| ResourceIdentity {
                    encoded_identity: r.encoded_identity,
                }),
                metadata: r.metadata.map(|m| Metadata {
                    native_metadata: Default::default(),
                    data_object_size: m.data_object_size,
                    last_modified_timestamp: m.last_modified_timestamp,
                }),
            }),
        })
    }

    async fn read_from_address(
        &mut self,
        request: ReadFromAddressRequest,
    ) -> Result<ReceiverStream<Result<ReadFromAddressResponse, Status>>, Status> {
        // Convert v1alpha request to v1beta request
        let v1beta_request = v1beta::ReadFromAddressRequest {
            resource_address: request.resource_address,
            download_preference: request.download_preference,
        };

        let (tx, rx) = mpsc::channel(1);
        let mut stream = self
            .client
            .read_from_address(request_with_headers_config(v1beta_request, &self.headers_config))
            .await?
            .into_inner();

        tokio::spawn(async move {
            loop {
                let response = stream.message().await;
                match response {
                    Ok(Some(response)) => {
                        // Convert v1beta response back to v1alpha response
                        let v1alpha_response = ReadFromAddressResponse {
                            reply_type: response.reply_type.map(|reply_type| match reply_type {
                                v1beta::read_from_address_response::ReplyType::ResourceInfo(r) => {
                                    read_from_address_response::ReplyType::ResourceInfo(ResourceInfo {
                                        resource_identity: r.resource_identity.map(|r| ResourceIdentity {
                                            encoded_identity: r.encoded_identity,
                                        }),
                                        metadata: r.metadata.map(|m| Metadata {
                                            native_metadata: Default::default(),
                                            data_object_size: m.data_object_size,
                                            last_modified_timestamp: m.last_modified_timestamp,
                                        }),
                                    })
                                },
                                v1beta::read_from_address_response::ReplyType::Chunk(chunk) => {
                                    read_from_address_response::ReplyType::Chunk(Chunk { chunk: chunk.chunk })
                                },
                                v1beta::read_from_address_response::ReplyType::Redirect(redirect) => {
                                    read_from_address_response::ReplyType::Redirect(Redirect {
                                        redirect_target_url: redirect.redirect_target_url,
                                        method: redirect.method,
                                        additional_headers: redirect
                                            .additional_headers
                                            .into_iter()
                                            .map(|h| Header {
                                                name: h.name,
                                                value: h.value,
                                            })
                                            .collect(),
                                    })
                                },
                            }),
                        };
                        let _ = tx.send(Ok(v1alpha_response)).await;
                    },
                    Ok(None) => break,
                    Err(e) => {
                        let _ = tx.send(Err(e)).await;
                        break;
                    },
                }
            }
        });
        Ok(ReceiverStream::new(rx))
    }

    async fn read(&mut self, request: ReadRequest) -> Result<ReceiverStream<Result<ReadResponse, Status>>, Status> {
        // Convert v1alpha request to v1beta request
        let v1beta_request = v1beta::ReadRequest {
            resource_identity: request.resource_identity.map(|r| v1beta::ResourceIdentity {
                encoded_identity: r.encoded_identity,
            }),
            download_preference: request.download_preference,
        };

        let (tx, rx) = mpsc::channel(1);
        let mut stream = self
            .client
            .read(request_with_headers_config(v1beta_request, &self.headers_config))
            .await?
            .into_inner();

        tokio::spawn(async move {
            loop {
                let response = stream.message().await;
                match response {
                    Ok(Some(response)) => {
                        // Convert v1beta response back to v1alpha response
                        let v1alpha_response = ReadResponse {
                            reply_type: response.reply_type.map(|reply_type| match reply_type {
                                v1beta::read_response::ReplyType::Metadata(m) => {
                                    read_response::ReplyType::Metadata(Metadata {
                                        native_metadata: Default::default(),
                                        data_object_size: m.data_object_size,
                                        last_modified_timestamp: m.last_modified_timestamp,
                                    })
                                },
                                v1beta::read_response::ReplyType::Chunk(chunk) => {
                                    read_response::ReplyType::Chunk(Chunk { chunk: chunk.chunk })
                                },
                                v1beta::read_response::ReplyType::Redirect(redirect) => {
                                    read_response::ReplyType::Redirect(Redirect {
                                        redirect_target_url: redirect.redirect_target_url,
                                        method: redirect.method,
                                        additional_headers: redirect
                                            .additional_headers
                                            .into_iter()
                                            .map(|h| Header {
                                                name: h.name,
                                                value: h.value,
                                            })
                                            .collect(),
                                    })
                                },
                            }),
                        };
                        let _ = tx.send(Ok(v1alpha_response)).await;
                    },
                    Ok(None) => break,
                    Err(e) => {
                        let _ = tx.send(Err(e)).await;
                        break;
                    },
                }
            }
        });
        Ok(ReceiverStream::new(rx))
    }

    async fn enumerate(
        &mut self,
        request: EnumerateRequest,
    ) -> Result<ReceiverStream<Result<EnumerateResponse, Status>>, Status> {
        // Convert v1alpha request to v1beta request
        let v1beta_request = v1beta::EnumerateRequest {
            resource_address: request.resource_address,
        };

        let (tx, rx) = mpsc::channel(1);
        let mut stream = self
            .client
            .enumerate(request_with_headers_config(v1beta_request, &self.headers_config))
            .await?
            .into_inner();

        tokio::spawn(async move {
            loop {
                let response = stream.message().await;
                match response {
                    Ok(Some(response)) => {
                        // Convert v1beta response back to v1alpha response
                        let v1alpha_response = EnumerateResponse {
                            items: response
                                .items
                                .into_iter()
                                .map(|i| AddressInfo {
                                    resource_address: i.resource_address,
                                    metadata: i.metadata.map(|m| Metadata {
                                        native_metadata: Default::default(),
                                        data_object_size: m.data_object_size,
                                        last_modified_timestamp: m.last_modified_timestamp,
                                    }),
                                })
                                .collect(),
                        };
                        let _ = tx.send(Ok(v1alpha_response)).await;
                    },
                    Ok(None) => break,
                    Err(e) => {
                        let _ = tx.send(Err(e)).await;
                        break;
                    },
                }
            }
        });
        Ok(ReceiverStream::new(rx))
    }

    async fn write(&mut self, request: WriteParameters, data: Option<Bytes>) -> Result<WriteResponse, Status> {
        let mut has_body = data.is_some();

        // Convert v1alpha request to v1beta request
        let v1beta_request = v1beta::WriteParameters {
            destination_resource_address: request.destination_resource_address,
            previous_version: request.previous_version.map(|v| v1beta::ResourceIdentity {
                encoded_identity: v.encoded_identity,
            }),
            fail_if_exists: request.fail_if_exists,
            data_object_size: request.data_object_size,
            upload_preference: request.upload_preference,
        };

        let (data_tx, data_rx) = oneshot::channel();
        let write_tx_stream = stream! {
            // Send the url
            yield v1beta::WriteRequest {
                write_request_type: Some(v1beta::write_request::WriteRequestType::Params(v1beta_request)),
            };

            if let Some(data) = data {
                // Wait for the server to tell us to send data
                data_rx.await.unwrap();

                // Send the data
                for chunk in data.chunks(1024) {
                    yield v1beta::WriteRequest { write_request_type: Some(v1beta::write_request::WriteRequestType::Chunk(v1beta::Chunk{ chunk: Bytes::copy_from_slice(chunk) })) };
                }
            }
        };
        let mut write_rx_stream = self
            .client
            .write(request_with_headers_config(write_tx_stream, &self.headers_config))
            .await?
            .into_inner();

        let mut data_tx = Some(data_tx);
        loop {
            let response = write_rx_stream.message().await;
            match response {
                Ok(Some(response)) => {
                    tracing::debug!(?response);
                    if has_body {
                        assert_eq!(
                            response.write_response_type,
                            Some(v1beta::write_response::WriteResponseType::WriteChunksAccepted(
                                v1beta::WriteChunksAccepted {}
                            ))
                        );
                        let tx = data_tx.take().unwrap();
                        let _ = tx.send(());
                        has_body = false;
                    } else {
                        return Ok(WriteResponse {
                            write_response_type: response.write_response_type.map(
                                |response_type| match response_type {
                                    v1beta::write_response::WriteResponseType::WriteChunksAccepted(
                                        v1beta::WriteChunksAccepted {},
                                    ) => WriteResponseType::WriteChunksAccepted(WriteChunksAccepted {}),
                                    v1beta::write_response::WriteResponseType::ResourceInfo(resource_info) => {
                                        WriteResponseType::ResourceInfo(ResourceInfo {
                                            resource_identity: resource_info.resource_identity.map(|r| {
                                                ResourceIdentity {
                                                    encoded_identity: r.encoded_identity,
                                                }
                                            }),
                                            metadata: resource_info.metadata.map(|m| Metadata {
                                                native_metadata: Default::default(),
                                                data_object_size: m.data_object_size,
                                                last_modified_timestamp: m.last_modified_timestamp,
                                            }),
                                        })
                                    },
                                    v1beta::write_response::WriteResponseType::WriteRedirect(write_redirect) => {
                                        WriteResponseType::WriteRedirect(WriteRedirectProperties {
                                            redirect_target_url: write_redirect.redirect_target_url,
                                            method: write_redirect.method,
                                            additional_headers: write_redirect
                                                .additional_headers
                                                .into_iter()
                                                .map(|h| Header {
                                                    name: h.name,
                                                    value: h.value,
                                                })
                                                .collect(),
                                            completion_header_names: write_redirect.completion_header_names,
                                        })
                                    },
                                    v1beta::write_response::WriteResponseType::MultipartUpload(multipart_upload) => {
                                        WriteResponseType::MultipartUpload(CreateMultipartUploadResponse {
                                            upload_id: multipart_upload.upload_id,
                                            first_part_write_redirect: multipart_upload.first_part_write_redirect.map(
                                                |w| WriteRedirectProperties {
                                                    redirect_target_url: w.redirect_target_url,
                                                    method: w.method,
                                                    additional_headers: w
                                                        .additional_headers
                                                        .into_iter()
                                                        .map(|h| Header {
                                                            name: h.name,
                                                            value: h.value,
                                                        })
                                                        .collect(),
                                                    completion_header_names: w.completion_header_names,
                                                },
                                            ),
                                            maximum_parts_number: multipart_upload.maximum_parts_number,
                                            minimum_size_per_part: multipart_upload.minimum_size_per_part,
                                            maximum_size_per_part: multipart_upload.maximum_size_per_part,
                                        })
                                    },
                                },
                            ),
                        });
                    }
                },
                Ok(None) => return Err(Status::internal("Server closed stream")),
                Err(e) => {
                    return Err(e);
                },
            }
        }
    }

    async fn delete(&mut self, request: DeleteRequest) -> Result<DeleteResponse, Status> {
        // Convert v1alpha request to v1beta request
        let v1beta_request = v1beta::DeleteRequest {
            resource_address: request.resource_address,
            previous_version: request.previous_version.map(|v| v1beta::ResourceIdentity {
                encoded_identity: v.encoded_identity,
            }),
        };

        self.client
            .delete(request_with_headers_config(v1beta_request, &self.headers_config))
            .await
            .map(|response| response.into_inner())?;

        Ok(DeleteResponse {})
    }

    async fn abort_multipart_upload(
        &mut self,
        request: AbortMultipartUploadRequest,
    ) -> Result<AbortMultipartUploadResponse, Status> {
        // Convert v1alpha request to v1beta request
        let v1beta_request = v1beta::AbortMultipartUploadRequest {
            upload_id: request.upload_id,
            destination_resource_address: request.destination_resource_address,
        };

        self.client
            .abort_multipart_upload(request_with_headers_config(v1beta_request, &self.headers_config))
            .await
            .map(|response| response.into_inner())?;

        // Convert v1beta response back to v1alpha response
        Ok(AbortMultipartUploadResponse {})
    }

    async fn complete_multipart_upload(
        &mut self,
        request: CompleteMultipartUploadRequest,
    ) -> Result<CompleteMultipartUploadResponse, Status> {
        // Convert v1alpha request to v1beta request
        let v1beta_request = v1beta::CompleteMultipartUploadRequest {
            upload_id: request.upload_id,
            destination_resource_address: request.destination_resource_address,
            parts: request
                .parts
                .into_iter()
                .map(|p| v1beta::CompletedUploadPart {
                    part_number: p.part_number,
                    headers: p
                        .headers
                        .into_iter()
                        .map(|h| v1beta::Header {
                            name: h.name,
                            value: h.value,
                        })
                        .collect(),
                })
                .collect(),
        };

        let response = self
            .client
            .complete_multipart_upload(request_with_headers_config(v1beta_request, &self.headers_config))
            .await
            .map(|response| response.into_inner())?;

        // Convert v1beta response back to v1alpha response
        Ok(CompleteMultipartUploadResponse {
            resource_info: response.resource_info.map(|r| ResourceInfo {
                resource_identity: r.resource_identity.map(|r| ResourceIdentity {
                    encoded_identity: r.encoded_identity,
                }),
                metadata: r.metadata.map(|m| Metadata {
                    native_metadata: Default::default(),
                    data_object_size: m.data_object_size,
                    last_modified_timestamp: m.last_modified_timestamp,
                }),
            }),
        })
    }

    async fn upload_part(&mut self, request: UploadPartRequest) -> Result<UploadPartResponse, Status> {
        // Convert v1alpha request to v1beta request
        let v1beta_request = v1beta::UploadPartRequest {
            upload_id: request.upload_id,
            destination_resource_address: request.destination_resource_address,
            part_number: request.part_number,
            part_count: request.part_count,
        };

        let response = self
            .client
            .upload_part(request_with_headers_config(v1beta_request, &self.headers_config))
            .await
            .map(|response| response.into_inner())?;

        // Convert v1beta response back to v1alpha response
        Ok(UploadPartResponse {
            part_write_redirects: response
                .part_write_redirects
                .into_iter()
                .map(|w| WriteRedirectProperties {
                    redirect_target_url: w.redirect_target_url,
                    method: w.method,
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
        // Convert v1alpha request to v1beta request
        let v1beta_request = v1beta::CompleteRedirectUploadRequest {
            destination_resource_address: request.destination_resource_address,
            additional_headers: request
                .additional_headers
                .into_iter()
                .map(|h| v1beta::Header {
                    name: h.name,
                    value: h.value,
                })
                .collect(),
        };

        let response = self
            .client
            .complete_redirect_upload(request_with_headers_config(v1beta_request, &self.headers_config))
            .await
            .map(|response| response.into_inner())?;

        // Convert v1beta response back to v1alpha response
        Ok(CompleteRedirectUploadResponse {
            resource_info: response.resource_info.map(|r| ResourceInfo {
                resource_identity: r.resource_identity.map(|r| ResourceIdentity {
                    encoded_identity: r.encoded_identity,
                }),
                metadata: r.metadata.map(|m| Metadata {
                    native_metadata: Default::default(),
                    data_object_size: m.data_object_size,
                    last_modified_timestamp: m.last_modified_timestamp,
                }),
            }),
        })
    }

    async fn fetch_write_type_info(
        &mut self,
        request: FetchWriteTypeInfoRequest,
    ) -> Result<FetchWriteTypeInfoResponse, Status> {
        // Convert v1alpha request to v1beta request
        let v1beta_request = v1beta::FetchWriteTypeInfoRequest {
            destination_resource_address: request.destination_resource_address,
        };

        let response = self
            .client
            .fetch_write_type_info(request_with_headers_config(v1beta_request, &self.headers_config))
            .await
            .map(|response| response.into_inner())?;

        // Convert v1beta response back to v1alpha response
        Ok(FetchWriteTypeInfoResponse {
            write_type_intervals: response
                .write_type_intervals
                .into_iter()
                .map(|w| WriteTypeForSizeInterval {
                    minimum_data_object_size: w.minimum_data_object_size,
                    maximum_data_object_size: w.maximum_data_object_size,
                    preferred_upload_method: w.preferred_upload_method,
                })
                .collect(),
        })
    }
}
