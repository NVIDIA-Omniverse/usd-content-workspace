// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Mutex;
use std::time::{Duration, Instant};

use storage_core_util::{error_limited, info_limited};

/// Inner width between the left and right box-drawing borders.
const W: usize = 78;

/// Result of a single API call.
#[derive(Debug, Clone)]
pub struct CallResult {
    pub api_name: String,
    pub phase: String,
    pub success: bool,
    pub error_message: Option<String>,
    pub latency: Duration,
}

/// Collects results from all API calls and prints a summary.
pub struct TrafficReport {
    results: Mutex<Vec<CallResult>>,
}

/// Format a line with box-drawing borders, padding the content to exactly `W` characters.
fn row(content: &str) -> String {
    let visual_len = content.chars().count();
    if visual_len >= W {
        format!(
            "║{}║",
            &content[..content.char_indices().nth(W).map_or(content.len(), |(i, _)| i)]
        )
    } else {
        format!("║{}{:pad$}║", content, "", pad = W - visual_len)
    }
}

fn border_top() -> String {
    format!("╔{:═>W$}╗", "")
}

fn border_mid() -> String {
    format!("╠{:═>W$}╣", "")
}

fn border_bot() -> String {
    format!("╚{:═>W$}╝", "")
}

impl TrafficReport {
    pub fn new() -> Self {
        Self {
            results: Mutex::new(Vec::new()),
        }
    }

    /// Record a successful API call.
    pub fn record_success(&self, phase: &str, api_name: &str, latency: Duration) {
        info_limited!(phase = phase, api = api_name, latency_ms = latency.as_millis(), "OK");
        self.results.lock().unwrap().push(CallResult {
            api_name: api_name.to_string(),
            phase: phase.to_string(),
            success: true,
            error_message: None,
            latency,
        });
    }

    /// Record a failed API call.
    pub fn record_failure(&self, phase: &str, api_name: &str, latency: Duration, error: &str) {
        error_limited!(
            phase = phase,
            api = api_name,
            latency_ms = latency.as_millis(),
            error = error,
            "FAIL"
        );
        self.results.lock().unwrap().push(CallResult {
            api_name: api_name.to_string(),
            phase: phase.to_string(),
            success: false,
            error_message: Some(error.to_string()),
            latency,
        });
    }

    /// Execute an async operation and record the result.
    pub async fn track<F, T>(&self, phase: &str, api_name: &str, f: F) -> Option<T>
    where
        F: std::future::Future<Output = Result<T, tonic::Status>>,
    {
        let start = Instant::now();
        match f.await {
            Ok(value) => {
                self.record_success(phase, api_name, start.elapsed());
                Some(value)
            },
            Err(e) => {
                self.record_failure(phase, api_name, start.elapsed(), &format!("{e}"));
                None
            },
        }
    }

    /// Print the final summary table.
    pub fn print_summary(&self) {
        let results = self.results.lock().unwrap();

        let total = results.len();
        let passed = results.iter().filter(|r| r.success).count();
        let failed = total - passed;

        println!();
        println!("{}", border_top());
        println!("{}", row(&format!("{:^W$}", "TRAFFIC GENERATOR SUMMARY")));
        println!("{}", border_mid());
        println!(
            "{}",
            row(&format!(
                "  {:<20} {:<30} {:<8} {:>12}",
                "Phase", "API", "Status", "Latency"
            ))
        );
        println!("{}", border_mid());

        for result in results.iter() {
            let status = if result.success { "OK" } else { "FAIL" };
            let latency = format!("{:.1}ms", result.latency.as_secs_f64() * 1000.0);
            println!(
                "{}",
                row(&format!(
                    "  {:<20} {:<30} {:<8} {:>12}",
                    result.phase, result.api_name, status, latency
                ))
            );
            if let Some(err) = &result.error_message {
                let max_err_len = W - 10; // "  err: " prefix + some margin
                let truncated = if err.len() > max_err_len {
                    format!("{}...", &err[..max_err_len - 3])
                } else {
                    err.clone()
                };
                println!("{}", row(&format!("    err: {truncated}")));
            }
        }

        println!("{}", border_mid());
        println!(
            "{}",
            row(&format!(
                "  Total: {:<5}  Passed: {:<5}  Failed: {:<5}",
                total, passed, failed
            ))
        );

        if !results.is_empty() {
            let total_latency: Duration = results.iter().map(|r| r.latency).sum();
            let avg_latency = total_latency / results.len() as u32;
            println!(
                "{}",
                row(&format!(
                    "  Total time: {:<12}  Avg latency: {:<12}",
                    format!("{:.1}ms", total_latency.as_secs_f64() * 1000.0),
                    format!("{:.1}ms", avg_latency.as_secs_f64() * 1000.0),
                ))
            );
        }

        println!("{}", border_bot());
        println!();

        if failed > 0 {
            println!("WARNING: {} API calls failed. See errors above.", failed);
        }
    }
}

/// Print the configuration header box (used by main).
pub fn print_config_header(
    grpc_addr: &str,
    rest_port: u16,
    base_url: &str,
    interface: &str,
    api_version: &str,
    iterations: u32,
) {
    let max_val = W - 17; // "  gRPC addr:    " = 16, plus 1 margin
    let truncated_url = if base_url.len() > max_val {
        format!("{}...", &base_url[..max_val - 3])
    } else {
        base_url.to_string()
    };

    println!("{}", border_top());
    println!("{}", row(&format!("{:^W$}", "Storage Core Traffic Generator")));
    println!("{}", border_mid());
    println!("{}", row(&format!("  gRPC addr:    {grpc_addr}")));
    println!("{}", row(&format!("  REST port:    {rest_port}")));
    println!("{}", row(&format!("  Base URL:     {truncated_url}")));
    println!("{}", row(&format!("  Interface:    {interface}")));
    println!("{}", row(&format!("  API Version:  {api_version}")));
    println!("{}", row(&format!("  Iterations:   {iterations}")));
    println!("{}", border_bot());
    println!();
}
