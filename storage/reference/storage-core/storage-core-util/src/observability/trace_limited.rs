// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Per-call-site tracing rate limiting.
//!
//! [`TraceLimiter`] wraps a [`FlowControl`] to track suppression state and provide a 3-state
//! model for log rate limiting:
//!
//! - **[`TraceControl::Ready`]** — log the message normally.
//! - **[`TraceControl::Blocked`]** — the call site is suppressed. On the *first* block,
//!   `initial: true` signals that a "SILENCED" notice should be emitted. Subsequent blocks
//!   are silent.
//! - **[`TraceControl::Unblocked`]** — the silence period has ended. Contains `time_blocked`
//!   and `blocked_count` for a summary message ("UN-SILENCED ... N messages over the last M
//!   seconds").
//!
//! # Default configuration via environment variables
//!
//! [`TraceLimiter::default()`] reads these environment variables once at first use. All values
//! accept [`humantime`](https://docs.rs/humantime) duration strings (e.g. `500ms`, `2s`, `1m`).
//!
//! | Env Var | Default | Description |
//! |---|---|---|
//! | `LOGS_RATE_LIMIT_COUNT` | `10` | Max messages per interval before throttling. |
//! | `LOGS_RATE_LIMIT_INTERVAL` | `1s` | Time window for the count. |
//! | `LOGS_RATE_LIMIT_SILENCE` | `60s` | How long a call site stays silent after exceeding the limit. |
//!
//! # Custom limiter instances
//!
//! Create a custom `TraceLimiter` when the defaults don't fit — e.g., a high-frequency data-plane
//! path that should silence for longer, or a critical path that should allow more messages:
//!
//! ```
//! use storage_core_util::observability::trace_limited::TraceLimiter;
//! use storage_core_util::observability::flow_control::TimeBasedFlowControl;
//! use std::time::Duration;
//!
//! let limiter = TraceLimiter::new(
//!     TimeBasedFlowControl::builder()
//!         .allow(3)
//!         .interval(Duration::from_secs(5))
//!         .reset_interval(Duration::from_secs(120))
//!         .build()
//! );
//! ```

use super::flow_control::{FlowControl, TimeBasedFlowControl, TimeBasedFlowControlConfig};
use std::{
    env,
    fmt::Debug,
    str::FromStr,
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};

/// Wraps a [`FlowControl`] to track suppression state for log rate limiting.
///
/// Use via the `warn_limited!` / `error_limited!` / etc. macros, or call
/// [`check()`](Self::check) directly for custom integrations.
///
/// # Outlier preference (SRD 3.8)
///
/// Messages at or above the configured `bypass_level` (default: `ERROR`) are never
/// suppressed, even when the call site is in its silence period. This ensures that
/// high-severity outliers are always logged.
pub struct TraceLimiter<FC = TimeBasedFlowControl> {
    inner: Mutex<TraceLimiterInner<FC>>,
    /// Messages at this level or more severe bypass rate limiting.
    /// Default: Some(ERROR). `None` disables bypass (all levels rate-limited).
    bypass_level: Option<tracing::Level>,
}

struct TraceLimiterInner<FC> {
    flow_control: FC,
    /// Tracked state during a suppression period.
    blocks: Option<BlockedState>,
}

/// State tracked during a suppression period.
struct BlockedState {
    start: Instant,
    summary: SuppressedSummary,
}

/// Per-severity counts of suppressed messages during a blocked period (SRD 3.8).
///
/// Surfaced in [`TraceControl::Unblocked`] so the UN-SILENCED message can include
/// an aggregate breakdown of what was suppressed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SuppressedSummary {
    pub error_count: usize,
    pub warn_count: usize,
    pub info_count: usize,
    pub debug_count: usize,
    pub trace_count: usize,
    /// Count of messages that bypassed suppression (e.g. ERROR when bypass_level=ERROR).
    /// These were emitted but are included here for completeness so operators know
    /// the full throughput during the suppression period.
    pub bypassed_count: usize,
}

impl SuppressedSummary {
    fn increment(&mut self, level: tracing::Level) {
        match level {
            l if l == tracing::Level::ERROR => self.error_count = self.error_count.saturating_add(1),
            l if l == tracing::Level::WARN => self.warn_count = self.warn_count.saturating_add(1),
            l if l == tracing::Level::INFO => self.info_count = self.info_count.saturating_add(1),
            l if l == tracing::Level::DEBUG => self.debug_count = self.debug_count.saturating_add(1),
            _ => self.trace_count = self.trace_count.saturating_add(1),
        }
    }

