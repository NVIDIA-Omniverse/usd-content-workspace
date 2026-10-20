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

use std::sync::Arc;

use axum::http::HeaderMap;
use fluent_uri::{pct_enc::DecodedChunk, Uri};
use parking_lot::RwLock;
use url_escape::{
    percent_encoding::{percent_encode, AsciiSet},
    NON_ALPHANUMERIC,
};

pub mod grpc;
pub mod rest;

const URI_COMPONENT_SET: AsciiSet = NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'!')
    .remove(b'~')
    .remove(b'*')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')')
    .remove(b'/');

#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum Decision {
    Allow,
    Deny,
    Missing,
}

/// Access evaluation context.
#[derive(Default, Debug, Clone)]
pub struct Context {
    inner: Arc<RwLock<ContextInner>>,
}

impl Context {
    /// Create an empty context.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new rule set.
    pub fn new_rule_set(&self) -> RuleSetGuard {
        let mut context = self.inner.write();

        let scope_id = context.rule_set_id_counter;
        context.rule_set_id_counter += 1;
        context.rule_sets.push(RuleSet {
            id: scope_id,
            ..Default::default()
        });

        RuleSetGuard {
            context: self.inner.clone(),
            rule_set_id: scope_id,
        }
    }

    /// Remove and return the headers of the recorded requests that evaluated the specified url, in arrival
    /// order. Requests about other urls stay recorded, so concurrently running tests do not observe each other.
    pub fn take_recorded_headers(&self, uri: &str) -> Vec<HeaderMap> {
        let uri = RuleSetGuard::encode_path_segments(uri);

        let mut context = self.inner.write();
        let (matching, others) = std::mem::take(&mut context.recorded_requests)
            .into_iter()
            .partition::<Vec<_>, _>(|request| request.uris.contains(&uri));
        context.recorded_requests = others;

        matching.into_iter().map(|request| request.headers).collect()
    }

    /// Record a received request with the urls it evaluated.
    pub(self) fn record_request(&self, headers: HeaderMap, uris: Vec<String>) {
        self.inner
            .write()
            .recorded_requests
            .push(RecordedRequest { headers, uris });
    }

    /// Evaluate access for the specified url and action.
    pub(self) fn evaluate_access(&self, kind: &str, uri: &str, action: &str) -> Decision {
        let context = self.inner.read();
        for scope in context.rule_sets.iter().rev() {
            for rule in scope.rules.iter().rev() {
                if rule.kind == kind && rule.uri == uri && rule.action == action {
                    return rule.decision;
                }
            }
        }

        Decision::Missing
    }
}

/// Scoped configuration guard.
#[derive(Debug)]
pub struct RuleSetGuard {
    context: Arc<RwLock<ContextInner>>,
    rule_set_id: usize,
}

impl RuleSetGuard {
    /// Allow read permission for the specified object.
    pub fn allow_object_read(&self, url: impl Into<String>) {
        self.set_object_read(url, Decision::Allow);
    }

    /// Forbid read for the specified object.
    pub fn forbid_object_read(&self, url: impl Into<String>) {
        self.set_object_read(url, Decision::Deny);
    }

    /// Allow write for the specified object.
    pub fn allow_object_write(&self, url: impl Into<String>) {
        self.set_object_write(url, Decision::Allow);
    }

    /// Allow read for the specified folder.
    pub fn allow_folder_read(&self, url: impl Into<String>) {
        self.set_folder_read(url, Decision::Allow);
    }

    /// Allow write for the specified folder.
    pub fn allow_folder_write(&self, url: impl Into<String>) {
        self.set_folder_write(url, Decision::Allow);
    }

    /// Forbid read for the specified folder.
    pub fn forbid_folder_read(&self, url: impl Into<String>) {
        self.set_folder_read(url, Decision::Deny);
    }

    /// Forbid write for the specified folder.
    pub fn forbid_folder_write(&self, url: impl Into<String>) {
        self.set_folder_write(url, Decision::Deny);
    }

    /// Forbid write for the specified folder.
    pub fn forbid_object_write(&self, url: impl Into<String>) {
        self.set_object_write(url, Decision::Deny);
    }

    /// Configure permission for the object write operation.
    pub fn set_object_write(&self, url: impl Into<String>, decision: Decision) {
        self.set_decision("object", url, "write", decision);
    }

    /// Configure permission for the object read operation.
    pub fn set_object_read(&self, url: impl Into<String>, decision: Decision) {
        self.set_decision("object", url, "read", decision);
    }

    /// Configure permission for the folder read operation.
    pub fn set_folder_read(&self, url: impl Into<String>, decision: Decision) {
        self.set_decision("folder", url, "read", decision);
    }

    /// Configure permission for the folder write operation.
    pub fn set_folder_write(&self, url: impl Into<String>, decision: Decision) {
        self.set_decision("folder", url, "write", decision);
    }

    /// Configure a permission decision for an action over a resource.
    pub fn set_decision(
        &self,
        kind: impl Into<String>,
        uri: impl Into<String>,
        action: impl Into<String>,
        decision: Decision,
    ) {
        let uri = Self::encode_path_segments(&uri.into());

        let mut context = self.context.write();
        let idx = context
            .rule_sets
            .binary_search_by_key(&self.rule_set_id, |scope| scope.id)
            .expect("rule set not found");
        context.rule_sets[idx].rules.push(Rule {
            kind: kind.into(),
            action: action.into(),
            uri,
            decision,
        });
    }

    /// Discard this scope with the associated rules.
    pub fn discard(self) {
        drop(self);
    }

    /// Encode path segments in the specified uri.
    fn encode_path_segments(uri: &str) -> String {
        let url = Uri::parse(uri).expect("invalid url");
        assert!(url.authority().is_some());

        // Consistently encode the path segments.
        let mut buffer = String::new();
        buffer.push_str(url.scheme().as_str());
        buffer.push_str("://");
        buffer.push_str(url.authority().unwrap().as_str());

        for chunk in url.path().decode() {
            match chunk {
                DecodedChunk::PctDecoded(decoded) => {
                    for value in percent_encode(&[decoded], &URI_COMPONENT_SET) {
                        buffer.push_str(value);
                    }
                },
                DecodedChunk::Unencoded(value) => {
                    for value in percent_encode(value.as_bytes(), &URI_COMPONENT_SET) {
                        buffer.push_str(value);
                    }
                },
            }
        }

        buffer
    }
}

impl Drop for RuleSetGuard {
    fn drop(&mut self) {
        let mut context = self.context.write();
        if let Ok(idx) = context
            .rule_sets
            .binary_search_by_key(&self.rule_set_id, |scope| scope.id)
        {
            context.rule_sets.remove(idx);
        }
    }
}

/// A rule in a rule set.
#[derive(Debug)]
struct Rule {
    uri: String,
    kind: String,
    action: String,
    decision: Decision,
}

/// A rule set in the context.
#[derive(Default, Debug)]
struct RuleSet {
    id: usize,
    rules: Vec<Rule>,
}

/// A request received by the mock.
#[derive(Debug)]
struct RecordedRequest {
    headers: HeaderMap,
    uris: Vec<String>,
}

/// A set of rules.
#[derive(Default, Debug)]
struct ContextInner {
    rule_sets: Vec<RuleSet>,
    rule_set_id_counter: usize,
    recorded_requests: Vec<RecordedRequest>,
}
