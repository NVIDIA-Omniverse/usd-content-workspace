// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashSet;

use futures::{stream::StreamExt, TryStreamExt};
use hex_literal::hex;
use rstest::*;
use sha2::{Digest, Sha256};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Code, Status};
use tracing::{debug, trace};

use reqwest::header::{HeaderMap, HeaderName};
use storage_core::traits::fileobject::v1alpha::*;
use storage_core::traits::versioning::v1alpha::{EnumerateVersionsRequest, EnumerateVersionsResponse};
use storage_core_integration_tests::cdn_storage_fixture;
use storage_core_integration_tests::fixture::*;
use storage_core_integration_tests::{assert_method_metric, storage_fixture};
use storage_core_test::identity::assert_identity_has_object_identifier_parameters;
use storage_core_test::service::{ApiVersion, Interface};

/// Extract the terminal error/status from an `enumerate_versions` result.
///
/// gRPC and REST deliver a terminal error differently: gRPC surfaces it as an outer
/// `Err(Status)` from `enumerate_versions().await`, while the REST client always returns
/// `Ok(stream)` and delivers the error as an item inside the stream (typically the first
/// and only item). This helper normalises both shapes: it returns the first error status
/// seen (upfront or streamed), or `None` if the enumeration completed without any error.
async fn terminal_enumerate_error(
    result: Result<ReceiverStream<Result<EnumerateVersionsResponse, Status>>, Status>,
) -> Option<Status> {
    match result {
        Err(status) => Some(status),
        Ok(stream) => stream.collect::<Vec<_>>().await.into_iter().find_map(Result::err),
    }
}

// Minio doesn't support the list_object_versions API used by this interface, so don't test versions on Minio.
#[rstest]
#[case::case_01_lorem_ipsum("unittests/versioning/Lorem%20ipsum.txt", 3)]
#[test_log::test(tokio::test)]
async fn versioning(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Cache::Stat, Cache::Disabled)] cache: Cache,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[case] base_address: &str,
    #[case] expected_count: usize,
) {
    let Some(base_address) = storage_fixture.make_url(backend, Access::Versioned, base_address) else {
        debug!(
            "Skipping test for {:?} {:?} {:?}",
            backend,
            Access::Versioned,
            base_address
        );
        return;
    };
    let mut client = storage_fixture
        .versioning_client(backend, cache, interface, api_version)
        .await;
    let resource_address = base_address;
    let request = EnumerateVersionsRequest {
        resource_address: resource_address.clone(),
    };

    let enum_versions_results: Vec<_> = client
        .enumerate_versions(request)
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap()
        .into_iter()
        .flat_map(|response| response.items)
        .collect();
    assert_eq!(enum_versions_results.len(), expected_count);

    for version_info in enum_versions_results.iter() {
        assert_identity_has_object_identifier_parameters(
            version_info
                .resource_info
                .as_ref()
                .expect("resource info is missing")
                .resource_identity
                .as_ref()
                .expect("resource identity is missing")
                .encoded_identity
                .as_str(),
            &resource_address,
            true,
        );

        let identity = url::Url::parse(
            version_info
                .resource_info
                .as_ref()
                .expect("resource info is missing")
                .resource_identity
                .as_ref()
                .expect("resource identity is missing")
                .encoded_identity
                .as_str(),
        )
        .unwrap();

        let version_id = identity
            .query_pairs()
            .find(|(key, _)| key == "version-id")
            .map(|(_, value)| value.to_string())
            .expect("version-id query parameter is missing");
        assert!(!version_id.is_empty(), "version-id query parameter should not be empty");
    }

    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.requests",
        "enumerate_versions",
        ["result" => "Ok"]
    );
    assert_method_metric!(storage_fixture, backend, api_version, "storage.sdk.requests", "enumerate_versions", ["result" => "Ok"]);
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.request.duration",
        "enumerate_versions",
        [],
        1
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.sdk.request.duration",
        "enumerate_versions",
        [],
        1
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.enumeration.items",
        "enumerate_versions",
        [],
        1
    );
}

