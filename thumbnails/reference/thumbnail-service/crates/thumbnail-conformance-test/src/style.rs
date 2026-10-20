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

//! Console output: PASS/FAIL/INFO/WARN markers and the pass/fail summary.
//!
//! Ported from `thumbnail-smoke-test`'s style, extended with a `suite` label
//! (REST vs gRPC) so a single run's summary can distinguish which transport a
//! failure came from.

use std::time::Instant;

pub const PASS: &str = "[PASS]";
pub const FAIL: &str = "[FAIL]";
pub const INFO: &str = "[INFO]";
pub const WARN: &str = "[WARN]";

/// A failure that is the run's own configuration rather than the deployment's
/// contract, and which flag the operator should look at.
///
/// A kind rather than a bool because the bool answered only one of the three
/// ways an invocation can be wrong. A rejected token and a mismatched endpoint
/// both made every case report a contract failure, so a run given the wrong
/// credentials accused the deployment of non-conformance in every line of its
/// output. Raised by Mitri on !221 as findings 10 and 11.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigurationFault {
    /// `--source-file` names nothing that resolves in storage.
    UnresolvableSource,
    /// The deployment rejected the run's credentials, or it has none.
    RejectedCredentials,
    /// The caller lacks access to the API or the requested resource.
    AccessDenied,
    /// The endpoint answered, but not with this API — a wrong `--rest-url` or
    /// `--grpc-url`, or a deployment not serving the route under test.
    EndpointMismatch,
    ClientTimeout,
    UnverifiableContract,
}

impl ConfigurationFault {
    /// The summary heading for this kind: what is wrong, and what to change.
    fn advice(self) -> &'static str {
        match self {
            Self::ClientTimeout => "incomplete observation: the client aborted at --http-timeout. Increase the timeout and rerun; this does not establish deployment health",
            Self::UnverifiableContract => "the advertised contract could not be verified. Check --rest-url and /api-docs/openapi.json; --rest-url is required for gRPC-only runs too",
            Self::UnresolvableSource => {
                "--source-file did not resolve in storage. Point it at an object that exists"
            }
            Self::RejectedCredentials => {
                "the deployment rejected this run's credentials. Check --bearer-token, \
                 --access-token-file or the --oauth2-* flags; an unauthenticated run only works \
                 against a deployment with authentication disabled"
            }
            Self::AccessDenied => {
                "access was denied. Check caller permissions and token scopes for the API and \
                 --source-file; this does not establish that the credentials are invalid"
            }
            Self::EndpointMismatch => {
                "the endpoint did not answer with this API. Check --rest-url and --grpc-url, and \
                 that the deployment serves the route under test"
            }
        }
    }
}

#[derive(Debug)]
pub struct TestError {
    pub message: String,
    /// Set by the case, not inferred from the message: the runner would
    /// otherwise have to match on prose that every case words differently, and
    /// a case added later would fall out of the classification silently.
    pub configuration_fault: Option<ConfigurationFault>,
}

impl TestError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            configuration_fault: None,
        }
    }

    /// A failure the run's own invocation caused. See [`ConfigurationFault`].
    pub fn configuration(fault: ConfigurationFault, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            configuration_fault: Some(fault),
        }
    }

    /// A failure caused by `--source-file` not resolving in storage.
    ///
    /// There is no read-only existence check on this API to catch it up front:
    /// `GET /v1alpha/thumbnails` answers an unknown address with an empty
    /// success and associate writes metadata without validating the target, so
    /// `CreateThumbnails` is the only operation that learns whether the source
    /// resolves and it learns by doing the work. A probe would therefore have to
    /// enqueue generation before the run announced its first case. Classifying
    /// the failure afterwards is what is left, and it is enough for the actual
    /// complaint: a misconfigured run read identically to a broken deployment.
    pub fn unresolvable_source(message: impl Into<String>) -> Self {
        Self::configuration(ConfigurationFault::UnresolvableSource, message)
    }
}

impl std::fmt::Display for TestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for TestError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Suite {
    Rest,
    Grpc,
}

impl std::fmt::Display for Suite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Suite::Rest => write!(f, "REST"),
            Suite::Grpc => write!(f, "gRPC"),
        }
    }
}

