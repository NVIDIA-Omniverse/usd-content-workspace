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

use std::{future::Future, iter::FusedIterator, num::NonZero, time::Duration};

use fastrand::Rng;
use serde::{Deserialize, Serialize};

const DEFAULT_INITIAL_BACKOFF: Duration = Duration::from_millis(100);
const DEFAULT_MAX_BACKOFF: Duration = Duration::from_millis(10000);
const DEFAULT_MAX_ATTEMPTS: NonZero<usize> = NonZero::new(5).unwrap();

const MAX_JITTER_MS: u64 = 200;

pub enum MaybeTransient<T> {
    Transient(T),
    Permanent(T),
}

/// Request retry configuration.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct RetryConfig {
    #[serde(with = "humantime_serde")]
    pub initial_backoff: Duration,
    #[serde(with = "humantime_serde")]
    pub max_backoff: Duration,
    pub attempts: NonZero<usize>,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            initial_backoff: DEFAULT_INITIAL_BACKOFF,
            max_backoff: DEFAULT_MAX_BACKOFF,
            attempts: DEFAULT_MAX_ATTEMPTS,
        }
    }
}

/// An exponential retry strategy with a randomized jitter value.
#[derive(Debug, Clone, Copy)]
pub struct RetryStrategy {
    initial_backoff_ms: u64,
    max_backoff_ms: u64,
    attempts: NonZero<usize>,
}

impl RetryStrategy {
    pub fn from_config(config: RetryConfig) -> Self {
        Self {
            initial_backoff_ms: config.initial_backoff.as_millis() as u64,
            max_backoff_ms: config.max_backoff.as_millis() as u64,
            attempts: config.attempts,
        }
    }

    pub fn iter_retry_intervals(&self) -> RetryIntervalIter {
        RetryIntervalIter {
            next_backoff_ms: self.initial_backoff_ms,
            max_backoff_ms: self.max_backoff_ms,
            remaining_attempts: self.attempts.get(),
            rng: Rng::new(),
        }
    }
}

pub struct RetryIntervalIter {
    next_backoff_ms: u64,
    max_backoff_ms: u64,
    remaining_attempts: usize,
    rng: Rng,
}

impl Iterator for RetryIntervalIter {
    type Item = Duration;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining_attempts == 0 {
            None
        } else {
            self.remaining_attempts -= 1;

            let current_backoff_ms = self.next_backoff_ms;
            let next_backoff_ms = (current_backoff_ms * 2).min(self.max_backoff_ms);
            let jitter_ms = self.rng.u64(..=MAX_JITTER_MS);

            self.next_backoff_ms = next_backoff_ms;

            Some(Duration::from_millis(current_backoff_ms + jitter_ms))
        }
    }
}

impl FusedIterator for RetryIntervalIter {}

pub async fn invoke_with_retry<Fut, T, E>(
    intervals: impl IntoIterator<Item = Duration>,
    mut delegate: impl FnMut() -> Fut,
) -> Result<T, E>
where
    Fut: Future<Output = Result<T, MaybeTransient<E>>>,
{
    let mut intervals = intervals.into_iter();
    loop {
        match delegate().await {
            Ok(result) => return Ok(result),
            Err(MaybeTransient::Permanent(error)) => return Err(error),
            Err(MaybeTransient::Transient(error)) => {
                let Some(next_retry_interval) = intervals.next() else {
                    return Err(error);
                };

                tokio::time::sleep(next_retry_interval).await;
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::assert_matches;

    use super::*;

    #[test]
    fn test_retry_strategy_iter_retry_intervals() {
        let mut intervals = RetryStrategy::from_config(RetryConfig {
            initial_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(1),
            attempts: NonZero::new(5).unwrap(),
        })
        .iter_retry_intervals();

        assert_between(
            intervals.next().unwrap(),
            Duration::from_millis(100),
            Duration::from_millis(300),
        );
        assert_between(
            intervals.next().unwrap(),
            Duration::from_millis(200),
            Duration::from_millis(400),
        );
        assert_between(
            intervals.next().unwrap(),
            Duration::from_millis(400),
            Duration::from_millis(600),
        );
        assert_between(
            intervals.next().unwrap(),
            Duration::from_millis(800),
            Duration::from_millis(1000),
        );
        assert_between(
            intervals.next().unwrap(),
            Duration::from_millis(1000),
            Duration::from_millis(1200),
        );
        assert!(intervals.next().is_none());
    }

    #[tokio::test]
    async fn test_invoke_with_retry_first_success() {
        let result = invoke_with_retry([] as [Duration; 0], {
            let mut index = 0usize;
            move || {
                let result = index;
                index += 1;

                async move { Result::<_, MaybeTransient<()>>::Ok(result) }
            }
        })
        .await;

        assert_matches!(result, Ok(0));
    }

    #[tokio::test]
    async fn test_invoke_with_retry_retries_transient_errors() {
        let result = invoke_with_retry(
            [Duration::from_secs(0), Duration::from_secs(0), Duration::from_secs(0)],
            {
                let mut index = 0usize;
                move || {
                    let current = index;
                    index += 1;

                    async move {
                        match current {
                            3 => Ok(3),
                            _ => Err(MaybeTransient::Transient(current)),
                        }
                    }
                }
            },
        )
        .await;

        assert_matches!(result, Ok(3));
    }

    #[tokio::test]
    async fn test_invoke_with_retry_attempts_exhausted() {
        let result = invoke_with_retry([Duration::from_secs(0), Duration::from_secs(0)], {
            let mut index = 0usize;
            move || {
                let result = index;
                index += 1;

                async move { Result::<(), _>::Err(MaybeTransient::Transient(result)) }
            }
        })
        .await;

        assert_matches!(result, Err(2));
    }

    #[tokio::test]
    async fn test_invoke_with_retry_returns_permanent_errors() {
        let result = invoke_with_retry(
            [Duration::from_secs(0), Duration::from_secs(0), Duration::from_secs(0)],
            {
                let mut index = 0usize;
                move || {
                    let current = index;
                    index += 1;

                    async move {
                        match current {
                            1 => Result::<(), _>::Err(MaybeTransient::Permanent(1)),
                            _ => Result::<(), _>::Err(MaybeTransient::Transient(current)),
                        }
                    }
                }
            },
        )
        .await;

        assert_matches!(result, Err(1usize));
    }

    #[track_caller]
    fn assert_between(value: Duration, min: Duration, max: Duration) {
        assert!(value >= min, "{value:?} is expected to be >= {min:?}");
        assert!(value <= max, "{value:?} is expected to be <= {max:?}")
    }
}
