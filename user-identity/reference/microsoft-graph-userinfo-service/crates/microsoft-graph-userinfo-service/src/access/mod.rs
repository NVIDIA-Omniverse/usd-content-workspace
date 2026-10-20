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

use serde_json::{Map, Value};

/// API access logger. Emits one structured record per business REST/gRPC call
/// when enabled, following the access-log record contract. Disabled by default;
/// every emit method is a no-op until enabled.
pub struct AccessLog {
    enabled: bool,
    headers: Vec<(String, String)>,
}

impl AccessLog {
    /// `headers` maps access-record field name to a lowercased request header
    /// name. Credential and reserved-field entries are expected to have already
    /// been dropped by configuration parsing.
    pub fn new(enabled: bool, headers: Vec<(String, String)>) -> Self {
        Self { enabled, headers }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Emits a record for a REST call. Skipped when disabled. A combined record
    /// carries `request.params` (`params` = `Some`); a call-level record for a
    /// request rejected before a handler omits `request` entirely
    /// (`params` = `None`).
    pub fn emit_rest(
        &self,
        header_fields: Vec<(String, String)>,
        operation: String,
        params: Option<Map<String, Value>>,
        status: u16,
        detail: Map<String, Value>,
    ) {
        if !self.enabled {
            return;
        }
        self.emit(build_record(
            Transport::Rest,
            header_fields,
            operation,
            params,
            i64::from(status),
            detail,
        ));
    }

    /// Emits a combined record for a unary gRPC call. Skipped when disabled.
    /// `item_count` is added to the success detail for list RPCs.
    pub fn emit_grpc<T>(
        &self,
        header_fields: Vec<(String, String)>,
        operation: String,
        params: Map<String, Value>,
        item_count: Option<usize>,
        result: &Result<tonic::Response<T>, tonic::Status>,
    ) {
        if !self.enabled {
            return;
        }
        let code = match result {
            Ok(_) => tonic::Code::Ok,
            Err(status) => status.code(),
        };

        let mut detail = Map::new();
        detail.insert("code".to_owned(), Value::from(code_name(code)));
        if let (Ok(_), Some(count)) = (result, item_count) {
            detail.insert("item_count".to_owned(), Value::from(count));
        }
        if let Err(status) = result {
            let message = status.message();
            if !message.is_empty() {
                detail.insert("message".to_owned(), Value::from(message));
            }
        }

        self.emit(build_record(
            Transport::Grpc,
            header_fields,
            operation,
            Some(params),
            i64::from(code as i32),
            detail,
        ));
    }

    /// Emits a call-level record for a gRPC request rejected before a handler
    /// runs (an unknown method, or a request whose payload failed to decode).
    /// `request` is omitted since no input was parsed into operation fields.
    pub fn emit_grpc_call_level(
        &self,
        header_fields: Vec<(String, String)>,
        operation: String,
        code: tonic::Code,
        message: &str,
    ) {
        if !self.enabled {
            return;
        }
        let mut detail = Map::new();
        detail.insert("code".to_owned(), Value::from(code_name(code)));
        if !message.is_empty() {
            detail.insert("message".to_owned(), Value::from(message));
        }
        self.emit(build_record(
            Transport::Grpc,
            header_fields,
            operation,
            None,
            i64::from(code as i32),
            detail,
        ));
    }

    /// Selects the configured header fields from incoming REST headers.
    pub fn header_fields_http(&self, headers: &http::HeaderMap) -> Vec<(String, String)> {
        self.headers
            .iter()
            .filter_map(|(field, header)| {
                header_value(headers.get(header)?.to_str().ok()).map(|value| (field.clone(), value))
            })
            .collect()
    }

    /// Selects the configured header fields from incoming gRPC metadata.
    pub fn header_fields_grpc(
        &self,
        metadata: &tonic::metadata::MetadataMap,
    ) -> Vec<(String, String)> {
        self.headers
            .iter()
            .filter_map(|(field, header)| {
                let value = metadata.get(header.as_str()).and_then(|v| v.to_str().ok());
                header_value(value).map(|value| (field.clone(), value))
            })
            .collect()
    }

    /// Emits one access record as a `tracing` event on the `access` target. The
    /// record is carried in the `access` field and rendered by the access-log
    /// formatter (`crate::telemetry`); the enclosing envelope carries
    /// `type: "access"`.
    fn emit(&self, access: Value) {
        tracing::info!(target: "access", r#type = "access", access = %access);
    }
}

fn header_value(raw: Option<&str>) -> Option<String> {
    let value = raw?.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_owned())
    }
}

