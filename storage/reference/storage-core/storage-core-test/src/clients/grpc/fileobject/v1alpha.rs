// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use async_stream::stream;
use bytes::Bytes;
use storage_core::traits::fileobject::v1alpha::file_object_service_client::FileObjectServiceClient;
use storage_core::traits::fileobject::v1alpha::write_request::WriteRequestType;
use storage_core::traits::fileobject::v1alpha::write_response::WriteResponseType;
use storage_core::traits::fileobject::v1alpha::{
    AbortMultipartUploadRequest, AbortMultipartUploadResponse, Chunk, CompleteMultipartUploadRequest,
    CompleteMultipartUploadResponse, CompleteRedirectUploadRequest, CompleteRedirectUploadResponse, CopyRequest,
    CopyResponse, DeleteRequest, DeleteResponse, EnumerateRequest, EnumerateResponse, FetchWriteTypeInfoRequest,
    FetchWriteTypeInfoResponse, GetNativeMetadataSupportRequest, GetNativeMetadataSupportResponse,
    GetOptimisticLockingSupportRequest, GetOptimisticLockingSupportResponse, IsReadOnlyRequest, IsReadOnlyResponse,
    MoveRequest, MoveResponse, ReadFromAddressRequest, ReadFromAddressResponse, ReadRequest, ReadResponse, StatRequest,
    StatResponse, UploadPartRequest, UploadPartResponse, WriteChunksAccepted, WriteParameters, WriteRequest,
    WriteResponse,
};
use tokio::sync::{mpsc, oneshot};
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::Channel;
use tonic::Status;

use super::super::{connect_with_retries, request_with_headers_config};
use crate::clients::traits::fileobject::FileObject;
use crate::headers::HeadersConfig;

pub struct FileObjectClient {
    client: FileObjectServiceClient<Channel>,
    headers_config: HeadersConfig,
}

impl FileObjectClient {
    pub async fn new(address: impl AsRef<str>, headers_config: HeadersConfig) -> Self {
        let client = connect_with_retries(|| FileObjectServiceClient::connect(address.as_ref().to_owned())).await;
        Self { client, headers_config }
    }
}

#[tonic::async_trait]
impl FileObject for FileObjectClient {
    async fn stat(&mut self, request: StatRequest) -> Result<StatResponse, Status> {
        self.client
            .stat(request_with_headers_config(request, &self.headers_config))
            .await
            .map(|response| response.into_inner())
    }

