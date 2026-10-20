// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Rate-limited macros for `tracing`.
//!
//! These macros provide per-call-site rate limiting, emitting SILENCED/UN-SILENCED notices
//! when throttling activates and deactivates. Each macro invocation creates a `static`
//! [`TraceLimiter`](super::trace_limited::TraceLimiter) at the call site, so different call sites
//! are rate-limited independently.
//!
//! # When to use rate-limited tracing
//!
//! Use these macros on **high-rate code paths** where a single error condition can produce
//! thousands of trace events per second — for example:
//! - Retry loops for backend connections
//! - Per-request errors on the data plane
//! - Cache miss / eviction warnings
//!
//! For code paths that fire at most a few times per minute, normal `tracing::warn!()` etc.
//! is fine.
//!
//! # Macro variants
//!
//! ## Simplest — default limiter
//!
//! ```ignore
//! use storage_core_util::warn_limited;
//!
//! warn_limited!("connection to backend failed: {err}");
//! ```
//!
//! ## Explicit limiter instance (`limit:`)
//!
//! Share a single limiter across multiple call sites:
//!
//! ```ignore
//! use std::sync::LazyLock;
//! use storage_core_util::observability::trace_limited::TraceLimiter;
//! use storage_core_util::warn_limited;
//!
//! static RETRY_LIMITER: LazyLock<TraceLimiter> = LazyLock::new(TraceLimiter::default);
//! warn_limited!(limit: RETRY_LIMITER, "retry attempt {n}");
//! ```
//!
//! ## Custom limiter type (`limit_type:`)
//!
//! Use a custom `TraceLimiter` type that implements `Default`:
//!
//! ```ignore
//! use storage_core_util::warn_limited;
//!
//! warn_limited!(limit_type: MyCustomLimiter, "something happened");
//! ```
//!
//! # Output when throttling activates
//!
//! ```text
//! WARN connection to backend failed: timeout
//! WARN ### SILENCED log at src/backend.rs:42:5 ###
//! ... (60s silence) ...
//! WARN ### UN-SILENCED log at src/backend.rs:42:5 -- 847 messages over the last 60.00 seconds ###
//! WARN connection to backend failed: timeout
//! ```

/// Internal base macro for rate-limited tracing. Use the level-specific macros instead.
#[macro_export]
#[doc(hidden)]
macro_rules! __trace_limited {
    (limit: $CONTROL:expr, target: $TARGET:expr, $LEVEL:expr, $($ARG:tt)+) => {{
        const __LEVEL: ::tracing::Level = $LEVEL;
        // Skip the mutex + flow-control arithmetic when the level is globally disabled.
        if !::tracing::level_enabled!(__LEVEL) {
            // no-op: level compiled out or filtered by subscriber max level
        } else {
        let __check = $CONTROL.check(__LEVEL);
        if let $crate::observability::trace_limited::TraceControl::Unblocked { time_blocked, summary, .. } = __check {
            ::tracing::event!(
                target: $TARGET,
                __LEVEL,
                "### UN-SILENCED log at {}:{}:{} -- {} over the last {} seconds ###",
                file!(),
                line!(),
                column!(),
                summary,
                time_blocked.as_secs_f64()
            );
        }
        if __check.should_emit() {
            ::tracing::event!(target: $TARGET, __LEVEL, $($ARG)+);
        }
        if __check.is_initial_block() {
            ::tracing::event!(
                target: $TARGET,
                __LEVEL,
                "### SILENCED log at {}:{}:{} ###",
                file!(),
                line!(),
                column!(),
            );
        }
        } // else level_enabled
    }};
    (limit: $CONTROL:expr, $LEVEL:expr, $($ARG:tt)+) => {
        $crate::__trace_limited!(limit: $CONTROL, target: module_path!(), $LEVEL, $($ARG)+)
    };
    (limit_type: $LIMITER:ty, target: $TARGET:expr, $LEVEL:expr, $($ARG:tt)+) => {{
        static __LIMITER: ::std::sync::LazyLock<$LIMITER> = ::std::sync::LazyLock::new(|| {
            <$LIMITER>::default()
        });
        $crate::__trace_limited!(limit: __LIMITER, target: $TARGET, $LEVEL, $($ARG)+)
    }};
    (limit_type: $LIMITER:ty, $LEVEL:expr, $($ARG:tt)+) => {
        $crate::__trace_limited!(limit_type: $LIMITER, target: module_path!(), $LEVEL, $($ARG)+)
    };
    (target: $TARGET:expr, $LEVEL:expr, $($ARG:tt)+) => {
        $crate::__trace_limited!(
            limit_type: $crate::observability::trace_limited::TraceLimiter,
            target: $TARGET,
            $LEVEL,
            $($ARG)+
        )
    };
    ($LEVEL:expr, $($ARG:tt)+) => {
        $crate::__trace_limited!(target: module_path!(), $LEVEL, $($ARG)+)
    }
}

