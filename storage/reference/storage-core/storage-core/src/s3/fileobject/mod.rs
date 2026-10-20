// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

pub mod v1alpha;
pub mod v1beta;

use std::{sync::Arc, time::Duration};

use aws_sdk_s3::{
    error::{ProvideErrorMetadata, SdkError},
    operation::list_object_versions::{ListObjectVersionsError, ListObjectVersionsOutput},
    presigning::PresigningConfig,
    primitives::ByteStream,
    types::{CommonPrefix, CompletedMultipartUpload, CompletedPart, Object, ObjectVersion},
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use bytesize::ByteSize;
use serde::{Deserialize, Serialize};
use snafu::prelude::*;
use storage_core_util::{debug_limited, error_limited, trace_limited, warn_limited};
use tracing::{info_span, instrument, Instrument};

use crate::traits::{error::Result, BoxFuture, Bytes, StreamingOptions};

/// Convert FileObject-native metadata into an S3 user-metadata map. S3 metadata
/// values are strings, so non-string protobuf values are rejected with
/// INVALID_ARGUMENT (the proto contract for values a backend cannot represent).
fn s3_metadata_from_native(
    native: &std::collections::HashMap<String, storage_core_protos::google::protobuf::Value>,
) -> Result<std::collections::HashMap<String, String>> {
    use storage_core_protos::google::protobuf::value::Kind;
    native
        .iter()
        .map(|(key, value)| {
            // Native metadata keys are stored raw (not through the MetadataService key encoding),
            // so reject keys the backend cannot represent up front rather than surfacing an opaque
            // SDK error later.
            if !crate::s3::metadata::is_valid_s3_metadata_key(key) {
                return Err(error::Error::invalid_argument(format!(
                    "native metadata key '{key}' is not a valid S3 metadata key"
                ))
                .into());
            }
            match &value.kind {
                Some(Kind::StringValue(s)) => Ok((key.clone(), s.clone())),
                _ => Err(error::Error::invalid_argument(format!(
                    "native metadata value for key '{key}' must be a string for this backend"
                ))
                .into()),
            }
        })
        .collect()
}

use storage_core_util::{
    additional_headers::{AdditionalHeaders, RequestType},
    api_types::{
        CompletedUploadPart, DownloadPreference, Header, MultipartUpload, ReadPart, Redirect, ResourceInfo,
        UploadMethod, UploadPreference, WritePart, WriteRedirect,
    },
    upload_id,
};

use super::{
    error,
    resource::{encode_path, Address, AddressingStyle, Identity, WrappedResourceInfo, ETAG},
    secrets::ClientCredentials,
    service::{Client, Storage},
};

const VERSION_ID_HEADER: &str = "x-amz-version-id";

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum ListObjectVersionsFallback {
    NoFallback,
    /// 403/AccessDenied. Caller lacks `s3:ListBucketVersions` on this bucket.
    /// Common for anonymous reads of public buckets where `s3:ListBucket` is
    /// allowed but `s3:ListBucketVersions` is not (e.g.
    /// `omniverse-content-production`). Cacheable in the per-Client latch:
    /// the latch is cleared on secrets reload alongside the rest of the
    /// `ClientMap`, so credential rotation re-probes — we don't strand a
    /// caller on `ListObjectsV2` after they gain the permission.
    CacheableDenied,
    /// 2xx-no-code (GCS structural mismatch) or 501/NotImplemented. Backend
    /// genuinely does not support `ListObjectVersions`.
    CacheableUnsupported,
}

/// Classify whether a `ListObjectVersions` error should fall back to
/// `ListObjectsV2`, and whether that observation is strong enough to cache as
/// a backend capability result.
///
/// For the GCS S3-interop shape, we identify three structural conditions that
/// must all hold:
///
/// 1. The error is `SdkError::ServiceError(_)` — the SDK did receive an HTTP
///    response from the backend. This rules out timeouts, DNS failures,
///    connection drops, and construction errors, where we have no evidence
///    about API support.
///
/// 2. The response's HTTP status is 2xx (success). Real service errors from
///    S3 carry 4xx/5xx (NotFound, AccessDenied, Throttling, etc.); a 2xx with
///    an error in the error channel means "backend thinks it replied OK but
///    the SDK disagreed with the body." That is specific to schema mismatches.
///
/// 3. `ProvideErrorMetadata::code()` returns `None`. A modeled S3 error (like
///    `NoSuchBucket`) carries its exception code in the response body and the
///    SDK surfaces it here. A deserialization failure — which is what
///    `ListObjectVersionsError::Unhandled` wraps — has no code because the
///    SDK never got far enough to extract one. We match on `code().is_none()`
///    rather than matching the `Unhandled` variant directly because the SDK
///    explicitly asks callers not to match its Unhandled variant (it plans to
///    add more variants over time; matching by code is forwards-compatible).
///
/// Conditions 2 and 3 together are load-bearing: 2xx alone is not enough
/// (`CompleteMultipartUpload` is documented to return 200 with an XML error
/// body that DOES carry an error code), and "no code" alone is not enough
/// (a 5xx Unhandled could be a transient gateway failure we should retry).
///
/// Transient network XML corruption would typically surface as
/// `SdkError::ResponseError` (response arrived but body was truncated or
/// malformed mid-stream) rather than a clean 2xx with no error code, so we
/// don't risk poisoning the cache on those.
fn classify_list_object_versions_fallback(
    err: &SdkError<ListObjectVersionsError, aws_smithy_runtime_api::http::Response>,
) -> ListObjectVersionsFallback {
    let SdkError::ServiceError(svc) = err else {
        return ListObjectVersionsFallback::NoFallback;
    };

    let status = svc.raw().status();
    let code = svc.err().code();

    if status.is_success() && code.is_none() {
        return ListObjectVersionsFallback::CacheableUnsupported;
    }

    if status.as_u16() == 501 || matches!(code, Some("NotImplemented")) {
        return ListObjectVersionsFallback::CacheableUnsupported;
    }

    if status.as_u16() == 403 || matches!(code, Some("AccessDenied")) {
        return ListObjectVersionsFallback::CacheableDenied;
    }

    ListObjectVersionsFallback::NoFallback
}

const LIST_OBJECT_VERSIONS_PAGE_MARKERS_PREFIX: &str = "s3lov1:";

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(in crate::s3) struct ListObjectVersionsPageMarkers {
    #[serde(rename = "k", skip_serializing_if = "Option::is_none")]
    pub key_marker: Option<String>,
    #[serde(rename = "v", skip_serializing_if = "Option::is_none")]
    pub version_id_marker: Option<String>,
    #[serde(rename = "t", skip_serializing_if = "Option::is_none")]
    pub list_objects_v2_continuation_token: Option<String>,
    #[serde(rename = "ld", default, skip_serializing_if = "is_false")]
    pub list_versions_done: bool,
    #[serde(rename = "od", default, skip_serializing_if = "is_false")]
    pub list_objects_v2_done: bool,
}

impl ListObjectVersionsPageMarkers {
    fn is_empty(&self) -> bool {
        self.key_marker.is_none()
            && self.version_id_marker.is_none()
            && self.list_objects_v2_continuation_token.is_none()
            && !self.list_versions_done
            && !self.list_objects_v2_done
    }

    fn is_complete(&self) -> bool {
        self.list_versions_done && self.list_objects_v2_done
    }

    fn for_list_object_versions_only(key_marker: Option<String>, version_id_marker: Option<String>) -> Self {
        Self {
            key_marker,
            version_id_marker,
            list_objects_v2_continuation_token: None,
            list_versions_done: false,
            list_objects_v2_done: true,
        }
    }
}

pub(in crate::s3) enum ListVersionsMarkerMode {
    /// `list_stat` has a single continuation token, so both S3 markers are packed
    /// into a tagged token.
    ListStatCombined,
    /// `enumerate_versions` already has separate fields for the key marker and
    /// version-id marker. Treat the continuation handle as a raw key marker.
    EnumerateSplit { version_id_token: Option<String> },
}

pub(in crate::s3) struct ListObjectVersionsPageResponse<T> {
    pub response: T,
    pub is_empty: bool,
}

/// Encode the markers `ListObjectVersions` and `ListObjectsV2` need to resume
/// pagination for versioned `list_stat`.
///
/// The tagged prefix makes this token self-identifying. Untagged tokens remain
/// legacy raw key markers, which avoids the ambiguity where a real key like
/// `3|abcdef` used to look like the old length-prefixed encoding.
pub(in crate::s3) fn encode_list_versions_page_markers(markers: ListObjectVersionsPageMarkers) -> Option<String> {
    if markers.is_empty() || markers.is_complete() {
        return None;
    }

    let json =
        serde_json::to_vec(&markers).expect("serializing ListObjectVersionsPageMarkers with string fields cannot fail");
    Some(format!(
        "{LIST_OBJECT_VERSIONS_PAGE_MARKERS_PREFIX}{}",
        URL_SAFE_NO_PAD.encode(json)
    ))
}

/// Decode a list-stat continuation token. Tagged tokens carry both S3 markers;
/// untagged tokens are treated as legacy raw key markers.
pub(in crate::s3) fn decode_list_versions_page_markers(handle: &str) -> error::Result<ListObjectVersionsPageMarkers> {
    let Some(encoded) = handle.strip_prefix(LIST_OBJECT_VERSIONS_PAGE_MARKERS_PREFIX) else {
        return Ok(ListObjectVersionsPageMarkers {
            key_marker: Some(handle.to_string()),
            version_id_marker: None,
            list_objects_v2_continuation_token: None,
            list_versions_done: false,
            list_objects_v2_done: true,
        });
    };

    let bytes = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|err| error::Error::invalid_argument(format!("Invalid S3 list versions page marker token: {err}")))?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(error::Error::from)?;
    let is_legacy_tagged = value
        .as_object()
        .map(|object| !object.contains_key("t") && !object.contains_key("ld") && !object.contains_key("od"))
        .unwrap_or(false);
    let mut markers: ListObjectVersionsPageMarkers = serde_json::from_value(value).map_err(error::Error::from)?;
    if is_legacy_tagged {
        markers.list_objects_v2_done = true;
    }
    Ok(markers)
}

/// Outcome of a post-error source-etag recheck. See [`Storage::source_etag_recheck`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EtagRecheck {
    SourceMatches,
    SourceStale,
    Ambiguous,
}

