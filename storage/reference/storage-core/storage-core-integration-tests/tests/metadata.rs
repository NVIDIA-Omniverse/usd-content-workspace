// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

#![allow(clippy::too_many_arguments)]

use assert_matches::assert_matches;
use futures::TryStreamExt;
use rstest::*;
use storage_core::traits::metadata::v1alpha::{value::Kind, ListValue, NullValue, Struct, Value};
use tonic::Code;
use tracing::debug;

use storage_core::traits::metadata::v1alpha::*;
use storage_core::traits::versioning::v1alpha::{EnumerateVersionsRequest, VersionsOrder};
use storage_core_integration_tests::fixture::*;
use storage_core_integration_tests::utils::*;
use storage_core_integration_tests::{assert_method_metric, assert_metric, storage_fixture};
use storage_core_test::service::{ApiVersion, Interface};

fn extract_string_value(value: &UserMetadataValue) -> &str {
    match value.value.as_ref().unwrap().kind.as_ref().unwrap() {
        Kind::StringValue(s) => s.as_str(),
        other => panic!("Expected StringValue, got {:?}", other),
    }
}

fn assert_value_eq(actual: &Value, expected: &Value) {
    assert_eq!(actual, expected, "metadata value mismatch");
}

fn string_value(s: &str) -> Value {
    Value {
        kind: Some(Kind::StringValue(s.to_string())),
    }
}

fn number_value(n: f64) -> Value {
    Value {
        kind: Some(Kind::NumberValue(n)),
    }
}

fn bool_value(b: bool) -> Value {
    Value {
        kind: Some(Kind::BoolValue(b)),
    }
}

fn null_value() -> Value {
    Value {
        kind: Some(Kind::NullValue(NullValue::NullValue as i32)),
    }
}

fn list_value(vals: Vec<Value>) -> Value {
    Value {
        kind: Some(Kind::ListValue(ListValue { values: vals })),
    }
}

fn struct_value(fields: Vec<(&str, Value)>) -> Value {
    Value {
        kind: Some(Kind::StructValue(Struct {
            fields: fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
        })),
    }
}

#[rstest]
#[case::case_01_hello_two_keys("unittests/metadata/hello.txt", &[("foo", "bar"), ("testmetadatakey", "testvalue")])]
#[test_log::test(tokio::test)]
async fn get_metadata(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3, Backend::Minio, Backend::GCPS3Compat)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::Public, Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(ApiVersion::V1Alpha)] api_version: ApiVersion,
    #[values((Cache::Stat, 10), (Cache::Disabled, 1))] cache: (Cache, usize),
    #[case] base_address: &str,
    #[case] expected_metadata: &[(&str, &str)],
) {
    let Some(address) = storage_fixture.make_url(backend, access, base_address) else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    let mut client = storage_fixture
        .metadata_client(backend, cache.0, interface, api_version)
        .await;

    for _ in 0..cache.1 {
        let request = GetMetadataRequest {
            uri: address.clone(),
            user_metadata_keys: expected_metadata.iter().map(|(k, _)| k.to_string()).collect(),
        };

        let response = client.get_metadata(request).await.unwrap();
        debug!(?response);

        assert_eq!(response.user_metadata.len(), expected_metadata.len());

        for (key, expected_value) in expected_metadata {
            let entry = response
                .user_metadata
                .get(*key)
                .unwrap_or_else(|| panic!("missing key '{}'", key));
            assert_eq!(extract_string_value(entry), *expected_value);
            assert!(entry.etag.is_empty(), "etag for '{}' should be empty", key);
        }
    }

    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.requests",
        "get_metadata",
        ["result" => "Ok"]
    );

    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.request.duration",
        "get_metadata",
        [],
        1
    );

    if cache.0 == Cache::Stat {
        assert_metric!(
            storage_fixture,
            "storage.cache.operations",
            ["access_type" => "miss", "name" => "metadata_cache"],
            "name=metadata_cache"
        );
        assert_metric!(
            storage_fixture,
            "storage.cache.operations",
            ["access_type" => "access", "name" => "metadata_cache"],
            "name=metadata_cache"
        );
    } else if cache.0 == Cache::Disabled {
        assert_method_metric!(
            storage_fixture,
            backend,
            api_version,
            "storage.sdk.requests",
            "get_metadata",
            ["result" => "Ok"]
        );

        assert_method_metric!(
            storage_fixture,
            backend,
            api_version,
            "storage.sdk.request.duration",
            "get_metadata",
            [],
            1
        );
    }
}

#[rstest]
#[case::case_01_hello_two_keys("unittests/metadata/hello.txt", &[("foo", "bar"), ("testmetadatakey", "testvalue")])]
#[test_log::test(tokio::test)]
async fn get_metadata_all_keys(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3, Backend::Minio, Backend::GCPS3Compat)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::Public, Access::PrivateKey)] access: Access,
    #[values(ApiVersion::V1Alpha)] api_version: ApiVersion,
    #[values(Cache::Disabled, Cache::Stat)] cache: Cache,
    #[case] base_address: &str,
    #[case] expected_metadata: &[(&str, &str)],
) {
    let Some(address) = storage_fixture.make_url(backend, access, base_address) else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    let mut client = storage_fixture
        .metadata_client(backend, cache, interface, api_version)
        .await;

    // Pass empty user_metadata_keys to request all keys
    let request = GetMetadataRequest {
        uri: address,
        user_metadata_keys: vec![],
    };

    let response = client.get_metadata(request).await.unwrap();
    debug!(?response);

    // Should contain at least the expected keys (may contain others set by other tests)
    for (key, expected_value) in expected_metadata {
        let entry = response
            .user_metadata
            .get(*key)
            .unwrap_or_else(|| panic!("missing key '{}' in all-keys response", key));
        assert_eq!(extract_string_value(entry), *expected_value);
        assert!(entry.etag.is_empty(), "etag for '{}' should be empty", key);
    }
}

