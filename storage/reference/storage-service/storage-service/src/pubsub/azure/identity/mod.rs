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

use std::sync::Arc;

use azure_core::StatusCode;
use azure_storage_blobs::{
    container::operations::BlobItem,
    prelude::{BlobClient, ContainerClient},
};
use futures::TryStreamExt;
use percent_encoding::utf8_percent_encode;
use snafu::IntoError;
use storage_core::azure::{resource::Address, Storage};
use storage_service_metrics::{
    future::FutureExt,
    status::result::Status,
    utils::{record_metric, record_metric_value},
};
use tracing::debug;
use url::Url;

use crate::{
    common::url::URI_COMPONENT_SET,
    pubsub::azure_service_bus::types::{BoxError, IdentityResolver},
};

pub mod error;

mod metrics;

#[derive(Debug)]
pub struct AzureIdentityResolver {
    storage: Arc<Storage>,
    metrics: metrics::Metrics,
}

impl AzureIdentityResolver {
    pub fn new(storage: Arc<Storage>) -> Self {
        Self {
            storage,
            metrics: metrics::Metrics::new(),
        }
    }

    async fn try_resolve_identity(&self, address: &str, etag: &str) -> Result<Option<String>, error::Error> {
        debug_assert!(!etag.starts_with('"'));
        debug_assert!(!etag.ends_with('"'));

        let url = Url::parse(address).map_err(|source| error::UrlParse.into_error(source))?;
        debug_assert!(url.query().is_none());
        debug_assert!(url.fragment().is_none());

        let address = Address::parse_blob(url).map_err(|source| error::AddressParse.into_error(source))?;
        let client = self
            .storage
            .client(&address)
            .await
            .map_err(|source| error::ClientCreate.into_error(source))?;

        let container_client = client.container_client(
            address
                .container()
                .map_err(|source| error::AddressParse.into_error(source))?,
        );

        let blob_name = address.blob();
        let blob_client = container_client.blob_client(blob_name.as_ref());

        debug!(
            ?address,
            ?etag,
            "attempting to resolve the version id from the latest state"
        );

        let maybe_resolved_version_id = self
            .try_resolve_version_id_with_get_metadata(&blob_client, etag)
            .await?;
        match maybe_resolved_version_id {
            MaybeResolvedVersionId::NotResolved => {
                // Nothing to do.
            },
            MaybeResolvedVersionId::VersioningDisabled => {
                debug!("version tag is missing from the get_properties response, falling back to unversioned");
                return Ok(Some(create_unversioned_identity(address, etag)));
            },
            MaybeResolvedVersionId::Resolved(version) => {
                debug!(?version, "version tag resolved");
                return Ok(Some(create_versioned_identity(address, etag, &version)));
            },
        }

        debug!("attempting to resolve the version id from the version list");

        let maybe_resolved_version_id = self
            .try_resolve_version_id_with_list_versions(&container_client, blob_name.as_ref(), etag)
            .await?;
        match maybe_resolved_version_id {
            MaybeResolvedVersionId::NotResolved => {
                // Nothing to do.
            },
            MaybeResolvedVersionId::VersioningDisabled => {
                debug!("version tag is missing from the list_blobs response, falling back to unversioned");
                return Ok(Some(create_unversioned_identity(address, etag)));
            },
            MaybeResolvedVersionId::Resolved(version) => {
                debug!(?version, "version tag resolved");
                return Ok(Some(create_versioned_identity(address, etag, &version)));
            },
        }

        debug!("failed to resolve version tag");

        Ok(None)
    }