/// Rate-limited `error!`.
#[macro_export]
macro_rules! error_limited {
    (limit: $LIMIT:expr, target: $TARGET:expr, $($ARG:tt)+) => {
        $crate::__trace_limited!(limit: $LIMIT, target: $TARGET, ::tracing::Level::ERROR, $($ARG)+)
    };
    (limit: $LIMIT:expr, $($ARG:tt)+) => {
        $crate::__trace_limited!(limit: $LIMIT, ::tracing::Level::ERROR, $($ARG)+)
    };
    (limit_type: $LIMIT:ty, target: $TARGET:expr, $($ARG:tt)+) => {
        $crate::__trace_limited!(limit_type: $LIMIT, target: $TARGET, ::tracing::Level::ERROR, $($ARG)+)
    };
    (limit_type: $LIMIT:ty, $($ARG:tt)+) => {
        $crate::__trace_limited!(limit_type: $LIMIT, ::tracing::Level::ERROR, $($ARG)+)
    };
    (target: $TARGET:expr, $($ARG:tt)+) => {
        $crate::__trace_limited!(target: $TARGET, ::tracing::Level::ERROR, $($ARG)+)
    };
    ($($ARG:tt)+) => {
        $crate::__trace_limited!(::tracing::Level::ERROR, $($ARG)+)
    };
}

/// Rate-limited `warn!`.
#[macro_export]
macro_rules! warn_limited {
    (limit: $LIMIT:expr, target: $TARGET:expr, $($ARG:tt)+) => {
        $crate::__trace_limited!(limit: $LIMIT, target: $TARGET, ::tracing::Level::WARN, $($ARG)+)
    };
    (limit: $LIMIT:expr, $($ARG:tt)+) => {
        $crate::__trace_limited!(limit: $LIMIT, ::tracing::Level::WARN, $($ARG)+)
    };
    (limit_type: $LIMIT:ty, target: $TARGET:expr, $($ARG:tt)+) => {
        $crate::__trace_limited!(limit_type: $LIMIT, target: $TARGET, ::tracing::Level::WARN, $($ARG)+)
    };
    (limit_type: $LIMIT:ty, $($ARG:tt)+) => {
        $crate::__trace_limited!(limit_type: $LIMIT, ::tracing::Level::WARN, $($ARG)+)
    };
    (target: $TARGET:expr, $($ARG:tt)+) => {
        $crate::__trace_limited!(target: $TARGET, ::tracing::Level::WARN, $($ARG)+)
    };
    ($($ARG:tt)+) => {
        $crate::__trace_limited!(::tracing::Level::WARN, $($ARG)+)
    };
}

/// Rate-limited `info!`.
#[macro_export]
macro_rules! info_limited {
    (limit: $LIMIT:expr, target: $TARGET:expr, $($ARG:tt)+) => {
        $crate::__trace_limited!(limit: $LIMIT, target: $TARGET, ::tracing::Level::INFO, $($ARG)+)
    };
    (limit: $LIMIT:expr, $($ARG:tt)+) => {
        $crate::__trace_limited!(limit: $LIMIT, ::tracing::Level::INFO, $($ARG)+)
    };
    (limit_type: $LIMIT:ty, target: $TARGET:expr, $($ARG:tt)+) => {
        $crate::__trace_limited!(limit_type: $LIMIT, target: $TARGET, ::tracing::Level::INFO, $($ARG)+)
    };
    (limit_type: $LIMIT:ty, $($ARG:tt)+) => {
        $crate::__trace_limited!(limit_type: $LIMIT, ::tracing::Level::INFO, $($ARG)+)
    };
    (target: $TARGET:expr, $($ARG:tt)+) => {
        $crate::__trace_limited!(target: $TARGET, ::tracing::Level::INFO, $($ARG)+)
    };
    ($($ARG:tt)+) => {
        $crate::__trace_limited!(::tracing::Level::INFO, $($ARG)+)
    };
}

