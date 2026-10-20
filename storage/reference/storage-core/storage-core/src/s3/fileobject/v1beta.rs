// SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;

use bytesize::ByteSize;
use snafu::prelude::*;
use storage_core_util::api_types::{ReadPart, ResourceInfo};
use storage_core_util::{debug_limited, trace_limited};
use tracing::instrument;

use crate::traits::{
    error::Result,
    fileobject::v1beta::{
        read_from_address_response, read_response, write_response, AbortMultipartUploadRequest,
        AbortMultipartUploadResponse, AddressInfo, Chunk, CompleteMultipartUploadRequest,
        CompleteMultipartUploadResponse, CompleteRedirectUploadRequest, CompleteRedirectUploadResponse, DeleteRequest,
        DeleteResponse, EnumerateRequest, EnumerateResponse, EnumerateResponseWithContinueToken,
        FetchWriteTypeInfoRequest, FetchWriteTypeInfoResponse, FileObject, ReadFromAddressRequest,
        ReadFromAddressResponse, ReadRequest, ReadResponse, StatRequest, StatResponse, UploadPartRequest,
        UploadPartResponse, UploadPreference, WriteChunksAccepted, WriteParameters, WriteResponse,
        WriteTypeForSizeInterval,
    },
    BoxFuture, BoxStream, Bytes, Request, StreamingOptions,
};

use storage_core_util::{additional_headers::AdditionalHeaders, api_types::WritePart};

use super::super::{
    error,
    resource::{Address, Identity, WrappedAddressInfo},
    service::Storage,
};

#[async_trait]
impl FileObject for Storage {
    fn inner(&self) -> Option<&dyn FileObject> {
        None
    }

    #[instrument(name = "s3::stat", skip_all)]
    async fn stat(&self, mut request: Request<StatRequest>) -> Result<StatResponse> {
        let additional_headers = request.extensions_mut().remove::<AdditionalHeaders>();
        debug_limited!(?request, ?additional_headers);

        let request = request.into_inner();
        let mut resource_address = Address::parse(&request.resource_address)?;
        let client = self.client(&mut resource_address).await?;

        let response = StatResponse {
            resource_info: Some(
                self.stat_sdk(&client, resource_address, additional_headers)
                    .await?
                    .into(),
            ),
        };
        debug_limited!(?response);

        Ok(response)
    }

    #[instrument(name = "s3::read_from_address", skip_all)]
    async fn read_from_address(
        &self,
        mut request: Request<ReadFromAddressRequest>,
    ) -> Result<BoxStream<ReadFromAddressResponse>> {
        let additional_headers = request.extensions_mut().remove::<AdditionalHeaders>();
        let cached_resource_info = request
            .extensions_mut()
            .remove::<crate::traits::fileobject::v1beta::ResourceInfo>();
        debug_limited!(?request, ?additional_headers, ?cached_resource_info);

        let request = request.into_inner();
        let mut resource_address = Address::parse(&request.resource_address)?;
        let client = self.client(&mut resource_address).await?;

        let resource_info: ResourceInfo = match cached_resource_info {
            Some(resource_info) => resource_info.try_into().map_err(error::Error::from)?,
            None => {
                self.stat_sdk(&client, resource_address, additional_headers.clone())
                    .await?
            },
        };
        debug_limited!(?resource_info);

        self.read_sdk(
            &client,
            resource_info,
            None,
            request.download_preference,
            additional_headers,
            |part| match part {
                ReadPart::Redirect(r) => ReadFromAddressResponse {
                    reply_type: Some(read_from_address_response::ReplyType::Redirect(r.into())),
                },
                ReadPart::ResourceInfo(r) => ReadFromAddressResponse {
                    reply_type: Some(read_from_address_response::ReplyType::ResourceInfo(r.into())),
                },
                ReadPart::Chunk(c) => ReadFromAddressResponse {
                    reply_type: Some(read_from_address_response::ReplyType::Chunk(Chunk { chunk: c })),
                },
            },
        )
        .await
    }

