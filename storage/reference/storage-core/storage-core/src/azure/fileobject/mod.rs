// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

pub mod v1alpha;
pub mod v1beta;

use std::fmt::{Debug, Display};
use std::time::Duration;

use azure_core::{
    date::parse_rfc1123,
    headers::{BLOB_TYPE, ETAG, IF_MATCH, IF_NONE_MATCH, LAST_MODIFIED, VERSION},
    request_options::{IfMatchCondition, IfSourceMatchCondition},
    AppendToUrlQuery,
};
use azure_storage::shared_access_signature::service_sas::BlobSasPermissions;
use azure_storage::CopyId;
use azure_storage_blobs::blob::{Blob, CopyStatus};
use azure_storage_blobs::prelude::{BlobBlockType, BlockId, BlockList, VersionId};
use bytes::Buf;
use futures::{StreamExt, TryStreamExt};
use storage_core_protos::google;

/// Convert FileObject-native metadata into an Azure blob `Metadata` map. Azure
/// metadata values are strings, so non-string protobuf values are rejected with
/// INVALID_ARGUMENT (the proto contract for values a backend cannot represent).
fn azure_metadata_from_native(
    native: &std::collections::HashMap<String, google::protobuf::Value>,
) -> Result<azure_core::request_options::Metadata> {
    let mut md = azure_core::request_options::Metadata::new();
    for (key, value) in native {
        // Native metadata keys are stored raw (not through the MetadataService key encoding), so
        // reject keys Azure cannot represent (it requires C-identifier names) up front rather than
        // surfacing an opaque SDK error later.
        if !crate::azure::metadata::is_valid_azure_metadata_key(key) {
            return Err(Error::invalid_argument(format!(
                "native metadata key '{key}' is not a valid Azure metadata key"
            ))
            .into());
        }
        match &value.kind {
            Some(google::protobuf::value::Kind::StringValue(s)) => {
                md.insert(key.clone(), s.clone());
            },
            _ => {
                return Err(Error::invalid_argument(format!(
                    "native metadata value for key '{key}' must be a string for this backend"
                ))
                .into());
            },
        }
    }
    Ok(md)
}
use storage_core_util::additional_headers::{AdditionalHeaders, RequestType};
use storage_core_util::api_types::{
    CompletedUploadPart, DownloadPreference, Header, MultipartUpload, ReadPart, Redirect, ResourceInfo, UploadMethod,
    UploadPreference, WritePart, WriteRedirect,
};
use storage_core_util::upload_id;
use storage_core_util::{debug_limited, trace_limited};
use time::OffsetDateTime;
use tracing::{info_span, instrument, Instrument};
use url::{Position, Url};
use uuid::Uuid;

use azure_storage_blobs::prelude::BlobClient;

use crate::{
    azure::{
        error::{Error, SdkError},
        resource::{Address, Identity, WrappedResourceInfo},
        service::{AzureClient, Storage, X_MS_VERISION},
    },
    traits::{error::Result, BoxFuture, Bytes},
};

/// Outcome of a post-error source-etag recheck. See [`Storage::source_etag_recheck`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EtagRecheck {
    SourceMatches,
    SourceStale,
    Ambiguous,
}

/// Non-terminal outcome of a single copy-status observation. Terminal failures
/// are surfaced as errors by [`Storage::classify_copy_status`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CopyStatusPoll {
    Completed,
    StillPending,
}

const AZURE_VERSION_ID_HEADER: &str = "x-ms-version-id";

impl Storage {
    /// Resolves the operator-specified read-only state for the endpoint serving
    /// `resource_address`: the per-endpoint `READ_ONLY` secrets entry wins, then
    /// the backend-wide `azure.read_only` config, then the default (writable).
    /// No backend call; only the in-memory secrets map is read.
    ///
    /// Errors with `not_found` when strict mode (`allow_unconfigured_endpoints =
    /// false`) is on and nothing configures the endpoint, since the backend would
    /// refuse to serve that address anyway.
    pub(super) async fn read_only_for(&self, resource_address: &Address) -> Result<bool> {
        let lookup_key = &resource_address.resource_address[..Position::BeforePath];
        let (matched_secret, endpoint_override) = {
            let secrets_guard = self.secrets.read().await;
            match secrets_guard.as_ref().and_then(|secrets| secrets.get(lookup_key)) {
                Some(creds) => (true, creds.read_only),
                None => (false, None),
            }
        };

        // Same reasoning as the S3 resolver: in strict mode an endpoint absent from
        // the secrets map is one this backend refuses to serve, so answering a
        // read-only state for it would describe an address every real operation
        // rejects with `endpoint not configured`.
        if !matched_secret && !self.config.allow_unconfigured_endpoints && self.config.read_only.is_none() {
            return Err(Error::not_found(format!("endpoint not configured: {lookup_key}")).into());
        }

        Ok(endpoint_override.or(self.config.read_only).unwrap_or(false))
    }

    fn stat_address_for_identity(identity: &Identity) -> String {
        match identity.version_id.as_deref() {
            Some(version_id) => identity
                .resource_address()
                .create_versioned_resource_address(version_id),
            None => identity.resource_address().to_string(),
        }
    }

    #[instrument(name = "azure::stat_internal", skip_all)]
    pub async fn stat_internal(
        &self,
        resource_address: impl AsRef<str> + Display + Debug,
        additional_headers: Option<AdditionalHeaders>,
    ) -> Result<ResourceInfo> {
        let url = Url::parse(resource_address.as_ref()).map_err(Error::from)?;
        let address = Address::parse_blob(url)?;

        // Can't stat a folder placeholder object
        if address.is_folder_placeholder() {
            return Err(Error::invalid_argument(format!(
                "Cannot stat a folder placeholder object {}",
                resource_address.as_ref()
            ))
            .into());
        }

        let mut builder = self
            .client(&address)
            .await?
            .container_client(address.container()?)
            .blob_client(address.blob())
            .get_properties();

        if let Some(version_id) = address.version_id() {
            builder = builder.blob_versioning(VersionId::new(version_id.to_string()));
        }

        if let Some(additional_headers) = additional_headers {
            builder = builder.context(Self::create_custom_context(additional_headers, RequestType::Api)?);
        }

        let properties = builder
            .into_future()
            .instrument(info_span!("Get Properties"))
            .await
            .inspect_err(|e| self.rate_limit_counters.get_properties.check_error(e))
            .map_err(Error::from)?;
        trace_limited!(?properties);

        let version_id = properties.blob.version_id.clone();
        let mut resource_info = WrappedResourceInfo::from((address.into(), properties.blob, version_id)).0;
        // Only surface backend blob metadata when the operator has opted in; otherwise pre-existing
        // metadata that was never exposed through this API stays hidden (native_metadata_enabled gate).
        if !self.config.native_metadata_enabled.unwrap_or(false) {
            resource_info.native_metadata.clear();
        }
        Ok(resource_info)
    }

