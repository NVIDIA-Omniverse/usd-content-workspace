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

use std::fmt::Display;

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