struct TestResult {
    suite: Suite,
    name: String,
    kind: TestResultKind,
    duration_ms: u128,
    error: Option<String>,
    configuration_fault: Option<ConfigurationFault>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TestResultKind {
    Pass,
    Fail,
    /// Not run, because a case it depends on did not pass.
    ///
    /// Selected coverage was not observed, so a skip prevents certification.
    Skip,
}

/// Accumulates pass/fail results across both suites and prints the running
/// output plus a final summary. Unlike `thumbnail-smoke-test`'s runner, this
/// one does not stop at the first failure — a conformance run reports every
/// case so a customer sees the full extent of a non-conformance, not just the
/// first symptom.
pub struct TestRunner {
    results: Vec<TestResult>,
}

impl Default for TestRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl TestRunner {
    pub fn new() -> Self {
        Self {
            results: Vec::new(),
        }
    }

    pub fn record_pass(&mut self, suite: Suite, name: &str, duration_ms: u128) {
        println!("{PASS} [{suite}] {name} ({duration_ms}ms)");
        self.results.push(TestResult {
            suite,
            name: name.to_string(),
            kind: TestResultKind::Pass,
            duration_ms,
            error: None,
            configuration_fault: None,
        });
    }

    pub fn record_fail(&mut self, suite: Suite, name: &str, duration_ms: u128, error: &TestError) {
        println!("{FAIL} [{suite}] {name} ({duration_ms}ms)");
        println!("       {}", error.message);
        self.results.push(TestResult {
            suite,
            name: name.to_string(),
            kind: TestResultKind::Fail,
            duration_ms,
            error: Some(error.message.clone()),
            configuration_fault: error.configuration_fault,
        });
    }

    /// Record a case that was not run because a prerequisite did not pass.
    ///
    /// The reason is required and printed, because "skipped" on its own is the
    /// thing this suite refuses to do: a certification that quietly omits cases
    /// is worse than one that fails.
    pub fn record_skip(&mut self, suite: Suite, name: &str, reason: &str) {
        println!("{WARN} [{suite}] {name} (skipped)");
        println!("       {reason}");
        self.results.push(TestResult {
            suite,
            name: name.to_string(),
            kind: TestResultKind::Skip,
            duration_ms: 0,
            error: Some(reason.to_string()),
            configuration_fault: None,
        });
    }

    /// Run a single async test case, recording the outcome. Returns whether it
    /// passed so callers can short-circuit dependent cases (e.g. skip the list
    /// assertion if the associate call itself failed) without duplicating the
    /// timing/record boilerplate at every call site.
    pub async fn run<F, Fut>(&mut self, suite: Suite, name: &str, case: F) -> bool
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<(), TestError>>,
    {
        let start = Instant::now();
        match case().await {
            Ok(()) => {
                self.record_pass(suite, name, start.elapsed().as_millis());
                true
            }
            Err(error) => {
                self.record_fail(suite, name, start.elapsed().as_millis(), &error);
                false
            }
        }
    }

    pub fn print_summary(&self) {
        for line in self.summary_lines() {
            println!("{line}");
        }
    }

    /// The summary as lines, so the rendering can be asserted on.
    ///
    /// Separated from printing for exactly one reason: the configuration-fault
    /// block was deletable and invertible with every test still green, because
    /// the tests covered the classification and nothing covered the output that
    /// carries it to an operator. Mitri raised that on !221 as finding 16.
    pub(crate) fn summary_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        let total = self.results.len();
        let passed = self
            .results
            .iter()
            .filter(|r| r.kind == TestResultKind::Pass)
            .count();
        let skipped = self
            .results
            .iter()
            .filter(|r| r.kind == TestResultKind::Skip)
            .count();
        let failed = total - passed - skipped;
        let total_ms: u128 = self.results.iter().map(|r| r.duration_ms).sum();