#[rstest]
#[case::case_01_string("string_val", string_value("hello world"))]
#[case::case_02_number("number_val", number_value(42.5))]
#[case::case_03_bool("bool_val", bool_value(true))]
#[case::case_04_null("null_val", null_value())]
#[case::case_05_list("list_val", list_value(vec![string_value("a"), number_value(1.0), bool_value(false)]))]
#[case::case_06_struct("struct_val", struct_value(vec![("nested_key", string_value("nested_value")), ("count", number_value(7.0))]))]
#[test_log::test(tokio::test)]
async fn update_and_get_metadata(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3, Backend::Minio, Backend::GCPS3Compat)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(ApiVersion::V1Alpha)] api_version: ApiVersion,
    #[values(Cache::Disabled, Cache::Stat)] cache: Cache,
    #[case] key: &str,
    #[case] value: Value,
) {
    let mut fo_client = storage_fixture
        .file_object_client(backend, cache, Interface::Grpc, ApiVersion::V1Alpha)
        .await;

    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, ApiVersion::V1Alpha)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    let file = temp_dir.new_file();
    let data = random_data(64);
    write_data_body(&file, &data, None, None, &mut fo_client).await;

    let mut client = storage_fixture
        .metadata_client(backend, cache, interface, api_version)
        .await;

    // Set metadata
    let update_response = client
        .update_metadata(UpdateMetadataRequest {
            uri: file.clone(),
            user_metadata_key: key.to_string(),
            user_metadata: Some(value.clone()),
            expected_etag: None,
        })
        .await
        .unwrap();
    debug!(?update_response);
    assert!(update_response.etag.is_empty());

    // Read it back and verify
    let get_response = client
        .get_metadata(GetMetadataRequest {
            uri: file.clone(),
            user_metadata_keys: vec![key.to_string()],
        })
        .await
        .unwrap();
    debug!(?get_response);

    assert_eq!(get_response.user_metadata.len(), 1);
    let entry = get_response
        .user_metadata
        .get(key)
        .unwrap_or_else(|| panic!("missing key '{}'", key));
    assert_value_eq(entry.value.as_ref().unwrap(), &value);
    assert!(entry.etag.is_empty());

    // Verify metrics
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.requests",
        "update_metadata",
        ["result" => "Ok"]
    );

    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.request.duration",
        "update_metadata",
        [],
        1
    );

    temp_dir.cleanup().await;
}

#[rstest]
#[test_log::test(tokio::test)]
async fn update_metadata_batch(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(Cache::Disabled)] cache: Cache,
) {
    let api_version = ApiVersion::V1Alpha;
    let mut fo_client = storage_fixture
        .file_object_client(backend, cache, Interface::Grpc, api_version)
        .await;

    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, api_version)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    let file = temp_dir.new_file();
    write_data_body(&file, &random_data(64), None, None, &mut fo_client).await;

    let mut client = storage_fixture
        .metadata_client(backend, cache, interface, api_version)
        .await;

    // Per the proto, "each entry is processed independently with the same rules
    // as a single UpdateMetadata call." Only same-(uri, user_metadata_key)
    // conflicts are undefined; different keys on the same URI must each land.
    // The helper serializes per-URI to honor this on HEAD-merge-PUT backends.
    let request = UpdateMetadataBatchRequest {
        requests: vec![
            UpdateMetadataRequest {
                uri: file.clone(),
                user_metadata_key: "batch_a".to_string(),
                user_metadata: Some(string_value("alpha")),
                expected_etag: None,
            },
            UpdateMetadataRequest {
                uri: file.clone(),
                user_metadata_key: String::new(),
                user_metadata: Some(string_value("bad")),
                expected_etag: None,
            },
            UpdateMetadataRequest {
                uri: file.clone(),
                user_metadata_key: "batch_b".to_string(),
                user_metadata: Some(number_value(7.0)),
                expected_etag: None,
            },
        ],
    };

    let mut stream = client.update_metadata_batch(request).await.unwrap();
    let mut results = Vec::new();
    while let Some(response) = stream.try_next().await.unwrap() {
        results.extend(response.results);
    }

    // Within a single URI group entries are serialized in input order, but
    // assert the indices form a permutation of [0..N) regardless before
    // sorting for the outcome checks.
    let mut indices: Vec<u32> = results.iter().map(|r| r.request_index).collect();
    indices.sort_unstable();
    assert_eq!(indices, vec![0, 1, 2]);
    results.sort_by_key(|result| result.request_index);

    assert_eq!(results.len(), 3);
    assert_matches!(
        results[0].outcome.as_ref().unwrap(),
        update_metadata_batch_entry_result::Outcome::Success(_)
    );
    assert_matches!(
        results[1].outcome.as_ref().unwrap(),
        update_metadata_batch_entry_result::Outcome::Error(error)
            if error.code == update_metadata_batch_entry_error::Code::InvalidArgument as i32
    );
    assert_matches!(
        results[2].outcome.as_ref().unwrap(),
        update_metadata_batch_entry_result::Outcome::Success(_)
    );

    // Both successful per-entry writes must persist on the same URI; the per-URI
    // serialization is what makes this safe with HEAD-merge-PUT backends.
    let get_response = client
        .get_metadata(GetMetadataRequest {
            uri: file.clone(),
            user_metadata_keys: vec!["batch_a".to_string(), "batch_b".to_string()],
        })
        .await
        .unwrap();
    assert_value_eq(
        get_response
            .user_metadata
            .get("batch_a")
            .unwrap()
            .value
            .as_ref()
            .unwrap(),
        &string_value("alpha"),
    );
    assert_value_eq(
        get_response
            .user_metadata
            .get("batch_b")
            .unwrap()
            .value
            .as_ref()
            .unwrap(),
        &number_value(7.0),
    );

    temp_dir.cleanup().await;
}

