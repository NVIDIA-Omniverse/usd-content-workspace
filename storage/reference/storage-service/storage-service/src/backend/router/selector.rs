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

use std::borrow::Cow;

use regex_automata::{
    dfa::{dense, Automaton, OverlappingState},
    util::syntax,
    Input, MatchKind,
};
use snafu::{IntoError, Location, Snafu};
use tracing::{info, warn};

use crate::{
    backend::router::Match,
    common::{
        endpoint::EndpointUrl,
        error::PrettyPrintErrorExt,
        routing::{
            az_blob_any_uri_regex, az_blob_object_uri_regex, s3_any_uri_regex, s3_object_uri_regex,
            WELL_KNOWN_AZ_BLOB_ANY_URI_REGEX, WELL_KNOWN_AZ_BLOB_OBJECT_URI_REGEX, WELL_KNOWN_S3_ANY_URI_REGEX,
            WELL_KNOWN_S3_OBJECT_URI_REGEX,
        },
    },
};

#[derive(Debug, Snafu)]
#[snafu(module, context(suffix(false)))]
pub enum BuildError {
    #[snafu(display("failed to create a pattern matcher"))]
    RegexCreate {
        source: Box<dyn std::error::Error + Send + Sync>,
        #[snafu(provide, implicit)]
        location: Location,
    },
}

/// A routed endpoint with its advertising options. A hidden endpoint is served as usual, it is
/// just not listed on the respective capability.
#[derive(Debug, Clone, Copy)]
pub struct RegexSelectorRoute<T> {
    pub endpoint: T,
    pub show_tla: bool,
    pub show_route: bool,
}

#[derive(Debug, Clone)]
pub struct RegexSelector {
    request_lookup: dense::DFA<Vec<u32>>,
    request_lookup_rules: Vec<RequestLookupRule>,
    wildcard_routes: Vec<String>,
}

impl RegexSelector {
    /// Create a selector for a set of azure endpoints.
    ///
    /// When `allow_unconfigured_endpoints` is set, requests to any `*.blob.core.windows.net`
    /// account are routed to (and advertised by) this backend; otherwise only explicitly
    /// configured endpoints match and everything else falls through to the dummy backend.
    pub fn try_from_azure_endpoints(
        routes: impl IntoIterator<Item = RegexSelectorRoute<impl AsRef<EndpointUrl>>>,
        allow_unconfigured_endpoints: bool,
    ) -> Result<Self, BuildError> {
        let (patterns, wildcard_routes) = if allow_unconfigured_endpoints {
            (
                vec![
                    (
                        Cow::Borrowed(WELL_KNOWN_AZ_BLOB_ANY_URI_REGEX),
                        RequestLookupRule::FolderOrObjectWithHiddenTla,
                    ),
                    (
                        Cow::Borrowed(WELL_KNOWN_AZ_BLOB_OBJECT_URI_REGEX),
                        RequestLookupRule::Object,
                    ),
                ],
                vec!["https://*.blob.core.windows.net/**".to_owned()],
            )
        } else {
            Default::default()
        };

        let selector = Self::try_from_routes(patterns, wildcard_routes, routes, |endpoint| {
            (
                az_blob_any_uri_regex(endpoint.scheme(), endpoint.hostname_port()),
                az_blob_object_uri_regex(endpoint.scheme(), endpoint.hostname_port()),
            )
        })?;
        info!(
            memory_usage = selector.request_lookup.memory_usage(),
            "azure blob routing table created"
        );

        Ok(selector)
    }

