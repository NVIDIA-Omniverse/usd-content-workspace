// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use fluent_uri::{
    pct_enc::{encoder::Query, DecodedChunk, EStr, Encoder},
    Uri,
};
use futures::future::BoxFuture;
use percent_encoding::{percent_encode, AsciiSet};
use snafu::IntoError;
use storage_core::layer::Layer;
use url_escape::{PATH, QUERY};

use crate::{
    backend::metadata::{
        batch::{BatchRequestEntry, BatchResponse, BatchResponseEntry},
        canonicalization::error::{CanonicalizationError, Error},
        *,
    },
    common::request::RequestExt,
};

mod error;

const RAW_PATH_ENCODE_SET: AsciiSet = PATH.add(b'%');
const RAW_QUERY_ENCODE_SET: AsciiSet = QUERY.add(b'%').add(b'+');

/// Canonicalizes resource URIs and resource locations in metadata calls.
pub struct UriCanonicalization;

impl<S> Layer<S> for UriCanonicalization {
    type Service = UriCanonicalizationService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        UriCanonicalizationService { inner }
    }
}

pub struct UriCanonicalizationService<T> {
    inner: T,
}

impl<T> UriCanonicalizationService<T> {
    async fn canonicalize_batch_and_invoke<'a, ReqE, ResE, ResT, I>(
        &self,
        mut requests: Vec<ReqE>,
        invoke: I,
    ) -> Result<ResT>
    where
        ReqE: BatchRequestEntry,
        ResE: BatchResponseEntry,
        ResT: BatchResponse<ResponseEntry = ResE>,
        I: FnOnce(Vec<ReqE>) -> BoxFuture<'a, Result<ResT>> + 'a,
    {
        let mut errors = Vec::new();
        let mut response_map: Option<Vec<u32>> = None;
        let mut request_index = 0u32;
        requests.retain_mut(|request| {
            let retain = if let Err(error) = canonicalize_resource_location_inplace(request.resource_location_mut()) {
                errors.push(ResE::from_error_entry(request_index, create_batch_error_entry(&error)));
                false
            } else {
                true
            };

            if retain {
                if let Some(response_map) = response_map.as_mut() {
                    response_map.push(request_index);
                }
            } else {
                if response_map.is_none() {
                    response_map = Some((0..request_index).collect());
                }
            }

            request_index += 1;

            retain
        });

        if requests.is_empty() {
            return Ok(ResT::from_entries(errors));
        }

        let mut response = invoke(requests).await?;
        if let Some(response_map) = response_map {
            response.update_request_indexes(&response_map);
        }

        response.extend(errors);

        Ok(response)
    }
}

#[async_trait::async_trait]
impl<T: Metadata> Metadata for UriCanonicalizationService<T> {
    fn inner(&self) -> Option<&dyn Metadata> {
        None
    }

    async fn get_metadata_batch(&self, request: Request<GetMetadataBatchRequest>) -> Result<GetMetadataBatchResponse> {
        let (metadata, extensions, request) = request.into_parts();
        self.canonicalize_batch_and_invoke(request.requests, |requests| {
            self.inner.get_metadata_batch(Request::from_parts(
                metadata,
                extensions,
                GetMetadataBatchRequest { requests },
            ))
        })
        .await
    }

    async fn query_metadata(
        &self,
        mut request: Request<QueryMetadataRequest>,
        options: StreamingOptions,
    ) -> Result<BoxStream<QueryMetadataResponsePage>> {
        canonicalize_resource_location_inplace(request.get_mut().resource_location.as_mut())?;
        self.inner.query_metadata(request, options).await
    }

    async fn update_metadata_batch(
        &self,
        request: Request<UpdateMetadataBatchRequest>,
    ) -> Result<UpdateMetadataBatchResponse> {
        let (metadata, extensions, request) = request.into_parts();
        self.canonicalize_batch_and_invoke(request.requests, |requests| {
            self.inner.update_metadata_batch(Request::from_parts(
                metadata,
                extensions,
                UpdateMetadataBatchRequest { requests },
            ))
        })
        .await
    }

    async fn delete_metadata_batch(
        &self,
        request: Request<DeleteMetadataBatchRequest>,
    ) -> Result<DeleteMetadataBatchResponse> {
        let (metadata, extensions, request) = request.into_parts();
        self.canonicalize_batch_and_invoke(request.requests, |requests| {
            self.inner.delete_metadata_batch(Request::from_parts(
                metadata,
                extensions,
                DeleteMetadataBatchRequest { requests },
            ))
        })
        .await
    }
}

enum QueryBehavior {
    Discard,
    RequireETag,
}