#[rstest]
#[case::case_01_string("string_val", string_value("to be deleted"))]
#[case::case_02_number("number_val", number_value(99.0))]
#[case::case_03_bool("bool_val", bool_value(false))]
#[test_log::test(tokio::test)]
async fn delete_metadata(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3, Backend::Minio, Backend::GCPS3Compat)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(ApiVersion::V1Alpha)] api_version: ApiVersion,
    #[values(Cache::Disabled, Cache::Stat)] cache: Cache,
    #[case] key: &str,
    #[case] value: Value,
) {
    let mut fo_client = storage_fixture
        .file_object_client(backend, cache, Interface::Grpc, ApiVersion::V1Alpha)
        .await;

    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, ApiVersion::V1Alpha)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    let file = temp_dir.new_file();
    let data = random_data(64);
    write_data_body(&file, &data, None, None, &mut fo_client).await;

    let mut client = storage_fixture
        .metadata_client(backend, cache, interface, api_version)
        .await;

    // Set metadata first
    client
        .update_metadata(UpdateMetadataRequest {
            uri: file.clone(),
            user_metadata_key: key.to_string(),
            user_metadata: Some(value.clone()),
            expected_etag: None,
        })
        .await
        .unwrap();

    // Verify it exists
    let get_response = client
        .get_metadata(GetMetadataRequest {
            uri: file.clone(),
            user_metadata_keys: vec![key.to_string()],
        })
        .await
        .unwrap();
    assert_eq!(get_response.user_metadata.len(), 1);

    // Delete it
    let delete_response = client
        .delete_metadata(DeleteMetadataRequest {
            uri: file.clone(),
            user_metadata_key: key.to_string(),
            expected_etag: None,
        })
        .await
        .unwrap();
    debug!(?delete_response);

    // Verify it's gone
    let get_response = client
        .get_metadata(GetMetadataRequest {
            uri: file.clone(),
            user_metadata_keys: vec![key.to_string()],
        })
        .await
        .unwrap();
    assert!(
        get_response.user_metadata.is_empty(),
        "expected metadata to be deleted, but got: {:?}",
        get_response.user_metadata
    );

    // Verify metrics
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.requests",
        "delete_metadata",
        ["result" => "Ok"]
    );

    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.request.duration",
        "delete_metadata",
        [],
        1
    );

    temp_dir.cleanup().await;
}

#[rstest]
#[test_log::test(tokio::test)]
async fn update_metadata_rejects_etag(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3, Backend::Minio, Backend::GCPS3Compat)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(ApiVersion::V1Alpha)] api_version: ApiVersion,
    #[values(Cache::Disabled, Cache::Stat)] cache: Cache,
) {
    let mut fo_client = storage_fixture
        .file_object_client(backend, cache, Interface::Grpc, ApiVersion::V1Alpha)
        .await;

    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, ApiVersion::V1Alpha)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    let file = temp_dir.new_file();
    write_data_body(&file, &random_data(64), None, None, &mut fo_client).await;

    let mut client = storage_fixture
        .metadata_client(backend, cache, interface, api_version)
        .await;

    let err = client
        .update_metadata(UpdateMetadataRequest {
            uri: file.clone(),
            user_metadata_key: "key".to_string(),
            user_metadata: Some(string_value("value")),
            expected_etag: Some("\"0x12345\"".to_string()),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);

    temp_dir.cleanup().await;
}

#[rstest]
#[test_log::test(tokio::test)]
async fn delete_metadata_rejects_etag(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3, Backend::Minio, Backend::GCPS3Compat)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(ApiVersion::V1Alpha)] api_version: ApiVersion,
    #[values(Cache::Disabled, Cache::Stat)] cache: Cache,
) {
    let mut fo_client = storage_fixture
        .file_object_client(backend, cache, Interface::Grpc, ApiVersion::V1Alpha)
        .await;

    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, ApiVersion::V1Alpha)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    let file = temp_dir.new_file();
    write_data_body(&file, &random_data(64), None, None, &mut fo_client).await;

    let mut client = storage_fixture
        .metadata_client(backend, cache, interface, api_version)
        .await;

    let err = client
        .delete_metadata(DeleteMetadataRequest {
            uri: file.clone(),
            user_metadata_key: "key".to_string(),
            expected_etag: Some("\"0x12345\"".to_string()),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);

    temp_dir.cleanup().await;
}

#[rstest]
#[case::case_01_goodbye_versions("unittests/metadata/goodbye.txt", "testversionkey", &[(1, "value1"), (4, "value4")])]
#[test_log::test(tokio::test)]
async fn get_versioned_metadata(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(ApiVersion::V1Alpha)] api_version: ApiVersion,
    #[values(Cache::Disabled, Cache::Stat)] cache: Cache,
    #[case] base_address: &str,
    #[case] key: &str,
    #[case] expected_versions: &[(usize, &str)],
) {
    let Some(address) = storage_fixture.make_url(backend, Access::Versioned, base_address) else {
        debug!("Skipping test for {:?} {:?}", backend, Access::Versioned);
        return;
    };

    let mut versioning_client = storage_fixture
        .versioning_client(backend, cache, interface, ApiVersion::V1Alpha)
        .await;

    let responses: Vec<_> = versioning_client
        .enumerate_versions(EnumerateVersionsRequest {
            resource_address: address,
        })
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();

    let versions_order = responses
        .first()
        .map(|r| r.versions_order())
        .unwrap_or(VersionsOrder::Unspecified);

    let mut versions: Vec<_> = responses.into_iter().flat_map(|response| response.items).collect();

    if versions_order == VersionsOrder::NewestFirst {
        versions.reverse();
    }

    let mut metadata_client = storage_fixture
        .metadata_client(backend, cache, interface, api_version)
        .await;

    for &(version_index, expected_value) in expected_versions {
        assert!(
            versions.len() > version_index,
            "expected at least {} versions, got {}",
            version_index + 1,
            versions.len()
        );

        let version_address = versions[version_index]
            .resource_address
            .as_ref()
            .unwrap_or_else(|| panic!("version {version_index} missing resource_address"));

        let response = metadata_client
            .get_metadata(GetMetadataRequest {
                uri: version_address.clone(),
                user_metadata_keys: vec![key.to_string()],
            })
            .await
            .unwrap();
        debug!(?response);

        let entry = response
            .user_metadata
            .get(key)
            .unwrap_or_else(|| panic!("missing key '{key}' on version {version_index}"));
        assert_eq!(
            extract_string_value(entry),
            expected_value,
            "version {version_index}: expected '{expected_value}'"
        );
    }
}