    #[instrument(name = "s3::read", skip_all)]
    async fn read(&self, mut request: Request<ReadRequest>) -> Result<BoxStream<ReadResponse>> {
        let additional_headers = request.extensions_mut().remove::<AdditionalHeaders>();
        debug_limited!(?request, ?additional_headers);

        // Identity no longer carries size/mtime, so we need
        // a stat to populate them before read_sdk can make the body-vs-redirect
        // decision. Stat cache layer will inject `ResourceInfo` via extensions
        // in the cached case; otherwise we stat live (HeadObject).
        //
        // We deliberately keep the CALLER's `encoded_identity` on the
        // ResourceInfo that flows to read_sdk. The stat is only used for
        // size/mtime (and tells us the current object exists); overwriting the
        // identity with the stat's current-version identity would silently
        // downgrade a read-of-specific-version into a read-of-current-version
        // and swallow legitimate precondition errors (wrong etag, bogus
        // version-id, etc.).
        let cached_resource_info = request
            .extensions_mut()
            .remove::<crate::traits::fileobject::v1beta::ResourceInfo>();
        let request = request.into_inner();
        let caller_encoded_identity = request
            .resource_identity
            .ok_or_else(|| error::Error::invalid_argument("resource_identity is required"))?
            .encoded_identity;
        let mut resource_identity = Identity::parse(&caller_encoded_identity)?;
        let client = self.client(&mut resource_identity.resource_address).await?;

        let stat_result = match cached_resource_info {
            Some(ri) => ri.try_into().map_err(error::Error::from)?,
            None => {
                self.stat_sdk(
                    &client,
                    resource_identity.resource_address.clone(),
                    additional_headers.clone(),
                )
                .await?
            },
        };
        let resource_info = ResourceInfo {
            native_metadata: Default::default(),
            encoded_identity: resource_identity.to_string(),
            data_object_size: stat_result.data_object_size,
            last_modified_timestamp: stat_result.last_modified_timestamp,
        };

        self.read_sdk(
            &client,
            resource_info,
            Some(resource_identity),
            request.download_preference,
            additional_headers,
            move |part| match part {
                ReadPart::Redirect(r) => {
                    debug_limited!(?r);
                    ReadResponse {
                        reply_type: Some(read_response::ReplyType::Redirect(r.into())),
                    }
                },
                ReadPart::ResourceInfo(r) => {
                    debug_limited!(?r);
                    ReadResponse {
                        reply_type: Some(read_response::ReplyType::Metadata(r.into())),
                    }
                },
                ReadPart::Chunk(c) => {
                    debug_limited!(?c);
                    ReadResponse {
                        reply_type: Some(read_response::ReplyType::Chunk(Chunk { chunk: c })),
                    }
                },
            },
        )
        .await
    }