        lines.push(String::new());
        lines.push("----------------------------------------".to_string());
        for result in &self.results {
            match result.kind {
                TestResultKind::Pass => {
                    lines.push(format!("{PASS} [{}] {}", result.suite, result.name))
                }
                TestResultKind::Fail => {
                    let error = result.error.as_deref().unwrap_or("(no detail)");
                    lines.push(format!(
                        "{FAIL} [{}] {} -> {}",
                        result.suite, result.name, error
                    ));
                }
                TestResultKind::Skip => {
                    let reason = result.error.as_deref().unwrap_or("(no detail)");
                    lines.push(format!(
                        "{WARN} [{}] {} -> skipped: {}",
                        result.suite, result.name, reason
                    ));
                }
            }
        }
        lines.push("----------------------------------------".to_string());
        let skipped_note = if skipped == 0 {
            String::new()
        } else {
            format!(", {skipped} skipped")
        };
        lines.push(format!(
            "{INFO} {passed} passed, {failed} failed{skipped_note} ({total} total, {total_ms}ms)"
        ));

        // One block per kind, in a fixed order so the output does not depend on
        // which case happened to fail first.
        for fault in [
            ConfigurationFault::RejectedCredentials,
            ConfigurationFault::AccessDenied,
            ConfigurationFault::EndpointMismatch,
            ConfigurationFault::UnresolvableSource,
            ConfigurationFault::ClientTimeout,
            ConfigurationFault::UnverifiableContract,
        ] {
            let affected = self.configuration_faults(fault);
            if affected.is_empty() {
                continue;
            }
            lines.push(String::new());
            if fault == ConfigurationFault::ClientTimeout {
                lines.push(format!(
                    "{WARN} incomplete run: {}. {} case(s) below have no complete response; requests may have reached the server.",
                    fault.advice(), affected.len()
                ));
            } else {
                lines.push(format!(
                    "{WARN} misconfigured run: {}. {} case(s) below could not verify their contract.",
                    fault.advice(), affected.len()
                ));
            }
            for result in affected {
                lines.push(format!("       [{}] {}", result.suite, result.name));
            }
        }
        lines.push(String::new());
        lines
    }

    /// Failures of one configuration kind, which the deployment did not cause.
    ///
    /// Reported separately from the count because they are indistinguishable
    /// inside it: three failed cases read as a non-conformant deployment
    /// whichever of the two produced them, and only one of them is worth
    /// escalating to whoever owns the service.
    fn configuration_faults(&self, fault: ConfigurationFault) -> Vec<&TestResult> {
        self.results
            .iter()
            .filter(|r| r.kind == TestResultKind::Fail && r.configuration_fault == Some(fault))
            .collect()
    }

