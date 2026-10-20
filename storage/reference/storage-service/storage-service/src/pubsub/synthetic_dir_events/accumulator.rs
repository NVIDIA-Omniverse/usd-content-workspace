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

use std::{collections::HashSet, sync::Arc};

use parking_lot::Mutex;

/// Thread-safe accumulator that holds two sets of directory paths — one for possible
/// `dir_created` events, one for possible `dir_deleted` — between flush cycles.
///
/// Insertions dedup naturally via `HashSet`. `drain` swaps each set independently with
/// an empty one and returns the previous contents — the two sets are not drained as a
/// single atomic snapshot. This is fine in practice because only the flush task drains;
/// the next interval picks up anything that landed between the two lock acquisitions.
#[derive(Debug, Default)]
pub struct Accumulator {
    created: Mutex<HashSet<String>>,
    deleted: Mutex<HashSet<String>>,
}

impl Accumulator {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn insert_created<I>(&self, paths: I)
    where
        I: IntoIterator<Item = String>,
    {
        self.created.lock().extend(paths);
    }

    pub fn insert_deleted<I>(&self, paths: I)
    where
        I: IntoIterator<Item = String>,
    {
        self.deleted.lock().extend(paths);
    }

    /// Take all accumulated paths, leaving the internal sets empty.
    pub fn drain(&self) -> DrainedPaths {
        let created = std::mem::take(&mut *self.created.lock());
        let deleted = std::mem::take(&mut *self.deleted.lock());
        DrainedPaths { created, deleted }
    }

    #[cfg(test)]
    pub fn created_len(&self) -> usize {
        self.created.lock().len()
    }

    #[cfg(test)]
    pub fn deleted_len(&self) -> usize {
        self.deleted.lock().len()
    }
}

pub struct DrainedPaths {
    pub created: HashSet<String>,
    pub deleted: HashSet<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedup_within_interval() {
        let acc = Accumulator::default();
        acc.insert_created(["a/".to_string(), "a/".to_string(), "b/".to_string()]);
        acc.insert_created(["a/".to_string()]);

        let drained = acc.drain();
        assert_eq!(drained.created.len(), 2);
        assert!(drained.created.contains("a/"));
        assert!(drained.created.contains("b/"));
    }

    #[test]
    fn drain_empties_sets() {
        let acc = Accumulator::default();
        acc.insert_created(["a/".to_string()]);
        acc.insert_deleted(["b/".to_string()]);

        let first = acc.drain();
        assert_eq!(first.created.len(), 1);
        assert_eq!(first.deleted.len(), 1);

        let second = acc.drain();
        assert!(second.created.is_empty());
        assert!(second.deleted.is_empty());
    }

    #[test]
    fn created_and_deleted_are_independent() {
        let acc = Accumulator::default();
        acc.insert_created(["a/".to_string()]);
        acc.insert_deleted(["a/".to_string()]);

        let drained = acc.drain();
        assert_eq!(drained.created.len(), 1);
        assert_eq!(drained.deleted.len(), 1);
    }
}
