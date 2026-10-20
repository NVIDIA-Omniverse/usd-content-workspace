// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Redelivery spacing shared by both durable queues.
//!
//! The broker requeues a negatively acknowledged delivery immediately, so any
//! spacing between attempts has to come from the consumer. Two pieces implement
//! it, and both queues use the same ones rather than reimplementing them:
//!
//! * [`RedeliveryTracker`] holds the per-delivery-key next-attempt time, which is
//!   what lets an early redelivery be recognised and rejected without redoing the
//!   work, and bounds how often that can happen before the session is recycled.
//! * [`DeferredNacks`] holds the negative acknowledgements themselves while their
//!   delay runs down.
//!
//! Both are keyed on the delivery's logical work identity rather than its
//! delivery tag, which is per-session and does not survive a redelivery.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Why a delivery is being negatively acknowledged.
///
/// Two operationally distinct conditions, which a bare NACK count cannot tell
/// apart. During dev verification the worker reported 31 NACKs against 14
/// retryable failures — the gap being early redeliveries rejected without
/// reprocessing. "We are retrying failed work" and "the broker is ignoring our
/// redelivery delay" call for different responses, so they are counted
/// separately.
///
/// Deliberately a *cause*, not a code path. Hitting the deferral bound or
/// flushing at teardown changes when the negative acknowledgement is sent, not
/// why the delivery needs one, so neither gets its own variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NackReason {
    /// Processing this delivery failed retryably.
    RetryableFailure,
    /// It arrived before its next-attempt time and was returned without being
    /// reprocessed.
    EarlyRedelivery,
}

impl NackReason {
    pub const ALL: [NackReason; 2] = [NackReason::RetryableFailure, NackReason::EarlyRedelivery];

    pub fn as_label(&self) -> &'static str {
        match self {
            NackReason::RetryableFailure => "retryable_failure",
            NackReason::EarlyRedelivery => "early_redelivery",
        }
    }

    /// Position in [`Self::ALL`], for indexing a per-reason counter array.
    pub fn index(&self) -> usize {
        match self {
            NackReason::RetryableFailure => 0,
            NackReason::EarlyRedelivery => 1,
        }
    }
}

/// Per-delivery-key state used to implement the anti-spin gate and circuit
/// breaker.
pub struct RedeliveryTracker {
    /// When each key is next allowed to enter processing. Set when a negative
    /// acknowledgement is scheduled; cleared on successful processing.
    next_attempt: HashMap<String, Instant>,
    /// Consecutive early redeliveries (arrived before `next_attempt`) per key
    /// since the last successful processing, with when the last one arrived.
    /// Drives the circuit breaker; the timestamp is what lets the entry age out
    /// for a key that went early once and then stopped coming back.
    early_redelivery_count: HashMap<String, (u32, Instant)>,
}

impl Default for RedeliveryTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl RedeliveryTracker {
    pub fn new() -> Self {
        Self {
            next_attempt: HashMap::new(),
            early_redelivery_count: HashMap::new(),
        }
    }

    /// Returns `true` if the key has a future next-attempt time, meaning this
    /// redelivery arrived early and must not re-enter processing.
    pub fn is_early(&self, key: &str) -> bool {
        self.next_attempt
            .get(key)
            .is_some_and(|t| *t > Instant::now())
    }

    /// Record that the key arrived early; returns the new consecutive count.
    pub fn record_early(&mut self, key: &str) -> u32 {
        let now = Instant::now();
        let state = self
            .early_redelivery_count
            .entry(key.to_string())
            .or_insert((0, now));
        state.0 = state.0.saturating_add(1);
        state.1 = now;
        state.0
    }

    /// Schedule the key's next-attempt time and reset its early-redelivery count.
    pub fn schedule_next_attempt(&mut self, key: &str, delay: Duration) {
        self.next_attempt
            .insert(key.to_string(), Instant::now() + delay);
        self.early_redelivery_count.remove(key);
    }

    /// Reset only the early-redelivery count, for a delivery that arrived on or
    /// after its next-attempt time and is about to be processed.
    pub fn clear_early_count(&mut self, key: &str) {
        self.early_redelivery_count.remove(key);
    }

    /// Clear all state for a key that has been processed successfully.
    pub fn clear(&mut self, key: &str) {
        self.next_attempt.remove(key);
        self.early_redelivery_count.remove(key);
    }

    /// Drop keys idle for longer than `older_than` — both the next-attempt time
    /// and the early-redelivery count.
    ///
    /// Without this the maps grow for the lifetime of a session: a key is only
    /// removed on success or reschedule, so a delivery that fails once and never
    /// returns leaves an entry behind. Sessions are long-lived by design, and the
    /// key space is the storage address space.
    ///
    /// A key still arriving early is kept, since dropping it would reset the
    /// circuit breaker mid-spin. That is judged on *when* it last arrived early,
    /// not merely on having an entry — keying off presence alone would pin both
    /// maps open forever for any key that ever went early once.
    pub fn evict_expired(&mut self, older_than: Duration) {
        let now = Instant::now();
        let idle = |at: &Instant| now.saturating_duration_since(*at) > older_than;

        let early = &self.early_redelivery_count;
        self.next_attempt.retain(|key, at| {
            !idle(at)
                || early
                    .get(key)
                    .is_some_and(|(_, last_early_at)| !idle(last_early_at))
        });
        self.early_redelivery_count
            .retain(|_, (_, last_early_at)| !idle(last_early_at));
    }

