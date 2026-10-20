// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Startup validation for observability configuration (SRD 3.11).
//!
//! Called at the start of [`try_init_observability`](super::try_init_observability) to validate
//! all logging-related environment variables and output sinks before the subscriber is installed.
//! If any configuration is invalid, the process terminates (via the existing panic in
//! `init_observability`).

use std::{env, fmt, io::Write};

/// Errors from observability configuration validation.
#[derive(Debug)]
pub enum ConfigValidationError {
    /// An environment variable has an invalid value that cannot be parsed.
    InvalidEnvVar {
        var: &'static str,
        value: String,
        reason: String,
    },
    /// stdout is not writable.
    StdoutNotWritable(std::io::Error),
    /// stderr is not writable.
    StderrNotWritable(std::io::Error),
}

impl fmt::Display for ConfigValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidEnvVar { var, value, reason } => {
                write!(f, "invalid value for {var}={value:?}: {reason}")
            },
            Self::StdoutNotWritable(e) => write!(f, "stdout is not writable: {e}"),
            Self::StderrNotWritable(e) => write!(f, "stderr is not writable: {e}"),
        }
    }
}

impl std::error::Error for ConfigValidationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::StdoutNotWritable(e) | Self::StderrNotWritable(e) => Some(e),
            Self::InvalidEnvVar { .. } => None,
        }
    }
}

/// Validate all logging-related configuration.
///
/// Returns `Ok(())` if all configuration is valid or unset (defaults will be used).
/// Returns `Err` if any set configuration value is invalid — the process should terminate.
pub fn validate_observability_config() -> Result<(), ConfigValidationError> {
    validate_env_vars()?;
    validate_output_sinks()?;
    warn_on_bad_otlp_endpoints();
    Ok(())
}

/// Validate that all `LOGS_*` and rate-limit env vars are parseable (if set).
fn validate_env_vars() -> Result<(), ConfigValidationError> {
    validate_env_parse::<usize>("LOGS_RATE_LIMIT_COUNT")?;
    validate_env_positive_int("LOGS_RATE_LIMIT_COUNT")?;
    validate_env_parse::<humantime::Duration>("LOGS_RATE_LIMIT_INTERVAL")?;
    validate_env_positive_duration("LOGS_RATE_LIMIT_INTERVAL")?;
    validate_env_parse::<humantime::Duration>("LOGS_RATE_LIMIT_SILENCE")?;
    validate_env_parse::<humantime::Duration>("LOGS_SLOW_THRESHOLD")?;
    validate_env_parse::<humantime::Duration>("LOGS_CONFIG_RELOG_INTERVAL")?;

    validate_level_env("LOGS_LEVEL")?;
    validate_bypass_level_env()?;
    validate_modules_env()?;
    validate_otlp_logs_protocol()?;

    Ok(())
}

/// Validate that an env var (if set) can be parsed as type `T`.
fn validate_env_parse<T: std::str::FromStr>(var: &'static str) -> Result<(), ConfigValidationError>
where
    T::Err: fmt::Display,
{
    if let Ok(val) = env::var(var) {
        if val.parse::<T>().is_err() {
            return Err(ConfigValidationError::InvalidEnvVar {
                var,
                value: val,
                reason: format!("cannot parse as {}", std::any::type_name::<T>()),
            });
        }
    }
    Ok(())
}

/// Validate that an integer env var (if set) is > 0.
fn validate_env_positive_int(var: &'static str) -> Result<(), ConfigValidationError> {
    if let Ok(val) = env::var(var) {
        if let Ok(n) = val.parse::<usize>() {
            if n == 0 {
                return Err(ConfigValidationError::InvalidEnvVar {
                    var,
                    value: val,
                    reason: "must be greater than 0".to_string(),
                });
            }
        }
    }
    Ok(())
}

/// Validate that a duration env var (if set) is > 0s.
fn validate_env_positive_duration(var: &'static str) -> Result<(), ConfigValidationError> {
    if let Ok(val) = env::var(var) {
        if let Ok(d) = val.parse::<humantime::Duration>() {
            if std::time::Duration::from(d).is_zero() {
                return Err(ConfigValidationError::InvalidEnvVar {
                    var,
                    value: val,
                    reason: "must be greater than 0s".to_string(),
                });
            }
        }
    }
    Ok(())
}

/// Validate `LOGS_RATE_LIMIT_BYPASS_LEVEL` (accepts level names or NONE/OFF).
fn validate_bypass_level_env() -> Result<(), ConfigValidationError> {
    if let Ok(val) = env::var("LOGS_RATE_LIMIT_BYPASS_LEVEL") {
        if !val.eq_ignore_ascii_case("NONE")
            && !val.eq_ignore_ascii_case("OFF")
            && super::logger::normalize_level_str(&val).is_none()
        {
            return Err(ConfigValidationError::InvalidEnvVar {
                var: "LOGS_RATE_LIMIT_BYPASS_LEVEL",
                value: val,
                reason: "expected ERROR, WARN, INFO, DEBUG, TRACE, NONE, or OFF".to_string(),
            });
        }
    }
    Ok(())
}

/// Validate that a level env var (if set) contains a recognized severity name.
fn validate_level_env(var: &'static str) -> Result<(), ConfigValidationError> {
    if let Ok(val) = env::var(var) {
        if super::logger::normalize_level_str(&val).is_none() {
            return Err(ConfigValidationError::InvalidEnvVar {
                var,
                value: val,
                reason: "expected ERROR, WARN, INFO, DEBUG, or TRACE".to_string(),
            });
        }
    }
    Ok(())
}

/// Validate `LOGS_MODULES` format: comma-separated `module=LEVEL` pairs.
fn validate_modules_env() -> Result<(), ConfigValidationError> {
    let Ok(val) = env::var(super::logger::LOGS_MODULES) else {
        return Ok(());
    };

    for entry in val.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        match entry.split_once('=') {
            None => {
                return Err(ConfigValidationError::InvalidEnvVar {
                    var: "LOGS_MODULES",
                    value: val.clone(),
                    reason: format!("entry {entry:?} is missing '=' separator; expected format: module=LEVEL"),
                });
            },
            Some((_, level_str)) => {
                let level_str = level_str.trim();
                if super::logger::normalize_level_str(level_str).is_none() {
                    return Err(ConfigValidationError::InvalidEnvVar {
                        var: "LOGS_MODULES",
                        value: val.clone(),
                        reason: format!(
                            "level {level_str:?} in entry {entry:?} is not recognized; expected ERROR, WARN, INFO, DEBUG, or TRACE"
                        ),
                    });
                }
            },
        }
    }

    Ok(())
}