#[rstest]
#[case::case_01_s3_lorem_ipsum(Backend::S3, "unittests/versioning/Lorem%20ipsum.txt", 3, 
        HashSet::from([
            hex!("4998f2f14a77240c7724da8d4a19e01815c9019be819b81bfd5e5059caac4d84"),
            hex!("2925fb47a5c8516931af8582552a07414714361785e0be1383e0a69a33dff66e"),
            hex!("a77a57f85809df86df784a927c7abec723847e9e921219a3d83c4d436e7404a6")
        ])
    )
]
#[case::case_02_azure_lorem_ipsum(Backend::Azure, "unittests/versioning/Lorem%20ipsum.txt", 3, 
        HashSet::from([
            hex!("2925fb47a5c8516931af8582552a07414714361785e0be1383e0a69a33dff66e"),
            hex!("78f5b9115f4924f23bb7dbe646e67dd297479f7711bb06c55360377a9927cb7e"),
            hex!("a77a57f85809df86df784a927c7abec723847e9e921219a3d83c4d436e7404a6")
        ])
    )
]
#[test_log::test(tokio::test)]
async fn read_versions(
    storage_fixture: &StorageFixture,
    #[case] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[case] base_address: &str,
    #[case] expected_count: usize,
    #[case] expected_hashes: HashSet<[u8; 32]>,
) {
    let Some(base_address) = storage_fixture.make_url(backend, Access::Versioned, base_address) else {
        debug!(
            "Skipping test for {:?} {:?} {:?}",
            backend,
            Access::Versioned,
            base_address
        );
        return;
    };
    let mut client = storage_fixture
        .versioning_client(backend, Cache::Disabled, interface, api_version)
        .await;
    let resource_address = base_address;
    let request = EnumerateVersionsRequest {
        resource_address: resource_address.clone(),
    };

    let enum_versions_stream = client.enumerate_versions(request).await.unwrap();

    let enum_versions_results = enum_versions_stream
        .filter_map(|x| async move { x.ok() })
        .collect::<Vec<_>>()
        .await;
    assert_eq!(enum_versions_results[0].items.len(), expected_count);

    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.requests",
        "enumerate_versions",
        ["result" => "Ok"]
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.enumeration.items",
        "enumerate_versions",
        [],
        1
    );

    let objects: HashSet<_> = enum_versions_results
        .iter()
        .flat_map(|response| {
            response.items.iter().map(|item| {
                item.resource_info
                    .as_ref()
                    .unwrap()
                    .resource_identity
                    .as_ref()
                    .unwrap()
                    .encoded_identity
                    .clone()
            })
        })
        .collect();
    let mut file_object_client = storage_fixture
        .file_object_client(backend, Cache::Disabled, interface, api_version)
        .await;
    let mut hashes: HashSet<[u8; 32]> = HashSet::new();
    for r in objects {
        let request = ReadRequest {
            resource_identity: Some(ResourceIdentity {
                encoded_identity: r.clone(),
            }),
            download_preference: Some(DownloadPreference::Body.into()),
        };
        let mut read_stream = file_object_client.read(request).await.unwrap();
        debug!(?read_stream);

        // metadata first, save the resource info so we can call read() afterward
        let mut hasher = Sha256::new();

        while let Some(Ok(read_response)) = read_stream.next().await {
            trace!(?read_response);

            match read_response.reply_type {
                Some(read_response::ReplyType::Chunk(chunk)) => {
                    trace!(?chunk);
                    hasher.update(chunk.chunk);
                },
                _ => debug!("Read chunk expected!"),
            }
        }

        let result = hasher.finalize();
        hashes.insert(result.into());
    }
    assert_eq!(hashes, expected_hashes);
}