impl Storage {
    /// Resolves the operator-specified read-only state for the endpoint serving
    /// `resource_address`: the per-endpoint `READ_ONLY` secrets entry wins, then
    /// the backend-wide `s3.read_only` config, then the default (writable). No
    /// backend call; only the in-memory secrets map is read.
    ///
    /// Errors with `not_found` when strict mode (`allow_unconfigured_endpoints =
    /// false`) is on and nothing configures the endpoint, since the backend would
    /// refuse to serve that address anyway.
    pub(super) async fn read_only_for(&self, resource_address: &Address) -> Result<bool> {
        let (matched_secret, endpoint_override) = {
            let secrets_guard = self.secrets.read().await;
            match secrets_guard.as_ref().and_then(|secrets| {
                secrets
                    .get(resource_address.config_key(AddressingStyle::Virtual))
                    .or_else(|| secrets.get(resource_address.config_key(AddressingStyle::Path)))
            }) {
                Some(creds) => (true, creds.read_only),
                None => (false, None),
            }
        };
        // In strict mode an endpoint absent from the secrets map is one this backend
        // refuses to serve, so reporting a read-only state for it would answer for an
        // address every real operation rejects with `endpoint not configured`. Reject
        // it here instead, matching `get_optimistic_locking_support` -- which gets the
        // same behaviour from `self.client(..)` -- without making a backend call. An
        // endpoint with an explicit setting is answered normally: it is configured.
        if !matched_secret && !self.config.allow_unconfigured_endpoints && self.config.read_only.is_none() {
            return Err(error::Error::not_found(format!(
                "endpoint not configured: {}",
                resource_address.host().unwrap_or("unknown")
            ))
            .into());
        }

        if let Some(read_only) = endpoint_override.or(self.config.read_only) {
            return Ok(read_only);
        }
        // No per-endpoint or backend-wide setting. Only an endpoint absent from the secrets
        // map is driven by env vars (that is the only case `Storage::create_client` consults
        // `ClientCredentials::from_env`), so honor the `READ_ONLY` env var only then — a
        // secrets-configured endpoint that merely omits `READ_ONLY` must not inherit the
        // process-wide env value. `from_env` reads every field with `.ok()`, so it yields
        // `read_only: None` (and we fall through to writable) when `READ_ONLY` is unset.
        if !matched_secret && self.config.allow_unconfigured_endpoints {
            if let Ok(env_creds) = ClientCredentials::from_env() {
                if let Some(read_only) = env_creds.read_only {
                    return Ok(read_only);
                }
            }
        }
        Ok(false)
    }

    // perform HeadObject operation on the resource address
    #[instrument(name = "s3::stat_sdk", skip_all)]
    pub async fn stat_sdk(
        &self,
        client: &Arc<Client>,
        resource_address: Address,
        additional_headers: Option<AdditionalHeaders>,
    ) -> Result<ResourceInfo> {
        // Can't stat a folder
        if resource_address.path().ends_with('/') {
            return Err(
                error::Error::invalid_argument(format!("Cannot stat a folder {}", resource_address.as_ref())).into(),
            );
        }

        let bucket = &client.bucket;
        let key = resource_address.key()?.into_owned();

        let mut builder = client.head_object().bucket(bucket).key(key);

        // Add version id if it's present
        if let Some(version_id) = resource_address.version_id() {
            builder = builder.version_id(version_id);
        }

        let builder = Self::apply_request_context(
            builder.customize(),
            additional_headers,
            RequestType::Api,
            client.proxy_interceptor.clone(),
            None,
            client.rate_limit_interceptors.head_object.clone(),
        );

        let head_object = builder
            .send()
            .instrument(info_span!("HeadObject"))
            .await
            // TODO clean up now the errors are isolated to S3 module
            .map_err(error::SdkError::from)
            .context(error::Sdk {})?;
        trace_limited!(?head_object);

        let identity: Identity = (resource_address, head_object.e_tag, head_object.version_id).into();
        let data_object_size = head_object.content_length.map(|len| len as u64);
        let last_modified_timestamp =
            head_object
                .last_modified
                .map(|ts| storage_core_protos::google::protobuf::Timestamp {
                    seconds: ts.secs(),
                    nanos: ts.subsec_nanos() as i32,
                });

        // HeadObject already returned any user metadata (x-amz-meta-*) — surface it with no extra call,
        // but only when the operator has opted in; otherwise pre-existing object metadata that was never
        // exposed through this API stays hidden (native_metadata_enabled gate).
        let mut resource_info = WrappedResourceInfo::from((&identity, data_object_size, last_modified_timestamp)).0;
        if self.config.native_metadata_enabled.unwrap_or(false) {
            resource_info.native_metadata = head_object
                .metadata
                .map(storage_core_util::api_types::native_metadata_from_strings)
                .unwrap_or_default();
        }
        Ok(resource_info)
    }

