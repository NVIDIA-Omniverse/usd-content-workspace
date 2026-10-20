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

//! Access logging: configuration, header copying, the response envelope, and the
//! single request-scoped collector shared by the REST middleware and the gRPC
//! Tower layer. The collector lives in a task-local scope (no `Arc`/`Mutex`);
//! handlers record `params`/`detail` through the zero-sized [`AccessLog`] handle,
//! and the transport emits once after the response. The record-building helpers
//! never inspect request bodies or map endpoint types.

use crate::service::authorization::AuthorizationErr;
use axum::http::StatusCode;
use clap::{ArgAction, Args};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::future::Future;
use tonic::Status;

/// Tracing target that carries access-log events. Filtered by `RUST_LOG` like
/// any other target and used to route events to the dedicated `AccessLogLayer`.
pub const ACCESS_TARGET: &str = "access";

/// `transport` value for REST-over-HTTP calls; tells consumers to read
/// `response.status` as an HTTP status code.
const TRANSPORT_REST: &str = "REST";

/// `transport` value for gRPC calls; tells consumers to read `response.status`
/// as a numeric gRPC status code.
const TRANSPORT_GRPC: &str = "gRPC";

/// Access-field names that are reserved for the envelope and can never be
/// overwritten by an operator-configured header mapping.
const RESERVED_FIELDS: [&str; 4] = ["transport", "operation", "request", "response"];

/// Clap/env-backed access-log arguments, flattened onto `ServiceConfig`.
#[derive(Args, Clone, Debug)]
pub struct AccessLogArgs {
    /// Master switch for API access logs. Disabled by default.
    #[arg(
        id = "access-log-enabled",
        long = "access-log-enabled",
        env = "ACCESS_LOG_ENABLED",
        action = ArgAction::Set,
        default_value_t = false,
        default_missing_value = "true",
        num_args = 0..=1,
        require_equals = false
    )]
    pub enabled: bool,

    /// JSON object mapping an access field name to the request header whose value
    /// should be copied onto every access line (e.g. `{"ip":"x-forwarded-for"}`).
    #[arg(
        id = "access-log-headers",
        long = "access-log-headers",
        env = "ACCESS_LOG_HEADERS",
        default_value = "{}"
    )]
    pub headers: String,
}

impl AccessLogArgs {
    /// Resolves the clap arguments into the runtime [`AccessLogConfig`] consumed
    /// by the collectors and emit helpers. `structured` mirrors the tracing mode
    /// and selects the emit shape (JSON envelope vs. a plain pretty message).
    pub fn config(&self, structured: bool) -> AccessLogConfig {
        AccessLogConfig {
            enabled: self.enabled,
            structured,
            headers: self.parse_headers(),
        }
    }

    fn parse_headers(&self) -> Vec<(String, String)> {
        match serde_json::from_str::<BTreeMap<String, String>>(&self.headers) {
            Ok(map) => map.into_iter().collect(),
            Err(err) => {
                tracing::warn!(
                    "Failed to parse ACCESS_LOG_HEADERS as a JSON object: {}",
                    err
                );
                Vec::new()
            }
        }
    }
}

/// Runtime access-log configuration resolved from [`AccessLogArgs`].
#[derive(Clone, Debug)]
pub struct AccessLogConfig {
    pub enabled: bool,
    /// Mirrors the tracing mode: `true` emits the JSON envelope for the
    /// `AccessLogLayer`, `false` emits a plain message the fmt layer formats.
    pub structured: bool,
    /// Ordered `(access field name, request header name)` pairs.
    pub headers: Vec<(String, String)>,
}

impl AccessLogConfig {
    /// Test-only constructor for a fully specified configuration.
    #[cfg(test)]
    pub fn new(enabled: bool, headers: Vec<(String, String)>) -> Self {
        Self {
            enabled,
            structured: false,
            headers,
        }
    }
}