// This folder contains a file named "base_file" with two versions and another file named "base_file_backup" with one version,
// and it ensures that only the versions of "base_file" are returned.
#[rstest]
#[case::case_01_base_file("unittests/versioning/base_file", 2)]
#[test_log::test(tokio::test)]
async fn enumerate_versions_matches_exactly(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Cache::Stat, Cache::Disabled)] cache: Cache,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[case] base_address: &str,
    #[case] expected_count: usize,
) {
    let Some(resource_address) = storage_fixture.make_url(backend, Access::Versioned, base_address) else {
        debug!(
            "Skipping test for {:?} {:?} {:?}",
            backend,
            Access::Versioned,
            base_address
        );
        return;
    };
    let mut client = storage_fixture
        .versioning_client(backend, cache, interface, api_version)
        .await;

    let request = EnumerateVersionsRequest {
        resource_address: resource_address.clone(),
    };
    let enum_stream = client.enumerate_versions(request).await.unwrap();
    trace!(?enum_stream);

    let enum_results = enum_stream
        .filter_map(|x| async move { x.ok() })
        .collect::<Vec<_>>()
        .await;
    trace!(?enum_results);
    assert_eq!(enum_results.len(), 1);
    assert_eq!(enum_results[0].items.len(), expected_count);

    for item in enum_results[0].items.iter() {
        let encoded_identity = &item
            .resource_info
            .as_ref()
            .unwrap()
            .resource_identity
            .as_ref()
            .unwrap()
            .encoded_identity;

        let url = url::Url::parse(encoded_identity).expect("encoded_identity should be a valid URL");
        assert!(
            url.path().ends_with(&base_address),
            "URL path '{}' should end with '{base_address}'",
            url.path()
        );

        if let Some(resource_address) = &item.resource_address {
            let url = url::Url::parse(resource_address).expect("resource_address should be a valid URL");
            assert!(
                url.path().ends_with(&base_address),
                "URL path '{}' should end with '{base_address}'",
                url.path()
            );
        }
    }

    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.requests",
        "enumerate_versions",
        ["result" => "Ok"]
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.request.duration",
        "enumerate_versions",
        [],
        1
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.enumeration.items",
        "enumerate_versions",
        [],
        1
    );
}

#[rstest]
#[case::case_01_missing_prefix("unittests/versioning/nonexistent")]
#[case::case_02_missing_prefix_slash("unittests/versioning/nonexistent/")]
#[case::case_03_missing_file("unittests/versioning/thisfiledoesnotexist.txt")]
#[test_log::test(tokio::test)]
async fn enumerate_versions_nonexistent_folder(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure, Backend::Minio)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(Access::Public, Access::PrivateKey)] access: Access,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[case] base_address: &str,
) {
    let Some(base_address) = storage_fixture.make_url(backend, access, base_address) else {
        debug!("Skipping test for {:?} {:?} {:?}", backend, access, base_address);
        return;
    };

    let mut client = storage_fixture
        .versioning_client(backend, Cache::Disabled, interface, api_version)
        .await;

    let request = EnumerateVersionsRequest {
        resource_address: base_address.to_string(),
    };
    // A nonexistent path is expected to yield NOT_FOUND. Versioning state is
    // operator-configured (no runtime detection), and the test fixtures leave it
    // unset, so no other error is legitimate here. The terminal error may arrive
    // upfront (gRPC) or as a stream item (REST); the helper normalises both.
    let err = terminal_enumerate_error(client.enumerate_versions(request).await)
        .await
        .expect("expected a terminal NOT_FOUND from enumerate_versions on a nonexistent path");
    assert_eq!(
        err.code(),
        Code::NotFound,
        "Expected NOT_FOUND, got {:?}: {}",
        err.code(),
        err.message()
    );
}

#[rstest]
#[test_log::test(tokio::test)]
async fn enumerate_versions_empty_top_level(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure, Backend::Minio)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
) {
    let Some(base_address) = storage_fixture.make_url(backend, Access::Empty, "") else {
        debug!("Skipping test for {:?}", backend);
        return;
    };

    let mut client = storage_fixture
        .versioning_client(backend, Cache::Disabled, interface, api_version)
        .await;

    let request = EnumerateVersionsRequest {
        resource_address: base_address.to_string(),
    };
    // An empty bucket yields an empty listing. Versioning state is operator-configured
    // (no runtime detection) and the fixtures leave it unset, so FAILED_PRECONDITION —
    // or any other error, upfront or streamed — is a real failure here.
    let stream = client
        .enumerate_versions(request)
        .await
        .unwrap_or_else(|e| panic!("unexpected upfront error from enumerate_versions on empty bucket: {e}"));
    let responses = stream.collect::<Vec<_>>().await;
    let mut saw_empty_page = false;
    for response in responses {
        match response {
            Ok(page) => {
                saw_empty_page = true;
                assert!(page.items.is_empty(), "expected no version items in an empty bucket");
            },
            Err(e) => panic!("unexpected streamed error from enumerate_versions on empty bucket: {e}"),
        }
    }
    assert!(
        saw_empty_page,
        "expected at least one (empty) response page from enumerate_versions"
    );
}

