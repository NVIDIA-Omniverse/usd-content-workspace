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
    fileobject::v1alpha::{
        read_from_address_response, read_response, write_response, AbortMultipartUploadRequest,
        AbortMultipartUploadResponse, AddressInfo, Chunk, CompleteMultipartUploadRequest,
        CompleteMultipartUploadResponse, CompleteRedirectUploadRequest, CompleteRedirectUploadResponse, CopyRequest,
        CopyResponse, DeleteRequest, DeleteResponse, EnumerateRequest, EnumerateResponse,
        EnumerateResponseWithContinueToken, FetchWriteTypeInfoRequest, FetchWriteTypeInfoResponse, FileObject,
        GetNativeMetadataSupportRequest, GetNativeMetadataSupportResponse, GetOptimisticLockingSupportRequest,
        GetOptimisticLockingSupportResponse, IsReadOnlyRequest, IsReadOnlyResponse, MoveRequest, MoveResponse,
        ReadFromAddressRequest, ReadFromAddressResponse, ReadRequest, ReadResponse, ResourceIdentity, StatRequest,
        StatResponse, UploadPartRequest, UploadPartResponse, UploadPreference, WriteChunksAccepted, WriteParameters,
        WriteResponse, WriteTypeForSizeInterval,
    },
    BoxFuture, BoxStream, Bytes, Request, StreamingOptions,
};

use storage_core_util::{additional_headers::AdditionalHeaders, api_types::WritePart};

