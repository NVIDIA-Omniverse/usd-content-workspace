// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use super::error::Error;
use clap::Args as ClapArgs;
use figment::{
    providers::{Env, Format, Json, Toml, Yaml},
    Figment,
};

#[derive(ClapArgs, Debug, Clone)]
pub struct ConfigArgs {
    #[arg(short, long, env = "CONFIGURATION_FILE")]
    pub configuration_file: Option<std::path::PathBuf>,

    #[arg(short, long, default_value = "STORAGE_SERVICE_", env = "ENV_PREFIX")]
    pub env_prefix: String,
}

pub fn generate_figment(args: &ConfigArgs) -> Result<Figment, Error> {
    let mut figment = Figment::new();

    // if specified attempt to load the configuration file
    if let Some(configuration_file) = &args.configuration_file {
        match configuration_file.extension() {
            Some(ext) if ext.eq_ignore_ascii_case("json") => {
                figment = figment.merge(Json::file(configuration_file));
            },
            Some(ext) if ext.eq_ignore_ascii_case("toml") => {
                figment = figment.merge(Toml::file(configuration_file));
            },
            Some(ext) if ext.eq_ignore_ascii_case("yaml") || ext.eq_ignore_ascii_case("yml") => {
                figment = figment.merge(Yaml::file(configuration_file));
            },
            _ => {},
        }
    }

    // merge in env var overrides
    figment = figment.merge(Env::prefixed(&args.env_prefix).split("__"));

    Ok(figment)
}
