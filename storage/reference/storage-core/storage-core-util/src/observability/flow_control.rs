// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Flow control primitives for rate limiting.
//!
//! This module provides the [`FlowControl`] trait and [`TimeBasedFlowControl`], a time-window
//! based rate limiter. Most users won't interact with this module directly — instead, use
//! the [`TraceLimiter`](super::trace_limited::TraceLimiter) and the `warn_limited!` family of macros
//! which wrap `FlowControl` for per-call-site log rate limiting.
//!
//! # How `TimeBasedFlowControl` works
//!
//! The limiter tracks a sliding time window. Within each window, up to `count` calls to
//! [`reserve()`](FlowControl::reserve) return [`FlowState::Ready`]. The call that hits the
//! limit returns [`FlowState::Restricted`] (still actionable, but signals the window is
//! exhausted). Subsequent calls return [`FlowState::Blocked`] until the silence/reset
//! interval expires, at which point the window resets.
//!
//! # Example
//!
//! ```
//! use storage_core_util::observability::flow_control::{FlowControl, TimeBasedFlowControl};
//! use std::time::Duration;
//!
//! let limiter = TimeBasedFlowControl::builder()
//!     .allow(5)
//!     .interval(Duration::from_secs(1))
//!     .reset_interval(Duration::from_secs(10))
//!     .build();
//!
//! // First 4 calls are Ready
//! for _ in 0..4 {
//!     assert!(limiter.reserve().should_perform());
//! }
//! // 5th call is Restricted (still performable, but signals limit reached)
//! let state = limiter.reserve();
//! assert!(state.should_perform());
//! assert!(state.wait_until().is_some());
//! ```

use std::{
    marker,
    sync::Mutex,
    time::{Duration, Instant},
};

/// A flow control strategy that decides whether an action should be performed.
pub trait FlowControl {
    /// Request permission to perform an action. Returns a [`FlowState`] indicating whether the
    /// action should proceed.
    fn reserve(&self) -> FlowState;
}

/// Result of a [`FlowControl::reserve`] call.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[must_use = "A FlowControl can only show you the appropriate action, you must be the one to perform it"]
pub enum FlowState {
    /// Not suppressed: the action should be performed immediately.
    Ready,
    /// Restricted: the action should be performed, but wait until the [`Instant`] before trying
    /// again. This is the transition state when the count limit is hit.
    Restricted(Instant),
    /// Suppressed: the action should not be performed. After the [`Instant`], the controller
    /// should be queried again.
    Blocked(Instant),
}

impl FlowState {
    /// Returns `true` if the action should be performed (`Ready` or `Restricted`).
    #[inline]
    pub fn should_perform(&self) -> bool {
        matches!(self, Self::Ready | Self::Restricted(_))
    }

    /// If restricted or blocked, returns the time to wait until.
    pub fn wait_until(&self) -> Option<Instant> {
        match self {
            Self::Ready => None,
            Self::Restricted(until) | Self::Blocked(until) => Some(*until),
        }
    }
}

/// Configuration for [`TimeBasedFlowControl`].
///
/// Use [`TimeBasedFlowControlConfig::builder()`] for ergonomic construction.
#[derive(Clone, Debug)]
pub struct TimeBasedFlowControlConfig {
    count: usize,
    interval: Duration,
    reset_interval: Option<Duration>,
}

impl TimeBasedFlowControlConfig {
    /// Create a builder for this config.
    pub fn builder() -> TimeBasedFlowControlBuilder<Self> {
        TimeBasedFlowControlBuilder::default()
    }
}

impl Default for TimeBasedFlowControlConfig {
    fn default() -> Self {
        Self {
            count: 50,
            interval: Duration::from_millis(500),
            reset_interval: None,
        }
    }
}

/// Builder for [`TimeBasedFlowControlConfig`] and [`TimeBasedFlowControl`].
pub struct TimeBasedFlowControlBuilder<FC> {
    config: TimeBasedFlowControlConfig,
    _out: marker::PhantomData<FC>,
}

impl<FC> Default for TimeBasedFlowControlBuilder<FC> {
    fn default() -> Self {
        Self {
            config: Default::default(),
            _out: marker::PhantomData,
        }
    }
}

impl<FC> TimeBasedFlowControlBuilder<FC> {
    /// Allow `count` actions in the interval before triggering the blocked state.
    pub fn allow(mut self, count: usize) -> Self {
        self.config.count = count;
        self
    }

    /// The time window during which up to the allowed count of actions may occur.
    pub fn interval(mut self, value: impl Into<Duration>) -> Self {
        self.config.interval = value.into();
        self
    }

    /// How long to stay blocked after exceeding the limit. If unset, defaults to `interval`.
    pub fn reset_interval(mut self, value: impl Into<Duration>) -> Self {
        self.config.reset_interval = Some(value.into());
        self
    }
}

impl TimeBasedFlowControlBuilder<TimeBasedFlowControl> {
    /// Build a [`TimeBasedFlowControl`].
    pub fn build(self) -> TimeBasedFlowControl {
        TimeBasedFlowControl::with_config(self.config)
    }
}