    #[cfg(test)]
    pub(crate) fn early_count(&self, key: &str) -> Option<u32> {
        self.early_redelivery_count
            .get(key)
            .map(|(count, _)| *count)
    }

    #[cfg(test)]
    pub(crate) fn tracked_keys(&self) -> usize {
        self.next_attempt.len()
    }

    #[cfg(test)]
    pub(crate) fn early_keys(&self) -> usize {
        self.early_redelivery_count.len()
    }
}

/// Deliveries whose negative acknowledgement is waiting out the redelivery delay.
///
/// This must be owned by a consume loop's own task, never slept on inside a
/// spawned processing task. A task sleeping out a delay longer than the session's
/// task-cleanup budget is aborted with the negative acknowledgement unsent, which
/// leaves the delivery in-flight until the broker's acknowledgement timeout
/// cancels the consumer — the failure this machinery exists to prevent. Holding
/// them here also makes the concurrency bound exact rather than a race between
/// tasks each reading a shared counter before incrementing it.
pub struct DeferredNacks {
    /// Send-at, delivery tag, and why it needs a negative acknowledgement. The
    /// reason travels with the entry because the send happens later, on the
    /// timer, by which point the originating branch is long gone.
    pending: Vec<(Instant, i64, NackReason)>,
    delay: Duration,
    limit: usize,
}

impl DeferredNacks {
    pub fn new(delay: Duration, limit: usize) -> Self {
        Self {
            pending: Vec::new(),
            delay,
            limit,
        }
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Hold `tag` for the configured delay. `false` means the concurrency bound
    /// is reached and the caller must negatively acknowledge immediately, so
    /// waiting work can never fill the prefetch window.
    pub fn try_defer(&mut self, tag: i64, reason: NackReason) -> bool {
        if self.pending.len() >= self.limit {
            return false;
        }
        self.pending
            .push((Instant::now() + self.delay, tag, reason));
        true
    }

    /// The earliest deadline, for an absolute `sleep_until` in the caller's
    /// `select!`. Absolute so rebuilding the future each iteration does not
    /// restart the wait.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.pending.iter().map(|(at, _, _)| *at).min()
    }

    /// Remove and return the tags whose delay has elapsed, with their reasons.
    pub fn take_due(&mut self) -> Vec<(i64, NackReason)> {
        let now = Instant::now();
        let mut due = Vec::new();
        self.pending.retain(|(at, tag, reason)| {
            if *at <= now {
                due.push((*tag, *reason));
                false
            } else {
                true
            }
        });
        due
    }

