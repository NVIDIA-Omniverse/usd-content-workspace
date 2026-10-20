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

use bit_vec::BitVec;
use storage_core::traits::{capabilities::v1alpha::*, Request, Result};
use storage_service_permissions::types::ResourceType;
use tonic::async_trait;

use crate::backend::permissions::{action::Action, error::Error, request::RequestExt, Permissions};

#[async_trait]
impl<S> Capabilities for Permissions<S>
where
    S: Capabilities,
{
    fn inner(&self) -> Option<&dyn Capabilities> {
        None
    }

    async fn list_routes(&self, request: Request<ListRoutesRequest>) -> Result<ListRoutesResponse> {
        let permission_metadata = request.get_permission_request_metadata(&self.inner.forward_headers)?;
        let mut response = self.next.list_routes(request).await?;

        // Routes follow the same rule as top level addresses: an evaluated route is
        // listed when the caller is explicitly allowed on it. A route is retained
        // without evaluation when its endpoint bypasses permission enforcement or when
        // the pattern has inner wildcards and cannot name a resource.
        let mut batch = self.batch();
        let unevaluated: BitVec = response
            .items
            .iter()
            .map(|item| {
                let Some(prefix) = get_concrete_route_prefix(item) else {
                    return Ok(true);
                };

                if self.inner.filter.is_route_excluded(prefix)? {
                    return Ok(true);
                }

                batch
                    .add_resource_hierarchically(prefix, Some(ResourceType::Folder), Action::ListRoutes)
                    .map_err(Error::response_resource_url_invalid)?;

                Ok(false)
            })
            .collect::<Result<_>>()?;

        let mut retain_iter = batch
            .execute(&permission_metadata)
            .await?
            .into_retain_iter_with_deny_on_missing();
        let mut unevaluated = unevaluated.into_iter();
        response
            .items
            .retain(|_| unevaluated.next() == Some(true) || retain_iter.next() == Some(true));

        Ok(response)
    }

    async fn list_top_level_addresses(
        &self,
        request: Request<ListTopLevelAddressesRequest>,
    ) -> Result<ListTopLevelAddressesResponse> {
        let permission_metadata = request.get_permission_request_metadata(&self.inner.forward_headers)?;
        let mut response = self.next.list_top_level_addresses(request).await?;

        // A bypassed endpoint's top-level address is always listed, even for a
        // principal with no ACLs, so only the enforced items enter the batch.
        let mut batch = self.batch();
        let bypassed: BitVec = response
            .items
            .iter()
            .map(|item| {
                let bypassed = self.is_permission_check_excluded(&item.top_level_address, ResourceType::Folder)?;
                if !bypassed {
                    batch
                        .add_resource_hierarchically(
                            &item.top_level_address,
                            Some(ResourceType::Folder),
                            Action::ListTopLevelAddresses,
                        )
                        .map_err(Error::response_resource_url_invalid)?;
                }

                Ok(bypassed)
            })
            .collect::<Result<_>>()?;

        let mut retain_iter = batch
            .execute(&permission_metadata)
            .await?
            .into_retain_iter_with_deny_on_missing();
        let mut bypassed = bypassed.into_iter();
        response
            .items
            .retain(|_| bypassed.next() == Some(true) || retain_iter.next() == Some(true));

        Ok(response)
    }
}

/// The route patterns that legitimately carry an inner wildcard: the AWS S3 and Azure Blob
/// "everything" templates a router advertises for a whole endpoint family (see
/// `RegexSelector::try_from_s3_endpoints` / `try_from_azure_endpoints` in
/// `backend::router::selector`). They match a family of hosts rather than naming a single resource,
/// so [`get_concrete_route_prefix`] keeps them unevaluated. Every other route the router advertises
/// is a concrete `<endpoint-prefix>/**`.
///
/// This is the closed set a `debug_assert!` checks against, so a new wildcard route shape that the
/// trailing-`**` suffix rule would silently misclassify is caught early in debug builds and tests
/// rather than leaking an unintended visibility decision in production.
const WELL_KNOWN_WILDCARD_ROUTES: [&str; 3] = [
    "https://*.s3.*.amazonaws.com/**",
    "https://*.s3-*.amazonaws.com/**",
    "https://*.blob.core.windows.net/**",
];

/// Return the folder address a route pattern covers, unless the pattern has inner wildcards.
fn get_concrete_route_prefix(route: &Route) -> Option<&str> {
    let route::RouteType::WildcardPattern(pattern) = route.route_type.as_ref()?;

    // Every advertised route is `<endpoint-prefix>/**`. Strip the trailing `**`; a pattern that
    // then still carries a `*` matches a whole endpoint family and names no single resource.
    let Some(prefix) = pattern.strip_suffix("**").filter(|prefix| !prefix.contains('*')) else {
        // The only patterns that reach here are the well-known AWS/Azure everything-templates.
        // Assert that, so a new route shape this rule would keep unevaluated -- and thus hide from,
        // or wrongly surface to, a caller -- is caught early rather than silently.
        debug_assert!(WELL_KNOWN_WILDCARD_ROUTES.contains(&pattern.as_str()));

        return None;
    };

    Some(prefix)
}

#[cfg(test)]
mod tests {
    use assert_matches::assert_matches;

    use super::*;

    fn route(pattern: &str) -> Route {
        Route {
            route_type: Some(route::RouteType::WildcardPattern(pattern.to_owned())),
        }
    }

    // A concrete endpoint prefix names a folder, so it is returned for permission evaluation.
    #[test]
    fn concrete_endpoint_prefix_is_evaluated() {
        assert_matches!(
            get_concrete_route_prefix(&route("https://account.blob.core.windows.net/**")),
            Some("https://account.blob.core.windows.net/")
        );
        assert_matches!(
            get_concrete_route_prefix(&route("https://bucket.s3.us-east-1.amazonaws.com/**")),
            Some("https://bucket.s3.us-east-1.amazonaws.com/")
        );
    }

    // The AWS/Azure everything-templates match a host family rather than a single resource, so they
    // stay unevaluated -- and, being the well-known templates, they satisfy the debug assertion this
    // path carries (these run in a debug/test build, where `debug_assert!` is active).
    #[test]
    fn well_known_wildcard_routes_are_unevaluated() {
        assert_matches!(
            get_concrete_route_prefix(&route("https://*.s3.*.amazonaws.com/**")),
            None
        );
        assert_matches!(
            get_concrete_route_prefix(&route("https://*.s3-*.amazonaws.com/**")),
            None
        );
        assert_matches!(
            get_concrete_route_prefix(&route("https://*.blob.core.windows.net/**")),
            None
        );
    }

    // A route with no wildcard pattern names nothing to evaluate.
    #[test]
    fn missing_route_type_is_unevaluated() {
        assert_matches!(get_concrete_route_prefix(&Route { route_type: None }), None);
    }
}