    /// Post-error source-etag recheck used to disambiguate a copy 412 between
    /// source staleness and destination conflict. Returns:
    ///   - `SourceMatches` — source still has the caller's expected etag (or
    ///     no etag guard was armed); the 412 must be the destination conflict.
    ///   - `SourceStale` — source returned 412 against `IfMatch::Match`; the
    ///     original copy 412 was the source staleness check failing.
    ///   - `Ambiguous` — the recheck failed for some other reason (client
    ///     setup, transient, permissions, etc.). The caller should surface the
    ///     original 412 rather than guess.
    async fn source_etag_recheck(
        &self,
        source_resource_address: &Address,
        expected_etag: Option<&str>,
        expected_version_id: Option<&str>,
        additional_headers: Option<AdditionalHeaders>,
    ) -> EtagRecheck {
        let Some(expected_etag) = expected_etag else {
            // No source-staleness guard armed — the 412 can only have come from
            // `if_match(NotMatch("*"))` on the destination.
            return EtagRecheck::SourceMatches;
        };

        let client = match self.client(source_resource_address).await {
            Ok(c) => c,
            Err(_) => return EtagRecheck::Ambiguous,
        };
        let container = match source_resource_address.container() {
            Ok(c) => c,
            Err(_) => return EtagRecheck::Ambiguous,
        };

        let mut builder = client
            .container_client(container)
            .blob_client(source_resource_address.blob())
            .get_properties()
            .if_match(IfMatchCondition::Match(expected_etag.to_string()));
        // When the copy targeted a specific source version (via `versionId` on
        // copy_source), the recheck must target the same version. Otherwise the
        // get_properties resolves to the latest version and reports SourceStale
        // even when the requested version still matches — misclassifying a
        // destination conflict as source staleness.
        if let Some(version_id) = expected_version_id {
            builder = builder.blob_versioning(VersionId::new(version_id.to_string()));
        }

        if let Some(additional_headers) = additional_headers {
            match Self::create_custom_context(additional_headers, RequestType::Api) {
                Ok(ctx) => builder = builder.context(ctx),
                Err(_) => return EtagRecheck::Ambiguous,
            }
        }

        match builder
            .into_future()
            .instrument(info_span!("Get Properties (etag disambiguation)"))
            .await
            .map_err(Error::from)
        {
            Ok(_) => EtagRecheck::SourceMatches,
            Err(err) if err.is_precondition_failed() => EtagRecheck::SourceStale,
            Err(_) => EtagRecheck::Ambiguous,
        }
    }

    #[instrument(name = "azure::read_internal", skip_all)]
    pub async fn read_internal<F, T>(
        &self,
        resource_info: ResourceInfo,
        // Pre-parsed identity from the caller, when available. The read-by-identity path
        // already parses to construct the stat URL; passing it through avoids a second
        // parse here. Callers without a parsed identity (e.g. read_from_address) pass
        // `None` and we parse from `resource_info.encoded_identity`.
        parsed_identity: Option<Identity>,
        download_preference: DownloadPreference,
        additional_headers: Option<AdditionalHeaders>,
        response_function: F,
    ) -> Result<crate::traits::BoxStream<T>>
    where
        F: Fn(ReadPart) -> T + std::marker::Send + 'static,
        T: std::marker::Send + 'static,
    {
        // The Identity URL no longer carries content-length
        // or last-modified. We receive a ResourceInfo from the caller (which obtained
        // it from a stat, cached or live) so we still have size/mtime for the body-vs-
        // redirect decision and for the metadata message.
        let resource_id = match parsed_identity {
            Some(identity) => identity,
            None => Identity::parse(&resource_info.encoded_identity)?,
        };

        // Get the client first to check for front door configuration
        let client = self.client(resource_id.resource_address()).await?;

        let download_preference = match download_preference {
            DownloadPreference::Body => DownloadPreference::Body,
            DownloadPreference::Redirect => DownloadPreference::Redirect,
            _ => resource_info
                .data_object_size
                .map_or(DownloadPreference::Redirect, |size| {
                    if size > self.config.read_redirect_size.as_u64() {
                        DownloadPreference::Redirect
                    } else {
                        DownloadPreference::Body
                    }
                }),
        };

        // Bake a resource info message that will be sent once all preparations are done.
        let resource_info_message = response_function(ReadPart::ResourceInfo(resource_info.clone()));

        match download_preference {
            DownloadPreference::Body => {
                // Read the blob
                let blob_client = client
                    .container_client(resource_id.container()?)
                    .blob_client(resource_id.blob());
                let mut builder = blob_client.get().chunk_size(u64::MAX);
                if let Some(additional_headers) = additional_headers {
                    builder = builder.context(Self::create_custom_context(additional_headers, RequestType::Data)?);
                }
                // Add the etag if-match guard. Identity no
                // longer carries last-modified, so there's no if-unmodified-since
                // fallback — every identity carries an etag (it's the one required
                // field on `Identity::parse`), so requiring it here is safe.
                let etag = resource_id.etag.ok_or_else(|| {
                    Error::invalid_argument(format!(
                        "Resource identifier did not contain etag: {}",
                        resource_info.encoded_identity
                    ))
                })?;
                builder = builder.if_match(IfMatchCondition::Match(etag));

                // Add version id if it's present
                if let Some(v) = resource_id.version_id {
                    builder = builder.blob_versioning(VersionId::new(v));
                }

                let mut stream = builder.into_stream();
                let response = stream
                    .try_next()
                    .await
                    .inspect_err(|e| self.rate_limit_counters.get_blob.check_error(e))
                    .map_err(Error::from)?
                    .ok_or_else(|| Error::internal("Response stream is empty"))?;

                let data = response.data;
                Ok(Box::pin(async_stream::stream! {
                    let mut data = data;
                    yield Ok(resource_info_message);
                    while let Some(value) = data.next().await {
                        let chunk = match value.map_err(Error::from) {
                            Ok(c) => c.chunk().to_vec(),
                            Err(e) => {
                                yield Err(e.into());
                                break;
                            },
                        };

                        yield Ok(response_function(ReadPart::Chunk(chunk.into())));
                    }
                }))
            },
            _ => {
                // Use the Front Door client if configured, otherwise use the direct client
                let base_client = match &client.front_door_client {
                    Some(front_door_client) => front_door_client,
                    None => &client.client,
                };
                let blob_client = base_client
                    .container_client(resource_id.container()?)
                    .blob_client(resource_id.blob());

                // Generate a presigned url for the blob
                let mut signed_url = if client.anonymous {
                    let url = blob_client.url().map_err(Error::from)?;
                    debug_limited!(?url, "Generating presigned URL while running in anonymous mode");
                    url
                } else {
                    self.generate_sas(
                        &client,
                        &blob_client,
                        BlobSasPermissions {
                            read: true,
                            ..Default::default()
                        },
                        OffsetDateTime::from_unix_timestamp(
                            self.expiration_normalizer
                                .get_expiration()
                                .map_err(Error::from)?
                                .end_time(),
                        )
                        .map_err(Error::from)?,
                    )
                    .instrument(info_span!("Get Presigned URL"))
                    .await?
                };

                let mut headers = Vec::new();
                headers.push(Header {
                    name: VERSION.as_str().to_owned(),
                    value: X_MS_VERISION.to_string(),
                });
                if let Some(e) = resource_id.etag {
                    headers.push(Header {
                        name: IF_MATCH.as_str().to_owned(),
                        value: e,
                    });
                }
                // Identity carries etag only (no
                // last-modified), so no if-unmodified-since fallback is possible.

                if let Some(additional_headers) = additional_headers {
                    headers.extend(
                        additional_headers
                            .request_headers(RequestType::Data)
                            .map(|(k, v)| Header { name: k, value: v }),
                    );
                }

                // Add version id if it's present
                if let Some(v) = resource_id.version_id {
                    VersionId::new(v).append_to_url_query(&mut signed_url);
                }

                #[allow(deprecated)]
                let redirect = Redirect {
                    method: "GET".to_owned(),
                    redirect_target_url: signed_url.to_string(),
                    additional_headers: headers,
                };
                Ok(Box::pin(async_stream::stream! {
                    yield Ok(resource_info_message);
                    yield Ok(response_function(ReadPart::Redirect(redirect)));
                }))
            },
        }
    }

