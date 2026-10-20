// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

pub mod grpc;
pub mod rest;

use crate::service_wrapper_module;

service_wrapper_module! {
    service_name: "capabilities",
    version: "v1beta",
    struct_name: Capabilities,
    field_name: capabilities,
    trait_path: crate::traits::capabilities::v1beta::Capabilities as CapabilitiesTrait,
}

use crate::service_registry::SERVICE_REGISTRY;
use crate::traits::capabilities::v1beta::{ListServicesResponse, ServiceEntry};
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
