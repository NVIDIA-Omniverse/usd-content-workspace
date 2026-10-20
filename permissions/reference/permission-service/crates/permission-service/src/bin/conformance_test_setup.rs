// SPDX-FileCopyrightText: Copyright 2026 NVIDIA CORPORATION & AFFILIATES
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

/// Sets up authorization data (metadata + policy) on a running permission-service
/// via its REST API, preparing it for conformance testing.
use clap::Parser;
use std::process::ExitCode;

#[derive(Parser, Debug)]
#[command(
    name = "conformance-test-setup",
    about = "Set up authorization data on a running permission-service for conformance testing"
)]
struct Args {
    #[arg(long, env = "REST_URL")]
    rest_url: String,

    #[arg(long, env = "PRINCIPAL", default_value = "conformance-test-principal")]
    principal: String,

    #[arg(long, env = "ACTION_NAME", default_value = "read")]
    action_name: String,

    #[arg(long, env = "ACTION_SERVICE", default_value = "conformance-test")]
    action_service: String,

    #[arg(long, env = "RESOURCE_ID", default_value = "conformance-test-resource")]
    resource_id: String,

    #[arg(long, env = "RESOURCE_TYPE", default_value = "ConformanceTestResource")]
    resource_type: String,

    #[arg(long, env = "WAIT_RETRIES", default_value_t = 10)]
    wait_retries: u32,

    #[arg(long, env = "WAIT_INTERVAL", default_value_t = 5)]
    wait_interval: u64,
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    let client = reqwest::Client::new();

    if let Err(msg) = wait_for_health(&client, &args).await {
        eprintln!("ERROR: {}", msg);
        return ExitCode::FAILURE;
    }

    if let Err(msg) = setup_metadata(&client, &args).await {
        eprintln!("ERROR: {}", msg);
        return ExitCode::FAILURE;
    }

    if let Err(msg) = setup_policy(&client, &args).await {
        eprintln!("ERROR: {}", msg);
        return ExitCode::FAILURE;
    }

    println!("Setup complete.");
    ExitCode::SUCCESS
}

async fn wait_for_health(client: &reqwest::Client, args: &Args) -> Result<(), String> {
    let url = format!("{}/health", args.rest_url);
    for attempt in 1..=args.wait_retries {
        match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => return Ok(()),
            Ok(resp) => eprintln!(
                "Health check attempt {}/{}: status {}",
                attempt,
                args.wait_retries,
                resp.status()
            ),
            Err(e) => eprintln!(
                "Health check attempt {}/{}: {}",
                attempt, args.wait_retries, e
            ),
        }
        tokio::time::sleep(std::time::Duration::from_secs(args.wait_interval)).await;
    }
    Err(format!(
        "Service at {} did not become healthy after {} retries",
        args.rest_url, args.wait_retries
    ))
}

async fn setup_metadata(client: &reqwest::Client, args: &Args) -> Result<(), String> {
    let base = &args.rest_url;

    let url = format!("{}/v1beta/services/{}/", base, args.action_service);
    let resp = client
        .put(&url)
        .json(&serde_json::json!({
            "service": args.action_service,
            "id_claim": ""
        }))
        .send()
        .await
        .map_err(|e| format!("PUT service metadata: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!(
            "PUT service metadata: {} {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        ));
    }
    println!("Registered service: {}", args.action_service);

    let url = format!(
        "{}/v1beta/services/{}/actions/{}/",
        base, args.action_service, args.action_name
    );
    let resp = client
        .put(&url)
        .send()
        .await
        .map_err(|e| format!("PUT service action: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!(
            "PUT service action: {} {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        ));
    }
    println!(
        "Registered action: {}:{}",
        args.action_service, args.action_name
    );

    let url = format!(
        "{}/v1beta/services/{}/resource-types/{}/",
        base, args.action_service, args.resource_type
    );
    let resp = client
        .put(&url)
        .json(&serde_json::json!({
            "service": args.action_service,
            "type": args.resource_type,
            "evaluationPriority": "permit"
        }))
        .send()
        .await
        .map_err(|e| format!("PUT resource type: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!(
            "PUT resource type: {} {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        ));
    }
    println!("Registered resource type: {}", args.resource_type);

    Ok(())
}

async fn setup_policy(client: &reqwest::Client, args: &Args) -> Result<(), String> {
    let policy_text = format!(
        "permit(principal == Principal::\"{}\", action == Action::\"{}:{}\", resource == {}::\"{}\");",
        args.principal, args.action_service, args.action_name, args.resource_type, args.resource_id
    );

    let url = format!("{}/v1beta/policies/", args.rest_url);
    let resp = client
        .put(&url)
        .json(&serde_json::json!({
            "policy": policy_text,
            "order": 0
        }))
        .send()
        .await
        .map_err(|e| format!("PUT policy: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!(
            "PUT policy: {} {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        ));
    }
    println!(
        "Created policy for principal={} action={}:{} resource={}::{}",
        args.principal, args.action_service, args.action_name, args.resource_type, args.resource_id
    );

    Ok(())
}
