// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Integration tests for the `MetadataKeyRouter`.
//!
//! Test wiring: the router routes `nv-omniverse-nucleus-hash` to the real backend
//! (with the `metadata_key_remapper` layer in front, mirroring production wiring) and
//! defaults everything else to a `MockMetadata`. Tests assert which side handled each
//! request by inspecting the mock's recorded calls.

#![allow(clippy::too_many_arguments)]

use rstest::*;
use storage_core::traits::metadata::v1alpha::{
    value::Kind, GetMetadataRequest, NullValue, UpdateMetadataRequest, UserMetadataValue, Value,
};
use storage_core_integration_tests::fixture::*;
use storage_core_integration_tests::mocks::metadata::MockRequest;
use storage_core_integration_tests::storage_fixture;
use storage_core_integration_tests::utils::*;
use storage_core_test::service::{ApiVersion, Interface};
use tracing::debug;

const ROUTED_KEY: &str = "nv-omniverse-nucleus-hash";

fn string_value(s: &str) -> Value {
    Value {
        kind: Some(Kind::StringValue(s.to_string())),
    }
}

fn extract_string(value: &Value) -> &str {
    match value.kind.as_ref().unwrap() {
        Kind::StringValue(s) => s.as_str(),
        other => panic!("expected StringValue, got {other:?}"),
    }
}

fn extract_user_string(value: &UserMetadataValue) -> &str {
    extract_string(value.value.as_ref().unwrap())
}

/// Pre-seeded values on the mock should appear keyed by the literal string.
fn seed_user_value(s: &str) -> UserMetadataValue {
    UserMetadataValue {
        value: Some(string_value(s)),
        etag: format!("seed-etag-{s}"),
    }
}

/// (1) A write+read of the routed key never touches the mock.
#[rstest]
#[test_log::test(tokio::test)]
async fn router_routed_key_hits_real_backend(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    let access = Access::PrivateKey;
    let Some(mut temp_dir) = storage_fixture
        .router_temp_directory(backend, access, ApiVersion::V1Alpha)
        .await
    else {
        debug!("Skipping {:?}", backend);
        return;
    };

    let mock = storage_fixture.router_mock(backend);
    let file = temp_dir.new_file();
    let mut fo_client = storage_fixture
        .router_file_object_client(backend, Interface::Grpc, ApiVersion::V1Alpha)
        .await;
    write_data_body(&file, &random_data(64), None, None, &mut fo_client).await;

    // Make sure no prior tests on the same fixture have polluted the mock for this URI.
    mock.reset_uri(&file);

    let mut client = storage_fixture
        .router_metadata_client(backend, interface, ApiVersion::V1Alpha)
        .await;

    client
        .update_metadata(UpdateMetadataRequest {
            uri: file.clone(),
            user_metadata_key: ROUTED_KEY.to_string(),
            user_metadata: Some(string_value("hash-v1")),
            expected_etag: None,
        })
        .await
        .unwrap();

    let response = client
        .get_metadata(GetMetadataRequest {
            uri: file.clone(),
            user_metadata_keys: vec![ROUTED_KEY.to_string()],
        })
        .await
        .unwrap();

    assert_eq!(response.user_metadata.len(), 1);
    let entry = response.user_metadata.get(ROUTED_KEY).expect("missing routed key");
    assert_eq!(extract_user_string(entry), "hash-v1");

    assert!(
        mock.requests_for(&file).is_empty(),
        "mock must not be called for a routed key"
    );

    temp_dir.cleanup().await;
}

/// (2) A read of an un-routed (default) key returns mock state, and a follow-up write
/// is recorded by the mock.
#[rstest]
#[test_log::test(tokio::test)]
async fn router_default_key_hits_mock(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    let access = Access::PrivateKey;
    let Some(mut temp_dir) = storage_fixture
        .router_temp_directory(backend, access, ApiVersion::V1Alpha)
        .await
    else {
        debug!("Skipping {:?}", backend);
        return;
    };

    let mock = storage_fixture.router_mock(backend);
    let file = temp_dir.new_file();
    mock.reset_uri(&file);

    // Pre-seed mock state.
    mock.set(&file, "default-key", seed_user_value("seeded"));

    let mut client = storage_fixture
        .router_metadata_client(backend, interface, ApiVersion::V1Alpha)
        .await;

    let response = client
        .get_metadata(GetMetadataRequest {
            uri: file.clone(),
            user_metadata_keys: vec!["default-key".to_string()],
        })
        .await
        .unwrap();

    assert_eq!(response.user_metadata.len(), 1);
    let entry = response.user_metadata.get("default-key").expect("missing default key");
    assert_eq!(extract_user_string(entry), "seeded");

    // Follow-up write on the default key.
    client
        .update_metadata(UpdateMetadataRequest {
            uri: file.clone(),
            user_metadata_key: "default-key".to_string(),
            user_metadata: Some(string_value("written-via-router")),
            expected_etag: None,
        })
        .await
        .unwrap();

    let recorded = mock.requests_for(&file);
    assert!(recorded
        .iter()
        .any(|r| matches!(r, MockRequest::Get { keys, .. } if keys == &vec!["default-key".to_string()])));
    assert!(
        recorded
            .iter()
            .any(|r| matches!(r, MockRequest::Update { key, .. } if key == "default-key")),
        "mock should have recorded the update on default-key, got {recorded:?}"
    );

    temp_dir.cleanup().await;
}

