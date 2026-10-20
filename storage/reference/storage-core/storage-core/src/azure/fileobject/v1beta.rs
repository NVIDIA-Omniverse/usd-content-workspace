// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use storage_core_util::additional_headers::AdditionalHeaders;
use storage_core_util::api_types::ResourceInfo;
use storage_core_util::debug_limited;
use tracing::instrument;
use url::Url;

use crate::{
    azure::{
        error::{Error, SdkError},
        fileobject::{ReadPart, WritePart},
        resource::{Address, ContainerAddress, Identity, WrappedAddressInfo},
        service::Storage,
    },
    traits::{
        error::Result,
        fileobject::v1beta::{
            read_from_address_response, read_response, write_response, AbortMultipartUploadRequest,
            AbortMultipartUploadResponse, Chunk, CompleteMultipartUploadRequest, CompleteMultipartUploadResponse,
            CompleteRedirectUploadRequest, CompleteRedirectUploadResponse, DeleteRequest, DeleteResponse,
            EnumerateRequest, EnumerateResponse, EnumerateResponseWithContinueToken, FetchWriteTypeInfoRequest,
            FetchWriteTypeInfoResponse, FileObject as FileObjectTrait, ReadFromAddressRequest, ReadFromAddressResponse,
            ReadRequest, ReadResponse, StatRequest, StatResponse, UploadPartRequest, UploadPartResponse,
            UploadPreference, WriteChunksAccepted, WriteParameters, WriteResponse, WriteTypeForSizeInterval,
        },
        BoxFuture, BoxStream, Bytes, Request, StreamingOptions,
    },
};

#[tonic::async_trait]
impl FileObjectTrait for Storage {
    fn inner(&self) -> Option<&dyn FileObjectTrait> {
        None
    }

    #[instrument(name = "azure::stat", skip_all)]
    async fn stat(&self, mut request: Request<StatRequest>) -> Result<StatResponse> {
        debug_limited!(?request);
        let additional_headers = request.extensions_mut().remove::<AdditionalHeaders>();
        let response = StatResponse {
            resource_info: Some(
                self.stat_internal(request.into_inner().resource_address, additional_headers)
                    .await?
                    .into(),
            ),
        };
        debug_limited!(?response);
        Ok(response)
    }