/// Validate `OTEL_EXPORTER_OTLP_LOGS_PROTOCOL` (if set) contains a supported protocol.
fn validate_otlp_logs_protocol() -> Result<(), ConfigValidationError> {
    for var in ["OTEL_EXPORTER_OTLP_LOGS_PROTOCOL", "OTEL_EXPORTER_OTLP_PROTOCOL"] {
        if let Ok(val) = env::var(var) {
            match val.as_str() {
                "grpc" | "http/protobuf" => {},
                _ => {
                    return Err(ConfigValidationError::InvalidEnvVar {
                        var: if var == "OTEL_EXPORTER_OTLP_LOGS_PROTOCOL" {
                            "OTEL_EXPORTER_OTLP_LOGS_PROTOCOL"
                        } else {
                            "OTEL_EXPORTER_OTLP_PROTOCOL"
                        },
                        value: val,
                        reason: "expected \"grpc\" or \"http/protobuf\"".to_string(),
                    });
                },
            }
        }
    }
    Ok(())
}

/// Verify stdout and stderr are writable (SRD 3.11: "write data to confirm validity").
fn validate_output_sinks() -> Result<(), ConfigValidationError> {
    // Write a single newline and flush to confirm the fd is open and the underlying
    // filesystem/pipe can accept data (an empty write would succeed even on a full disk).
    std::io::stdout()
        .write_all(b"\n")
        .and_then(|()| std::io::stdout().flush())
        .map_err(ConfigValidationError::StdoutNotWritable)?;
    std::io::stderr()
        .write_all(b"\n")
        .and_then(|()| std::io::stderr().flush())
        .map_err(ConfigValidationError::StderrNotWritable)?;
    Ok(())
}

