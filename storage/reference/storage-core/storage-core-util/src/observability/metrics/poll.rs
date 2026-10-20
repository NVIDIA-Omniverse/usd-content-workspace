// SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

// PollMetricFuture has a lifetime parameter which is used and needed and there is no way to elide it, but Clippy still
// complains about it. It is probably in the generated code, but there is not a way to fix it.
#![allow(clippy::needless_lifetimes)]

use std::{
    pin::Pin,
    task::{Context, Poll},
};

use super::*;
use pin_project::{pin_project, pinned_drop};

/// Metrics common to an async poll operation (e.g.: something you would `.await` on).
///
/// With a given name, this metric makes the following sub-metrics:
///
/// * `{name}_started`: [`UIntCounter`] -- number of times this operation was started. This is counted up after the
///   first `.await`, so instrumented futures that are never `.await`ed will not be counted.
/// * `{name}_complete`: [`UIntCounter`] -- the number of times this operation completed. The associated attribute
///   `"immediate"` is set to `true` if the operation completed immediately or `false` if we had to wait for it to
///   complete. The associated attribute `"abandoned"` is set to `true` if we did not ever see a result (e.g.: message
///   was sent to a remote server, but we stopped caring about the result before we received a response). The attributes
///   `"immediate"` and `"abandoned"` will never be set to `true` at the same time, as such an operation did not really
///   start in the first place.
/// * `{name}_time`: [`Histogram`] -- the time spent waiting for this operation to complete (only recorded if we did not
///   abandon the operation). Operations that completed immediately will be recorded as having a time of `0`.
///
/// ```ignore
/// use coreapi_framework_prototype::metrics::{metric, WithPollMetric};
///
/// async fn foo() { }
///
/// # async {
/// foo()
///     .with_poll_metric(metric!(PollMetric, name = "foo"))
///     .await;
/// # };
/// ```
pub struct PollMetric {
    started: UIntCounter,
    complete: UIntCounter,
    time: Histogram,
}

impl PollMetric {
    const KEY_IMMEDIATE: Key = Key::from_static_str("immediate");
    const KEY_ABANDONED: Key = Key::from_static_str("abandoned");

    pub fn builder() -> PollMetricBuilder {
        PollMetricBuilder::default()
    }
}

#[derive(Default)]
pub struct PollMetricBuilder {
    started: InstrumentConfigBuilder<UIntCounter, InstrumentConfigCommon>,
    complete: InstrumentConfigBuilder<UIntCounter, InstrumentConfigCommon>,
    time: InstrumentConfigBuilder<Histogram, HistogramConfig<f64>>,
}

impl PollMetricBuilder {
    /// Set the name of the metric we are tracking.
    pub fn name(mut self, name: impl AsRef<str>) -> Self {
        let name = name.as_ref();

        self.started = self
            .started
            .name(format!("{name}_started"))
            .description("the number of times this operation was started")
            .unit("count");
        self.complete = self
            .complete
            .name(format!("{name}_complete"))
            .description(r#"the number of times this operation completed. The associated attribute `"immediate"` is set to `true` if the operation completed immediately or `false` if we had to wait for it to complete. The associated attribute `"abandoned"` is set to `true` if we did not ever see a result"#)
            .unit("count");
        self.time = self
            .time
            .name(format!("{name}_time"))
            .description("the time spent waiting for this operation to complete (only recorded if we did not abandon the operation). Operations that completed immediately will be recorded as having a time of `0`")
            .unit("s")
            .buckets(HistogramCommonBucket::Time);
        self
    }

    pub fn attrs<K: Into<Key>>(mut self, attrs: impl IntoIterator<Item = K>) -> Self {
        let mut attrs: Vec<_> = attrs.into_iter().map(Into::into).collect();

        self.started = self.started.attrs(attrs.clone());
        self.time = self.time.attrs(attrs.clone());

        attrs.push(PollMetric::KEY_IMMEDIATE);
        attrs.push(PollMetric::KEY_ABANDONED);
        self.complete = self.complete.attrs(attrs);

        self
    }

    pub fn try_build(self, meter: &Meter) -> Result<PollMetric, InstrumentBuildError> {
        Ok(PollMetric {
            started: self.started.try_build(meter)?,
            complete: self.complete.try_build(meter)?,
            time: self.time.try_build(meter)?,
        })
    }

    pub fn build(self, meter: &Meter) -> PollMetric {
        self.try_build(meter).unwrap()
    }
}

impl InstrumentBuild for PollMetric {
    type Config = PollMetricBuilder;

