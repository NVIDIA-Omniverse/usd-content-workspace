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

//! Aggregate visibility, since there is no task table any more.
//!
//! Retiring Postgres removed `GET /tasks/{id}`, Flower, and `/celery/stats`.
//! Combined with permanent failures being acknowledged, a poison thumbnail would
//! otherwise leave no trace at all: no DLQ entry, no task row, no event. Losing
//! per-item queryability is accepted; losing all visibility is not.
//!
//! Each metric family is instrumented twice:
//!
//! - **OTel counter / observable gauge** — primary export path; the SDK exports
//!   to whatever OTLP backend is configured (or nowhere when disabled).
//! - **Shadow `AtomicU64` / `AtomicUsize`** — local read-back; `render()` reads
//!   these to produce the Prometheus text that the probe surface serves on `/metrics`.
//!
//! The two are kept in sync: every mutation updates the OTel instrument *and* the
//! corresponding atomic. For the unacknowledged-deliveries gauge the atomic is
//! shared via `Arc` with the OTel observable callback so both read the same value.

use std::sync::{
    atomic::{AtomicU64, AtomicUsize, Ordering},
    Arc,
};

use opentelemetry::metrics::{Counter, Histogram, Meter, ObservableGauge};
use opentelemetry::KeyValue;

use crate::outcome::{FailureClass, WorkOutcome};
use thumbnail_common::notifications::redelivery::NackReason;

/// One counter per class, held by position in [`FailureClass::ALL`].
///
/// An array rather than a field per class: at ten stages the hand-written
/// struct-and-match version was two more places to forget a variant, and the
/// whole point of splitting the classes is that none of them goes unreported.
struct ClassCounters([AtomicU64; FailureClass::ALL.len()]);

impl Default for ClassCounters {
    fn default() -> Self {
        Self(std::array::from_fn(|_| AtomicU64::new(0)))
    }
}

impl ClassCounters {
    fn counter(&self, class: FailureClass) -> &AtomicU64 {
        &self.0[class.index()]
    }
}

/// Why a consume session ended.
///
/// A label rather than a metric per cause, because the operator's question is a
/// breakdown: "it is reconnecting, why". Every variant is emitted at zero so a
/// cause that has not occurred is visibly absent rather than missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEndCause {
    /// Withheld acknowledgements forced a teardown, or a conversion panicked.
    Recycle,
    /// The bearer was near expiry and the session was reopened deliberately.
    TokenExpiring,
    /// The stream failed authentication and the token was refreshed.
    AuthRefreshed,
    /// The durable queue no longer exists and is being re-provisioned.
    QueueMissing,
    /// The transport failed, or the stream simply ended.
    Interrupted,
}

impl SessionEndCause {
    pub const ALL: [Self; 5] = [
        Self::Recycle,
        Self::TokenExpiring,
        Self::AuthRefreshed,
        Self::QueueMissing,
        Self::Interrupted,
    ];

    pub fn as_label(self) -> &'static str {
        match self {
            Self::Recycle => "recycle",
            Self::TokenExpiring => "token_expiring",
            Self::AuthRefreshed => "auth_refreshed",
            Self::QueueMissing => "queue_missing",
            Self::Interrupted => "interrupted",
        }
    }
}

/// Upper bounds, in seconds, for the conversion-duration histogram.
///
/// Chosen around the observed working range rather than a generic ladder: the
/// load test showed conversions of 0.3-3.2 MB JPEGs completing well under a
/// second, so the resolution is concentrated there, with the long tail left
/// coarse because what matters above ten seconds is only that it happened.
const CONVERSION_DURATION_BUCKETS: [f64; 9] = [0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0];

pub struct WorkerMetrics {
    // ── OTel instruments (primary OTLP export) ───────────────────────────────
    outcomes_counter: Counter<u64>,
    failures_counter: Counter<u64>,
    conversion_duration_histogram: Histogram<f64>,
    recycles_counter: Counter<u64>,
    token_reconnects_counter: Counter<u64>,
    orphaned_blobs_counter: Counter<u64>,
    session_ends_counter: Counter<u64>,
    nacks_counter: Counter<u64>,
    /// Observable gauge whose callback reads from [`Self::unacknowledged`].
    /// The leading `_` suppresses the dead_code lint; the field must be kept
    /// alive for the duration of the process so the callback continues to fire.
    _unacknowledged_gauge: ObservableGauge<u64>,

