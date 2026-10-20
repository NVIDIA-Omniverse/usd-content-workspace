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

#[cfg(test)]
mod notifications;

#[cfg(test)]
mod proptest_tests;

#[cfg(test)]
mod storage_tests;

#[cfg(test)]
mod storage_metadata_tests;

#[cfg(test)]
mod storage_association_tests;

#[cfg(test)]
mod eligibility_tests;

#[cfg(test)]
mod thumbnail_sizes_tests;

#[cfg(test)]
mod utils_tests;

#[cfg(test)]
mod generation_tests;

#[cfg(test)]
pub(crate) fn parse_runtime_config_from_env_for_tests() -> crate::RuntimeConfigArgs {
    try_parse_runtime_config_for_tests(&[])
        .unwrap_or_else(|e| panic!("failed to parse test runtime config from env: {e}"))
}

/// Parse a test runtime config with `overrides` appended to the argument list.
///
/// For tests that need to prove a *rejected* value is rejected. Passing it on
/// the argument list rather than exporting it keeps the failure inside the one
/// test: these run concurrently in a single process, and a variable set to an
/// invalid value is visible to every other test that parses config — which is
/// 59 call sites. `#[serial]` does not help, since it orders serial tests
/// against each other and not against the rest of the suite.
#[cfg(test)]
pub(crate) fn try_parse_runtime_config_for_tests(
    overrides: &[&str],
) -> Result<crate::RuntimeConfigArgs, clap::Error> {
    #[derive(clap::Parser, Debug)]
    struct TestRuntimeCli {
        #[command(flatten)]
        runtime: crate::RuntimeConfigArgs,
    }

    let arg_or_default = |key: &str, default: &str| {
        crate::config::raw_env_value(key)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| default.to_string())
    };

    let mut args = vec![
        "thumbnail-common-test".to_string(),
        "--thumbnail-path-template".to_string(),
        arg_or_default(
            "THUMBNAIL_PATH_TEMPLATE",
            "{resource_path}/.thumbs/{width}x{height}/{resource_filename}.auto.{output_encoding}",
        ),
        "--thumbnail-sizes".to_string(),
        arg_or_default("THUMBNAIL_SIZES", "256x256"),
        "--thumbnail-encoding-format".to_string(),
        arg_or_default("THUMBNAIL_ENCODING_FORMAT", "PNG"),
        "--thumbnail-transform-type".to_string(),
        arg_or_default("THUMBNAIL_TRANSFORM_TYPE", "SQUARE"),
        "--thumbnail-crop-strategy".to_string(),
        arg_or_default("THUMBNAIL_CROP_STRATEGY", "SMART"),
        "--thumbnail-quality".to_string(),
        arg_or_default("THUMBNAIL_QUALITY", "85"),
        "--thumbnail-preserve-aspect-ratio".to_string(),
        arg_or_default("THUMBNAIL_PRESERVE_ASPECT_RATIO", "true"),
    ];
    args.extend(overrides.iter().map(|value| value.to_string()));

    <TestRuntimeCli as clap::Parser>::try_parse_from(args).map(|parsed| parsed.runtime)
}
