// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use rstest::{fixture, rstest};
use storage_core::{
    api_services::fileobject::v1alpha::FileObjectBuilder,
    api_services::versioning::v1alpha::VersioningBuilder,
    ingress::IdentityLayer,
    storage::{StorageService, StorageServiceServer},
    traits::fileobject::v1alpha::FileObject,
    traits::versioning::v1alpha::Versioning,
};
use storage_core_integration_tests::names::{test_names, TestNames, AZURE_CONTAINER};
use storage_core_test::{
    service::{ApiVersion, Interface},
    storage_service_runner::StorageServiceRunner,
};
use url::Url;

fn create_service<B: FileObject + Versioning>(backend: B) -> StorageServiceServer<IdentityLayer> {
    let backend = Arc::new(backend);
    StorageService::builder()
        .add_service(FileObjectBuilder::new().backend(backend.clone()))
        .add_service(VersioningBuilder::new().backend(backend))
}

#[track_caller]
fn must_get_env(env: &str) -> String {
    std::env::var(env).unwrap_or_else(|_| panic!("{} is missing", env))
}

/// The `Lorem ipsum.txt` fixture object in an S3 test bucket.
fn s3_lorem_ipsum(bucket: &str) -> String {
    format!("{}/unittests/readFile/Lorem%20ipsum.txt", TestNames::s3_url(bucket))
}

/// The `Lorem ipsum.txt` fixture blob in an Azure test container.
fn azure_lorem_ipsum(account: &str, container: &str) -> String {
    format!(
        "{}/{container}/storage-service-unittests/unittests/readFile/Lorem%20ipsum.txt",
        TestNames::azure_url(account)
    )
}

#[fixture]
#[once]
fn s3_runner() -> StorageServiceRunner {
    use aws_credential_types::Credentials;
    use storage_core::s3::{
        secrets::{ClientCredentials, CredentialsOverride, Secrets},
        Storage,
    };

    let access_key_id = must_get_env("OMNI_STORAGE_SERVICE_AWS_ACCESS_KEY_ID");
    let secret_access_key = must_get_env("OMNI_STORAGE_SERVICE_AWS_SECRET_ACCESS_KEY");
    let credentials = Credentials::new(access_key_id, secret_access_key, None, None, "static");

    let backend = Storage::new(
        Default::default(),
        Some(Secrets(
            [
                (
                    TestNames::s3_host(&test_names().s3_bucket),
                    ClientCredentials {
                        credentials_override: Some(CredentialsOverride::CredentialsProvider(Arc::new(credentials))),
                        access_key_id: Some("invalid-access-key-id".into()),
                        secret_access_key: Some("invalid-secret-access-key".into()),
                        ..Default::default()
                    },
                ),
                (
                    TestNames::s3_host(&test_names().s3_public_bucket),
                    ClientCredentials {
                        credentials_override: Some(CredentialsOverride::Anonymous),
                        access_key_id: Some("invalid-access-key-id".into()),
                        secret_access_key: Some("invalid-secret-access-key".into()),
                        role_arn: Some("invalid-role-arn".into()),
                        ..Default::default()
                    },
                ),
            ]
            .into(),
        )),
    )
    .unwrap();

    StorageServiceRunner::new::<IdentityLayer, tonic::body::Body>(create_service(backend), Default::default())
}