    // ── Shadow atomics (local read-back for render()) ─────────────────────────
    succeeded: AtomicU64,
    permanent: AtomicU64,
    retryable: AtomicU64,
    by_class: ClassCounters,
    /// Cumulative seconds, as microseconds, for the histogram's `_sum`.
    conversion_duration_micros_total: AtomicU64,
    /// Observations per bucket, by position in [`CONVERSION_DURATION_BUCKETS`],
    /// plus one trailing slot for `+Inf`.
    conversion_duration_buckets: [AtomicU64; CONVERSION_DURATION_BUCKETS.len() + 1],
    conversion_duration_count: AtomicU64,
    recycles: AtomicU64,
    token_reconnects: AtomicU64,
    orphaned_blobs: AtomicU64,
    /// One counter per [`SessionEndCause`], held by discriminant.
    session_ends: [AtomicU64; SessionEndCause::ALL.len()],
    /// One counter per [`NackReason`], held by position in `NackReason::ALL`.
    nacks: [AtomicU64; NackReason::ALL.len()],
    /// Shared with the observable gauge callback so both the gauge export and
    /// `render()` read from the same authoritative value.
    unacknowledged: Arc<AtomicUsize>,
}

impl WorkerMetrics {
    /// Create a `WorkerMetrics` wired to the given OTel meter.
    ///
    /// In production the meter comes from `opentelemetry::global::meter()`,
    /// which is set by `init_tracing()`. In tests use [`Self::new_for_tests`].
    pub fn new(meter: Meter) -> Self {
        let outcomes_counter = meter
            .u64_counter("thumbnail_worker_outcomes_total")
            .with_description("Generate messages by terminal outcome.")
            .build();
        let failures_counter = meter
            .u64_counter("thumbnail_worker_failures_total")
            .with_description("Failures by the pipeline stage that produced them.")
            .build();
        let conversion_duration_histogram = meter
            .f64_histogram("thumbnail_worker_conversion_duration_seconds")
            .with_description(
                "Time spent converting a generate message, for messages that reached conversion.",
            )
            .with_unit("s")
            .with_boundaries(CONVERSION_DURATION_BUCKETS.to_vec())
            .build();
        let recycles_counter = meter
            .u64_counter("thumbnail_worker_stream_recycles_total")
            .with_description("Stream teardowns to force redelivery of withheld acknowledgements.")
            .build();
        let token_reconnects_counter = meter
            .u64_counter("thumbnail_worker_stream_token_reconnects_total")
            .with_description("Stream teardowns after a successful near-expiry token refresh.")
            .build();
        let orphaned_blobs_counter = meter
            .u64_counter("thumbnail_worker_orphaned_blobs_total")
            .with_description(
                "Uploads that succeeded and were then permanently rejected by associate, \
                 leaving a blob no metadata references.",
            )
            .build();
        let session_ends_counter = meter
            .u64_counter("thumbnail_worker_session_ends_total")
            .with_description("Consume sessions ended, by cause.")
            .build();
        let nacks_counter = meter
            .u64_counter("thumbnail_worker_nacks_total")
            .with_description("Deliveries negatively acknowledged for redelivery, by cause.")
            .build();

        let unacknowledged = Arc::new(AtomicUsize::new(0));
        let gauge_source = Arc::clone(&unacknowledged);
        let _unacknowledged_gauge = meter
            .u64_observable_gauge("thumbnail_worker_unacknowledged_deliveries")
            .with_description(
                "Deliveries received and not yet acknowledged, in flight plus withheld.",
            )
            .with_callback(move |instrument| {
                instrument.observe(gauge_source.load(Ordering::Relaxed) as u64, &[]);
            })
            .build();

        Self {
            outcomes_counter,
            failures_counter,
            conversion_duration_histogram,
            recycles_counter,
            token_reconnects_counter,
            orphaned_blobs_counter,
            session_ends_counter,
            nacks_counter,
            _unacknowledged_gauge,
            succeeded: AtomicU64::new(0),
            permanent: AtomicU64::new(0),
            retryable: AtomicU64::new(0),
            by_class: ClassCounters::default(),
            conversion_duration_micros_total: AtomicU64::new(0),
            conversion_duration_buckets: std::array::from_fn(|_| AtomicU64::new(0)),
            conversion_duration_count: AtomicU64::new(0),
            recycles: AtomicU64::new(0),
            token_reconnects: AtomicU64::new(0),
            orphaned_blobs: AtomicU64::new(0),
            session_ends: std::array::from_fn(|_| AtomicU64::new(0)),
            nacks: std::array::from_fn(|_| AtomicU64::new(0)),
            unacknowledged,
        }
    }

