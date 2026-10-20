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

//! The worker must associate only through thumbnail-service's gRPC API.
//!
//! `thumbnail-common` legitimately exports association writers — the service
//! itself needs them — and the worker links that crate for the work-event
//! contract and the outbound plumbing. So the boundary cannot be enforced by
//! visibility, and "we agreed not to" is not enforcement either: the failure
//! this guards against is someone reaching for the convenient local write during
//! a debugging session and it surviving review.
//!
//! Scanning the worker's own source for those entry points is mechanical, cheap,
//! and fails loudly the moment one appears. If a legitimate use ever arises, this
//! test is the place to argue it.

use std::path::Path;

/// Entry points that would establish an association without the service, and
/// the reason each is forbidden here.
const FORBIDDEN: &[(&str, &str)] = &[
    (
        "store_association",
        "writes association metadata directly, bypassing AssociateThumbnail",
    ),
    (
        "disassociate_thumbnail",
        "removes associations, which is the service's to own",
    ),
    (
        "disassociate_all_thumbnails",
        "sweeps associations, which is the service's to own",
    ),
    (
        "update_storage_metadata",
        "is the metadata write underneath association storage",
    ),
    (
        "remove_storage_metadata",
        "is the metadata removal underneath association storage",
    ),
    (
        "AssociationEventKind",
        "publishing omni.thumbnail.associated is the service's job after its own write",
    ),
    (
        "publish_event",
        "the worker publishes nothing; association events follow the service's write",
    ),
];

fn worker_sources() -> Vec<(String, String)> {
    fn collect(dir: &Path, out: &mut Vec<(String, String)>) {
        for entry in std::fs::read_dir(dir).expect("worker source directory must be readable") {
            let path = entry.expect("directory entry").path();
            if path.is_dir() {
                collect(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let contents =
                    std::fs::read_to_string(&path).expect("source file must be readable");
                out.push((path.display().to_string(), contents));
            }
        }
    }

    let mut sources = Vec::new();
    collect(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut sources,
    );
    assert!(!sources.is_empty(), "the scan found no worker sources");
    sources
}

#[test]
fn the_worker_never_writes_associations_itself() {
    let sources = worker_sources();
    let mut violations = Vec::new();

    for (path, contents) in &sources {
        for (line_number, line) in contents.lines().enumerate() {
            // Prose is where the rule is explained, so only code counts.
            let code = line.trim_start();
            if code.starts_with("//") || code.starts_with("*") {
                continue;
            }

            for (symbol, reason) in FORBIDDEN {
                if code.contains(symbol) {
                    violations.push(format!("{path}:{}: '{symbol}' {reason}", line_number + 1));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "the worker must associate only via thumbnail-service gRPC:\n{}",
        violations.join("\n")
    );
}

/// The positive half: the worker does call the association RPC, so the test
/// above cannot pass simply because association was dropped entirely.
#[test]
fn the_worker_does_call_the_association_rpc() {
    let sources = worker_sources();

    assert!(
        sources
            .iter()
            .find(|(path, _)| path.ends_with("pipeline.rs"))
            .is_some_and(
                |(_, contents)| contents.contains("if let Err(error) = associate_thumbnail(")
            ),
        "the worker should call AssociateThumbnail; a passing boundary check means \
         nothing if nothing associates at all"
    );
}

/// The worker also has no business talking to the HTTP association endpoint: v1
/// uses gRPC only, with no transport switch.
#[test]
fn the_worker_does_not_use_the_http_association_endpoint() {
    for (path, contents) in worker_sources() {
        assert!(
            !contents.contains("/v1alpha/associations"),
            "{path} references the HTTP association endpoint; v1 associates over gRPC only"
        );
    }
}
