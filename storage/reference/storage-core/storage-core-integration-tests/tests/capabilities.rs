// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use futures::StreamExt;
use rstest::*;
use storage_core::traits::capabilities::v1alpha::*;
use storage_core::traits::filefolder::v1alpha::{FolderAddress, ListStatRequest};
use storage_core_integration_tests::fixture::*;
use storage_core_integration_tests::names::{test_names, TestNames};
use storage_core_integration_tests::storage_fixture;
use storage_core_test::service::{ApiVersion, Interface};
use tracing::debug;

#[rstest]
#[test_log::test(tokio::test)]
async fn capabilities(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure, Backend::Minio, Backend::GCPS3Compat)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
) {
    let mut client = storage_fixture
        .capabilities_client(backend, interface, api_version)
        .await;

    let request = ListServicesRequest {};
    let list_services_response = client.list_services(request).await.unwrap();

    // Convert response to a HashMap for easier comparison without ordering
    let actual_services: std::collections::HashMap<String, std::collections::HashSet<String>> = list_services_response
        .services
        .into_iter()
        .map(|service| {
            let versions: std::collections::HashSet<String> = service.service_versions.into_iter().collect();
            (service.service_name, versions)
        })
        .collect();

    // Expected services with their versions
    let expected_services: std::collections::HashMap<String, std::collections::HashSet<String>> = [
        (
            "fileobject".to_string(),
            ["v1alpha".to_string(), "v1beta".to_string()].into_iter().collect(),
        ),
        (
            "capabilities".to_string(),
            ["v1alpha".to_string(), "v1beta".to_string()].into_iter().collect(),
        ),
        (
            "filefolder".to_string(),
            ["v1alpha".to_string(), "v1beta".to_string()].into_iter().collect(),
        ),
        (
            "versioning".to_string(),
            ["v1alpha".to_string(), "v1beta".to_string()].into_iter().collect(),
        ),
        ("metadata".to_string(), ["v1alpha".to_string()].into_iter().collect()),
    ]
    .into_iter()
    .collect();

    // Verify we have the expected number of services
    assert_eq!(
        actual_services.len(),
        expected_services.len(),
        "Expected {} services, but got {}",
        expected_services.len(),
        actual_services.len()
    );

    // Verify each expected service exists with correct versions
    for (expected_name, expected_versions) in &expected_services {
        let actual_versions = actual_services
            .get(expected_name.as_str())
            .unwrap_or_else(|| panic!("Service '{}' not found in response", expected_name));

        assert_eq!(
            actual_versions, expected_versions,
            "Service '{}' has incorrect versions. Expected: {:?}, Got: {:?}",
            expected_name, expected_versions, actual_versions
        );
    }

    // Verify no unexpected services are present
    for actual_name in actual_services.keys() {
        assert!(
            expected_services.contains_key(actual_name),
            "Unexpected service '{}' found in response",
            actual_name
        );
    }
}

#[rstest]
#[test_log::test(tokio::test)]
async fn top_level_addresses(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure, Backend::Minio, Backend::GCPS3Compat)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
) {
    let mut client = storage_fixture
        .capabilities_client(backend, interface, api_version)
        .await;

    let request = ListTopLevelAddressesRequest {};
    let list_top_level_addresses_response = client
        .list_top_level_addresses(request)
        .await
        .unwrap()
        .items
        .into_iter()
        .map(|item| item.top_level_address)
        .collect::<Vec<String>>();

    let minio_address = storage_fixture.minio_address.clone();
    let names = test_names();
    let expected_top_level_addresses = match backend {
        Backend::S3 | Backend::Minio | Backend::GCPS3Compat => vec![
            TestNames::s3_url(&names.s3_bucket),
            TestNames::s3_url(&names.s3_versioned_bucket),
            format!("http://{}/{}", minio_address, names.s3_bucket),
            format!("http://{}/{}", minio_address, names.s3_public_bucket),
            format!("http://{}/{}", minio_address, names.s3_versioned_bucket),
            format!("http://{}/{}", minio_address, names.s3_empty_bucket),
            TestNames::gcs_url(&names.s3_bucket),
            TestNames::gcs_url(&names.s3_empty_bucket),
            "omniverse://myhost.mydomain".to_string(),
        ],
        Backend::Azure => vec![
            TestNames::azure_url(&names.azure_private_account),
            TestNames::azure_url(&names.azure_versioned_account),
            "omniverse://myhost.mydomain".to_string(),
        ],
    };

    // Assert that each response address starts with one of the expected addresses
    for response_addr in &list_top_level_addresses_response {
        debug!("response_addr: {}", response_addr);
        assert!(
            expected_top_level_addresses
                .iter()
                .any(|expected| response_addr.starts_with(expected)),
            "Response address '{response_addr}' doesn't start with any expected address"
        );
    }

    // Each top level address must be passable to list_stat
    let mut file_folder_client = storage_fixture
        .file_folder_client(backend, Cache::Disabled, interface, api_version)
        .await;
    let minio_enabled = StorageFixture::check_minio_server(minio_address.clone());
    for top_level_address in &list_top_level_addresses_response {
        // Minio might not be running, so skip the test if it's not
        if top_level_address.starts_with(&format!("http://{minio_address}")) && !minio_enabled {
            continue;
        }
        // The empty buckets have nothing to list, so the "got some items" check below does not apply.
        if top_level_address
            .trim_end_matches('/')
            .ends_with(&format!("/{}", names.s3_empty_bucket))
        {
            continue;
        }
        let request = ListStatRequest {
            include_native_metadata: false,
            folder: Some(FolderAddress {
                uri: top_level_address.clone(),
            }),
        };
        let mut response_stream = file_folder_client.list_stat(request).await.unwrap();
        let mut responses = Vec::new();

        // Process each item in the stream, failing on any errors
        while let Some(result) = response_stream.next().await {
            responses.push(result.unwrap_or_else(|e| panic!("ListStat stream error for {top_level_address}: {e:?}")));
        }

        // Don't really care what the actual results are, just that we got some
        let total_items: usize = responses
            .iter()
            .map(|response| response.subfolder_addresses.len() + response.entries.len())
            .sum();
        assert!(
            total_items > 0,
            "ListStat returned no responses for {top_level_address}"
        );
    }
}