enum PathBehavior {
    KeepUnchanged,
    AddTrailingSlash,
}

#[track_caller]
fn canonicalize_resource_location_inplace(location: Option<&mut ResourceLocation>) -> Result<(), Error> {
    use error::error::{ResourceLocationMissing, ResourceUriInvalid};
    use resource_location::Resource;

    match location
        .ok_or_else(|| ResourceLocationMissing.build())?
        .resource
        .as_mut()
        .ok_or_else(|| ResourceLocationMissing.build())?
    {
        Resource::ResourceAddress(value) => {
            *value = canonicalize_resource_uri_with_options(value, QueryBehavior::Discard, PathBehavior::KeepUnchanged)
                .map_err(|source| ResourceUriInvalid.into_error(source))?;
        },
        Resource::ResourceIdentity(value) => {
            value.encoded_identity = canonicalize_resource_uri_with_options(
                &value.encoded_identity,
                QueryBehavior::RequireETag,
                PathBehavior::KeepUnchanged,
            )
            .map_err(|source| ResourceUriInvalid.into_error(source))?;
        },
        Resource::FolderAddress(value) => {
            value.uri = canonicalize_resource_uri_with_options(
                &value.uri,
                QueryBehavior::Discard,
                PathBehavior::AddTrailingSlash,
            )
            .map_err(|source| ResourceUriInvalid.into_error(source))?;
        },
    }

    Ok(())
}

fn create_batch_error_entry(error: &Error) -> BatchErrorEntry {
    BatchErrorEntry {
        code: batch_error_entry::Code::InvalidArgument as i32,
        message: error.to_string(),
    }
}

fn canonicalize_resource_uri_with_options(
    uri: &str,
    etag_behavior: QueryBehavior,
    path_behavior: PathBehavior,
) -> Result<String, CanonicalizationError> {
    use error::canonicalization_error::{AuthorityMissing, ETagMissing, ResourceUriInvalid, UnexpectedUserInfo};

    let uri = Uri::parse(uri).map_err(|source| ResourceUriInvalid.into_error(source))?;
    let authority = uri.authority().ok_or_else(|| AuthorityMissing.build())?;
    if authority.has_userinfo() {
        return UnexpectedUserInfo.fail();
    }

    let etag = match etag_behavior {
        QueryBehavior::Discard => None,
        QueryBehavior::RequireETag => Some(get_etag_query_parameter(uri.query()).ok_or_else(|| ETagMissing.build())?),
    };

    let mut buffer = Vec::with_capacity(uri.as_str().len());
    buffer.extend(uri.scheme().as_str().as_bytes().iter().map(u8::to_ascii_lowercase));
    buffer.extend_from_slice(b"://");
    buffer.extend(authority.as_str().as_bytes().iter().map(u8::to_ascii_lowercase));

    let path = uri.path();
    if path.is_empty() {
        buffer.extend_from_slice(b"/");
    } else {
        append_transcoded(&mut buffer, path, false, &RAW_PATH_ENCODE_SET);
        if matches!(path_behavior, PathBehavior::AddTrailingSlash) && !buffer.ends_with(b"/") {
            buffer.extend_from_slice(b"/");
        }
    }

    if let Some(etag) = etag {
        buffer.extend_from_slice(b"?etag=");
        append_transcoded(&mut buffer, etag, true, &RAW_QUERY_ENCODE_SET);
    }

    debug_assert!(buffer.is_ascii());

    Ok(unsafe { String::from_utf8_unchecked(buffer) })
}

fn append_transcoded(
    dst: &mut Vec<u8>,
    value: &EStr<impl Encoder>,
    decode_plus_as_space: bool,
    ascii_set: &'static AsciiSet,
) {
    for chunk in value.decode() {
        match chunk {
            DecodedChunk::PctDecoded(byte) => {
                encode_and_append_uri_component(dst, &[byte], ascii_set);
            },
            DecodedChunk::Unencoded(chunk) => {
                if !decode_plus_as_space {
                    encode_and_append_uri_component(dst, chunk.as_bytes(), ascii_set);
                    continue;
                }

                let mut chunk = chunk.as_bytes();
                while !chunk.is_empty() {
                    if let Some(plus_pos) = chunk.iter().position(|b| *b == b'+') {
                        if plus_pos != 0 {
                            encode_and_append_uri_component(dst, &chunk[..plus_pos], ascii_set);
                        }

                        dst.extend_from_slice(b"%20");
                        chunk = &chunk[plus_pos + 1..];
                    } else {
                        encode_and_append_uri_component(dst, chunk, ascii_set);
                        break;
                    }
                }
            },
        }
    }
}

