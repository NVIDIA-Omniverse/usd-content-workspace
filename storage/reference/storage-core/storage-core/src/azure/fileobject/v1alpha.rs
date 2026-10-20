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
        fileobject::v1alpha::{
            read_from_address_response, read_response, write_response, AbortMultipartUploadRequest,
            AbortMultipartUploadResponse, Chunk, CompleteMultipartUploadRequest, CompleteMultipartUploadResponse,
            CompleteRedirectUploadRequest, CompleteRedirectUploadResponse, CopyRequest, CopyResponse, DeleteRequest,
            DeleteResponse, EnumerateRequest, EnumerateResponse, EnumerateResponseWithContinueToken,
            FetchWriteTypeInfoRequest, FetchWriteTypeInfoResponse, FileObject as FileObjectTrait,
            GetNativeMetadataSupportRequest, GetNativeMetadataSupportResponse, GetOptimisticLockingSupportRequest,
            GetOptimisticLockingSupportResponse, IsReadOnlyRequest, IsReadOnlyResponse, MoveRequest, MoveResponse,
            ReadFromAddressRequest, ReadFromAddressResponse, ReadRequest, ReadResponse, ResourceIdentity, StatRequest,
            StatResponse, UploadPartRequest, UploadPartResponse, UploadPreference, WriteChunksAccepted,
            WriteParameters, WriteResponse, WriteTypeForSizeInterval,
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
            .remove::<crate::traits::fileobject::v1alpha::ResourceInfo>();
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

        // Identity no longer carries size/mtime, so we need
        // a stat before deciding body-vs-redirect. Stat cache layer will inject
        // `ResourceInfo` into extensions on the way in when configured;
        // otherwise we stat live.
        let cached_resource_info = request
            .extensions_mut()
            .remove::<crate::traits::fileobject::v1alpha::ResourceInfo>();
        let request = request.into_inner();
        let encoded_identity = match request.resource_identity {
            Some(resource_identity) => resource_identity.encoded_identity,
            None => return Err(Error::invalid_argument("No resource_identity provided").into()),
        };

        // See v1beta::read — stat only supplies size/mtime; keep the caller's
        // original identity so read_internal uses their etag + version-id for
        // if-match + version selection on the actual GetBlob.
        let identity = Identity::parse(&encoded_identity)?;
        let stat_result: ResourceInfo = match cached_resource_info {
            Some(ri) => ri.try_into().map_err(Error::from)?,
            None => {
                self.stat_internal(Self::stat_address_for_identity(&identity), additional_headers.clone())
                    .await?
            },
        };
        let resource_info = ResourceInfo {
            // Carry the native metadata the stat already surfaced (gated at the stat source);
            // an empty map when the backend has native metadata disabled.
            native_metadata: stat_result.native_metadata,
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
        let additional_headers = request.extensions_mut().remove::<AdditionalHeaders>();
        let request = request.into_inner();
        // Do not log the full request: WriteParameters.native_metadata carries client-supplied,
        // potentially sensitive keys/values. Log a redacted view instead.
        debug_limited!(
            destination = %request.destination_resource_address,
            data_object_size = request.data_object_size,
            upload_preference = ?request.upload_preference,
            native_metadata_keys = request.native_metadata.len(),
            "azure::write"
        );

        self.write_internal(
            request.destination_resource_address,
            request.previous_version.as_ref().map(|v| v.encoded_identity.as_str()),
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

    #[instrument(name = "azure::copy", skip_all)]
    async fn copy(&self, mut request: Request<CopyRequest>) -> Result<CopyResponse> {
        debug_limited!(?request);
        let additional_headers = request.extensions_mut().remove::<AdditionalHeaders>();
        let request = request.into_inner();
        let source_id = request
            .source_resource_identity
            .ok_or_else(|| Error::invalid_argument("No source resource identity provided"))?;

        let destination_info = self
            .copy_internal(
                Identity::parse(&source_id.encoded_identity)?,
                Address::parse_blob(Url::parse(request.destination_resource_address.as_str()).map_err(Error::from)?)?,
                request
                    .previous_version
                    .as_ref()
                    .map(|v| Identity::parse(v.encoded_identity.as_str()))
                    .transpose()?,
                request.fail_if_exists.unwrap_or(false),
                additional_headers,
                false,
            )
            .await?;

        Ok(CopyResponse {
            resource_identity: Some(ResourceIdentity {
                encoded_identity: destination_info.encoded_identity,
            }),
        })
    }

    #[instrument(name = "azure::move_object", skip_all)]
    async fn move_object(&self, mut request: Request<MoveRequest>) -> Result<MoveResponse> {
        debug_limited!(?request);
        let additional_headers = request.extensions_mut().remove::<AdditionalHeaders>();

        // Check if the resource info is cached
        let cached_resource_info = request
            .extensions_mut()
            .remove::<crate::traits::fileobject::v1alpha::ResourceInfo>();

        let request = request.into_inner();

        let source_id = if let Some(id) = &request.source_previous_version {
            Some(id.encoded_identity.as_str())
        } else if let Some(info) = &cached_resource_info {
            Some(
                info.resource_identity
                    .as_ref()
                    .ok_or_else(|| Error::invalid_argument("Cached resource info missing resource identity"))?
                    .encoded_identity
                    .as_str(),
            )
        } else {
            None
        };

        let resource_info = self
            .move_object_internal(
                request.source_resource_address.as_str(),
                request.destination_resource_address.as_str(),
                source_id,
                request
                    .destination_previous_version
                    .as_ref()
                    .map(|v| v.encoded_identity.as_str()),
                request.fail_if_exists.unwrap_or(false),
                additional_headers,
            )
            .await?;

        Ok(MoveResponse {
            resource_identity: Some(ResourceIdentity {
                encoded_identity: resource_info.encoded_identity,
            }),
        })
    }

    #[instrument(name = "azure::get_optimistic_locking_support", skip_all)]
    async fn get_optimistic_locking_support(
        &self,
        _request: Request<GetOptimisticLockingSupportRequest>,
    ) -> Result<GetOptimisticLockingSupportResponse> {
        Ok(GetOptimisticLockingSupportResponse {
            supports_write: true,
            supports_delete: true,
            supports_copy: true,
            supports_move: true,
            supports_write_fail_if_exists: true,
            supports_copy_fail_if_exists: true,
            supports_move_fail_if_exists: true,
        })
    }

    #[instrument(name = "azure::get_native_metadata_support", skip_all)]
    async fn get_native_metadata_support(
        &self,
        request: Request<GetNativeMetadataSupportRequest>,
    ) -> Result<GetNativeMetadataSupportResponse> {
        // Validate the address and resolve the configured endpoint before answering (per the proto
        // contract: malformed addresses / unconfigured endpoints are explicit errors). No remote call.
        let addr_url = Url::parse(&request.into_inner().resource_address).map_err(Error::from)?;
        let address = Address::parse_blob(addr_url)?;
        let _client = self.client(&address).await?;
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
        let url = Url::parse(&request.get_ref().resource_address).map_err(Error::from)?;
        let resource_address = Address::parse_prefix(url)?;
        Ok(IsReadOnlyResponse {
            is_read_only: self.read_only_for(&resource_address).await?,
        })
    }
}

#[cfg(test)]
mod strict_mode_tests {
    use super::*;
    use crate::azure::secrets::{ClientCredentials, Secrets};
    use crate::azure::service::Config;
    use std::collections::HashMap;

    /// An Azure endpoint that is deliberately absent from the (empty) secrets map.
    const UNCONFIGURED: &str = "https://unconfigured.blob.core.windows.net/container/blob";
    /// Host of the configured source account used by the copy test.
    const SOURCE: &str = "https://source.blob.core.windows.net";

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

    #[tokio::test]
    async fn abort_multipart_upload_rejects_unconfigured_endpoint() {
        let storage = strict_storage();
        let err = storage
            .abort_multipart_upload(Request::new(AbortMultipartUploadRequest {
                upload_id: "upload-1".to_string(),
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
    async fn copy_rejects_unconfigured_destination() {
        // The source account IS configured; only the destination is unconfigured.
        // This proves copy_internal rejects the *destination* endpoint — if the
        // check were removed, `self.client(source)` would still resolve, so a
        // source-configured storage is what makes the test able to catch a revert.
        let storage = Storage::new(
            Config {
                allow_unconfigured_endpoints: false,
                ..Config::default()
            },
            Some(Secrets(HashMap::from([(
                SOURCE.to_string(),
                ClientCredentials {
                    storage_key: Some("key".to_string()),
                    ..Default::default()
                },
            )]))),
        )
        .expect("storage construction should succeed");

        let err = storage
            .copy(Request::new(CopyRequest {
                source_resource_identity: Some(ResourceIdentity {
                    encoded_identity: format!("{SOURCE}/container/blob?etag=abc123"),
                }),
                destination_resource_address: UNCONFIGURED.to_string(),
                previous_version: None,
                fail_if_exists: None,
            }))
            .await
            .expect_err("unconfigured destination must be rejected");
        assert!(
            err.to_string().contains("unconfigured.blob.core.windows.net"),
            "error should name the destination host: {err}"
        );
    }
}

#[cfg(test)]
mod native_metadata_support_tests {
    use super::*;
    use crate::azure::service::Config;

    const ADDRESS: &str = "https://account.blob.core.windows.net/container/blob";

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
    use crate::azure::service::Config;

    const ADDRESS: &str = "https://account.blob.core.windows.net/container/blob";

    #[tokio::test]
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

        use crate::azure::secrets::{ClientCredentials, Secrets};

        // Backend-wide read_only is true, but the per-endpoint secrets entry
        // overrides it to false: the override must win, so the address is writable.
        // With Config::default() (read_only None) this test could not tell
        // `override.or(config)` from `config.or(override)`.
        let secrets = Secrets(HashMap::from([(
            "https://account.blob.core.windows.net".to_string(),
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
}
