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

use std::collections::{HashMap, HashSet};
use std::hash::Hash;

use arrayvec::ArrayVec;

pub trait TryCollectExt<T, E> {
    fn try_collect(self) -> Result<T, E>;
}

impl<I, T, E> TryCollectExt<Vec<T>, E> for I
where
    I: Iterator<Item = Result<T, E>>,
{
    fn try_collect(self) -> Result<Vec<T>, E> {
        let mut result = match self.size_hint() {
            (_, Some(size)) => Vec::with_capacity(size),
            _ => Vec::new(),
        };

        for item in self {
            result.push(item?);
        }

        Ok(result)
    }
}

impl<I, T, E> TryCollectExt<HashSet<T>, E> for I
where
    I: Iterator<Item = Result<T, E>>,
    T: Hash + Eq,
{
    fn try_collect(self) -> Result<HashSet<T>, E> {
        let mut result = match self.size_hint() {
            (_, Some(size)) => HashSet::with_capacity(size),
            _ => HashSet::new(),
        };

        for item in self {
            result.insert(item?);
        }

        Ok(result)
    }
}

impl<K, V, E, I> TryCollectExt<HashMap<K, V>, E> for I
where
    I: Iterator<Item = Result<(K, V), E>>,
    K: Hash + Eq,
{
    fn try_collect(self) -> Result<HashMap<K, V>, E> {
        let mut result = match self.size_hint() {
            (_, Some(size)) => HashMap::with_capacity(size),
            _ => HashMap::new(),
        };

        for item in self {
            let (key, value) = item?;
            result.insert(key, value);
        }

        Ok(result)
    }
}

pub trait ChunksExt<T> {
    fn chunks<const N: usize>(self) -> impl Iterator<Item = ArrayVec<T, N>>;
}

impl<T, I> ChunksExt<T> for I
where
    I: Iterator<Item = T>,
{
    fn chunks<const N: usize>(self) -> impl Iterator<Item = ArrayVec<T, N>> {
        ChunkIterator::<I, N> { inner: self }
    }
}

pub struct ChunkIterator<T, const N: usize> {
    inner: T,
}

impl<T, const N: usize> Iterator for ChunkIterator<T, N>
where
    T: Iterator,
{
    type Item = ArrayVec<T::Item, N>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut result = ArrayVec::new();
        for idx in 0..N {
            if let Some(value) = self.inner.next() {
                result.push(value);
            } else {
                if idx == 0 {
                    return None;
                }

                break;
            }
        }

        Some(result)
    }
}

pub trait RetainEachExt {
    fn retain_each(&mut self, retain: &mut impl Iterator<Item = bool>);
}

impl<T> RetainEachExt for Vec<T> {
    fn retain_each(&mut self, retain: &mut impl Iterator<Item = bool>) {
        self.retain(|_| retain.next() == Some(true));
    }
}