/// One request-side record awaiting the shared response envelope. `params` is the
/// endpoint-selected input; `detail` is an optional per-item success object.
#[derive(Clone, Debug)]
pub struct AccessRecord {
    pub params: Value,
    pub detail: Option<Value>,
}

impl AccessRecord {
    pub fn new(params: Value) -> Self {
        Self {
            params,
            detail: None,
        }
    }
}

/// Response side of one access line. `detail` is always a JSON object when
/// present, never a bare string or array.
#[derive(Clone, Debug, Serialize)]
pub struct AccessResponse {
    pub status: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Value>,
}

/// Wraps a plain text message as the homogeneous `detail` object so a response
/// detail is always a struct (`{ "message": "..." }`).
pub fn message_detail(message: impl Into<String>) -> Value {
    json!({ "message": message.into() })
}

/// Resolves the operator-configured header extras against a case-normalized
/// lookup. Reserved field names, `Authorization`, and `Cookie` are never copied,
/// and missing or empty headers are omitted.
pub fn resolve_header_extras<F>(config: &AccessLogConfig, lookup: F) -> Vec<(String, String)>
where
    F: Fn(&str) -> Option<String>,
{
    let mut extras = Vec::new();
    for (field, header) in &config.headers {
        if RESERVED_FIELDS.contains(&field.as_str()) {
            continue;
        }
        let header = header.to_ascii_lowercase();
        if header == "authorization" || header == "cookie" {
            continue;
        }
        if let Some(value) = lookup(&header) {
            if !value.is_empty() {
                extras.push((field.clone(), value));
            }
        }
    }
    extras
}

/// Emits the access lines for one API call as `info` events on target
/// [`ACCESS_TARGET`], applying the cap and truncation rules.
pub fn emit_access(
    config: &AccessLogConfig,
    transport: &str,
    operation: &str,
    header_extras: &[(String, String)],
    records: &[AccessRecord],
    status: u16,
    failure: Option<&Value>,
) {
    for access in build_access_objects(
        config,
        transport,
        operation,
        header_extras,
        records,
        status,
        failure,
    ) {
        if config.structured {
            if let Ok(serialized) = serde_json::to_string(&access) {
                tracing::info!(target: ACCESS_TARGET, access = serialized.as_str(), "access");
            }
        } else {
            tracing::info!(target: ACCESS_TARGET, "{}", pretty_body(&access));
        }
    }
}

/// Renders one `access` object as an indented, multi-line message for pretty
/// output: a leading newline, then one `path: value` line per leaf field. Nested
/// keys are joined with `.` and array elements indexed with `[n]`, so the fmt
/// layer prints readable rows instead of embedded JSON.
fn pretty_body(access: &Value) -> String {
    let mut fields = Vec::new();
    flatten_access(String::new(), access, &mut fields);
    let mut body = String::new();
    for (path, value) in fields {
        body.push_str("\n    ");
        body.push_str(&path);
        body.push_str(": ");
        body.push_str(&value);
    }
    body
}

fn flatten_access(prefix: String, value: &Value, out: &mut Vec<(String, String)>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                flatten_access(path, child, out);
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                flatten_access(format!("{prefix}[{index}]"), child, out);
            }
        }
        Value::String(text) => out.push((prefix, text.clone())),
        other => out.push((prefix, other.to_string())),
    }
}