/// Rate-limited `debug!`.
#[macro_export]
macro_rules! debug_limited {
    (limit: $LIMIT:expr, target: $TARGET:expr, $($ARG:tt)+) => {
        $crate::__trace_limited!(limit: $LIMIT, target: $TARGET, ::tracing::Level::DEBUG, $($ARG)+)
    };
    (limit: $LIMIT:expr, $($ARG:tt)+) => {
        $crate::__trace_limited!(limit: $LIMIT, ::tracing::Level::DEBUG, $($ARG)+)
    };
    (limit_type: $LIMIT:ty, target: $TARGET:expr, $($ARG:tt)+) => {
        $crate::__trace_limited!(limit_type: $LIMIT, target: $TARGET, ::tracing::Level::DEBUG, $($ARG)+)
    };
    (limit_type: $LIMIT:ty, $($ARG:tt)+) => {
        $crate::__trace_limited!(limit_type: $LIMIT, ::tracing::Level::DEBUG, $($ARG)+)
    };
    (target: $TARGET:expr, $($ARG:tt)+) => {
        $crate::__trace_limited!(target: $TARGET, ::tracing::Level::DEBUG, $($ARG)+)
    };
    ($($ARG:tt)+) => {
        $crate::__trace_limited!(::tracing::Level::DEBUG, $($ARG)+)
    };
}

/// Rate-limited `trace!`.
#[macro_export]
macro_rules! trace_limited {
    (limit: $LIMIT:expr, target: $TARGET:expr, $($ARG:tt)+) => {
        $crate::__trace_limited!(limit: $LIMIT, target: $TARGET, ::tracing::Level::TRACE, $($ARG)+)
    };
    (limit: $LIMIT:expr, $($ARG:tt)+) => {
        $crate::__trace_limited!(limit: $LIMIT, ::tracing::Level::TRACE, $($ARG)+)
    };
    (limit_type: $LIMIT:ty, target: $TARGET:expr, $($ARG:tt)+) => {
        $crate::__trace_limited!(limit_type: $LIMIT, target: $TARGET, ::tracing::Level::TRACE, $($ARG)+)
    };
    (limit_type: $LIMIT:ty, $($ARG:tt)+) => {
        $crate::__trace_limited!(limit_type: $LIMIT, ::tracing::Level::TRACE, $($ARG)+)
    };
    (target: $TARGET:expr, $($ARG:tt)+) => {
        $crate::__trace_limited!(target: $TARGET, ::tracing::Level::TRACE, $($ARG)+)
    };
    ($($ARG:tt)+) => {
        $crate::__trace_limited!(::tracing::Level::TRACE, $($ARG)+)
    };
}

/// Emit a tracing event at a runtime-determined level.
///
/// Unlike `tracing::event!` which requires a const level, this macro accepts a
/// `tracing::Level` expression evaluated at runtime and dispatches to the
/// corresponding `tracing::{error,warn,info,debug,trace}!` macro.
///
/// ```ignore
/// use storage_core_util::trace_at_level;
/// let level = tracing::Level::WARN;
/// trace_at_level!(level, "something happened: {err}");
/// trace_at_level!(level, key = %value, "structured fields work too");
/// ```
#[macro_export]
macro_rules! trace_at_level {
    (target: $target:expr, $level:expr, $($args:tt)+) => {
        match $level {
            ::tracing::Level::ERROR => ::tracing::error!(target: $target, $($args)+),
            ::tracing::Level::WARN  => ::tracing::warn!(target: $target, $($args)+),
            ::tracing::Level::INFO  => ::tracing::info!(target: $target, $($args)+),
            ::tracing::Level::DEBUG => ::tracing::debug!(target: $target, $($args)+),
            ::tracing::Level::TRACE => ::tracing::trace!(target: $target, $($args)+),
        }
    };
    ($level:expr, $($args:tt)+) => {
        match $level {
            ::tracing::Level::ERROR => ::tracing::error!($($args)+),
            ::tracing::Level::WARN  => ::tracing::warn!($($args)+),
            ::tracing::Level::INFO  => ::tracing::info!($($args)+),
            ::tracing::Level::DEBUG => ::tracing::debug!($($args)+),
            ::tracing::Level::TRACE => ::tracing::trace!($($args)+),
        }
    };
}