#[rstest]
#[test_log::test(tokio::test)]
async fn update_metadata_value_too_large(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3, Backend::Minio)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(ApiVersion::V1Alpha)] api_version: ApiVersion,
    #[values(Cache::Disabled, Cache::Stat)] cache: Cache,
) {
    let mut fo_client = storage_fixture
        .file_object_client(backend, cache, Interface::Grpc, ApiVersion::V1Alpha)
        .await;

    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, ApiVersion::V1Alpha)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    let file = temp_dir.new_file();
    write_data_body(&file, &random_data(64), None, None, &mut fo_client).await;

    let mut client = storage_fixture
        .metadata_client(backend, cache, interface, api_version)
        .await;

    let large_value = "x".repeat(10 * 1024);
    let err = client
        .update_metadata(UpdateMetadataRequest {
            uri: file.clone(),
            user_metadata_key: "big".to_string(),
            user_metadata: Some(string_value(&large_value)),
            expected_etag: None,
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);

    temp_dir.cleanup().await;
}

#[rstest]
#[test_log::test(tokio::test)]
async fn get_metadata_not_found(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3, Backend::Minio, Backend::GCPS3Compat)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(ApiVersion::V1Alpha)] api_version: ApiVersion,
    #[values(Cache::Disabled, Cache::Stat)] cache: Cache,
) {
    let Some(address) = storage_fixture.make_url(backend, access, "unittests/metadata/doesnotexist.txt") else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    let mut client = storage_fixture
        .metadata_client(backend, cache, interface, api_version)
        .await;

    let err = client
        .get_metadata(GetMetadataRequest {
            uri: address,
            user_metadata_keys: vec!["anykey".to_string()],
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::NotFound);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn get_metadata_missing_key(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3, Backend::Minio, Backend::GCPS3Compat)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::Public, Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(ApiVersion::V1Alpha)] api_version: ApiVersion,
    #[values(Cache::Disabled, Cache::Stat)] cache: Cache,
) {
    let Some(address) = storage_fixture.make_url(backend, access, "unittests/metadata/hello.txt") else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    let mut client = storage_fixture
        .metadata_client(backend, cache, interface, api_version)
        .await;

    let response = client
        .get_metadata(GetMetadataRequest {
            uri: address,
            user_metadata_keys: vec!["nosuchkey".to_string()],
        })
        .await
        .unwrap();

    assert!(
        response.user_metadata.is_empty(),
        "expected empty metadata for non-existent key, got: {:?}",
        response.user_metadata
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn update_metadata_missing_value(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3, Backend::Minio, Backend::GCPS3Compat)] backend: Backend,
    // only test GRPC because rest does not allow missing values
    #[values(Interface::Grpc)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(ApiVersion::V1Alpha)] api_version: ApiVersion,
    #[values(Cache::Disabled, Cache::Stat)] cache: Cache,
) {
    let mut fo_client = storage_fixture
        .file_object_client(backend, cache, Interface::Grpc, ApiVersion::V1Alpha)
        .await;

    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, ApiVersion::V1Alpha)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    let file = temp_dir.new_file();
    write_data_body(&file, &random_data(64), None, None, &mut fo_client).await;

    let mut client = storage_fixture
        .metadata_client(backend, cache, interface, api_version)
        .await;

    let err = client
        .update_metadata(UpdateMetadataRequest {
            uri: file.clone(),
            user_metadata_key: "k".to_string(),
            user_metadata: None,
            expected_etag: None,
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);

    temp_dir.cleanup().await;
}

#[rstest]
#[test_log::test(tokio::test)]
async fn update_metadata_overwrites_value(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3, Backend::Minio, Backend::GCPS3Compat)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(ApiVersion::V1Alpha)] api_version: ApiVersion,
    #[values(Cache::Disabled, Cache::Stat)] cache: Cache,
) {
    let mut fo_client = storage_fixture
        .file_object_client(backend, cache, Interface::Grpc, ApiVersion::V1Alpha)
        .await;

    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, ApiVersion::V1Alpha)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    let file = temp_dir.new_file();
    write_data_body(&file, &random_data(64), None, None, &mut fo_client).await;

    let mut client = storage_fixture
        .metadata_client(backend, cache, interface, api_version)
        .await;

    client
        .update_metadata(UpdateMetadataRequest {
            uri: file.clone(),
            user_metadata_key: "k".to_string(),
            user_metadata: Some(string_value("v1")),
            expected_etag: None,
        })
        .await
        .unwrap();

    client
        .update_metadata(UpdateMetadataRequest {
            uri: file.clone(),
            user_metadata_key: "k".to_string(),
            user_metadata: Some(string_value("v2")),
            expected_etag: None,
        })
        .await
        .unwrap();

    let response = client
        .get_metadata(GetMetadataRequest {
            uri: file.clone(),
            user_metadata_keys: vec!["k".to_string()],
        })
        .await
        .unwrap();

    let entry = response.user_metadata.get("k").expect("missing key 'k'");
    assert_eq!(extract_string_value(entry), "v2");

    temp_dir.cleanup().await;
}

#[rstest]
#[test_log::test(tokio::test)]
async fn update_metadata_multiple_keys(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3, Backend::Minio, Backend::GCPS3Compat)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(ApiVersion::V1Alpha)] api_version: ApiVersion,
    #[values(Cache::Disabled, Cache::Stat)] cache: Cache,
) {
    let mut fo_client = storage_fixture
        .file_object_client(backend, cache, Interface::Grpc, ApiVersion::V1Alpha)
        .await;

    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, ApiVersion::V1Alpha)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    let file = temp_dir.new_file();
    write_data_body(&file, &random_data(64), None, None, &mut fo_client).await;

    let mut client = storage_fixture
        .metadata_client(backend, cache, interface, api_version)
        .await;

    client
        .update_metadata(UpdateMetadataRequest {
            uri: file.clone(),
            user_metadata_key: "a".to_string(),
            user_metadata: Some(string_value("1")),
            expected_etag: None,
        })
        .await
        .unwrap();

    client
        .update_metadata(UpdateMetadataRequest {
            uri: file.clone(),
            user_metadata_key: "b".to_string(),
            user_metadata: Some(string_value("2")),
            expected_etag: None,
        })
        .await
        .unwrap();

    let response = client
        .get_metadata(GetMetadataRequest {
            uri: file.clone(),
            user_metadata_keys: vec!["a".to_string(), "b".to_string()],
        })
        .await
        .unwrap();

    assert_eq!(response.user_metadata.len(), 2);
    let entry_a = response.user_metadata.get("a").expect("missing key 'a'");
    assert_eq!(extract_string_value(entry_a), "1");
    let entry_b = response.user_metadata.get("b").expect("missing key 'b'");
    assert_eq!(extract_string_value(entry_b), "2");

    temp_dir.cleanup().await;
}

