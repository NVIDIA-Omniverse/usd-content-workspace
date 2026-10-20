// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::time::{Duration, SystemTime, SystemTimeError, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// expiration of any generated presigned url
    /// max Azure presignged URL expiration is 7 days
    /// <https://learn.microsoft.com/en-us/azure/storage/blobs/storage-blob-user-delegation-sas-create-cli>
    #[serde(default = "default_presigned_expiration", with = "humantime_serde")]
    pub presigned_expiration: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            presigned_expiration: default_presigned_expiration(),
        }
    }
}

fn default_presigned_expiration() -> Duration {
    Duration::from_secs(60 * 60 * 24)
}

/// Helper class that normalizes expiration times to "epochs" based on the configurated expiration time.
/// This is useful for making presigned URLs consistent over a time period.    
#[derive(Debug, Clone)]
pub struct ExpirationNormalizer {
    pub expiration_duration: u64,
}

impl ExpirationNormalizer {
    pub fn new(config: &Config) -> Self {
        Self {
            expiration_duration: config.presigned_expiration.as_secs(),
        }
    }

    /// Gets the expiration time to use for the current time based on the configured expiration time.
    ///
    /// This works by determining the current "expiration epoch", and adjusts the "start time" to be the
    /// start of the current epoch, and the "expires_in" will be set to end at the end of the next epoch.
    /// This ensures that the expiration time is valid for *at least* the configured expiration time, but
    /// may be valid for up to two times the configured expiration time.
    ///
    ///
    /// This diagram shows how expiration epochs work:
    ///
    /// ```text
    ///        +------------------+                   
    ///        |                  |                   
    ///  +-----+-----++-----------++-----------+      
    ///  |  EPOCH 1  ||  EPOCH 2  ||  EPOCH 3  |  ....
    ///  +-----------++-----------++-----------+      
    ///                     |                  |      
    ///                     +------------------+      
    /// ```
    ///
    /// The epochs are time periods based on the configured expiration duration.      
    pub fn get_expiration(&self) -> Result<NormalizedExpiration, SystemTimeError> {
        Ok(self.get_expiration_with_current_time(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs()))
    }

    // Expose this function to make testing easier, but actual code can just use get_expiration
    pub fn get_expiration_with_current_time(&self, current_time: u64) -> NormalizedExpiration {
        let current_epoch = current_time / self.expiration_duration;
        let start_time = current_epoch * self.expiration_duration;
        let expires_in = self.expiration_duration * 2;
        NormalizedExpiration {
            start_time: UNIX_EPOCH + Duration::from_secs(start_time),
            end_time: start_time + expires_in,
            expires_in,
        }
    }
}

pub struct NormalizedExpiration {
    pub start_time: SystemTime,
    pub end_time: u64,
    pub expires_in: u64,
}

impl NormalizedExpiration {
    pub fn end_time(&self) -> i64 {
        self.end_time as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epochs() {
        let base_time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();

        // 5 second expiration
        let expiration = ExpirationNormalizer::new(&Config {
            presigned_expiration: Duration::from_secs(5),
        });
        let epoch = expiration.expiration_duration;

        // ~11 days worth of seconds to test
        for i in base_time..base_time + 1000000 {
            let normalized = expiration.get_expiration_with_current_time(i);
            assert_eq!(normalized.start_time, UNIX_EPOCH + Duration::from_secs(i - (i % epoch)));
            assert_eq!(normalized.expires_in, epoch * 2);
        }

        // 15min expiration
        let expiration = ExpirationNormalizer::new(&Config {
            presigned_expiration: Duration::from_secs(15 * 60),
        });
        let epoch = expiration.expiration_duration;

        for i in base_time..base_time + 1000000 {
            let normalized = expiration.get_expiration_with_current_time(i);
            assert_eq!(normalized.start_time, UNIX_EPOCH + Duration::from_secs(i - (i % epoch)));
            assert_eq!(normalized.expires_in, epoch * 2);
        }

        // 1 day expiration
        let expiration = ExpirationNormalizer::new(&Config {
            presigned_expiration: Duration::from_secs(24 * 60 * 60),
        });
        let epoch = expiration.expiration_duration;

        for i in base_time..base_time + 1000000 {
            let normalized = expiration.get_expiration_with_current_time(i);
            assert_eq!(normalized.start_time, UNIX_EPOCH + Duration::from_secs(i - (i % epoch)));
            assert_eq!(normalized.expires_in, epoch * 2);
        }
    }
}