    /// Create a selector for a set of s3 endpoints.
    ///
    /// When `allow_unconfigured_endpoints` is set, requests to any `*.amazonaws.com` bucket are
    /// routed to (and advertised by) this backend; otherwise only explicitly configured
    /// endpoints match and everything else falls through to the dummy backend.
    pub fn try_from_s3_endpoints(
        routes: impl IntoIterator<Item = RegexSelectorRoute<impl AsRef<EndpointUrl>>>,
        allow_unconfigured_endpoints: bool,
    ) -> Result<Self, BuildError> {
        let (patterns, wildcard_routes) = if allow_unconfigured_endpoints {
            (
                vec![
                    (
                        Cow::Borrowed(WELL_KNOWN_S3_ANY_URI_REGEX),
                        RequestLookupRule::FolderOrObjectWithHiddenTla,
                    ),
                    (Cow::Borrowed(WELL_KNOWN_S3_OBJECT_URI_REGEX), RequestLookupRule::Object),
                ],
                vec![
                    "https://*.s3.*.amazonaws.com/**".to_owned(),
                    "https://*.s3-*.amazonaws.com/**".to_owned(),
                ],
            )
        } else {
            Default::default()
        };

        let selector = Self::try_from_routes(patterns, wildcard_routes, routes, |endpoint| {
            (
                s3_any_uri_regex(endpoint.scheme(), endpoint.hostname_port(), endpoint.path()),
                s3_object_uri_regex(endpoint.scheme(), endpoint.hostname_port(), endpoint.path()),
            )
        })?;
        info!(
            memory_usage = selector.request_lookup.memory_usage(),
            "s3 routing table created"
        );

        Ok(selector)
    }

    fn try_from_routes(
        mut patterns: Vec<(Cow<'static, str>, RequestLookupRule)>,
        mut wildcard_routes: Vec<String>,
        routes: impl IntoIterator<Item = RegexSelectorRoute<impl AsRef<EndpointUrl>>>,
        create_endpoint_patterns: impl Fn(&EndpointUrl) -> (String, String),
    ) -> Result<Self, BuildError> {
        for route in routes {
            let endpoint = route.endpoint.as_ref();
            let (folder_pattern, object_pattern) = create_endpoint_patterns(endpoint);
            patterns.push((
                Cow::Owned(folder_pattern),
                if route.show_tla {
                    RequestLookupRule::FolderOrObjectWithVisibleTla
                } else {
                    RequestLookupRule::FolderOrObjectWithHiddenTla
                },
            ));
            patterns.push((Cow::Owned(object_pattern), RequestLookupRule::Object));

            if route.show_route {
                wildcard_routes.push(format!("{endpoint}**"));
            }
        }

        let (patterns, rules): (Vec<_>, Vec<_>) = patterns.into_iter().unzip();

        Ok(Self {
            request_lookup: build_dfa(&patterns)?,
            request_lookup_rules: rules,
            wildcard_routes,
        })
    }

    fn matches(&self, value: &str, accepts: impl Fn(RequestLookupRule) -> bool) -> bool {
        let input = Input::new(value);
        let mut state = OverlappingState::start();

        loop {
            if let Err(error) = self.request_lookup.try_search_overlapping_fwd(&input, &mut state) {
                warn!(error = %error.pretty(), "resource uri matching failed");

                return false;
            }

            let Some(matched) = state.get_match() else {
                return false;
            };

            if accepts(self.request_lookup_rules[matched.pattern().as_usize()]) {
                return true;
            }
        }
    }
}

impl Match for RegexSelector {
    fn match_resource_address(&self, value: &str) -> bool {
        self.matches(value, |rule| matches!(rule, RequestLookupRule::Object))
    }

    fn match_resource_identity(&self, value: &str) -> bool {
        self.matches(value, |rule| matches!(rule, RequestLookupRule::Object))
    }

    fn match_folder_address(&self, value: &str) -> bool {
        self.matches(value, |_| true)
    }

    fn filter_top_level_address(&self, address: &str) -> bool {
        self.matches(address, |rule| {
            matches!(rule, RequestLookupRule::FolderOrObjectWithVisibleTla)
        })
    }