/// Pure builder for the `access` objects of one API call. Returns one object per
/// record, adding `request.index` when a call emits more than one line. When a
/// call produced no records (rejected before or without a handler), a single
/// call-level object with no `request.params` is returned so invalid requests are
/// still logged. Separated from [`emit_access`] so the envelope shape is
/// unit-testable without a subscriber.
pub(crate) fn build_access_objects(
    config: &AccessLogConfig,
    transport: &str,
    operation: &str,
    header_extras: &[(String, String)],
    records: &[AccessRecord],
    status: u16,
    failure: Option<&Value>,
) -> Vec<Value> {
    if !config.enabled {
        return Vec::new();
    }

    if records.is_empty() {
        return vec![access_object(
            transport,
            operation,
            header_extras,
            Value::Object(Map::new()),
            status,
            failure,
        )];
    }

    let multiple = records.len() > 1;
    let mut objects = Vec::with_capacity(records.len());
    for (index, record) in records.iter().enumerate() {
        let mut request = Map::new();
        if multiple {
            request.insert("index".to_string(), json!(index));
        }
        request.insert("params".to_string(), record.params.clone());
        let detail = failure.or(record.detail.as_ref());
        objects.push(access_object(
            transport,
            operation,
            header_extras,
            Value::Object(request),
            status,
            detail,
        ));
    }

    objects
}

fn access_object(
    transport: &str,
    operation: &str,
    header_extras: &[(String, String)],
    request: Value,
    status: u16,
    detail: Option<&Value>,
) -> Value {
    let response = AccessResponse {
        status,
        detail: detail.cloned(),
    };

    let mut access = Map::new();
    access.insert("transport".to_string(), json!(transport));
    access.insert("operation".to_string(), json!(operation));
    for (field, value) in header_extras {
        access.insert(field.clone(), json!(value));
    }
    // A call-level record (rejected before a handler) has an empty request; omit
    // the `request` field entirely rather than emitting an empty object.
    let has_request = request.as_object().is_none_or(|map| !map.is_empty());
    if has_request {
        access.insert("request".to_string(), request);
    }
    access.insert(
        "response".to_string(),
        serde_json::to_value(response).unwrap_or(Value::Null),
    );
    Value::Object(access)
}

tokio::task_local! {
    /// Request-scoped access-log collector, installed by the REST middleware and
    /// the gRPC Tower layer around the handler and read back to emit once the
    /// response is known. A [`RefCell`] (never `Arc`/`Mutex`) is enough because
    /// the scope is per task and the handler and the emit step run sequentially,
    /// never concurrently.
    ///
    /// Limitation: a task-local is bound to the request task only; it is not
    /// inherited by tasks the handler starts with `tokio::spawn`/`spawn_blocking`.
    /// Inside such a task the [`AccessLog`] methods silently no-op (their records
    /// are dropped), and the emit already ran on the request task. Concurrency
    /// that stays on the request task (`join!`, `select!`, buffered streams) is
    /// fine and shares the one buffer. To log work fanned out to spawned tasks,
    /// return the data from those tasks to the request task and record it there,
    /// rather than calling [`AccessLog`] from inside the spawned task.
    static ACCESS: RefCell<AccessLogState>;
}

/// Request-scoped buffer of access records plus the metadata needed to emit them.
/// The same shape backs REST and gRPC; the transport picks the matching emit path
/// ([`emit_rest`] or [`emit_grpc`]).
pub(crate) struct AccessLogState {
    operation: String,
    header_extras: Vec<(String, String)>,
    config: AccessLogConfig,
    records: Vec<AccessRecord>,
    call_detail: Option<Value>,
    failure: Option<Value>,
    outcome: Option<(i32, Option<String>)>,
}

impl AccessLogState {
    pub(crate) fn new(
        operation: String,
        header_extras: Vec<(String, String)>,
        config: AccessLogConfig,
    ) -> Self {
        Self {
            operation,
            header_extras,
            config,
            records: Vec::new(),
            call_detail: None,
            failure: None,
            outcome: None,
        }
    }
}

/// Zero-sized handle handlers use to record access-log data. Every method
/// delegates to the request-scoped [`ACCESS`] state and is a no-op outside a
/// scope (access logging disabled), so handlers never branch on configuration
/// beyond the [`AccessLog::is_enabled`] guard for expensive unwinds.
#[derive(Clone, Copy, Debug)]
pub struct AccessLog;

