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

pub struct Hex<'a> {
    bytes: &'a [u8],
}

impl<'a> Hex<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }
}

impl std::fmt::Debug for Hex<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "b\"")?;
        for b in self.bytes {
            write!(f, "{:02x}", b)?;
        }

        write!(f, "\"")?;

        Ok(())
    }
}

pub trait HexExt {
    fn hex<'a>(&'a self) -> Hex<'a>;
}

impl<T: AsRef<[u8]>> HexExt for T {
    fn hex<'a>(&'a self) -> Hex<'a> {
        Hex::new(self.as_ref())
    }
}