    pub fn parse_download_preference(pref: Option<i32>) -> Result<DownloadPreference> {
        match pref {
            Some(d) => match d {
                0 => Ok(DownloadPreference::Unspecified),
                1 => Ok(DownloadPreference::Body),
                2 => Ok(DownloadPreference::Redirect),
                _ => Err(Error::invalid_argument(format!("Invalid Download Preference provided: {d}",)).into()),
            },
            None => Ok(DownloadPreference::Unspecified),
        }
    }

    #[allow(clippy::too_many_arguments)]
    #[instrument(name = "azure::write_interal", skip_all)]
    pub async fn write_internal<F, T>(
        &self,
        destination_resource_address: impl AsRef<str> + Debug,
        previous_version: Option<impl AsRef<str> + Debug>,
        fail_if_exists: bool,
        data_object_size: u64,
        upload_preference: Option<i32>,
        data_future: BoxFuture<Bytes>,
        additional_headers: Option<AdditionalHeaders>,
        native_metadata: std::collections::HashMap<String, google::protobuf::Value>,
        response_function: F,
    ) -> Result<crate::traits::BoxStream<T>>
    where
        F: Fn(WritePart) -> T + std::marker::Send + 'static,
        T: std::marker::Send + 'static,
    {
        let upload_preference = match upload_preference {
            Some(pref) => match pref {
                pref if (pref == UploadPreference::Body as i32 || pref == UploadPreference::Unspecified as i32) => {
                    // Don't let direct uploads over the maximum configured size
                    if data_object_size <= self.config.write_body_max_size.as_u64() {
                        UploadPreference::Body
                    } else {
                        UploadPreference::Multipart
                    }
                },
                pref if pref == UploadPreference::Redirect as i32 => UploadPreference::Redirect,
                pref if pref == UploadPreference::Multipart as i32 => UploadPreference::Multipart,
                _ => {
                    return Err(Error::invalid_argument(format!("Unknown upload preference specified: {pref}")).into());
                },
            },
            None => match data_object_size {
                size if size < self.config.write_body_max_size.as_u64() => UploadPreference::Body,
                size if size >= self.config.write_body_max_size.as_u64()
                    && size < self.config.write_redirect_max_size.as_u64() =>
                {
                    UploadPreference::Redirect
                },
                _ => UploadPreference::Multipart,
            },
        };

        debug_limited!(?upload_preference);

        let addr_url = Url::parse(destination_resource_address.as_ref()).map_err(Error::from)?;
        let address = Address::parse_blob(addr_url)?;

        if address.is_versioned() {
            return Err(Error::invalid_argument(format!(
                "Versioned resource addresses are not supported for writing: {}",
                address
            ))
            .into());
        }
        if fail_if_exists && previous_version.is_some() {
            return Err(Error::invalid_argument("fail_if_exists and previous_version are mutually exclusive").into());
        }

        // Native metadata writes are not supported while GetNativeMetadataSupport reports false. Validate
        // the map first so a malformed one still yields INVALID_ARGUMENT (the address is already parsed),
        // then reject any non-empty map with UNIMPLEMENTED regardless of native_metadata_enabled or upload
        // preference. This keeps the write path coherent with the advertised capability; read-side surfacing
        // of native metadata stays behind native_metadata_enabled.
        let backend_metadata: Option<azure_core::request_options::Metadata> = if native_metadata.is_empty() {
            None
        } else {
            azure_metadata_from_native(&native_metadata)?;
            return Err(crate::traits::error::UnimplementedError::with_message(
                "native metadata writes are not supported by this backend (GetNativeMetadataSupport reports false)",
            )
            .into());
        };

        let azure_client = self.client(&address).await?;
        let blob_client = azure_client
            .container_client(address.container()?)
            .blob_client(address.blob());

        match upload_preference {
            UploadPreference::Body => {
                // Collect data
                let custom_context = additional_headers
                    .map(|h| Self::create_custom_context(h, RequestType::Data))
                    .transpose()?;
                let previous_version = previous_version.as_ref().map(|v| v.as_ref().to_string());
                let rate_limit_counter = self.rate_limit_counters.put_block_blob.clone();
                Ok(Box::pin(async_stream::stream! {
                    // Native metadata rides on the same PutBlob request — no extra call. It was
                    // validated and converted above; the proto map is echoed on the response.
                    let native_metadata = native_metadata;
                    let backend_metadata = backend_metadata;
                    // Tell client to send us data
                    yield Ok(response_function(WritePart::WriteChunksAccepted));

                    // Get the data from the caller
                    let buffer = match data_future.await {
                        Ok(b) => b,
                        Err(e) => {
                            yield Err(e);
                            return;
                        },
                    };

                    let mut builder = blob_client.put_block_blob(buffer);
                    if let Some(custom_context) = custom_context {
                        builder = builder.context(custom_context);
                    }
                    if let Some(md) = backend_metadata {
                        builder = builder.metadata(md);
                    }

                    // Check if a resource id was provided
                    if let Some(id) = previous_version.as_ref() {
                        let resource_id = match Identity::parse(id) {
                            Ok(i) => i,
                            Err(e) => {
                                yield Err(e.into());
                                return;
                            },
                        };
                        // Identity carries etag only.
                        // No last-modified fallback; identity without an etag is
                        // malformed.
                        match resource_id.etag {
                            Some(e) => {
                                builder = builder.if_match(IfMatchCondition::Match(e));
                            },
                            None => {
                                yield Err(Error::invalid_argument(format!(
                                    "Resource identifier did not contain etag: {}",
                                    id
                                ))
                                .into());
                                return;
                            },
                        }
                    }
                    if fail_if_exists {
                        builder = builder.if_match(IfMatchCondition::NotMatch("*".to_string()));
                    }

                    let write_res = match builder
                        .into_future()
                        .instrument(info_span!("Put Blob"))
                        .await
                        .inspect_err(|e| rate_limit_counter.check_error(e))
                        .map_err(Error::from)
                    {
                        Ok(w) => w,
                        Err(e) => {
                            let e = if fail_if_exists && e.is_precondition_failed() {
                                Error::conflict("Resource already exists")
                            } else {
                                e
                            };
                            yield Err(e.into());
                            return;
                        },
                    };

                    debug_limited!(?write_res);

                    // Echo the accepted native metadata on the write response (no extra backend call);
                    // the PutBlob response itself does not return it.
                    let mut written = WrappedResourceInfo::from((address.into(), write_res, data_object_size)).0;
                    written.native_metadata = native_metadata;
                    yield Ok(response_function(WritePart::ResourceInfo(written)));
                }))
            },
            UploadPreference::Redirect => {
                // Generate a presigned url for the blob
                let signed_url = self
                    .generate_sas(
                        &azure_client,
                        &blob_client,
                        BlobSasPermissions {
                            write: true,
                            ..Default::default()
                        },
                        OffsetDateTime::from_unix_timestamp(
                            self.expiration_normalizer
                                .get_expiration()
                                .map_err(Error::from)?
                                .end_time(),
                        )
                        .map_err(Error::from)?,
                    )
                    .instrument(info_span!("Get Presigned URL"))
                    .await?;

                let mut request_headers = Vec::new();
                if let Some(id) = previous_version {
                    let resource_id = Identity::parse(id.as_ref())?;
                    // Identity carries etag only — no
                    // last-modified fallback.
                    if let Some(e) = resource_id.etag {
                        request_headers.push(Header {
                            name: IF_MATCH.as_str().to_owned(),
                            value: e,
                        });
                    }
                }
                if fail_if_exists {
                    request_headers.push(Header {
                        name: IF_NONE_MATCH.as_str().to_owned(),
                        value: "*".to_string(),
                    });
                }

                if let Some(additional_headers) = additional_headers {
                    request_headers.extend(
                        additional_headers
                            .request_headers(RequestType::Data)
                            .map(|(k, v)| Header { name: k, value: v }),
                    );
                }

                request_headers.push(Header {
                    name: VERSION.as_str().to_owned(),
                    value: X_MS_VERISION.to_string(),
                });
                request_headers.push(Header {
                    name: BLOB_TYPE.as_str().to_owned(),
                    value: "BlockBlob".to_string(),
                });

                let redirect = WriteRedirect {
                    redirect_target_url: signed_url.to_string(),
                    method: UploadMethod::Put,
                    additional_headers: request_headers,
                    // Request the necessary headers to make a resource identity
                    completion_header_names: vec![
                        ETAG.as_str().to_owned(),
                        LAST_MODIFIED.as_str().to_owned(),
                        AZURE_VERSION_ID_HEADER.to_owned(),
                    ],
                };
                Ok(Box::pin(async_stream::stream! {
                    yield Ok(response_function(WritePart::Redirect(redirect)));
                }))
            },
            UploadPreference::Multipart => {
                // No need to create an empty blob here. The first call to PutBlock will create an empty
                // blob if it doesn't already exist.

                let upload_id = Uuid::new_v4().to_string();
                let redirect = self
                    .create_upload_part(&upload_id, 0, &address, additional_headers)
                    .await?;
                let multipart = MultipartUpload {
                    // Encode the upload id into the upload id
                    upload_id: upload_id::encode(
                        &upload_id,
                        previous_version.as_ref().map(|v| v.as_ref()),
                        fail_if_exists,
                    ),
                    first_part_write_redirect: redirect,
                    minimum_size_per_part: Some(0),
                    // 4000 MiB
                    maximum_size_per_part: Some(4000 * 1024 * 1024),
                    maximum_parts_number: Some(50000),
                };
                Ok(Box::pin(async_stream::stream! {
                    yield Ok(response_function(WritePart::MultipartUpload(multipart)));
                }))
            },
            UploadPreference::Unspecified => {
                // Should never reach here with Unspecified because we check that above and pick the action base
                // on size
                Err(Error::internal("Could not determine upload preference").into())
            },
        }
    }

