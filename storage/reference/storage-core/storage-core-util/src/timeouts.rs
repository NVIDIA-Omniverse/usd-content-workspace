// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Configuration for the timeouts.
///
/// The configuration key is `timeouts` when extracting from a Figment struct.
///
/// Example configuration:
/// ```toml
/// [timeouts]
/// connect = "60s"
/// read = "60s"
/// operation = "60s"
/// operation_attempt = "60s"
/// keepalive = "60s"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// The connect timeout is a limit on the amount of time it takes to initiate a socket connection.
    #[serde(default = "default_connect", with = "humantime_serde")]
    pub connect: Duration,

    /// The read timeout is the limit on the amount of time it takes to read the first byte of a response
    /// from the time the request is initiated.
    #[serde(default = "default_read", with = "humantime_serde")]
    pub read: Duration,

    /// An operation represents the full request/response lifecycle of a call to a service.
    /// The operation timeout is a limit on the total amount of time it takes for an operation to be fully serviced,
    /// including the time for all retries that may have been attempted for it.
    ///
    /// If you want to set a timeout on individual retry attempts, then see [Self::operation_attempt].
    #[serde(default = "default_operation", with = "humantime_serde")]
    pub operation: Duration,

    /// An operation represents the full request/response lifecycle of a call to a service.
    /// When retries are enabled, then this setting makes it possible to set a timeout for individual
    /// retry attempts (including the initial attempt) for an operation.
    ///
    /// If you want to set a timeout on the total time for an entire request including all of its retries,
    /// then see [Self::operation]. This only applies to S3 backend.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "humantime_serde")]
    pub operation_attempt: Option<Duration>,

    /// In HTTP connectors, used by backends, this is the timeout for receiving an acknowledgement of the keep-alive ping.
    /// If the ping is not acknowledged within the timeout, the connection will be closed.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "humantime_serde")]
    pub keepalive: Option<Duration>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            connect: default_connect(),
            read: default_read(),
            operation: default_operation(),
            operation_attempt: None,
            keepalive: None,
        }
    }
}

fn default_connect() -> Duration {
    Duration::from_secs(60)
}

fn default_read() -> Duration {
    Duration::from_secs(60)
}

fn default_operation() -> Duration {
    Duration::from_secs(120)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialize_with_defaults() {
        let config = Config::default();
        let json = serde_json::to_string(&config).unwrap();

        // Verify that JSON contains the default durations in human-readable format
        assert!(json.contains("connect"));
        assert!(json.contains("read"));
        assert!(json.contains("operation"));

        // Optional fields should be skipped when None
        assert!(!json.contains("operation_attempt"));
        assert!(!json.contains("keepalive"));
    }

    #[test]
    fn deserialize_with_defaults() {
        let json = r#"{
            "connect": "60s",
            "read": "60s",
            "operation": "120s"
        }"#;

        let config: Config = serde_json::from_str(json).unwrap();

        assert_eq!(config.connect, Duration::from_secs(60));
        assert_eq!(config.read, Duration::from_secs(60));
        assert_eq!(config.operation, Duration::from_secs(120));
        assert_eq!(config.operation_attempt, None);
        assert_eq!(config.keepalive, None);
    }

    #[test]
    fn serialize_with_all_fields() {
        let config = Config {
            connect: Duration::from_secs(30),
            read: Duration::from_secs(45),
            operation: Duration::from_secs(180),
            operation_attempt: Some(Duration::from_secs(20)),
            keepalive: Some(Duration::from_secs(90)),
        };

        let json = serde_json::to_string(&config).unwrap();

        // Verify all fields are present
        assert!(json.contains("connect"));
        assert!(json.contains("read"));
        assert!(json.contains("operation"));
        assert!(json.contains("operation_attempt"));
        assert!(json.contains("keepalive"));
    }

    #[test]
    fn deserialize_with_all_fields() {
        let json = r#"{
            "connect": "30s",
            "read": "45s",
            "operation": "3m",
            "operation_attempt": "20s",
            "keepalive": "90s"
        }"#;

        let config: Config = serde_json::from_str(json).unwrap();

        assert_eq!(config.connect, Duration::from_secs(30));
        assert_eq!(config.read, Duration::from_secs(45));
        assert_eq!(config.operation, Duration::from_secs(180));
        assert_eq!(config.operation_attempt, Some(Duration::from_secs(20)));
        assert_eq!(config.keepalive, Some(Duration::from_secs(90)));
    }

    #[test]
    fn roundtrip_with_defaults() {
        let original = Config::default();
        let json = serde_json::to_string(&original).unwrap();
        let deserialized: Config = serde_json::from_str(&json).unwrap();

        assert_eq!(original.connect, deserialized.connect);
        assert_eq!(original.read, deserialized.read);
        assert_eq!(original.operation, deserialized.operation);
        assert_eq!(original.operation_attempt, deserialized.operation_attempt);
        assert_eq!(original.keepalive, deserialized.keepalive);
    }

    #[test]
    fn roundtrip_with_all_fields() {
        let original = Config {
            connect: Duration::from_secs(15),
            read: Duration::from_secs(25),
            operation: Duration::from_secs(200),
            operation_attempt: Some(Duration::from_secs(10)),
            keepalive: Some(Duration::from_secs(75)),
        };

        let json = serde_json::to_string(&original).unwrap();
        let deserialized: Config = serde_json::from_str(&json).unwrap();

        assert_eq!(original.connect, deserialized.connect);
        assert_eq!(original.read, deserialized.read);
        assert_eq!(original.operation, deserialized.operation);
        assert_eq!(original.operation_attempt, deserialized.operation_attempt);
        assert_eq!(original.keepalive, deserialized.keepalive);
    }

    #[test]
    fn deserialize_various_duration_formats() {
        // Test with minutes and seconds
        let json = r#"{
            "connect": "1m30s",
            "read": "2m",
            "operation": "5m",
            "operation_attempt": "500ms",
            "keepalive": "1h"
        }"#;

        let config: Config = serde_json::from_str(json).unwrap();

        assert_eq!(config.connect, Duration::from_secs(90));
        assert_eq!(config.read, Duration::from_secs(120));
        assert_eq!(config.operation, Duration::from_secs(300));
        assert_eq!(config.operation_attempt, Some(Duration::from_millis(500)));
        assert_eq!(config.keepalive, Some(Duration::from_secs(3600)));
    }

    #[test]
    fn deserialize_missing_optional_fields() {
        let json = r#"{
            "connect": "30s",
            "read": "45s",
            "operation": "120s"
        }"#;

        let config: Config = serde_json::from_str(json).unwrap();

        assert_eq!(config.connect, Duration::from_secs(30));
        assert_eq!(config.read, Duration::from_secs(45));
        assert_eq!(config.operation, Duration::from_secs(120));
        assert_eq!(config.operation_attempt, None);
        assert_eq!(config.keepalive, None);
    }

    #[test]
    fn deserialize_empty_uses_defaults() {
        let json = "{}";

        let config: Config = serde_json::from_str(json).unwrap();

        // Should use default values
        assert_eq!(config.connect, default_connect());
        assert_eq!(config.read, default_read());
        assert_eq!(config.operation, default_operation());
        assert_eq!(config.operation_attempt, None);
        assert_eq!(config.keepalive, None);
    }
}