    /// Total number of suppressed messages across all levels.
    pub fn total(&self) -> usize {
        self.error_count
            .saturating_add(self.warn_count)
            .saturating_add(self.info_count)
            .saturating_add(self.debug_count)
            .saturating_add(self.trace_count)
    }
}

impl std::fmt::Display for SuppressedSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} message", self.total())?;
        if self.total() != 1 {
            write!(f, "s")?;
        }
        // Show per-level breakdown when more than one level is present
        let levels: &[(usize, &str)] = &[
            (self.error_count, "error"),
            (self.warn_count, "warn"),
            (self.info_count, "info"),
            (self.debug_count, "debug"),
            (self.trace_count, "trace"),
        ];
        let non_zero = levels.iter().filter(|(c, _)| *c > 0).count();
        if non_zero > 1 {
            write!(f, " (")?;
            let mut first = true;
            for &(count, label) in levels {
                if count > 0 {
                    if !first {
                        write!(f, ", ")?;
                    }
                    write!(f, "{count} {label}")?;
                    first = false;
                }
            }
            write!(f, ")")?;
        }
        if self.bypassed_count > 0 {
            write!(f, " + {} bypassed", self.bypassed_count)?;
        }
        Ok(())
    }
}

impl<FC: FlowControl> TraceLimiter<FC> {
    /// Create a new limiter with the given flow control strategy.
    ///
    /// Uses the default bypass level (`ERROR`): error-level messages always pass through
    /// even during suppression.
    pub fn new(flow_control: FC) -> Self {
        Self {
            inner: Mutex::new(TraceLimiterInner {
                flow_control,
                blocks: None,
            }),
            bypass_level: default_bypass_level(),
        }
    }

    /// Create a new limiter with a custom bypass level.
    ///
    /// Messages at `bypass_level` or more severe will never be suppressed.
    /// Pass `None` to disable bypass entirely (all levels are rate-limited).
    pub fn with_bypass_level(flow_control: FC, bypass_level: Option<tracing::Level>) -> Self {
        Self {
            inner: Mutex::new(TraceLimiterInner {
                flow_control,
                blocks: None,
            }),
            bypass_level,
        }
    }

    /// Check whether a log message should be emitted.
    ///
    /// Messages at or above `bypass_level` always return [`TraceControl::Ready`], even during
    /// suppression (SRD 3.8: "prefer logging the outliers").
    pub fn check(&self, level: tracing::Level) -> TraceControl {
        let Ok(mut inner) = self.inner.lock() else {
            // Fail-open on poison: logging must never block or crash the application.
            // The critical section is trivial arithmetic and cannot panic in practice.
            return TraceControl::Ready;
        };

        #[allow(clippy::collapsible_else_if)]
        if inner.flow_control.reserve().should_perform() {
            if let Some(blocked) = inner.blocks.take() {
                TraceControl::Unblocked {
                    time_blocked: blocked.start.elapsed(),
                    blocked_count: blocked.summary.total(),
                    summary: blocked.summary,
                }
            } else {
                TraceControl::Ready
            }
        } else {
            // Would be blocked — but high-severity messages bypass suppression
            if self.bypass_level.is_some_and(|bl| level <= bl) {
                // Track bypassed messages so the summary reflects full throughput
                if let Some(ref mut blocked) = inner.blocks {
                    blocked.summary.bypassed_count = blocked.summary.bypassed_count.saturating_add(1);
                }
                return TraceControl::Ready;
            }

            if let Some(ref mut blocked) = inner.blocks {
                blocked.summary.increment(level);
                TraceControl::Blocked { initial: false }
            } else {
                let mut summary = SuppressedSummary::default();
                summary.increment(level);
                inner.blocks = Some(BlockedState {
                    start: Instant::now(),
                    summary,
                });
                TraceControl::Blocked { initial: true }
            }
        }
    }
}