    /// Test-only constructor using a no-op meter so tests never need a live
    /// OTLP exporter.
    ///
    /// Always compiled and `pub` so integration tests in `tests/` can reach it.
    /// Integration tests compile the library without `cfg(test)`, making
    /// `#[cfg(test)]`-gated items invisible. This is a no-op meter and safe
    /// to expose; the name makes the intent clear.
    pub fn new_for_tests() -> Self {
        let guard = otel_tools::exporters::metrics::MetricsGuard::disabled_for_test();
        Self::new(guard.meter_or_noop("thumbnail-task-worker"))
    }

    /// Record a terminal outcome, labelled from the outcome's own class rather
    /// than inferred from whether an acknowledgement was sent.
    pub fn record(&self, outcome: &WorkOutcome, conversion_millis: u64) {
        match outcome {
            WorkOutcome::Succeeded { .. } => {
                self.succeeded.fetch_add(1, Ordering::Relaxed);
                self.outcomes_counter
                    .add(1, &[KeyValue::new("outcome", "success")]);
            }
            WorkOutcome::Permanent { class, .. } => {
                self.permanent.fetch_add(1, Ordering::Relaxed);
                self.by_class
                    .counter(*class)
                    .fetch_add(1, Ordering::Relaxed);
                self.outcomes_counter
                    .add(1, &[KeyValue::new("outcome", "permanent_failure")]);
                self.failures_counter
                    .add(1, &[KeyValue::new("class", class.as_label())]);
            }
            WorkOutcome::Retryable { class, .. } => {
                self.retryable.fetch_add(1, Ordering::Relaxed);
                self.by_class
                    .counter(*class)
                    .fetch_add(1, Ordering::Relaxed);
                self.outcomes_counter
                    .add(1, &[KeyValue::new("outcome", "retryable_failure")]);
                self.failures_counter
                    .add(1, &[KeyValue::new("class", class.as_label())]);
            }
        }
        // Only messages that actually converted. A delivery rejected for its
        // schema or its format never decoded anything, so folding its elapsed
        // time into conversion latency depresses the figure with work that is not
        // conversion — and those rejections are fast, which is what makes the
        // distortion systematic rather than noise.
        if outcome.reached_conversion() {
            self.observe_conversion_duration(conversion_millis);
        }
    }

