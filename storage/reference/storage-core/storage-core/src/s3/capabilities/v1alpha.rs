// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use tracing::instrument;

use crate::{
    traits::capabilities::v1alpha::{
        route::RouteType, Capabilities, ListRoutesRequest, ListRoutesResponse, ListTopLevelAddressesRequest,
        ListTopLevelAddressesResponse, Route, TopLevelAddressEntry,
    },
    traits::{async_trait, Request, Result},
};

use super::super::service::Storage;

#[async_trait]
impl Capabilities for Storage {
    fn inner(&self) -> Option<&dyn Capabilities> {
        None
    }

    #[instrument(name = "s3::list_top_level_addresses", skip_all)]
    async fn list_top_level_addresses(
        &self,
        _request: Request<ListTopLevelAddressesRequest>,
    ) -> Result<ListTopLevelAddressesResponse> {
        // Gather any configured hosts from the config and return them as top level addresses.
        Ok(ListTopLevelAddressesResponse {
            items: self
                .secrets
                .read()
                .await
                .as_ref()
                .map(|object| {
                    object
                        .keys()
                        .map(|key| TopLevelAddressEntry {
                            top_level_address: key.to_string(),
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
        })
    }

    #[instrument(name = "s3::list_routes", skip_all)]
    async fn list_routes(&self, _request: Request<ListRoutesRequest>) -> Result<ListRoutesResponse> {
        // When unconfigured endpoints are allowed, start with the broad wildcards that cover
        // all AWS S3 buckets.  When blocked, omit those wildcards so clients only discover
        // the explicitly configured endpoint routes (added below).
        let mut items = if self.config.allow_unconfigured_endpoints {
            vec![
                Route {
                    route_type: Some(RouteType::WildcardPattern("https://*.s3.*.amazonaws.com/**".into())),
                },
                Route {
                    route_type: Some(RouteType::WildcardPattern("https://*.s3-*.amazonaws.com/**".into())),
                },
            ]
        } else {
            vec![]
        };

        // Always advertise routes for any configured secret hostnames so that
        // clients can reach them regardless of the allow_unconfigured_endpoints setting.
        let secrets_guard = self.secrets.read().await;
        if let Some(secrets) = &*secrets_guard {
            for key in secrets.keys() {
                let pattern = format!("{}/**", key);
                items.push(Route {
                    route_type: Some(RouteType::WildcardPattern(pattern)),
                });
            }
        }

        Ok(ListRoutesResponse { items })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::s3::secrets::{ClientCredentials, Secrets};
    use crate::s3::service::Config;

    const S3_WILDCARDS: [&str; 2] = ["https://*.s3.*.amazonaws.com/**", "https://*.s3-*.amazonaws.com/**"];
    const CONFIGURED_HOST: &str = "configured.s3.us-east-1.amazonaws.com";

    fn route_patterns(response: ListRoutesResponse) -> Vec<String> {
        response
            .items
            .into_iter()
            .filter_map(|route| match route.route_type {
                Some(RouteType::WildcardPattern(pattern)) => Some(pattern),
                _ => None,
            })
            .collect()
    }

    fn storage_with(allow_unconfigured_endpoints: bool) -> Storage {
        let secrets = Secrets(HashMap::from([(
            format!("https://{CONFIGURED_HOST}"),
            ClientCredentials {
                access_key_id: Some("abc".to_string()),
                secret_access_key: Some("xyz".to_string()),
                ..Default::default()
            },
        )]));
        Storage::new(
            Config {
                allow_unconfigured_endpoints,
                ..Config::default()
            },
            Some(secrets),
        )
        .expect("storage construction should succeed")
    }

    /// Default (permissive) config advertises the broad S3 wildcards alongside the
    /// configured host — the invariant the integration `routes` test relies on.
    #[tokio::test]
    async fn list_routes_advertises_wildcards_when_unconfigured_allowed() {
        let storage = storage_with(true);

        let patterns = route_patterns(
            storage
                .list_routes(Request::new(ListRoutesRequest {}))
                .await
                .expect("list_routes"),
        );

        for wildcard in S3_WILDCARDS {
            assert!(patterns.iter().any(|p| p == wildcard), "missing wildcard {wildcard}");
        }
        assert!(patterns.iter().any(|p| p.contains(CONFIGURED_HOST)));
    }

    /// Strict mode drops the wildcards so clients can't discover unconfigured
    /// buckets, but the configured host is still advertised.
    #[tokio::test]
    async fn list_routes_omits_wildcards_but_keeps_configured_host_in_strict_mode() {
        let storage = storage_with(false);

        let patterns = route_patterns(
            storage
                .list_routes(Request::new(ListRoutesRequest {}))
                .await
                .expect("list_routes"),
        );

        for wildcard in S3_WILDCARDS {
            assert!(
                !patterns.iter().any(|p| p == wildcard),
                "unexpected wildcard {wildcard}"
            );
        }
        assert!(patterns.iter().any(|p| p.contains(CONFIGURED_HOST)));
    }
}