#[derive(Clone, Copy)]
enum Transport {
    Rest,
    Grpc,
}

impl Transport {
    fn as_str(self) -> &'static str {
        match self {
            Self::Rest => "REST",
            Self::Grpc => "gRPC",
        }
    }
}

/// Builds the `access` object of a record. The result is the value logged under
/// the `access` field; the enclosing envelope carries `type: "access"`. When
/// `params` is `None` the record is call-level and `request` is omitted.
fn build_record(
    transport: Transport,
    header_fields: Vec<(String, String)>,
    operation: String,
    params: Option<Map<String, Value>>,
    status: i64,
    detail: Map<String, Value>,
) -> Value {
    let mut access = Map::new();
    access.insert("transport".to_owned(), Value::from(transport.as_str()));
    for (field, value) in header_fields {
        access.insert(field, Value::from(value));
    }
    access.insert("operation".to_owned(), Value::from(operation));

    if let Some(params) = params {
        let mut request = Map::new();
        request.insert("params".to_owned(), Value::Object(params));
        access.insert("request".to_owned(), Value::Object(request));
    }

    let mut response = Map::new();
    response.insert("status".to_owned(), Value::from(status));
    if !detail.is_empty() {
        response.insert("detail".to_owned(), Value::Object(detail));
    }
    access.insert("response".to_owned(), Value::Object(response));

    Value::Object(access)
}

fn code_name(code: tonic::Code) -> &'static str {
    use tonic::Code;
    match code {
        Code::Ok => "OK",
        Code::Cancelled => "CANCELLED",
        Code::Unknown => "UNKNOWN",
        Code::InvalidArgument => "INVALID_ARGUMENT",
        Code::DeadlineExceeded => "DEADLINE_EXCEEDED",
        Code::NotFound => "NOT_FOUND",
        Code::AlreadyExists => "ALREADY_EXISTS",
        Code::PermissionDenied => "PERMISSION_DENIED",
        Code::ResourceExhausted => "RESOURCE_EXHAUSTED",
        Code::FailedPrecondition => "FAILED_PRECONDITION",
        Code::Aborted => "ABORTED",
        Code::OutOfRange => "OUT_OF_RANGE",
        Code::Unimplemented => "UNIMPLEMENTED",
        Code::Internal => "INTERNAL",
        Code::Unavailable => "UNAVAILABLE",
        Code::DataLoss => "DATA_LOSS",
        Code::Unauthenticated => "UNAUTHENTICATED",
    }
}

/// Whether a matched REST route template is a business operation whose record
/// carries `request.params`. Business operations live under `/api/`.
pub fn is_business_route(route_template: &str) -> bool {
    route_template.starts_with("/api/")
}

/// Whether a matched REST route is an infrastructure endpoint that must never
/// be logged, even on failure (health and readiness probes).
pub fn is_infra_route(route_template: &str) -> bool {
    matches!(route_template, "/healthz" | "/readyz")
}

/// Extracts `request.params` for a REST call from the matched route template
/// (path parameters) and the request URI (whitelisted query parameters).
pub fn rest_params(route_template: &str, uri: &http::Uri) -> Map<String, Value> {
    let mut params = Map::new();
    for (key, value) in path_params(route_template, uri.path()) {
        params.insert(key, Value::from(value));
    }
    for (key, value) in query_params(uri.query()) {
        params.entry(key).or_insert(Value::from(value));
    }
    params
}

fn path_params(route_template: &str, uri_path: &str) -> Vec<(String, String)> {
    route_template
        .split('/')
        .zip(uri_path.split('/'))
        .filter_map(|(template, actual)| {
            let placeholder = template.strip_prefix('{')?.strip_suffix('}')?;
            let value = actual.trim();
            if value.is_empty() {
                return None;
            }
            Some((path_param_key(placeholder).to_owned(), value.to_owned()))
        })
        .collect()
}

fn path_param_key(placeholder: &str) -> &'static str {
    match placeholder {
        "userId" => "user_id",
        "groupId" => "group_id",
        _ => "id",
    }
}

fn query_params(query: Option<&str>) -> Vec<(String, String)> {
    let Some(query) = query else {
        return Vec::new();
    };
    query
        .split('&')
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            if !REST_QUERY_PARAMS.contains(&key) {
                return None;
            }
            let value = value.trim();
            if value.is_empty() {
                return None;
            }
            Some((key.to_owned(), value.to_owned()))
        })
        .collect()
}

const REST_QUERY_PARAMS: [&str; 4] = ["email", "display_name", "limit", "cursor"];

