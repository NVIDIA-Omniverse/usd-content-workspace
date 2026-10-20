// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

pub mod grpc;
pub mod rest;

use crate::service_wrapper_module;

service_wrapper_module! {
    service_name: "capabilities",
    version: "v1alpha",
    struct_name: Capabilities,
    field_name: capabilities,
    trait_path: crate::traits::capabilities::v1alpha::Capabilities as CapabilitiesTrait,
}

use crate::service_registry::SERVICE_REGISTRY;
use crate::traits::capabilities::v1alpha::{ListServicesResponse, ServiceEntry};
use std::collections::HashMap;

pub fn list_service_response() -> ListServicesResponse {
    let service_infos = SERVICE_REGISTRY.list_services();

    // Group services by name and collect their versions
    let mut service_map: HashMap<String, Vec<String>> = HashMap::new();

    for info in service_infos {
        service_map.entry(info.name).or_default().push(info.version);
    }

    // Convert to ServiceEntry format
    let services: Vec<ServiceEntry> = service_map
        .into_iter()
        .map(|(service_name, service_versions)| ServiceEntry {
            service_name,
            service_versions,
        })
        .collect();

    ListServicesResponse { services }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service_registry::ServiceInfo;
    use serial_test::serial;

    #[test]
    #[serial]
    fn test_list_service_response_empty_registry() {
        // Clear the registry to start with a clean state
        SERVICE_REGISTRY.clear();

        let response = list_service_response();
        assert!(response.services.is_empty());
    }

    #[test]
    #[serial]
    fn test_list_service_response_single_service_single_version() {
        // Clear the registry to start with a clean state
        SERVICE_REGISTRY.clear();

        // Register a single service with one version
        SERVICE_REGISTRY.register(ServiceInfo {
            name: "test_service".to_string(),
            version: "v1alpha".to_string(),
        });

        let response = list_service_response();
        assert_eq!(response.services.len(), 1);

        let service = &response.services[0];
        assert_eq!(service.service_name, "test_service");
        assert_eq!(service.service_versions, vec!["v1alpha"]);
    }

    #[test]
    #[serial]
    fn test_list_service_response_single_service_multiple_versions() {
        // Clear the registry to start with a clean state
        SERVICE_REGISTRY.clear();

        // Register a single service with multiple versions
        SERVICE_REGISTRY.register(ServiceInfo {
            name: "fileobject".to_string(),
            version: "v1alpha".to_string(),
        });
        SERVICE_REGISTRY.register(ServiceInfo {
            name: "fileobject".to_string(),
            version: "v1beta".to_string(),
        });

        let response = list_service_response();
        assert_eq!(response.services.len(), 1);

        let service = &response.services[0];
        assert_eq!(service.service_name, "fileobject");
        assert_eq!(service.service_versions.len(), 2);
        assert!(service.service_versions.contains(&"v1alpha".to_string()));
        assert!(service.service_versions.contains(&"v1beta".to_string()));
    }

    #[test]
    #[serial]
    fn test_list_service_response_multiple_services() {
        // Clear the registry to start with a clean state
        SERVICE_REGISTRY.clear();

        // Register multiple services with different versions
        SERVICE_REGISTRY.register(ServiceInfo {
            name: "fileobject".to_string(),
            version: "v1alpha".to_string(),
        });
        SERVICE_REGISTRY.register(ServiceInfo {
            name: "fileobject".to_string(),
            version: "v1beta".to_string(),
        });
        SERVICE_REGISTRY.register(ServiceInfo {
            name: "capabilities".to_string(),
            version: "v1alpha".to_string(),
        });
        SERVICE_REGISTRY.register(ServiceInfo {
            name: "filefolder".to_string(),
            version: "v1alpha".to_string(),
        });
        SERVICE_REGISTRY.register(ServiceInfo {
            name: "versioning".to_string(),
            version: "v1alpha".to_string(),
        });

        let response = list_service_response();
        assert_eq!(response.services.len(), 4);

        // Find each service and verify its versions
        let services_by_name: std::collections::HashMap<String, &ServiceEntry> =
            response.services.iter().map(|s| (s.service_name.clone(), s)).collect();

        // Check fileobject service (should have 2 versions)
        let fileobject = services_by_name.get("fileobject").unwrap();
        assert_eq!(fileobject.service_versions.len(), 2);
        assert!(fileobject.service_versions.contains(&"v1alpha".to_string()));
        assert!(fileobject.service_versions.contains(&"v1beta".to_string()));

        // Check capabilities service (should have 1 version)
        let capabilities = services_by_name.get("capabilities").unwrap();
        assert_eq!(capabilities.service_versions, vec!["v1alpha"]);

        // Check filefolder service (should have 1 version)
        let filefolder = services_by_name.get("filefolder").unwrap();
        assert_eq!(filefolder.service_versions, vec!["v1alpha"]);

        // Check versioning service (should have 1 version)
        let versioning = services_by_name.get("versioning").unwrap();
        assert_eq!(versioning.service_versions, vec!["v1alpha"]);
    }

    #[test]
    #[serial]
    fn test_list_service_response_duplicate_versions_ignored() {
        // Clear the registry to start with a clean state
        SERVICE_REGISTRY.clear();

        // Register the same service with the same version multiple times
        SERVICE_REGISTRY.register(ServiceInfo {
            name: "test_service".to_string(),
            version: "v1alpha".to_string(),
        });
        SERVICE_REGISTRY.register(ServiceInfo {
            name: "test_service".to_string(),
            version: "v1alpha".to_string(),
        });
        SERVICE_REGISTRY.register(ServiceInfo {
            name: "test_service".to_string(),
            version: "v1alpha".to_string(),
        });

        let response = list_service_response();
        assert_eq!(response.services.len(), 1);

        let service = &response.services[0];
        assert_eq!(service.service_name, "test_service");
        // Note: The current implementation doesn't deduplicate versions,
        // so we expect 3 identical versions
        assert_eq!(service.service_versions.len(), 3);
        assert!(service.service_versions.iter().all(|v| v == "v1alpha"));
    }

    #[test]
    #[serial]
    fn test_list_service_response_mixed_case_service_names() {
        // Clear the registry to start with a clean state
        SERVICE_REGISTRY.clear();

        // Register services with different case variations
        SERVICE_REGISTRY.register(ServiceInfo {
            name: "FileObject".to_string(),
            version: "v1alpha".to_string(),
        });
        SERVICE_REGISTRY.register(ServiceInfo {
            name: "fileobject".to_string(),
            version: "v1beta".to_string(),
        });
        SERVICE_REGISTRY.register(ServiceInfo {
            name: "FILEOBJECT".to_string(),
            version: "v1beta".to_string(),
        });

        let response = list_service_response();
        // Should create 3 separate services since names are case-sensitive
        assert_eq!(response.services.len(), 3);

        let service_names: std::collections::HashSet<String> =
            response.services.iter().map(|s| s.service_name.clone()).collect();

        assert!(service_names.contains("FileObject"));
        assert!(service_names.contains("fileobject"));
        assert!(service_names.contains("FILEOBJECT"));
    }

    #[test]
    #[serial]
    fn test_list_service_response_large_number_of_services() {
        // Clear the registry to start with a clean state
        SERVICE_REGISTRY.clear();

        // Register many services to test performance and correctness
        for i in 0..100 {
            SERVICE_REGISTRY.register(ServiceInfo {
                name: format!("service_{}", i),
                version: "v1alpha".to_string(),
            });
        }

        // Add some services with multiple versions
        for i in 0..10 {
            SERVICE_REGISTRY.register(ServiceInfo {
                name: format!("multi_version_service_{}", i),
                version: "v1alpha".to_string(),
            });
            SERVICE_REGISTRY.register(ServiceInfo {
                name: format!("multi_version_service_{}", i),
                version: "v1beta".to_string(),
            });
            SERVICE_REGISTRY.register(ServiceInfo {
                name: format!("multi_version_service_{}", i),
                version: "v1gamma".to_string(),
            });
        }

        let response = list_service_response();

        // Verify all expected services are present
        let service_names: std::collections::HashSet<String> =
            response.services.iter().map(|s| s.service_name.clone()).collect();

        // Check that all our test services are present
        for i in 0..100 {
            assert!(
                service_names.contains(&format!("service_{}", i)),
                "Missing service_{}",
                i
            );
        }

        for i in 0..10 {
            assert!(
                service_names.contains(&format!("multi_version_service_{}", i)),
                "Missing multi_version_service_{}",
                i
            );
        }

        // Verify the multi-version services have the correct number of versions
        for i in 0..10 {
            let service_name = format!("multi_version_service_{}", i);
            let service = response
                .services
                .iter()
                .find(|s| s.service_name == service_name)
                .unwrap_or_else(|| panic!("Service {service_name} not found"));
            assert_eq!(
                service.service_versions.len(),
                3,
                "Service {} should have 3 versions",
                service_name
            );
            assert!(service.service_versions.contains(&"v1alpha".to_string()));
            assert!(service.service_versions.contains(&"v1beta".to_string()));
            assert!(service.service_versions.contains(&"v1gamma".to_string()));
        }

        // Verify single-version services have exactly one version
        for i in 0..100 {
            let service_name = format!("service_{}", i);
            let service = response
                .services
                .iter()
                .find(|s| s.service_name == service_name)
                .unwrap_or_else(|| panic!("Service {service_name} not found"));
            assert_eq!(
                service.service_versions,
                vec!["v1alpha"],
                "Service {} should have exactly one version",
                service_name
            );
        }
    }
}