    pub async fn create_upload_part(
        &self,
        upload_id: impl AsRef<str>,
        part_number: u32,
        address: &Address,
        additional_headers: Option<AdditionalHeaders>,
    ) -> Result<WriteRedirect> {
        // Generate a presigned url for the blob
        let azure_client = self.client(address).await?;
        let blob_client = azure_client
            .container_client(address.container()?)
            .blob_client(address.blob());

        let mut signed_url = self
            .generate_sas(
                &azure_client,
                &blob_client,
                BlobSasPermissions {
                    write: true,
                    ..Default::default()
                },
                OffsetDateTime::from_unix_timestamp(
                    self.expiration_normalizer
                        .get_expiration()
                        .map_err(Error::from)?
                        .end_time(),
                )
                .map_err(Error::from)?,
            )
            .instrument(info_span!("Get Shared Access Signature"))
            .await?;

        // Now that we have the signed url, have to add query params to put a block list
        let block_id = BlockId::new(Self::generate_block_id(upload_id.as_ref(), part_number as i32));
        block_id.append_to_url_query(&mut signed_url);

        // Indicate this is a block put
        signed_url.query_pairs_mut().append_pair("comp", "block");

        let mut request_headers = Vec::new();

        if let Some(additional_headers) = additional_headers {
            request_headers.extend(
                additional_headers
                    .request_headers(RequestType::Data)
                    .map(|(k, v)| Header { name: k, value: v }),
            );
        }

        Ok(WriteRedirect {
            redirect_target_url: signed_url.to_string(),
            method: UploadMethod::Put,
            additional_headers: request_headers,
            completion_header_names: Vec::new(),
        })
    }

    pub async fn upload_part_internal(
        &self,
        upload_id: impl AsRef<str>,
        destination_resource_address: impl AsRef<str> + Debug,
        part_number: u32,
        part_count: Option<u32>,
        additional_headers: Option<AdditionalHeaders>,
    ) -> Result<Vec<WriteRedirect>> {
        let url = Url::parse(destination_resource_address.as_ref()).map_err(Error::from)?;
        let address = Address::parse_blob(url)?;

        let (upload_id, _) = upload_id::decode(upload_id.as_ref());

        let part_count = part_count.unwrap_or(1);
        if part_count == 0 {
            return Err(
                Error::invalid_argument("Provided part count is 0. A specified part count must be >= 1").into(),
            );
        }
        let mut parts = Vec::with_capacity(part_count as usize);

        for part_number in part_number..part_number + part_count {
            parts.push(
                self.create_upload_part(&upload_id, part_number, &address, additional_headers.clone())
                    .await?,
            );
        }

        debug_limited!(?parts);

        Ok(parts)
    }

    pub fn generate_block_id(key: impl AsRef<str>, part_number: i32) -> Bytes {
        // All block IDs must be the same length before encoding, so pad part number out to six spaces,
        // to allow for the max of 100000 blocks
        Bytes::from(format!("{}-{:06}", key.as_ref(), part_number))
    }

    fn copy_source_url(source_resource_identity: &Identity, is_move: bool) -> Url {
        let mut copy_source = source_resource_identity.resource_address.resource_address.clone();
        // Caller query parameters are useful when resolving an identity, but
        // they are not part of Azure's Copy Blob source URL. Forwarding them in
        // x-ms-copy-source can change the backend request's semantics.
        copy_source.set_query(None);
        if let Some(version_id) = source_resource_identity.version_id.as_ref().filter(|_| !is_move) {
            copy_source.query_pairs_mut().append_pair("versionId", version_id);
        }
        copy_source
    }