/// Read a value from the environment variable `var_name`, falling back to `default` if unset or
/// unparseable.
fn get_default_from_env<T>(var_name: &str, default: T) -> T
where
    T: FromStr + Debug,
    <T as FromStr>::Err: Debug,
{
    let Some(val_str) = env::var_os(var_name) else {
        tracing::trace!("environment variable {var_name} is not specified -- using {default:?}");
        return default;
    };
    let Some(val_str) = val_str.to_str() else {
        tracing::warn!("value for {var_name} {val_str:?} is not UTF-8 encoded -- falling back to {default:?}");
        return default;
    };
    match val_str.parse() {
        Ok(val) => val,
        Err(e) => {
            tracing::warn!(
                "value for {var_name} {val_str:?} could not be parsed ({e:?}) -- falling back to {default:?}"
            );
            default
        },
    }
}

/// Parse a `tracing::Level` from a string (case-insensitive).
fn parse_level(s: &str) -> Option<tracing::Level> {
    match s.to_uppercase().as_str() {
        "ERROR" => Some(tracing::Level::ERROR),
        "WARN" | "WARNING" => Some(tracing::Level::WARN),
        "INFO" => Some(tracing::Level::INFO),
        "DEBUG" => Some(tracing::Level::DEBUG),
        "TRACE" => Some(tracing::Level::TRACE),
        _ => None,
    }
}

/// Read the default bypass level from `LOGS_RATE_LIMIT_BYPASS_LEVEL`.
///
/// Default: `Some(ERROR)`. Set to `NONE` or `OFF` to disable bypass entirely.
fn default_bypass_level() -> Option<tracing::Level> {
    static LEVEL: LazyLock<Option<tracing::Level>> = LazyLock::new(|| match env::var("LOGS_RATE_LIMIT_BYPASS_LEVEL") {
        Ok(s) if s.eq_ignore_ascii_case("NONE") || s.eq_ignore_ascii_case("OFF") => None,
        Ok(s) => Some(parse_level(&s).unwrap_or(tracing::Level::ERROR)),
        Err(_) => Some(tracing::Level::ERROR),
    });
    *LEVEL
}

impl Default for TraceLimiter<TimeBasedFlowControl> {
    fn default() -> Self {
        static PRECONFIG: LazyLock<TimeBasedFlowControlConfig> = LazyLock::new(|| {
            TimeBasedFlowControlConfig::builder()
                .allow(get_default_from_env("LOGS_RATE_LIMIT_COUNT", 10))
                .interval(get_default_from_env(
                    "LOGS_RATE_LIMIT_INTERVAL",
                    humantime::Duration::from(Duration::from_secs(1)),
                ))
                .reset_interval(get_default_from_env(
                    "LOGS_RATE_LIMIT_SILENCE",
                    humantime::Duration::from(Duration::from_secs(60)),
                ))
                .build()
        });
        Self::new(TimeBasedFlowControl::with_config(PRECONFIG.clone()))
    }
}

/// Result of [`TraceLimiter::check`].
#[derive(Clone, Copy, Debug)]
pub enum TraceControl {
    /// The message should be logged normally.
    Ready,
    /// The call site has been un-silenced after a suppression period. Log the message and
    /// include a summary of suppressed messages over `time_blocked`.
    Unblocked {
        /// How long the call site was silenced.
        time_blocked: Duration,
        /// Total number of suppressed log attempts during the silence period.
        /// Equal to `summary.total()`. Kept for backward compatibility.
        blocked_count: usize,
        /// Per-severity breakdown of suppressed messages (SRD 3.8).
        summary: SuppressedSummary,
    },
    /// The call site is suppressed. If `initial` is `true`, this is the first suppression and
    /// a "SILENCED" notice should be emitted.
    Blocked {
        /// `true` on the first blocked attempt (emit a SILENCED notice).
        initial: bool,
    },
}

impl TraceControl {
    /// Returns `true` if the log message should be emitted (`Ready` or `Unblocked`).
    #[inline]
    pub fn should_emit(&self) -> bool {
        matches!(self, Self::Ready | Self::Unblocked { .. })
    }