    #[instrument(name = "s3::enumerate", skip_all)]
    async fn enumerate(
        &self,
        mut request: Request<EnumerateRequest>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<EnumerateResponseWithContinueToken>> {
        let additional_headers = request.extensions_mut().remove::<AdditionalHeaders>();
        debug_limited!(?request, ?streaming, ?additional_headers);

        let request = request.into_inner();
        let mut resource_address = Address::parse(&request.resource_address)?;
        let client = self.client(&mut resource_address).await?;

        self.list_sdk(
            &client,
            None,
            resource_address,
            streaming,
            additional_headers,
            move |contents, _, continue_token, resource_address| {
                let contents: Vec<_> = contents.collect();

                // If the response is a single object, and it is the same as the resource address,
                // return a not found error. The filtering in list_sdk would remove this item from the iterator, so we just
                // need to check if the iterator is empty. This is the behavior specified in the Storage API conformance tests.
                if contents.is_empty() {
                    return Err(error::SdkError::not_found("Resource Address not enumerable"))
                        .context(error::Sdk {})?;
                }

                let bucket_address = resource_address.bucket_address()?;
                Ok(EnumerateResponseWithContinueToken {
                    response: EnumerateResponse {
                        items: contents
                            .into_iter()
                            .filter_map(|object| {
                                trace_limited!(?object);
                                if let Some(key) = object.key() {
                                    let wrapped_address_info: WrappedAddressInfo =
                                        (&bucket_address.get_object_address(key), object).into();
                                    let address_info: AddressInfo = wrapped_address_info.0.into();
                                    Some(address_info)
                                } else {
                                    None
                                }
                            })
                            .collect(),
                    },
                    continue_token,
                })
            },
        )
        .await
    }

    #[instrument(name = "s3::write", skip_all)]
    async fn write(
        &self,
        mut request: Request<WriteParameters>,
        data_future: BoxFuture<Bytes>,
    ) -> Result<BoxStream<WriteResponse>> {
        debug_limited!(?request);
        let additional_headers = request.extensions_mut().remove::<AdditionalHeaders>();
        let request = request.into_inner();
        debug_limited!(?request);

        self.write_sdk(
            Address::parse(&request.destination_resource_address)?,
            request.previous_version.map(|v| v.encoded_identity),
            request.fail_if_exists.unwrap_or(false),
            request.data_object_size,
            request.upload_preference,
            data_future,
            additional_headers,
            Default::default(),
            |p| match p {
                WritePart::WriteChunksAccepted => WriteResponse {
                    write_response_type: Some(write_response::WriteResponseType::WriteChunksAccepted(
                        WriteChunksAccepted {},
                    )),
                },
                WritePart::ResourceInfo(r) => WriteResponse {
                    write_response_type: Some(write_response::WriteResponseType::ResourceInfo(r.into())),
                },
                WritePart::Redirect(r) => WriteResponse {
                    write_response_type: Some(write_response::WriteResponseType::WriteRedirect(r.into())),
                },
                WritePart::MultipartUpload(m) => WriteResponse {
                    write_response_type: Some(write_response::WriteResponseType::MultipartUpload(m.into())),
                },
            },
        )
        .await
    }

    #[instrument(name = "s3::delete", skip_all)]
    async fn delete(&self, mut request: Request<DeleteRequest>) -> Result<DeleteResponse> {
        let additional_headers = request.extensions_mut().remove::<AdditionalHeaders>();
        debug_limited!(?request, ?additional_headers);

        let request = request.into_inner();

        let resource_address = Address::parse(&request.resource_address)?;
        trace_limited!(?resource_address);

        self.delete_sdk(
            resource_address,
            request.previous_version.map(|v| v.encoded_identity),
            additional_headers,
        )
        .await?;

        Ok(DeleteResponse {})
    }

    #[instrument(name = "s3::abort_multipart_upload", skip_all)]
    async fn abort_multipart_upload(
        &self,
        mut request: Request<AbortMultipartUploadRequest>,
    ) -> Result<AbortMultipartUploadResponse> {
        let additional_headers = request.extensions_mut().remove::<AdditionalHeaders>();
        debug_limited!(?request, ?additional_headers);

        let request = request.into_inner();

        let destination_resource_address = Address::parse(request.destination_resource_address)?;
        trace_limited!(?destination_resource_address);

        self.abort_multipart_upload_sdk(request.upload_id, destination_resource_address, additional_headers)
            .await?;

        let response = AbortMultipartUploadResponse::default();
        debug_limited!(?response);

        Ok(response)
    }

    #[instrument(name = "s3::complete_multipart_upload", skip_all)]
    async fn complete_multipart_upload(
        &self,
        mut request: Request<CompleteMultipartUploadRequest>,
    ) -> Result<CompleteMultipartUploadResponse> {
        let additional_headers = request.extensions_mut().remove::<AdditionalHeaders>();
        debug_limited!(?request, ?additional_headers);

        let request = request.into_inner();

        let destination_resource_address = Address::parse(request.destination_resource_address)?;
        trace_limited!(?destination_resource_address);

        let response = self
            .complete_multipart_upload_sdk(
                request.upload_id,
                destination_resource_address,
                request.parts.into_iter().map(|p| p.into()).collect(),
                additional_headers,
            )
            .await?;

        Ok(CompleteMultipartUploadResponse {
            resource_info: Some(response.into()),
        })
    }

    #[instrument(name = "s3::upload_part", skip_all)]
    async fn upload_part(&self, mut request: Request<UploadPartRequest>) -> Result<UploadPartResponse> {
        let additional_headers = request.extensions_mut().remove::<AdditionalHeaders>();
        debug_limited!(?request, ?additional_headers);

        let request = request.into_inner();

        let destination_resource_address = Address::parse(request.destination_resource_address)?;
        trace_limited!(?destination_resource_address);

        let parts = self
            .upload_part_sdk(
                request.upload_id,
                destination_resource_address,
                request.part_number,
                request.part_count,
                additional_headers,
            )
            .await?;

        let response = UploadPartResponse {
            part_write_redirects: parts.into_iter().map(|p| p.into()).collect(),
        };

        Ok(response)
    }

    #[instrument(name = "s3::complete_redirect_upload", skip_all)]
    async fn complete_redirect_upload(
        &self,
        request: Request<CompleteRedirectUploadRequest>,
    ) -> Result<CompleteRedirectUploadResponse> {
        let request = request.into_inner();

        let destination_resource_address = Address::parse(request.destination_resource_address)?;
        trace_limited!(?destination_resource_address);
        self.check_endpoint_configured(&destination_resource_address.url)
            .await?;

        let response = self
            .complete_redirect_upload_sdk(
                destination_resource_address,
                request.additional_headers.into_iter().map(|h| h.into()).collect(),
            )
            .await?;

        Ok(CompleteRedirectUploadResponse {
            resource_info: Some(response.into()),
        })
    }

    #[instrument(name = "s3::fetch_write_type_info", skip_all)]
    async fn fetch_write_type_info(
        &self,
        request: Request<FetchWriteTypeInfoRequest>,
    ) -> Result<FetchWriteTypeInfoResponse> {
        debug_limited!(?request);

        // The conformance tests require that versioned resource addresses are not supported for fetching write type info,
        // even though we don't actually look at the resource address for this call.
        let request = request.into_inner();
        let destination_resource_address = Address::parse(&request.destination_resource_address)?;
        if destination_resource_address.is_versioned() {
            return Err(error::Error::invalid_argument(format!(
                "Versioned resource addresses are not supported for fetching write type info: {}",
                destination_resource_address
            ))
            .into());
        }
        self.check_endpoint_configured(&destination_resource_address.url)
            .await?;

        let response = FetchWriteTypeInfoResponse {
            write_type_intervals: vec![
                WriteTypeForSizeInterval {
                    // only do really small objects over the gRPC channel
                    minimum_data_object_size: 0,
                    maximum_data_object_size: self.config.write_body_size.as_u64(),
                    preferred_upload_method: UploadPreference::Body.into(),
                },
                WriteTypeForSizeInterval {
                    // skip the overhead of multipart medium sized objects
                    minimum_data_object_size: self.config.write_body_size.as_u64(),
                    maximum_data_object_size: self.config.write_redirect_size.as_u64(),
                    preferred_upload_method: UploadPreference::Redirect.into(),
                },
                WriteTypeForSizeInterval {
                    // https://docs.aws.amazon.com/AmazonS3/latest/userguide/qfacts.html
                    minimum_data_object_size: self.config.write_redirect_size.as_u64(),
                    maximum_data_object_size: ByteSize::tib(5).as_u64(),
                    preferred_upload_method: UploadPreference::Multipart.into(),
                },
            ],
        };
        debug_limited!(?response);

        Ok(response)
    }
}
