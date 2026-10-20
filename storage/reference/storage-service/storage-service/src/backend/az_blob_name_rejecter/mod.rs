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

use std::{borrow::Cow, sync::Arc};

use futures::future::BoxFuture;
use regex_automata::{
    dfa::{dense::DFA, Automaton},
    util::syntax,
    Input,
};
use snafu::IntoError;
use storage_core::{
    layer::Layer,
    traits::{Request, Result},
};

use tracing::info;

use crate::common::{
    endpoint::EndpointUrl,
    future::create_error_future,
    routing::{az_blob_any_uri_regex, WELL_KNOWN_AZ_BLOB_ANY_URI_REGEX},
};

mod error;
mod filefolder;
mod fileobject;
mod metadata;
mod utils;
mod versioning;

/// Rejects Azure Blob names that contain ambiguous characters and sequences, which undergo
/// undeclared normalization behind the scenes.
#[derive(Debug, Clone)]
pub struct AzureBlobNameRejecter {
    filter: Arc<DFA<Vec<u32>>>,
}

impl AzureBlobNameRejecter {
    /// Create a name rejecter from a sequence of Azure Blob endpoint URLs.
    pub fn try_from_azure_endpoints(
        endpoints: impl IntoIterator<Item = impl AsRef<EndpointUrl>>,
    ) -> Result<Self, error::InitError> {
        use error::init_error::*;

        let mut patterns = vec![Cow::Borrowed(WELL_KNOWN_AZ_BLOB_ANY_URI_REGEX)];
        for endpoint in endpoints {
            let endpoint = endpoint.as_ref();
            if endpoint.path() != "/" {
                return EndpointPathInvalid.fail();
            }

            patterns.push(Cow::Owned(az_blob_any_uri_regex(
                endpoint.scheme(),
                endpoint.hostname_port(),
            )));
        }

        let filter = DFA::builder()
            .syntax(syntax::Config::new().unicode(false))
            .build_many(&patterns)
            .map_err(|source| DFABuild.into_error(Box::new(source)))?;
        info!(memory_usage = filter.memory_usage(), "azure blob name filter created");

        Ok(Self {
            filter: Arc::new(filter),
        })
    }
}

impl<S> Layer<S> for AzureBlobNameRejecter {
    type Service = AzureBlobNameRejecterService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        AzureBlobNameRejecterService {
            filter: self.filter.clone(),
            inner,
        }
    }
}

#[derive(Debug)]
pub struct AzureBlobNameRejecterService<S> {
    inner: S,
    filter: Arc<DFA<Vec<u32>>>,
}