    /// Copy a blob from one address to another.
    ///
    /// This function takes parsed parameters instead of strings because move needs to preparse everything to ensure
    /// all addresses are valid before making any sdk calls.
    pub async fn copy_internal(
        &self,
        source_resource_identity: Identity,
        destination_resource_address: Address,
        previous_version: Option<Identity>,
        fail_if_exists: bool,
        additional_headers: Option<AdditionalHeaders>,
        is_move: bool,
    ) -> Result<ResourceInfo> {
        if destination_resource_address.is_versioned() {
            return Err(Error::invalid_argument(format!(
                "Versioned resource addresses are not supported for copy/move destination: {}",
                destination_resource_address
            ))
            .into());
        }
        if fail_if_exists && previous_version.is_some() {
            return Err(Error::invalid_argument("fail_if_exists and previous_version are mutually exclusive").into());
        }

        self.check_endpoint_configured(&destination_resource_address.resource_address)
            .await?;

        // The Copy Blob PUT must be authenticated against the *destination*
        // account. Building the destination blob client on the source client (as
        // it did before) silently wrote into the source account under the
        // destination's container name and returned an identity for the
        // destination; `move` then deleted the source — data loss.
        let client = self.client(&destination_resource_address).await?;
        let source_client = self.client(source_resource_identity.resource_address()).await?;

        // We only allow copying from a version for copy operations, not move operations.
        let mut copy_source = Self::copy_source_url(&source_resource_identity, is_move);

        // For a cross-account copy the destination account cannot read the source
        // with its own credentials, so hand Copy Blob a short-lived read-SAS'd
        // source URL. Same-account copies (the common case) keep the bare source
        // URL for an intra-account server-side copy; a public/anonymous source
        // needs no SAS.
        let same_account = source_resource_identity.resource_address.resource_address.host_str()
            == destination_resource_address.resource_address.host_str();
        if !same_account && !source_client.anonymous {
            // The SDK's SAS helpers only sign blob-scoped tokens (sr=b) and
            // always leave the string-to-sign's snapshot/version field empty.
            // Azure requires a version-scoped SAS (sr=bv, signed over the
            // version id) to authorize reads of a specific version — appending
            // `versionId` to an already-signed sr=b token does not — so reject
            // rather than hand Copy Blob a source URL that fails authorization.
            if let Some(version_id) = source_resource_identity.version_id.as_ref().filter(|_| !is_move) {
                return Err(Error::invalid_argument(format!(
                    "Cross-account copy of a specific source version is not supported: cannot generate a \
                     version-scoped SAS for {} (versionId {})",
                    source_resource_identity.resource_address, version_id
                ))
                .into());
            }
            let source_blob_client = source_client
                .container_client(source_resource_identity.container()?)
                .blob_client(source_resource_identity.blob());
            let expiry = OffsetDateTime::from_unix_timestamp(
                self.expiration_normalizer
                    .get_expiration()
                    .map_err(Error::from)?
                    .end_time(),
            )
            .map_err(Error::from)?;
            let sas_source = self
                .generate_sas(
                    &source_client,
                    &source_blob_client,
                    BlobSasPermissions {
                        read: true,
                        ..Default::default()
                    },
                    expiry,
                )
                .instrument(info_span!("Copy Blob Source SAS"))
                .await?;
            copy_source = sas_source;
        }

        let mut builder = client
            .container_client(destination_resource_address.container()?)
            .blob_client(destination_resource_address.blob())
            .copy(copy_source);

        // Clone additional_headers before consuming it on the builder context
        // so we can re-use them on the disambiguation get_properties call below.
        let additional_headers_for_disambiguation = additional_headers.clone();
        if let Some(additional_headers) = additional_headers {
            builder = builder.context(Self::create_custom_context(additional_headers, RequestType::Api)?);
        }

        // Identity carries etag only — no last-modified
        // fallback. Both source and destination (previous_version) must have an
        // etag or they're malformed.
        let source_etag = source_resource_identity.etag.clone().ok_or_else(|| {
            Error::invalid_argument(format!(
                "Source resource identifier did not contain etag: {:?}",
                source_resource_identity
            ))
        })?;
        builder = builder.if_source_match(IfSourceMatchCondition::Match(source_etag));

        // Add destination checks if provided
        if let Some(destination_id) = previous_version {
            // Ensure the previous version blob matches the destination blob.
            if destination_id.blob() != destination_resource_address.blob() {
                return Err(Error::from(SdkError::precondition_failed(format!(
                    "Previous version blob does not match destination blob: {} != {}",
                    destination_id.blob(),
                    destination_resource_address.blob()
                )))
                .into());
            }
            let dest_etag = destination_id.etag.ok_or_else(|| {
                Error::invalid_argument(format!(
                    "Destination resource identifier did not contain etag: {:?}",
                    destination_resource_address
                ))
            })?;
            builder = builder.if_match(IfMatchCondition::Match(dest_etag));
        }
        if fail_if_exists {
            builder = builder.if_match(IfMatchCondition::NotMatch("*".to_string()));
        }

        let source_etag_for_disambiguation = source_resource_identity.etag.clone();
        let source_resource_address_for_disambiguation = source_resource_identity.resource_address.clone();
        // Mirror the version-targeting logic used in `copy_source` above so the
        // recheck queries the same version (or the latest, for moves). Otherwise
        // a destination conflict on `if_match(NotMatch("*"))` gets misclassified
        // as source staleness whenever the latest source differs from the
        // version actually being copied.
        let source_version_id_for_disambiguation = if is_move {
            None
        } else {
            source_resource_identity.version_id.clone()
        };
        let copy_res = match builder
            .into_future()
            .instrument(info_span!("Copy Blob"))
            .await
            .inspect_err(|e| self.rate_limit_counters.copy_blob.check_error(e))
            .map_err(Error::from)
        {
            Ok(res) => res,
            Err(err) if fail_if_exists && err.is_precondition_failed() => {
                // A 412 here is ambiguous between `if_source_match` (source
                // staleness) and `if_match(NotMatch("*"))` (destination
                // conflict). Recheck the source's etag to disambiguate. On
                // ambiguous recheck (transient/permissions error) we surface
                // the original 412 rather than guess.
                match self
                    .source_etag_recheck(
                        &source_resource_address_for_disambiguation,
                        source_etag_for_disambiguation.as_deref(),
                        source_version_id_for_disambiguation.as_deref(),
                        additional_headers_for_disambiguation,
                    )
                    .await
                {
                    EtagRecheck::SourceMatches => {
                        return Err(Error::conflict("Resource already exists").into());
                    },
                    EtagRecheck::SourceStale | EtagRecheck::Ambiguous => {
                        return Err(err.into());
                    },
                }
            },
            Err(err) => return Err(err.into()),
        };

        debug_limited!(?copy_res);

        // Azure Copy Blob is asynchronous: a 202 can report `pending` (typical
        // for the cross-account copies the SAS'd-source branch above enables),
        // meaning only an uncommitted placeholder exists at the destination.
        // Returning now would let `move` delete the source of an in-flight copy
        // — which fails the copy and destroys the object — so wait for a
        // terminal state before reporting the destination identity.
        match Self::classify_copy_status(
            &copy_res.copy_id,
            Some(&copy_res.copy_id),
            Some(copy_res.copy_status),
            None,
        )? {
            // The source's content-length is no longer on Identity. The
            // CopyBlobResponse doesn't carry it either, so we pass None — callers who
            // need it will get it from a subsequent stat (cache-fronted).
            CopyStatusPoll::Completed => {
                Ok(WrappedResourceInfo::from((destination_resource_address.into(), copy_res, None)).0)
            },
            CopyStatusPoll::StillPending => {
                let blob = self
                    .await_copy_completion(
                        &client,
                        &destination_resource_address,
                        &copy_res.copy_id,
                        additional_headers_for_disambiguation,
                    )
                    .await?;
                // A pending 202's etag belongs to the placeholder — the blob is
                // rewritten when the copy commits — so build the identity from
                // the destination's final properties instead of `copy_res`.
                let version_id = blob.version_id.clone();
                Ok(WrappedResourceInfo::from((destination_resource_address.into(), blob, version_id)).0)
            },
        }
    }

