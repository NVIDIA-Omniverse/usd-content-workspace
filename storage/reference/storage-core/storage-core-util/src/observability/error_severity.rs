// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Error severity configuration (SRD 3.2).
//!
//! Maps each [`tonic::Code`] to the [`tracing::Level`] at which API-boundary errors
//! should be logged. Defaults follow Section 3.2 (Errors) of the Omniverse
//! logging guidelines (Logging SRD).

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::warn_limited;
use figment::Figment;
use tracing::Level;

// ---------------------------------------------------------------------------
// Figment Config (follows the from_figment pattern)
// ---------------------------------------------------------------------------

/// Figment-extractable configuration for the `[error_severity]` section.
///
/// Keys are gRPC code names (case-insensitive), values are log level names.
/// An empty map (or absent section) means "use all SRD defaults."
#[derive(Debug, Clone, Default)]
pub struct Config {
    overrides: HashMap<String, String>,
}

impl Config {
    pub const CONFIG_KEY: &str = "error_severity";
}

impl TryFrom<&Figment> for Config {
    type Error = figment::Error;

    fn try_from(figment: &Figment) -> Result<Self, Self::Error> {
        let overrides: HashMap<String, String> = figment.focus(Self::CONFIG_KEY).extract()?;
        Ok(Self { overrides })
    }
}

impl From<Config> for ErrorSeverityConfig {
    fn from(config: Config) -> Self {
        if config.overrides.is_empty() {
            Self::default()
        } else {
            Self::from_overrides(&config.overrides)
        }
    }
}

// ---------------------------------------------------------------------------
// ErrorSeverityConfig (runtime representation)
// ---------------------------------------------------------------------------

/// Maps each [`tonic::Code`] to the [`tracing::Level`] at which API-boundary errors
/// should be logged, per Section 3.2 (Errors) of the Omniverse logging guidelines (Logging SRD).
///
/// # Defaults
///
/// | Level | Codes | SRD rationale |
/// |-------|-------|---------------|
/// | DEBUG | Cancelled | Not an error — client-initiated |
/// | INFO  | InvalidArgument, NotFound, AlreadyExists, OutOfRange, FailedPrecondition | User input / expected errors (SRD 3.2) |
/// | WARN  | PermissionDenied, Unauthenticated, DeadlineExceeded, Unavailable, ResourceExhausted | Transient errors impacting availability (SRD 3.2) |
/// | ERROR | Ok, Unknown, Aborted, Unimplemented, Internal, DataLoss | Unexpected / non-transient errors (SRD 3.2) |
#[derive(Debug, Clone)]
pub struct ErrorSeverityConfig {
    levels: Vec<Level>, // indexed by tonic::Code as usize
}

/// Number of [`tonic::Code`] variants (Ok through Unauthenticated = 0..=16).
const TONIC_CODE_COUNT: usize = tonic::Code::Unauthenticated as usize + 1;

impl ErrorSeverityConfig {
    /// Look up the log level for a given gRPC status code.
    #[inline]
    pub fn level(&self, code: tonic::Code) -> Level {
        self.levels.get(code as usize).copied().unwrap_or(Level::ERROR)
    }

    /// Build from SRD defaults, then apply string-keyed overrides.
    ///
    /// Keys are tonic code names (e.g. `"NotFound"`, `"InvalidArgument"`).
    /// Values are level names (e.g. `"INFO"`, `"WARN"`).
    /// Unknown keys or unparseable values are logged as warnings and skipped.
    pub fn from_overrides(overrides: &HashMap<String, String>) -> Self {
        let mut config = Self::default();
        for (code_name, level_name) in overrides {
            let Some(code) = parse_tonic_code(code_name) else {
                warn_limited!(code_name, "unknown error code in error_severity config, ignoring");
                continue;
            };
            let Some(level) = parse_level(level_name) else {
                warn_limited!(
                    level_name,
                    code_name,
                    "unknown level in error_severity config, ignoring"
                );
                continue;
            };
            config.levels[code as usize] = level;
        }
        config
    }
}

impl Default for ErrorSeverityConfig {
    /// Defaults per the Omniverse Logging SRD (Section 3.2 — Errors).
    fn default() -> Self {
        let mut levels = vec![Level::ERROR; TONIC_CODE_COUNT];

        // DEBUG: client cancelled — not actionable on the server side.
        levels[tonic::Code::Cancelled as usize] = Level::DEBUG;

        // INFO: "Expected errors, such as invalid user input, should be logged at INFO
        // level." (SRD 3.2 — User Input Errors).
        levels[tonic::Code::InvalidArgument as usize] = Level::INFO;
        levels[tonic::Code::NotFound as usize] = Level::INFO;
        levels[tonic::Code::AlreadyExists as usize] = Level::INFO;
        levels[tonic::Code::OutOfRange as usize] = Level::INFO;
        levels[tonic::Code::FailedPrecondition as usize] = Level::INFO;

        // WARN: "Errors that are expected and can be recovered from (usually with retries)
        // should be logged at WARNING level if they impact availability of the whole
        // process." (SRD 3.2 — Transient Errors).
        levels[tonic::Code::PermissionDenied as usize] = Level::WARN;
        levels[tonic::Code::Unauthenticated as usize] = Level::WARN;
        levels[tonic::Code::DeadlineExceeded as usize] = Level::WARN;
        levels[tonic::Code::Unavailable as usize] = Level::WARN;
        levels[tonic::Code::ResourceExhausted as usize] = Level::WARN;

        // ERROR: "All errors that are not expected and not transient must be logged at
        // ERROR level or above." (SRD 3.2 — Errors).
        // (Ok, Unknown, Aborted, Unimplemented, Internal, DataLoss — the default fill)

        Self { levels }
    }
}