/// (3) Multi-key get fans out: routed key from real backend, default key from mock,
/// merged in one response. Mock receives only its key.
#[rstest]
#[test_log::test(tokio::test)]
async fn router_get_metadata_multi_key_fans_out(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    let access = Access::PrivateKey;
    let Some(mut temp_dir) = storage_fixture
        .router_temp_directory(backend, access, ApiVersion::V1Alpha)
        .await
    else {
        debug!("Skipping {:?}", backend);
        return;
    };

    let mock = storage_fixture.router_mock(backend);
    let file = temp_dir.new_file();
    mock.reset_uri(&file);

    let mut fo_client = storage_fixture
        .router_file_object_client(backend, Interface::Grpc, ApiVersion::V1Alpha)
        .await;
    write_data_body(&file, &random_data(64), None, None, &mut fo_client).await;

    // Write the routed key (lands on the real backend).
    let mut client = storage_fixture
        .router_metadata_client(backend, interface, ApiVersion::V1Alpha)
        .await;
    client
        .update_metadata(UpdateMetadataRequest {
            uri: file.clone(),
            user_metadata_key: ROUTED_KEY.to_string(),
            user_metadata: Some(string_value("real-side")),
            expected_etag: None,
        })
        .await
        .unwrap();

    // Pre-seed the mock with the default key.
    mock.set(&file, "other-key", seed_user_value("mock-side"));

    let response = client
        .get_metadata(GetMetadataRequest {
            uri: file.clone(),
            user_metadata_keys: vec![ROUTED_KEY.to_string(), "other-key".to_string()],
        })
        .await
        .unwrap();

    assert_eq!(response.user_metadata.len(), 2);
    assert_eq!(
        extract_user_string(response.user_metadata.get(ROUTED_KEY).unwrap()),
        "real-side"
    );
    assert_eq!(
        extract_user_string(response.user_metadata.get("other-key").unwrap()),
        "mock-side"
    );

    // Mock saw only its key — never the routed one.
    let recorded = mock.requests_for(&file);
    let get_calls: Vec<_> = recorded
        .iter()
        .filter_map(|r| match r {
            MockRequest::Get { keys, .. } => Some(keys.clone()),
            _ => None,
        })
        .collect();
    assert!(!get_calls.is_empty(), "mock should have received a Get");
    for keys in &get_calls {
        assert_eq!(keys, &vec!["other-key".to_string()], "mock got unexpected keys");
    }

    temp_dir.cleanup().await;
}

/// (4) Empty-keys get fans out: routed branch asked for its keys, default asked with
/// empty list. All keys merge into one response.
#[rstest]
#[test_log::test(tokio::test)]
async fn router_get_metadata_empty_keys_fans_out(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3)] backend: Backend,
    #[values(Interface::Grpc)] interface: Interface,
) {
    let access = Access::PrivateKey;
    let Some(mut temp_dir) = storage_fixture
        .router_temp_directory(backend, access, ApiVersion::V1Alpha)
        .await
    else {
        debug!("Skipping {:?}", backend);
        return;
    };

    let mock = storage_fixture.router_mock(backend);
    let file = temp_dir.new_file();
    mock.reset_uri(&file);

    let mut fo_client = storage_fixture
        .router_file_object_client(backend, Interface::Grpc, ApiVersion::V1Alpha)
        .await;
    write_data_body(&file, &random_data(64), None, None, &mut fo_client).await;

    let mut client = storage_fixture
        .router_metadata_client(backend, interface, ApiVersion::V1Alpha)
        .await;

    client
        .update_metadata(UpdateMetadataRequest {
            uri: file.clone(),
            user_metadata_key: ROUTED_KEY.to_string(),
            user_metadata: Some(string_value("real-side-all")),
            expected_etag: None,
        })
        .await
        .unwrap();

    mock.set(&file, "alpha", seed_user_value("a"));
    mock.set(&file, "beta", seed_user_value("b"));

    let response = client
        .get_metadata(GetMetadataRequest {
            uri: file.clone(),
            user_metadata_keys: Vec::new(),
        })
        .await
        .unwrap();

    assert!(
        response.user_metadata.contains_key(ROUTED_KEY),
        "missing routed key in get-all response"
    );
    assert!(response.user_metadata.contains_key("alpha"));
    assert!(response.user_metadata.contains_key("beta"));
    assert_eq!(
        extract_user_string(response.user_metadata.get(ROUTED_KEY).unwrap()),
        "real-side-all"
    );

    // Mock should have received Get with empty keys (give-me-all on the default branch).
    let recorded = mock.requests_for(&file);
    let saw_empty_get = recorded
        .iter()
        .any(|r| matches!(r, MockRequest::Get { keys, .. } if keys.is_empty()));
    assert!(
        saw_empty_get,
        "mock should see Get([]) on the default branch, got {recorded:?}"
    );

    temp_dir.cleanup().await;
}