    /// Returns `true` if this is the first time the log point is suppressed.
    #[inline]
    pub fn is_initial_block(&self) -> bool {
        matches!(self, Self::Blocked { initial: true })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::observability::flow_control::TimeBasedFlowControl;

    fn make_limiter(count: usize, interval: Duration, reset: Duration) -> TraceLimiter {
        TraceLimiter::new(
            TimeBasedFlowControl::builder()
                .allow(count)
                .interval(interval)
                .reset_interval(reset)
                .build(),
        )
    }

    fn make_limiter_with_bypass(
        count: usize,
        interval: Duration,
        reset: Duration,
        bypass_level: Option<tracing::Level>,
    ) -> TraceLimiter {
        TraceLimiter::with_bypass_level(
            TimeBasedFlowControl::builder()
                .allow(count)
                .interval(interval)
                .reset_interval(reset)
                .build(),
            bypass_level,
        )
    }

    #[test]
    fn messages_pass_under_limit() {
        let limiter = make_limiter(5, Duration::from_secs(10), Duration::from_secs(60));
        for _ in 0..4 {
            assert!(
                matches!(limiter.check(tracing::Level::WARN), TraceControl::Ready),
                "should be Ready under limit"
            );
        }
    }

    #[test]
    fn blocked_after_limit() {
        let limiter = make_limiter(2, Duration::from_secs(10), Duration::from_secs(60));

        // First two pass
        assert!(limiter.check(tracing::Level::WARN).should_emit());
        assert!(limiter.check(tracing::Level::WARN).should_emit());

        // Third is initial block
        let ctrl = limiter.check(tracing::Level::WARN);
        assert!(ctrl.is_initial_block());
        assert!(!ctrl.should_emit());

        // Fourth is subsequent block
        let ctrl = limiter.check(tracing::Level::WARN);
        assert!(matches!(ctrl, TraceControl::Blocked { initial: false }));
        assert!(!ctrl.should_emit());
    }

    #[test]
    fn unblocked_with_count_after_silence() {
        let reset_ms = std::env::var_os("CI").map(|_| 500).unwrap_or(20);
        let reset = Duration::from_millis(reset_ms);
        let limiter = make_limiter(1, Duration::from_secs(10), reset);

        // One passes
        assert!(limiter.check(tracing::Level::WARN).should_emit());

        // Block
        let ctrl = limiter.check(tracing::Level::WARN);
        assert!(ctrl.is_initial_block());

        // More blocked attempts
        for _ in 0..5 {
            assert!(!limiter.check(tracing::Level::WARN).should_emit());
        }

        // Wait for reset
        std::thread::sleep(reset);

        // Should get Unblocked with count and summary
        let ctrl = limiter.check(tracing::Level::WARN);
        match ctrl {
            TraceControl::Unblocked {
                blocked_count,
                time_blocked,
                summary,
            } => {
                // 1 initial + 5 subsequent = 6 blocked attempts
                assert_eq!(blocked_count, 6);
                assert_eq!(summary.total(), 6);
                assert_eq!(summary.warn_count, 6);
                assert!(time_blocked >= reset);
            },
            other => panic!("expected Unblocked, got {other:?}"),
        }
    }

    // --- P0-2: bypass level tests ---

    #[test]
    fn error_bypasses_suppression() {
        let limiter = make_limiter_with_bypass(
            1,
            Duration::from_secs(10),
            Duration::from_secs(60),
            Some(tracing::Level::ERROR),
        );

        // First passes normally
        assert!(limiter.check(tracing::Level::WARN).should_emit());

        // WARN is blocked
        let ctrl = limiter.check(tracing::Level::WARN);
        assert!(ctrl.is_initial_block());

        // ERROR bypasses the block
        assert!(
            matches!(limiter.check(tracing::Level::ERROR), TraceControl::Ready),
            "ERROR should bypass suppression"
        );

        // WARN is still blocked
        assert!(!limiter.check(tracing::Level::WARN).should_emit());
    }

    #[test]
    fn bypass_does_not_reset_block_state() {
        let reset_ms = std::env::var_os("CI").map(|_| 500).unwrap_or(20);
        let reset = Duration::from_millis(reset_ms);
        let limiter = make_limiter_with_bypass(1, Duration::from_secs(10), reset, Some(tracing::Level::ERROR));

        // Exhaust the window
        assert!(limiter.check(tracing::Level::WARN).should_emit());
        assert!(limiter.check(tracing::Level::WARN).is_initial_block());

        // Block some WARN messages
        for _ in 0..3 {
            assert!(!limiter.check(tracing::Level::WARN).should_emit());
        }

        // Bypass with ERROR — block state should persist
        assert!(limiter.check(tracing::Level::ERROR).should_emit());

        // Wait for reset
        std::thread::sleep(reset);

        // Unblocked should report 4 blocked WARN messages + 1 bypassed ERROR
        let ctrl = limiter.check(tracing::Level::WARN);
        match ctrl {
            TraceControl::Unblocked {
                blocked_count, summary, ..
            } => {
                assert_eq!(blocked_count, 4);
                assert_eq!(summary.warn_count, 4);
                assert_eq!(summary.error_count, 0);
                assert_eq!(summary.bypassed_count, 1);
            },
            other => panic!("expected Unblocked, got {other:?}"),
        }
    }

    #[test]
    fn bypass_none_disables_bypass() {
        let limiter = make_limiter_with_bypass(1, Duration::from_secs(10), Duration::from_secs(60), None);

        // First passes
        assert!(limiter.check(tracing::Level::ERROR).should_emit());

        // Everything is blocked, even ERROR
        let ctrl = limiter.check(tracing::Level::ERROR);
        assert!(ctrl.is_initial_block());
    }

    #[test]
    fn bypass_level_warn_lets_warn_and_error_through() {
        let limiter = make_limiter_with_bypass(
            1,
            Duration::from_secs(10),
            Duration::from_secs(60),
            Some(tracing::Level::WARN),
        );

        // Exhaust the window
        assert!(limiter.check(tracing::Level::INFO).should_emit());

        // INFO is blocked
        assert!(limiter.check(tracing::Level::INFO).is_initial_block());

        // WARN and ERROR bypass
        assert!(limiter.check(tracing::Level::WARN).should_emit());
        assert!(limiter.check(tracing::Level::ERROR).should_emit());

        // INFO still blocked
        assert!(!limiter.check(tracing::Level::INFO).should_emit());
    }

    // --- P0-3: suppressed summary tests ---

    #[test]
    fn summary_tracks_per_level_counts() {
        let reset_ms = std::env::var_os("CI").map(|_| 500).unwrap_or(20);
        let reset = Duration::from_millis(reset_ms);
        // Disable bypass so all levels get blocked
        let limiter = make_limiter_with_bypass(1, Duration::from_secs(10), reset, None);

        // Exhaust window
        assert!(limiter.check(tracing::Level::WARN).should_emit());

        // Block various levels
        limiter.check(tracing::Level::ERROR); // initial block
        limiter.check(tracing::Level::ERROR);
        limiter.check(tracing::Level::WARN);
        limiter.check(tracing::Level::WARN);
        limiter.check(tracing::Level::WARN);
        limiter.check(tracing::Level::INFO);
        limiter.check(tracing::Level::DEBUG);

        // Wait for reset
        std::thread::sleep(reset);

        let ctrl = limiter.check(tracing::Level::WARN);
        match ctrl {
            TraceControl::Unblocked { summary, .. } => {
                assert_eq!(summary.error_count, 2);
                assert_eq!(summary.warn_count, 3);
                assert_eq!(summary.info_count, 1);
                assert_eq!(summary.debug_count, 1);
                assert_eq!(summary.trace_count, 0);
                assert_eq!(summary.total(), 7);
            },
            other => panic!("expected Unblocked, got {other:?}"),
        }
    }

    #[test]
    fn summary_display_single_level() {
        let s = SuppressedSummary {
            warn_count: 5,
            ..Default::default()
        };
        assert_eq!(s.to_string(), "5 messages");
    }

    #[test]
    fn summary_display_multiple_levels() {
        let s = SuppressedSummary {
            error_count: 2,
            warn_count: 10,
            info_count: 3,
            ..Default::default()
        };
        assert_eq!(s.to_string(), "15 messages (2 error, 10 warn, 3 info)");
    }

    #[test]
    fn summary_display_singular() {
        let s = SuppressedSummary {
            warn_count: 1,
            ..Default::default()
        };
        assert_eq!(s.to_string(), "1 message");
    }

    #[test]
    fn blocked_count_equals_summary_total() {
        let reset_ms = std::env::var_os("CI").map(|_| 500).unwrap_or(20);
        let reset = Duration::from_millis(reset_ms);
        let limiter = make_limiter(1, Duration::from_secs(10), reset);

        assert!(limiter.check(tracing::Level::WARN).should_emit());
        for _ in 0..10 {
            limiter.check(tracing::Level::WARN);
        }

        std::thread::sleep(reset);

        if let TraceControl::Unblocked {
            blocked_count, summary, ..
        } = limiter.check(tracing::Level::WARN)
        {
            assert_eq!(blocked_count, summary.total());
        }
    }
}