/// Get metadata using a resource identity URI (from write response).
/// The identity references the current head, so the operation should succeed.
/// Identity-URI handling is uniform across get/update/delete: a fresh identity
/// (matches current head) succeeds; a stale identity (head moved) fails with
/// FailedPrecondition. The two tests below run all three verbs through the same
/// scaffold and assert that contract.
#[derive(Debug, Clone, Copy, PartialEq)]
enum IdentityVerb {
    Get,
    Update,
    Delete,
}

async fn run_identity_op(
    verb: IdentityVerb,
    client: &mut Box<dyn storage_core_test::clients::traits::metadata::Metadata>,
    uri: String,
) -> Result<(), tonic::Status> {
    match verb {
        IdentityVerb::Get => client
            .get_metadata(GetMetadataRequest {
                uri,
                user_metadata_keys: vec![],
            })
            .await
            .map(|_| ()),
        IdentityVerb::Update => client
            .update_metadata(UpdateMetadataRequest {
                uri,
                user_metadata_key: "identitytest".to_string(),
                user_metadata: Some(string_value("works")),
                expected_etag: None,
            })
            .await
            .map(|_| ()),
        IdentityVerb::Delete => client
            .delete_metadata(DeleteMetadataRequest {
                uri,
                user_metadata_key: "nonexistentkey".to_string(),
                expected_etag: None,
            })
            .await
            .map(|_| ()),
    }
}

/// Fresh identity URI: the etag in the URI still matches the current head, so
/// the operation succeeds for all three verbs.
#[rstest]
#[case::case_01_get(IdentityVerb::Get)]
#[case::case_02_update(IdentityVerb::Update)]
#[case::case_03_delete(IdentityVerb::Delete)]
#[test_log::test(tokio::test)]
async fn metadata_with_fresh_identity(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3, Backend::Minio)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(ApiVersion::V1Alpha)] api_version: ApiVersion,
    #[values(Cache::Disabled)] cache: Cache,
    #[case] verb: IdentityVerb,
) {
    let mut fo_client = storage_fixture
        .file_object_client(backend, cache, Interface::Grpc, ApiVersion::V1Alpha)
        .await;

    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, ApiVersion::V1Alpha)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    let file = temp_dir.new_file();
    let identity = write_data_body(&file, &random_data(64), None, None, &mut fo_client)
        .await
        .unwrap();

    let mut client = storage_fixture
        .metadata_client(backend, cache, interface, api_version)
        .await;

    run_identity_op(verb, &mut client, identity.encoded_identity.clone())
        .await
        .unwrap();

    temp_dir.cleanup().await;
}

/// Stale identity URI: the head moved after the identity was issued. Update and
/// delete fail because their version-id check fires against the (now-different)
/// head; get on a non-versioned bucket fails for the same reason.
///
/// Caveat: on a *versioned* bucket, get with a stale identity actually succeeds
/// — the URI pins to a specific immutable version, so the etag still matches.
/// That combination is skipped explicitly below.
#[rstest]
#[case::case_01_get(IdentityVerb::Get)]
#[case::case_02_update(IdentityVerb::Update)]
#[case::case_03_delete(IdentityVerb::Delete)]
#[test_log::test(tokio::test)]
async fn metadata_with_stale_identity(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3, Backend::Minio)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey, Access::Versioned)] access: Access,
    #[values(ApiVersion::V1Alpha)] api_version: ApiVersion,
    #[values(Cache::Disabled)] cache: Cache,
    #[case] verb: IdentityVerb,
) {
    // Versioned + Get: the stale identity points to an immutable version that still
    // exists, so the read succeeds — not a precondition failure. Other verbs trip
    // the version-id mismatch check on the head and do fail.
    if verb == IdentityVerb::Get && access == Access::Versioned {
        debug!("Skipping: get on a versioned-bucket stale identity reads the immutable version");
        return;
    }
    let mut fo_client = storage_fixture
        .file_object_client(backend, cache, Interface::Grpc, ApiVersion::V1Alpha)
        .await;

    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, ApiVersion::V1Alpha)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    let file = temp_dir.new_file();
    let old_identity = write_data_body(&file, &random_data(64), None, None, &mut fo_client)
        .await
        .unwrap();

    // Overwrite the file to invalidate the identity etag.
    write_data_body(&file, &random_data(128), None, None, &mut fo_client).await;

    let mut client = storage_fixture
        .metadata_client(backend, cache, interface, api_version)
        .await;

    let err = run_identity_op(verb, &mut client, old_identity.encoded_identity.clone())
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::FailedPrecondition);

    temp_dir.cleanup().await;
}

// ---------------------------------------------------------------------------
// Metadata key compatibility tests (special characters, casing)
// ---------------------------------------------------------------------------

