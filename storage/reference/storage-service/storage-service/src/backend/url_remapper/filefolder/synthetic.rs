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

use std::collections::HashSet;

use tracing::warn;

const SYNTHETIC_RESUME_PREFIX: &str = "urlremapper:syn:";

pub fn synthetic_resume_token(skip: usize) -> String {
    format!("{SYNTHETIC_RESUME_PREFIX}{skip}")
}

pub fn parse_synthetic_resume_token(token: &str) -> Option<usize> {
    token.strip_prefix(SYNTHETIC_RESUME_PREFIX).and_then(|n| n.parse().ok())
}

pub trait SyntheticPaginable: Sized {
    fn current_len(&self) -> usize;

    fn extend_synthetic_subfolders(&mut self, synthetic: impl IntoIterator<Item = String>);

    fn set_continue_token(&mut self, token: String);

    fn drop_real_subfolders_with_uri(&mut self, drop_if_in: &HashSet<&str>) -> Vec<String>;

    fn new_synthetic_only_page(synthetic: impl IntoIterator<Item = String>) -> Self;
}

macro_rules! __impl_synthetic_paginable {
    ($wrapper:ident, $inner:ident, $folder_addr:ident, $non_folder_field:ident) => {
        impl $crate::backend::url_remapper::filefolder::synthetic::SyntheticPaginable for $wrapper {
            fn current_len(&self) -> usize {
                self.response.subfolder_addresses.len() + self.response.$non_folder_field.len()
            }

            fn extend_synthetic_subfolders(&mut self, synthetic: impl IntoIterator<Item = String>) {
                self.response
                    .subfolder_addresses
                    .extend(synthetic.into_iter().map(|uri| $folder_addr { uri }));
            }

            fn set_continue_token(&mut self, token: String) {
                self.continue_token = Some(token);
            }

            fn drop_real_subfolders_with_uri(&mut self, drop_if_in: &::std::collections::HashSet<&str>) -> Vec<String> {
                let mut dropped = Vec::new();
                self.response.subfolder_addresses.retain_mut(|addr| {
                    if drop_if_in.contains(addr.uri.as_str()) {
                        dropped.push(::std::mem::take(&mut addr.uri));
                        false
                    } else {
                        true
                    }
                });
                dropped
            }

            fn new_synthetic_only_page(synthetic: impl IntoIterator<Item = String>) -> Self {
                Self {
                    response: $inner {
                        subfolder_addresses: synthetic.into_iter().map(|uri| $folder_addr { uri }).collect(),
                        $non_folder_field: ::std::default::Default::default(),
                    },
                    continue_token: None,
                }
            }
        }
    };
}

pub(crate) use __impl_synthetic_paginable;

pub fn paginate<R>(
    parent_url: &str,
    mut last_inner_page: R,
    mut synthetic: Vec<String>,
    max_page_size: Option<usize>,
) -> Vec<R>
where
    R: SyntheticPaginable,
{
    if synthetic.is_empty() {
        return vec![last_inner_page];
    }
    dedupe_real_against_synthetic(parent_url, &mut last_inner_page, &synthetic);

    let cap = max_page_size.map(|n| n.max(1)).unwrap_or(usize::MAX);
    let current_len = last_inner_page.current_len();

    if current_len.saturating_add(synthetic.len()) <= cap {
        last_inner_page.extend_synthetic_subfolders(synthetic);
        return vec![last_inner_page];
    }

    // Move the head that fits into the last inner page in place; `synthetic`
    // then holds only the overflow, so no split allocation is needed.
    let remaining_in_last = cap.saturating_sub(current_len);
    last_inner_page.extend_synthetic_subfolders(synthetic.drain(..remaining_in_last));

    let mut remaining = synthetic.len();
    let mut pages = Vec::with_capacity(1 + remaining.div_ceil(cap));
    let mut emitted = remaining_in_last;
    last_inner_page.set_continue_token(synthetic_resume_token(emitted));
    pages.push(last_inner_page);

    // Consume the overflow by value, cap items at a time -- the strings are
    // moved straight into each page, never cloned.
    let mut overflow = synthetic.into_iter();
    while remaining > 0 {
        let take = cap.min(remaining);
        remaining -= take;
        emitted += take;
        let mut page = R::new_synthetic_only_page(overflow.by_ref().take(take));
        if remaining > 0 {
            page.set_continue_token(synthetic_resume_token(emitted));
        }
        pages.push(page);
    }
    pages
}

pub fn resume_page<R>(synthetic: Vec<String>, skip: usize, max_page_size: Option<usize>) -> R
where
    R: SyntheticPaginable,
{
    let cap = max_page_size.map(|n| n.max(1)).unwrap_or(usize::MAX);
    let total = synthetic.len();
    let next_skip = skip.saturating_add(cap).min(total);
    let mut response = R::new_synthetic_only_page(synthetic.into_iter().skip(skip).take(cap));
    if next_skip < total {
        response.set_continue_token(synthetic_resume_token(next_skip));
    }
    response
}

fn dedupe_real_against_synthetic<R: SyntheticPaginable>(parent_url: &str, response: &mut R, synthetic: &[String]) {
    debug_assert!(
        !synthetic.is_empty(),
        "caller must short-circuit the empty-synthetic case before invoking dedupe"
    );
    let synthetic_set: HashSet<&str> = synthetic.iter().map(String::as_str).collect();
    let dropped = response.drop_real_subfolders_with_uri(&synthetic_set);
    if !dropped.is_empty() {
        warn!(
            ?parent_url,
            ?dropped,
            "real subfolder addresses collide with synthetic mount-points; dropping real entries, rule wins",
        );
    }
}