    /// Record one conversion duration into both the OTel histogram and the
    /// shadow buckets `render()` reads.
    ///
    /// A distribution rather than a cumulative total: a total divided by a count
    /// yields a mean and nothing else, so it cannot answer whether a latency rise
    /// is the whole population shifting or a tail forming. That question is the
    /// reason latency telemetry is kept.
    fn observe_conversion_duration(&self, millis: u64) {
        let seconds = millis as f64 / 1_000.0;
        self.conversion_duration_histogram.record(seconds, &[]);

        self.conversion_duration_micros_total
            .fetch_add(millis.saturating_mul(1_000), Ordering::Relaxed);
        self.conversion_duration_count
            .fetch_add(1, Ordering::Relaxed);

        // Cumulative buckets: an observation counts in its own bucket and every
        // wider one, which is what the Prometheus histogram format means by
        // `le`. The trailing slot is `+Inf`.
        let first = CONVERSION_DURATION_BUCKETS
            .iter()
            .position(|bound| seconds <= *bound)
            .unwrap_or(CONVERSION_DURATION_BUCKETS.len());
        for bucket in self.conversion_duration_buckets.iter().skip(first) {
            bucket.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn delivery_received(&self) {
        self.unacknowledged.fetch_add(1, Ordering::Relaxed);
    }

    pub fn delivery_acknowledged(&self) {
        self.unacknowledged.fetch_sub(1, Ordering::Relaxed);
    }

    /// A negative acknowledgement ends a delivery's in-flight state just as an
    /// acknowledgement does, so it has to release the gauge too. Counting it
    /// separately is what keeps retries visible now that the recycle — which used
    /// to be the retry signal, and reset the gauge as a side effect — is rare.
    ///
    /// Labelled by `reason`, because the bare count conflates two conditions an
    /// operator has to act on differently: work that failed and is being retried,
    /// versus a broker redelivering ahead of the requested delay so the consumer
    /// keeps rejecting it. Dev verification showed 31 NACKs against 14 retryable
    /// failures, and nothing in the metric explained the gap.
    ///
    /// Call this only once the negative acknowledgement has actually been sent. A
    /// NACK that could not be delivered leaves the delivery in flight and the
    /// worker recycles, so counting it here would report a redelivery that never
    /// happened and release a gauge slot that is still occupied. The recycle's
    /// `reset_unacknowledged` is what clears those.
    pub fn delivery_nacked(&self, reason: NackReason) {
        self.unacknowledged.fetch_sub(1, Ordering::Relaxed);
        self.nacks[reason.index()].fetch_add(1, Ordering::Relaxed);
        self.nacks_counter
            .add(1, &[KeyValue::new("reason", reason.as_label())]);
    }

    /// Reset on reconnect: the platform redelivers everything unacknowledged, so
    /// the previous session's count no longer describes anything.
    pub fn reset_unacknowledged(&self) {
        self.unacknowledged.store(0, Ordering::Relaxed);
    }

    pub fn unacknowledged(&self) -> usize {
        self.unacknowledged.load(Ordering::Relaxed)
    }

    pub fn recycled(&self) {
        self.recycles.fetch_add(1, Ordering::Relaxed);
        self.recycles_counter.add(1, &[]);
    }

    pub fn token_reconnected(&self) {
        self.token_reconnects.fetch_add(1, Ordering::Relaxed);
        self.token_reconnects_counter.add(1, &[]);
    }

    /// A consume session ended, labelled with why.
    ///
    /// `recycles_total` and `token_reconnects_total` already existed, but the
    /// remaining causes were logged and not counted — so a deployment reconnecting
    /// continuously because the transport kept dropping looked identical, in
    /// telemetry, to a healthy one. A reconnect rate is only interpretable if
    /// every cause contributes to it.
    pub fn session_ended(&self, cause: SessionEndCause) {
        self.session_ends[cause as usize].fetch_add(1, Ordering::Relaxed);
        self.session_ends_counter
            .add(1, &[KeyValue::new("cause", cause.as_label())]);
    }

    /// An upload succeeded and the association was then permanently rejected.
    ///
    /// Counted separately from the `associate` failure class, which cannot say
    /// whether the upload had happened yet. The distinction is the whole point:
    /// a failure before the upload leaves nothing behind, while one after it
    /// leaves a blob at the destination that no metadata references — and the
    /// storage-deleted sweep cannot find it, because that sweep enumerates
    /// association metadata on the source. Without this counter the population
    /// is invisible and grows.
    pub fn blob_orphaned(&self) {
        self.orphaned_blobs.fetch_add(1, Ordering::Relaxed);
        self.orphaned_blobs_counter.add(1, &[]);
    }

    /// Prometheus text exposition for the probe surface.
    ///
    /// Reads from the shadow atomics rather than from the OTel instruments,
    /// since OTel counters are write-only with no read-back API. The atomics
    /// are kept in sync with the OTel instruments by every mutation method.
    pub fn render(&self) -> String {
        let mut out = String::new();

        family(
            &mut out,
            "thumbnail_worker_outcomes_total",
            "counter",
            "Generate messages by terminal outcome.",
        );
        for (outcome, value) in [
            ("success", &self.succeeded),
            ("permanent_failure", &self.permanent),
            ("retryable_failure", &self.retryable),
        ] {
            sample(
                &mut out,
                "thumbnail_worker_outcomes_total",
                Some(("outcome", outcome)),
                value.load(Ordering::Relaxed),
            );
        }

        family(
            &mut out,
            "thumbnail_worker_failures_total",
            "counter",
            "Failures by the pipeline stage that produced them.",
        );
        for class in FailureClass::ALL {
            sample(
                &mut out,
                "thumbnail_worker_failures_total",
                Some(("class", class.as_label())),
                self.by_class.counter(class).load(Ordering::Relaxed),
            );
        }

        family(
            &mut out,
            "thumbnail_worker_conversion_duration_seconds",
            "histogram",
            "Time spent converting a generate message, for messages that reached conversion.",
        );
        for (index, bound) in CONVERSION_DURATION_BUCKETS.iter().enumerate() {
            out.push_str(&format!(
                "thumbnail_worker_conversion_duration_seconds_bucket{{le=\"{bound}\"}} {}\n",
                self.conversion_duration_buckets[index].load(Ordering::Relaxed)
            ));
        }
        out.push_str(&format!(
            "thumbnail_worker_conversion_duration_seconds_bucket{{le=\"+Inf\"}} {}\n",
            self.conversion_duration_buckets[CONVERSION_DURATION_BUCKETS.len()]
                .load(Ordering::Relaxed)
        ));
        out.push_str(&format!(
            "thumbnail_worker_conversion_duration_seconds_sum {}\n",
            self.conversion_duration_micros_total
                .load(Ordering::Relaxed) as f64
                / 1_000_000.0
        ));
        out.push_str(&format!(
            "thumbnail_worker_conversion_duration_seconds_count {}\n",
            self.conversion_duration_count.load(Ordering::Relaxed)
        ));

        family(
            &mut out,
            "thumbnail_worker_unacknowledged_deliveries",
            "gauge",
            "Deliveries received and not yet acknowledged, in flight plus withheld.",
        );
        sample(
            &mut out,
            "thumbnail_worker_unacknowledged_deliveries",
            None,
            self.unacknowledged.load(Ordering::Relaxed) as u64,
        );

        family(
            &mut out,
            "thumbnail_worker_nacks_total",
            "counter",
            "Deliveries negatively acknowledged for redelivery, by cause.",
        );
        for reason in NackReason::ALL {
            sample(
                &mut out,
                "thumbnail_worker_nacks_total",
                Some(("reason", reason.as_label())),
                self.nacks[reason.index()].load(Ordering::Relaxed),
            );
        }

        family(
            &mut out,
            "thumbnail_worker_stream_recycles_total",
            "counter",
            "Stream teardowns to force redelivery of withheld acknowledgements.",
        );
        sample(
            &mut out,
            "thumbnail_worker_stream_recycles_total",
            None,
            self.recycles.load(Ordering::Relaxed),
        );

        family(
            &mut out,
            "thumbnail_worker_session_ends_total",
            "counter",
            "Consume sessions ended, by cause.",
        );
        for cause in SessionEndCause::ALL {
            sample(
                &mut out,
                "thumbnail_worker_session_ends_total",
                Some(("cause", cause.as_label())),
                self.session_ends[cause as usize].load(Ordering::Relaxed),
            );
        }

        family(
            &mut out,
            "thumbnail_worker_orphaned_blobs_total",
            "counter",
            "Uploads left unreferenced by a permanent associate rejection.",
        );
        sample(
            &mut out,
            "thumbnail_worker_orphaned_blobs_total",
            None,
            self.orphaned_blobs.load(Ordering::Relaxed),
        );

        family(
            &mut out,
            "thumbnail_worker_stream_token_reconnects_total",
            "counter",
            "Stream teardowns after a successful near-expiry token refresh.",
        );
        sample(
            &mut out,
            "thumbnail_worker_stream_token_reconnects_total",
            None,
            self.token_reconnects.load(Ordering::Relaxed),
        );

        out
    }
}

/// Emit the `# HELP` and `# TYPE` lines for one metric family.
///
/// Exactly once per family, before its samples: a repeat is a parse error in
/// stricter scrapers, which is the trap in emitting these alongside each labelled
/// sample instead.
fn family(out: &mut String, name: &str, kind: &str, help: &str) {
    out.push_str(&format!("# HELP {name} {help}\n# TYPE {name} {kind}\n"));
}

fn sample(out: &mut String, name: &str, label: Option<(&str, &str)>, value: u64) {
    match label {
        Some((key, value_label)) => {
            out.push_str(&format!("{name}{{{key}=\"{value_label}\"}} {value}\n"))
        }
        None => out.push_str(&format!("{name} {value}\n")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcomes_are_counted_by_terminal_class() {
        let metrics = WorkerMetrics::new_for_tests();

        metrics.record(
            &WorkOutcome::Succeeded {
                destination: "d".to_string(),
            },
            12,
        );
        metrics.record(
            &WorkOutcome::permanent(FailureClass::UnsupportedFormat, "d", "heic"),
            3,
        );
        metrics.record(
            &WorkOutcome::retryable(FailureClass::Upload, "d", "unavailable"),
            4,
        );

        let rendered = metrics.render();
        assert!(rendered.contains("outcome=\"success\"} 1"));
        assert!(rendered.contains("outcome=\"permanent_failure\"} 1"));
        assert!(rendered.contains("outcome=\"retryable_failure\"} 1"));
        assert!(rendered.contains("class=\"unsupported_format\"} 1"));
        assert!(rendered.contains("class=\"upload\"} 1"));

        // 12ms (success) + 4ms (upload failure, which had already decoded) = 16ms.
        // The 3ms unsupported-format rejection is excluded: it never decoded
        // anything, and counting a fast rejection as conversion latency reports a
        // service faster than it is.
        assert!(
            rendered.contains("thumbnail_worker_conversion_duration_seconds_count 2"),
            "only the two deliveries that reached conversion are observed:\n{rendered}"
        );
        assert!(
            rendered.contains("thumbnail_worker_conversion_duration_seconds_sum 0.016"),
            "the excluded rejection must not contribute to the sum:\n{rendered}"
        );
        // Both observations are under 50ms, so every bucket holds both.
        assert!(
            rendered.contains("thumbnail_worker_conversion_duration_seconds_bucket{le=\"0.05\"} 2"),
            "cumulative buckets must include an observation in every wider bound:\n{rendered}"
        );
        assert!(
            rendered.contains("thumbnail_worker_conversion_duration_seconds_bucket{le=\"+Inf\"} 2"),
            "+Inf must hold every observation:\n{rendered}"
        );
    }

    /// A class nobody has hit must still be reported, or its absence looks like
    /// a broken exporter rather than a healthy worker.
    #[test]
    fn every_failure_class_is_present_even_at_zero() {
        let rendered = WorkerMetrics::new_for_tests().render();

        for class in FailureClass::ALL {
            assert!(
                rendered.contains(&format!("class=\"{}\"}} 0", class.as_label())),
                "{} should be reported at zero",
                class.as_label()
            );
        }
    }

    /// Operators need to see retries accumulating before the queue reaches the
    /// platform's maximum length and starts dead-lettering the overflow.
    #[test]
    fn unacknowledged_deliveries_are_observable() {
        let metrics = WorkerMetrics::new_for_tests();

        metrics.delivery_received();
        metrics.delivery_received();
        assert_eq!(metrics.unacknowledged(), 2);

        metrics.delivery_acknowledged();
        assert_eq!(metrics.unacknowledged(), 1);
        assert!(metrics
            .render()
            .contains("thumbnail_worker_unacknowledged_deliveries 1"));
    }

    /// Valid exposition, not just plausible text: every family declares its help
    /// and type exactly once, before its samples. Declaring them per labelled
    /// sample is the easy mistake and stricter scrapers reject it.
    #[test]
    fn every_family_is_declared_exactly_once() {
        let rendered = WorkerMetrics::new_for_tests().render();

        let families: Vec<&str> = rendered
            .lines()
            .filter_map(|line| line.strip_prefix("# TYPE "))
            .map(|line| line.split_whitespace().next().expect("a name follows TYPE"))
            .collect();

        assert_eq!(families.len(), 9, "one TYPE per family: {rendered}");

        for name in &families {
            assert_eq!(
                rendered
                    .lines()
                    .filter(|line| line.starts_with(&format!("# HELP {name} ")))
                    .count(),
                1,
                "{name} must declare HELP exactly once"
            );

            let first_sample = rendered
                .lines()
                .position(|line| !line.starts_with('#') && line.starts_with(name))
                .expect("every family has at least one sample");
            let type_line = rendered
                .lines()
                .position(|line| line.starts_with(&format!("# TYPE {name} ")))
                .expect("checked above");
            assert!(
                type_line < first_sample,
                "{name} must declare its type before its samples"
            );
        }
    }

    /// A reconnect redelivers everything unacknowledged, so carrying the old
    /// count forward would double-count the same deliveries.
    #[test]
    fn a_reconnect_resets_the_unacknowledged_count() {
        let metrics = WorkerMetrics::new_for_tests();
        metrics.delivery_received();
        metrics.delivery_received();

        metrics.reset_unacknowledged();

        assert_eq!(metrics.unacknowledged(), 0);
    }
}