#[fixture]
#[once]
fn azure_runner() -> StorageServiceRunner {
    use azure_core::new_http_client;
    use azure_identity::ClientSecretCredential;
    use storage_core::azure::{
        secrets::{ClientCredentials, CredentialOverride, Secrets},
        Storage,
    };

    let tenant_id = must_get_env("OMNI_STORAGE_SERVICE_AZURE_TENANT_ID");
    let client_id = must_get_env("OMNI_STORAGE_SERVICE_AZURE_CLIENT_ID");
    let client_secret = must_get_env("OMNI_STORAGE_SERVICE_AZURE_CLIENT_SECRET");
    let credential = ClientSecretCredential::new(
        new_http_client(),
        Url::parse("https://login.microsoftonline.com").unwrap(),
        tenant_id,
        client_id,
        client_secret,
    );

    let backend = Storage::new(
        Default::default(),
        Some(Secrets(
            [
                (
                    TestNames::azure_host(&test_names().azure_private_account),
                    ClientCredentials {
                        credential_override: Some(CredentialOverride::TokenCredential(Arc::new(credential))),
                        client_id: Some("invalid-client-id".into()),
                        client_secret: Some("invalid-client-secret".into()),
                        tenant_id: Some("invalid-tenant-id".into()),
                        ..Default::default()
                    },
                ),
                (
                    TestNames::azure_host(&test_names().azure_versioned_account),
                    ClientCredentials {
                        credential_override: Some(CredentialOverride::Anonymous),
                        client_id: Some("invalid-client-id".into()),
                        client_secret: Some("invalid-client-secret".into()),
                        tenant_id: Some("invalid-tenant-id".into()),
                        ..Default::default()
                    },
                ),
            ]
            .into(),
        )),
    )
    .unwrap();

    StorageServiceRunner::new::<IdentityLayer, tonic::body::Body>(create_service(backend), Default::default())
}

#[rstest]
#[case::case_01_s3_private(s3_lorem_ipsum(&test_names().s3_bucket))]
#[case::case_02_s3_public(s3_lorem_ipsum(&test_names().s3_public_bucket))]
#[test_log::test(tokio::test)]
async fn test_s3_credentials_override(s3_runner: &StorageServiceRunner, #[case] resource_address: String) {
    use storage_core::traits::fileobject::v1alpha::StatRequest;

    let response = s3_runner
        .file_object_client(Interface::Grpc, ApiVersion::V1Alpha)
        .await
        .stat(StatRequest { resource_address })
        .await
        .expect("stat failed");

    assert_eq!(
        response
            .resource_info
            .expect("resource info is missing")
            .metadata
            .expect("metadata is missing")
            .data_object_size,
        Some(3549)
    );
}

#[rstest]
#[case::case_01_azure_private(azure_lorem_ipsum(&test_names().azure_private_account, AZURE_CONTAINER))]
#[case::case_02_azure_public(azure_lorem_ipsum(
    &test_names().azure_public_account,
    &test_names().azure_public_container
))]
#[test_log::test(tokio::test)]
async fn test_azure_credentials_override(azure_runner: &StorageServiceRunner, #[case] resource_address: String) {
    use storage_core::traits::fileobject::v1alpha::StatRequest;

    let response = azure_runner
        .file_object_client(Interface::Grpc, ApiVersion::V1Alpha)
        .await
        .stat(StatRequest { resource_address })
        .await
        .expect("stat failed");

    assert_eq!(
        response
            .resource_info
            .expect("resource info is missing")
            .metadata
            .expect("metadata is missing")
            .data_object_size,
        Some(3549)
    );
}

// IsReadOnly and IsVersioned are capability queries answered from configuration; they do not
// call the backend, so the default S3 config (read_only unset, versioning_enabled unset)
// reports writable and versioned across both interfaces.
#[rstest]
#[test_log::test(tokio::test)]
async fn test_is_read_only_reports_configured_default(
    s3_runner: &StorageServiceRunner,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use storage_core::traits::fileobject::v1alpha::IsReadOnlyRequest;

    let response = s3_runner
        .file_object_client(interface, ApiVersion::V1Alpha)
        .await
        .is_read_only(IsReadOnlyRequest {
            resource_address: s3_lorem_ipsum(&test_names().s3_bucket),
        })
        .await
        .expect("is_read_only failed");

    assert!(!response.is_read_only);
}

#[rstest]
#[test_log::test(tokio::test)]
async fn test_is_versioned_reports_configured_default(
    s3_runner: &StorageServiceRunner,
    #[values(Interface::Grpc, Interface::Rest)] interface: Interface,
) {
    use storage_core::traits::versioning::v1alpha::IsVersionedRequest;

    let response = s3_runner
        .versioning_client(interface, ApiVersion::V1Alpha)
        .await
        .is_versioned(IsVersionedRequest {
            resource_address: s3_lorem_ipsum(&test_names().s3_bucket),
        })
        .await
        .expect("is_versioned failed");

    assert!(response.is_versioned);
}