#[cfg(test)]
mod tests {
    use super::*;

    fn logger() -> AccessLog {
        AccessLog::new(
            true,
            vec![
                ("ip".to_owned(), "x-forwarded-for".to_owned()),
                ("ua".to_owned(), "user-agent".to_owned()),
            ],
        )
    }

    #[test]
    fn is_business_route_excludes_infrastructure() {
        assert!(is_business_route("/api/v1alpha/users"));
        assert!(is_business_route("/api/v1alpha/users/{userId}"));
        assert!(!is_business_route("/healthz"));
        assert!(!is_business_route("/readyz"));
    }

    #[test]
    fn rest_params_extracts_path_and_query() {
        let uri: http::Uri = "/api/v1alpha/groups/g-1/users?email=a@b.com&limit=5&secret=x"
            .parse()
            .unwrap();
        let params = rest_params("/api/v1alpha/groups/{groupId}/users", &uri);
        assert_eq!(params.get("group_id"), Some(&Value::from("g-1")));
        assert_eq!(params.get("email"), Some(&Value::from("a@b.com")));
        assert_eq!(params.get("limit"), Some(&Value::from("5")));
        assert_eq!(params.get("secret"), None);
    }

    #[test]
    fn rest_params_for_users_me_is_empty() {
        let uri: http::Uri = "/api/v1alpha/users/me".parse().unwrap();
        assert!(rest_params("/api/v1alpha/users/me", &uri).is_empty());
    }

    #[test]
    fn header_fields_http_skips_missing_and_empty() {
        let mut headers = http::HeaderMap::new();
        headers.insert("x-forwarded-for", "203.0.113.10".parse().unwrap());
        headers.insert("user-agent", "".parse().unwrap());
        let fields = logger().header_fields_http(&headers);
        assert_eq!(fields, vec![("ip".to_owned(), "203.0.113.10".to_owned())]);
    }

    #[test]
    fn build_record_get_has_no_detail() {
        let mut params = Map::new();
        params.insert("user_id".to_owned(), Value::from("u-1"));
        let record = build_record(
            Transport::Rest,
            vec![("ip".to_owned(), "203.0.113.10".to_owned())],
            "GET /api/v1alpha/users/{userId}".to_owned(),
            Some(params),
            200,
            Map::new(),
        );
        assert_eq!(record["transport"], Value::from("REST"));
        assert_eq!(record["ip"], Value::from("203.0.113.10"));
        assert_eq!(record["request"]["params"]["user_id"], Value::from("u-1"));
        assert_eq!(record["response"]["status"], Value::from(200));
        assert!(record["response"].get("detail").is_none());
    }

    #[test]
    fn build_record_list_carries_item_count() {
        let mut detail = Map::new();
        detail.insert("item_count".to_owned(), Value::from(3));
        let record = build_record(
            Transport::Grpc,
            Vec::new(),
            "/nvidia.omniverse.userinfo.v1alpha.UserInfoService/ListUsers".to_owned(),
            Some(Map::new()),
            0,
            detail,
        );
        assert_eq!(record["transport"], Value::from("gRPC"));
        assert_eq!(record["response"]["status"], Value::from(0));
        assert_eq!(record["response"]["detail"]["item_count"], Value::from(3));
    }

    #[test]
    fn build_record_call_level_omits_request() {
        let mut detail = Map::new();
        detail.insert(
            "message".to_owned(),
            Value::from("missing or invalid authorization"),
        );
        let record = build_record(
            Transport::Rest,
            Vec::new(),
            "GET /api/v1alpha/users".to_owned(),
            None,
            401,
            detail,
        );
        assert_eq!(record["transport"], Value::from("REST"));
        assert!(record.get("request").is_none());
        assert_eq!(record["response"]["status"], Value::from(401));
        assert_eq!(
            record["response"]["detail"]["message"],
            Value::from("missing or invalid authorization")
        );
    }

    #[test]
    fn is_infra_route_matches_probes_only() {
        assert!(is_infra_route("/healthz"));
        assert!(is_infra_route("/readyz"));
        assert!(!is_infra_route("/api/v1alpha/users"));
        assert!(!is_infra_route("/nope"));
    }

    #[test]
    fn code_name_matches_status_names() {
        assert_eq!(code_name(tonic::Code::Ok), "OK");
        assert_eq!(code_name(tonic::Code::NotFound), "NOT_FOUND");
        assert_eq!(
            code_name(tonic::Code::PermissionDenied),
            "PERMISSION_DENIED"
        );
    }
}