    #[instrument(name = "azure::read_from_address", skip_all)]
    async fn read_from_address(
        &self,
        mut request: Request<ReadFromAddressRequest>,
    ) -> Result<BoxStream<ReadFromAddressResponse>> {
        debug_limited!(?request);
        let additional_headers = request.extensions_mut().remove::<AdditionalHeaders>();

        // Check if the resource info is cached
        let cached_resource_info = request
            .extensions_mut()
            .remove::<crate::traits::fileobject::v1beta::ResourceInfo>();
        let request = request.into_inner();

        // If not, stat the resource
        let resource_info: ResourceInfo = match cached_resource_info {
            Some(resource_info) => {
                debug_limited!(?resource_info, "Using cached resource info");
                resource_info.try_into().map_err(Error::from)?
            },
            None => {
                self.stat_internal(request.resource_address, additional_headers.clone())
                    .await?
            },
        };

        debug_limited!(?resource_info);

        let download_preference = Self::parse_download_preference(request.download_preference)?;

        self.read_internal(
            resource_info,
            None,
            download_preference,
            additional_headers,
            move |p| match p {
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

    #[instrument(name = "azure::read", skip_all)]
    async fn read(&self, mut request: Request<ReadRequest>) -> Result<BoxStream<ReadResponse>> {
        debug_limited!(?request);
        let additional_headers = request.extensions_mut().remove::<AdditionalHeaders>();

        // Read-by-identity used to derive `data_object_size`
        // and `last_modified_timestamp` from the identity URL. Those query params
        // were removed, so we now need a stat to supply them before
        // deciding body-vs-redirect in `read_internal`.
        //
        // The stat_cache layer (when configured) injects a `ResourceInfo` into
        // request extensions on the way in, so in the common case we avoid a
        // live HeadObject. Fall back to stat_internal when extensions have
        // nothing cached.
        let cached_resource_info = request
            .extensions_mut()
            .remove::<crate::traits::fileobject::v1beta::ResourceInfo>();
        let request = request.into_inner();
        let encoded_identity = match request.resource_identity {
            Some(resource_identity) => resource_identity.encoded_identity,
            None => return Err(Error::invalid_argument("No resource_identity provided").into()),
        };

        // The stat is used ONLY to supply `data_object_size` and
        // `last_modified_timestamp` (for body-vs-redirect + the metadata message).
        // We deliberately keep the caller's original `encoded_identity` on the
        // ResourceInfo so that `read_internal` sees the caller-supplied etag +
        // version-id — which is what drives `if-match` + version selection on the
        // actual GetBlob. Overwriting it with the stat's (current-version)
        // identity would silently downgrade a read-of-specific-version into a
        // read-of-current-version and swallow legitimate precondition errors.
        let identity = Identity::parse(&encoded_identity)?;
        let stat_result: ResourceInfo = match cached_resource_info {
            Some(ri) => ri.try_into().map_err(Error::from)?,
            None => {
                self.stat_internal(Self::stat_address_for_identity(&identity), additional_headers.clone())
                    .await?
            },
        };
        let resource_info = ResourceInfo {
            native_metadata: Default::default(),
            encoded_identity: Identity::create_url(
                identity.resource_address.resource_address.clone(),
                identity.etag.clone(),
                identity.version_id.clone(),
            ),
            data_object_size: stat_result.data_object_size,
            last_modified_timestamp: stat_result.last_modified_timestamp,
        };

        let download_preference = Self::parse_download_preference(request.download_preference)?;

        self.read_internal(
            resource_info,
            Some(identity),
            download_preference,
            additional_headers,
            |p| match p {
                ReadPart::Redirect(r) => ReadResponse {
                    reply_type: Some(read_response::ReplyType::Redirect(r.into())),
                },
                ReadPart::ResourceInfo(r) => ReadResponse {
                    reply_type: Some(read_response::ReplyType::Metadata(r.into())),
                },
                ReadPart::Chunk(c) => ReadResponse {
                    reply_type: Some(read_response::ReplyType::Chunk(Chunk { chunk: c })),
                },
            },
        )
        .await
    }

    #[instrument(name = "azure::write", skip_all)]
    async fn write(
        &self,
        mut request: Request<WriteParameters>,
        data_future: BoxFuture<Bytes>,
    ) -> Result<BoxStream<WriteResponse>> {
        debug_limited!(?request);
        let additional_headers = request.extensions_mut().remove::<AdditionalHeaders>();
        let request = request.into_inner();
        debug_limited!(?request);

        self.write_internal(
            request.destination_resource_address,
            request.previous_version.as_ref().map(|v| v.encoded_identity.as_str()),
            request.fail_if_exists.unwrap_or(false),
            request.data_object_size,
            request.upload_preference,
            data_future,
            additional_headers,
            // v1beta has no native metadata field.
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

    #[instrument(name = "azure::delete", skip_all)]
    async fn delete(&self, mut request: Request<DeleteRequest>) -> Result<DeleteResponse> {
        debug_limited!(?request);
        let additional_headers = request.extensions_mut().remove::<AdditionalHeaders>();
        let request = request.into_inner();

        self.delete_internal(
            request.resource_address.as_str(),
            request.previous_version.as_ref().map(|v| v.encoded_identity.as_str()),
            additional_headers,
        )
        .await?;

        Ok(DeleteResponse {})
    }

    #[instrument(name = "azure::complete_multipart_upload", skip_all)]
    async fn complete_multipart_upload(
        &self,
        mut request: Request<CompleteMultipartUploadRequest>,
    ) -> Result<CompleteMultipartUploadResponse> {
        debug_limited!(?request);
        let additional_headers = request.extensions_mut().remove::<AdditionalHeaders>();
        let request = request.into_inner();

        let resource_info = self
            .complete_multipart_upload_internal(
                request.upload_id.as_str(),
                request.destination_resource_address.as_str(),
                request.parts.into_iter().map(|p| p.into()).collect(),
                additional_headers,
            )
            .await?;

        let response = CompleteMultipartUploadResponse {
            resource_info: Some(resource_info.into()),
        };
        Ok(response)
    }

    #[instrument(name = "azure::abort_multipart_upload", skip_all)]
    async fn abort_multipart_upload(
        &self,
        request: Request<AbortMultipartUploadRequest>,
    ) -> Result<AbortMultipartUploadResponse> {
        debug_limited!(?request);
        let request = request.into_inner();
        // The abort itself is a no-op (see below), but strict mode must still reject
        // requests for endpoints that are not configured so the endpoint allowlist is
        // enforced consistently across every address-bearing RPC.
        let url = Url::parse(&request.destination_resource_address).map_err(Error::from)?;
        self.check_endpoint_configured(&url).await?;
        // NOTE: There's no safe way to abort a multipart upload because we can't
        // safely clear the uncommitted block list because someone else could also
        // be performing a multipart upload and we don't want to remove their blocks.
        // So for now do nothing and let garbage collection take care of it
        let response = AbortMultipartUploadResponse::default();
        Ok(response)
    }

    #[instrument(name = "azure::upload_part", skip_all)]
    async fn upload_part(&self, mut request: Request<UploadPartRequest>) -> Result<UploadPartResponse> {
        debug_limited!(?request);
        let additional_headers = request.extensions_mut().remove::<AdditionalHeaders>();
        let request = request.into_inner();

        let parts = self
            .upload_part_internal(
                request.upload_id.as_str(),
                request.destination_resource_address.as_str(),
                request.part_number,
                request.part_count,
                additional_headers,
            )
            .await?;

        Ok(UploadPartResponse {
            part_write_redirects: parts.into_iter().map(|p| p.into()).collect(),
        })
    }

    #[instrument(name = "azure::complete_redirect_upload", skip_all)]
    async fn complete_redirect_upload(
        &self,
        request: Request<CompleteRedirectUploadRequest>,
    ) -> Result<CompleteRedirectUploadResponse> {
        debug_limited!(?request);
        let request = request.into_inner();

        let url = Url::parse(&request.destination_resource_address).map_err(Error::from)?;
        self.check_endpoint_configured(&url).await?;

        let resource_info = self.complete_redirect_upload_internal(
            request.destination_resource_address.as_str(),
            request.additional_headers.into_iter().map(|h| h.into()).collect(),
        )?;

        Ok(CompleteRedirectUploadResponse {
            resource_info: Some(resource_info.into()),
        })
    }

    #[instrument(name = "azure::fetch_write_type_info", skip_all)]
    async fn fetch_write_type_info(
        &self,
        request: Request<FetchWriteTypeInfoRequest>,
    ) -> Result<FetchWriteTypeInfoResponse> {
        debug_limited!(?request);
        let request = request.into_inner();

        // The conformance tests require that versioned resource addresses are not supported for fetching write type info,
        // even though we don't actually look at the resource address for this call.
        let address = Address::parse_blob(Url::parse(&request.destination_resource_address).map_err(Error::from)?)?;
        if address.is_versioned() {
            return Err(Error::invalid_argument(format!(
                "Versioned resource addresses are not supported for fetching write type info: {}",
                address
            ))
            .into());
        }
        self.check_endpoint_configured(&address.resource_address).await?;

        Ok(FetchWriteTypeInfoResponse {
            write_type_intervals: vec![
                WriteTypeForSizeInterval {
                    minimum_data_object_size: 0,
                    maximum_data_object_size: self.config.write_body_max_size.as_u64(),
                    preferred_upload_method: UploadPreference::Body.into(),
                },
                WriteTypeForSizeInterval {
                    minimum_data_object_size: self.config.write_body_max_size.as_u64(),
                    maximum_data_object_size: self.config.write_redirect_max_size.as_u64(),
                    preferred_upload_method: UploadPreference::Redirect.into(),
                },
                WriteTypeForSizeInterval {
                    minimum_data_object_size: self.config.write_redirect_max_size.as_u64(),
                    // 50,000 4MiB blocks is the limit for a multipart upload.
                    // That's approximately 190.7 TiB
                    maximum_data_object_size: 50000 * 4 * 1024 * 1024,
                    preferred_upload_method: UploadPreference::Multipart.into(),
                },
            ],
        })
    }

    #[instrument(name = "azure::enumerate", skip_all)]
    async fn enumerate(
        &self,
        mut request: Request<EnumerateRequest>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<EnumerateResponseWithContinueToken>> {
        let additional_headers = request.extensions_mut().remove::<AdditionalHeaders>();

        debug_limited!(?request);
        let request = request.into_inner();
        let url = Url::parse(&request.resource_address).map_err(Error::from)?;
        let container_address = ContainerAddress::try_from_url(&url)?;
        let resource_address = Address::parse_prefix(url)?;
        self.list_internal(
            None,
            resource_address,
            false,
            streaming,
            additional_headers,
            move |blobs, _, next_marker, resource_address| {
                // Collect the blobs so we can check length and iterate
                let blobs: Vec<_> = blobs.collect();

                // If the response is a single object, and it is the same as the resource address,
                // return a not found error. This is the behavior specified in the Storage API conformance tests.
                if blobs.len() == 1 && blobs[0].name == resource_address.prefix() {
                    return Err(Error::from(SdkError::not_found("Resource Address not enumerable")).into());
                }

                Ok(EnumerateResponseWithContinueToken {
                    response: EnumerateResponse {
                        items: blobs
                            .into_iter()
                            .map(|b| {
                                WrappedAddressInfo::from((container_address.get_blob_url(&b.name), b))
                                    .0
                                    .into()
                            })
                            .collect(),
                    },
                    continue_token: next_marker.map(|m| m.as_str().to_string()),
                })
            },
        )
        .await
    }
}
