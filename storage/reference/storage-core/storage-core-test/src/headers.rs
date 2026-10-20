// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;

use storage_core::traits::fileobject::v1alpha::Header;

/// Configuration for request headers and validation
#[derive(Debug, Clone, Default)]
pub struct HeadersConfig {
    /// Headers to add to all requests made by the clients.
    pub headers: HashMap<String, String>,
    /// Headers that should be validated in responses from the server.
    pub validation_headers: HashMap<String, String>,
}

impl HeadersConfig {
    /// Create a new HeadersConfig with the specified headers and validation headers
    pub fn new(headers: HashMap<String, String>, validation_headers: HashMap<String, String>) -> Self {
        Self {
            headers,
            validation_headers,
        }
    }

    /// Get all headers as an iterator
    pub fn get_headers(&self) -> impl Iterator<Item = (&String, &String)> {
        self.headers.iter()
    }

    /// Validate the headers returned by the server
    pub fn validate_headers(&self, headers: &[Header]) {
        tracing::debug!(?headers);

        // Validate user-specified headers
        for (expected_name, expected_value) in &self.validation_headers {
            assert!(
                headers
                    .iter()
                    .any(|h| h.name == *expected_name && h.value == *expected_value),
                "Expected header '{}' with value '{}' not found in response headers",
                expected_name,
                expected_value
            );
        }
    }
}
