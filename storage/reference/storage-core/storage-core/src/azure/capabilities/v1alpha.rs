// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use tracing::instrument;

use crate::{
    azure::service::Storage,
    traits::capabilities::v1alpha::{
        route::RouteType, Capabilities as CapabilitiesV1Alpha, ListRoutesRequest, ListRoutesResponse,
        ListTopLevelAddressesRequest, ListTopLevelAddressesResponse, Route, TopLevelAddressEntry,
    },
    traits::{Request, Result},
};

use storage_core_util::additional_headers::AdditionalHeaders;

#[tonic::async_trait]
impl CapabilitiesV1Alpha for Storage {
    fn inner(&self) -> Option<&dyn CapabilitiesV1Alpha> {
        None
    }

    #[instrument(name = "azure::list_top_level_addresses", skip_all)]
    async fn list_top_level_addresses(
        &self,
        mut request: Request<ListTopLevelAddressesRequest>,
    ) -> Result<ListTopLevelAddressesResponse> {
        let additional_headers = request.extensions_mut().remove::<AdditionalHeaders>();

        let addresses = self.list_top_level_addresses_internal(additional_headers).await?;

        Ok(ListTopLevelAddressesResponse {
            items: addresses
                .into_iter()
                .map(|address| TopLevelAddressEntry {
                    top_level_address: address,
                })
                .collect(),
        })
    }

    #[instrument(name = "azure::list_routes", skip_all)]
    async fn list_routes(&self, _request: Request<ListRoutesRequest>) -> Result<ListRoutesResponse> {
        // When unconfigured endpoints are allowed, start with the broad wildcard that covers
        // all Azure Blob Storage accounts.  When blocked, omit the wildcard so clients only
        // discover the explicitly configured storage account routes (added below).
        let mut items = if self.config.allow_unconfigured_endpoints {
            vec![Route {
                route_type: Some(RouteType::WildcardPattern("https://*.blob.core.windows.net/**".into())),
            }]
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
    use crate::azure::secrets::{ClientCredentials, Secrets};
    use crate::azure::service::Config;

    const AZURE_WILDCARD: &str = "https://*.blob.core.windows.net/**";
    const CONFIGURED_HOST: &str = "configured.blob.core.windows.net";

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
                storage_key: Some("key".to_string()),
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

    /// Default (permissive) config advertises the broad wildcard route alongside
    /// the configured host — this is the invariant the integration `routes` test
    /// relies on.
    #[tokio::test]
    async fn list_routes_advertises_wildcard_when_unconfigured_allowed() {
        let storage = storage_with(true);

        let patterns = route_patterns(
            storage
                .list_routes(Request::new(ListRoutesRequest {}))
                .await
                .expect("list_routes"),
        );

        assert!(patterns.iter().any(|p| p == AZURE_WILDCARD));
        assert!(patterns.iter().any(|p| p.contains(CONFIGURED_HOST)));
    }

    /// Strict mode drops the wildcard so clients can't discover unconfigured
    /// accounts, but the configured host is still advertised — keeping accepted
    /// endpoints and advertised routes consistent.
    #[tokio::test]
    async fn list_routes_omits_wildcard_but_keeps_configured_host_in_strict_mode() {
        let storage = storage_with(false);

        let patterns = route_patterns(
            storage
                .list_routes(Request::new(ListRoutesRequest {}))
                .await
                .expect("list_routes"),
        );

        assert!(!patterns.iter().any(|p| p == AZURE_WILDCARD));
        assert!(patterns.iter().any(|p| p.contains(CONFIGURED_HOST)));
    }
}