/// Verify that metadata keys with special characters round-trip correctly on all
/// backends. Characters that are invalid for a backend's native key format are
/// transparently hex-encoded with the `nv_key_` prefix.
#[rstest]
#[case::case_01_content_type("content-type", "val1")]
#[case::case_02_last_modified("last.modified", "val2")]
#[case::case_03_numeric_author("1st-author", "val3")]
#[case::case_04_underscore("my_tag", "val4")]
#[case::case_05_nv_prefix("nv_key_custom", "val5")]
#[case::case_06_space("my key", "val6")]
#[case::case_07_colon("my:key", "val7")]
#[case::case_08_slash("my/key", "val8")]
#[case::case_09_equals("my=key", "val9")]
#[case::case_10_control_char("my\x01key", "val10")]
#[test_log::test(tokio::test)]
async fn metadata_key_special_chars_roundtrip(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3, Backend::Minio, Backend::GCPS3Compat)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(ApiVersion::V1Alpha)] api_version: ApiVersion,
    #[values(Cache::Disabled)] cache: Cache,
    #[case] key: &str,
    #[case] value: &str,
) {
    // nginx proxy silently drops HTTP headers with non-tchar characters
    // (periods, spaces, delimiters, etc.). S3 metadata keys become
    // x-amz-meta-{encoded_key} headers — if the encoded key still contains
    // non-tchar chars, the proxy strips it, causing a signature mismatch.
    // Azure is unaffected because all special chars are hex-encoded.
    // Keys that are encoded on S3 (spaces, colons, etc.) produce hex-only
    // encoded forms that ARE valid through the proxy, so we only skip keys
    // that S3 passes through natively but nginx rejects (periods).
    if std::env::var("UCC_PROXY").is_ok() && key.contains('.') && !matches!(backend, Backend::Azure) {
        debug!("Skipping: nginx proxy strips headers with periods in key names");
        return;
    }

    let mut fo_client = storage_fixture
        .file_object_client(backend, cache, Interface::Grpc, ApiVersion::V1Alpha)
        .await;

    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, ApiVersion::V1Alpha)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    let file = temp_dir.new_file();
    write_data_body(&file, &random_data(64), None, None, &mut fo_client).await;

    let mut client = storage_fixture
        .metadata_client(backend, cache, interface, api_version)
        .await;

    // Write metadata with the test key
    client
        .update_metadata(UpdateMetadataRequest {
            uri: file.clone(),
            user_metadata_key: key.to_string(),
            user_metadata: Some(string_value(value)),
            expected_etag: None,
        })
        .await
        .unwrap();

    // Read it back by key
    let response = client
        .get_metadata(GetMetadataRequest {
            uri: file.clone(),
            user_metadata_keys: vec![key.to_string()],
        })
        .await
        .unwrap();

    assert_eq!(response.user_metadata.len(), 1, "expected exactly one key");
    let entry = response
        .user_metadata
        .get(&key.to_lowercase())
        .unwrap_or_else(|| panic!("missing key '{}'", key.to_lowercase()));
    assert_eq!(extract_string_value(entry), value);

    temp_dir.cleanup().await;
}

/// Verify that a mix of key types (valid identifiers, hyphenated, prefix-colliding)
/// on the same object all round-trip correctly, and that deleting one leaves others intact.
#[rstest]
#[test_log::test(tokio::test)]
async fn metadata_key_mixed_keys(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3, Backend::Minio, Backend::GCPS3Compat)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(ApiVersion::V1Alpha)] api_version: ApiVersion,
    #[values(Cache::Disabled, Cache::Stat)] cache: Cache,
) {
    let mut fo_client = storage_fixture
        .file_object_client(backend, cache, Interface::Grpc, ApiVersion::V1Alpha)
        .await;

    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, ApiVersion::V1Alpha)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    let file = temp_dir.new_file();
    write_data_body(&file, &random_data(64), None, None, &mut fo_client).await;

    let mut client = storage_fixture
        .metadata_client(backend, cache, interface, api_version)
        .await;

    let keys_and_values = [
        ("content-type", "text/plain"),
        ("my_tag", "hello"),
        ("nv_key_reserved", "reserved"),
    ];

    // Write all keys
    for (key, value) in &keys_and_values {
        client
            .update_metadata(UpdateMetadataRequest {
                uri: file.clone(),
                user_metadata_key: key.to_string(),
                user_metadata: Some(string_value(value)),
                expected_etag: None,
            })
            .await
            .unwrap();
    }

    // Read all keys back
    let response = client
        .get_metadata(GetMetadataRequest {
            uri: file.clone(),
            user_metadata_keys: vec![],
        })
        .await
        .unwrap();

    for (key, value) in &keys_and_values {
        let entry = response
            .user_metadata
            .get(&key.to_lowercase())
            .unwrap_or_else(|| panic!("missing key '{}'", key));
        assert_eq!(extract_string_value(entry), *value);
    }

    // Delete the encoded key "content-type"
    client
        .delete_metadata(DeleteMetadataRequest {
            uri: file.clone(),
            user_metadata_key: "content-type".to_string(),
            expected_etag: None,
        })
        .await
        .unwrap();

    // Read all keys again — "content-type" should be gone
    let response = client
        .get_metadata(GetMetadataRequest {
            uri: file.clone(),
            user_metadata_keys: vec![],
        })
        .await
        .unwrap();

    assert!(
        !response.user_metadata.contains_key("content-type"),
        "content-type should have been deleted"
    );
    assert_eq!(
        extract_string_value(response.user_metadata.get("my_tag").expect("missing my_tag")),
        "hello"
    );
    assert_eq!(
        extract_string_value(
            response
                .user_metadata
                .get("nv_key_reserved")
                .expect("missing nv_key_reserved")
        ),
        "reserved"
    );

    temp_dir.cleanup().await;
}

/// Verify that metadata keys are case-insensitive: writing "TestKey" and reading
/// with "testkey" should return the value. S3 lowercases keys natively; Azure does
/// so via our encoding layer.
#[rstest]
#[test_log::test(tokio::test)]
async fn metadata_key_case_normalization(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3, Backend::Minio, Backend::GCPS3Compat)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(ApiVersion::V1Alpha)] api_version: ApiVersion,
    #[values(Cache::Disabled, Cache::Stat)] cache: Cache,
) {
    let mut fo_client = storage_fixture
        .file_object_client(backend, cache, Interface::Grpc, ApiVersion::V1Alpha)
        .await;

    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, ApiVersion::V1Alpha)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    let file = temp_dir.new_file();
    write_data_body(&file, &random_data(64), None, None, &mut fo_client).await;

    let mut client = storage_fixture
        .metadata_client(backend, cache, interface, api_version)
        .await;

    // Write with mixed case
    client
        .update_metadata(UpdateMetadataRequest {
            uri: file.clone(),
            user_metadata_key: "TestKey".to_string(),
            user_metadata: Some(string_value("case_test")),
            expected_etag: None,
        })
        .await
        .unwrap();

    // Read with lowercase filter — should match
    let response = client
        .get_metadata(GetMetadataRequest {
            uri: file.clone(),
            user_metadata_keys: vec!["testkey".to_string()],
        })
        .await
        .unwrap();

    assert_eq!(response.user_metadata.len(), 1);
    let entry = response.user_metadata.get("testkey").expect("missing key 'testkey'");
    assert_eq!(extract_string_value(entry), "case_test");

    // Read with original mixed-case filter — should also match
    let response = client
        .get_metadata(GetMetadataRequest {
            uri: file.clone(),
            user_metadata_keys: vec!["TestKey".to_string()],
        })
        .await
        .unwrap();

    assert_eq!(response.user_metadata.len(), 1);
    let entry = response
        .user_metadata
        .get("testkey")
        .expect("missing key 'testkey' via mixed-case filter");
    assert_eq!(extract_string_value(entry), "case_test");

    temp_dir.cleanup().await;
}