    fn try_build(meter: &Meter, config: Self::Config) -> Result<Self, InstrumentBuildError> {
        config.try_build(meter)
    }
}

impl MetricBinder for PollMetric {
    type Bound = BoundPollMetric;

    fn try_bind<A: Into<Attribute>>(
        &self,
        attributes: impl IntoIterator<Item = A>,
    ) -> Result<Self::Bound, MetricBindError> {
        let attributes: Vec<_> = attributes.into_iter().map(Into::into).collect();

        // NOTE: This is pretty ugly.
        Ok(BoundPollMetric {
            started: self.started.try_bind(attributes.clone())?,
            complete_immediate: self.complete.try_bind(
                attributes.iter().cloned().chain(
                    [
                        Attribute::new(PollMetric::KEY_IMMEDIATE, Value::from(true)),
                        Attribute::new(PollMetric::KEY_ABANDONED, Value::from(false)),
                    ]
                    .iter()
                    .cloned(),
                ),
            )?,
            complete_wait: self.complete.try_bind(
                attributes.iter().cloned().chain(
                    [
                        Attribute::new(PollMetric::KEY_IMMEDIATE, Value::from(false)),
                        Attribute::new(PollMetric::KEY_ABANDONED, Value::from(false)),
                    ]
                    .iter()
                    .cloned(),
                ),
            )?,
            abandoned: self.complete.try_bind(
                attributes.iter().cloned().chain(
                    [
                        Attribute::new(PollMetric::KEY_IMMEDIATE, Value::from(false)),
                        Attribute::new(PollMetric::KEY_ABANDONED, Value::from(true)),
                    ]
                    .iter()
                    .cloned(),
                ),
            )?,
            time: self.time.try_bind(attributes)?,
        })
    }
}

pub struct BoundPollMetric {
    started: BoundUIntCounter,
    complete_immediate: BoundUIntCounter,
    complete_wait: BoundUIntCounter,
    abandoned: BoundUIntCounter,
    time: BoundHistogram,
}

/// A wrapping `Future` that tracks a [`PollMetric`] for it.
#[must_use = "futures do nothing unless polled"]
#[pin_project(PinnedDrop)]
pub struct PollMetricFuture<'a, F> {
    #[pin]
    inner: F,
    state: PollState,
    metric: &'a BoundPollMetric,
}

#[derive(Debug)]
enum PollState {
    NotStarted,
    Waiting(Instant),
    Complete,
}

#[pinned_drop]
impl<F> PinnedDrop for PollMetricFuture<'_, F> {
    fn drop(self: Pin<&mut Self>) {
        // if we're still waiting, we've been abandoned
        if let PollState::Waiting(_) = &self.state {
            self.metric.abandoned.add(1);
        }
    }
}

impl<F: core::future::Future> core::future::Future for PollMetricFuture<'_, F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let this = self.project();

        if let PollState::NotStarted = &this.state {
            this.metric.started.add(1);
        }

        let result = this.inner.poll(cx);

        if result.is_ready() {
            let (metric_complete, time) = match &this.state {
                PollState::Waiting(start) => (&this.metric.complete_wait, start.elapsed().as_secs_f64()),
                PollState::NotStarted => (&this.metric.complete_immediate, 0.0),
                PollState::Complete => {
                    // calling an already-complete future is not allowed...this should never be hit, but if we do, just
                    // assume whatever metric we hit was the right one
                    return result;
                },
            };
            metric_complete.add(1);
            this.metric.time.record(time);
            *this.state = PollState::Complete;
        } else if let PollState::NotStarted = &this.state {
            *this.state = PollState::Waiting(Instant::now());
        }
        result
    }
}

pub trait WithPollMetric: Sized {
    fn with_poll_metric(self, metric: &BoundPollMetric) -> PollMetricFuture<'_, Self>;
}

impl<F> WithPollMetric for F {
    fn with_poll_metric(self, metric: &BoundPollMetric) -> PollMetricFuture<'_, Self> {
        PollMetricFuture {
            inner: self,
            state: PollState::NotStarted,
            metric,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn regular_usage() {
        async fn foo() {}

        foo().with_poll_metric(metric!(PollMetric, name = "test.foo")).await;

        async fn bar() {
            tokio::task::yield_now().await;
        }

        bar().with_poll_metric(metric!(PollMetric, name = "test.bar")).await;

        let task =
            tokio::task::spawn(std::future::pending::<()>().with_poll_metric(metric!(PollMetric, name = "test.baz")));
        tokio::task::yield_now().await;
        task.abort();
    }
}
