// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::fmt::Debug;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use crate::observability::metrics::{self, BoundTimerMetric, BoundUIntGauge, MetricBinder, TimerMetricGuard};
use crate::{error_limited, warn_limited};

// Wrapper around a semaphore that emits metrics for acquisition time of permits and number of available permits
pub struct Semaphore {
    inner: Arc<tokio::sync::Semaphore>,
    total_permits: u64,
    /// Available permits at or below which we log WARN (80% utilized)
    warn_threshold: u64,
    /// Available permits at or below which we log ERROR (95% utilized)
    error_threshold: u64,
    name: String,
    available_gauge: BoundUIntGauge,
    waiters_gauge: BoundUIntGauge,
    acquire_time: BoundTimerMetric,
    max_waiters_gauge: BoundUIntGauge,
    min_permits_gauge: BoundUIntGauge,
    max_waiters: AtomicU64,
    current_waiters: AtomicU64,
    min_permits: AtomicU64,
}

impl Semaphore {
    /// Default warn threshold: log WARN when 80% of permits are in use.
    pub const DEFAULT_WARN_UTILIZATION_PCT: u8 = 80;
    /// Default error threshold: log ERROR when 95% of permits are in use.
    pub const DEFAULT_ERROR_UTILIZATION_PCT: u8 = 95;

    pub fn new(permits: usize, name: impl AsRef<str>) -> Self {
        Self::with_thresholds(
            permits,
            name,
            Self::DEFAULT_WARN_UTILIZATION_PCT,
            Self::DEFAULT_ERROR_UTILIZATION_PCT,
        )
    }

