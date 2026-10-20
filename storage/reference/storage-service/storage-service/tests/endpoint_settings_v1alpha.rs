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

//! Tests for the per-endpoint settings the API reports back: whether an endpoint is read-only and
//! whether it has versioning. Both answers come from the configuration, so no bucket or storage
//! account is ever reached.

use rstest::{fixture, rstest};

use crate::common::{service::Service, types::Interface};

mod common;

const DEFAULT_ENDPOINT: &str = "https://default.s3.us-west-2.example.com";
const READ_ONLY_ENDPOINT: &str = "https://read-only.s3.us-west-2.example.com";
const ANONYMOUS_ENDPOINT: &str = "https://anonymous.s3.us-west-2.example.com";
const UNVERSIONED_ENDPOINT: &str = "https://unversioned.s3.us-west-2.example.com";
const ANONYMOUS_AZURE_ENDPOINT: &str = "https://anonymous.blob.core.windows.net";

/// Fictional endpoints covering the per-endpoint read-only and versioning settings, one of them
/// left at the defaults. Both answers are derived from the configuration, so the buckets and the
/// storage account are never accessed.
#[fixture]
#[once]
fn endpoint_settings_service() -> Service {
    use serde_json::json;

    let credentials = json!({
        "access_key": {
            "access_key_id": "unused",
            "secret_access_key": "unused",
        },
    });

    Service::start_with_config(json!({
        "storage": {
            "azure_blob": {
                "endpoints": {
                    ANONYMOUS_AZURE_ENDPOINT: {
                        "credentials": { "anonymous": true },
                        "read_only": false,
                    },
                },
            },
            "s3": {
                "endpoints": {
                    DEFAULT_ENDPOINT: { "credentials": &credentials },
                    READ_ONLY_ENDPOINT: {
                        "credentials": &credentials,
                        "read_only": true,
                    },
                    ANONYMOUS_ENDPOINT: {
                        "credentials": { "anonymous": true },
                        "read_only": false,
                    },
                    UNVERSIONED_ENDPOINT: {
                        "credentials": &credentials,
                        "versioning_enabled": false,
                    },
                },
            },
        },
        "rest": { "addr": "0.0.0.0", "port": 0 },
        "grpc": { "addr": "0.0.0.0", "port": 0 },
    }))
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_fileobject_is_read_only_reports_a_writable_endpoint(
    endpoint_settings_service: &Service,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::fileobject::types::v1alpha::IsReadOnlyRequest;

    let fileobject_client = endpoint_settings_service.fileobject_v1alpha_client(interface).await;

    let response = fileobject_client
        .is_read_only(IsReadOnlyRequest {
            resource_address: format!("{DEFAULT_ENDPOINT}/object.usd"),
        })
        .await
        .expect("is read only failed");

    assert!(!response.is_read_only);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_fileobject_is_read_only_reports_a_read_only_endpoint(
    endpoint_settings_service: &Service,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::fileobject::types::v1alpha::IsReadOnlyRequest;

    let fileobject_client = endpoint_settings_service.fileobject_v1alpha_client(interface).await;

    let response = fileobject_client
        .is_read_only(IsReadOnlyRequest {
            resource_address: format!("{READ_ONLY_ENDPOINT}/object.usd"),
        })
        .await
        .expect("is read only failed");

    assert!(response.is_read_only);
}

/// Anonymous credentials outrank the endpoint's own `read_only = false`.
#[rstest]
#[test_log::test(tokio::test)]
async fn test_fileobject_is_read_only_reports_an_anonymous_endpoint(
    endpoint_settings_service: &Service,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::fileobject::types::v1alpha::IsReadOnlyRequest;

    let fileobject_client = endpoint_settings_service.fileobject_v1alpha_client(interface).await;

    let response = fileobject_client
        .is_read_only(IsReadOnlyRequest {
            resource_address: format!("{ANONYMOUS_ENDPOINT}/object.usd"),
        })
        .await
        .expect("is read only failed");

    assert!(response.is_read_only);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_fileobject_is_read_only_reports_an_anonymous_azure_endpoint(
    endpoint_settings_service: &Service,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::fileobject::types::v1alpha::IsReadOnlyRequest;

    let fileobject_client = endpoint_settings_service.fileobject_v1alpha_client(interface).await;

    let response = fileobject_client
        .is_read_only(IsReadOnlyRequest {
            resource_address: format!("{ANONYMOUS_AZURE_ENDPOINT}/container/object.usd"),
        })
        .await
        .expect("is read only failed");

    assert!(response.is_read_only);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_versioning_is_versioned_reports_a_versioned_endpoint(
    endpoint_settings_service: &Service,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::versioning::types::v1alpha::IsVersionedRequest;

    let versioning_client = endpoint_settings_service.versioning_v1alpha_client(interface).await;

    let response = versioning_client
        .is_versioned(IsVersionedRequest {
            resource_address: format!("{DEFAULT_ENDPOINT}/object.usd"),
        })
        .await
        .expect("is versioned failed");

    assert!(response.is_versioned);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_versioning_is_versioned_reports_an_unversioned_endpoint(
    endpoint_settings_service: &Service,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use common::clients::versioning::types::v1alpha::IsVersionedRequest;

    let versioning_client = endpoint_settings_service.versioning_v1alpha_client(interface).await;

    let response = versioning_client
        .is_versioned(IsVersionedRequest {
            resource_address: format!("{UNVERSIONED_ENDPOINT}/object.usd"),
        })
        .await
        .expect("is versioned failed");

    assert!(!response.is_versioned);
}
