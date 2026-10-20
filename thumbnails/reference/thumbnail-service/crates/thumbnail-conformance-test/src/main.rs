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

mod auth;
mod config;
mod contract;
mod fixtures;
mod grpc;
mod rest;
mod style;

use std::process::ExitCode;

use clap::Parser;

use config::{Args, TestSuite};
use style::{Suite, TestError, TestRunner, FAIL, INFO};

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();

    match run(&args).await {
        Ok(all_passed) => {
            if all_passed {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(error) => {
            eprintln!("{FAIL} {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run(args: &Args) -> Result<bool, TestError> {
    if args.source_file.trim().is_empty() {
        return Err(TestError::new(
            "SOURCE_FILE/--source-file must be set to a non-empty fixture source",
        ));
    }
    if args.rest_url.as_deref().is_none_or(str::is_empty) {
        return Err(TestError::new(
            "REST_URL/--rest-url is required for OpenAPI verification in every suite, including grpc",
        ));
    }
    if args.runs_grpc() && args.grpc_url.as_deref().is_none_or(str::is_empty) {
        return Err(TestError::new(
            "GRPC_URL/--grpc-url is required for the grpc and all test suites",
        ));
    }

    println!("----------------------------------------");
    println!("Thumbnail Service Conformance Test");
    println!("----------------------------------------");
    println!("Test suite: {:?}", args.test_suite);
    if let Some(rest_url) = &args.rest_url {
        println!("REST URL: {rest_url}");
    }
    if let Some(grpc_url) = &args.grpc_url {
        println!("gRPC URL: {grpc_url} (TLS: {})", args.grpc_tls_enabled);
    }
    println!("Wait retries: {}", args.wait_retries);
    println!(
        "Wait interval: {}",
        humantime::format_duration(args.wait_interval)
    );
    println!(
        "HTTP timeout: {}",
        humantime::format_duration(args.http_timeout)
    );
    println!("----------------------------------------");
    println!();

    let token = auth::resolve_token(args).await?;
    println!(
        "{INFO} Auth mode: {}",
        if token.is_some() {
            "bearer token configured"
        } else {
            "unauthenticated"
        }
    );

    let mut runner = TestRunner::new();

    let rest_client = rest::RestClient::new(
        args.rest_url.as_deref().unwrap_or_default(),
        token.clone(),
        args.http_timeout,
    );
    let mut rest_ready = false;
    if args.runs_rest() {
        if let Err(error) = rest::wait_for_ready(
            &rest_client,
            &args.ready_path,
            args.wait_retries,
            args.wait_interval,
        )
        .await
        {
            runner.record_fail(Suite::Rest, "REST readiness (run incomplete)", 0, &error);
            if !args.runs_grpc() {
                runner.print_summary();
                return Ok(false);
            }
        } else {
            rest_ready = true;
        }
    }
    let bound = match rest::advertised_bound(&rest_client).await {
        Ok(bound) => {
            runner.record_pass(
                if rest_ready { Suite::Rest } else { Suite::Grpc },
                "verify advertised OpenAPI specification bound",
                0,
            );
            bound
        }
        Err(error) => {
            runner.record_fail(
                if rest_ready { Suite::Rest } else { Suite::Grpc },
                "verify advertised OpenAPI specification bound",
                0,
                &error,
            );
            runner.print_summary();
            return Ok(false);
        }
    };
    if rest_ready {
        rest::run(&mut runner, &rest_client, &args.source_file, bound).await;
    }

    if matches!(args.test_suite, TestSuite::All | TestSuite::Grpc) {
        let grpc_url = args.grpc_url.as_deref().unwrap_or_default();
        // Recorded as a failed case rather than propagated with `?`. Returning
        // here skipped `print_summary`, so a gRPC endpoint that would not
        // connect threw away every REST result already gathered — including the
        // misconfiguration warning, in the situation most likely to have caused
        // it. The run still fails; it now says what it managed to establish
        // first.
        match grpc::wait_for_ready(
            grpc_url,
            args.grpc_tls_enabled,
            token.clone(),
            args.http_timeout,
            args.wait_retries,
            args.wait_interval,
        )
        .await
        {
            Ok(mut client) => grpc::run(&mut runner, &mut client, &args.source_file, bound).await,
            Err(error) => {
                runner.record_fail(Suite::Grpc, "gRPC readiness (run incomplete)", 0, &error)
            }
        }
    }

    runner.print_summary();
    Ok(runner.all_passed())
}