impl AccessLog {
    /// Whether access logging is active for this request. Handlers guard
    /// expensive unwind construction with this so no work is done when disabled.
    pub fn is_enabled(&self) -> bool {
        with_access(|_| ()).is_some()
    }

    /// Records a single access line with the given input `params`.
    pub fn params(&self, params: Value) {
        with_access(|state| state.records.push(AccessRecord::new(params)));
    }

    /// Records one access line per element (batch unwind). The iterator is not
    /// consumed when access logging is disabled.
    pub fn items<I>(&self, params: I)
    where
        I: IntoIterator<Item = Value>,
    {
        with_access(|state| {
            state
                .records
                .extend(params.into_iter().map(AccessRecord::new))
        });
    }

    /// Records the success `detail` for a single-line call (status comes from the
    /// response for REST, or the terminal gRPC status).
    pub fn detail(&self, detail: Value) {
        with_access(|state| state.call_detail = Some(detail));
    }

    /// Attaches per-item success details to the recorded lines in order. The
    /// iterator is not consumed when access logging is disabled.
    pub fn zip_details<I>(&self, details: I)
    where
        I: IntoIterator<Item = Value>,
    {
        with_access(|state| {
            for (record, detail) in state.records.iter_mut().zip(details) {
                record.detail = Some(detail);
            }
        });
    }

    /// Records a REST whole-call failure `detail.message` and returns the
    /// `(status, body)` tuple to return from the handler. Use where the client
    /// body equals the logged message.
    pub fn fail(&self, status: StatusCode, message: impl Into<String>) -> (StatusCode, String) {
        let message = message.into();
        with_access(|state| state.failure = Some(message_detail(message.clone())));
        (status, message)
    }

    /// Records a REST whole-call failure from an [`AuthorizationErr`], logging the
    /// verbose internal `message()` while returning the canonical client body (for
    /// example `"Permission denied"` on 403).
    pub fn fail_err(&self, err: &AuthorizationErr) -> (StatusCode, String) {
        with_access(|state| state.failure = Some(message_detail(err.message())));
        err.to_status()
    }

    /// Records a failed gRPC outcome from a [`Status`] and returns it unchanged so
    /// handlers can `return Err(access.fail_grpc(status))`.
    pub fn fail_grpc(&self, status: Status) -> Status {
        with_access(|state| {
            state.outcome = Some((i32::from(status.code()), Some(status.message().to_string())));
        });
        status
    }

    /// Records a whole-call failure `detail.message` observed at the transport
    /// (for example an error body produced before or without a handler), unless a
    /// handler already recorded a failure. Used by the REST middleware.
    pub fn record_failure(&self, message: impl Into<String>) {
        with_access(|state| {
            if state.failure.is_none() {
                state.failure = Some(message_detail(message.into()));
            }
        });
    }

    /// Records the terminal gRPC outcome observed from the response headers, unless
    /// a handler already recorded one. Used by the gRPC layer so calls rejected
    /// before a handler (unauthenticated, unimplemented, decode errors) still carry
    /// their status. An empty `message` is dropped.
    pub fn record_grpc_outcome(&self, code: i32, message: Option<String>) {
        with_access(|state| {
            if state.outcome.is_none() {
                let message = message.filter(|value| !value.is_empty());
                state.outcome = Some((code, message));
            }
        });
    }
}

/// Applies `apply` to the request-scoped state, returning `None` when access
/// logging is disabled (no scope installed) so callers become no-ops.
fn with_access<R>(apply: impl FnOnce(&mut AccessLogState) -> R) -> Option<R> {
    ACCESS.try_with(|cell| apply(&mut cell.borrow_mut())).ok()
}

/// Runs `future` inside a request-scoped access-log collector. Installed only when
/// access logging is enabled; otherwise the transport skips the scope and the
/// [`AccessLog`] handle methods are no-ops.
pub(crate) async fn scoped<F: Future>(state: AccessLogState, future: F) -> F::Output {
    ACCESS.scope(RefCell::new(state), future).await
}

