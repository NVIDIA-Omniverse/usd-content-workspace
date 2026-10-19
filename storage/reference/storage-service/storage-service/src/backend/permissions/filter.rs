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

use regex_automata::{
    dfa::{dense, Automaton, OverlappingState},
    util::syntax,
    Input, MatchKind,
};
use snafu::{IntoError, Location, Snafu};
use storage_service_permissions::types::ResourceType;
use tracing::info;

use super::error::{Error, FilterSnafu};
use crate::common::{
    endpoint::EndpointUrl,
    routing::{
        az_blob_account_uri_regex, az_blob_any_uri_regex, az_blob_object_uri_regex, s3_any_uri_regex,
        s3_object_uri_regex,
    },
};

#[derive(Debug, Snafu)]
#[snafu(visibility(pub), module, context(suffix(false)))]
pub enum BuildError {
    #[snafu(display("failed to create the permission request filter"))]
    DfaCreate {
        source: Box<dyn std::error::Error + Send + Sync>,
        #[snafu(provide, implicit)]
        location: Location,
    },
}

/// Excludes resources at endpoints with `permissions_enabled = false` from permission checks.
#[derive(Debug)]
pub struct PermissionRequestFilter {
    request_lookup: dense::DFA<Vec<u32>>,
    request_lookup_rules: Vec<RequestLookupRule>,
}

impl PermissionRequestFilter {
    /// Build a filter from S3 and Azure Blob endpoints that opted out of permission enforcement.
    pub fn try_from_endpoints(
        s3_endpoints: impl IntoIterator<Item = impl AsRef<EndpointUrl>>,
        az_blob_endpoints: impl IntoIterator<Item = impl AsRef<EndpointUrl>>,
    ) -> Result<Self, BuildError> {
        let patterns = s3_endpoints
            .into_iter()
            .map(|endpoint| {
                let endpoint = endpoint.as_ref();

                (
                    s3_any_uri_regex(endpoint.scheme(), endpoint.hostname_port(), endpoint.path()),
                    s3_object_uri_regex(endpoint.scheme(), endpoint.hostname_port(), endpoint.path()),
                    None,
                )
            })
            .chain(az_blob_endpoints.into_iter().map(|endpoint| {
                let endpoint = endpoint.as_ref();

                (
                    az_blob_any_uri_regex(endpoint.scheme(), endpoint.hostname_port()),
                    az_blob_object_uri_regex(endpoint.scheme(), endpoint.hostname_port()),
                    // Advertised Azure routes start at the account root without a container.
                    Some(az_blob_account_uri_regex(endpoint.scheme(), endpoint.hostname_port())),
                )
            }))
            .flat_map(|(any_pattern, object_pattern, route_pattern)| {
                [
                    (any_pattern, RequestLookupRule::FolderOrObject),
                    (object_pattern, RequestLookupRule::Object),
                ]
                .into_iter()
                .chain(route_pattern.map(|pattern| (pattern, RequestLookupRule::Route)))
            });
        let (patterns, request_lookup_rules): (Vec<_>, Vec<_>) = patterns.unzip();
        let request_lookup = dense::Builder::new()
            .configure(dense::Config::new().minimize(true).match_kind(MatchKind::All))
            .syntax(syntax::Config::new().unicode(false))
            .build_many(&patterns)
            .map_err(|source| build_error::DfaCreate.into_error(Box::new(source)))?;
        info!(
            memory_usage = request_lookup.memory_usage(),
            "permission request filter created"
        );

        Ok(Self {
            request_lookup,
            request_lookup_rules,
        })
    }

    /// Whether the resource URI is excluded from permission checks for its resource type.
    pub fn is_excluded(&self, resource_uri: &str, resource_type: ResourceType) -> Result<bool, Error> {
        match resource_type {
            ResourceType::Folder => self.matches(resource_uri, |rule| {
                matches!(rule, RequestLookupRule::FolderOrObject | RequestLookupRule::Object)
            }),
            ResourceType::Object => self.matches(resource_uri, |rule| matches!(rule, RequestLookupRule::Object)),
        }
    }

    /// Whether a concrete advertised route prefix is excluded from permission checks.
    pub fn is_route_excluded(&self, route_prefix: &str) -> Result<bool, Error> {
        self.matches(route_prefix, |_| true)
    }

    fn matches(&self, uri: &str, accepts: impl Fn(RequestLookupRule) -> bool) -> Result<bool, Error> {
        let input = Input::new(uri);
        let mut state = OverlappingState::start();

        loop {
            self.request_lookup
                .try_search_overlapping_fwd(&input, &mut state)
                .map_err(|source| FilterSnafu.into_error(Box::new(source)))?;

            let Some(matched) = state.get_match() else {
                return Ok(false);
            };

            if accepts(self.request_lookup_rules[matched.pattern().as_usize()]) {
                return Ok(true);
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum RequestLookupRule {
    Object,
    FolderOrObject,
    Route,
}

#[cfg(test)]
mod tests {
    use rstest::{fixture, rstest};
    use storage_service_permissions::types::ResourceType;

    use super::PermissionRequestFilter;
    use crate::common::endpoint::EndpointUrl;

    #[fixture]
    fn configured_filter() -> PermissionRequestFilter {
        PermissionRequestFilter::try_from_endpoints(
            [EndpointUrl::parse_s3_endpoint_url("https://storage.example/bucket").unwrap()],
            [EndpointUrl::parse_azure_blob_endpoint_url("https://public.blob.core.windows.net").unwrap()],
        )
        .unwrap()
    }

    #[rstest]
    #[case("https://storage.example/bucket", ResourceType::Folder)]
    #[case("https://storage.example/bucket/", ResourceType::Folder)]
    #[case("https://storage.example/bucket/object", ResourceType::Object)]
    #[case("https://public.blob.core.windows.net/container", ResourceType::Folder)]
    #[case("https://public.blob.core.windows.net/container/", ResourceType::Folder)]
    #[case("https://public.blob.core.windows.net/container/object", ResourceType::Object)]
    fn test_matching_resource_is_excluded(
        configured_filter: PermissionRequestFilter,
        #[case] uri: &str,
        #[case] resource_type: ResourceType,
    ) {
        assert!(configured_filter.is_excluded(uri, resource_type).unwrap());
    }

    #[rstest]
    #[case("https://storage.example/bucket", ResourceType::Object)]
    #[case("https://storage.example/other/object", ResourceType::Object)]
    #[case("https://storage.example/other", ResourceType::Folder)]
    #[case("https://public.blob.core.windows.net/container", ResourceType::Object)]
    #[case("https://private.blob.core.windows.net/container/object", ResourceType::Object)]
    #[case("https://private.blob.core.windows.net/container", ResourceType::Folder)]
    fn test_mismatching_resource_is_not_excluded(
        configured_filter: PermissionRequestFilter,
        #[case] uri: &str,
        #[case] resource_type: ResourceType,
    ) {
        assert!(!configured_filter.is_excluded(uri, resource_type).unwrap());
    }
}
