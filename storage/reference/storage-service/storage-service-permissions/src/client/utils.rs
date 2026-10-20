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

use std::num::NonZero;

pub struct BatchIter<T> {
    inner: T,
    batch_size: NonZero<usize>,
}

impl<T: Iterator> Iterator for BatchIter<T> {
    type Item = Vec<T::Item>;

    fn next(&mut self) -> Option<Self::Item> {
        let batch_size = self.batch_size.get();
        let first = self.inner.next()?;

        let mut batch = match self.size_hint() {
            (_, Some(exact)) => Vec::with_capacity(batch_size.min(exact + 1)),
            (at_least, None) => Vec::with_capacity(batch_size.min(at_least + 1)),
        };
        batch.push(first);

        for _ in 1..batch_size {
            if let Some(item) = self.inner.next() {
                batch.push(item);
            } else {
                break;
            }
        }

        Some(batch)
    }
}

pub trait BatchExt: Sized {
    type Item;

    fn batches(self, batch_size: NonZero<usize>) -> BatchIter<Self>;
}

impl<T: Iterator> BatchExt for T {
    type Item = T::Item;

    fn batches(self, batch_size: NonZero<usize>) -> BatchIter<Self> {
        BatchIter {
            inner: self,
            batch_size,
        }
    }
}

pub trait TryCollectVecExt {
    type Item;
    type Error;

    fn try_collect_vec(self) -> Result<Vec<Self::Item>, Self::Error>;
}

impl<I, R, E> TryCollectVecExt for I
where
    I: Iterator<Item = Result<R, E>>,
{
    type Item = R;
    type Error = E;

    fn try_collect_vec(self) -> Result<Vec<Self::Item>, Self::Error> {
        let mut result = match self.size_hint() {
            (_, Some(exact)) => Vec::with_capacity(exact),
            (at_least, None) => Vec::with_capacity(at_least),
        };

        for item in self.into_iter() {
            result.push(item?);
        }

        Ok(result)
    }
}
