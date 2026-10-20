// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

mod config_validation;
pub mod error_severity;
pub use config_validation::{validate_observability_config, ConfigValidationError};

mod crash;
pub use crash::install_panic_handler;

pub mod flow_control;

mod init;
pub use init::{
    init_observability, is_tracing_enabled, set_metric_prefix, try_init_observability, Guard, MetricPrefixError,
};

pub mod slow_operation;

pub mod trace_limited;

mod trace_limited_macros;

pub mod metrics;

pub mod telemetry_middleware;

mod logger;
