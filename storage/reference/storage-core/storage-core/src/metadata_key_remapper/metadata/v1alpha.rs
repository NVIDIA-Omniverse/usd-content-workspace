// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;
use tracing::instrument;

use storage_core_builder::traits::{
    error::Result,
    metadata::v1alpha::{
        DeleteMetadataRequest, DeleteMetadataResponse, GetMetadataRequest, GetMetadataResponse, Metadata,
        UpdateMetadataBatchRequest, UpdateMetadataBatchResponse, UpdateMetadataRequest, UpdateMetadataResponse,
    },
    Request,
};

use crate::metadata_key_remapper::MetadataKeyRemapper;

#[async_trait]
impl<S> Metadata for MetadataKeyRemapper<S>
where
    S: Metadata,
{
    fn inner(&self) -> Option<&dyn Metadata> {
        Some(&self.inner)
    }

    #[instrument(name = "metadata_key_remapper::get_metadata", skip_all)]
    async fn get_metadata(&self, mut request: Request<GetMetadataRequest>) -> Result<GetMetadataResponse> {
        // Forward translation: client → storage on the requested key list.
        // An empty list means "give me everything", so leave it empty.
        let req = request.get_mut();
        for key in req.user_metadata_keys.iter_mut() {
            if let Some(storage_key) = self.forward.get(key.as_str()) {
                *key = storage_key.clone();
            }
        }

        let mut response = self.inner.get_metadata(request).await?;

        // Reverse translation: storage → client on the response map keys.
        response.user_metadata = response
            .user_metadata
            .into_iter()
            .map(|(k, v)| (self.to_client_key(k), v))
            .collect();

        Ok(response)
    }

    #[instrument(name = "metadata_key_remapper::update_metadata", skip_all)]
    async fn update_metadata(&self, mut request: Request<UpdateMetadataRequest>) -> Result<UpdateMetadataResponse> {
        let req = request.get_mut();
        req.user_metadata_key = self.to_storage_key(&req.user_metadata_key);

        self.inner.update_metadata(request).await
    }

    #[instrument(name = "metadata_key_remapper::update_metadata_batch", skip_all)]
    async fn update_metadata_batch(
        &self,
        mut request: Request<UpdateMetadataBatchRequest>,
    ) -> Result<UpdateMetadataBatchResponse> {
        for entry in request.get_mut().requests.iter_mut() {
            entry.user_metadata_key = self.to_storage_key(&entry.user_metadata_key);
        }

        self.inner.update_metadata_batch(request).await
    }

    #[instrument(name = "metadata_key_remapper::delete_metadata", skip_all)]
    async fn delete_metadata(&self, mut request: Request<DeleteMetadataRequest>) -> Result<DeleteMetadataResponse> {
        let req = request.get_mut();
        req.user_metadata_key = self.to_storage_key(&req.user_metadata_key);

        self.inner.delete_metadata(request).await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;
    use crate::metadata_key_remapper::{Config, MetadataKeyRemapperLayer};
    use storage_core_builder::layer::Layer;
    use storage_core_builder::traits::metadata::v1alpha::UserMetadataValue;

    /// Capture the request the inner service receives, and emit a configurable response.
    #[derive(Default)]
    struct StubInner {
        last_get_keys: Mutex<Vec<String>>,
        last_update_key: Mutex<Option<String>>,
        last_batch_keys: Mutex<Vec<String>>,
        last_delete_key: Mutex<Option<String>>,
        get_response_keys: Mutex<Vec<String>>,
    }

    impl StubInner {
        fn with_get_response(keys: Vec<&str>) -> Self {
            Self {
                get_response_keys: Mutex::new(keys.into_iter().map(String::from).collect()),
                ..Default::default()
            }
        }
    }

    #[async_trait]
    impl Metadata for StubInner {
        fn inner(&self) -> Option<&dyn Metadata> {
            None
        }

        async fn get_metadata(&self, request: Request<GetMetadataRequest>) -> Result<GetMetadataResponse> {
            *self.last_get_keys.lock().unwrap() = request.into_inner().user_metadata_keys;
            let user_metadata: HashMap<String, UserMetadataValue> = self
                .get_response_keys
                .lock()
                .unwrap()
                .iter()
                .map(|k| {
                    (
                        k.clone(),
                        UserMetadataValue {
                            value: None,
                            etag: format!("etag-{k}"),
                        },
                    )
                })
                .collect();
            Ok(GetMetadataResponse { user_metadata })
        }

        async fn update_metadata(&self, request: Request<UpdateMetadataRequest>) -> Result<UpdateMetadataResponse> {
            *self.last_update_key.lock().unwrap() = Some(request.into_inner().user_metadata_key);
            Ok(UpdateMetadataResponse { etag: "etag".into() })
        }

        async fn update_metadata_batch(
            &self,
            request: Request<UpdateMetadataBatchRequest>,
        ) -> Result<UpdateMetadataBatchResponse> {
            *self.last_batch_keys.lock().unwrap() = request
                .into_inner()
                .requests
                .into_iter()
                .map(|entry| entry.user_metadata_key)
                .collect();
            Ok(UpdateMetadataBatchResponse { results: Vec::new() })
        }

        async fn delete_metadata(&self, request: Request<DeleteMetadataRequest>) -> Result<DeleteMetadataResponse> {
            *self.last_delete_key.lock().unwrap() = Some(request.into_inner().user_metadata_key);
            Ok(DeleteMetadataResponse {})
        }
    }

    fn build(mappings: &[(&str, &str)], inner: StubInner) -> MetadataKeyRemapper<StubInner> {
        let config = Config {
            enabled: true,
            mappings: mappings.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect(),
        };
        let layer = MetadataKeyRemapperLayer::new(config).unwrap().unwrap();
        layer.layer(inner)
    }

    #[tokio::test]
    async fn get_translates_request_keys_and_response_keys() {
        let inner = StubInner::with_get_response(vec!["nv_omniverse_nucleus_hash", "untouched"]);
        let svc = build(&[("nv-omniverse-nucleus-hash", "nv_omniverse_nucleus_hash")], inner);

        let request = Request::new(GetMetadataRequest {
            uri: "https://b.s3.amazonaws.com/k".into(),
            user_metadata_keys: vec!["nv-omniverse-nucleus-hash".into(), "passthrough".into()],
        });

        let response = svc.get_metadata(request).await.unwrap();

        // Request was rewritten client → storage; pass-through key untouched.
        assert_eq!(
            *svc.inner.last_get_keys.lock().unwrap(),
            vec!["nv_omniverse_nucleus_hash".to_string(), "passthrough".to_string()],
        );
        // Response was rewritten storage → client; unmapped key passed through.
        assert!(response.user_metadata.contains_key("nv-omniverse-nucleus-hash"));
        assert!(response.user_metadata.contains_key("untouched"));
        assert!(!response.user_metadata.contains_key("nv_omniverse_nucleus_hash"));
    }

    #[tokio::test]
    async fn get_with_empty_keys_only_translates_response() {
        let inner = StubInner::with_get_response(vec!["nv_omniverse_nucleus_hash"]);
        let svc = build(&[("nv-omniverse-nucleus-hash", "nv_omniverse_nucleus_hash")], inner);

        let request = Request::new(GetMetadataRequest {
            uri: "https://b.s3.amazonaws.com/k".into(),
            user_metadata_keys: vec![],
        });

        let response = svc.get_metadata(request).await.unwrap();

        assert!(svc.inner.last_get_keys.lock().unwrap().is_empty());
        assert!(response.user_metadata.contains_key("nv-omniverse-nucleus-hash"));
    }

    #[tokio::test]
    async fn update_translates_key() {
        let svc = build(
            &[("nv-omniverse-nucleus-hash", "nv_omniverse_nucleus_hash")],
            StubInner::default(),
        );

        let request = Request::new(UpdateMetadataRequest {
            uri: "https://b.s3.amazonaws.com/k".into(),
            user_metadata_key: "nv-omniverse-nucleus-hash".into(),
            user_metadata: None,
            expected_etag: None,
        });

        svc.update_metadata(request).await.unwrap();

        assert_eq!(
            svc.inner.last_update_key.lock().unwrap().as_deref(),
            Some("nv_omniverse_nucleus_hash"),
        );
    }

    #[tokio::test]
    async fn update_batch_translates_each_key() {
        let svc = build(
            &[("nv-omniverse-nucleus-hash", "nv_omniverse_nucleus_hash")],
            StubInner::default(),
        );

        let request = Request::new(UpdateMetadataBatchRequest {
            requests: vec![
                UpdateMetadataRequest {
                    uri: "https://b.s3.amazonaws.com/k".into(),
                    user_metadata_key: "nv-omniverse-nucleus-hash".into(),
                    user_metadata: None,
                    expected_etag: None,
                },
                UpdateMetadataRequest {
                    uri: "https://b.s3.amazonaws.com/k".into(),
                    user_metadata_key: "passthrough".into(),
                    user_metadata: None,
                    expected_etag: None,
                },
            ],
        });

        svc.update_metadata_batch(request).await.unwrap();

        assert_eq!(
            *svc.inner.last_batch_keys.lock().unwrap(),
            vec!["nv_omniverse_nucleus_hash".to_string(), "passthrough".to_string(),],
        );
    }

    #[tokio::test]
    async fn delete_translates_key() {
        let svc = build(
            &[("nv-omniverse-nucleus-hash", "nv_omniverse_nucleus_hash")],
            StubInner::default(),
        );

        let request = Request::new(DeleteMetadataRequest {
            uri: "https://b.s3.amazonaws.com/k".into(),
            user_metadata_key: "nv-omniverse-nucleus-hash".into(),
            expected_etag: None,
        });

        svc.delete_metadata(request).await.unwrap();

        assert_eq!(
            svc.inner.last_delete_key.lock().unwrap().as_deref(),
            Some("nv_omniverse_nucleus_hash"),
        );
    }

    #[tokio::test]
    async fn unmapped_keys_pass_through_untouched() {
        let svc = build(&[], StubInner::default());

        let request = Request::new(UpdateMetadataRequest {
            uri: "https://b.s3.amazonaws.com/k".into(),
            user_metadata_key: "anything".into(),
            user_metadata: None,
            expected_etag: None,
        });

        svc.update_metadata(request).await.unwrap();

        assert_eq!(svc.inner.last_update_key.lock().unwrap().as_deref(), Some("anything"));
    }
}