/// Emits the collected REST access lines with the resolved HTTP status. Called by
/// the middleware inside the scope after the handler returns.
pub(crate) fn emit_rest(status: u16) {
    with_access(|state| {
        apply_call_detail(state);
        emit_access(
            &state.config,
            TRANSPORT_REST,
            &state.operation,
            &state.header_extras,
            &state.records,
            status,
            state.failure.as_ref(),
        );
    });
}

/// Emits the collected gRPC access lines using the recorded outcome (defaulting to
/// `OK`), injecting `detail.code`. Called by the Tower layer inside the scope after
/// the RPC returns.
pub(crate) fn emit_grpc() {
    with_access(|state| {
        let (code, message) = state.outcome.clone().unwrap_or((0, None));
        let code_name = grpc_code_name(code);

        let failure = if code == 0 {
            apply_call_detail(state);
            for record in state.records.iter_mut() {
                let mut detail = match record.detail.take() {
                    Some(Value::Object(map)) => map,
                    _ => Map::new(),
                };
                detail.insert("code".to_string(), json!(code_name));
                record.detail = Some(Value::Object(detail));
            }
            None
        } else {
            let mut detail = match message {
                Some(message) if !message.is_empty() => match message_detail(message) {
                    Value::Object(map) => map,
                    _ => Map::new(),
                },
                _ => Map::new(),
            };
            detail.insert("code".to_string(), json!(code_name));
            Some(Value::Object(detail))
        };

        emit_access(
            &state.config,
            TRANSPORT_GRPC,
            &state.operation,
            &state.header_extras,
            &state.records,
            code as u16,
            failure.as_ref(),
        );
    });
}

/// Moves a single-line success `detail` onto the sole record when the handler used
/// [`AccessLog::detail`] instead of per-item details.
fn apply_call_detail(state: &mut AccessLogState) {
    if state.failure.is_some() {
        return;
    }
    if let Some(detail) = state.call_detail.take() {
        if state.records.len() == 1 && state.records[0].detail.is_none() {
            state.records[0].detail = Some(detail);
        }
    }
}