    /// Poll the destination blob of an in-flight Copy Blob operation until it
    /// reaches a terminal state, returning the destination's final properties.
    ///
    /// Bounded by the configured operation timeout; a copy still pending at the
    /// deadline is surfaced as a timeout error so callers never act on an
    /// uncommitted destination. The SDK exposes no Abort Copy operation, so a
    /// timed-out copy is left running in the background — the error message
    /// says so.
    async fn await_copy_completion(
        &self,
        client: &AzureClient,
        destination: &Address,
        copy_id: &CopyId,
        additional_headers: Option<AdditionalHeaders>,
    ) -> std::result::Result<Blob, Error> {
        const INITIAL_POLL_INTERVAL: Duration = Duration::from_millis(500);
        const MAX_POLL_INTERVAL: Duration = Duration::from_secs(5);

        let blob_client = client
            .container_client(destination.container()?)
            .blob_client(destination.blob());

        let deadline = tokio::time::Instant::now() + self.config.timeouts.operation;
        let mut interval = INITIAL_POLL_INTERVAL;
        loop {
            tokio::time::sleep(interval).await;
            interval = (interval * 2).min(MAX_POLL_INTERVAL);

            let mut builder = blob_client.get_properties();
            if let Some(additional_headers) = additional_headers.clone() {
                builder = builder.context(Self::create_custom_context(additional_headers, RequestType::Api)?);
            }
            let properties = builder
                .into_future()
                .instrument(info_span!("Copy Status Poll"))
                .await
                .inspect_err(|e| self.rate_limit_counters.get_properties.check_error(e))
                .map_err(Error::from)?;

            let blob = properties.blob;
            match Self::classify_copy_status(
                copy_id,
                blob.properties.copy_id.as_ref(),
                blob.properties.copy_status,
                blob.properties.copy_status_description.as_deref(),
            )? {
                CopyStatusPoll::Completed => return Ok(blob),
                CopyStatusPoll::StillPending => {
                    if tokio::time::Instant::now() >= deadline {
                        return Err(SdkError::Timeout {
                            message: format!(
                                "Copy Blob {copy_id} to {destination} did not complete within {:?}; \
                                 the copy may still be running in the background",
                                self.config.timeouts.operation
                            ),
                            sdk_error_code: None,
                        }
                        .into());
                    }
                },
            }
        }
    }

    /// Classify a single observation of the destination blob's copy state for
    /// the Copy Blob operation `expected_copy_id`. Terminal failures — a
    /// failed/aborted status, the copy id being replaced by a competing
    /// writer, or the copy markers missing entirely — come back as errors so
    /// callers preserve the source. Pure so the pending-copy handling is
    /// unit-testable without a live backend.
    fn classify_copy_status(
        expected_copy_id: &CopyId,
        observed_copy_id: Option<&CopyId>,
        copy_status: Option<CopyStatus>,
        copy_status_description: Option<&str>,
    ) -> std::result::Result<CopyStatusPoll, Error> {
        if observed_copy_id != Some(expected_copy_id) {
            return Err(Error::conflict(format!(
                "Copy operation {expected_copy_id} was replaced on the destination (current copy id: {})",
                observed_copy_id.map_or_else(|| "none".to_string(), ToString::to_string)
            )));
        }
        match copy_status {
            Some(CopyStatus::Success) => Ok(CopyStatusPoll::Completed),
            Some(CopyStatus::Pending) => Ok(CopyStatusPoll::StillPending),
            Some(status @ (CopyStatus::Failed | CopyStatus::Aborted)) => Err(Error::internal(format!(
                "Copy operation {expected_copy_id} did not complete: status {}, description: {}",
                <&str>::from(status),
                copy_status_description.unwrap_or("<none>")
            ))),
            None => Err(Error::internal(format!(
                "Copy operation {expected_copy_id} reports no copy status on the destination"
            ))),
        }
    }

    /// Delete a blob.
    ///
    /// This function returns a Result<(), Error> instead of Result<()> because `move_object_internal` needs to inspect the error.
    pub async fn delete_internal(
        &self,
        resource_address: impl AsRef<str> + Debug,
        previous_version: Option<impl AsRef<str> + Debug>,
        additional_headers: Option<AdditionalHeaders>,
    ) -> std::result::Result<(), Error> {
        let url = Url::parse(resource_address.as_ref()).map_err(Error::from)?;
        let address = Address::parse_blob(url)?;

        if address.is_versioned() {
            return Err(Error::invalid_argument(format!(
                "Versioned resource addresses are not supported for deleting: {}",
                address
            )));
        }

        let mut builder = self
            .client(&address)
            .await?
            .container_client(address.container()?)
            .blob_client(address.blob())
            .delete();

        if let Some(additional_headers) = additional_headers {
            builder = builder.context(Self::create_custom_context(additional_headers, RequestType::Api)?);
        }

        // Check if a resource id was provided. The identity carries
        // etag only — no last-modified fallback.
        if let Some(id) = previous_version {
            let resource_id = Identity::parse(id.as_ref())?;
            let etag = resource_id.etag.ok_or_else(|| {
                Error::invalid_argument(format!("Resource identifier did not contain etag: {}", id.as_ref()))
            })?;
            builder = builder.if_match(IfMatchCondition::Match(etag));
        }

        let result = builder
            .into_future()
            .instrument(info_span!("Delete Blob"))
            .await
            .inspect_err(|e| self.rate_limit_counters.delete_blob.check_error(e))
            .map_err(SdkError::from);

        match result {
            Ok(delete_response) => {
                debug_limited!(?delete_response);
                Ok(())
            },
            Err(SdkError::NotFound { .. }) => {
                debug_limited!("Not Found returned for delete request. Returning Ok");
                Ok(())
            },
            Err(e) => Err(Error::from(e)),
        }
    }

    pub fn complete_redirect_upload_internal(
        &self,
        destination_resource_address: impl AsRef<str> + Debug,
        additional_headers: Vec<Header>,
    ) -> Result<ResourceInfo> {
        Self::resource_info_from_redirect_upload(destination_resource_address, additional_headers)
    }