// Versioning state is operator-configured only (no runtime detection). With the
// config unset — as on every test fixture — repeated enumerations on a
// non-versioned bucket must keep reaching the backend and must never
// short-circuit with FAILED_PRECONDITION. This pins the removal of the
// auto-detection latch on every backend that previously had inference code
// (S3/Minio shared one implementation, Azure had its own): reintroducing
// either would flip a later call here.
#[rstest]
#[test_log::test(tokio::test)]
async fn enumerate_versions_never_short_circuits_by_default(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Minio, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
) {
    let Some(resource_address) =
        storage_fixture.make_url(backend, Access::PrivateKey, "unittests/readFile/Lorem ipsum.txt")
    else {
        debug!("Skipping test for {:?} {:?}", backend, Access::PrivateKey);
        return;
    };

    let mut client = storage_fixture
        .versioning_client(backend, Cache::Disabled, interface, api_version)
        .await;

    let make_request = || EnumerateVersionsRequest {
        resource_address: resource_address.clone(),
    };

    for i in 0..3 {
        let err = terminal_enumerate_error(client.enumerate_versions(make_request()).await).await;
        assert_ne!(
            err.as_ref().map(Status::code),
            Some(Code::FailedPrecondition),
            "enumerate #{i} short-circuited with FAILED_PRECONDITION; versioning must not be auto-detected"
        );
    }
}

// Positive companion: on a genuinely versioned bucket repeated enumerations keep
// returning version items — none may error or short-circuit.
#[rstest]
#[test_log::test(tokio::test)]
async fn enumerate_versions_versioned_repeated_calls_succeed(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure)] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
) {
    let Some(resource_address) =
        storage_fixture.make_url(backend, Access::Versioned, "unittests/versioning/Lorem%20ipsum.txt")
    else {
        debug!("Skipping test for {:?} {:?}", backend, Access::Versioned);
        return;
    };

    let mut client = storage_fixture
        .versioning_client(backend, Cache::Disabled, interface, api_version)
        .await;

    let make_request = || EnumerateVersionsRequest {
        resource_address: resource_address.clone(),
    };

    // Enumerate several times over the versioned object; every call must succeed.
    for i in 0..3 {
        let stream = client
            .enumerate_versions(make_request())
            .await
            .unwrap_or_else(|e| panic!("enumerate #{i} failed upfront: {e}"));
        let items: Vec<_> = stream
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .map(|r| r.unwrap_or_else(|e| panic!("enumerate #{i} streamed an error: {e}")))
            .flat_map(|response| response.items)
            .collect();
        assert!(
            !items.is_empty(),
            "enumerate #{i} on a versioned object should return version items"
        );
    }
}