impl TimeBasedFlowControlBuilder<TimeBasedFlowControlConfig> {
    /// Build the config without creating a flow control instance.
    pub fn build(self) -> TimeBasedFlowControlConfig {
        self.config
    }
}

/// Thread-safe, time-window based rate limiter.
///
/// Allows up to `count` actions per `interval`. After exceeding the limit, blocks for
/// `reset_interval` (or `interval` if unset) before allowing actions again.
pub struct TimeBasedFlowControl {
    config: TimeBasedFlowControlConfig,
    state: Mutex<TimeBasedFlowControlState>,
}

impl TimeBasedFlowControl {
    /// Create a builder.
    pub fn builder() -> TimeBasedFlowControlBuilder<Self> {
        TimeBasedFlowControlBuilder::default()
    }

    /// Create from an existing config.
    pub fn with_config(config: TimeBasedFlowControlConfig) -> Self {
        Self {
            config,
            state: Mutex::new(TimeBasedFlowControlState::new()),
        }
    }
}

impl FlowControl for TimeBasedFlowControl {
    fn reserve(&self) -> FlowState {
        let Ok(mut state) = self.state.lock() else {
            // Fail-open on poison: logging must never block or crash the application.
            // The critical section (reserve) is trivial arithmetic and cannot panic.
            return FlowState::Ready;
        };
        state.reserve(&self.config)
    }
}

#[derive(Debug)]
struct TimeBasedFlowControlState {
    blocked_until: Option<Instant>,
    window_end_time: Instant,
    events_in_window: usize,
}

impl TimeBasedFlowControlState {
    fn new() -> Self {
        Self {
            blocked_until: None,
            window_end_time: Instant::now(),
            events_in_window: 0,
        }
    }

    fn reserve(&mut self, config: &TimeBasedFlowControlConfig) -> FlowState {
        let now = Instant::now();
        if let Some(blocked_until) = self.blocked_until {
            if now < blocked_until {
                return FlowState::Blocked(blocked_until);
            }
            self.blocked_until = None;
        }

        if self.window_end_time < now {
            self.window_end_time = now + config.interval;
            self.events_in_window = 0;
        }

        self.events_in_window += 1;
        // count == 0 means "block after first event" (not "disable rate limiting")
        if self.events_in_window >= config.count.max(1) {
            let block_until = if let Some(reset_interval) = config.reset_interval {
                now + reset_interval
            } else {
                self.window_end_time
            };
            self.blocked_until = Some(block_until);
            return FlowState::Restricted(block_until);
        }

        FlowState::Ready
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{FlowControl, FlowState, TimeBasedFlowControl};

    #[test]
    fn simple_short_timer_until_block() {
        let allow_count = 100;
        let interval = Duration::from_secs(10);
        let before_first_message = Instant::now();
        let suppressor = TimeBasedFlowControl::builder()
            .allow(allow_count)
            .interval(interval)
            .build();

        for trial in 1..=allow_count {
            let resp = suppressor.reserve();
            assert!(resp.should_perform());
            if trial == allow_count {
                assert!(resp.wait_until().is_some(), "last call should have a wait");
            } else {
                assert!(
                    resp.wait_until().is_none(),
                    "trial {trial}/{allow_count} should not get push back"
                );
            }
        }
        let after_last_message = Instant::now();

        let resp = suppressor.reserve();
        let FlowState::Blocked(until) = resp else {
            panic!("should have been blocked, but {resp:?} instead");
        };

        let min_until = before_first_message + interval;
        let max_until = after_last_message + interval;
        assert!(min_until <= until);
        assert!(until <= max_until);
    }

    #[test]
    fn hammer_unblock() {
        let suppressor = TimeBasedFlowControl::builder()
            .allow(5)
            .interval(Duration::from_millis(std::env::var_os("CI").map(|_| 500).unwrap_or(10)))
            .build();

        for _ in 0..5 {
            assert!(suppressor.reserve().should_perform());
        }
        let FlowState::Blocked(unblock_time) = suppressor.reserve() else {
            panic!("we should be blocked");
        };
        loop {
            match suppressor.reserve() {
                FlowState::Ready | FlowState::Restricted(_) => break,
                FlowState::Blocked(until) => assert_eq!(unblock_time, until),
            }
        }
        assert!(unblock_time <= Instant::now());
    }

    #[test]
    fn reset_interval_respected() {
        let reset = Duration::from_millis(std::env::var_os("CI").map(|_| 500).unwrap_or(20));
        let suppressor = TimeBasedFlowControl::builder()
            .allow(1)
            .interval(Duration::from_secs(10))
            .reset_interval(reset)
            .build();

        // First call is ready
        assert!(suppressor.reserve().should_perform());
        // Second call should be blocked
        let FlowState::Blocked(_) = suppressor.reserve() else {
            panic!("should be blocked");
        };

        // Wait for the reset interval
        std::thread::sleep(reset);

        // Should be unblocked now
        assert!(suppressor.reserve().should_perform());
    }
}
