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

use std::{future::Future, time::Duration};

use fastrand::Rng;
use tokio::time::sleep;

/// Jitter value multipliers.
static JITTER_MULTIPLIERS: [f64; 11] = [0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0];

/// Retry an asynchronous operation with backoff.
pub async fn retry_with_backoff<'a, T, Fut, Out, Err>(backoff: &Backoff, mut func: T) -> Result<Out, Err>
where
    T: FnMut() -> Fut + 'a,
    Fut: Future<Output = Result<Out, Err>> + 'a,
{
    let backoff_iter = backoff.iter();
    match func().await {
        Ok(value) => Ok(value),
        Err(mut error) => {
            for delay in backoff_iter {
                sleep(delay).await;
                match func().await {
                    Ok(value) => return Ok(value),
                    Err(next_error) => error = next_error,
                }
            }

            Err(error)
        },
    }
}

/// An exponential backoff strategy.
#[derive(Debug, Clone, Copy)]
pub struct Backoff {
    pub min: f64,
    pub max: f64,
    pub exp: f64,
    pub jitter: f64,
    pub retries: usize,
}

impl Backoff {
    /// Create a backoff strategy using the default configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a backoff interval iterator.
    pub fn iter(&self) -> BackoffIterator {
        BackoffIterator {
            next: self.min,
            max: self.max,
            exp: self.exp,
            jitter: self.jitter,
            retries_left: self.retries,
            rng: Rng::new(),
        }
    }
}

impl Default for Backoff {
    fn default() -> Self {
        Self {
            min: 0.5,
            max: 10.0,
            exp: 2.0,
            jitter: 0.3,
            retries: 5,
        }
    }
}

pub struct BackoffIterator {
    next: f64,
    max: f64,
    exp: f64,
    jitter: f64,
    retries_left: usize,
    rng: Rng,
}

impl Iterator for BackoffIterator {
    type Item = Duration;

    fn next(&mut self) -> Option<Self::Item> {
        if self.retries_left == 0 {
            None
        } else {
            let current = self.next + self.jitter * JITTER_MULTIPLIERS[self.rng.usize(0..JITTER_MULTIPLIERS.len())];

            self.retries_left -= 1;
            self.next = (self.next * self.exp).min(self.max);

            Some(Duration::from_secs_f64(current))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::Backoff;

    #[test]
    fn test_backoff_iterator() {
        let values: Vec<_> = Backoff {
            jitter: 0.0,
            ..Default::default()
        }
        .iter()
        .collect();

        assert_eq!(
            values,
            vec![
                Duration::from_secs_f64(0.5),
                Duration::from_secs_f64(1.0),
                Duration::from_secs_f64(2.0),
                Duration::from_secs_f64(4.0),
                Duration::from_secs_f64(8.0),
            ],
        );
    }

    #[test]
    fn test_backoff_jitter_applies_to_each_value() {
        let values: Vec<_> = Backoff::default().iter().collect();
        let min_values = [
            Duration::from_secs_f64(0.5),
            Duration::from_secs_f64(1.0),
            Duration::from_secs_f64(2.0),
            Duration::from_secs_f64(4.0),
            Duration::from_secs_f64(8.0),
        ];
        let max_values = [
            Duration::from_secs_f64(0.8),
            Duration::from_secs_f64(1.3),
            Duration::from_secs_f64(2.3),
            Duration::from_secs_f64(4.3),
            Duration::from_secs_f64(8.3),
        ];

        assert!(values.iter().zip(min_values.iter()).all(|(a, b)| a >= b));
        assert!(values.iter().zip(max_values.iter()).all(|(a, b)| a <= b));
    }
}