    async fn try_resolve_version_id_with_get_metadata(
        &self,
        client: &BlobClient,
        etag: &str,
    ) -> Result<MaybeResolvedVersionId, error::Error> {
        debug_assert!(!etag.starts_with('"'));
        debug_assert!(!etag.ends_with('"'));

        let blob_properties_or_error = async { client.get_properties().await }
            .with_latency_metric(&self.metrics.get_properties_latency)
            .await
            .inspect(record_metric_value(
                &self.metrics.get_properties_results,
                StatusCode::Ok,
            ))
            .inspect_err(record_metric(&self.metrics.get_properties_results));

        let blob_properties = match blob_properties_or_error {
            Ok(blob_properties) => blob_properties,
            Err(error) => match error.as_http_error() {
                Some(e) if e.status() == StatusCode::NotFound => {
                    debug!(?error, "blob not found");
                    return Ok(MaybeResolvedVersionId::NotResolved);
                },
                _ => {
                    return Err(error::GetProperties.into_error(error));
                },
            },
        };

        let Some(version_id) = blob_properties.blob.version_id else {
            return Ok(MaybeResolvedVersionId::VersioningDisabled);
        };

        if strip_quotes_if_needed(blob_properties.blob.properties.etag.as_ref()) == etag {
            return Ok(MaybeResolvedVersionId::Resolved(version_id));
        }

        Ok(MaybeResolvedVersionId::NotResolved)
    }

    async fn try_resolve_version_id_with_list_versions(
        &self,
        client: &ContainerClient,
        blob: &str,
        etag: &str,
    ) -> Result<MaybeResolvedVersionId, error::Error> {
        debug_assert!(!etag.starts_with('"'));
        debug_assert!(!etag.ends_with('"'));

        let mut stream = client
            .list_blobs()
            .include_versions(true)
            .include_deleted(true)
            .prefix(blob.to_string())
            .into_stream();
        while let Some(page) = stream
            .try_next()
            .with_latency_metric(&self.metrics.list_blobs_latency)
            .await
            .inspect(record_metric_value(&self.metrics.list_blobs_results, StatusCode::Ok))
            .inspect_err(record_metric(&self.metrics.list_blobs_results))
            .map_err(|source| error::ListBlobs.into_error(source))?
        {
            self.metrics
                .list_blobs_response_size
                .record(page.blobs.items.len() as u64);

            for item in page.blobs.items.into_iter() {
                // We don't expect blob prefixes. So, this branch should never trigger.
                let BlobItem::Blob(item) = item else {
                    continue;
                };

                // Response entries are sorted alphabetically. We can stop listing entries once we've encountered
                // a different blob name.
                if item.name != blob {
                    break;
                }

                let Some(version_id) = item.version_id else {
                    return Ok(MaybeResolvedVersionId::VersioningDisabled);
                };

                if item.properties.etag.as_ref() == etag {
                    return Ok(MaybeResolvedVersionId::Resolved(version_id));
                }
            }

            // We don't want to submit false metrics.
            if page.next_marker.is_none() {
                break;
            }
        }

        Ok(MaybeResolvedVersionId::NotResolved)
    }
}

#[async_trait::async_trait]
impl IdentityResolver for AzureIdentityResolver {
    async fn resolve_identity(&self, address: &str, etag: &str) -> Result<Option<String>, BoxError> {
        debug_assert!(!etag.starts_with('"'));
        debug_assert!(!etag.ends_with('"'));

        self.try_resolve_identity(address, etag)
            .with_latency_metric(&self.metrics.resolve_identity_latency)
            .await
            .inspect(record_metric_value(&self.metrics.resolve_identity_results, Status::Ok))
            .inspect_err(record_metric_value(&self.metrics.resolve_identity_results, Status::Err))
            .map_err(|source| Box::new(source) as BoxError)
    }
}

#[derive(Debug)]
enum MaybeResolvedVersionId {
    VersioningDisabled,
    NotResolved,
    Resolved(String),
}

fn create_unversioned_identity(address: Address, etag: &str) -> String {
    debug_assert!(!etag.starts_with('"'));
    debug_assert!(!etag.ends_with('"'));
    debug_assert!(address.resource_address.fragment().is_none());
    debug_assert!(address.resource_address.query().is_none());

    format!(
        "{}?etag=%22{}%22",
        address.resource_address.as_str(),
        utf8_percent_encode(etag, &URI_COMPONENT_SET)
    )
}