    fn resource_info_from_redirect_upload(
        destination_resource_address: impl AsRef<str> + Debug,
        additional_headers: Vec<Header>,
    ) -> Result<ResourceInfo> {
        let address = Address::parse_blob(Url::parse(destination_resource_address.as_ref()).map_err(Error::from)?)?;

        let mut etag = None;
        let mut last_modified = None;
        let mut version_id = None;
        for header in &additional_headers {
            match &header.name {
                h if h.eq_ignore_ascii_case(ETAG.as_str()) => etag = Some(header.value.clone()),
                h if h.eq_ignore_ascii_case(LAST_MODIFIED.as_str()) => last_modified = Some(header.value.clone()),
                h if h.eq_ignore_ascii_case(AZURE_VERSION_ID_HEADER) => version_id = Some(header.value.clone()),
                _ => continue,
            }
        }

        if etag.is_none() {
            return Err(Error::invalid_argument(format!(
                "Requested {:?} header not returned. Provided headers: {:?}",
                ETAG, additional_headers
            ))
            .into());
        }

        // `last-modified` is no longer required to complete the redirect (matches
        // the S3 backend's behavior). When the client relays it
        // we populate `last_modified_timestamp`; when absent we leave it as
        // `None` and let the normal metadata flow supply it.
        let last_modified_timestamp = last_modified
            .as_deref()
            .map(|l| {
                parse_rfc1123(l)
                    .map(|t| google::protobuf::Timestamp {
                        seconds: t.unix_timestamp(),
                        nanos: t.nanosecond() as i32,
                    })
                    .map_err(Error::from)
            })
            .transpose()?;

        Ok(ResourceInfo {
            native_metadata: Default::default(),
            encoded_identity: Identity::create_url(address.into(), etag, version_id),
            data_object_size: None,
            last_modified_timestamp,
        })
    }

    pub async fn complete_multipart_upload_internal(
        &self,
        upload_id: impl AsRef<str> + Debug,
        destination_resource_address: impl AsRef<str> + Debug,
        mut parts: Vec<CompletedUploadPart>,
        additional_headers: Option<AdditionalHeaders>,
    ) -> Result<ResourceInfo> {
        let url = Url::parse(destination_resource_address.as_ref()).map_err(Error::from)?;
        let address = Address::parse_blob(url)?;

        let decoded_upload_id = upload_id::decode_with_options(upload_id.as_ref());
        let upload_id = decoded_upload_id.upload_id;
        let previous_version = decoded_upload_id.resource_identity;

        // Sort part numbers so that we get the block ids in the right order
        parts.sort_by_key(|a| a.part_number);

        // Gather all block ids
        let mut block_list = BlockList::default();
        for part in parts {
            block_list
                .blocks
                .push(BlobBlockType::new_uncommitted(BlockId::new(Self::generate_block_id(
                    upload_id,
                    part.part_number as i32,
                ))));
        }

        let mut builder = self
            .client(&address)
            .await?
            .container_client(address.container()?)
            .blob_client(address.blob())
            .put_block_list(block_list);

        if let Some(additional_headers) = additional_headers {
            builder = builder.context(Self::create_custom_context(additional_headers, RequestType::Api)?);
        }

        // Add destination checks if provided. Identity carries etag
        // only — no last-modified fallback.
        if let Some(id) = previous_version {
            let destination_id = Identity::parse(id)?;
            let etag = destination_id
                .etag
                .ok_or_else(|| Error::invalid_argument(format!("Resource identifier did not contain etag: {id}")))?;
            builder = builder.if_match(IfMatchCondition::Match(etag));
        }
        if decoded_upload_id.fail_if_exists {
            builder = builder.if_match(IfMatchCondition::NotMatch("*".to_string()));
        }

        let complete_res = builder
            .into_future()
            .instrument(info_span!("Put Block List"))
            .await
            .inspect_err(|e| self.rate_limit_counters.put_block_list.check_error(e))
            .map_err(Error::from)
            .map_err(|err| {
                if decoded_upload_id.fail_if_exists && err.is_precondition_failed() {
                    Error::conflict("Resource already exists")
                } else {
                    err
                }
            })?;

        debug_limited!(?complete_res);

        Ok(WrappedResourceInfo::from((address.into(), complete_res)).0)
    }

    pub async fn move_object_internal(
        &self,
        source_resource_address: impl AsRef<str> + Debug,
        destination_resource_address: impl AsRef<str> + Debug,
        source_previous_version: Option<impl AsRef<str> + Debug>,
        destination_previous_version: Option<impl AsRef<str> + Debug>,
        fail_if_exists: bool,
        additional_headers: Option<AdditionalHeaders>,
    ) -> Result<ResourceInfo> {
        let source_addr = Address::parse_blob(Url::parse(source_resource_address.as_ref()).map_err(Error::from)?)?;
        if source_addr.is_versioned() {
            return Err(Error::invalid_argument(format!(
                "Versioned resource addresses are not supported for move source: {}",
                source_addr
            ))
            .into());
        }

        // Can't move a folder placeholder object
        if source_addr.is_folder_placeholder() {
            return Err(Error::from(SdkError::not_found(format!(
                "Cannot move a folder placeholder object {}",
                source_resource_address.as_ref()
            )))
            .into());
        }

        // Parse the destination resource address prior to issuing any sdk calls to ensure it's a valid address.
        let dest_addr = Address::parse_blob(Url::parse(destination_resource_address.as_ref()).map_err(Error::from)?)?;

        // Get the source resource identity. If it's not provided, we need to stat it.
        // We need to get a resource identity so we can do a conditional delete of the source resource.
        let source_id = if let Some(id) = source_previous_version {
            id.as_ref().to_string()
        } else {
            self.stat_internal(source_resource_address.as_ref(), additional_headers.clone())
                .await?
                .encoded_identity
        };

        // Copy the source resource to the destination resource.
        let resource_info = self
            .copy_internal(
                Identity::parse(&source_id)?,
                dest_addr,
                destination_previous_version
                    .map(|v| Identity::parse(v.as_ref()))
                    .transpose()?,
                fail_if_exists,
                additional_headers.clone(),
                true,
            )
            .await?;

        // Delete the source resource only if it still matches the original resource identity.
        match self
            .delete_internal(source_resource_address, Some(&source_id), additional_headers.clone())
            .await
        {
            Ok(_) => Ok(resource_info),
            Err(e) => match e {
                Error::Sdk {
                    source: SdkError::NotFound { .. },
                    ..
                } => {
                    debug_limited!("Source resource not found. Returning Ok");
                    Ok(resource_info)
                },
                Error::Sdk {
                    source: SdkError::PreconditionFailed { .. },
                    ..
                } => {
                    debug_limited!("Source resource changed. Returning Ok");
                    Ok(resource_info)
                },
                _ => Err(e
                    .prepend_message("Source object has been copied to destination, but failed to delete source")
                    .into()),
            },
        }
    }