    pub fn all_passed(&self) -> bool {
        !self.results.is_empty() && self.results.iter().all(|r| r.kind == TestResultKind::Pass)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The classification is only useful if a case that hits an unresolvable
    /// source constructs its error through `unresolvable_source`, and only
    /// honest if nothing else does. A case added later with a plain
    /// `TestError::new` on its not-found branch reports a contract failure for a
    /// misconfigured run, which is the complaint this exists to answer.
    #[test]
    fn only_configuration_failures_are_classified_and_only_under_their_own_kind() {
        let mut runner = TestRunner::new();
        runner.record_pass(Suite::Rest, "a passing case", 1);
        runner.record_fail(
            Suite::Rest,
            "a contract failure",
            1,
            &TestError::new("returned 500"),
        );
        runner.record_fail(
            Suite::Rest,
            "a rejected token",
            1,
            &TestError::configuration(ConfigurationFault::RejectedCredentials, "returned 401"),
        );
        runner.record_fail(
            Suite::Grpc,
            "a misconfigured source",
            1,
            &TestError::unresolvable_source("returned NOT_FOUND"),
        );

        let sources = runner.configuration_faults(ConfigurationFault::UnresolvableSource);
        assert_eq!(
            sources.len(),
            1,
            "the contract failure is not the run's own, and the token fault is a different kind"
        );
        assert_eq!(sources[0].name, "a misconfigured source");

        let credentials = runner.configuration_faults(ConfigurationFault::RejectedCredentials);
        assert_eq!(credentials.len(), 1);
        assert_eq!(credentials[0].name, "a rejected token");

        assert!(
            runner
                .configuration_faults(ConfigurationFault::EndpointMismatch)
                .is_empty(),
            "a kind nothing was recorded under must stay empty"
        );
    }

    /// The classification is only worth anything if it reaches the operator, and
    /// nothing covered the block that carries it: it was deletable, and its
    /// emptiness test invertible, with every other test still green.
    #[test]
    fn the_summary_names_each_configuration_fault_and_the_flag_to_change() {
        let mut runner = TestRunner::new();
        runner.record_fail(
            Suite::Rest,
            "a rejected token",
            1,
            &TestError::configuration(ConfigurationFault::RejectedCredentials, "returned 401"),
        );
        runner.record_fail(
            Suite::Grpc,
            "a misconfigured source",
            1,
            &TestError::unresolvable_source("returned NOT_FOUND"),
        );

        let summary = runner.summary_lines().join("\n");

        assert!(summary.contains("--bearer-token"), "{summary}");
        assert!(summary.contains("--source-file"), "{summary}");
        assert_eq!(
            summary.matches(WARN).count(),
            2,
            "one block per kind present, and none for a kind that is absent: {summary}"
        );
        assert!(
            !summary.contains("--rest-url"),
            "no endpoint-mismatch block when nothing was classified as one: {summary}"
        );
        // Both affected cases are listed under their block, so an operator can
        // see which failures the misconfiguration accounts for.
        assert!(summary.contains("a rejected token"), "{summary}");
        assert!(summary.contains("a misconfigured source"), "{summary}");
    }

    #[test]
    fn denied_access_reports_permissions_instead_of_rejected_credentials() {
        let mut runner = TestRunner::new();
        runner.record_fail(
            Suite::Grpc,
            "CreateThumbnails denied source access",
            0,
            &TestError::configuration(ConfigurationFault::AccessDenied, "PERMISSION_DENIED"),
        );
        let summary = runner.summary_lines().join("\n");
        assert!(summary.contains("caller permissions") && summary.contains("token scopes"));
        assert!(summary.contains("--source-file"));
        assert!(!summary.contains("deployment rejected this run's credentials"));
        assert_eq!(summary.matches(WARN).count(), 1, "{summary}");
        assert!(!runner.all_passed());
    }

    #[test]
    fn a_run_with_no_configuration_fault_prints_no_warning() {
        let mut runner = TestRunner::new();
        runner.record_pass(Suite::Rest, "a passing case", 1);
        runner.record_fail(
            Suite::Rest,
            "a contract failure",
            1,
            &TestError::new("returned 500"),
        );

        let summary = runner.summary_lines().join("\n");
        assert!(
            !summary.contains(WARN),
            "a genuine contract failure must not be excused as misconfiguration: {summary}"
        );
    }

    /// A misconfigured run still fails. Reporting the cause differently is not
    /// the same as tolerating it: a run that certified a deployment while
    /// quietly excusing the cases it could not reach would be worse than one
    /// that fails.
    #[test]
    fn a_configuration_fault_still_fails_the_run() {
        let mut runner = TestRunner::new();
        runner.record_fail(
            Suite::Rest,
            "a misconfigured run",
            1,
            &TestError::unresolvable_source("returned 404"),
        );

        assert!(!runner.all_passed());
    }
}

#[cfg(test)]
mod skip_tests {
    use super::*;
    #[test]
    fn selected_skip_is_visible_excluded_from_passes_and_cannot_certify() {
        let mut runner = TestRunner::new();
        runner.record_pass(Suite::Rest, "accepted", 1);
        runner.record_skip(
            Suite::Grpc,
            "dependent association",
            "association prerequisite did not pass",
        );
        assert_eq!(runner.results[1].kind, TestResultKind::Skip);
        assert!(!runner.all_passed());
        let summary = runner.summary_lines().join("\n");
        assert!(
            summary.contains("1 passed, 0 failed, 1 skipped"),
            "{summary}"
        );
        assert!(
            summary.contains("[WARN] [gRPC] dependent association") && summary.contains("skipped"),
            "{summary}"
        );
        assert!(summary.contains("association prerequisite did not pass"));
        assert!(!TestRunner::new().all_passed());
    }
    #[test]
    fn timeout_summary_explains_incomplete_observation() {
        let mut runner = TestRunner::new();
        runner.record_fail(
            Suite::Grpc,
            "timed out",
            1,
            &TestError::configuration(ConfigurationFault::ClientTimeout, "deadline elapsed"),
        );
        let summary = runner.summary_lines().join("\n");
        assert!(summary.contains("--http-timeout") && summary.contains("incomplete observation"));
        assert!(summary.contains("requests may have reached the server"));
        assert!(!runner.all_passed());
    }
}