    fn iter_wildcard_routes(&self) -> impl Iterator<Item = &str> {
        self.wildcard_routes.iter().map(String::as_str)
    }
}

#[derive(Debug, Clone, Copy)]
enum RequestLookupRule {
    Object,
    FolderOrObjectWithVisibleTla,
    FolderOrObjectWithHiddenTla,
}

fn build_dfa(patterns: &[impl AsRef<str>]) -> Result<dense::DFA<Vec<u32>>, BuildError> {
    if patterns.is_empty() {
        return dense::DFA::never_match().map_err(|source| build_error::RegexCreate.into_error(Box::new(source)));
    }

    dense::Builder::new()
        .configure(dense::Config::new().minimize(true).match_kind(MatchKind::All))
        .syntax(syntax::Config::new().unicode(false))
        .build_many(patterns)
        .map_err(|source| build_error::RegexCreate.into_error(Box::new(source)))
}

#[cfg(test)]
mod tests {
    use rstest::{fixture, rstest};

    use super::{RegexSelector, RegexSelectorRoute};
    use crate::{backend::router::Match, common::endpoint::EndpointUrl};

    fn shown(endpoint: EndpointUrl) -> RegexSelectorRoute<EndpointUrl> {
        RegexSelectorRoute {
            endpoint,
            show_tla: true,
            show_route: true,
        }
    }

    #[fixture]
    fn az_selector(#[default(false)] allow_unconfigured_endpoints: bool) -> RegexSelector {
        RegexSelector::try_from_azure_endpoints(
            [shown(
                EndpointUrl::parse_azure_blob_endpoint_url("https://account.example").unwrap(),
            )],
            allow_unconfigured_endpoints,
        )
        .unwrap()
    }

    #[fixture]
    fn s3_selector(#[default(false)] allow_unconfigured_endpoints: bool) -> RegexSelector {
        RegexSelector::try_from_s3_endpoints(
            [shown(
                EndpointUrl::parse_s3_endpoint_url("https://storage.example/bucket").unwrap(),
            )],
            allow_unconfigured_endpoints,
        )
        .unwrap()
    }

    #[fixture]
    fn advertised_selector() -> RegexSelector {
        RegexSelector::try_from_s3_endpoints(
            [
                shown(EndpointUrl::parse_s3_endpoint_url("https://shown.s3.us-west-2.amazonaws.com").unwrap()),
                RegexSelectorRoute {
                    endpoint: EndpointUrl::parse_s3_endpoint_url("https://unlisted.s3.us-west-2.amazonaws.com")
                        .unwrap(),
                    show_tla: false,
                    show_route: true,
                },
                RegexSelectorRoute {
                    endpoint: EndpointUrl::parse_s3_endpoint_url("https://hidden.s3.us-west-2.amazonaws.com").unwrap(),
                    show_tla: false,
                    show_route: false,
                },
            ],
            true,
        )
        .unwrap()
    }

    fn overlapping_selector(reverse_rules: bool) -> RegexSelector {
        let mut routes = [
            RegexSelectorRoute {
                endpoint: EndpointUrl::parse_s3_endpoint_url("https://storage.example").unwrap(),
                show_tla: false,
                show_route: false,
            },
            shown(EndpointUrl::parse_s3_endpoint_url("https://storage.example/bucket").unwrap()),
        ];
        if reverse_rules {
            routes.reverse();
        }

        RegexSelector::try_from_s3_endpoints(routes, false).unwrap()
    }

    #[rstest]
    #[case("https://account.example/container", RegexSelector::match_folder_address)]
    #[case("https://account.example/container/", RegexSelector::match_folder_address)]
    #[case("https://account.example/container/object", RegexSelector::match_resource_address)]
    #[case("https://account.example/container/object", RegexSelector::match_resource_identity)]
    fn test_configured_azure_resource_matches(
        az_selector: RegexSelector,
        #[case] uri: &str,
        #[case] matches: fn(&RegexSelector, &str) -> bool,
    ) {
        assert!(matches(&az_selector, uri));
    }