// ---------------------------------------------------------------------------
// Metadata key remapper layer
// ---------------------------------------------------------------------------

/// Verify that the metadata_key_remapper layer translates the Nucleus hash key:
/// the client uses `nv-omniverse-nucleus-hash` while the backend stores it under
/// `nv_omniverse_nucleus_hash`.
///
/// Test strategy:
/// 1. Write the client key — the remapper rewrites it to the storage key on the way
///    to the backend.
/// 2. Read the client key — the remapper rewrites the backend's storage key back to
///    the client key in the response.
/// 3. Write the literal storage key. It is not in the forward map, so it passes
///    through and lands on the SAME backend slot the first write created. A
///    subsequent read of the client key must return the second write's value,
///    which is only possible if the first write was actually translated to the
///    storage key in the backend.
#[rstest]
#[test_log::test(tokio::test)]
async fn metadata_key_remapper_nucleus_hash(
    storage_fixture: &StorageFixture,
    #[values(Backend::Azure, Backend::S3, Backend::Minio, Backend::GCPS3Compat)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(ApiVersion::V1Alpha)] api_version: ApiVersion,
    #[values(Cache::Disabled)] cache: Cache,
) {
    const CLIENT_KEY: &str = "nv-omniverse-nucleus-hash";
    const STORAGE_KEY: &str = "nv_omniverse_nucleus_hash";

    let mut fo_client = storage_fixture
        .file_object_client(backend, cache, Interface::Grpc, ApiVersion::V1Alpha)
        .await;

    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, ApiVersion::V1Alpha)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    let file = temp_dir.new_file();
    write_data_body(&file, &random_data(64), None, None, &mut fo_client).await;

    let mut client = storage_fixture
        .metadata_client(backend, cache, interface, api_version)
        .await;

    // (1) Write using the client key. The remapper should translate to the storage key.
    client
        .update_metadata(UpdateMetadataRequest {
            uri: file.clone(),
            user_metadata_key: CLIENT_KEY.to_string(),
            user_metadata: Some(string_value("v1")),
            expected_etag: None,
        })
        .await
        .unwrap();

    // (2) Read using the client key. Response must come back keyed by the client key.
    let response = client
        .get_metadata(GetMetadataRequest {
            uri: file.clone(),
            user_metadata_keys: vec![CLIENT_KEY.to_string()],
        })
        .await
        .unwrap();
    assert_eq!(response.user_metadata.len(), 1);
    let entry = response
        .user_metadata
        .get(CLIENT_KEY)
        .unwrap_or_else(|| panic!("missing client key '{CLIENT_KEY}'"));
    assert_eq!(extract_string_value(entry), "v1");
    assert!(
        !response.user_metadata.contains_key(STORAGE_KEY),
        "response should not surface the storage key '{STORAGE_KEY}'"
    );

    // (3) Write using the storage key directly. Not in the forward map, so it passes
    // through and lands on the same backend slot. The next read of the client key
    // must reflect this overwrite.
    client
        .update_metadata(UpdateMetadataRequest {
            uri: file.clone(),
            user_metadata_key: STORAGE_KEY.to_string(),
            user_metadata: Some(string_value("v2")),
            expected_etag: None,
        })
        .await
        .unwrap();

    let response = client
        .get_metadata(GetMetadataRequest {
            uri: file.clone(),
            user_metadata_keys: vec![CLIENT_KEY.to_string()],
        })
        .await
        .unwrap();
    let entry = response
        .user_metadata
        .get(CLIENT_KEY)
        .unwrap_or_else(|| panic!("missing client key '{CLIENT_KEY}' after storage-key overwrite"));
    assert_eq!(
        extract_string_value(entry),
        "v2",
        "client-key write and storage-key write must collide on the same backend slot"
    );

    // (4) Get-all (empty key list) must surface the entry under the client key, not the storage key.
    let response = client
        .get_metadata(GetMetadataRequest {
            uri: file.clone(),
            user_metadata_keys: vec![],
        })
        .await
        .unwrap();
    assert!(
        response.user_metadata.contains_key(CLIENT_KEY),
        "get-all response should contain client key '{CLIENT_KEY}'"
    );
    assert!(
        !response.user_metadata.contains_key(STORAGE_KEY),
        "get-all response should not leak storage key '{STORAGE_KEY}'"
    );

    // (5) Delete using the client key. The remapper translates and the backend slot is removed.
    client
        .delete_metadata(DeleteMetadataRequest {
            uri: file.clone(),
            user_metadata_key: CLIENT_KEY.to_string(),
            expected_etag: None,
        })
        .await
        .unwrap();

    let response = client
        .get_metadata(GetMetadataRequest {
            uri: file.clone(),
            user_metadata_keys: vec![CLIENT_KEY.to_string()],
        })
        .await
        .unwrap();
    assert!(
        response.user_metadata.is_empty(),
        "expected metadata to be deleted, got: {:?}",
        response.user_metadata
    );

    temp_dir.cleanup().await;
}

