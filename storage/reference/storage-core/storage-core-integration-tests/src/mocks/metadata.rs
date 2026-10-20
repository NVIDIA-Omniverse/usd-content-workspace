// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! In-process mock metadata service used as the default branch of the
//! `MetadataKeyRouter` in router integration tests.
//!
//! The mock keeps state in a `(uri, key) -> UserMetadataValue` map and records every
//! incoming request. Tests pre-seed state via `set`, drive the router from outside,
//! and inspect both `get`/`requests_for` to verify dispatch.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use storage_core::traits::metadata::v1alpha::{
    update_metadata_batch_fanout, DeleteMetadataRequest, DeleteMetadataResponse, GetMetadataRequest,
    GetMetadataResponse, Metadata, UpdateMetadataBatchRequest, UpdateMetadataBatchResponse, UpdateMetadataRequest,
    UpdateMetadataResponse, UserMetadataValue, Value,
};
use storage_core::traits::{async_trait, error::Result, Request};

#[derive(Debug, Clone)]
pub enum MockRequest {
    Get {
        uri: String,
        keys: Vec<String>,
    },
    Update {
        uri: String,
        key: String,
        value: UserMetadataValue,
    },
    Delete {
        uri: String,
        key: String,
    },
}

pub struct MockMetadata {
    state: Mutex<HashMap<(String, String), UserMetadataValue>>,
    requests: Mutex<Vec<MockRequest>>,
    etag_counter: AtomicU64,
}

impl MockMetadata {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(HashMap::new()),
            requests: Mutex::new(Vec::new()),
            etag_counter: AtomicU64::new(0),
        })
    }

    /// Seed a value for `(uri, key)`. The etag on the supplied `value` is preserved.
    pub fn set(&self, uri: impl Into<String>, key: impl Into<String>, value: UserMetadataValue) {
        let uri = uri.into();
        let key = key.into();
        self.state.lock().unwrap().insert((uri, key), value);
    }

    /// Read the current value for `(uri, key)`, ignoring the request log.
    pub fn get(&self, uri: &str, key: &str) -> Option<UserMetadataValue> {
        self.state
            .lock()
            .unwrap()
            .get(&(uri.to_string(), key.to_string()))
            .cloned()
    }

    /// Drop all stored values for `uri`. Does not touch the request log.
    pub fn reset_uri(&self, uri: &str) {
        self.state.lock().unwrap().retain(|(u, _), _| u != uri);
    }

    /// All recorded requests targeting `uri`, in arrival order.
    pub fn requests_for(&self, uri: &str) -> Vec<MockRequest> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| match r {
                MockRequest::Get { uri: u, .. } => u == uri,
                MockRequest::Update { uri: u, .. } => u == uri,
                MockRequest::Delete { uri: u, .. } => u == uri,
            })
            .cloned()
            .collect()
    }

    fn next_etag(&self) -> String {
        let n = self.etag_counter.fetch_add(1, Ordering::SeqCst);
        format!("mock-etag-{n}")
    }
}

#[async_trait]
impl Metadata for MockMetadata {
    fn inner(&self) -> Option<&dyn Metadata> {
        None
    }

    fn backend(&self) -> &'static str {
        "mock"
    }

    async fn get_metadata(&self, request: Request<GetMetadataRequest>) -> Result<GetMetadataResponse> {
        let req = request.into_inner();
        self.requests.lock().unwrap().push(MockRequest::Get {
            uri: req.uri.clone(),
            keys: req.user_metadata_keys.clone(),
        });

        let state = self.state.lock().unwrap();
        let mut user_metadata: HashMap<String, UserMetadataValue> = HashMap::new();
        if req.user_metadata_keys.is_empty() {
            // Give-me-all: return every stored key for this URI.
            for ((u, k), v) in state.iter() {
                if u == &req.uri {
                    user_metadata.insert(k.clone(), v.clone());
                }
            }
        } else {
            // Selective: return only the requested keys that exist.
            for k in &req.user_metadata_keys {
                if let Some(v) = state.get(&(req.uri.clone(), k.clone())) {
                    user_metadata.insert(k.clone(), v.clone());
                }
            }
        }

        Ok(GetMetadataResponse { user_metadata })
    }

    async fn update_metadata(&self, request: Request<UpdateMetadataRequest>) -> Result<UpdateMetadataResponse> {
        let req = request.into_inner();
        let etag = self.next_etag();
        let stored = UserMetadataValue {
            value: req.user_metadata.clone().or(Some(Value { kind: None })),
            etag: etag.clone(),
        };

        self.requests.lock().unwrap().push(MockRequest::Update {
            uri: req.uri.clone(),
            key: req.user_metadata_key.clone(),
            value: stored.clone(),
        });

        self.state
            .lock()
            .unwrap()
            .insert((req.uri, req.user_metadata_key), stored);

        Ok(UpdateMetadataResponse { etag })
    }

    async fn update_metadata_batch(
        &self,
        request: Request<UpdateMetadataBatchRequest>,
    ) -> Result<UpdateMetadataBatchResponse> {
        update_metadata_batch_fanout(self, request).await
    }

    async fn delete_metadata(&self, request: Request<DeleteMetadataRequest>) -> Result<DeleteMetadataResponse> {
        let req = request.into_inner();
        self.requests.lock().unwrap().push(MockRequest::Delete {
            uri: req.uri.clone(),
            key: req.user_metadata_key.clone(),
        });
        self.state.lock().unwrap().remove(&(req.uri, req.user_metadata_key));
        Ok(DeleteMetadataResponse {})
    }
}