    async fn read_from_address(
        &mut self,
        request: ReadFromAddressRequest,
    ) -> Result<ReceiverStream<Result<ReadFromAddressResponse, Status>>, Status> {
        let (tx, rx) = mpsc::channel(1);
        let mut stream = self
            .client
            .read_from_address(request_with_headers_config(request, &self.headers_config))
            .await?
            .into_inner();

        tokio::spawn(async move {
            loop {
                let response = stream.message().await;
                match response {
                    Ok(Some(response)) => {
                        let _ = tx.send(Ok(response)).await;
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
        let (tx, rx) = mpsc::channel(1);
        let mut stream = self
            .client
            .read(request_with_headers_config(request, &self.headers_config))
            .await?
            .into_inner();

        tokio::spawn(async move {
            loop {
                let response = stream.message().await;
                match response {
                    Ok(Some(response)) => {
                        let _ = tx.send(Ok(response)).await;
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
        let (tx, rx) = mpsc::channel(1);
        let mut stream = self
            .client
            .enumerate(request_with_headers_config(request, &self.headers_config))
            .await?
            .into_inner();

        tokio::spawn(async move {
            loop {
                let response = stream.message().await;
                match response {
                    Ok(Some(response)) => {
                        let _ = tx.send(Ok(response)).await;
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

        // Channel to wait for server reply
        let (data_tx, data_rx) = oneshot::channel();
        let write_tx_stream = stream! {
            // Send the url
            yield WriteRequest {
                write_request_type: Some(WriteRequestType::Params(request)),
            };

            if let Some(data) = data {
                // Wait for the server to tell us to send data
                data_rx.await.unwrap();

                // Send the data
                for chunk in data.chunks(1024) {
                    yield WriteRequest { write_request_type: Some(WriteRequestType::Chunk(Chunk{ chunk: Bytes::copy_from_slice(chunk) })) };
                }
            }
        };

        let mut write_rx_stream = self
            .client
            .write(request_with_headers_config(write_tx_stream, &self.headers_config))
            .await?
            .into_inner();

        let mut data_tx = Some(data_tx); // Wrap in Option
        loop {
            let response = write_rx_stream.message().await;
            match response {
                Ok(Some(response)) => {
                    tracing::debug!(?response);
                    if has_body {
                        assert_eq!(
                            response.write_response_type,
                            Some(WriteResponseType::WriteChunksAccepted(WriteChunksAccepted {}))
                        );
                        let tx = data_tx.take().unwrap();
                        let _ = tx.send(());
                        has_body = false;
                    } else {
                        return Ok(response);
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
        self.client
            .delete(request_with_headers_config(request, &self.headers_config))
            .await
            .map(|response| response.into_inner())
    }

    async fn abort_multipart_upload(
        &mut self,
        request: AbortMultipartUploadRequest,
    ) -> Result<AbortMultipartUploadResponse, Status> {
        self.client
            .abort_multipart_upload(request_with_headers_config(request, &self.headers_config))
            .await
            .map(|response| response.into_inner())
    }

    async fn complete_multipart_upload(
        &mut self,
        request: CompleteMultipartUploadRequest,
    ) -> Result<CompleteMultipartUploadResponse, Status> {
        self.client
            .complete_multipart_upload(request_with_headers_config(request, &self.headers_config))
            .await
            .map(|response| response.into_inner())
    }

    async fn upload_part(&mut self, request: UploadPartRequest) -> Result<UploadPartResponse, Status> {
        self.client
            .upload_part(request_with_headers_config(request, &self.headers_config))
            .await
            .map(|response| response.into_inner())
    }

    async fn complete_redirect_upload(
        &mut self,
        request: CompleteRedirectUploadRequest,
    ) -> Result<CompleteRedirectUploadResponse, Status> {
        self.client
            .complete_redirect_upload(request_with_headers_config(request, &self.headers_config))
            .await
            .map(|response| response.into_inner())
    }

    async fn fetch_write_type_info(
        &mut self,
        request: FetchWriteTypeInfoRequest,
    ) -> Result<FetchWriteTypeInfoResponse, Status> {
        self.client
            .fetch_write_type_info(request_with_headers_config(request, &self.headers_config))
            .await
            .map(|response| response.into_inner())
    }

    async fn get_optimistic_locking_support(
        &mut self,
        request: GetOptimisticLockingSupportRequest,
    ) -> Result<GetOptimisticLockingSupportResponse, Status> {
        self.client
            .get_optimistic_locking_support(request_with_headers_config(request, &self.headers_config))
            .await
            .map(|response| response.into_inner())
    }

    async fn get_native_metadata_support(
        &mut self,
        request: GetNativeMetadataSupportRequest,
    ) -> Result<GetNativeMetadataSupportResponse, Status> {
        self.client
            .get_native_metadata_support(request_with_headers_config(request, &self.headers_config))
            .await
            .map(|response| response.into_inner())
    }

    async fn is_read_only(&mut self, request: IsReadOnlyRequest) -> Result<IsReadOnlyResponse, Status> {
        self.client
            .is_read_only(request_with_headers_config(request, &self.headers_config))
            .await
            .map(|response| response.into_inner())
    }

    async fn copy(&mut self, request: CopyRequest) -> Result<CopyResponse, Status> {
        self.client
            .copy(request_with_headers_config(request, &self.headers_config))
            .await
            .map(|response| response.into_inner())
    }

    async fn move_object(&mut self, request: MoveRequest) -> Result<MoveResponse, Status> {
        self.client
            .r#move(request_with_headers_config(request, &self.headers_config))
            .await
            .map(|response| response.into_inner())
    }
}