/// Rate-limited tracing at a runtime-determined level.
///
/// Combines [`trace_at_level!`] with [`TraceLimiter`](super::trace_limited::TraceLimiter)
/// to rate-limit high-frequency call sites whose severity is determined at runtime.
/// Emits SILENCED / UN-SILENCED notices automatically.
///
/// ```ignore
/// use storage_core_util::trace_at_level_limited;
///
/// // Per-call-site default limiter:
/// trace_at_level_limited!(level, "something happened");
///
/// // Explicit limiter reference:
/// trace_at_level_limited!(limit: &MY_LIMITER, level, "something happened");
/// ```
#[macro_export]
macro_rules! trace_at_level_limited {
    (limit: $CONTROL:expr, target: $TARGET:expr, $LEVEL:expr, $($ARG:tt)+) => {{
        let __level = $LEVEL;
        let __check = $CONTROL.check(__level);
        if let $crate::observability::trace_limited::TraceControl::Unblocked { time_blocked, summary, .. } = __check {
            $crate::trace_at_level!(
                target: $TARGET,
                __level,
                "### UN-SILENCED log at {}:{}:{} -- {} over the last {} seconds ###",
                file!(),
                line!(),
                column!(),
                summary,
                time_blocked.as_secs_f64()
            );
        }
        if __check.should_emit() {
            $crate::trace_at_level!(target: $TARGET, __level, $($ARG)+);
        }
        if __check.is_initial_block() {
            $crate::trace_at_level!(
                target: $TARGET,
                __level,
                "### SILENCED log at {}:{}:{} ###",
                file!(),
                line!(),
                column!(),
            );
        }
    }};
    (limit: $CONTROL:expr, $LEVEL:expr, $($ARG:tt)+) => {{
        $crate::trace_at_level_limited!(limit: $CONTROL, target: module_path!(), $LEVEL, $($ARG)+)
    }};
    (target: $TARGET:expr, $LEVEL:expr, $($ARG:tt)+) => {{
        static __LIMITER: ::std::sync::LazyLock<$crate::observability::trace_limited::TraceLimiter> =
            ::std::sync::LazyLock::new(|| <$crate::observability::trace_limited::TraceLimiter>::default());
        $crate::trace_at_level_limited!(limit: __LIMITER, target: $TARGET, $LEVEL, $($ARG)+)
    }};
    ($LEVEL:expr, $($ARG:tt)+) => {{
        static __LIMITER: ::std::sync::LazyLock<$crate::observability::trace_limited::TraceLimiter> =
            ::std::sync::LazyLock::new(|| <$crate::observability::trace_limited::TraceLimiter>::default());
        $crate::trace_at_level_limited!(limit: __LIMITER, target: module_path!(), $LEVEL, $($ARG)+)
    }};
}

/// Log at INFO level if an operation exceeds a duration threshold (SRD 3.7 Outliers).
///
/// Three invocation forms:
///
/// ## Guard creation (logs on drop if slow)
///
/// ```ignore
/// use storage_core_util::log_if_slow;
/// use std::time::Duration;
///
/// // With explicit threshold:
/// let _guard = log_if_slow!(guard: "cache_lookup", Duration::from_secs(5));
///
/// // With default threshold (from LOGS_SLOW_THRESHOLD env var, default 10s):
/// let _guard = log_if_slow!(guard: "cache_lookup");
/// ```
///
/// ## Inline elapsed check
///
/// ```ignore
/// use storage_core_util::log_if_slow;
/// use std::time::{Duration, Instant};
///
/// let start = Instant::now();
/// // ... do work ...
/// log_if_slow!(start, Duration::from_secs(5), "cache lookup for {key}");
/// log_if_slow!(start, Duration::from_secs(5), %key, "cache lookup");
/// ```
#[macro_export]
macro_rules! log_if_slow {
    // Guard creation with explicit threshold
    (guard: $op:expr, $threshold:expr) => {
        $crate::observability::slow_operation::SlowOperationGuard::new($op, $threshold)
    };
    // Guard creation with default threshold
    (guard: $op:expr) => {
        $crate::observability::slow_operation::SlowOperationGuard::with_default_threshold($op)
    };
    // Inline check: log_if_slow!(start, threshold, "message")
    // or: log_if_slow!(start, threshold, key = value, "message")
    ($start:expr, $threshold:expr, $($arg:tt)+) => {{
        let __elapsed = $start.elapsed();
        if __elapsed >= $threshold {
            ::tracing::info!(
                elapsed_ms = __elapsed.as_millis() as u64,
                threshold_ms = $threshold.as_millis() as u64,
                $($arg)+
            );
        }
    }};
}
