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

//! Types returned by the test collectors.
//!
//! These roughly correspond to the Otel Protobuf definitions.
#![allow(missing_docs)]
pub mod common;
pub mod logs;
pub mod metrics;
pub mod resource;
pub(crate) mod span_context;
pub mod trace;

pub(crate) mod proto {
    pub use opentelemetry_proto::tonic::common::v1 as common;
    pub use opentelemetry_proto::tonic::logs::v1 as logs;
    pub use opentelemetry_proto::tonic::metrics::v1 as metrics;
    pub use opentelemetry_proto::tonic::resource::v1 as resource;
    pub use opentelemetry_proto::tonic::trace::v1 as trace;
}

macro_rules! opt_into {
    ($v:ident) => {
        $v.map(Into::into)
    };
}
pub(crate) use opt_into;

macro_rules! vec_into {
    ($v:ident) => {
        $v.into_iter().map(Into::into).collect()
    };
}
pub(crate) use vec_into;

#[doc(hidden)]
#[derive(Debug)]
pub enum Any {
    Logs(proto::logs::ResourceLogs),
    Metrics(proto::metrics::ResourceMetrics),
    Spans(proto::trace::ResourceSpans),
}