// ---------------------------------------------------------------------------
// Global config
// ---------------------------------------------------------------------------

static ERROR_SEVERITY: OnceLock<ErrorSeverityConfig> = OnceLock::new();

/// Set the global error severity configuration. Should be called once at startup.
/// Subsequent calls are silently ignored (first-write-wins).
pub fn init_error_severity(config: ErrorSeverityConfig) {
    let _ = ERROR_SEVERITY.set(config);
}

/// Get the active error severity configuration, falling back to SRD defaults.
pub fn error_severity() -> &'static ErrorSeverityConfig {
    ERROR_SEVERITY.get_or_init(ErrorSeverityConfig::default)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_tonic_code(s: &str) -> Option<tonic::Code> {
    // Case-insensitive to match parse_level behavior.
    // Also accepts common aliases (e.g. "AccessDenied" for PermissionDenied).
    match s.to_lowercase().as_str() {
        "ok" => Some(tonic::Code::Ok),
        "cancelled" => Some(tonic::Code::Cancelled),
        "unknown" => Some(tonic::Code::Unknown),
        "invalidargument" => Some(tonic::Code::InvalidArgument),
        "deadlineexceeded" => Some(tonic::Code::DeadlineExceeded),
        "notfound" => Some(tonic::Code::NotFound),
        "alreadyexists" => Some(tonic::Code::AlreadyExists),
        "permissiondenied" | "accessdenied" => Some(tonic::Code::PermissionDenied),
        "resourceexhausted" => Some(tonic::Code::ResourceExhausted),
        "failedprecondition" => Some(tonic::Code::FailedPrecondition),
        "aborted" => Some(tonic::Code::Aborted),
        "outofrange" => Some(tonic::Code::OutOfRange),
        "unimplemented" => Some(tonic::Code::Unimplemented),
        "internal" => Some(tonic::Code::Internal),
        "unavailable" => Some(tonic::Code::Unavailable),
        "dataloss" => Some(tonic::Code::DataLoss),
        "unauthenticated" => Some(tonic::Code::Unauthenticated),
        _ => None,
    }
}

