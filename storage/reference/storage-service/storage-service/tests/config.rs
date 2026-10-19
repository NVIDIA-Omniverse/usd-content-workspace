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

use bytes::Bytes;
use serde_json::json;
use tonic::transport::Endpoint;

use crate::common::{
    clients::{fileobject, utils::grpc::connect_with_retries},
    env::get_env,
    random::get_random_path,
    service::handle::ServiceHandle,
};
use toml::toml;

mod common;

#[test_log::test(tokio::test)]
async fn test_env_variable_references_are_interpolated_in_json() {
    let env = get_env().unwrap_azure();
    let config_json = json!({
        "storage": {
            "azure_blob": {
                "endpoints": {
                    env.storage_account_url(): {
                        "credentials": {
                            "storage_key": {
                                "storage_key": "${STORAGE_SERVICE_STORAGE_KEY}"
                            }
                        }
                    },
                },
            },
        },
        "egress_throttle": {
            "enabled": "${STORAGE_SERVICE_EGRESS_THROTTLE_ENABLED}",
            "max_concurrency": "${STORAGE_SERVICE_EGRESS_MAX_CONCURRENCY}",
        },
        "grpc": {
            "port": 0,
        },
        "rest": {
            "port": 0,
        },
        "metrics": {
            "port": 0,
        },
    })
    .to_string();
    let service = ServiceHandle::spawn(
        env!("CARGO_BIN_EXE_storage-service"),
        ["--config-json", &config_json],
        [
            ("STORAGE_SERVICE_STORAGE_KEY", env.storage_key.as_str()),
            ("STORAGE_SERVICE_EGRESS_THROTTLE_ENABLED", "true"),
            ("STORAGE_SERVICE_EGRESS_MAX_CONCURRENCY", "10"),
        ],
    );

    assert_write_succeeds(service.grpc_endpoint()).await;
}

#[test_log::test(tokio::test)]
async fn test_env_variable_references_are_interpolated_in_toml() {
    let env = get_env().unwrap_azure();
    let config_toml = toml! {
        [storage.azure_blob.endpoints."${STORAGE_ACCOUNT_URL}"]
        credentials.storage_key.storage_key = "${STORAGE_SERVICE_STORAGE_KEY}"

        [egress_throttle]
        enabled = "${STORAGE_SERVICE_EGRESS_THROTTLE_ENABLED}"
        max_concurrency = "${STORAGE_SERVICE_EGRESS_MAX_CONCURRENCY}"

        [rest]
        port = 0

        [grpc]
        port = 0

        [metrics]
        port = 0
    }
    .to_string();

    let storage_account_url = env.storage_account_url();
    let service = ServiceHandle::spawn(
        env!("CARGO_BIN_EXE_storage-service"),
        ["--config-toml", config_toml.as_str()],
        [
            ("STORAGE_ACCOUNT_URL", storage_account_url.as_str()),
            ("STORAGE_SERVICE_STORAGE_KEY", env.storage_key.as_str()),
            ("STORAGE_SERVICE_EGRESS_THROTTLE_ENABLED", "true"),
            ("STORAGE_SERVICE_EGRESS_MAX_CONCURRENCY", "10"),
        ],
    );

    assert_write_succeeds(service.grpc_endpoint()).await;
}

/// Call the write object operation against the service and ensure it has succeeded.
async fn assert_write_succeeds(endpoint: impl Into<String>) {
    use fileobject::{grpc::v1alpha::Client as GrpcClient, v1alpha::Client};

    let env = get_env().unwrap_azure();

    let endpoint = Bytes::from(endpoint.into().into_bytes());
    let channel = connect_with_retries(move || {
        let endpoint = Endpoint::from_shared(endpoint.clone()).expect("grpc endpoint parse failed");
        async move { endpoint.connect().await }
    })
    .await;

    Client::Grpc(GrpcClient::new(channel))
        .write_random_object(env.resource_url(get_random_path()), 32)
        .await
        .expect("object write failed");
}