fn encode_and_append_uri_component(dst: &mut Vec<u8>, value: &[u8], ascii_set: &'static AsciiSet) {
    for encoded in percent_encode(value, ascii_set) {
        dst.extend_from_slice(encoded.as_bytes());
    }
}

fn get_etag_query_parameter(query: Option<&EStr<Query>>) -> Option<&EStr<Query>> {
    for segment in query?.split('&') {
        let Some((key, value)) = segment.split_once('=') else {
            continue;
        };

        if key == "etag" {
            return Some(value);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::{canonicalize_resource_uri_with_options, PathBehavior, QueryBehavior};

    #[rstest]
    #[case(
        "https://s3.amazonaws.com",
        QueryBehavior::Discard,
        PathBehavior::KeepUnchanged,
        "https://s3.amazonaws.com/"
    )]
    #[case(
        "https://s3.amazonaws.com/",
        QueryBehavior::Discard,
        PathBehavior::KeepUnchanged,
        "https://s3.amazonaws.com/"
    )]
    #[case(
        "https://s3.amazonaws.com",
        QueryBehavior::Discard,
        PathBehavior::AddTrailingSlash,
        "https://s3.amazonaws.com/"
    )]
    #[case(
        "https://s3.amazonaws.com/",
        QueryBehavior::Discard,
        PathBehavior::AddTrailingSlash,
        "https://s3.amazonaws.com/"
    )]
    #[case(
        "https://s3.amazonaws.com/path",
        QueryBehavior::Discard,
        PathBehavior::AddTrailingSlash,
        "https://s3.amazonaws.com/path/"
    )]
    #[case(
        "https://s3.amazonaws.com/path/",
        QueryBehavior::Discard,
        PathBehavior::AddTrailingSlash,
        "https://s3.amazonaws.com/path/"
    )]
    #[case(
        "https://s3.amazonaws.com/path",
        QueryBehavior::Discard,
        PathBehavior::KeepUnchanged,
        "https://s3.amazonaws.com/path"
    )]
    #[case(
        "https://s3.amazonaws.com/path?etag=abc",
        QueryBehavior::Discard,
        PathBehavior::KeepUnchanged,
        "https://s3.amazonaws.com/path"
    )]
    #[case(
        "https://s3.amazonaws.com/path?query",
        QueryBehavior::Discard,
        PathBehavior::KeepUnchanged,
        "https://s3.amazonaws.com/path"
    )]
    #[case(
        "https://s3.amazonaws.com/path#fragment",
        QueryBehavior::Discard,
        PathBehavior::KeepUnchanged,
        "https://s3.amazonaws.com/path"
    )]
    #[case(
        "https://63.64.65.66:179/path",
        QueryBehavior::Discard,
        PathBehavior::KeepUnchanged,
        "https://63.64.65.66:179/path"
    )]
    #[case(
        "https://myhost.mydomain/%30%31%32%33%2f%4f%3fetag=123",
        QueryBehavior::Discard,
        PathBehavior::KeepUnchanged,
        "https://myhost.mydomain/0123/O%3Fetag=123"
    )]
    #[case(
        "https://myhost.mydomain/%30%31%32%33%2f%4f%3fetag=123",
        QueryBehavior::Discard,
        PathBehavior::KeepUnchanged,
        "https://myhost.mydomain/0123/O%3Fetag=123"
    )]
    #[case(
        "https://myhost.mydomain/path/../?etag=a+b",
        QueryBehavior::RequireETag,
        PathBehavior::KeepUnchanged,
        "https://myhost.mydomain/path/../?etag=a%20b"
    )]
    #[case(
        "https://myhost.mydomain/path+segment/?etag=+++a+b++%20%2b",
        QueryBehavior::RequireETag,
        PathBehavior::KeepUnchanged,
        "https://myhost.mydomain/path+segment/?etag=%20%20%20a%20b%20%20%20%2B"
    )]
    #[case(
        "https://localhost/a?etag=%21+%21",
        QueryBehavior::RequireETag,
        PathBehavior::KeepUnchanged,
        "https://localhost/a?etag=!%20!"
    )]
    #[case(
        "https://localhost/a?etag=+%21+%21",
        QueryBehavior::RequireETag,
        PathBehavior::KeepUnchanged,
        "https://localhost/a?etag=%20!%20!"
    )]
    fn test_canonicalize_resource_uri_with_options(
        #[case] uri: &str,
        #[case] etag_behavior: QueryBehavior,
        #[case] path_behavior: PathBehavior,
        #[case] expected: &str,
    ) {
        let value = canonicalize_resource_uri_with_options(uri, etag_behavior, path_behavior).unwrap();
        assert_eq!(value, expected);
    }
}