    // perform read via GetObject operation or presigned url on the resource address
    // depending on the download preference
    #[allow(clippy::too_many_arguments)]
    #[instrument(name = "s3::read_sdk", skip_all)]
    pub async fn read_sdk<T, F>(
        &self,
        client: &Arc<Client>,
        resource_info: ResourceInfo,
        // Pre-parsed identity from the caller, when available. The read-by-identity path
        // already parses to look up the client; passing it through here avoids a second
        // parse. Read-from-address callers that only have an `Address` should pass `None`,
        // and we'll parse from `resource_info.encoded_identity`.
        parsed_identity: Option<Identity>,
        download_preference: Option<i32>,
        additional_headers: Option<AdditionalHeaders>,
        response_function: F,
    ) -> Result<crate::traits::BoxStream<T>>
    where
        F: Fn(ReadPart) -> T + std::marker::Send + 'static,
        T: std::marker::Send + 'static,
    {
        // if the download preference is not specified, and the object is small, return the data directly
        // as it might be cached locally in the small object cache. if the user specifies redirect, always
        // honor the preference, though this may result in objects in the small object cache being ignored.
        // data object size is not required to be present in the resource info, so we need to check for that.
        // if the data object size is not present, we will always redirect.
        let download_preference = match download_preference.unwrap_or_default().into() {
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
        debug_limited!(?download_preference, ?resource_info, cloudfront = ?client.cloudfront_interceptor);

        let mut resource_identity = match parsed_identity {
            Some(identity) => identity,
            None => Identity::try_from(&resource_info)?,
        };
        // Ensure the addressing style is set for the resource identity, which it might not have been
        // if it came from the stat cache.
        resource_identity.resource_address.addressing_style = client.addressing_style;
        resource_identity.resource_address.max_key_bytes = self.config.max_key_bytes;
        let key = resource_identity.key()?;
        let etag = resource_identity.etag()?;

        let builder = || {
            let bucket = &client.bucket;
            let mut builder = client.get_object().bucket(bucket).key(key).if_match(etag);

            // supply version id if it is in the resource id
            if let Some(version_id) = &resource_identity.version_id {
                builder = builder.version_id(version_id);
            }

            builder
        };

        // perform the read operation by download preference
        match download_preference {
            DownloadPreference::Body => {
                let output = Self::apply_request_context(
                    builder().customize(),
                    additional_headers,
                    RequestType::Data,
                    client.proxy_interceptor.clone(),
                    None,
                    client.rate_limit_interceptors.get_object.clone(),
                )
                .send()
                .instrument(info_span!("GetObject"))
                .await
                .map_err(error::SdkError::from)
                .context(error::Sdk {})?;
                trace_limited!(?output);

                // ETag is the primary staleness guard — mismatch means the caller is
                // asking for an object version that no longer matches the identity.
                //
                // Content-length and last-modified are no longer carried on Identity
                // (it is etag-only now), so there's nothing to validate against here.
                // The body size is authoritative via the HTTP response; consumers that
                // want a pre-read size expectation get it via ResourceInfo.data_object_size,
                // which is populated by the stat-before-read step (Phase 4b).
                let etag = output
                    .e_tag
                    .ok_or_else(|| error::Error::internal("ETag is missing from the response"))?;
                if &etag != resource_identity.etag()? {
                    return Err(error::Error::invalid_argument(format!(
                        "ETag mismatch {} != {}",
                        etag,
                        resource_identity.etag()?
                    ))
                    .into());
                }

                let body = output.body;
                Ok(Box::pin(async_stream::stream! {
                    yield Ok(response_function(ReadPart::ResourceInfo(resource_info)));
                    let mut body = body;
                    loop {
                        match body.try_next().await {
                            Ok(Some(chunk)) => yield Ok(response_function(ReadPart::Chunk(chunk))),
                            Ok(None) => break,
                            Err(error) => {
                                // Don't log here — the error propagates to the API
                                // boundary where log_grpc_error/log_http_error will
                                // log it once with correct severity (SRD 3.2: log once).
                                yield Err(error::Error::from(error).into());
                                break;
                            },
                        }
                    }
                }))
            },
            // treat anything not Body as Redirect
            _ => {
                let expiration = self
                    .expiration_normalizer
                    .get_expiration()
                    .map_err(error::Error::from)?;
                let presign_builder = PresigningConfig::builder()
                    .start_time(expiration.start_time)
                    .expires_in(Duration::from_secs(expiration.expires_in));

                let builder = Self::apply_request_context(
                    builder().customize(),
                    None,
                    RequestType::Data,
                    None,
                    client.cloudfront_interceptor.clone(),
                    client.rate_limit_interceptors.get_object.clone(),
                );

                let presigned_read = builder
                    .presigned(presign_builder.build().context(error::AwsPresigningConfig {
                        context: resource_identity.to_string(),
                    })?)
                    .instrument(info_span!("presigned"))
                    .await
                    .map_err(error::SdkError::from)
                    .context(error::Sdk {})?;
                trace_limited!(?presigned_read);

                let mut headers: Vec<_> = presigned_read
                    .headers()
                    .map(|(name, value)| Header {
                        name: name.into(),
                        value: value.into(),
                    })
                    .collect();

                if let Some(additional_headers) = additional_headers {
                    headers.extend(
                        additional_headers
                            .request_headers(RequestType::Data)
                            .map(|(k, v)| Header { name: k, value: v }),
                    );
                }

                #[allow(deprecated)]
                let redirect = Redirect {
                    redirect_target_url: presigned_read.uri().to_owned(),
                    method: presigned_read.method().to_owned(),
                    additional_headers: headers,
                };

                Ok(Box::pin(async_stream::stream! {
                    yield Ok(response_function(ReadPart::ResourceInfo(resource_info)));
                    yield Ok(response_function(ReadPart::Redirect(redirect)));
                }))
            },
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn write_sdk<F, T>(
        &self,
        mut destination_resource_address: Address,
        previous_version: Option<String>,
        fail_if_exists: bool,
        data_object_size: u64,
        upload_preference: Option<i32>,
        data_future: BoxFuture<Bytes>,
        additional_headers: Option<AdditionalHeaders>,
        native_metadata: std::collections::HashMap<String, storage_core_protos::google::protobuf::Value>,
        response_function: F,
    ) -> Result<crate::traits::BoxStream<T>>
    where
        F: Fn(WritePart) -> T + std::marker::Send + 'static,
        T: std::marker::Send + 'static,
    {
        if destination_resource_address.is_versioned() {
            return Err(error::Error::invalid_argument(format!(
                "Versioned resource addresses are not supported for writing: {}",
                destination_resource_address
            ))
            .into());
        }
        if fail_if_exists && previous_version.is_some() {
            return Err(
                error::Error::invalid_argument("fail_if_exists and previous_version are mutually exclusive").into(),
            );
        }

        // Native metadata writes are not supported while GetNativeMetadataSupport reports false. Validate
        // the map first so a malformed one still yields INVALID_ARGUMENT (the address is already parsed),
        // then reject any non-empty map with UNIMPLEMENTED regardless of native_metadata_enabled or upload
        // preference. This keeps the write path coherent with the advertised capability; read-side surfacing
        // of native metadata stays behind native_metadata_enabled.
        let backend_metadata: Option<std::collections::HashMap<String, String>> = if native_metadata.is_empty() {
            None
        } else {
            s3_metadata_from_native(&native_metadata)?;
            return Err(crate::traits::error::UnimplementedError::with_message(
                "native metadata writes are not supported by this backend (GetNativeMetadataSupport reports false)",
            )
            .into());
        };

        // determine upload preference, user's preference may be not honored due to other constraints...
        let upload_preference = match upload_preference {
            Some(pref) => match pref.into() {
                UploadPreference::Body | UploadPreference::Unspecified => {
                    // Don't let direct uploads over the maximum configured size
                    if data_object_size <= self.config.write_body_size.as_u64() {
                        UploadPreference::Body
                    } else {
                        UploadPreference::Multipart
                    }
                },
                UploadPreference::Redirect => UploadPreference::Redirect,
                UploadPreference::Multipart => UploadPreference::Multipart,
            },
            None => match data_object_size {
                size if size < self.config.write_body_size.as_u64() => UploadPreference::Body,
                size if size >= self.config.write_body_size.as_u64()
                    && size < self.config.write_redirect_size.as_u64() =>
                {
                    UploadPreference::Redirect
                },
                _ => UploadPreference::Multipart,
            },
        };
        trace_limited!(?upload_preference);

        let client = self.client(&mut destination_resource_address).await?;
        let bucket = client.bucket.to_string();
        let key = destination_resource_address.key()?.to_string();

        match upload_preference {
            UploadPreference::Body => {
                // parse out the if_match for condition write
                let if_match = Self::if_match(previous_version.as_deref())?;

                // client will stream us write chunks
                // buffer all the data before calling sdk.
                // this way, the operation is retryable, if it the data was streamed
                // there would be no way to replay the stream. if the user wants
                // the maximum performance out of write, multipart write should be used
                Ok(Box::pin(async_stream::stream! {
                    // Native metadata rides on the same PutObject request — no extra call. It was
                    // validated and converted above; the proto map is echoed on the response.
                    let native_metadata = native_metadata;
                    let backend_metadata = backend_metadata;
                    // tell client to send us data
                    yield Ok(response_function(WritePart::WriteChunksAccepted));

                    // Get the data from the caller
                    let buffer = match data_future.await {
                        Ok(b) => b,
                        Err(e) => {
                            yield Err(e);
                            return;
                        },
                    };

                    let mut builder = client
                        .put_object()
                        .bucket(&bucket)
                        .key(key)
                        .body(ByteStream::from(buffer));

                    // S3 just added conditional writes in November 2024!
                    // https://aws.amazon.com/about-aws/whats-new/2024/11/amazon-s3-functionality-conditional-writes/
                    if let Some(if_match) = if_match {
                        builder = builder.if_match(if_match);
                    }
                    if fail_if_exists {
                        builder = builder.if_none_match("*");
                    }
                    if let Some(md) = backend_metadata {
                        builder = builder.set_metadata(Some(md));
                    }

                    let builder = Self::apply_request_context(
                        builder.customize(),
                        additional_headers,
                        RequestType::Data,
                        client.proxy_interceptor.clone(),
                        None,
                        client.rate_limit_interceptors.put_object.clone(),
                    );

                    match builder
                        .send()
                        .instrument(info_span!("pub_object"))
                        .await
                        .map_err(error::SdkError::from)
                        .context(error::Sdk {})
                    {
                        Ok(result) => {
                            debug_limited!(?result);

                            let identity: Identity =
                                (destination_resource_address, result.e_tag, result.version_id).into();
                            trace_limited!(?identity);

                            // Echo the accepted native metadata on the write response (no extra backend
                            // call); the PutObject response itself does not return it.
                            let mut written = WrappedResourceInfo::from((&identity, Some(data_object_size), None)).0;
                            written.native_metadata = native_metadata;
                            yield Ok(response_function(WritePart::ResourceInfo(written)));
                        },
                        Err(err) => {
                            let err = if fail_if_exists && err.is_precondition_failed() {
                                error::Error::conflict("Resource already exists")
                            } else {
                                err
                            };
                            error_limited!(?err);
                            yield Err(err.into());
                        },
                    }
                }))
            },
            UploadPreference::Redirect => {
                let mut builder = client.put_object().bucket(bucket).key(key);

                // S3 just added conditional writes in November 2024!
                // https://aws.amazon.com/about-aws/whats-new/2024/11/amazon-s3-functionality-conditional-writes/
                if let Some(if_match) = Self::if_match(previous_version.as_deref())? {
                    builder = builder.if_match(if_match);
                }
                if fail_if_exists {
                    builder = builder.if_none_match("*");
                }

                let expiration = self
                    .expiration_normalizer
                    .get_expiration()
                    .map_err(error::Error::from)?;
                let presign_builder = PresigningConfig::builder()
                    .start_time(expiration.start_time)
                    .expires_in(Duration::from_secs(expiration.expires_in));

                let presigned_request = builder
                    .presigned(presign_builder.build().context(error::AwsPresigningConfig {
                        context: destination_resource_address.to_string(),
                    })?)
                    .instrument(info_span!("presigned"))
                    .await
                    .map_err(error::SdkError::from)
                    .context(error::Sdk {})?;
                trace_limited!(?presigned_request);

                let method = match presigned_request.method() {
                    "POST" => UploadMethod::Post,
                    "PUT" => UploadMethod::Put,
                    unknown => {
                        return Err(error::Error::internal(format!("invalid upload method returned: {unknown}")).into())
                    },
                };

                let mut request_headers: Vec<Header> = presigned_request
                    .headers()
                    .map(|(name, value)| Header {
                        name: name.into(),
                        value: value.into(),
                    })
                    .collect();

                // Add the additional headers to the redirect
                if let Some(additional_headers) = additional_headers {
                    additional_headers
                        .request_headers(RequestType::Data)
                        .for_each(|(name, value)| {
                            request_headers.push(Header { name, value });
                        });
                }

                let response = WriteRedirect {
                    redirect_target_url: presigned_request.uri().to_owned(),
                    method,
                    additional_headers: request_headers,
                    // Identity needs etag + version-id; ask the client to relay
                    // these response headers back on upload completion.
                    completion_header_names: vec![ETAG.into(), VERSION_ID_HEADER.into()],
                };
                debug_limited!(?response);

                Ok(Box::pin(async_stream::stream! {
                    yield Ok(response_function(WritePart::Redirect(response)));
                }))
            },
            UploadPreference::Multipart => {
                let result = Self::apply_request_context(
                    client.create_multipart_upload().bucket(bucket).key(key).customize(),
                    additional_headers.clone(),
                    RequestType::Api,
                    client.proxy_interceptor.clone(),
                    None,
                    client.rate_limit_interceptors.create_multipart_upload.clone(),
                )
                .send()
                .await
                .map_err(error::SdkError::from)
                .context(error::Sdk {})?;
                trace_limited!(?result);

                let upload_id = result.upload_id.ok_or(error::Error::internal("Upload id is missing"))?;

                let first_part_write_redirect = self
                    .create_upload_part(&upload_id, 0, &mut destination_resource_address, additional_headers)
                    .await?;

                // s3 multipart upload limits
                // https://docs.aws.amazon.com/AmazonS3/latest/userguide/qfacts.html
                let response = MultipartUpload {
                    upload_id: upload_id::encode(&upload_id, previous_version.as_deref(), fail_if_exists),
                    first_part_write_redirect,
                    minimum_size_per_part: Some(ByteSize::mib(5).as_u64()),
                    maximum_size_per_part: Some(ByteSize::gib(5).as_u64()),
                    maximum_parts_number: Some(10000),
                };

                debug_limited!(?response);

                Ok(Box::pin(async_stream::stream! {
                    yield Ok(response_function(WritePart::MultipartUpload(response)));
                }))
            },
            UploadPreference::Unspecified => {
                // should never reach here because we check that above and pick the action base on data size
                Err(error::Error::internal("Could not determine upload preference").into())
            },
        }
    }

    pub async fn list_sdk<T, F>(
        &self,
        client: &Arc<Client>,
        delimiter: Option<&str>,
        resource_address: Address,
        streaming: StreamingOptions,
        additional_headers: Option<AdditionalHeaders>,
        response_function: F,
    ) -> Result<crate::traits::BoxStream<T>>
    where
        T: std::marker::Send + 'static,
        F: Fn(
                Box<dyn Iterator<Item = &Object> + '_>,
                Box<dyn Iterator<Item = &CommonPrefix> + '_>,
                Option<String>,
                &Address,
            ) -> Result<T>
            + std::marker::Send
            + 'static,
    {
        if resource_address.is_versioned() {
            return Err(error::Error::invalid_argument(format!(
                "Versioned resource addresses are not supported for listing: {}",
                resource_address
            ))
            .into());
        }

        let bucket = &client.bucket;
        let prefix = resource_address.key()?.to_string();
        trace_limited!(bucket, %prefix, ?resource_address);

        // If the request is for the top level of a bucket, we should return an empty response
        // instead of a NOT_FOUND error
        let is_top_level = prefix.is_empty();

        // prefix search
        let mut builder = client.list_objects_v2().bucket(bucket).prefix(&prefix);

        if let Some(delimiter) = delimiter {
            builder = builder.delimiter(delimiter);
        }

        let stream = match streaming {
            StreamingOptions::Paginated(pagination) => {
                if let Some(max_keys) = pagination.max_page_size.filter(|&x| x != 0 && x <= i32::MAX as u32) {
                    builder = builder.max_keys(max_keys as i32);
                }
                if let Some(continue_token) = pagination.continuation_handle {
                    builder = builder.continuation_token(continue_token);
                }
                false
            },
            StreamingOptions::Streamed => {
                builder = builder.max_keys(1000);
                true
            },
        };

        let client = client.clone();

        Ok(Box::pin(async_stream::stream! {
            let mut builder = builder;
            loop {
                let result = Self::apply_request_context(
                    builder.clone().customize(),
                    additional_headers.clone(),
                    RequestType::Api,
                    client.proxy_interceptor.clone(),
                    None,
                    client.rate_limit_interceptors.list_objects_v2.clone(),
                )
                .send()
                .await;
                trace_limited!(?result);

                match result {
                    Ok(output) => {
                        debug_limited!(
                            entries = output.contents().len(),
                            prefixes = output.common_prefixes().len(),
                            stream
                        );

                        // this logic is from the storage api + conformance tests...
                        // if no results are returned, return a NOT_FOUND error instead of an empty response
                        // unless its the top level of the bucket, in which case return an empty response...
                        //
                        // there is a weird edge case where the result is paged, but the last page does not have
                        // any entries. in that case, we do not want to return a NOT_FOUND error, since there were
                        // entries on the previous pages. so we check for the continuation token to be None
                        if !is_top_level
                            && output.contents().is_empty()
                            && output.common_prefixes().is_empty()
                            && builder.get_continuation_token().is_none()
                        {
                            yield Err(error::SdkError::not_found("No elements found"))
                                .context(error::Sdk {})
                                .map_err(|e| e.into());
                        }

                        // update continuation token, response function will consume output, but
                        let continuation_token = output.next_continuation_token().map(String::from);
                        debug_limited!(?continuation_token, stream);

                        // Filter out the folder object from the response
                        let response = response_function(
                            Box::new(output.contents().iter().filter(|object| {
                                if let Some(key) = object.key() {
                                    key != prefix
                                } else {
                                    false
                                }
                            })),
                            Box::new(output.common_prefixes().iter()),
                            continuation_token.clone(),
                            &resource_address,
                        );
                        yield response;

                        // break for REST requests which don't support streaming
                        if !stream {
                            break;
                        }

                        // if streaming, update continuation token
                        if let Some(token) = &continuation_token {
                            builder = builder.continuation_token(token);
                        } else {
                            break;
                        }
                    },
                    Err(error) => {
                        error_limited!(?error);
                        yield Err(error::SdkError::from(error)).context(error::Sdk {}).map_err(|err| {
                            Box::new(err) as Box<dyn storage_core_builder::traits::StorageErrorExt + 'static>
                        });
                        break;
                    },
                }
            }
        }))
    }

    /// List object versions using the `ListObjectVersions` S3 API.
    ///
    /// Used by both `list_stat` (with delimiter, is_latest filtering in callback)
    /// and `enumerate_versions` (without delimiter, all versions in callback).
    /// The callback receives `ListObjectVersionsOutput` and filters as needed.
    ///
    /// When `fallback_to_list_objects_v2` is true and the backend does not support
    /// `ListObjectVersions` (e.g. GCS S3-compatible API), transparently falls back
    /// to `ListObjectsV2` and converts the results to `ListObjectVersionsOutput`.
    /// Objects from the fallback have `version_id: None` and `is_latest: true`.
    ///
    /// The `Client` struct caches whether the backend supports `ListObjectVersions`
    /// (see `Client::list_versions_support`) so that after two confirming
    /// observations we skip the wasted round-trip on subsequent calls. For known-GCS
    /// endpoints the cache is pre-seeded at client construction, so even the first
    /// call goes straight to the fallback path.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::s3) async fn list_versions_sdk<T, F>(
        &self,
        client: &Arc<Client>,
        delimiter: Option<&str>,
        resource_address: Address,
        streaming: StreamingOptions,
        marker_mode: ListVersionsMarkerMode,
        fallback_to_list_objects_v2: bool,
        additional_headers: Option<AdditionalHeaders>,
        response_function: F,
    ) -> Result<crate::traits::BoxStream<T>>
    where
        T: std::marker::Send + 'static,
        F: Fn(
                ListObjectVersionsOutput,
                &Address,
                ListObjectVersionsPageMarkers,
            ) -> Result<ListObjectVersionsPageResponse<T>>
            + std::marker::Send
            + std::marker::Sync
            + 'static,
    {
        if resource_address.is_versioned() {
            return Err(error::Error::invalid_argument(format!(
                "Versioned resource addresses are not supported for listing versions: {}",
                resource_address
            ))
            .into());
        }

        let bucket = &client.bucket;
        let prefix = resource_address.key()?;
        let prefix_string = prefix.to_string();
        trace_limited!(bucket, %prefix);

        // If the request is for the top level of a bucket, we should return an empty response
        // instead of a NOT_FOUND error
        let is_top_level = prefix.is_empty();

        let mut builder = client.list_object_versions().bucket(bucket).prefix(prefix);

        let delimiter_owned = delimiter.map(String::from);
        if let Some(delimiter) = delimiter {
            builder = builder.delimiter(delimiter);
        }

        // Capture pagination settings for the ListObjectVersions request and
        // potential ListObjectsV2 fallback.
        let list_stat_combined = matches!(&marker_mode, ListVersionsMarkerMode::ListStatCombined);

        let (stream, max_keys, initial_markers, has_initial_markers) = match (streaming, marker_mode) {
            (StreamingOptions::Paginated(pagination), ListVersionsMarkerMode::ListStatCombined) => {
                let mk = pagination
                    .max_page_size
                    .filter(|&x| x != 0 && x <= i32::MAX as u32)
                    .map(|x| x as i32);
                if let Some(max_keys) = mk {
                    builder = builder.max_keys(max_keys);
                }
                let markers = pagination
                    .continuation_handle
                    .as_deref()
                    .map(decode_list_versions_page_markers)
                    .transpose()?
                    .unwrap_or_default();
                let has_initial_markers = !markers.is_empty();
                if !markers.list_versions_done {
                    if let Some(ref key) = markers.key_marker {
                        builder = builder.key_marker(key);
                    }
                    if let Some(ref vid) = markers.version_id_marker {
                        builder = builder.version_id_marker(vid);
                    }
                }

                (false, mk, markers, has_initial_markers)
            },
            (StreamingOptions::Paginated(pagination), ListVersionsMarkerMode::EnumerateSplit { version_id_token }) => {
                let mk = pagination
                    .max_page_size
                    .filter(|&x| x != 0 && x <= i32::MAX as u32)
                    .map(|x| x as i32);
                if let Some(max_keys) = mk {
                    builder = builder.max_keys(max_keys);
                }
                let key_marker = pagination.continuation_handle;
                let has_initial_markers = key_marker.is_some() || version_id_token.is_some();
                if let Some(ref key) = key_marker {
                    builder = builder.key_marker(key);
                }
                if let Some(ref vid) = version_id_token {
                    builder = builder.version_id_marker(vid);
                }
                (
                    false,
                    mk,
                    ListObjectVersionsPageMarkers::for_list_object_versions_only(key_marker, version_id_token),
                    has_initial_markers,
                )
            },
            (StreamingOptions::Streamed, ListVersionsMarkerMode::ListStatCombined) => {
                builder = builder.max_keys(1000);
                (true, Some(1000), ListObjectVersionsPageMarkers::default(), false)
            },
            (StreamingOptions::Streamed, ListVersionsMarkerMode::EnumerateSplit { version_id_token }) => {
                builder = builder.max_keys(1000);
                let has_initial_markers = version_id_token.is_some();
                if let Some(ref vid) = version_id_token {
                    builder = builder.version_id_marker(vid);
                }
                (
                    true,
                    Some(1000),
                    ListObjectVersionsPageMarkers::for_list_object_versions_only(None, version_id_token),
                    has_initial_markers,
                )
            },
        };

        let client = client.clone();

        Ok(Box::pin(async_stream::stream! {
            let mut builder = builder;
            let mut entry_count = 0;

                // Decide whether to attempt `ListObjectVersions` at all.
                //
                // See `Client::list_versions_support` for the full rationale.
                // In short: we maintain a per-client cache of whether this backend's
                // S3-compatible API actually supports `ListObjectVersions`. GCS does
                // not; real S3 and MinIO do. The cache lets us skip a wasted
                // round-trip on known-broken backends.
                //
                //   * `ConfirmedUnsupported` AND caller allows fallback → skip the
                //     call, jump straight to the `ListObjectsV2` emulation path.
                //     This is the steady-state hot path on GCS (and on any other
                //     backend we've confirmed to be broken via two observations).
                //
                //   * Any other state → attempt the call. A single unsupported
                //     observation is NOT enough to latch; we need a second
                //     confirmation to rule out a transient WAF/proxy flake that
                //     could otherwise pin a real-S3 client to `ListObjectsV2` for
                //     its entire lifetime. Transient transport/service errors do
                //     NOT update the state — a network blip says nothing about API
                //     support.
                let skip_to_fallback = fallback_to_list_objects_v2
                    && crate::s3::service::list_versions_support::should_skip(&client.list_versions_support);
                let list_versions_already_done = list_stat_combined && initial_markers.list_versions_done;

                // `first_raw` is `None` when we skipped the attempt (cache hit),
                // `Some(Ok|Err)` otherwise. Holding the raw `SdkError` (pre-conversion
                // to storage-core's error type) is important: we need structural
                // access to the SDK variants to detect the GCS shape without
                // stringifying the error.
                let mut latch_transition: Option<
                    crate::s3::service::list_versions_support::LatchTransition,
                > = None;
                let first_raw = if skip_to_fallback || list_versions_already_done {
                    None
                } else {
                    let raw = Self::apply_request_context(
                        builder.clone().customize(),
                        additional_headers.clone(),
                        RequestType::Api,
                        client.proxy_interceptor.clone(),
                        None,
                        client.rate_limit_interceptors.list_object_versions.clone(),
                    )
                    .send()
                    .instrument(info_span!("list_versions"))
                    .await;

                    // Update the state BEFORE branching on fallback so that a
                    // concurrent `enumerate_versions` call (which passes
                    // `fallback_to_list_objects_v2 = false` and therefore doesn't
                    // fall back) still records the negative observation for
                    // subsequent `list_stat` callers on the same client.
                    //
                    // State transitions are monotonic per-call; concurrent
                    // observers converge.
                    match &raw {
                        Ok(_) => {
                            crate::s3::service::list_versions_support::record_success(&client.list_versions_support);
                        },
                        Err(err)
                            if matches!(
                                classify_list_object_versions_fallback(err),
                                ListObjectVersionsFallback::CacheableDenied
                                    | ListObjectVersionsFallback::CacheableUnsupported
                            ) =>
                        {
                            latch_transition = Some(
                                crate::s3::service::list_versions_support::record_unsupported(
                                    &client.list_versions_support,
                                ),
                            );
                        },
                        Err(_) => {
                            // Transient/transport/other service error. Leave the
                            // state untouched so we'll retry `ListObjectVersions`
                            // on the next call.
                        },
                    }

                    Some(raw)
                };

                // After the (possible) attempt, decide whether to take the
                // fallback branch. Either we skipped the call because the cache
                // told us to, or we just observed the structural error.
                let use_fallback = match (skip_to_fallback, &first_raw) {
                    (true, _) => true,
                    (_, Some(Err(err)))
                        if fallback_to_list_objects_v2
                            && classify_list_object_versions_fallback(err) != ListObjectVersionsFallback::NoFallback =>
                    {
                        true
                    },
                    _ => false,
                };

                if use_fallback {
                    // Log on each live observation of the unsupported shape (up
                    // to two per client before the latch confirms, since we
                    // require two observations to converge on
                    // `ConfirmedUnsupported`). Skipping straight to fallback
                    // because the client is already latched or pre-seeded is
                    // silent — we don't spam once per `list_stat`.
                    if !skip_to_fallback {
                        use crate::s3::service::list_versions_support::LatchTransition;

                        let denied = matches!(
                            first_raw
                                .as_ref()
                                .and_then(|raw| raw.as_ref().err())
                                .map(classify_list_object_versions_fallback),
                            Some(ListObjectVersionsFallback::CacheableDenied),
                        );
                        let action = if denied {
                            "denied by backend"
                        } else {
                            "not supported by backend"
                        };

                        match latch_transition {
                            Some(LatchTransition::FirstObservation) => {
                                warn_limited!(
                                    "ListObjectVersions {action} (1st attempt; will latch after 1 more), falling back to ListObjectsV2"
                                );
                            },
                            Some(LatchTransition::Confirmed) => {
                                warn_limited!(
                                    "ListObjectVersions {action} (2nd attempt; latched, future calls will skip ListObjectVersions), falling back to ListObjectsV2"
                                );
                            },
                            Some(LatchTransition::NoChange) | None => {
                                warn_limited!(
                                    "ListObjectVersions {action}, falling back to ListObjectsV2"
                                );
                            },
                        }
                    }

                    // Build ListObjectsV2 request with the same parameters
                    let mut v2_builder = client.list_objects_v2().bucket(&client.bucket).prefix(&prefix_string);
                    if let Some(ref d) = delimiter_owned {
                        v2_builder = v2_builder.delimiter(d);
                    }
                    if let Some(mk) = max_keys {
                        v2_builder = v2_builder.max_keys(mk);
                    }
                    let initial_v2_token_for_fallback = initial_markers
                        .list_objects_v2_continuation_token
                        .as_ref()
                        .or(initial_markers.key_marker.as_ref());
                    if let Some(token) = initial_v2_token_for_fallback {
                        v2_builder = v2_builder.continuation_token(token);
                    }

                    loop {
                        let result = Self::apply_request_context(
                            v2_builder.clone().customize(),
                            additional_headers.clone(),
                            RequestType::Api,
                            client.proxy_interceptor.clone(),
                            None,
                            client.rate_limit_interceptors.list_objects_v2.clone(),
                        )
                        .send()
                        .await;

                        match result {
                            Ok(output) => {
                                // Convert ListObjectsV2 output to ListObjectVersionsOutput
                                let versions: Vec<ObjectVersion> = output
                                    .contents()
                                    .iter()
                                    .map(|obj| {
                                        ObjectVersion::builder()
                                            .set_key(obj.key().map(String::from))
                                            .set_e_tag(obj.e_tag().map(String::from))
                                            .set_size(obj.size())
                                            .set_last_modified(obj.last_modified().cloned())
                                            .set_is_latest(Some(true))
                                            .build()
                                    })
                                    .collect();

                                let continuation_token =
                                    output.next_continuation_token().filter(|s| !s.is_empty()).map(String::from);
                                let next_markers = ListObjectVersionsPageMarkers {
                                    key_marker: None,
                                    version_id_marker: None,
                                    list_objects_v2_continuation_token: continuation_token.clone(),
                                    list_versions_done: true,
                                    list_objects_v2_done: continuation_token.is_none(),
                                };

                                let synthetic = ListObjectVersionsOutput::builder()
                                    .set_versions(Some(versions))
                                    .set_common_prefixes(Some(output.common_prefixes().to_vec()))
                                    .set_next_key_marker(continuation_token.clone())
                                    .build();

                                entry_count += synthetic.versions().len();
                                debug_limited!(
                                    entry_count,
                                    prefixes = synthetic.common_prefixes().len(),
                                    stream,
                                    fallback = true
                                );

                                if !is_top_level
                                    && synthetic.versions().is_empty()
                                    && synthetic.common_prefixes().is_empty()
                                    && v2_builder.get_continuation_token().is_none()
                                {
                                    yield Err(error::SdkError::not_found("No elements found"))
                                        .context(error::Sdk {})
                                        .map_err(|e| e.into());
                                    break;
                                }

                                let response = response_function(synthetic, &resource_address, next_markers)
                                    .map(|page| page.response);
                                yield response;

                                if !stream {
                                    break;
                                }

                                if let Some(token) = &continuation_token {
                                    v2_builder = v2_builder.continuation_token(token);
                                } else {
                                    break;
                                }
                            },
                            Err(error) => {
                                error_limited!(?error);
                                yield Err(error::SdkError::from(error)).context(error::Sdk {}).map_err(|err| {
                                    Box::new(err)
                                        as Box<dyn storage_core_builder::traits::StorageErrorExt + 'static>
                                });
                                break;
                            },
                        }
                    }
                } else {
                    // ListObjectVersions path — process first response and continue pagination.
                    //
                    // We come here in three cases:
                    //   1. The call succeeded. (first_raw = Some(Ok(_)))
                    //   2. The call failed with an error that is NOT the GCS-style
                    //      unsupported shape — e.g. a real NotFound, AccessDenied,
                    //      throttle, transport failure, or a ServiceError with a 2xx
                    //      body that IS a modeled error (not Unhandled). These all
                    //      propagate unchanged to the caller. (first_raw = Some(Err(_)))
                    //   3. The call failed with the GCS shape but the caller is
                    //      `enumerate_versions` (fallback disabled). That caller asked
                    //      for version history, and the backend can't provide it, so
                    //      propagating the error is the correct behavior.
                    let first_response = if list_versions_already_done {
                        Ok(ListObjectVersionsOutput::builder().build())
                    } else {
                        // Control flow invariant: `first_raw` must be `Some` here
                        // unless the version stream was already complete. Rather
                        // than asserting with `.expect()` (which would panic and
                        // take down the service if a future change breaks the
                        // invariant), extract via `let-else` and report an internal
                        // error through the channel. The caller sees a normal error;
                        // the service keeps running.
                        let Some(raw_result) = first_raw else {
                            yield Err(error::Error::internal(
                                "list_versions_sdk: reached ListObjectVersions path with no result",
                            )
                            .into());
                            return;
                        };
                        raw_result.map_err(error::SdkError::from).context(error::Sdk {})
                    };

                    // One step of `ListObjectVersions` pagination. `process_response` projects an
                    // SDK output into the caller's response type plus the next page's markers.
                    // Returning `None` from `process_response` signals "structurally empty page,
                    // no more pages, not a top-level listing" — i.e. the not-found case.
                    struct PaginationStep<T> {
                        response: Result<(T, bool)>,
                        next_markers: ListObjectVersionsPageMarkers,
                        has_more: bool,
                    }

                    struct ListObjectsV2PrefixPage {
                        common_prefixes: Vec<CommonPrefix>,
                        next_token: Option<String>,
                    }

                    let mut saw_non_empty_response = false;
                    let mut process_response =
                        |output: ListObjectVersionsOutput,
                         next_markers: ListObjectVersionsPageMarkers|
                         -> Option<PaginationStep<T>> {
                        entry_count += output.versions().len();
                        debug_limited!(entry_count, prefixes = output.common_prefixes().len(), stream);

                        let has_more = if list_stat_combined {
                            !next_markers.is_complete()
                        } else {
                            next_markers.key_marker.is_some() || next_markers.version_id_marker.is_some()
                        };
                        let raw_page_empty = output.versions().is_empty()
                            && output.common_prefixes().is_empty()
                            && output.delete_markers().is_empty();

                        if !is_top_level && raw_page_empty && !has_more {
                            return None; // signal NOT_FOUND
                        }

                        let response = response_function(output, &resource_address, next_markers.clone())
                            .map(|page| (page.response, page.is_empty));
                        Some(PaginationStep {
                            response,
                            next_markers,
                            has_more,
                        })
                    };

                    let clean_marker = |marker: Option<&str>| marker.filter(|s| !s.is_empty()).map(String::from);
                    let mut list_versions_done = list_versions_already_done;
                    let mut list_objects_v2_done = !list_stat_combined || initial_markers.list_objects_v2_done;
                    let mut next_list_objects_v2_token =
                        initial_markers.list_objects_v2_continuation_token.clone();

                    // Linear pagination loop: `response_result` carries the outcome of the
                    // request whose response we're about to handle (the first request was made
                    // above; subsequent ones are made at the bottom of the loop).
                    let mut response_result = first_response;
                    loop {
                        let mut output = match response_result {
                            Ok(output) => output,
                            Err(err) => {
                                error_limited!(?err);
                                yield Err(err.into());
                                break;
                            },
                        };

                        // MinIO emits empty `<NextKeyMarker></NextKeyMarker>` and
                        // `<NextVersionIdMarker></NextVersionIdMarker>` elements even when
                        // `IsTruncated=false`; the AWS SDK parses these as `Some("")`. Treat
                        // an empty marker as no marker — it cannot identify a real next page.
                        let (next_key_marker, next_version_id_marker) = if list_versions_done {
                            (None, None)
                        } else {
                            let key_marker = clean_marker(output.next_key_marker());
                            let version_id_marker = clean_marker(output.next_version_id_marker());
                            list_versions_done = key_marker.is_none() && version_id_marker.is_none();
                            (key_marker, version_id_marker)
                        };

                        let v2_common_prefixes = if list_stat_combined && !list_objects_v2_done {
                            let mut v2_builder = client.list_objects_v2().bucket(&client.bucket).prefix(&prefix_string);
                            if let Some(ref d) = delimiter_owned {
                                v2_builder = v2_builder.delimiter(d);
                            }
                            if let Some(mk) = max_keys {
                                v2_builder = v2_builder.max_keys(mk);
                            }
                            v2_builder = v2_builder.set_continuation_token(next_list_objects_v2_token.clone());

                            let result = Self::apply_request_context(
                                v2_builder.customize(),
                                additional_headers.clone(),
                                RequestType::Api,
                                client.proxy_interceptor.clone(),
                                None,
                                client.rate_limit_interceptors.list_objects_v2.clone(),
                            )
                            .send()
                            .instrument(info_span!("list_objects_v2_prefixes"))
                            .await;

                            match result {
                                Ok(output) => {
                                    let page = ListObjectsV2PrefixPage {
                                        common_prefixes: output.common_prefixes().to_vec(),
                                        next_token: output
                                            .next_continuation_token()
                                            .filter(|s| !s.is_empty())
                                            .map(String::from),
                                    };
                                    next_list_objects_v2_token = page.next_token.clone();
                                    list_objects_v2_done = next_list_objects_v2_token.is_none();
                                    page.common_prefixes
                                },
                                Err(error) => {
                                    error_limited!(?error);
                                    yield Err(error::SdkError::from(error)).context(error::Sdk {}).map_err(|err| {
                                        Box::new(err)
                                            as Box<dyn storage_core_builder::traits::StorageErrorExt + 'static>
                                    });
                                    break;
                                },
                            }
                        } else {
                            Vec::new()
                        };

                        if list_stat_combined {
                            output.common_prefixes = Some(v2_common_prefixes);
                        }

                        let next_markers = if list_stat_combined {
                            ListObjectVersionsPageMarkers {
                                key_marker: next_key_marker,
                                version_id_marker: next_version_id_marker,
                                list_objects_v2_continuation_token: next_list_objects_v2_token.clone(),
                                list_versions_done,
                                list_objects_v2_done,
                            }
                        } else {
                            ListObjectVersionsPageMarkers::for_list_object_versions_only(
                                next_key_marker,
                                next_version_id_marker,
                            )
                        };

                        // Two NOT_FOUND paths:
                        //   - `process_response` returns `None` for a structurally empty page.
                        //   - The callback's filter returned every entry on the last page.
                        // Both are emitted only when the caller didn't resume from a marker
                        // and we haven't seen data on a prior page — otherwise the right
                        // behavior is to close out without an error.
                        let Some(step) = process_response(output, next_markers) else {
                            if !has_initial_markers && !saw_non_empty_response {
                                yield Err(error::SdkError::not_found("No versions found"))
                                    .context(error::Sdk {})
                                    .map_err(|e| e.into());
                            }
                            break;
                        };

                        let (response, is_empty) = match step.response {
                            Ok(pair) => pair,
                            Err(err) => {
                                yield Err(err);
                                break;
                            },
                        };

                        if !is_empty {
                            saw_non_empty_response = true;
                        }

                        if is_empty
                            && !step.has_more
                            && !is_top_level
                            && !has_initial_markers
                            && !saw_non_empty_response
                        {
                            yield Err(error::SdkError::not_found("No versions found"))
                                .context(error::Sdk {})
                                .map_err(|e| e.into());
                            break;
                        }

                        // Forward the page UNLESS we're streaming and the page is
                        // empty-but-has-more — that's just noise the client doesn't need
                        // to see; we'll keep paginating internally.
                        if !(stream && is_empty && step.has_more) {
                            yield Ok(response);
                        }

                        // Paginated mode returns one page; stream mode iterates.
                        if !stream {
                            break;
                        }

                        if !step.has_more {
                            break;
                        }

                        if list_stat_combined && step.next_markers.list_versions_done {
                            response_result = Ok(ListObjectVersionsOutput::builder().build());
                            continue;
                        }

                        if step.next_markers.key_marker.is_none() && step.next_markers.version_id_marker.is_none() {
                            break;
                        }

                        builder = builder
                            .set_key_marker(step.next_markers.key_marker.clone())
                            .set_version_id_marker(step.next_markers.version_id_marker.clone());

                        response_result = Self::apply_request_context(
                            builder.clone().customize(),
                            additional_headers.clone(),
                            RequestType::Api,
                            client.proxy_interceptor.clone(),
                            None,
                            client.rate_limit_interceptors.list_object_versions.clone(),
                        )
                        .send()
                        .instrument(info_span!("list_versions"))
                        .await
                        .map_err(error::SdkError::from)
                        .context(error::Sdk {});
                    }
                }
        }))
    }

    pub async fn create_upload_part(
        &self,
        upload_id: impl AsRef<str> + std::fmt::Debug,
        part_number: u32,
        destination_resource_address: &mut Address,
        additional_headers: Option<AdditionalHeaders>,
    ) -> Result<WriteRedirect> {
        let client = self.client(destination_resource_address).await?;
        let bucket = &client.bucket;
        let key = destination_resource_address.key()?;

        let expiration = self
            .expiration_normalizer
            .get_expiration()
            .map_err(error::Error::from)?;
        let presign_builder = PresigningConfig::builder()
            .start_time(expiration.start_time)
            .expires_in(Duration::from_secs(expiration.expires_in));

        // https://docs.aws.amazon.com/AmazonS3/latest/userguide/mpuoverview.html
        // need to make sure we convert 0 based part number to 1 based
        let presigned_request = client
            .upload_part()
            .bucket(bucket)
            .key(key.to_string())
            .upload_id(upload_id.as_ref())
            .part_number((part_number + 1) as i32)
            .presigned(presign_builder.build().context(error::AwsPresigningConfig {
                context: destination_resource_address.to_string(),
            })?)
            .await
            .map_err(error::SdkError::from)
            .context(error::Sdk {})?;
        trace_limited!(?presigned_request);

        let method = match presigned_request.method() {
            "POST" => UploadMethod::Post,
            "PUT" => UploadMethod::Put,
            unknown => {
                return Err(error::Error::internal(format!("invalid upload method returned: {unknown}")).into());
            },
        };

        let mut request_headers: Vec<Header> = presigned_request
            .headers()
            .map(|(name, value)| Header {
                name: name.into(),
                value: value.into(),
            })
            .collect();

        // Add the additional headers to the redirect
        if let Some(additional_headers) = additional_headers {
            additional_headers
                .request_headers(RequestType::Data)
                .for_each(|(name, value)| {
                    request_headers.push(Header { name, value });
                });
        }

        let redirect = WriteRedirect {
            redirect_target_url: presigned_request.uri().to_owned(),
            method,
            additional_headers: request_headers,
            completion_header_names: vec![ETAG.to_owned()],
        };
        debug_limited!(?redirect);

        Ok(redirect)
    }

    pub async fn delete_sdk(
        &self,
        mut resource_address: Address,
        previous_version: Option<String>,
        additional_headers: Option<AdditionalHeaders>,
    ) -> std::result::Result<(), super::error::Error> {
        if resource_address.is_versioned() {
            return Err(error::Error::invalid_argument(format!(
                "Versioned resource addresses are not supported for deleting: {}",
                resource_address
            )));
        }

        let client = self.client(&mut resource_address).await?;
        let bucket = &client.bucket;
        let key = resource_address.key()?;

        let mut builder = client.delete_object().bucket(bucket).key(key);

        let if_match = Self::if_match(previous_version.as_deref())?;
        if let Some(if_match) = if_match {
            builder = builder.if_match(if_match);
        }

        let response = Self::apply_request_context(
            builder.customize(),
            additional_headers,
            RequestType::Api,
            client.proxy_interceptor.clone(),
            None,
            client.rate_limit_interceptors.delete_object.clone(),
        )
        .send()
        .instrument(info_span!("delete_object"))
        .await
        .map_err(error::SdkError::from)
        .context(error::Sdk {})?;
        debug_limited!(?response);

        Ok(())
    }

    async fn abort_multipart_upload_sdk(
        &self,
        upload_id: String,
        mut destination_resource_address: Address,
        additional_headers: Option<AdditionalHeaders>,
    ) -> Result<()> {
        let (upload_id, _) = upload_id::decode(&upload_id);

        let client = self.client(&mut destination_resource_address).await?;
        let bucket = &client.bucket;
        let key = destination_resource_address.key()?.to_string();

        let builder = client
            .abort_multipart_upload()
            .bucket(bucket)
            .key(key)
            .upload_id(upload_id);

        let result = Self::apply_request_context(
            builder.customize(),
            additional_headers,
            RequestType::Api,
            client.proxy_interceptor.clone(),
            None,
            client.rate_limit_interceptors.abort_multipart_upload.clone(),
        )
        .send()
        .instrument(info_span!("abort_multipart_upload"))
        .await
        .map_err(error::SdkError::from)
        .context(error::Sdk {})?;
        trace_limited!(?result);

        Ok(())
    }

    pub async fn complete_multipart_upload_sdk(
        &self,
        upload_id: String,
        mut destination_resource_address: Address,
        mut parts: Vec<CompletedUploadPart>,
        additional_headers: Option<AdditionalHeaders>,
    ) -> Result<ResourceInfo> {
        let decoded_upload_id = upload_id::decode_with_options(&upload_id);
        let upload_id = decoded_upload_id.upload_id;
        let previous_version = decoded_upload_id.resource_identity;

        // parse out the if_match for condition write
        let if_match = if let Some(previous_version) = previous_version {
            let Identity { etag, .. } = Identity::parse(previous_version)?;
            etag
        } else {
            None
        };

        let client = self.client(&mut destination_resource_address).await?;
        let bucket = &client.bucket;
        let key = destination_resource_address.key()?.to_string();

        // sort part numbers so that we get the block ids in the right order
        parts.sort_by_key(|a| a.part_number);

        let mut upload_parts: Vec<CompletedPart> = Vec::with_capacity(parts.len());

        // https://docs.aws.amazon.com/AmazonS3/latest/userguide/mpuoverview.html
        // need to make sure we convert 0 based part number to 1 based
        for part in parts {
            let Some(header) = part.headers.first() else {
                return Err(
                    error::Error::invalid_argument(format!("No headers found for part {}", part.part_number)).into(),
                );
            };

            if header.name.to_lowercase() != ETAG {
                return Err(error::Error::invalid_argument(format!("Invalid header name: {}", header.name)).into());
            }
            let completed_part = CompletedPart::builder()
                .e_tag(header.value.to_owned())
                .part_number((part.part_number + 1) as i32)
                .build();
            trace_limited!(?completed_part);
            upload_parts.push(completed_part)
        }

        let completed_multipart_upload = CompletedMultipartUpload::builder()
            .set_parts(Some(upload_parts))
            .build();

        let mut builder = client
            .complete_multipart_upload()
            .bucket(bucket)
            .key(key)
            .multipart_upload(completed_multipart_upload)
            .upload_id(upload_id);

        // S3 just added conditional writes in November 2024!
        // https://aws.amazon.com/about-aws/whats-new/2024/11/amazon-s3-functionality-conditional-writes/
        if let Some(if_match) = if_match {
            builder = builder.if_match(if_match);
        }
        if decoded_upload_id.fail_if_exists {
            builder = builder.if_none_match("*");
        }

        let result = Self::apply_request_context(
            builder.customize(),
            additional_headers,
            RequestType::Api,
            client.proxy_interceptor.clone(),
            None,
            client.rate_limit_interceptors.complete_multipart_upload.clone(),
        )
        .send()
        .instrument(info_span!("complete_multipart_upload"))
        .await
        .map_err(error::SdkError::from)
        .context(error::Sdk {})
        .map_err(|err| {
            if decoded_upload_id.fail_if_exists && err.is_precondition_failed() {
                error::Error::conflict("Resource already exists")
            } else {
                err
            }
        })?;
        trace_limited!(?result);

        let identity: Identity = (destination_resource_address, result.e_tag, result.version_id).into();
        trace_limited!(?identity);

        let response = WrappedResourceInfo::from(&identity).0;
        debug_limited!(?response);

        Ok(response)
    }

    pub async fn upload_part_sdk(
        &self,
        upload_id: String,
        mut destination_resource_address: Address,
        part_number: u32,
        part_count: Option<u32>,
        additional_headers: Option<AdditionalHeaders>,
    ) -> Result<Vec<WriteRedirect>> {
        let (upload_id, _) = upload_id::decode(&upload_id);

        let part_count = part_count.unwrap_or(1);
        if part_count == 0 {
            return Err(error::Error::invalid_argument("Part count must be >= 1").into());
        }

        let mut parts = Vec::with_capacity(part_count as usize);

        for part_number in part_number..part_number + part_count {
            parts.push(
                self.create_upload_part(
                    &upload_id,
                    part_number,
                    &mut destination_resource_address,
                    additional_headers.clone(),
                )
                .await?,
            );
        }

        debug_limited!(?parts);

        Ok(parts)
    }

    pub async fn complete_redirect_upload_sdk(
        &self,
        destination_resource_address: Address,
        additional_headers: Vec<Header>,
    ) -> Result<ResourceInfo> {
        let mut etag = None;
        let mut version_id = None;
        for header in &additional_headers {
            match &header.name {
                h if h.eq_ignore_ascii_case(ETAG) => etag = Some(header.value.clone()),
                h if h.eq_ignore_ascii_case(VERSION_ID_HEADER) => version_id = Some(header.value.clone()),
                _ => continue,
            }
        }

        if etag.is_none() {
            return Err(error::Error::invalid_argument(format!(
                "ETag header not returned. Provided headers: {:?}",
                additional_headers
            ))
            .into());
        }

        let identity = Identity {
            resource_address: destination_resource_address,
            etag,
            version_id,
        };

        let response = WrappedResourceInfo::from(&identity).0;
        debug_limited!(?response);

        Ok(response)
    }

    /// Post-error source-etag recheck used to disambiguate a copy 412 between
    /// source staleness and destination conflict. Returns:
    ///   - `SourceMatches` — source still has the caller's expected etag (or
    ///     no etag guard was armed); the 412 must be the destination conflict.
    ///   - `SourceStale` — source returned 412 against `if_match(expected)`;
    ///     the original copy 412 was the source staleness check failing.
    ///   - `Ambiguous` — the recheck failed for some other reason (transient,
    ///     permissions, etc.). The caller should surface the original 412
    ///     rather than guess.
    async fn source_etag_recheck(
        client: &Arc<Client>,
        bucket: &str,
        key: &str,
        expected_etag: Option<&str>,
        expected_version_id: Option<&str>,
        additional_headers: Option<AdditionalHeaders>,
    ) -> EtagRecheck {
        let Some(expected_etag) = expected_etag else {
            // No source-staleness guard armed — the 412 can only have come
            // from `if_none_match("*")` on the destination.
            return EtagRecheck::SourceMatches;
        };

        let mut builder = client.head_object().bucket(bucket).key(key).if_match(expected_etag);
        // When the copy targeted a specific source version (via `?versionId=` in
        // copy_source), the recheck must target the same version. Otherwise the
        // HEAD resolves to the latest version and reports SourceStale even when
        // the requested version still matches — misclassifying a destination
        // conflict as source staleness.
        if let Some(version_id) = expected_version_id {
            builder = builder.version_id(version_id);
        }
        let builder = Self::apply_request_context(
            builder.customize(),
            additional_headers,
            RequestType::Api,
            client.proxy_interceptor.clone(),
            None,
            client.rate_limit_interceptors.head_object.clone(),
        );
        match builder
            .send()
            .instrument(info_span!("HeadObject (etag disambiguation)"))
            .await
            .map_err(error::SdkError::from)
            .context(error::Sdk {})
        {
            Ok(_) => EtagRecheck::SourceMatches,
            Err(err) if err.is_precondition_failed() => EtagRecheck::SourceStale,
            Err(_) => EtagRecheck::Ambiguous,
        }
    }

    pub async fn copy_sdk(
        &self,
        mut source_resource_identity: Identity,
        mut destination_resource_address: Address,
        previous_version: Option<String>,
        fail_if_exists: bool,
        additional_headers: Option<AdditionalHeaders>,
        is_move: bool,
    ) -> Result<Identity> {
        if destination_resource_address.is_versioned() {
            return Err(error::Error::invalid_argument(format!(
                "Versioned resource addresses are not supported for copy destination: {}",
                destination_resource_address
            ))
            .into());
        }
        if fail_if_exists && previous_version.is_some() {
            return Err(
                error::Error::invalid_argument("fail_if_exists and previous_version are mutually exclusive").into(),
            );
        }

        let client = self.client(&mut destination_resource_address).await?;
        let bucket = &client.bucket;
        let key = destination_resource_address.key()?;

        // If a previous version is provided, we need to check if the key matches the destination key.
        let dest_etag = if let Some(previous_version) = previous_version {
            let prev_ver = Identity::parse(&previous_version)?;
            if prev_ver.key()? != key {
                return Err(error::SdkError::precondition_failed(format!(
                    "Previous version key does not match destination key: {} != {}",
                    prev_ver.key()?,
                    key
                )))
                .context(error::Sdk {})?;
            }
            Some(prev_ver.etag()?.to_owned())
        } else {
            None
        };

        let source_client = self.client(&mut source_resource_identity.resource_address).await?;
        let source_bucket = source_client.bucket.clone();
        let source_key = source_resource_identity.key()?.into_owned();
        trace_limited!(%source_bucket, %source_key, %bucket, %key);

        // The x-amz-copy-source header requires the key to be percent-encoded (AWS S3 API spec).
        let encoded_source_key = encode_path(&source_key);

        // We only allow copying from a version for copy operations, not move operations.
        let copy_source = match source_resource_identity.version_id.as_ref() {
            Some(version_id) if !is_move => {
                format!("{source_bucket}/{encoded_source_key}?versionId={version_id}")
            },
            _ => format!("{source_bucket}/{encoded_source_key}"),
        };
        trace_limited!(%copy_source);

        let mut builder = client.copy_object().bucket(bucket).copy_source(copy_source).key(key);

        // Keep a copy of the source etag for post-error disambiguation (if a
        // 412 fires when both `copy_source_if_match` and `if_none_match("*")`
        // are armed, we re-check the source to figure out which precondition
        // actually failed). The version_id mirrors the targeting logic used in
        // `copy_source` above so the recheck queries the same version (or the
        // latest, for moves) — otherwise we'd misclassify a destination conflict
        // as source staleness whenever the latest source differs from the
        // version actually being copied.
        let source_etag_for_disambiguation = source_resource_identity.etag.clone();
        let source_version_id_for_disambiguation = if is_move {
            None
        } else {
            source_resource_identity.version_id.clone()
        };

        // etag constraints. The Identity no longer
        // carries last_modified, so we rely solely on `copy_source_if_match`
        // (etag) for the source-object staleness guard. Etag changes whenever
        // content changes, so a stale caller still gets a precondition failure.
        if let Some(etag) = source_resource_identity.etag {
            trace_limited!(etag);
            builder = builder.copy_source_if_match(etag);
        }

        // if-match constraints on the destination object
        if let Some(dest_etag) = dest_etag {
            builder = builder.if_match(dest_etag);
        }
        if fail_if_exists {
            builder = builder.if_none_match("*");
        }

        let send_result = Self::apply_request_context(
            builder.customize(),
            additional_headers.clone(),
            RequestType::Api,
            client.proxy_interceptor.clone(),
            None,
            client.rate_limit_interceptors.copy_object.clone(),
        )
        .send()
        .instrument(info_span!("copy_object"))
        .await
        .map_err(error::SdkError::from)
        .context(error::Sdk {});

        let result = match send_result {
            Ok(result) => result,
            Err(err) if fail_if_exists && err.is_precondition_failed() => {
                // A 412 here is ambiguous between `copy_source_if_match` (source
                // staleness) and `if_none_match("*")` (destination conflict).
                // Recheck the source's etag to disambiguate. On ambiguous
                // recheck (transient/permissions error during the recheck) we
                // surface the original 412 rather than guess.
                match Self::source_etag_recheck(
                    &source_client,
                    &source_bucket,
                    &source_key,
                    source_etag_for_disambiguation.as_deref(),
                    source_version_id_for_disambiguation.as_deref(),
                    additional_headers.clone(),
                )
                .await
                {
                    EtagRecheck::SourceMatches => {
                        return Err(error::Error::conflict("Resource already exists").into());
                    },
                    EtagRecheck::SourceStale | EtagRecheck::Ambiguous => {
                        return Err(err.into());
                    },
                }
            },
            Err(err) => return Err(err.into()),
        };
        trace_limited!(?result);

        // extract version_id from the outer CopyObjectOutput before destructuring,
        // as copy_object_result (CopyObjectResult) does not carry it.
        let version_id = result.version_id.clone();
        let result = result
            .copy_object_result
            .ok_or(error::Error::internal("S3 returned an empty copy result"))?;

        let resource_identity: Identity = (destination_resource_address, result.e_tag, version_id).into();

        debug_limited!(?resource_identity);

        Ok(resource_identity)
    }

    // parse out the etag for if-match from an identity
    pub fn if_match(resource_identity: Option<&str>) -> std::result::Result<Option<String>, error::Error> {
        match resource_identity {
            Some(resource_identity) => {
                let Identity { etag, .. } = Identity::parse(resource_identity)?;
                Ok(etag)
            },
            None => Ok(None),
        }
    }

    pub async fn move_object_sdk(
        &self,
        mut source_resource_address: Address,
        destination_resource_address: Address,
        source_previous_version: Option<Identity>,
        destination_previous_version: Option<String>,
        fail_if_exists: bool,
        additional_headers: Option<AdditionalHeaders>,
    ) -> Result<Identity> {
        if source_resource_address.is_versioned() {
            return Err(error::Error::invalid_argument(format!(
                "Versioned resource addresses are not supported for move source: {}",
                source_resource_address
            ))
            .into());
        }

        // Check if the source and destination addresses are folders. NOT_FOUND might be a bit strange,
        // but if someone provides a folder address without the '/', we'll return NOT_FOUND, so for
        // consistency, we'll return NOT_FOUND as well if the address explicitly ends with a '/'.
        if source_resource_address.path().ends_with('/') {
            return Err(error::SdkError::not_found(format!(
                "Move source address cannot be a folder: {}",
                source_resource_address.as_ref()
            )))
            .context(error::Sdk {})?;
        }
        if destination_resource_address.path().ends_with('/') {
            return Err(error::SdkError::not_found(format!(
                "Move destination address cannot be a folder: {}",
                destination_resource_address.as_ref()
            )))
            .context(error::Sdk {})?;
        }

        let source_id = if let Some(id) = source_previous_version {
            id
        } else {
            let client = self.client(&mut source_resource_address).await?;
            (&self
                .stat_sdk(&client, source_resource_address.clone(), additional_headers.clone())
                .await?)
                .try_into()?
        };

        let resource_identity = self
            .copy_sdk(
                source_id.clone(),
                destination_resource_address.clone(),
                destination_previous_version,
                fail_if_exists,
                additional_headers.clone(),
                true,
            )
            .await?;

        // If the source and destination are the same, we don't want to delete the source object.
        // This would result in deleting the object we just copied because it would have the same etag
        // etag as the source object and we can't provide a last-modified time to delete
        if source_resource_address == destination_resource_address {
            debug_limited!("Source and destination objects are the same. Not deleting source object");
            return Ok(resource_identity);
        }

        match self
            .delete_sdk(
                source_resource_address,
                Some(source_id.to_string()),
                additional_headers.clone(),
            )
            .await
        {
            Ok(_) => Ok(resource_identity),
            Err(e) => match &e {
                error::Error::Sdk { source, .. } => match &**source {
                    error::SdkError::NotFound { .. } => {
                        debug_limited!("Source resource not found. Returning Ok");
                        Ok(resource_identity)
                    },
                    error::SdkError::PreconditionFailed { .. } => {
                        debug_limited!("Source resource changed. Returning Ok");
                        Ok(resource_identity)
                    },
                    _ => Err(e
                        .prepend_message("Source object has been copied to destination, but failed to delete source")
                        .into()),
                },
                _ => Err(e
                    .prepend_message("Source object has been copied to destination, but failed to delete source")
                    .into()),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use aws_sdk_s3::{error::SdkError, operation::list_object_versions::ListObjectVersionsError};
    use aws_smithy_runtime_api::{
        client::result::ServiceError,
        http::{Response, StatusCode},
    };
    use aws_smithy_types::{body::SdkBody, error::ErrorMetadata};
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

    use super::{
        classify_list_object_versions_fallback, decode_list_versions_page_markers, encode_list_versions_page_markers,
        ListObjectVersionsFallback, ListObjectVersionsPageMarkers, LIST_OBJECT_VERSIONS_PAGE_MARKERS_PREFIX,
    };

    fn list_versions_markers(
        key_marker: Option<&str>,
        version_id_marker: Option<&str>,
    ) -> ListObjectVersionsPageMarkers {
        ListObjectVersionsPageMarkers {
            key_marker: key_marker.map(String::from),
            version_id_marker: version_id_marker.map(String::from),
            list_objects_v2_continuation_token: None,
            list_versions_done: false,
            list_objects_v2_done: true,
        }
    }

    fn service_error(
        status: u16,
        code: Option<&str>,
    ) -> SdkError<ListObjectVersionsError, aws_smithy_runtime_api::http::Response> {
        let source = match code {
            Some(code) => ListObjectVersionsError::generic(ErrorMetadata::builder().code(code).build()),
            None => ListObjectVersionsError::unhandled("unmodeled response"),
        };
        let raw = Response::new(StatusCode::try_from(status).unwrap(), SdkBody::empty());
        SdkError::ServiceError(ServiceError::builder().source(source).raw(raw).build())
    }

    #[test]
    fn list_object_versions_fallback_classifier_cacheable_gcs_shape() {
        let err = service_error(200, None);
        assert_eq!(
            classify_list_object_versions_fallback(&err),
            ListObjectVersionsFallback::CacheableUnsupported
        );
    }

    #[test]
    fn list_object_versions_fallback_classifier_access_denied_is_cacheable() {
        let err = service_error(403, Some("AccessDenied"));
        assert_eq!(
            classify_list_object_versions_fallback(&err),
            ListObjectVersionsFallback::CacheableDenied
        );
    }

    #[test]
    fn list_object_versions_fallback_classifier_not_implemented_is_cacheable() {
        let err = service_error(501, Some("NotImplemented"));
        assert_eq!(
            classify_list_object_versions_fallback(&err),
            ListObjectVersionsFallback::CacheableUnsupported
        );
    }

    #[test]
    fn list_object_versions_fallback_classifier_transport_errors_do_not_fallback() {
        let err: SdkError<ListObjectVersionsError, aws_smithy_runtime_api::http::Response> =
            SdkError::construction_failure("request build failed");
        assert_eq!(
            classify_list_object_versions_fallback(&err),
            ListObjectVersionsFallback::NoFallback
        );
    }

    #[test]
    fn tagged_page_markers_round_trip_both_markers() {
        let token =
            encode_list_versions_page_markers(list_versions_markers(Some("photos/cat.jpg"), Some("vid-abc-123")))
                .expect("both markers present");
        assert!(token.starts_with(LIST_OBJECT_VERSIONS_PAGE_MARKERS_PREFIX));
        let markers = decode_list_versions_page_markers(&token).unwrap();
        assert_eq!(
            (markers.key_marker, markers.version_id_marker),
            (Some("photos/cat.jpg".into()), Some("vid-abc-123".into()))
        );
        assert!(markers.list_objects_v2_done);
    }

    #[test]
    fn tagged_page_markers_round_trip_key_only() {
        let token = encode_list_versions_page_markers(list_versions_markers(Some("photos/cat.jpg"), None))
            .expect("key present");
        let markers = decode_list_versions_page_markers(&token).unwrap();
        assert_eq!(
            (markers.key_marker, markers.version_id_marker),
            (Some("photos/cat.jpg".into()), None)
        );
        assert!(markers.list_objects_v2_done);
    }

    #[test]
    fn tagged_page_markers_round_trip_dual_cursors() {
        let token = encode_list_versions_page_markers(ListObjectVersionsPageMarkers {
            key_marker: Some("photos/cat.jpg".into()),
            version_id_marker: Some("vid-abc-123".into()),
            list_objects_v2_continuation_token: Some("v2-token".into()),
            list_versions_done: false,
            list_objects_v2_done: false,
        })
        .expect("dual cursors present");
        let markers = decode_list_versions_page_markers(&token).unwrap();
        assert_eq!(
            markers,
            ListObjectVersionsPageMarkers {
                key_marker: Some("photos/cat.jpg".into()),
                version_id_marker: Some("vid-abc-123".into()),
                list_objects_v2_continuation_token: Some("v2-token".into()),
                list_versions_done: false,
                list_objects_v2_done: false,
            }
        );
    }

    #[test]
    fn tagged_page_markers_round_trip_one_source_done() {
        let token = encode_list_versions_page_markers(ListObjectVersionsPageMarkers {
            key_marker: None,
            version_id_marker: None,
            list_objects_v2_continuation_token: Some("v2-token".into()),
            list_versions_done: true,
            list_objects_v2_done: false,
        })
        .expect("v2 cursor still active");
        let markers = decode_list_versions_page_markers(&token).unwrap();
        assert_eq!(
            markers,
            ListObjectVersionsPageMarkers {
                key_marker: None,
                version_id_marker: None,
                list_objects_v2_continuation_token: Some("v2-token".into()),
                list_versions_done: true,
                list_objects_v2_done: false,
            }
        );
    }

    #[test]
    fn encode_none_when_no_resume_state_or_both_sources_done() {
        assert_eq!(
            encode_list_versions_page_markers(ListObjectVersionsPageMarkers::default()),
            None
        );
        assert_eq!(
            encode_list_versions_page_markers(ListObjectVersionsPageMarkers {
                key_marker: None,
                version_id_marker: None,
                list_objects_v2_continuation_token: None,
                list_versions_done: true,
                list_objects_v2_done: true,
            }),
            None
        );
    }

    #[test]
    fn decode_treats_untagged_input_as_raw_key_marker() {
        let markers = decode_list_versions_page_markers("legacy-bare-key").unwrap();
        assert_eq!(
            (markers.key_marker, markers.version_id_marker),
            (Some("legacy-bare-key".into()), None)
        );
        assert!(markers.list_objects_v2_done);
    }

    #[test]
    fn decode_treats_marker_looking_untagged_input_as_raw_key_marker() {
        let markers = decode_list_versions_page_markers("3|abcdef").unwrap();
        assert_eq!(
            (markers.key_marker, markers.version_id_marker),
            (Some("3|abcdef".into()), None)
        );
        assert!(markers.list_objects_v2_done);
    }

    #[test]
    fn decode_treats_legacy_tagged_token_as_list_versions_only() {
        let json = br#"{"k":"legacy-key","v":"legacy-version"}"#;
        let token = format!(
            "{LIST_OBJECT_VERSIONS_PAGE_MARKERS_PREFIX}{}",
            URL_SAFE_NO_PAD.encode(json)
        );
        let markers = decode_list_versions_page_markers(&token).unwrap();
        assert_eq!(
            markers,
            ListObjectVersionsPageMarkers {
                key_marker: Some("legacy-key".into()),
                version_id_marker: Some("legacy-version".into()),
                list_objects_v2_continuation_token: None,
                list_versions_done: false,
                list_objects_v2_done: true,
            }
        );
    }

    #[test]
    fn tagged_page_markers_round_trip_marker_looking_key() {
        let token =
            encode_list_versions_page_markers(list_versions_markers(Some("3|abcdef"), Some("vid-123"))).unwrap();
        let markers = decode_list_versions_page_markers(&token).unwrap();
        assert_eq!(
            (markers.key_marker, markers.version_id_marker),
            (Some("3|abcdef".into()), Some("vid-123".into()))
        );
    }

    #[test]
    fn tagged_page_markers_round_trip_key_containing_pipes() {
        let key = "odd|key|with|pipes";
        let token = encode_list_versions_page_markers(list_versions_markers(Some(key), Some("v1"))).unwrap();
        let markers = decode_list_versions_page_markers(&token).unwrap();
        assert_eq!(
            (markers.key_marker, markers.version_id_marker),
            (Some(key.into()), Some("v1".into()))
        );
    }

    #[test]
    fn tagged_page_markers_round_trip_key_with_multibyte_utf8() {
        let key = "photos/café.jpg";
        let token = encode_list_versions_page_markers(list_versions_markers(Some(key), Some("v-xyz"))).unwrap();
        let markers = decode_list_versions_page_markers(&token).unwrap();
        assert_eq!(
            (markers.key_marker, markers.version_id_marker),
            (Some(key.into()), Some("v-xyz".into()))
        );
    }

    #[test]
    fn decode_rejects_malformed_tagged_token() {
        let token = format!("{LIST_OBJECT_VERSIONS_PAGE_MARKERS_PREFIX}not-valid-base64!");
        assert!(decode_list_versions_page_markers(&token).is_err());
    }

    #[test]
    fn tagged_page_markers_round_trip_version_id_only() {
        let token = encode_list_versions_page_markers(list_versions_markers(None, Some("vid-only"))).unwrap();
        let markers = decode_list_versions_page_markers(&token).unwrap();
        assert_eq!(
            (markers.key_marker, markers.version_id_marker),
            (None, Some("vid-only".into()))
        );
    }
}
