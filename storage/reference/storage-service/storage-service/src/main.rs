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

#![recursion_limit = "256"]

use std::{error::Error, process::ExitCode};

use clap::Parser;
use storage_service::{
    args::Args,
    common::error::PrettyPrintErrorExt,
    observability,
    service::{Error as ServiceError, Service},
};

#[tokio::main]
async fn main() -> ExitCode {
    let future = async {
        let _guard = observability::init_observability(env!("CARGO_BIN_NAME"), env!("CARGO_PKG_VERSION"))
            .await
            .map_err(|source| {
                ServiceError::new_with_source("failed to initialize application observability", source)
            })?;

        let args = Args::parse();
        let mut config = args
            .load_config()
            .map_err(|source| ServiceError::new_with_source("failed to load application configuration", source))?;

        if let Some(prefix) = config.metrics.prefix.take() {
            storage_core_util::observability::set_metric_prefix(prefix).map_err(|source| {
                ServiceError::new_with_source("failed to configure the application metric prefix", source)
            })?;
        }

        Service::new(config)
            .bootstrap_only(args.bootstrap_only)
            .run()
            .await
            .map_err(|source| ServiceError::new_with_source("storage service execution failed", source))?;

        Ok::<(), Box<dyn Error>>(())
    };

    if let Err(error) = future.await {
        eprint!("{}", error.pretty());
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}