fn parse_level(s: &str) -> Option<Level> {
    match s.to_uppercase().as_str() {
        "ERROR" => Some(Level::ERROR),
        "WARN" => Some(Level::WARN),
        "INFO" => Some(Level::INFO),
        "DEBUG" => Some(Level::DEBUG),
        "TRACE" => Some(Level::TRACE),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_levels_match_srd() {
        let config = ErrorSeverityConfig::default();

        assert_eq!(config.level(tonic::Code::Cancelled), Level::DEBUG);

        assert_eq!(config.level(tonic::Code::InvalidArgument), Level::INFO);
        assert_eq!(config.level(tonic::Code::NotFound), Level::INFO);
        assert_eq!(config.level(tonic::Code::AlreadyExists), Level::INFO);
        assert_eq!(config.level(tonic::Code::OutOfRange), Level::INFO);
        assert_eq!(config.level(tonic::Code::FailedPrecondition), Level::INFO);

        assert_eq!(config.level(tonic::Code::PermissionDenied), Level::WARN);
        assert_eq!(config.level(tonic::Code::Unauthenticated), Level::WARN);
        assert_eq!(config.level(tonic::Code::DeadlineExceeded), Level::WARN);
        assert_eq!(config.level(tonic::Code::Unavailable), Level::WARN);
        assert_eq!(config.level(tonic::Code::ResourceExhausted), Level::WARN);

        assert_eq!(config.level(tonic::Code::Ok), Level::ERROR);
        assert_eq!(config.level(tonic::Code::Unknown), Level::ERROR);
        assert_eq!(config.level(tonic::Code::Aborted), Level::ERROR);
        assert_eq!(config.level(tonic::Code::Unimplemented), Level::ERROR);
        assert_eq!(config.level(tonic::Code::Internal), Level::ERROR);
        assert_eq!(config.level(tonic::Code::DataLoss), Level::ERROR);
    }

    #[test]
    fn from_overrides_applies_changes() {
        let overrides = HashMap::from([
            ("NotFound".to_string(), "WARN".to_string()),
            ("Internal".to_string(), "info".to_string()),
            ("AccessDenied".to_string(), "DEBUG".to_string()),
        ]);
        let config = ErrorSeverityConfig::from_overrides(&overrides);

        assert_eq!(config.level(tonic::Code::NotFound), Level::WARN);
        assert_eq!(config.level(tonic::Code::Internal), Level::INFO);
        assert_eq!(config.level(tonic::Code::PermissionDenied), Level::DEBUG);
        assert_eq!(config.level(tonic::Code::InvalidArgument), Level::INFO);
        assert_eq!(config.level(tonic::Code::Cancelled), Level::DEBUG);
    }

    #[test]
    fn from_overrides_ignores_invalid_entries() {
        let overrides = HashMap::from([
            ("BogusCode".to_string(), "WARN".to_string()),
            ("NotFound".to_string(), "BOGUS_LEVEL".to_string()),
        ]);
        let config = ErrorSeverityConfig::from_overrides(&overrides);
        assert_eq!(config.level(tonic::Code::NotFound), Level::INFO);
    }

    #[test]
    fn parse_tonic_code_coverage() {
        let cases = [
            ("Ok", tonic::Code::Ok),
            ("Cancelled", tonic::Code::Cancelled),
            ("Unknown", tonic::Code::Unknown),
            ("InvalidArgument", tonic::Code::InvalidArgument),
            ("DeadlineExceeded", tonic::Code::DeadlineExceeded),
            ("NotFound", tonic::Code::NotFound),
            ("AlreadyExists", tonic::Code::AlreadyExists),
            ("PermissionDenied", tonic::Code::PermissionDenied),
            ("AccessDenied", tonic::Code::PermissionDenied),
            ("ResourceExhausted", tonic::Code::ResourceExhausted),
            ("FailedPrecondition", tonic::Code::FailedPrecondition),
            ("Aborted", tonic::Code::Aborted),
            ("OutOfRange", tonic::Code::OutOfRange),
            ("Unimplemented", tonic::Code::Unimplemented),
            ("Internal", tonic::Code::Internal),
            ("Unavailable", tonic::Code::Unavailable),
            ("DataLoss", tonic::Code::DataLoss),
            ("Unauthenticated", tonic::Code::Unauthenticated),
        ];
        for (name, expected) in cases {
            assert_eq!(parse_tonic_code(name), Some(expected), "failed for {name}");
        }
        assert_eq!(parse_tonic_code("Nope"), None);
    }

    #[test]
    fn parse_level_case_insensitive() {
        assert_eq!(parse_level("error"), Some(Level::ERROR));
        assert_eq!(parse_level("WARN"), Some(Level::WARN));
        assert_eq!(parse_level("Info"), Some(Level::INFO));
        assert_eq!(parse_level("debug"), Some(Level::DEBUG));
        assert_eq!(parse_level("TRACE"), Some(Level::TRACE));
        assert_eq!(parse_level("nope"), None);
    }

    /// Compile-time exhaustiveness check — will fail to compile if tonic adds new variants.
    #[test]
    fn ensure_all_tonic_codes_handled() {
        fn check(code: tonic::Code) {
            match code {
                tonic::Code::Ok
                | tonic::Code::Cancelled
                | tonic::Code::Unknown
                | tonic::Code::InvalidArgument
                | tonic::Code::DeadlineExceeded
                | tonic::Code::NotFound
                | tonic::Code::AlreadyExists
                | tonic::Code::PermissionDenied
                | tonic::Code::ResourceExhausted
                | tonic::Code::FailedPrecondition
                | tonic::Code::Aborted
                | tonic::Code::OutOfRange
                | tonic::Code::Unimplemented
                | tonic::Code::Internal
                | tonic::Code::Unavailable
                | tonic::Code::DataLoss
                | tonic::Code::Unauthenticated => {},
            }
        }
        check(tonic::Code::Ok);

        assert_eq!(
            tonic::Code::Unauthenticated as usize,
            16,
            "tonic::Code enum values may have changed — update TONIC_CODE_COUNT"
        );

        let config = ErrorSeverityConfig::default();
        assert_eq!(config.levels.len(), TONIC_CODE_COUNT);
    }

    #[test]
    fn config_from_figment_round_trip() {
        let figment = Figment::new()
            .merge(("error_severity.NotFound", "WARN"))
            .merge(("error_severity.Internal", "DEBUG"));

        let config = Config::try_from(&figment).expect("figment extraction should succeed");
        let severity: ErrorSeverityConfig = config.into();

        assert_eq!(severity.level(tonic::Code::NotFound), Level::WARN);
        assert_eq!(severity.level(tonic::Code::Internal), Level::DEBUG);
        // Non-overridden codes keep defaults
        assert_eq!(severity.level(tonic::Code::Cancelled), Level::DEBUG);
        assert_eq!(severity.level(tonic::Code::InvalidArgument), Level::INFO);
    }
}
