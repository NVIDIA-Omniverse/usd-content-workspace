// SPDX-FileCopyrightText: Copyright 2026 NVIDIA CORPORATION & AFFILIATES
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

//! REST conformance tests for the public `/api/v1alpha/...` endpoints.

pub mod groups;
pub mod users;

use crate::TestRunner;
use crate::fixtures::SampledFixtures;

/// Top-level REST conformance entry point.
pub async fn run(
    runner: &mut TestRunner,
    client: &reqwest::Client,
    rest_url: &str,
    auth_header: Option<&str>,
    data: &SampledFixtures,
) {
    let rest_url = rest_url.trim_end_matches('/');
    users::run_tests(runner, client, rest_url, auth_header, data).await;
    groups::run_tests(runner, client, rest_url, auth_header, data).await;
}
