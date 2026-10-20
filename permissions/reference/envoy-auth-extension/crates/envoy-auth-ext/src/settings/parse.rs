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

use super::AppSettings;

pub fn parse_config(
    config: Option<&str>,
    generate: Option<&str>,
) -> Result<AppSettings, Box<dyn std::error::Error + Send + Sync>> {
    let mut builder = config::Config::builder();

    // first, defaults
    let default_config = serde_json::to_string(&AppSettings::default()).unwrap();
    builder = builder.add_source(config::File::from_str(
        &default_config,
        config::FileFormat::Json,
    ));

    // second, config file if provided
    if let Some(path) = config {
        builder = builder.add_source(config::File::with_name(path));
    }

    let settings = builder.build()?.try_deserialize()?;

    if let Some(path) = generate {
        let extension = std::path::Path::new(path)
            .extension()
            .and_then(|ext| ext.to_str());
        let contents = match extension {
            Some("toml") => toml::to_string(&settings)?,
            Some("json") => serde_json::to_string_pretty(&settings)?,
            other => {
                return Err(format!(
                    "unknown extension `{}` for generate path `{}`",
                    other.unwrap_or("<none>"),
                    path
                )
                .into());
            }
        };
        std::fs::write(path, contents)?;
    }

    Ok(settings)
}
