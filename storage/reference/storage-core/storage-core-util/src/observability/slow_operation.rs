// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Slow operation detection utilities (SRD 3.7 Outliers).
//!
//! Provides [`SlowOperationGuard`], an RAII guard that logs at INFO level when a scope
//! takes longer than a configurable threshold. Per the SRD: "Log outliers at INFO level.
//! Consider logging at one level above the same non-outlier logs."
//!
//! # Usage
//!
//! ## Guard pattern (logs on drop if slow)
//!
//! ```ignore
//! use storage_core_util::observability::slow_operation::SlowOperationGuard;
//! use std::time::Duration;
//!
//! let _guard = SlowOperationGuard::new("cache_lookup", Duration::from_secs(5));
//! // ... do work ...
//! // If this scope takes > 5s, INFO log is emitted on drop.
//! ```
//!
//! ## Default threshold from environment
//!
//! ```ignore
//! let _guard = SlowOperationGuard::with_default_threshold("cache_lookup");
//! // Uses LOGS_SLOW_THRESHOLD env var (default: 10s)
//! ```
//!
//! ## Macro shorthand
//!
//! ```ignore
//! use storage_core_util::log_if_slow;
//! use std::time::Instant;
//!
//! // Guard creation:
//! let _guard = log_if_slow!(guard: "cache_lookup", Duration::from_secs(5));
//!
//! // Inline elapsed check:
//! let start = Instant::now();
//! // ... do work ...
//! log_if_slow!(start, Duration::from_secs(5), "cache lookup");
//! ```

use crate::info_limited;
use std::{
    env,
    sync::LazyLock,
    time::{Duration, Instant},
};

/// Default slow-operation threshold, overridable via `LOGS_SLOW_THRESHOLD` env var.
/// Accepts [`humantime`] duration strings (e.g. `5s`, `500ms`, `2m`). Defaults to 10 seconds.
static DEFAULT_THRESHOLD: LazyLock<Duration> = LazyLock::new(|| {
    env::var("LOGS_SLOW_THRESHOLD")
        .ok()
        .and_then(|s| s.parse::<humantime::Duration>().ok())
        .map(Duration::from)
        .unwrap_or(Duration::from_secs(10))
});

/// Returns the default slow-operation threshold (from `LOGS_SLOW_THRESHOLD` env var or 10s).
pub fn default_slow_threshold() -> Duration {
    *DEFAULT_THRESHOLD
}

/// RAII guard that logs at INFO level on [`Drop`] if the guarded scope exceeds a duration
/// threshold.
///
/// Per SRD 3.7: "Log outliers at INFO level. Consider logging at one level above the same
/// non-outlier logs."
pub struct SlowOperationGuard {
    operation: &'static str,
    threshold: Duration,
    start: Instant,
    context: Option<String>,
}

impl SlowOperationGuard {
    /// Create a guard with the given operation name and threshold.
    pub fn new(operation: &'static str, threshold: Duration) -> Self {
        Self {
            operation,
            threshold,
            start: Instant::now(),
            context: None,
        }
    }

    /// Create a guard using the default threshold from `LOGS_SLOW_THRESHOLD` env var (default 10s).
    pub fn with_default_threshold(operation: &'static str) -> Self {
        Self::new(operation, default_slow_threshold())
    }

    /// Attach additional context that will appear in the log message if the operation is slow.
    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.context = Some(context.into());
        self
    }

    /// Elapsed time since guard creation.
    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    /// Manually check if the operation has exceeded the threshold.
    /// Returns `true` if slow (elapsed >= threshold).
    pub fn is_slow(&self) -> bool {
        self.start.elapsed() >= self.threshold
    }
}

impl Drop for SlowOperationGuard {
    fn drop(&mut self) {
        let elapsed = self.start.elapsed();
        if elapsed >= self.threshold {
            if let Some(ref ctx) = self.context {
                info_limited!(
                    operation = self.operation,
                    elapsed_ms = elapsed.as_millis() as u64,
                    threshold_ms = self.threshold.as_millis() as u64,
                    context = ctx.as_str(),
                    "Slow operation detected",
                );
            } else {
                info_limited!(
                    operation = self.operation,
                    elapsed_ms = elapsed.as_millis() as u64,
                    threshold_ms = self.threshold.as_millis() as u64,
                    "Slow operation detected",
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_reports_slow_with_zero_threshold() {
        let guard = SlowOperationGuard::new("test_op", Duration::ZERO);
        // Any non-zero elapsed time exceeds a zero threshold
        std::thread::sleep(Duration::from_millis(1));
        assert!(guard.is_slow());
    }

    #[test]
    fn guard_not_slow_with_large_threshold() {
        let guard = SlowOperationGuard::new("test_op", Duration::from_secs(3600));
        assert!(!guard.is_slow());
    }

    #[test]
    fn guard_with_context() {
        let guard = SlowOperationGuard::new("test_op", Duration::ZERO).with_context("key=abc123".to_string());
        assert!(guard.context.as_deref() == Some("key=abc123"));
    }

    #[test]
    fn default_threshold_is_reasonable() {
        // Without LOGS_SLOW_THRESHOLD set, should be 10s
        let threshold = default_slow_threshold();
        assert!(threshold.as_secs() > 0, "threshold should be positive");
    }

    #[test]
    fn elapsed_increases() {
        let guard = SlowOperationGuard::new("test_op", Duration::from_secs(3600));
        let e1 = guard.elapsed();
        std::thread::sleep(Duration::from_millis(5));
        let e2 = guard.elapsed();
        assert!(e2 > e1);
    }
}