/// Warn (but don't fail) if configured OTLP endpoint URLs look malformed.
/// Endpoints may not be reachable at startup in container orchestration environments.
fn warn_on_bad_otlp_endpoints() {
    for var in ["OTEL_EXPORTER_OTLP_LOGS_ENDPOINT", "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT"] {
        if let Ok(endpoint) = env::var(var) {
            if !endpoint.starts_with("http://") && !endpoint.starts_with("https://") {
                // Use eprintln since the tracing subscriber isn't installed yet
                eprintln!(
                    "WARNING: {var}={endpoint:?} does not start with http:// or https:// -- OTLP export may fail"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serial_test::serial;

    use super::*;

    /// Helper to run a closure with a temporary env var set, restoring afterward.
    /// Tests using this must be annotated with `#[serial]` to avoid env var races.
    fn with_env_var(var: &str, val: &str, f: impl FnOnce()) {
        unsafe {
            env::set_var(var, val);
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        unsafe {
            env::remove_var(var);
        }
        if let Err(e) = result {
            std::panic::resume_unwind(e);
        }
    }

    #[test]
    #[serial]
    fn valid_config_passes() {
        assert!(validate_env_vars().is_ok());
    }

    #[test]
    #[serial]
    fn invalid_rate_limit_count() {
        with_env_var("LOGS_RATE_LIMIT_COUNT", "banana", || {
            let err = validate_env_vars().unwrap_err();
            assert!(
                matches!(
                    err,
                    ConfigValidationError::InvalidEnvVar {
                        var: "LOGS_RATE_LIMIT_COUNT",
                        ..
                    }
                ),
                "expected InvalidEnvVar for LOGS_RATE_LIMIT_COUNT, got: {err:?}"
            );
        });
    }

    #[test]
    #[serial]
    fn invalid_rate_limit_interval() {
        with_env_var("LOGS_RATE_LIMIT_INTERVAL", "not-a-duration", || {
            let err = validate_env_vars().unwrap_err();
            assert!(matches!(
                err,
                ConfigValidationError::InvalidEnvVar {
                    var: "LOGS_RATE_LIMIT_INTERVAL",
                    ..
                }
            ));
        });
    }

    #[test]
    #[serial]
    fn invalid_logs_level() {
        with_env_var("LOGS_LEVEL", "BOGUS", || {
            let err = validate_env_vars().unwrap_err();
            assert!(matches!(
                err,
                ConfigValidationError::InvalidEnvVar { var: "LOGS_LEVEL", .. }
            ));
        });
    }

    #[test]
    #[serial]
    fn valid_logs_level() {
        with_env_var("LOGS_LEVEL", "WARN", || {
            assert!(validate_env_vars().is_ok());
        });
    }

    #[test]
    #[serial]
    fn invalid_modules_missing_equals() {
        with_env_var("LOGS_MODULES", "storage_core", || {
            let err = validate_env_vars().unwrap_err();
            assert!(matches!(
                err,
                ConfigValidationError::InvalidEnvVar {
                    var: "LOGS_MODULES",
                    ..
                }
            ));
        });
    }

    #[test]
    #[serial]
    fn invalid_modules_bad_level() {
        with_env_var("LOGS_MODULES", "storage_core=NOPE", || {
            let err = validate_env_vars().unwrap_err();
            assert!(matches!(
                err,
                ConfigValidationError::InvalidEnvVar {
                    var: "LOGS_MODULES",
                    ..
                }
            ));
        });
    }

    #[test]
    #[serial]
    fn valid_modules() {
        with_env_var("LOGS_MODULES", "storage_core=DEBUG,hyper=ERROR", || {
            assert!(validate_env_vars().is_ok());
        });
    }

    #[test]
    fn output_sinks_writable() {
        assert!(validate_output_sinks().is_ok());
    }

    #[test]
    #[serial]
    fn zero_rate_limit_count_rejected() {
        with_env_var("LOGS_RATE_LIMIT_COUNT", "0", || {
            let err = validate_env_vars().unwrap_err();
            assert!(matches!(
                err,
                ConfigValidationError::InvalidEnvVar {
                    var: "LOGS_RATE_LIMIT_COUNT",
                    ..
                }
            ));
        });
    }

    #[test]
    #[serial]
    fn zero_rate_limit_interval_rejected() {
        with_env_var("LOGS_RATE_LIMIT_INTERVAL", "0s", || {
            let err = validate_env_vars().unwrap_err();
            assert!(matches!(
                err,
                ConfigValidationError::InvalidEnvVar {
                    var: "LOGS_RATE_LIMIT_INTERVAL",
                    ..
                }
            ));
        });
    }

    #[test]
    #[serial]
    fn valid_otlp_protocol_grpc() {
        with_env_var("OTEL_EXPORTER_OTLP_LOGS_PROTOCOL", "grpc", || {
            assert!(validate_env_vars().is_ok());
        });
    }

    #[test]
    #[serial]
    fn valid_otlp_protocol_http() {
        with_env_var("OTEL_EXPORTER_OTLP_LOGS_PROTOCOL", "http/protobuf", || {
            assert!(validate_env_vars().is_ok());
        });
    }

    #[test]
    #[serial]
    fn invalid_otlp_logs_protocol() {
        with_env_var("OTEL_EXPORTER_OTLP_LOGS_PROTOCOL", "banana", || {
            let err = validate_env_vars().unwrap_err();
            assert!(matches!(
                err,
                ConfigValidationError::InvalidEnvVar {
                    var: "OTEL_EXPORTER_OTLP_LOGS_PROTOCOL",
                    ..
                }
            ));
        });
    }

    #[test]
    #[serial]
    fn invalid_otlp_generic_protocol() {
        with_env_var("OTEL_EXPORTER_OTLP_PROTOCOL", "xml", || {
            let err = validate_env_vars().unwrap_err();
            assert!(matches!(
                err,
                ConfigValidationError::InvalidEnvVar {
                    var: "OTEL_EXPORTER_OTLP_PROTOCOL",
                    ..
                }
            ));
        });
    }

    #[test]
    fn config_validation_error_display() {
        let err = ConfigValidationError::InvalidEnvVar {
            var: "LOGS_LEVEL",
            value: "BOGUS".to_string(),
            reason: "expected ERROR, WARN, INFO, DEBUG, or TRACE".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("LOGS_LEVEL"));
        assert!(msg.contains("BOGUS"));
    }
}