fn create_versioned_identity(address: Address, etag: &str, version: &str) -> String {
    debug_assert!(!etag.starts_with('"'));
    debug_assert!(!etag.ends_with('"'));
    debug_assert!(address.resource_address.fragment().is_none());
    debug_assert!(address.resource_address.query().is_none());

    format!(
        "{}?etag=%22{}%22&version-id={}",
        address.resource_address.as_str(),
        utf8_percent_encode(etag, &URI_COMPONENT_SET),
        utf8_percent_encode(version, &URI_COMPONENT_SET)
    )
}

fn strip_quotes_if_needed(value: &str) -> &str {
    debug_assert_eq!(value.starts_with('"'), value.ends_with('"'));

    value
        .strip_prefix('"')
        .map(|value| value.strip_suffix('"').unwrap_or(value))
        .unwrap_or(value)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use azure_storage_blobs::prelude::ContainerClient;
    use bytes::Bytes;
    use futures::TryStreamExt;
    use percent_encoding::utf8_percent_encode;
    use rstest::{fixture, rstest};
    use storage_core::azure::Storage;

    use crate::{common::url::URI_COMPONENT_SET, pubsub::azure_service_bus::types::IdentityResolver};

    use super::AzureIdentityResolver;

    struct Helpers {
        storage_account: String,
        storage_container: String,
        storage: Arc<Storage>,
        resolver: AzureIdentityResolver,
        client: ContainerClient,
    }

    impl Helpers {
        fn get_random_blob_name(&self) -> String {
            use rand::seq::IndexedRandom;

            const CHARACTERS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

            let get_random_path_segment = || -> String {
                unsafe {
                    String::from_utf8_unchecked(CHARACTERS.choose_multiple(&mut rand::rng(), 32).cloned().collect())
                }
            };

            format!("{}/{}", get_random_path_segment(), get_random_path_segment())
        }

        fn get_blob_url(&self, name: &str) -> String {
            format!(
                "https://{}.blob.core.windows.net/{}/{}",
                self.storage_account, self.storage_container, name
            )
        }

        fn get_blob_version_url(&self, name: &str, etag: &str, version: &str) -> String {
            format!(
                "https://{}.blob.core.windows.net/{}/{}?etag=%22{}%22&version-id={}",
                self.storage_account,
                self.storage_container,
                name,
                utf8_percent_encode(etag, &URI_COMPONENT_SET),
                utf8_percent_encode(version, &URI_COMPONENT_SET)
            )
        }

        async fn create_blob_version(&self, name: &str) -> (String, String, Bytes) {
            use rand::Rng;

            let blob_data = Bytes::from_iter(rand::rng().random_iter().take(16));

            let blob_properties = self
                .client
                .blob_client(name)
                .put_block_blob(blob_data.clone())
                .await
                .expect("put blob failed");
            let version_id = blob_properties.version_id.expect("version id is missing");
            let etag = &(blob_properties.etag[1..blob_properties.etag.len() - 1]);

            (version_id, etag.into(), blob_data)
        }

        async fn delete_blob(&self, name: &str) {
            self.client
                .blob_client(name)
                .delete()
                .await
                .expect("delete blob failed");
        }

        async fn read_from_identity(&self, identity: impl Into<String>) -> Bytes {
            use bytes::BytesMut;
            use storage_core::traits::{fileobject::v1alpha::*, Request};

            let mut stream = self
                .storage
                .read(Request::new(ReadRequest {
                    resource_identity: Some(ResourceIdentity {
                        encoded_identity: identity.into(),
                    }),
                    download_preference: Some(DownloadPreference::Body as i32),
                }))
                .await
                .expect("read failed");

            stream
                .try_next()
                .await
                .expect("read failed")
                .expect("unexpected end of stream");

            let mut data = BytesMut::new();
            while let Some(message) = stream.try_next().await.expect("read failed") {
                match message.reply_type {
                    Some(read_response::ReplyType::Chunk(chunk)) => {
                        data.extend_from_slice(&chunk.chunk);
                    },
                    _ => panic!("unexpected message type"),
                }
            }

            data.into()
        }
    }

    #[fixture]
    #[once]
    fn helpers() -> Helpers {
        use azure_storage::StorageCredentials;
        use azure_storage_blobs::prelude::BlobServiceClient;

        use storage_core::azure::{
            secrets::{ClientCredentials, Secrets},
            Storage,
        };

        let storage_account = std::env::var("AZURE_BLOB_STORAGE_ACCOUNT").unwrap();
        let storage_container = std::env::var("AZURE_BLOB_STORAGE_CONTAINER").unwrap();
        let storage_key = std::env::var("AZURE_BLOB_STORAGE_KEY").unwrap();

        let storage = Arc::new(
            Storage::new(
                Default::default(),
                Some(Secrets(
                    [(
                        format!("https://{}.blob.core.windows.net", storage_account,),
                        ClientCredentials {
                            storage_key: Some(storage_key.clone()),
                            ..Default::default()
                        },
                    )]
                    .into(),
                )),
            )
            .unwrap(),
        );

        let resolver = AzureIdentityResolver::new(storage.clone());

        let client = BlobServiceClient::new(
            &storage_account,
            StorageCredentials::access_key(&storage_account, storage_key),
        );

        Helpers {
            client: client.container_client(&storage_container),
            storage,
            resolver,
            storage_account,
            storage_container,
        }
    }

    #[rstest]
    #[test_log::test(tokio::test)]
    async fn test_identity_resolver_resolves_identity_for_head(helpers: &Helpers) {
        let blob_name = helpers.get_random_blob_name();
        let blob_address = helpers.get_blob_url(&blob_name);
        let (blob_version, blob_etag, blob_data) = helpers.create_blob_version(&blob_name).await;

        let blob_identity = helpers
            .resolver
            .resolve_identity(&blob_address, &blob_etag)
            .await
            .expect("failed to resolve blob identity");

        assert_eq!(
            blob_identity,
            Some(helpers.get_blob_version_url(&blob_name, &blob_etag, &blob_version))
        );

        let read_blob_data = helpers.read_from_identity(&blob_identity.unwrap()).await;

        assert_eq!(read_blob_data, blob_data);
    }

    #[rstest]
    #[test_log::test(tokio::test)]
    async fn test_identity_resolver_resolves_identity_for_version(helpers: &Helpers) {
        let blob_name = helpers.get_random_blob_name();
        let blob_address = helpers.get_blob_url(&blob_name);
        let (blob_version, blob_etag, blob_data) = helpers.create_blob_version(&blob_name).await;
        helpers.create_blob_version(&blob_name).await;

        let blob_identity = helpers
            .resolver
            .resolve_identity(&blob_address, &blob_etag)
            .await
            .expect("failed to resolve blob identity");

        assert_eq!(
            blob_identity,
            Some(helpers.get_blob_version_url(&blob_name, &blob_etag, &blob_version))
        );

        let read_blob_data = helpers.read_from_identity(&blob_identity.unwrap()).await;

        assert_eq!(read_blob_data, blob_data);
    }

    #[rstest]
    #[test_log::test(tokio::test)]
    async fn test_identity_resolver_resolve_unknown_identity(helpers: &Helpers) {
        let blob_name = helpers.get_random_blob_name();
        let blob_address = helpers.get_blob_url(&blob_name);
        helpers.create_blob_version(&blob_name).await;

        let blob_identity = helpers
            .resolver
            .resolve_identity(&blob_address, "etag")
            .await
            .expect("failed to resolve blob identity");

        assert_eq!(blob_identity, None);
    }

    #[rstest]
    #[test_log::test(tokio::test)]
    async fn test_identity_resolver_resolve_deleted_version_identity(helpers: &Helpers) {
        let blob_name = helpers.get_random_blob_name();
        let blob_address = helpers.get_blob_url(&blob_name);
        let (blob_version, blob_etag, _) = helpers.create_blob_version(&blob_name).await;
        helpers.delete_blob(&blob_name).await;

        let blob_identity = helpers
            .resolver
            .resolve_identity(&blob_address, &blob_etag)
            .await
            .expect("failed to resolve blob identity");

        assert_eq!(
            blob_identity,
            Some(helpers.get_blob_version_url(&blob_name, &blob_etag, &blob_version))
        );
    }
}
