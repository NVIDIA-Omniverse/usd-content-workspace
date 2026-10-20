// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use http::StatusCode;
use rstest::*;
use storage_core_integration_tests::fixture::*;
use storage_core_integration_tests::storage_fixture;
use storage_core_test::clients::rest::request_with_retries;

#[rstest]
#[test_log::test(tokio::test)]
async fn openapi(
    storage_fixture: &StorageFixture,
    #[values(Backend::S3, Backend::Azure, Backend::Minio, Backend::GCPS3Compat)] backend: Backend,
) {
    let rest_port = storage_fixture.rest_port(backend, Cache::Disabled);

    // Check that the OpenAPI schema is available for each endpoint
    for url in [
        format!("http://localhost:{rest_port}/v1alpha/capabilities/openapi.json"),
        format!("http://localhost:{rest_port}/v1alpha/fileobject/openapi.json"),
        format!("http://localhost:{rest_port}/v1alpha/filefolder/openapi.json"),
        format!("http://localhost:{rest_port}/v1alpha/versioning/openapi.json"),
        format!("http://localhost:{rest_port}/v1alpha/metadata/openapi.json"),
    ] {
        tracing::info!("Checking OpenAPI schema for {}", url);
        let response = request_with_retries(|| reqwest::get(url.clone())).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.text().await.unwrap();
        assert!(!body.is_empty());
    }

    // Check that the Swagger UI is available
    let url = url::Url::parse(&format!("http://localhost:{rest_port}/swagger-ui/")).unwrap();
    tracing::info!("Checking Swagger UI at {}", url);
    let response = request_with_retries(|| reqwest::get(url.clone())).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.text().await.unwrap();
    assert!(body.contains("Swagger UI"));
}
