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

use super::{common, opt_into, proto, resource, span_context, vec_into};
use opentelemetry::trace::SpanContext;
pub use proto::logs::SeverityNumber;

#[derive(Debug, Clone, PartialEq)]
pub struct OutputLogs {
    pub resource_logs: Vec<ResourceLogs>,
}

impl From<Vec<proto::logs::ResourceLogs>> for OutputLogs {
    fn from(value: Vec<proto::logs::ResourceLogs>) -> Self {
        let resource_logs = vec_into!(value);
        Self { resource_logs }
    }
}

impl OutputLogs {
    #[track_caller]
    pub fn assert_timestamps(&mut self) {
        let timestamp = common::TimestampRange::now();
        for resource_log in &mut self.resource_logs {
            for scope_log in &mut resource_log.scope_logs {
                for log in &mut scope_log.log_records {
                    // time_unix_nano seems to be always zero
                    assert_eq!(
                        timestamp, log.observed_time_unix_nano,
                        "log observed time {} < {} < {}: {:#?}",
                        timestamp.start, log.observed_time_unix_nano, timestamp.end, log
                    );
                    log.observed_time_unix_nano = 0;
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResourceLogs {
    pub resource: Option<resource::Resource>,
    pub schema_url: String,
    pub scope_logs: Vec<ScopeLogs>,
}

impl From<proto::logs::ResourceLogs> for ResourceLogs {
    fn from(value: proto::logs::ResourceLogs) -> Self {
        let proto::logs::ResourceLogs {
            resource,
            scope_logs,
            schema_url,
        } = value;
        let resource = opt_into!(resource);
        let scope_logs = vec_into!(scope_logs);
        Self {
            resource,
            schema_url,
            scope_logs,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScopeLogs {
    pub scope: Option<common::InstrumentationScope>,
    pub schema_url: String,
    pub log_records: Vec<LogRecord>,
}

impl From<proto::logs::ScopeLogs> for ScopeLogs {
    fn from(value: proto::logs::ScopeLogs) -> Self {
        let proto::logs::ScopeLogs {
            scope,
            log_records,
            schema_url,
        } = value;
        let scope = opt_into!(scope);
        let log_records = vec_into!(log_records);
        Self {
            scope,
            schema_url,
            log_records,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LogRecord {
    pub severity_number: Result<SeverityNumber, i32>,
    pub severity_text: String,
    pub event_name: String,
    pub body: Option<common::Value>,
    pub attributes: Vec<common::KeyValue>,
    pub dropped_attributes_count: u32,
    pub context: SpanContext,
    pub time_unix_nano: u64,
    pub observed_time_unix_nano: u64,
}

impl LogRecord {
    pub fn new(
        severity_number: SeverityNumber,
        severity_text: &str,
        name: &str,
        body: &str,
        attrs: Vec<common::KeyValue>,
        context: SpanContext,
    ) -> Self {
        Self {
            severity_number: Ok(severity_number),
            severity_text: severity_text.to_string(),
            event_name: name.to_string(),
            body: Some(body.into()),
            attributes: attrs,
            dropped_attributes_count: 0,
            context,
            time_unix_nano: 0,
            observed_time_unix_nano: 0,
        }
    }
}

impl From<proto::logs::LogRecord> for LogRecord {
    fn from(value: proto::logs::LogRecord) -> Self {
        let proto::logs::LogRecord {
            time_unix_nano,
            observed_time_unix_nano,
            severity_number,
            severity_text,
            body,
            attributes,
            dropped_attributes_count,
            flags,
            trace_id,
            span_id,
            event_name,
        } = value;
        let severity_number = severity_number.try_into().map_err(|_e| severity_number);
        let body = body.and_then(|v| v.value).map(Into::into);
        let attributes = vec_into!(attributes);
        let context = span_context::make(&trace_id, &span_id, flags, "");
        Self {
            severity_number,
            severity_text,
            event_name,
            body,
            attributes,
            dropped_attributes_count,
            context,
            time_unix_nano,
            observed_time_unix_nano,
        }
    }
}