    /// Generates a presigned URL for a blob by creating a SAS (Shared Access Signature) token.
    ///
    /// Both auth paths produce SAS tokens with the same epoch-aligned expiry from the
    /// `ExpirationNormalizer`. This ensures that all service replicas generate identical
    /// presigned URLs for the same blob within the same epoch, enabling downstream caching
    /// (CDN, nginx, etc.).
    ///
    /// - **Account key auth**: Signs the SAS using the storage account's access key.
    ///   The `expiry` parameter is used directly as the SAS `se` (signed expiry).
    ///
    /// - **Service principal auth**: Signs the SAS using a User Delegation Key. The SDK's
    ///   `user_delegation_shared_access_signature` uses the delegation key's `signed_expiry`
    ///   as the SAS `se`. We request the delegation key with `signed_expiry` set to the same
    ///   epoch-aligned `expiry`, so both paths produce identical SAS expiry times.
    ///   See `get_or_fetch_delegation_key` for details.
    async fn generate_sas(
        &self,
        client: &AzureClient,
        blob_client: &BlobClient,
        permissions: BlobSasPermissions,
        expiry: OffsetDateTime,
    ) -> std::result::Result<Url, Error> {
        let sas = if client.use_user_delegation {
            let delegation_key = self.get_or_fetch_delegation_key(&client.client, expiry).await?;
            blob_client
                .user_delegation_shared_access_signature(permissions, &delegation_key)
                .await
                .map_err(Error::from)?
        } else {
            blob_client
                .shared_access_signature(permissions, expiry)
                .await
                .map_err(Error::from)?
        };
        blob_client.generate_signed_blob_url(&sas).map_err(Error::from)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::azure::secrets::{ClientCredentials, Secrets};
    use crate::azure::service::Config;

    fn copy_id(uuid: &str) -> CopyId {
        uuid.parse().unwrap()
    }

    #[test]
    fn classify_copy_status_success_is_completed() {
        let id = copy_id("11111111-2222-3333-4444-555555555555");
        let poll = Storage::classify_copy_status(&id, Some(&id), Some(CopyStatus::Success), None).unwrap();
        assert_eq!(poll, CopyStatusPoll::Completed);
    }

    #[test]
    fn classify_copy_status_pending_is_still_pending() {
        let id = copy_id("11111111-2222-3333-4444-555555555555");
        let poll = Storage::classify_copy_status(&id, Some(&id), Some(CopyStatus::Pending), None).unwrap();
        assert_eq!(poll, CopyStatusPoll::StillPending);
    }

    #[test]
    fn classify_copy_status_failed_is_error_with_description() {
        let id = copy_id("11111111-2222-3333-4444-555555555555");
        let err = Storage::classify_copy_status(&id, Some(&id), Some(CopyStatus::Failed), Some("read timed out"))
            .expect_err("failed copy must be an error");
        assert!(err.to_string().contains("read timed out"), "unexpected error: {err}");
    }

    #[test]
    fn classify_copy_status_aborted_is_error() {
        let id = copy_id("11111111-2222-3333-4444-555555555555");
        Storage::classify_copy_status(&id, Some(&id), Some(CopyStatus::Aborted), None)
            .expect_err("aborted copy must be an error");
    }

    #[test]
    fn classify_copy_status_replaced_copy_id_is_error() {
        let id = copy_id("11111111-2222-3333-4444-555555555555");
        let other = copy_id("99999999-8888-7777-6666-555555555555");
        let err = Storage::classify_copy_status(&id, Some(&other), Some(CopyStatus::Success), None)
            .expect_err("a competing copy must be an error");
        assert!(err.to_string().contains("replaced"), "unexpected error: {err}");
    }

    #[test]
    fn classify_copy_status_missing_copy_markers_is_error() {
        let id = copy_id("11111111-2222-3333-4444-555555555555");
        Storage::classify_copy_status(&id, None, None, None).expect_err("missing copy id must be an error");
        Storage::classify_copy_status(&id, Some(&id), None, None).expect_err("missing copy status must be an error");
    }

    #[test]
    fn copy_source_url_strips_caller_query_parameters() {
        let identity = Identity::parse(
            "https://account.blob.core.windows.net/container/blob?download=1&etag=%22abc%22&custom=a%2Bb&version-id=v1",
        )
        .unwrap();

        assert_eq!(
            Storage::copy_source_url(&identity, false).as_str(),
            "https://account.blob.core.windows.net/container/blob?versionId=v1"
        );
        assert_eq!(
            Storage::copy_source_url(&identity, true).as_str(),
            "https://account.blob.core.windows.net/container/blob"
        );
    }

    #[tokio::test]
    async fn copy_rejects_cross_account_versioned_source() {
        // Both accounts are configured with credentials, so the copy reaches
        // the cross-account SAS branch and must be rejected there — the SDK
        // cannot sign a version-scoped (sr=bv) SAS for the source. The
        // rejection fires before any network call.
        let credentials = || ClientCredentials {
            storage_key: Some("a2V5".to_string()),
            ..Default::default()
        };
        let storage = Storage::new(
            Config::default(),
            Some(Secrets(HashMap::from([
                ("https://source.blob.core.windows.net".to_string(), credentials()),
                ("https://dest.blob.core.windows.net".to_string(), credentials()),
            ]))),
        )
        .expect("storage construction should succeed");

        let err = storage
            .copy_internal(
                Identity::parse("https://source.blob.core.windows.net/container/blob?etag=%22abc%22&version-id=v1")
                    .unwrap(),
                Address::parse_blob(Url::parse("https://dest.blob.core.windows.net/container/blob").unwrap()).unwrap(),
                None,
                false,
                None,
                false,
            )
            .await
            .expect_err("cross-account copy of a specific version must be rejected");
        assert!(
            err.to_string()
                .contains("Cross-account copy of a specific source version is not supported"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn stat_address_for_identity_preserves_version_id() {
        let identity =
            Identity::parse("https://account.blob.core.windows.net/container/blob?etag=%22abc%22&version-id=v1")
                .unwrap();

        let stat_address = Storage::stat_address_for_identity(&identity);

        assert_eq!(
            stat_address,
            "https://account.blob.core.windows.net/container/blob?version-id=v1"
        );
    }

    #[test]
    fn stat_address_for_identity_uses_unversioned_address_when_no_version_id() {
        let identity = Identity::parse("https://account.blob.core.windows.net/container/blob?etag=%22abc%22").unwrap();

        let stat_address = Storage::stat_address_for_identity(&identity);

        assert_eq!(stat_address, "https://account.blob.core.windows.net/container/blob");
    }

    #[test]
    fn redirect_upload_reads_azure_version_id_header_into_identity() {
        let info = Storage::resource_info_from_redirect_upload(
            "https://account.blob.core.windows.net/container/blob",
            vec![
                Header {
                    name: ETAG.as_str().to_owned(),
                    value: "\"abc\"".to_owned(),
                },
                Header {
                    name: "X-MS-VERSION-ID".to_owned(),
                    value: "2026-05-08T12:34:56.0000000Z".to_owned(),
                },
            ],
        )
        .unwrap();

        let identity = Identity::parse(&info.encoded_identity).unwrap();
        assert_eq!(identity.etag.as_deref(), Some("\"abc\""));
        assert_eq!(identity.version_id.as_deref(), Some("2026-05-08T12:34:56.0000000Z"));
    }

    #[test]
    fn redirect_upload_canonicalizes_identity_path() {
        const LOOSE_PATH: &str = "!$%&'()*+,:;=@[]^|";
        const STRICT_PATH: &str = "%21%24%25%26%27%28%29%2A%2B%2C%3A%3B%3D%40%5B%5D%5E%7C";

        let headers = || {
            vec![Header {
                name: ETAG.as_str().to_owned(),
                value: "\"abc\"".to_owned(),
            }]
        };
        let loose = Storage::resource_info_from_redirect_upload(
            format!("https://account.blob.core.windows.net/container/{LOOSE_PATH}"),
            headers(),
        )
        .unwrap();
        let strict_url = format!("https://account.blob.core.windows.net/container/{STRICT_PATH}");
        let strict = Storage::resource_info_from_redirect_upload(&strict_url, headers()).unwrap();

        assert_eq!(
            Url::parse(&loose.encoded_identity).unwrap().path(),
            format!("/container/{STRICT_PATH}")
        );
        assert_eq!(loose.encoded_identity, strict.encoded_identity);
    }
}
