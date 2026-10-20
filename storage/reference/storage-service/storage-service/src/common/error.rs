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

use std::{fmt::Display, ops::Deref, sync::Arc};

use storage_core::traits::{StorageError, StorageErrorExt};

pub struct PrettyPrintError<'a, T: ?Sized> {
    error: &'a T,
}

impl<'a, T: ?Sized> PrettyPrintError<'a, T> {
    pub fn new(error: &'a T) -> Self {
        Self { error }
    }
}

impl<'a, T: ?Sized + std::error::Error + 'static> Display for PrettyPrintError<'a, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Error: {}", self.error)?;

        if let Some(mut source) = self.error.source() {
            writeln!(f, "Caused by:")?;

            let mut index = 0usize;
            loop {
                writeln!(f, "  {}: {}", index, source)?;

                let Some(next_source) = source.source() else {
                    break;
                };

                source = next_source;
                index += 1;
            }
        }

        Ok(())
    }
}

pub trait PrettyPrintErrorExt {
    fn pretty(&self) -> PrettyPrintError<'_, Self>;
}

impl<T: ?Sized + std::error::Error + 'static> PrettyPrintErrorExt for T {
    fn pretty(&self) -> PrettyPrintError<'_, Self> {
        PrettyPrintError::new(self)
    }
}

pub struct ArcStorageError(pub Arc<StorageError>);

impl Deref for ArcStorageError {
    type Target = dyn StorageErrorExt;

    fn deref(&self) -> &Self::Target {
        &**self.0
    }
}

impl std::fmt::Display for ArcStorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

impl std::fmt::Debug for ArcStorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.0, f)
    }
}

impl std::error::Error for ArcStorageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.source()
    }
}

impl From<Arc<StorageError>> for ArcStorageError {
    fn from(value: Arc<StorageError>) -> Self {
        Self(value)
    }
}

#[cfg(test)]
mod tests {
    use std::{error::Error, sync::Arc};

    use storage_core::traits::StorageErrorExt;

    #[derive(Debug)]
    struct TestError;

    impl std::fmt::Display for TestError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("test error")
        }
    }

    impl Error for TestError {}

    impl StorageErrorExt for TestError {
        fn get_size(&self) -> usize {
            unimplemented!()
        }

        fn http_status(&self) -> axum::http::StatusCode {
            unimplemented!()
        }

        fn tonic_status(&self) -> tonic::Status {
            unimplemented!()
        }
    }

    #[test]
    fn test_small_object_cache_error_returns_source() {
        use storage_core::small_object_cache::error::Error;

        let error = Error::CachedError {
            source: Arc::new(Box::new(TestError)),
        };

        let source = error.source();
        assert!(source.is_some());
        assert!(source.unwrap().downcast_ref::<TestError>().is_some());
    }

    #[test]
    fn test_stat_cache_error_returns_source() {
        use storage_core::stat_cache::error::Error;

        let error = Error::CachedError {
            source: Arc::new(Box::new(TestError)),
        };

        let source = error.source();
        assert!(source.is_some());
        assert!(source.unwrap().downcast_ref::<TestError>().is_some());
    }
}
