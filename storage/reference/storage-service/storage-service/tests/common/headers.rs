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

use std::sync::atomic::{AtomicUsize, Ordering};

/// Get the headers to add to each request
pub fn get_headers() -> Vec<(String, String)> {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    vec![
        // This should be returned by the server
        (
            "x-request-id".to_string(),
            format!("test-{}", COUNTER.fetch_add(1, Ordering::Relaxed)),
        ),
        // This should not be returned by the server
        ("x-omniverse-request-id".to_string(), "zomgwtfbbq".to_string()),
        ("authorization".to_string(), "Bearer valid.token".to_string()),
    ]
}
