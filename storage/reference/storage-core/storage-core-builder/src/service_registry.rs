// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::{LazyLock, RwLock};

#[derive(Clone, Debug)]
pub struct ServiceInfo {
    pub name: String,
    pub version: String,
}

pub trait ServiceInfoProvider {
    /// The service info advertised in the storage capabilities `ListServices`.
    ///
    /// Returns `None` for interfaces served alongside the storage API that are not part of the
    /// storage capabilities surface. Such foreign interfaces have nothing to do with the storage
    /// and cannot provide a meaningful `ServiceInfo`, so they opt out of `ListServices` by
    /// returning `None` (the default).
    fn service_info(&self) -> Option<ServiceInfo> {
        None
    }
}

/// A global registry for tracking registered services.
///
/// This registry allows services to be registered and queried across the application.
/// It's used primarily by the capabilities service to implement the `list_services` method.
pub struct ServiceRegistry {
    services: RwLock<Vec<ServiceInfo>>,
}

impl ServiceRegistry {
    /// Creates a new service registry.
    pub fn new() -> Self {
        Self {
            services: RwLock::new(Vec::new()),
        }
    }

    /// Registers a service in the global registry.
    ///
    /// # Parameters
    ///
    /// * `service_info` - The service information to register
    pub fn register(&self, service_info: ServiceInfo) {
        if let Ok(mut services) = self.services.write() {
            services.push(service_info);
        }
    }

    /// Lists all registered services.
    ///
    /// # Returns
    ///
    /// Returns a vector of all registered service information.
    pub fn list_services(&self) -> Vec<ServiceInfo> {
        self.services
            .read()
            .map(|services| services.clone())
            .unwrap_or_default()
    }

    /// Clears all registered services.
    ///
    /// This is primarily useful for testing.
    pub fn clear(&self) {
        if let Ok(mut services) = self.services.write() {
            services.clear();
        }
    }
}

impl Default for ServiceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Global service registry instance.
///
/// This is used throughout the application to register and query services.
pub static SERVICE_REGISTRY: LazyLock<ServiceRegistry> = LazyLock::new(ServiceRegistry::new);
