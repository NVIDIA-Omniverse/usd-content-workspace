// SPDX-FileCopyrightText: Copyright (c) 2024-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
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

use crate::common::extract::resp_grpc_status_code_to_name;
use crate::rpc_map::RpcMap;
use opentelemetry::{KeyValue, StringValue};
use tonic::codegen::http;

const GRPC_METHOD_ATTR: &str = "grpc.method";
const GRPC_STATUS_ATTR: &str = "grpc.status";

/// Returns the matched method or a value derived from `uri`.
///
/// See [`attr_method_safe`] to limit the cardinality of the returned method
/// value.
pub(crate) fn attr_method_raw(rpc_map: &RpcMap, uri: &http::Uri) -> KeyValue {
    let path = uri.path();
    let rpc = path.strip_prefix('/').unwrap_or(path);
    let value = rpc_map
        .find(rpc)
        .unwrap_or_else(|| StringValue::from(rpc.to_string()));
    KeyValue::new(GRPC_METHOD_ATTR, value)
}

/// For a non-empty RPC map, returns the matched method or "other", to prevent
/// high cardinality situations.
///
/// For an empty RPC map, the method is derived from the `uri`.
pub(crate) fn attr_method_safe(rpc_map: &RpcMap, uri: &http::Uri) -> KeyValue {
    let path = uri.path();
    let rpc = path.strip_prefix('/').unwrap_or(path);
    let value = match rpc_map.find(rpc) {
        Some(v) => v,
        None if rpc_map.is_empty() => {
            // "bad" fallback
            StringValue::from(rpc.to_string())
        }
        None => {
            // safe fallback
            StringValue::from("other")
        }
    };
    KeyValue::new(GRPC_METHOD_ATTR, value)
}

pub(crate) fn attr_status_code(code: Option<u16>) -> KeyValue {
    let code = code.unwrap_or(tonic::Code::Unknown as _);
    let value = resp_grpc_status_code_to_name(code)
        .map(StringValue::from)
        .unwrap_or_else(|| {
            // format unknown codes as an integer, to be forwards-compatible
            StringValue::from(format!("{code}"))
        });
    KeyValue::new(GRPC_STATUS_ATTR, value)
}

/// Target sentinel value.
///
/// When the target cannot be determined, there are two choices. The attribute
/// could be omitted completely. This is undesirable for metrics, since when
/// using e.g. Prometheus, the label set for a time-series would be inconsistent
/// (e.g.`sum by (grpc_target)` wouldn't work).
///
/// An empty string is rough. There's [gRPC proposal
/// A109](https://github.com/grpc/proposal/pull/528) "Target Attribute Filter
/// for OpenTelemetry Metrics". Although it hasn't been formally adopted, it has
/// been incorporated into at least the [gRPC Java
/// implementation](https://github.com/grpc/grpc-java/commit/c5f5ee0).
///
/// > The string `"other"` is chosen as a stable, low-cardinality placeholder
/// > value to represent all filtered targets.
///
/// (In this case, we're using it to signify no information, which is slightly
/// different to filtering.)
pub(crate) const TARGET_OTHER: &str = "other";

pub(crate) fn target_fallback(parts: &http::request::Parts) -> Option<StringValue> {
    let uri = &parts.uri;
    // the `grpc.target` attribute is a pass-through for the channel target,
    // which we may not have. the A66 spec is light on specifying what exactly a
    // target is; for clarity i looked at the "gRPC Name Resolution"
    // documentation
    // (https://github.com/grpc/grpc/blob/c0728bdae796f214a9fc56a617b21075c2ed3b40/doc/naming.md)
    // and the Go gRPC implementation
    // (https://github.com/grpc/grpc-go/blob/v1.79.3/clientconn.go#L157-L178).
    //
    // we try and construct a target from the request URI. note that if a client
    // is constructed without an explicit origin, the scheme and authority will
    // be empty. tonic only overrides the path per RPC. conversely, that means
    // the path is useless for the target. so, in all cases the target is
    // constructed without a trailing slash since only the scheme and authority
    // are relevant.
    let authority = uri.authority().map(http::uri::Authority::as_str);
    let scheme = uri.scheme_str();
    match scheme {
        Some(scheme) => {
            // it's currently impossible to create a `http::Uri` with a scheme
            // but no authority. for a general URI, an empty authority is valid.
            // but for a HTTP/HTTPS URI it isn't. see RFC 9110 "HTTP Semantics":
            //
            // https://www.rfc-editor.org/rfc/rfc9110#section-4.2.1
            // > A sender MUST NOT generate an "https" URI with an empty host
            // > identifier.
            //
            // however, if it were possible, `scheme://` is still meaningful,
            // and the target does *not* have to be a valid HTTP URI, only a
            // valid URI.
            let authority = authority.unwrap_or_default();
            Some(StringValue::from(format!("{scheme}://{authority}")))
        }
        None => {
            // see reasoning above. interestingly, a `http::Uri` can be created
            // with an authority but not a scheme. additionally, the Go gRPC
            // implementation lists "foo.googleapis.com:8080" as an example
            // target. therefore, i think a bare authority is still meaningful
            // and not in violation of the A66 spec.
            authority.map(|authority| StringValue::from(authority.to_string()))
        }
    }
}

#[cfg(test)]
mod tests;