#[rstest]
#[test_log::test(tokio::test)]
async fn update_metadata_batch_exceeds_cap_returns_invalid_argument(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(Cache::Disabled)] cache: Cache,
) {
    let api_version = ApiVersion::V1Alpha;
    let mut fo_client = storage_fixture
        .file_object_client(backend, cache, Interface::Grpc, api_version)
        .await;

    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, api_version)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    let file = temp_dir.new_file();
    write_data_body(&file, &random_data(64), None, None, &mut fo_client).await;

    let mut client = storage_fixture
        .metadata_client(backend, cache, interface, api_version)
        .await;

    // Build a request exactly one entry over the documented cap (256). The
    // server must reject at the boundary before processing any entries — the
    // backend never sees this request. (We don't pair this with an "at the
    // cap" success case because per-URI serialization would make 256
    // sequential HEAD-merge-PUT calls take minutes; the fan-out path itself
    // is covered by the other batch tests.)
    let requests: Vec<UpdateMetadataRequest> = (0..257u32)
        .map(|i| UpdateMetadataRequest {
            uri: file.clone(),
            user_metadata_key: format!("key_{i}"),
            user_metadata: Some(string_value(&format!("value_{i}"))),
            expected_etag: None,
        })
        .collect();

    let result = client
        .update_metadata_batch(UpdateMetadataBatchRequest { requests })
        .await;
    let err = result.expect_err("batch above cap must be rejected");
    assert_eq!(err.code(), Code::InvalidArgument);

    temp_dir.cleanup().await;
}

#[rstest]
#[test_log::test(tokio::test)]
async fn update_metadata_batch_reports_not_found_for_missing_uri(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
    #[values(Cache::Disabled)] cache: Cache,
) {
    let api_version = ApiVersion::V1Alpha;
    let mut fo_client = storage_fixture
        .file_object_client(backend, cache, Interface::Grpc, api_version)
        .await;

    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, api_version)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    let existing = temp_dir.new_file();
    write_data_body(&existing, &random_data(64), None, None, &mut fo_client).await;

    let missing = temp_dir.new_file(); // never written

    let mut client = storage_fixture
        .metadata_client(backend, cache, interface, api_version)
        .await;

    let request = UpdateMetadataBatchRequest {
        requests: vec![
            UpdateMetadataRequest {
                uri: existing.clone(),
                user_metadata_key: "ok".to_string(),
                user_metadata: Some(string_value("yes")),
                expected_etag: None,
            },
            UpdateMetadataRequest {
                uri: missing.clone(),
                user_metadata_key: "ok".to_string(),
                user_metadata: Some(string_value("no")),
                expected_etag: None,
            },
        ],
    };

    let mut stream = client.update_metadata_batch(request).await.unwrap();
    let mut results = Vec::new();
    while let Some(page) = stream.try_next().await.unwrap() {
        results.extend(page.results);
    }
    results.sort_by_key(|result| result.request_index);

    assert_eq!(results.len(), 2);
    assert_matches!(
        results[0].outcome.as_ref().unwrap(),
        update_metadata_batch_entry_result::Outcome::Success(_)
    );
    assert_matches!(
        results[1].outcome.as_ref().unwrap(),
        update_metadata_batch_entry_result::Outcome::Error(error)
            if error.code == update_metadata_batch_entry_error::Code::NotFound as i32
    );

    temp_dir.cleanup().await;
}

#[rstest]
#[test_log::test(tokio::test)]
async fn update_metadata_batch_invalidates_cache(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::PrivateKey)] access: Access,
) {
    // With the cache layer enabled, a batch write through the gRPC/REST
    // boundary fans out per-entry through `update_metadata`, which the cache
    // layer invalidates. This test pins that invariant: after a batch write,
    // read-back through the cache must observe the new values, not stale ones.
    //
    // Per the proto contract, same-URI batches are unsafe (HEAD-merge-PUT
    // backends race), so this test uses distinct URIs per entry to keep the
    // invalidation assertion well-defined.
    let cache = Cache::Stat;
    let api_version = ApiVersion::V1Alpha;
    let mut fo_client = storage_fixture
        .file_object_client(backend, cache, Interface::Grpc, api_version)
        .await;

    let Some(mut temp_dir) = storage_fixture
        .temp_directory(backend, access, cache, api_version)
        .await
    else {
        debug!("Skipping test for {:?} {:?}", backend, access);
        return;
    };

    let file_a = temp_dir.new_file();
    let file_b = temp_dir.new_file();
    write_data_body(&file_a, &random_data(64), None, None, &mut fo_client).await;
    write_data_body(&file_b, &random_data(64), None, None, &mut fo_client).await;

    let mut client = storage_fixture
        .metadata_client(backend, cache, interface, api_version)
        .await;

    // Seed the cache by reading each URI's metadata first (will be empty).
    for file in [&file_a, &file_b] {
        let _ = client
            .get_metadata(GetMetadataRequest {
                uri: file.clone(),
                user_metadata_keys: vec!["seed_key".to_string()],
            })
            .await
            .unwrap();
    }

    // Now batch-write a new key to each URI.
    let request = UpdateMetadataBatchRequest {
        requests: vec![
            UpdateMetadataRequest {
                uri: file_a.clone(),
                user_metadata_key: "cache_key_a".to_string(),
                user_metadata: Some(string_value("a-value")),
                expected_etag: None,
            },
            UpdateMetadataRequest {
                uri: file_b.clone(),
                user_metadata_key: "cache_key_b".to_string(),
                user_metadata: Some(string_value("b-value")),
                expected_etag: None,
            },
        ],
    };
    let mut stream = client.update_metadata_batch(request).await.unwrap();
    while stream.try_next().await.unwrap().is_some() {}

    // Read-back must observe the new values; if cache invalidation didn't
    // fire per-entry, we'd see an empty/stale result.
    let response_a = client
        .get_metadata(GetMetadataRequest {
            uri: file_a.clone(),
            user_metadata_keys: vec!["cache_key_a".to_string()],
        })
        .await
        .unwrap();
    assert_value_eq(
        response_a
            .user_metadata
            .get("cache_key_a")
            .unwrap()
            .value
            .as_ref()
            .unwrap(),
        &string_value("a-value"),
    );

    let response_b = client
        .get_metadata(GetMetadataRequest {
            uri: file_b.clone(),
            user_metadata_keys: vec!["cache_key_b".to_string()],
        })
        .await
        .unwrap();
    assert_value_eq(
        response_b
            .user_metadata
            .get("cache_key_b")
            .unwrap()
            .value
            .as_ref()
            .unwrap(),
        &string_value("b-value"),
    );

    temp_dir.cleanup().await;
}