    /// Remove and return every held tag with its reason, for a teardown flush.
    /// The remaining delay is abandoned deliberately: the session is ending, so
    /// the reconnect supplies the spacing, and an unsent negative
    /// acknowledgement would leave the delivery in-flight until the broker's
    /// timeout. The reason stays the original cause — teardown changes when the
    /// negative acknowledgement is sent, not why it was needed.
    pub fn drain_all(&mut self) -> Vec<(i64, NackReason)> {
        self.pending
            .drain(..)
            .map(|(_, tag, reason)| (tag, reason))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_key_is_not_early() {
        let tracker = RedeliveryTracker::new();
        assert!(!tracker.is_early("omniverse://bucket/file.usd"));
    }

    #[test]
    fn a_key_with_a_future_next_attempt_is_early() {
        let mut tracker = RedeliveryTracker::new();
        tracker.schedule_next_attempt("omniverse://bucket/file.usd", Duration::from_secs(3600));
        assert!(tracker.is_early("omniverse://bucket/file.usd"));
    }

    #[test]
    fn a_key_whose_next_attempt_has_passed_is_not_early() {
        let mut tracker = RedeliveryTracker::new();
        tracker.schedule_next_attempt("key", Duration::ZERO);
        assert!(
            !tracker.is_early("key"),
            "a delay that has already elapsed must let the delivery through"
        );
    }

    #[test]
    fn clearing_a_key_makes_it_not_early() {
        let mut tracker = RedeliveryTracker::new();
        tracker.schedule_next_attempt("key", Duration::from_secs(3600));
        tracker.clear("key");
        assert!(!tracker.is_early("key"));
    }

    #[test]
    fn early_counts_accumulate_per_key() {
        let mut tracker = RedeliveryTracker::new();
        assert_eq!(tracker.record_early("a"), 1);
        assert_eq!(tracker.record_early("a"), 2);
        assert_eq!(tracker.record_early("b"), 1, "keys are independent");
    }

    #[test]
    fn scheduling_resets_the_early_count() {
        let mut tracker = RedeliveryTracker::new();
        tracker.record_early("key");
        tracker.record_early("key");
        tracker.schedule_next_attempt("key", Duration::from_secs(60));
        assert_eq!(tracker.early_count("key"), None);
    }

    /// A session runs for days and the key space is the storage address space, so
    /// entries for keys that never come back have to age out.
    #[test]
    fn an_idle_key_is_evicted_while_one_still_within_its_delay_is_kept() {
        let mut tracker = RedeliveryTracker::new();
        tracker.schedule_next_attempt("stale", Duration::ZERO);
        tracker.schedule_next_attempt("still-waiting", Duration::from_secs(3600));

        tracker.evict_expired(Duration::ZERO);

        assert!(
            tracker.next_attempt.contains_key("still-waiting"),
            "a key whose delay has not elapsed must be kept"
        );
        assert!(
            !tracker.next_attempt.contains_key("stale"),
            "an elapsed key with nothing else holding it must be evicted"
        );
        assert_eq!(tracker.tracked_keys(), 1);
    }

    /// A key still arriving early is mid-spin, and dropping it would reset the
    /// circuit breaker just as it was about to fire.
    #[test]
    fn a_key_that_recently_arrived_early_is_kept() {
        let mut tracker = RedeliveryTracker::new();
        tracker.schedule_next_attempt("spinning", Duration::ZERO);
        tracker.record_early("spinning");

        // A window the just-recorded early arrival is comfortably inside.
        tracker.evict_expired(Duration::from_secs(3600));

        assert_eq!(tracker.early_count("spinning"), Some(1));
        assert!(
            tracker.next_attempt.contains_key("spinning"),
            "an actively spinning key must keep its next-attempt time"
        );
    }

    /// Judging "still early" on presence alone rather than recency pinned both
    /// maps open for the life of the session for any key that ever went early —
    /// and left `early_redelivery_count` itself never pruned at all.
    #[test]
    fn early_state_does_not_pin_a_key_open_forever() {
        let mut tracker = RedeliveryTracker::new();
        tracker.schedule_next_attempt("abandoned", Duration::ZERO);
        tracker.record_early("abandoned");
        assert_eq!(tracker.early_keys(), 1);

        // Any elapsed time counts as idle against a zero retention window.
        std::thread::sleep(Duration::from_millis(2));
        tracker.evict_expired(Duration::ZERO);

        assert_eq!(
            tracker.tracked_keys(),
            0,
            "an idle key must not be held open by a stale early count"
        );
        assert_eq!(
            tracker.early_keys(),
            0,
            "the early-redelivery map must be pruned too, not only next_attempt"
        );
    }

    #[test]
    fn deferral_holds_up_to_the_limit_then_refuses() {
        let mut deferred = DeferredNacks::new(Duration::from_secs(60), 2);
        assert!(deferred.try_defer(1, NackReason::RetryableFailure));
        assert!(deferred.try_defer(2, NackReason::RetryableFailure));
        assert!(
            !deferred.try_defer(3, NackReason::RetryableFailure),
            "past the bound the caller must NACK immediately instead of holding"
        );
        assert_eq!(deferred.len(), 2);
    }

    #[test]
    fn only_elapsed_deferrals_come_due() {
        let mut deferred = DeferredNacks::new(Duration::from_secs(3600), 4);
        deferred.try_defer(1, NackReason::RetryableFailure);
        assert!(
            deferred.take_due().is_empty(),
            "a deferral whose delay has not elapsed must not be sent"
        );
        assert_eq!(deferred.len(), 1);

        let mut due_now = DeferredNacks::new(Duration::ZERO, 4);
        due_now.try_defer(7, NackReason::EarlyRedelivery);
        assert_eq!(
            due_now.take_due(),
            vec![(7, NackReason::EarlyRedelivery)],
            "the reason must survive the deferral, since the send happens later"
        );
        assert!(due_now.is_empty());
    }

    #[test]
    fn the_next_deadline_is_the_earliest_held() {
        let mut deferred = DeferredNacks::new(Duration::from_secs(3600), 4);
        assert!(deferred.next_deadline().is_none());
        deferred.try_defer(1, NackReason::RetryableFailure);
        let first = deferred.next_deadline().expect("a deadline is held");
        deferred.try_defer(2, NackReason::RetryableFailure);
        assert_eq!(
            deferred.next_deadline(),
            Some(first),
            "the earliest deadline must drive the timer"
        );
    }

    /// The teardown flush is what keeps a session end from abandoning a delivery.
    #[test]
    fn draining_returns_every_held_tag_regardless_of_delay() {
        let mut deferred = DeferredNacks::new(Duration::from_secs(3600), 4);
        deferred.try_defer(11, NackReason::RetryableFailure);
        deferred.try_defer(12, NackReason::EarlyRedelivery);
        let mut drained = deferred.drain_all();
        drained.sort_unstable_by_key(|(tag, _)| *tag);
        assert_eq!(
            drained,
            vec![
                (11, NackReason::RetryableFailure),
                (12, NackReason::EarlyRedelivery)
            ],
            "a teardown flush keeps each delivery's original cause"
        );
        assert!(deferred.is_empty());
    }
}