impl<S> AzureBlobNameRejecterService<S> {
    fn invoke_with_resource_address<'a, T, R>(
        &self,
        request: Request<T>,
        parameter: impl FnOnce(&T) -> &String,
        invoke: impl FnOnce(Request<T>) -> BoxFuture<'a, Result<R>>,
    ) -> BoxFuture<'a, Result<R>> {
        self.invoke_with_parameter(
            request,
            |message| Ok(parameter(message).as_str()),
            |source| error::error::ResourceAddressInvalid.into_error(source),
            invoke,
        )
    }

    fn invoke_with_parameter<'a, T, R>(
        &self,
        request: Request<T>,
        parameter: impl FnOnce(&T) -> Result<&str, error::Error>,
        invalid: impl FnOnce(error::InvalidUrl) -> error::Error,
        invoke: impl FnOnce(Request<T>) -> BoxFuture<'a, Result<R>>,
    ) -> BoxFuture<'a, Result<R>> {
        let validation = parameter(request.get_ref()).and_then(|parameter| self.validate_blob_path(parameter, invalid));
        match validation {
            Ok(()) => invoke(request),
            Err(error) => create_error_future(error),
        }
    }

    /// Validate the blob path after a matching URL prefix.
    fn validate_blob_path(
        &self,
        url: &str,
        invalid: impl FnOnce(error::InvalidUrl) -> error::Error,
    ) -> Result<(), error::Error> {
        use error::error::DFALookup;

        let match_ = self
            .filter
            .try_search_fwd(&Input::new(url))
            .map_err(|source| DFALookup.into_error(Box::new(source)))?;
        if let Some(match_) = match_ {
            let blob_path = match url.as_bytes()[match_.offset() - 1] {
                b'?' | b'#' => "",
                _ => &url[match_.offset()..],
            };

            utils::validate_blob_name(blob_path.bytes()).map_err(invalid)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use rstest::{fixture, rstest};
    use snafu::IntoError;
    use storage_core::layer::Layer;

    use super::{
        error::{error, Error, InvalidUrl},
        AzureBlobNameRejecter, AzureBlobNameRejecterService,
    };
    use crate::common::endpoint::EndpointUrl;

    #[fixture]
    fn service() -> AzureBlobNameRejecterService<()> {
        let layer = AzureBlobNameRejecter::try_from_azure_endpoints([
            EndpointUrl::parse_azure_blob_endpoint_url("http://custom.example.com/").unwrap(),
            EndpointUrl::parse_azure_blob_endpoint_url("https://other.example.com/").unwrap(),
            EndpointUrl::parse_azure_blob_endpoint_url("http://127.0.0.1:10000/").unwrap(),
        ])
        .unwrap();
        layer.layer(())
    }

    #[rstest]
    #[case("https://blobsblobs.blob.core.windows.net/container/object.")]
    #[case("HTTPS://BLOBSBLOBS.BLOB.CORE.WINDOWS.NET/%63ontainer%2fobject.")]
    fn test_rejects_invalid_paths_on_undeclared_well_known_hosts(
        service: AzureBlobNameRejecterService<()>,
        #[case] url: &str,
    ) {
        let result = service.validate_blob_path(url, |source| error::ResourceAddressInvalid.into_error(source));

        assert_matches!(
            result,
            Err(Error::ResourceAddressInvalid {
                source: InvalidUrl::TrailingDot { .. },
                ..
            }),
        );
    }

    #[rstest]
    #[case("http://custom.example.com/container/object.")]
    #[case("HTTP://CUSTOM.EXAMPLE.COM/container%2fobject.")]
    #[case("http://127.0.0.1:10000/container/object.")]
    #[case("HtTp://127.0.0.1:10000/%63ontainer%2Fobject.")]
    fn test_rejects_invalid_paths_on_declared_endpoints(service: AzureBlobNameRejecterService<()>, #[case] url: &str) {
        let result = service.validate_blob_path(url, |source| error::ResourceAddressInvalid.into_error(source));

        assert_matches!(
            result,
            Err(Error::ResourceAddressInvalid {
                source: InvalidUrl::TrailingDot { .. },
                ..
            }),
        );
    }

    #[rstest]
    #[case("http://blobsblobs.blob.core.windows.net/container/object")]
    #[case("HTTPS://BLOBSBLOBS.BLOB.CORE.WINDOWS.NET/container/object")]
    #[case("HtTp://CuStOm.ExAmPlE.CoM/")]
    #[case("http://custom.example.com/container?%00")]
    #[case("http://custom.example.com/container#%C2%80")]
    #[case("http://custom.example.com/container/blob?%00")]
    #[case("http://custom.example.com/container/blob#%C2%80")]
    fn test_accepts_valid_paths_on_matching_endpoints(service: AzureBlobNameRejecterService<()>, #[case] url: &str) {
        let result = service.validate_blob_path(url, |_| unreachable!("valid path must not invoke the error mapper"));

        assert!(result.is_ok());
    }

    #[rstest]
    #[case("https://unconfigured.example.com/parent/name.")]
    #[case("https://blobsblobs.blob.core.windows.net.evil/parent/name.")]
    #[case("https://blobsblobsXblob.core.windows.net/parent/name.")]
    #[case("http://customXexample.com/parent/name.")]
    #[case("http://custom.example.com.evil/parent/name.")]
    #[case("https://custom.example.com/parent/name.")]
    #[case("http://other.example.com/parent/name.")]
    #[case("ftp://blobsblobs.blob.core.windows.net/parent/name.")]
    #[case("prefix-http://custom.example.com/parent/name.")]
    #[case("http://127.0.0.1:10001/parent/name.")]
    #[case("http://127.0.0.1:100000/parent/name.")]
    #[case("https://blobsblobs.blob.core.windows.net")]
    #[case("http://custom.example.com")]
    #[case("https://other.example.com")]
    fn test_accepts_urls_outside_match_set(service: AzureBlobNameRejecterService<()>, #[case] url: &str) {
        let result = service.validate_blob_path(url, |_| {
            unreachable!("nonmatching URL must not invoke the error mapper")
        });

        assert!(result.is_ok());
    }
}
