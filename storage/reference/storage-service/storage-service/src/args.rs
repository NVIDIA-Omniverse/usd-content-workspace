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

use std::path::PathBuf;

use clap::Parser;
use figment::{
    providers::{Format, Json, Toml},
    Figment,
};
use tracing::info;

use crate::{common::figment::Interpolate, config::Config, error::ConfigError};

#[derive(Parser, Debug)]
pub struct Args {
    /// Path to the service configuration file.
    #[arg(long, env = "STORAGE_SERVICE_CONFIG_FILE")]
    pub config_file: Option<PathBuf>,

    /// Service configuration, serialized in the json format.
    #[arg(long, env = "STORAGE_SERVICE_CONFIG_JSON")]
    pub config_json: Option<String>,

    /// Service configuration, serialized in the toml format.
    #[arg(long, env = "STORAGE_SERVICE_CONFIG_TOML")]
    pub config_toml: Option<String>,

    /// Initialize all components - including the metadata backends, which
    /// apply their database migrations - and exit without serving. Intended
    /// for an init container, so service replicas start only after the
    /// initialization work is done.
    #[arg(long, env = "STORAGE_SERVICE_BOOTSTRAP_ONLY")]
    pub bootstrap_only: bool,
}

impl Args {
    /// Load service configuration.
    pub fn load_config(&self) -> Result<Config, ConfigError> {
        let figment = if let Some(path) = self.config_file.as_ref() {
            info!(?path, "loading service configuration from file");

            Figment::new().merge(Interpolate::new(Toml::file(path)))
        } else if let Some(config) = self.config_toml.as_ref() {
            info!("loading service configuration from inline toml");

            Figment::new().merge(Interpolate::new(Toml::string(config)))
        } else if let Some(config) = self.config_json.as_ref() {
            info!("loading service configuration from inline json");

            Figment::new().merge(Interpolate::new(Json::string(config)))
        } else {
            return Err(ConfigError::new("service configuration is missing"));
        };

        figment
            .extract()
            .map_err(|error| ConfigError::new_with_source("failed to parse service config", error))
    }
}
