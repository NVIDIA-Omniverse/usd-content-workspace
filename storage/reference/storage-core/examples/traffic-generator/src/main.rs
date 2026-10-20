// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

mod capabilities;
mod filefolder;
mod fileobject;
mod metadata;
mod report;
mod runner;
mod versioning;

use clap::Parser;
use runner::{ApiVersionArg, InterfaceArg};
use tracing_subscriber::EnvFilter;

/// Traffic generator for storage-core API services.
///
/// Connects to a running storage-core service and systematically exercises
/// every API function (FileObject, FileFolder, Capabilities, Versioning)
/// across the selected interface(s) and API version(s).
#[derive(Parser, Debug)]
#[command(name = "traffic-generator", version, about)]
struct Args {
    /// gRPC server address (e.g., http://127.0.0.1:50051)
    #[arg(long, default_value = "http://127.0.0.1:50051")]
    grpc_addr: String,

    /// REST server port (e.g., 8080)
    #[arg(long, default_value_t = 8080)]
    rest_port: u16,

    /// Base URL for resource addresses.
    /// This is the storage backend URL prefix used for all file/folder operations.
    /// Example: `https://exampleaccountprivate.blob.core.windows.net/unittests/`
    #[arg(long)]
    base_url: String,

    /// Interface(s) to exercise: grpc, rest, or both
    #[arg(long, value_enum, default_value_t = InterfaceArg::Both)]
    interface: InterfaceArg,

    /// API version(s) to exercise: v1-alpha, v1-beta, or both
    #[arg(long, value_enum, default_value_t = ApiVersionArg::V1Alpha)]
    api_version: ApiVersionArg,

    /// Number of iterations to run the full traffic sweep
    #[arg(long, short = 'n', default_value_t = 1)]
    iterations: u32,
}

#[tokio::main]
async fn main() {
    // Initialize tracing with env filter (default to info)
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let args = Args::parse();

    // Ensure base_url ends with /
    let base_url = if args.base_url.ends_with('/') {
        args.base_url.clone()
    } else {
        format!("{}/", args.base_url)
    };

    report::print_config_header(
        &args.grpc_addr,
        args.rest_port,
        &base_url,
        &format!("{:?}", args.interface),
        &format!("{:?}", args.api_version),
        args.iterations,
    );

    let report = report::TrafficReport::new();

    for i in 1..=args.iterations {
        if args.iterations > 1 {
            println!();
            println!("┌──────────────────────────────────────────────────────────────────────────────┐");
            println!("│  Iteration {}/{:<62} │", i, args.iterations);
            println!("└──────────────────────────────────────────────────────────────────────────────┘");
        }

        runner::run(
            &args.grpc_addr,
            args.rest_port,
            &base_url,
            args.interface,
            args.api_version,
            &report,
        )
        .await;
    }

    report.print_summary();
}