/// Maps a numeric gRPC status code to its canonical status name for
/// `response.detail.code`.
fn grpc_code_name(code: i32) -> &'static str {
    match code {
        0 => "OK",
        1 => "CANCELLED",
        2 => "UNKNOWN",
        3 => "INVALID_ARGUMENT",
        4 => "DEADLINE_EXCEEDED",
        5 => "NOT_FOUND",
        6 => "ALREADY_EXISTS",
        7 => "PERMISSION_DENIED",
        8 => "RESOURCE_EXHAUSTED",
        9 => "FAILED_PRECONDITION",
        10 => "ABORTED",
        11 => "OUT_OF_RANGE",
        12 => "UNIMPLEMENTED",
        13 => "INTERNAL",
        14 => "UNAVAILABLE",
        15 => "DATA_LOSS",
        16 => "UNAUTHENTICATED",
        _ => "UNKNOWN",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> AccessLogConfig {
        AccessLogConfig::new(true, Vec::new())
    }

    #[test]
    fn disabled_emits_nothing() {
        let mut cfg = config();
        cfg.enabled = false;
        let records = [AccessRecord::new(json!({"id": "1"}))];
        assert!(
            build_access_objects(&cfg, TRANSPORT_REST, "GET /x", &[], &records, 200, None)
                .is_empty()
        );
    }

    #[test]
    fn empty_records_emit_call_level() {
        let objects = build_access_objects(
            &config(),
            TRANSPORT_REST,
            "POST /v1beta/policies/",
            &[],
            &[],
            401,
            Some(&message_detail("Unauthorized")),
        );
        assert_eq!(objects.len(), 1);
        assert_eq!(objects[0]["operation"], json!("POST /v1beta/policies/"));
        assert!(objects[0].get("request").is_none());
        assert_eq!(objects[0]["response"]["status"], json!(401));
        assert_eq!(
            objects[0]["response"]["detail"],
            json!({"message": "Unauthorized"})
        );
    }

    #[test]
    fn empty_records_without_failure_omit_request_and_detail() {
        let objects = build_access_objects(
            &config(),
            TRANSPORT_REST,
            "GET /unknown",
            &[],
            &[],
            404,
            None,
        );
        assert_eq!(objects.len(), 1);
        assert!(objects[0].get("request").is_none());
        assert_eq!(objects[0]["response"]["status"], json!(404));
        assert!(objects[0]["response"].get("detail").is_none());
    }

    #[test]
    fn single_line_has_no_index() {
        let records = [AccessRecord {
            params: json!({"id": "1"}),
            detail: Some(json!({"item_count": 3})),
        }];
        let objects = build_access_objects(
            &config(),
            TRANSPORT_REST,
            "GET /x",
            &[],
            &records,
            200,
            None,
        );
        assert_eq!(objects.len(), 1);
        assert_eq!(objects[0]["transport"], json!("REST"));
        let request = &objects[0]["request"];
        assert!(request.get("index").is_none());
        assert_eq!(request["params"], json!({"id": "1"}));
        assert_eq!(objects[0]["response"]["status"], json!(200));
        assert_eq!(objects[0]["response"]["detail"], json!({"item_count": 3}));
    }

    #[test]
    fn success_without_detail_omits_detail_key() {
        let records = [AccessRecord::new(json!({}))];
        let objects = build_access_objects(
            &config(),
            TRANSPORT_REST,
            "GET /health",
            &[],
            &records,
            200,
            None,
        );
        assert!(objects[0]["response"].get("detail").is_none());
    }

    #[test]
    fn detail_is_always_an_object() {
        let detail = message_detail("Policy with ID '1' could not be found.");
        assert!(detail.is_object());
        assert_eq!(
            detail["message"],
            json!("Policy with ID '1' could not be found.")
        );
    }

    #[test]
    fn multiple_lines_carry_index() {
        let records: Vec<AccessRecord> =
            (0..3).map(|i| AccessRecord::new(json!({"n": i}))).collect();
        let objects = build_access_objects(
            &config(),
            TRANSPORT_REST,
            "POST /batch",
            &[],
            &records,
            200,
            None,
        );
        assert_eq!(objects.len(), 3);
        for (i, object) in objects.iter().enumerate() {
            assert_eq!(object["request"]["index"], json!(i));
        }
    }

    #[test]
    fn failure_overrides_every_line() {
        let records: Vec<AccessRecord> = (0..2)
            .map(|i| AccessRecord {
                params: json!({"n": i}),
                detail: Some(json!({"decision": "allow"})),
            })
            .collect();
        let failure = message_detail("boom");
        let objects = build_access_objects(
            &config(),
            TRANSPORT_REST,
            "POST /batch",
            &[],
            &records,
            500,
            Some(&failure),
        );
        for object in &objects {
            assert_eq!(object["response"]["status"], json!(500));
            assert_eq!(object["response"]["detail"], json!({"message": "boom"}));
        }
    }

    #[test]
    fn header_copy_skips_reserved_and_credentials() {
        let cfg = AccessLogConfig::new(
            true,
            vec![
                ("ip".to_string(), "x-forwarded-for".to_string()),
                ("operation".to_string(), "x-op".to_string()),
                ("token".to_string(), "Authorization".to_string()),
                ("session".to_string(), "Cookie".to_string()),
                ("missing".to_string(), "x-missing".to_string()),
            ],
        );
        let extras = resolve_header_extras(&cfg, |header| match header {
            "x-forwarded-for" => Some("203.0.113.10".to_string()),
            "x-op" => Some("spoofed".to_string()),
            "authorization" => Some("Bearer secret".to_string()),
            "cookie" => Some("sid=abc".to_string()),
            _ => None,
        });
        assert_eq!(extras, vec![("ip".to_string(), "203.0.113.10".to_string())]);
    }

    #[test]
    fn header_extras_appear_on_lines() {
        let extras = vec![("ip".to_string(), "203.0.113.10".to_string())];
        let records = [AccessRecord::new(json!({}))];
        let objects = build_access_objects(
            &config(),
            TRANSPORT_REST,
            "GET /x",
            &extras,
            &records,
            200,
            None,
        );
        assert_eq!(objects[0]["ip"], json!("203.0.113.10"));
    }

    #[test]
    fn access_response_skips_absent_detail() {
        let response = AccessResponse {
            status: 204,
            detail: None,
        };
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value, json!({"status": 204}));
    }

    fn scope<R>(config: AccessLogConfig, body: impl FnOnce() -> R) -> R {
        let state = AccessLogState::new("GET /x".to_string(), Vec::new(), config);
        ACCESS.sync_scope(RefCell::new(state), body)
    }

    fn snapshot() -> Vec<Value> {
        with_access(|state| {
            apply_call_detail(state);
            build_access_objects(
                &state.config,
                TRANSPORT_REST,
                &state.operation,
                &state.header_extras,
                &state.records,
                200,
                state.failure.as_ref(),
            )
        })
        .unwrap_or_default()
    }

    #[test]
    fn handle_records_params_and_detail_in_scope() {
        let objects = scope(config(), || {
            AccessLog.params(json!({"id": "1"}));
            AccessLog.detail(json!({"item_count": 3}));
            assert!(AccessLog.is_enabled());
            snapshot()
        });
        assert_eq!(objects.len(), 1);
        assert_eq!(objects[0]["request"]["params"], json!({"id": "1"}));
        assert_eq!(objects[0]["response"]["detail"], json!({"item_count": 3}));
    }

    #[test]
    fn handle_is_noop_and_disabled_outside_scope() {
        assert!(!AccessLog.is_enabled());
        let mut iterated = false;
        AccessLog.items(std::iter::once_with(|| {
            iterated = true;
            json!({})
        }));
        assert!(!iterated);
    }

    #[test]
    fn fail_records_message_and_overrides_lines() {
        let objects = scope(config(), || {
            AccessLog.params(json!({"id": "1"}));
            let (status, body) = AccessLog.fail(StatusCode::NOT_FOUND, "missing");
            assert_eq!(status, StatusCode::NOT_FOUND);
            assert_eq!(body, "missing");
            snapshot()
        });
        assert_eq!(
            objects[0]["response"]["detail"],
            json!({"message": "missing"})
        );
    }

    #[test]
    fn grpc_emit_injects_code_on_success() {
        let records = scope(config(), || {
            AccessLog.params(json!({"id": "1"}));
            AccessLog.detail(json!({"decision": "allow"}));
            emit_grpc();
            with_access(|state| state.records[0].detail.clone()).flatten()
        });
        assert_eq!(records, Some(json!({"decision": "allow", "code": "OK"})));
    }

    #[test]
    fn record_failure_sets_only_when_absent() {
        let objects = scope(config(), || {
            AccessLog.record_failure("boom");
            AccessLog.record_failure("later");
            snapshot()
        });
        assert_eq!(objects[0]["response"]["detail"], json!({"message": "boom"}));
        assert!(objects[0].get("request").is_none());
    }

    #[test]
    fn record_grpc_outcome_sets_only_when_absent_and_drops_empty_message() {
        scope(config(), || {
            AccessLog.record_grpc_outcome(16, Some("denied".to_string()));
            AccessLog.record_grpc_outcome(0, None);
            with_access(|state| assert_eq!(state.outcome, Some((16, Some("denied".to_string())))));
        });
        scope(config(), || {
            AccessLog.record_grpc_outcome(12, Some(String::new()));
            with_access(|state| assert_eq!(state.outcome, Some((12, None))));
        });
    }
}
