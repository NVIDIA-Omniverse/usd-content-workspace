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

//! Fixture sampling: every case that needs an association creates and tears
//! down its own thumbnail address rather than assuming any pre-existing
//! *association or thumbnail* data in the target deployment (see the "Fixture
//! sampling, not seeded data" requirement in
//! specs/thumbnail-conformance-test-suite/spec.md). A run against a deployment
//! with no existing associations must pass exactly the same as one against a
//! long-lived instance with unrelated ones.
//!
//! The caller-supplied source file is the exception and must already exist: the
//! cases here only write metadata keyed on its address, but CreateThumbnails
//! resolves the source's identity with a storage Stat. See the "caller-supplied
//! source file must resolve in storage" requirement.
//!
//! The fixtures this module names are the ones the suite can remove, and that is
//! the whole of them. The `CreateThumbnails` cases that reach the `Stat`
//! generate real thumbnails whose addresses the *deployment's* path template
//! renders, so nothing here can predict or name them — see the teardown
//! reasoning in AGENTS.md's "Fixture sampling" section. A helper for those would
//! have to guess an address, and guessing wrong means deleting an association
//! the suite never made.

use uuid::Uuid;

/// A thumbnail address unique to this run and this case, so repeated or
/// concurrent runs against the same deployment never collide on a stale
/// association left over from a previous (possibly failed) run.
pub fn unique_thumbnail_address(source_file: &str, label: &str) -> String {
    format!(
        "{source_file}.conformance-test-{label}-{}.png",
        Uuid::new_v4()
    )
}