/// (5) Delete dispatches by key: routed deletes hit the real backend, default deletes
/// hit the mock.
#[rstest]
#[test_log::test(tokio::test)]
async fn router_delete_routes_correctly(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3)] backend: Backend,
    #[values(Interface::Grpc)] interface: Interface,
) {
    let access = Access::PrivateKey;
    let Some(mut temp_dir) = storage_fixture
        .router_temp_directory(backend, access, ApiVersion::V1Alpha)
        .await
    else {
        debug!("Skipping {:?}", backend);
        return;
    };

    let mock = storage_fixture.router_mock(backend);
    let file = temp_dir.new_file();
    mock.reset_uri(&file);

    let mut fo_client = storage_fixture
        .router_file_object_client(backend, Interface::Grpc, ApiVersion::V1Alpha)
        .await;
    write_data_body(&file, &random_data(64), None, None, &mut fo_client).await;

    // Set up: routed key on the real backend, default key on the mock.
    let mut client = storage_fixture
        .router_metadata_client(backend, interface, ApiVersion::V1Alpha)
        .await;
    client
        .update_metadata(UpdateMetadataRequest {
            uri: file.clone(),
            user_metadata_key: ROUTED_KEY.to_string(),
            user_metadata: Some(string_value("to-delete-routed")),
            expected_etag: None,
        })
        .await
        .unwrap();
    mock.set(&file, "default-key", seed_user_value("to-delete-default"));

    // Snapshot mock requests before deletes so we can isolate just the delete calls.
    let before = mock.requests_for(&file).len();

    // Delete routed key — should not be recorded by the mock.
    client
        .delete_metadata(storage_core::traits::metadata::v1alpha::DeleteMetadataRequest {
            uri: file.clone(),
            user_metadata_key: ROUTED_KEY.to_string(),
            expected_etag: None,
        })
        .await
        .unwrap();

    // Delete default key — should be recorded by the mock and removed from its state.
    client
        .delete_metadata(storage_core::traits::metadata::v1alpha::DeleteMetadataRequest {
            uri: file.clone(),
            user_metadata_key: "default-key".to_string(),
            expected_etag: None,
        })
        .await
        .unwrap();

    let after_requests = mock.requests_for(&file);
    let new_requests = &after_requests[before..];
    let mut saw_default_delete = false;
    for r in new_requests {
        match r {
            MockRequest::Delete { key, .. } if key == "default-key" => saw_default_delete = true,
            MockRequest::Delete { key, .. } if key == ROUTED_KEY => {
                panic!("mock unexpectedly saw a delete for the routed key")
            },
            _ => {},
        }
    }
    assert!(saw_default_delete, "mock should have seen delete on default-key");
    assert!(
        mock.get(&file, "default-key").is_none(),
        "default-key should be gone from mock"
    );

    // Sanity: read both keys back via the router. Routed key is gone (404 on real
    // backend bubbles up as missing). Default key is gone (mock returns nothing).
    let response = client
        .get_metadata(GetMetadataRequest {
            uri: file.clone(),
            user_metadata_keys: vec![ROUTED_KEY.to_string(), "default-key".to_string()],
        })
        .await;
    // Real backend may return Ok({}) (key absent) or a not-found status depending on
    // backend; either is acceptable. We only assert no surprising successful keys.
    if let Ok(resp) = response {
        assert!(!resp.user_metadata.contains_key("default-key"));
    }

    // Silence unused-import warning when running with reduced fixtures.
    let _ = NullValue::NullValue;

    temp_dir.cleanup().await;
}