#[rstest]
#[test_log::test(tokio::test)]
async fn routes(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure, Backend::Minio, Backend::GCPS3Compat)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use storage_core::traits::capabilities::v1alpha::route::RouteType;

    let mut client = storage_fixture
        .capabilities_client(backend, interface, ApiVersion::V1Alpha)
        .await;

    let request = ListRoutesRequest {};
    let list_routes_response = client.list_routes(request).await.unwrap();

    // The response includes both generic wildcard patterns and routes derived from configured
    // secret hostnames. Verify that the expected generic wildcard patterns are present.
    let expected_wildcard_routes = match backend {
        Backend::S3 | Backend::Minio | Backend::GCPS3Compat => vec![
            Route {
                route_type: Some(RouteType::WildcardPattern("https://*.s3.*.amazonaws.com/**".into())),
            },
            Route {
                route_type: Some(RouteType::WildcardPattern("https://*.s3-*.amazonaws.com/**".into())),
            },
        ],
        Backend::Azure => vec![Route {
            route_type: Some(RouteType::WildcardPattern("https://*.blob.core.windows.net/**".into())),
        }],
    };

    let minio_address = storage_fixture.minio_address.clone();
    let names = test_names();
    let expected_secret_routes: Vec<Route> = match backend {
        Backend::S3 | Backend::Minio | Backend::GCPS3Compat => vec![
            format!("{}/**", TestNames::s3_url(&names.s3_bucket)),
            format!("{}/**", TestNames::s3_url(&names.s3_versioned_bucket)),
            format!("http://{minio_address}/{}/**", names.s3_bucket),
            format!("http://{minio_address}/{}/**", names.s3_public_bucket),
            format!("http://{minio_address}/{}/**", names.s3_versioned_bucket),
            format!("http://{minio_address}/{}/**", names.s3_empty_bucket),
            format!("{}/**", TestNames::gcs_url(&names.s3_bucket)),
            format!("{}/**", TestNames::gcs_url(&names.s3_empty_bucket)),
        ],
        Backend::Azure => vec![
            format!("{}/**", TestNames::azure_url(&names.azure_private_account)),
            format!("{}/**", TestNames::azure_url(&names.azure_versioned_account)),
        ],
    }
    .into_iter()
    .map(|s| Route {
        route_type: Some(RouteType::WildcardPattern(s)),
    })
    .collect();

    let items = &list_routes_response.items;
    assert!(!items.is_empty(), "Expected at least one route, got none");
    for expected in &expected_wildcard_routes {
        assert!(
            items.contains(expected),
            "Expected wildcard route {expected:?} not found in response: {items:?}"
        );
    }
    for expected in &expected_secret_routes {
        assert!(
            items.contains(expected),
            "Expected secret-derived route {expected:?} not found in response: {items:?}"
        );
    }

    // All routes should have the wildcard pattern variant and end with /**
    for route in items {
        match &route.route_type {
            Some(RouteType::WildcardPattern(pattern)) => {
                assert!(pattern.ends_with("/**"), "Route pattern should end with /**: {pattern}");
            },
            None => panic!("Route should have a route_type"),
        }
    }
}