use super::super::{
    error,
    resource::{Address, Identity, WrappedAddressInfo},
    service::{BackendKind, Storage},
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
            .remove::<crate::traits::fileobject::v1alpha::ResourceInfo>();
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

        // See v1beta::read — stat only supplies size/mtime; keep the caller's
        // original identity so read_sdk uses their etag + version-id for
        // if-match + version selection on the actual GetObject.
        let cached_resource_info = request
            .extensions_mut()
            .remove::<crate::traits::fileobject::v1alpha::ResourceInfo>();
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
            // Carry the native metadata the stat already surfaced (gated at the stat source);
            // an empty map when the backend has native metadata disabled.
            native_metadata: stat_result.native_metadata,
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
                            // Filter out any subfolder placeholder objects
                            .filter(|object| object.key().map(|k| !k.ends_with('/')).unwrap_or(false))
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
        let additional_headers = request.extensions_mut().remove::<AdditionalHeaders>();
        let request = request.into_inner();
        // Do not log the full request: WriteParameters.native_metadata carries client-supplied,
        // potentially sensitive keys/values. Log a redacted view instead.
        debug_limited!(
            destination = %request.destination_resource_address,
            data_object_size = request.data_object_size,
            upload_preference = ?request.upload_preference,
            native_metadata_keys = request.native_metadata.len(),
            "s3::write"
        );

        self.write_sdk(
            Address::parse(&request.destination_resource_address)?,
            request.previous_version.map(|v| v.encoded_identity),
            request.fail_if_exists.unwrap_or(false),
            request.data_object_size,
            request.upload_preference,
            data_future,
            additional_headers,
            request.native_metadata,
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

    #[instrument(name = "s3::copy", skip_all)]
    async fn copy(&self, mut request: Request<CopyRequest>) -> Result<CopyResponse> {
        let additional_headers = request.extensions_mut().remove::<AdditionalHeaders>();
        debug_limited!(?request, ?additional_headers);

        let request = request.into_inner();

        let source_resource_identity = Identity::try_from(request.source_resource_identity.as_ref())?;
        trace_limited!(?source_resource_identity);

        let destination_resource_address = Address::parse(request.destination_resource_address)?;
        trace_limited!(?destination_resource_address);

        let resource_identity = self
            .copy_sdk(
                source_resource_identity,
                destination_resource_address,
                request.previous_version.map(|v| v.encoded_identity),
                request.fail_if_exists.unwrap_or(false),
                additional_headers,
                false,
            )
            .await?;

        let response = CopyResponse {
            resource_identity: Some(ResourceIdentity {
                encoded_identity: resource_identity.to_string(),
            }),
        };
        debug_limited!(?response);

        Ok(response)
    }

    #[instrument(name = "s3::move_object", skip_all)]
    async fn move_object(&self, mut request: Request<MoveRequest>) -> Result<MoveResponse> {
        let additional_headers = request.extensions_mut().remove::<AdditionalHeaders>();
        debug_limited!(?request, ?additional_headers);

        // Check if the resource info is cached
        let cached_resource_info = request
            .extensions_mut()
            .remove::<crate::traits::fileobject::v1alpha::ResourceInfo>();

        let request = request.into_inner();

        let source_resource_address = Address::parse(request.source_resource_address)?;
        trace_limited!(?source_resource_address);

        let destination_resource_address = Address::parse(request.destination_resource_address)?;
        trace_limited!(?destination_resource_address);

        let source_id = if let Some(id) = &request.source_previous_version {
            Some(Identity::parse(&id.encoded_identity)?)
        } else {
            cached_resource_info.map(Identity::try_from).transpose()?
        };

        let resource_identity = self
            .move_object_sdk(
                source_resource_address,
                destination_resource_address,
                source_id,
                request.destination_previous_version.map(|v| v.encoded_identity),
                request.fail_if_exists.unwrap_or(false),
                additional_headers,
            )
            .await?;

        let response = MoveResponse {
            resource_identity: Some(ResourceIdentity {
                encoded_identity: resource_identity.to_string(),
            }),
        };
        debug_limited!(?response);

        Ok(response)
    }

    #[instrument(name = "s3::get_optimistic_locking_support", skip_all)]
    async fn get_optimistic_locking_support(
        &self,
        request: Request<GetOptimisticLockingSupportRequest>,
    ) -> Result<GetOptimisticLockingSupportResponse> {
        let mut address = Address::parse(&request.into_inner().resource_address)?;
        let client = self.client(&mut address).await?;
        Ok(optimistic_locking_support(client.backend_kind))
    }

    #[instrument(name = "s3::get_native_metadata_support", skip_all)]
    async fn get_native_metadata_support(
        &self,
        request: Request<GetNativeMetadataSupportRequest>,
    ) -> Result<GetNativeMetadataSupportResponse> {
        // Validate the address and resolve the configured endpoint before answering (per the proto
        // contract: malformed addresses / unconfigured endpoints are explicit errors). No remote call.
        let mut address = Address::parse(&request.into_inner().resource_address)?;
        let _client = self.client(&mut address).await?;
        // Report false in the pre-release state: native-metadata *writes* are not supported through this
        // API (every non-empty native_metadata write is rejected with UNIMPLEMENTED, coherent with this
        // false), and only read-side surfacing is enabled behind native_metadata_enabled. Flip to reflect
        // real write support once write persistence lands on all upload paths.
        Ok(GetNativeMetadataSupportResponse {
            supports_native_metadata: false,
        })
    }

    async fn is_read_only(&self, request: Request<IsReadOnlyRequest>) -> Result<IsReadOnlyResponse> {
        // Parse so a malformed address errors rather than silently reporting a state
        // for a URL the backend could never serve. Read-only is a declarative operator
        // setting (per-endpoint override, then backend-wide); there is no runtime detection.
        let resource_address = Address::parse(&request.get_ref().resource_address)?;
        Ok(IsReadOnlyResponse {
            is_read_only: self.read_only_for(&resource_address).await?,
        })
    }
}

/// Map a backend variant to the proto's optimistic-locking capability response.
///
/// Per the proto, `supports_write/delete/copy/move` each mean "conditional
/// execution with previous_version", and the four `*_fail_if_exists` fields
/// mean "create-only fail_if_exists support". Both axes vary independently by
/// backend variant. Capability matrix derived from existing integration test
/// exclusions:
///   - conditional writes:      S3, Azure, MinIO  (excluded: GCP)
///   - conditional copy/move:   S3, Azure          (excluded: MinIO, GCP)
///   - conditional delete:      S3, Azure          (excluded: MinIO, GCP)
///   - fail_if_exists write:    S3, Azure, MinIO
///   - fail_if_exists copy/move: S3, Azure
///
/// Azure is handled in the Azure backend; this maps S3-shaped backends only.
fn optimistic_locking_support(kind: BackendKind) -> GetOptimisticLockingSupportResponse {
    match kind {
        BackendKind::Aws => GetOptimisticLockingSupportResponse {
            supports_write: true,
            supports_delete: true,
            supports_copy: true,
            supports_move: true,
            supports_write_fail_if_exists: true,
            supports_copy_fail_if_exists: true,
            supports_move_fail_if_exists: true,
        },
        BackendKind::Other => GetOptimisticLockingSupportResponse {
            // Conservative reporting for non-AWS S3-compat (MinIO and similar).
            // MinIO supports conditional writes and write fail_if_exists but
            // not conditional copy/move/delete. Custom S3-compat backends with
            // richer support should be detected explicitly (e.g., by host
            // pattern or operator-supplied config) and added to the matcher.
            supports_write: true,
            supports_delete: false,
            supports_copy: false,
            supports_move: false,
            supports_write_fail_if_exists: true,
            supports_copy_fail_if_exists: false,
            supports_move_fail_if_exists: false,
        },
        BackendKind::GcpS3Compat => GetOptimisticLockingSupportResponse {
            supports_write: false,
            supports_delete: false,
            supports_copy: false,
            supports_move: false,
            supports_write_fail_if_exists: false,
            supports_copy_fail_if_exists: false,
            supports_move_fail_if_exists: false,
        },
    }
}

#[cfg(test)]
mod strict_mode_tests {
    use super::*;
    use crate::s3::service::Config;

    /// An S3 endpoint that is deliberately absent from the (empty) secrets map.
    const UNCONFIGURED: &str = "https://unconfigured.s3.us-east-1.amazonaws.com/key";

    fn strict_storage() -> Storage {
        Storage::new(
            Config {
                allow_unconfigured_endpoints: false,
                ..Config::default()
            },
            None,
        )
        .expect("storage construction should succeed")
    }

    #[tokio::test]
    async fn fetch_write_type_info_rejects_unconfigured_endpoint() {
        let storage = strict_storage();
        let err = storage
            .fetch_write_type_info(Request::new(FetchWriteTypeInfoRequest {
                destination_resource_address: UNCONFIGURED.to_string(),
            }))
            .await
            .expect_err("unconfigured endpoint must be rejected");
        assert!(
            err.to_string().contains("endpoint not configured"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn is_read_only_rejects_unconfigured_endpoint() {
        let storage = strict_storage();
        let err = storage
            .is_read_only(Request::new(IsReadOnlyRequest {
                resource_address: UNCONFIGURED.to_string(),
            }))
            .await
            .expect_err("unconfigured endpoint must be rejected");
        assert!(
            err.to_string().contains("endpoint not configured"),
            "unexpected error: {err}"
        );
    }

    /// Strict mode with a backend-wide `read_only` is a configured answer for every
    /// endpoint, so it must still be reported rather than rejected.
    #[tokio::test]
    async fn is_read_only_answers_in_strict_mode_when_backend_wide_setting_exists() {
        let storage = Storage::new(
            Config {
                allow_unconfigured_endpoints: false,
                read_only: Some(true),
                ..Config::default()
            },
            None,
        )
        .expect("storage construction should succeed");
        let response = storage
            .is_read_only(Request::new(IsReadOnlyRequest {
                resource_address: UNCONFIGURED.to_string(),
            }))
            .await
            .expect("a backend-wide setting answers for any endpoint");
        assert!(response.is_read_only);
    }

    #[tokio::test]
    async fn complete_redirect_upload_rejects_unconfigured_endpoint() {
        let storage = strict_storage();
        let err = storage
            .complete_redirect_upload(Request::new(CompleteRedirectUploadRequest {
                destination_resource_address: UNCONFIGURED.to_string(),
                additional_headers: vec![],
            }))
            .await
            .expect_err("unconfigured endpoint must be rejected");
        assert!(
            err.to_string().contains("endpoint not configured"),
            "unexpected error: {err}"
        );
    }
}

#[cfg(test)]
mod native_metadata_support_tests {
    use super::*;
    use crate::s3::service::Config;

    const ADDRESS: &str = "https://bucket.s3.us-east-1.amazonaws.com/key";

    fn storage(native_metadata_enabled: Option<bool>) -> Storage {
        Storage::new(
            Config {
                native_metadata_enabled,
                ..Config::default()
            },
            None,
        )
        .expect("storage construction should succeed")
    }

    async fn reports(storage: &Storage) -> bool {
        storage
            .get_native_metadata_support(Request::new(GetNativeMetadataSupportRequest {
                resource_address: ADDRESS.to_string(),
            }))
            .await
            .expect("capability query should succeed")
            .supports_native_metadata
    }

    // GetNativeMetadataSupport reports false in the pre-release state: native-metadata writes are
    // unsupported on every upload path, regardless of native_metadata_enabled.
    #[tokio::test]
    async fn reports_false_when_disabled() {
        assert!(!reports(&storage(None)).await);
        assert!(!reports(&storage(Some(false))).await);
    }

    #[tokio::test]
    async fn reports_false_even_when_enabled_pre_release() {
        assert!(!reports(&storage(Some(true))).await);
    }

    #[tokio::test]
    async fn multipart_write_with_native_metadata_is_unimplemented() {
        assert_eq!(
            write_code(UploadPreference::Multipart).await,
            tonic::Code::Unimplemented
        );
    }

    #[tokio::test]
    async fn redirect_write_with_native_metadata_is_unimplemented() {
        assert_eq!(write_code(UploadPreference::Redirect).await, tonic::Code::Unimplemented);
    }

    // Coherent-false: a direct-body write carrying native metadata is rejected too (not just
    // redirect/multipart), so the write surface matches the false capability.
    #[tokio::test]
    async fn direct_body_write_with_native_metadata_is_unimplemented() {
        assert_eq!(write_code(UploadPreference::Body).await, tonic::Code::Unimplemented);
    }

    #[tokio::test]
    async fn invalid_native_metadata_key_is_invalid_argument() {
        let storage = storage(Some(true));
        let data_future = Box::pin(async { Ok(bytes::Bytes::new()) });
        let err = storage
            .write(
                Request::new(WriteParameters {
                    destination_resource_address: ADDRESS.to_string(),
                    native_metadata: storage_core_util::api_types::native_metadata_from_strings([(
                        "bad key".to_string(),
                        "v".to_string(),
                    )]),
                    data_object_size: 3,
                    upload_preference: Some(UploadPreference::Body as i32),
                    ..Default::default()
                }),
                data_future,
            )
            .await
            .err()
            .expect("a native metadata key the backend cannot represent must be rejected");
        assert_eq!(err.tonic_status().code(), tonic::Code::InvalidArgument);
    }

    async fn write_code(preference: UploadPreference) -> tonic::Code {
        let storage = storage(Some(true));
        let data_future = Box::pin(async { Ok(bytes::Bytes::new()) });
        let err = storage
            .write(
                Request::new(WriteParameters {
                    destination_resource_address: ADDRESS.to_string(),
                    native_metadata: storage_core_util::api_types::native_metadata_from_strings([(
                        "k".to_string(),
                        "v".to_string(),
                    )]),
                    data_object_size: 3,
                    upload_preference: Some(preference as i32),
                    ..Default::default()
                }),
                data_future,
            )
            .await
            .err()
            .expect("write carrying native metadata on a non-body path must fail");
        err.tonic_status().code()
    }
}

#[cfg(test)]
mod read_only_tests {
    use super::*;
    use crate::s3::service::Config;
    use serial_test::serial;

    const ADDRESS: &str = "https://bucket.s3.us-east-1.amazonaws.com/key";

    // Serial with the `READ_ONLY` env test: with no secrets/config, the resolver
    // falls back to the env var, so a concurrent env mutation could flip this.
    #[tokio::test]
    #[serial]
    async fn defaults_to_writable() {
        let storage = Storage::new(Config::default(), None).expect("storage construction should succeed");
        let response = storage
            .is_read_only(Request::new(IsReadOnlyRequest {
                resource_address: ADDRESS.to_string(),
            }))
            .await
            .expect("is_read_only should succeed");
        assert!(!response.is_read_only);
    }

    #[tokio::test]
    #[serial]
    async fn env_read_only_drives_answer_when_unconfigured() {
        // In the unconfigured-endpoint mode (no secrets, allow_unconfigured), the
        // READ_ONLY env var that `from_env` parses must actually drive is_read_only.
        unsafe {
            std::env::set_var("READ_ONLY", "true");
        }
        let storage = Storage::new(Config::default(), None).expect("storage construction should succeed");
        let result = storage
            .is_read_only(Request::new(IsReadOnlyRequest {
                resource_address: ADDRESS.to_string(),
            }))
            .await;
        unsafe {
            std::env::remove_var("READ_ONLY");
        }
        assert!(
            result.expect("is_read_only should succeed").is_read_only,
            "READ_ONLY=true must make the endpoint read-only when nothing else is configured"
        );
    }

    #[tokio::test]
    async fn reports_configured_read_only() {
        let storage = Storage::new(
            Config {
                read_only: Some(true),
                ..Config::default()
            },
            None,
        )
        .expect("storage construction should succeed");
        let response = storage
            .is_read_only(Request::new(IsReadOnlyRequest {
                resource_address: ADDRESS.to_string(),
            }))
            .await
            .expect("is_read_only should succeed");
        assert!(response.is_read_only);
    }

    #[tokio::test]
    async fn rejects_malformed_address() {
        let storage = Storage::new(Config::default(), None).expect("storage construction should succeed");
        storage
            .is_read_only(Request::new(IsReadOnlyRequest {
                resource_address: "not a url".to_string(),
            }))
            .await
            .expect_err("a malformed address must error rather than report a state");
    }

    #[tokio::test]
    async fn endpoint_override_wins_over_backend_default() {
        use std::collections::HashMap;

        use crate::s3::secrets::{ClientCredentials, Secrets};

        // Backend-wide read_only is true, but the per-endpoint secrets entry
        // overrides it to false: the override must win, so the address is writable.
        // With Config::default() (read_only None) this test could not tell
        // `override.or(config)` from `config.or(override)`.
        let secrets = Secrets(HashMap::from([(
            "https://bucket.s3.us-east-1.amazonaws.com".to_string(),
            ClientCredentials {
                read_only: Some(false),
                ..Default::default()
            },
        )]));
        let storage = Storage::new(
            Config {
                read_only: Some(true),
                ..Config::default()
            },
            Some(secrets),
        )
        .expect("storage construction should succeed");

        let response = storage
            .is_read_only(Request::new(IsReadOnlyRequest {
                resource_address: ADDRESS.to_string(),
            }))
            .await
            .expect("is_read_only should succeed");

        assert!(!response.is_read_only);
    }

    // Serial with the other `READ_ONLY` env tests.
    #[tokio::test]
    #[serial]
    async fn env_read_only_ignored_for_secrets_configured_endpoint() {
        use std::collections::HashMap;

        use crate::s3::secrets::{ClientCredentials, Secrets};

        // The endpoint has a secrets entry but no `read_only`, and there is no backend-wide
        // `s3.read_only`. Because `create_client` only drives an endpoint from env vars when it
        // is absent from the secrets map, the process-wide `READ_ONLY=true` must not leak here:
        // the address stays writable.
        let secrets = Secrets(HashMap::from([(
            "https://bucket.s3.us-east-1.amazonaws.com".to_string(),
            ClientCredentials::default(),
        )]));
        unsafe {
            std::env::set_var("READ_ONLY", "true");
        }
        let storage = Storage::new(Config::default(), Some(secrets)).expect("storage construction should succeed");
        let result = storage
            .is_read_only(Request::new(IsReadOnlyRequest {
                resource_address: ADDRESS.to_string(),
            }))
            .await;
        unsafe {
            std::env::remove_var("READ_ONLY");
        }
        assert!(
            !result.expect("is_read_only should succeed").is_read_only,
            "a secrets-configured endpoint without read_only must stay writable despite READ_ONLY=true in the env",
        );
    }
}