    #[rstest]
    #[case("https://account.example/container", RegexSelector::match_resource_address)]
    #[case("https://account.example/container", RegexSelector::match_resource_identity)]
    #[case("https://other.blob.core.windows.net/container", RegexSelector::match_folder_address)]
    #[case(
        "https://other.blob.core.windows.net/container/object",
        RegexSelector::match_resource_address
    )]
    fn test_unmatched_azure_resource_does_not_match(
        az_selector: RegexSelector,
        #[case] uri: &str,
        #[case] matches: fn(&RegexSelector, &str) -> bool,
    ) {
        assert!(!matches(&az_selector, uri));
    }

    #[rstest]
    #[case("https://storage.example/bucket", RegexSelector::match_folder_address)]
    #[case("https://storage.example/bucket/", RegexSelector::match_folder_address)]
    #[case("https://storage.example/bucket/object", RegexSelector::match_resource_address)]
    #[case("https://storage.example/bucket/object", RegexSelector::match_resource_identity)]
    fn test_configured_s3_resource_matches(
        s3_selector: RegexSelector,
        #[case] uri: &str,
        #[case] matches: fn(&RegexSelector, &str) -> bool,
    ) {
        assert!(matches(&s3_selector, uri));
    }

    #[rstest]
    #[case("https://storage.example/bucket", RegexSelector::match_resource_address)]
    #[case("https://storage.example/bucket", RegexSelector::match_resource_identity)]
    #[case("https://other.s3.us-west-2.amazonaws.com", RegexSelector::match_folder_address)]
    #[case(
        "https://other.s3.us-west-2.amazonaws.com/object",
        RegexSelector::match_resource_address
    )]
    fn test_unmatched_s3_resource_does_not_match(
        s3_selector: RegexSelector,
        #[case] uri: &str,
        #[case] matches: fn(&RegexSelector, &str) -> bool,
    ) {
        assert!(!matches(&s3_selector, uri));
    }

    #[rstest]
    #[case("https://other.blob.core.windows.net/container", RegexSelector::match_folder_address)]
    #[case(
        "https://other.blob.core.windows.net/container/",
        RegexSelector::match_folder_address
    )]
    #[case(
        "https://other.blob.core.windows.net/container/object",
        RegexSelector::match_resource_address
    )]
    #[case(
        "https://other.blob.core.windows.net/container/object",
        RegexSelector::match_resource_identity
    )]
    fn test_unconfigured_azure_resource_matches_when_allowed(
        #[with(true)] az_selector: RegexSelector,
        #[case] uri: &str,
        #[case] matches: fn(&RegexSelector, &str) -> bool,
    ) {
        assert!(matches(&az_selector, uri));
    }

    #[rstest]
    #[case(
        "https://other.blob.core.windows.net/container",
        RegexSelector::match_resource_address
    )]
    #[case(
        "https://other.blob.core.windows.net/container",
        RegexSelector::match_resource_identity
    )]
    #[case(
        "https://other.blob.core.windows.net/container",
        RegexSelector::filter_top_level_address
    )]
    fn test_unconfigured_azure_root_does_not_match_object_or_visible_address(
        #[with(true)] az_selector: RegexSelector,
        #[case] uri: &str,
        #[case] matches: fn(&RegexSelector, &str) -> bool,
    ) {
        assert!(!matches(&az_selector, uri));
    }

    #[rstest]
    #[case("https://other.s3.us-west-2.amazonaws.com", RegexSelector::match_folder_address)]
    #[case("https://other.s3.us-west-2.amazonaws.com/", RegexSelector::match_folder_address)]
    #[case(
        "https://other.s3.us-west-2.amazonaws.com/object",
        RegexSelector::match_resource_address
    )]
    #[case(
        "https://other.s3.us-west-2.amazonaws.com/object",
        RegexSelector::match_resource_identity
    )]
    fn test_unconfigured_s3_resource_matches_when_allowed(
        #[with(true)] s3_selector: RegexSelector,
        #[case] uri: &str,
        #[case] matches: fn(&RegexSelector, &str) -> bool,
    ) {
        assert!(matches(&s3_selector, uri));
    }

    #[rstest]
    #[case("https://other.s3.us-west-2.amazonaws.com", RegexSelector::match_resource_address)]
    #[case("https://other.s3.us-west-2.amazonaws.com", RegexSelector::match_resource_identity)]
    #[case("https://other.s3.us-west-2.amazonaws.com", RegexSelector::filter_top_level_address)]
    fn test_unconfigured_s3_root_does_not_match_object_or_visible_address(
        #[with(true)] s3_selector: RegexSelector,
        #[case] uri: &str,
        #[case] matches: fn(&RegexSelector, &str) -> bool,
    ) {
        assert!(!matches(&s3_selector, uri));
    }

    #[test]
    fn test_without_endpoints_matches_nothing() {
        let selector =
            RegexSelector::try_from_s3_endpoints(std::iter::empty::<RegexSelectorRoute<EndpointUrl>>(), false).unwrap();

        assert!(!selector.match_folder_address("https://bucket.s3.us-west-2.amazonaws.com"));
    }

    #[rstest]
    fn test_unconfigured_azure_wildcard_is_advertised(#[with(true)] az_selector: RegexSelector) {
        assert_eq!(
            az_selector.iter_wildcard_routes().collect::<Vec<_>>(),
            ["https://*.blob.core.windows.net/**", "https://account.example/**",]
        );
    }

    #[rstest]
    fn test_unconfigured_s3_wildcards_are_advertised(#[with(true)] s3_selector: RegexSelector) {
        assert_eq!(
            s3_selector.iter_wildcard_routes().collect::<Vec<_>>(),
            [
                "https://*.s3.*.amazonaws.com/**",
                "https://*.s3-*.amazonaws.com/**",
                "https://storage.example/bucket/**",
            ]
        );
    }

    #[rstest]
    fn test_shown_endpoint_is_visible(advertised_selector: RegexSelector) {
        assert!(advertised_selector.filter_top_level_address("https://shown.s3.us-west-2.amazonaws.com"));
    }

    #[rstest]
    #[case("https://unlisted.s3.us-west-2.amazonaws.com")]
    #[case("https://hidden.s3.us-west-2.amazonaws.com")]
    fn test_hidden_endpoint_is_not_visible(advertised_selector: RegexSelector, #[case] uri: &str) {
        assert!(!advertised_selector.filter_top_level_address(uri));
    }

    #[rstest]
    #[case("https://unlisted.s3.us-west-2.amazonaws.com/object")]
    #[case("https://hidden.s3.us-west-2.amazonaws.com/object")]
    fn test_hidden_endpoint_is_routed(advertised_selector: RegexSelector, #[case] uri: &str) {
        assert!(advertised_selector.match_resource_address(uri));
    }

    #[rstest]
    fn test_only_shown_routes_are_advertised(advertised_selector: RegexSelector) {
        assert_eq!(
            advertised_selector.iter_wildcard_routes().collect::<Vec<_>>(),
            [
                "https://*.s3.*.amazonaws.com/**",
                "https://*.s3-*.amazonaws.com/**",
                "https://shown.s3.us-west-2.amazonaws.com/**",
                "https://unlisted.s3.us-west-2.amazonaws.com/**",
            ]
        );
    }

    #[test]
    fn test_all_hidden_endpoints_advertise_no_routes() {
        let selector = RegexSelector::try_from_s3_endpoints(
            [RegexSelectorRoute {
                endpoint: EndpointUrl::parse_s3_endpoint_url("https://hidden.s3.us-west-2.amazonaws.com").unwrap(),
                show_tla: false,
                show_route: false,
            }],
            false,
        )
        .unwrap();

        assert!(selector.iter_wildcard_routes().next().is_none());
    }

    #[rstest]
    #[case(false)]
    #[case(true)]
    fn test_overlapping_visible_route_matches(#[case] reverse_rules: bool) {
        let selector = overlapping_selector(reverse_rules);

        assert!(selector.filter_top_level_address("https://storage.example/bucket"));
    }

    #[rstest]
    #[case(false)]
    #[case(true)]
    fn test_overlapping_hidden_route_does_not_match(#[case] reverse_rules: bool) {
        let selector = overlapping_selector(reverse_rules);

        assert!(!selector.filter_top_level_address("https://storage.example"));
    }
}