#[rstest]
#[case::case_01_s3_lorem_ipsum(Backend::S3, "unittests/versioning/Lorem%20ipsum.txt", 3,
        HashSet::from([
            hex!("4998f2f14a77240c7724da8d4a19e01815c9019be819b81bfd5e5059caac4d84"),
            hex!("2925fb47a5c8516931af8582552a07414714361785e0be1383e0a69a33dff66e"),
            hex!("a77a57f85809df86df784a927c7abec723847e9e921219a3d83c4d436e7404a6")
        ])
    )
]
#[case::case_02_azure_lorem_ipsum(Backend::Azure, "unittests/versioning/Lorem%20ipsum.txt", 3,
        HashSet::from([
            hex!("2925fb47a5c8516931af8582552a07414714361785e0be1383e0a69a33dff66e"),
            hex!("78f5b9115f4924f23bb7dbe646e67dd297479f7711bb06c55360377a9927cb7e"),
            hex!("a77a57f85809df86df784a927c7abec723847e9e921219a3d83c4d436e7404a6")
        ])
    )
]
#[test_log::test(tokio::test)]
async fn read_versions_cdn(
    #[from(cdn_storage_fixture)] storage_fixture: &StorageFixture,
    #[case] backend: Backend,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
    #[values(ApiVersion::V1Alpha, ApiVersion::V1Beta)] api_version: ApiVersion,
    #[case] base_address: &str,
    #[case] expected_count: usize,
    #[case] expected_hashes: HashSet<[u8; 32]>,
) {
    let Some(base_address) = storage_fixture.make_url(backend, Access::Versioned, base_address) else {
        debug!(
            "Skipping test for {:?} {:?} {:?}",
            backend,
            Access::Versioned,
            base_address
        );
        return;
    };
    let mut client = storage_fixture
        .versioning_client(backend, Cache::Disabled, interface, api_version)
        .await;
    let resource_address = base_address.clone();
    let request = EnumerateVersionsRequest {
        resource_address: resource_address.clone(),
    };

    let enum_versions_stream = client.enumerate_versions(request).await.unwrap();

    let enum_versions_results = enum_versions_stream
        .filter_map(|x| async move { x.ok() })
        .collect::<Vec<_>>()
        .await;
    assert_eq!(enum_versions_results[0].items.len(), expected_count);

    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.requests",
        "enumerate_versions",
        ["result" => "Ok"]
    );
    assert_method_metric!(
        storage_fixture,
        backend,
        api_version,
        "storage.enumeration.items",
        "enumerate_versions",
        [],
        1
    );

    let objects: HashSet<_> = enum_versions_results
        .iter()
        .flat_map(|response| {
            response.items.iter().map(|item| {
                item.resource_info
                    .as_ref()
                    .unwrap()
                    .resource_identity
                    .as_ref()
                    .unwrap()
                    .encoded_identity
                    .clone()
            })
        })
        .collect();
    debug!(?objects);

    let mut file_object_client = storage_fixture
        .file_object_client(backend, Cache::Disabled, interface, api_version)
        .await;
    let mut hashes: HashSet<[u8; 32]> = HashSet::new();
    for r in objects {
        let request = ReadRequest {
            resource_identity: Some(ResourceIdentity {
                encoded_identity: r.clone(),
            }),
            download_preference: Some(DownloadPreference::Redirect.into()),
        };
        let mut read_stream = file_object_client.read(request).await.unwrap();
        debug!(?read_stream);

        // First message is metadata
        match read_stream.next().await.unwrap() {
            Ok(read_response) => match read_response.reply_type {
                Some(read_response::ReplyType::Metadata(_)) => {},
                other => panic!("Expected metadata as first message, got {:?}", other),
            },
            Err(e) => panic!("Read reply error: {e:?}"),
        }

        let mut hasher = Sha256::new();

        // Second message is redirect; read data from the returned URL
        match read_stream.next().await.unwrap() {
            Ok(read_response) => match read_response.reply_type {
                Some(read_response::ReplyType::Redirect(redirect)) => {
                    let mut headers = HeaderMap::new();
                    for header in redirect.additional_headers {
                        headers.insert(
                            HeaderName::from_bytes(header.name.as_bytes()).unwrap(),
                            header.value.parse().unwrap(),
                        );
                    }
                    debug!(%redirect.redirect_target_url, ?headers);
                    let body = reqwest::Client::new()
                        .get(redirect.redirect_target_url)
                        .headers(headers)
                        .send()
                        .await
                        .unwrap()
                        .bytes()
                        .await
                        .unwrap();
                    debug!(?body);
                    hasher.update(body);
                },
                other => panic!("Expected redirect response but got {:?}", other),
            },
            Err(_) => panic!("Expected redirect response but got empty reply!"),
        }

        let result = hasher.finalize();
        hashes.insert(result.into());
    }
    debug!(base_address, ?hashes, ?expected_hashes);
    assert_eq!(hashes, expected_hashes);
}
