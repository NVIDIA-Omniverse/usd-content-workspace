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

#![allow(missing_docs)]
mod capture;
mod de;
mod parse;

pub use capture::TraceCapture;
pub use parse::LogParseError;

use opentelemetry::{SpanId, TraceId};
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct LogLine<'a> {
    pub message: &'a str,
    pub level: LogLevel,
    #[serde(with = "de::time_unix_nano")]
    pub timestamp: u64,
    pub target: &'a str,
    #[serde(default)]
    pub line: Option<u32>,
    #[serde(default)]
    pub file: Option<&'a str>,
    #[serde(with = "de::trace_id", default)]
    pub trace_id: Option<TraceId>,
    #[serde(with = "de::span_id", default)]
    pub span_id: Option<SpanId>,
    #[serde(flatten)]
    pub attrs: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Logs<'a> {
    pub lines: Vec<LogLine<'a>>,
}