    pub fn with_thresholds(
        permits: usize,
        name: impl AsRef<str>,
        warn_utilization_pct: u8,
        error_utilization_pct: u8,
    ) -> Self {
        let pod_name = std::env::var("STORAGE_POD_NAME").unwrap_or("pod".to_string());
        let pkg_version = env!("CARGO_PKG_VERSION").to_string();

        metrics::metric_base!(
            static SEM_AVAIL_PERMITS_BASE: UIntGauge,
            name = "semaphore.permits",
            description = "Current number of available semaphore permits",
            { name, pod_name, pkg_version }
        );
        let available_gauge = SEM_AVAIL_PERMITS_BASE.bind([
            ("name", name.as_ref().to_string()),
            ("pod_name", pod_name.clone()),
            ("pkg_version", pkg_version.clone()),
        ]);
        available_gauge.record(permits as u64);

        metrics::metric_base!(
            static SEM_MIN_PERMITS_BASE: UIntGauge,
            name = "semaphore.permits.min",
            description = "Lowest number of available permits observed since startup",
            { name, pod_name, pkg_version }
        );
        let min_permits_gauge = SEM_MIN_PERMITS_BASE.bind([
            ("name", name.as_ref().to_string()),
            ("pod_name", pod_name.clone()),
            ("pkg_version", pkg_version.clone()),
        ]);
        min_permits_gauge.record(permits as u64);

        metrics::metric_base!(
            static SEM_WAITERS_BASE: UIntGauge,
            name = "semaphore.waiters",
            description = "Current number of tasks waiting to acquire permits",
            { name, pod_name, pkg_version }
        );

        metrics::metric_base!(
            static SEM_MAX_WAITERS_BASE: UIntGauge,
            name = "semaphore.waiters.max",
            description = "Peak number of waiting tasks observed since startup",
            { name, pod_name, pkg_version }
        );

        metrics::metric_base!(
            static SEM_ACQUIRE_TIME_BASE: TimerMetric,
            name = "semaphore.acquire.duration",
            { name, pod_name, pkg_version }
        );

        Self {
            inner: Arc::new(tokio::sync::Semaphore::new(permits)),
            total_permits: permits as u64,
            warn_threshold: Self::remaining_threshold(permits, warn_utilization_pct),
            error_threshold: Self::remaining_threshold(permits, error_utilization_pct),
            name: name.as_ref().to_string(),
            available_gauge,
            waiters_gauge: SEM_WAITERS_BASE.bind([
                ("name", name.as_ref().to_string()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ]),
            acquire_time: SEM_ACQUIRE_TIME_BASE.bind([
                ("name", name.as_ref().to_string()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ]),
            max_waiters_gauge: SEM_MAX_WAITERS_BASE.bind([
                ("name", name.as_ref().to_string()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ]),
            min_permits_gauge,
            max_waiters: AtomicU64::new(0),
            current_waiters: AtomicU64::new(0),
            min_permits: AtomicU64::new(permits as u64),
        }
    }

    /// Convert a utilization percentage into the number of remaining permits at that threshold.
    /// Returns 0 when disabled (permits==0 or pct==0).
    fn remaining_threshold(permits: usize, utilization_pct: u8) -> u64 {
        let pct = utilization_pct.min(100) as u64;
        if permits > 0 && pct > 0 {
            permits as u64 * (100 - pct) / 100
        } else {
            0
        }
    }

    pub async fn acquire(&self) -> Result<tokio::sync::SemaphorePermit<'_>, tokio::sync::AcquireError> {
        let _timer = self.preacquire();
        let output = self.inner.acquire().await;
        self.postacquire();
        output
    }

    pub async fn acquire_owned(&self) -> Result<tokio::sync::OwnedSemaphorePermit, tokio::sync::AcquireError> {
        let _timer = self.preacquire();
        let output = self.inner.clone().acquire_owned().await;
        self.postacquire();
        output
    }

    fn preacquire(&self) -> TimerMetricGuard {
        let timer = self.acquire_time.start();
        let current_waiters = self.current_waiters.fetch_add(1, Ordering::Relaxed);
        self.waiters_gauge.record(current_waiters);
        timer
    }

    fn postacquire(&self) {
        // Calculate the current number of people waiting.
        // There are race conditions here, but preventing them would add even more synchronization, and the
        // results should be enough to still be useful.
        let current_waiters = self.current_waiters.fetch_sub(1, Ordering::Relaxed);
        self.waiters_gauge.record(current_waiters);
        if current_waiters > self.max_waiters.load(Ordering::Relaxed) {
            self.max_waiters.store(current_waiters, Ordering::Relaxed);
            self.max_waiters_gauge.record(current_waiters);
        }

        // Calculate minimum available permits
        let permits_available = self.inner.available_permits() as u64;
        self.available_gauge.record(permits_available);
        if permits_available < self.min_permits.load(Ordering::Relaxed) {
            self.min_permits.store(permits_available, Ordering::Relaxed);
            self.min_permits_gauge.record(permits_available);
        }

        // Resource threshold logging (SRD 3.9) — thresholds precomputed at construction
        if self.total_permits > 0 {
            if permits_available <= self.error_threshold {
                error_limited!(
                    name = %self.name,
                    available = permits_available,
                    total = self.total_permits,
                    waiters = current_waiters,
                    utilization_pct = (self.total_permits - permits_available) * 100 / self.total_permits,
                    "Throttle nearly exhausted, requests will queue"
                );
            } else if permits_available <= self.warn_threshold {
                warn_limited!(
                    name = %self.name,
                    available = permits_available,
                    total = self.total_permits,
                    waiters = current_waiters,
                    utilization_pct = (self.total_permits - permits_available) * 100 / self.total_permits,
                    "Throttle permits running low"
                );
            }
        }
    }
}

impl Debug for Semaphore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Debug::fmt(&self.inner, f)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::Semaphore;

    #[test_log::test()]
    fn max_waiters() {
        let permit_count = 100;
        let sema = Semaphore::new(permit_count, "max_waiters_test");

        for acquire_count in [permit_count / 2, permit_count * 2] {
            for _ in 0..acquire_count {
                let _ = sema.preacquire();
            }

            assert_eq!(sema.current_waiters.load(Ordering::Relaxed) as usize, acquire_count);

            for _ in 0..acquire_count {
                sema.postacquire();
            }

            assert_eq!(sema.current_waiters.load(Ordering::Relaxed) as usize, 0);
            assert_eq!(sema.max_waiters.load(Ordering::Relaxed) as usize, acquire_count);
        }
    }

    #[test_log::test(tokio::test)]
    async fn available_permits() {
        let permit_count = 100;
        let sema = Semaphore::new(permit_count, "available_permits_test");

        for acquire_count in [permit_count / 2, permit_count] {
            let mut permits = Vec::new();
            for _ in 0..acquire_count {
                permits.push(sema.acquire().await);
            }

            assert_eq!(
                sema.min_permits.load(Ordering::Relaxed) as usize,
                permit_count - acquire_count
            );

            // Release all permits and check metrics again
            permits.clear();

            let _ = sema.acquire().await;
            assert_eq!(
                sema.min_permits.load(Ordering::Relaxed) as usize,
                permit_count - acquire_count
            );
        }
    }
}
