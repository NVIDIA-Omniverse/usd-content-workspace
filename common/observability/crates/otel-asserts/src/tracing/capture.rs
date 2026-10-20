// SPDX-FileCopyrightText: Copyright (c) 2024-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
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

use std::fmt;
use std::sync::{Arc, Mutex};
use tracing_subscriber::fmt::MakeWriter;

/// Implements [`MakeWriter`] for use with tracing subscribers.
///
/// This captures the tracing output, which can be retrieved later in tests.
#[derive(Clone)]
#[repr(transparent)]
pub struct TraceCapture(Arc<Mutex<String>>);

impl TraceCapture {
    /// Create a new instance.
    #[inline]
    #[expect(clippy::new_without_default)]
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(String::new())))
    }

    fn writer(&self) -> Self {
        self.clone()
    }

    /// Returns the current tracing output, and replaces it with an empty
    /// string.
    #[inline]
    pub fn take(&self) -> String {
        let mut buf = self.0.lock().unwrap();
        std::mem::take(&mut *buf)
    }
}

impl fmt::Debug for TraceCapture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TraceCapture").finish_non_exhaustive()
    }
}

impl std::io::Write for TraceCapture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let line = std::str::from_utf8(buf).unwrap();
        // print to stdout, so tracing is visible in test output
        print!("{line}");
        self.0.lock().unwrap().push_str(line);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for TraceCapture {
    type Writer = Self;

    fn make_writer(&self) -> Self::Writer {
        self.writer()
    }
}
