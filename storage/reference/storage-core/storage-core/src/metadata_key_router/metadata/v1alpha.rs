// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;

use async_trait::async_trait;
use futures::future::{join_all, try_join_all};
use tracing::instrument;

use storage_core_builder::traits::{
    error::Result,
    metadata::v1alpha::{
        batch_entry_error, update_metadata_batch_entry_result, validate_metadata_batch_size, DeleteMetadataRequest,
        DeleteMetadataResponse, GetMetadataRequest, GetMetadataResponse, Metadata, UpdateMetadataBatchEntryResult,
        UpdateMetadataBatchRequest, UpdateMetadataBatchResponse, UpdateMetadataRequest, UpdateMetadataResponse,
        UserMetadataValue,
    },
    Request,
};
use storage_core_util::info_limited;

use crate::metadata_key_router::router::{MetadataKeyRouter, RouteTarget};

#[async_trait]
impl Metadata for MetadataKeyRouter {
    fn inner(&self) -> Option<&dyn Metadata> {
        None
    }

    #[instrument(name = "metadata_key_router::get_metadata", skip_all)]
    async fn get_metadata(&self, request: Request<GetMetadataRequest>) -> Result<GetMetadataResponse> {
        // Every fan-out call carries the caller's metadata (credentials, tracing) and
        // extensions (`AdditionalHeaders`, `CacheControl`); only the requested keys differ.
        let (metadata, extensions, req) = request.0.into_parts();
        let uri = req.uri;
        let forward = |user_metadata_keys: Vec<String>| -> Request<GetMetadataRequest> {
            let body = GetMetadataRequest {
                uri: uri.clone(),
                user_metadata_keys,
            };
            tonic::Request::from_parts(metadata.clone(), extensions.clone(), body).into()
        };

        // Empty `user_metadata_keys` means "give me everything". Ask every non-default
        // branch for the keys it owns and ask the default for everything else (empty).
        if req.user_metadata_keys.is_empty() {
            let mut futures = Vec::with_capacity(self.branches.len() + 1);
            for branch in &self.branches {
                futures.push(branch.service.get_metadata(forward(branch.keys.clone())));
            }
            futures.push(self.default.get_metadata(forward(Vec::new())));

            let responses = try_join_all(futures).await?;
            return Ok(merge_responses(&uri, responses));
        }

        // Bucket each requested key into the branch that owns it, or the default bucket.
        let mut branch_buckets: HashMap<usize, Vec<String>> = HashMap::new();
        let mut default_bucket: Vec<String> = Vec::new();
        for key in req.user_metadata_keys {
            match self.route_for_key(&key) {
                RouteTarget::Branch(i) => branch_buckets.entry(i).or_default().push(key),
                RouteTarget::Default => default_bucket.push(key),
            }
        }

        let mut futures = Vec::with_capacity(branch_buckets.len() + 1);
        for (idx, keys) in branch_buckets {
            futures.push(self.branches[idx].service.get_metadata(forward(keys)));
        }
        if !default_bucket.is_empty() {
            futures.push(self.default.get_metadata(forward(default_bucket)));
        }

        let responses = try_join_all(futures).await?;
        Ok(merge_responses(&uri, responses))
    }

    #[instrument(name = "metadata_key_router::update_metadata", skip_all)]
    async fn update_metadata(&self, request: Request<UpdateMetadataRequest>) -> Result<UpdateMetadataResponse> {
        let target = self.route_for_key(&request.get_ref().user_metadata_key);
        match target {
            RouteTarget::Branch(i) => self.branches[i].service.update_metadata(request).await,
            RouteTarget::Default => self.default.update_metadata(request).await,
        }
    }

    #[instrument(name = "metadata_key_router::update_metadata_batch", skip_all)]
    async fn update_metadata_batch(
        &self,
        request: Request<UpdateMetadataBatchRequest>,
    ) -> Result<UpdateMetadataBatchResponse> {
        validate_metadata_batch_size(request.get_ref().requests.len())?;
        let (metadata, extensions, body) = request.0.into_parts();

        let mut order: Vec<RouteTarget> = Vec::new();
        let mut groups: HashMap<RouteTarget, (Vec<UpdateMetadataRequest>, Vec<u32>)> = HashMap::new();
        for (idx, entry) in body.requests.into_iter().enumerate() {
            let target = self.route_for_key(&entry.user_metadata_key);
            if !groups.contains_key(&target) {
                order.push(target);
            }
            let (requests, indices) = groups.entry(target).or_default();
            requests.push(entry);
            indices.push(idx as u32);
        }

        // Fast-path: if every entry routes to the same target, forward the batch
        // unchanged so the downstream backend can apply its native batch
        // optimization (e.g., a single array-based SQL update) without us
        // re-allocating the request or remapping indices.
        if order.len() == 1 {
            let target = order.into_iter().next().expect("order.len() == 1");
            let service = match target {
                RouteTarget::Branch(i) => &self.branches[i].service,
                RouteTarget::Default => &self.default,
            };
            let (requests, _indices) = groups.remove(&target).unwrap_or_default();
            let forwarded =
                tonic::Request::from_parts(metadata, extensions, UpdateMetadataBatchRequest { requests }).into();
            return service.update_metadata_batch(forwarded).await;
        }

        let mut calls = Vec::with_capacity(order.len());
        for target in order {
            let service = match target {
                RouteTarget::Branch(i) => self.branches[i].service.clone(),
                RouteTarget::Default => self.default.clone(),
            };
            let (requests, indices) = groups.remove(&target).unwrap_or_default();
            let metadata = metadata.clone();
            let extensions = extensions.clone();
            calls.push(async move {
                let forwarded =
                    tonic::Request::from_parts(metadata, extensions, UpdateMetadataBatchRequest { requests }).into();
                match service.update_metadata_batch(forwarded).await {
                    Ok(mut response) => {
                        remap_batch_indices(&mut response.results, &indices);
                        response.results
                    },
                    // A top-level error from one branch must not discard the
                    // results from sibling branches that may have already
                    // succeeded. Synthesize a per-entry error for each of this
                    // branch's original indices so the caller sees them as
                    // failed entries rather than losing the whole batch.
                    Err(err) => {
                        let entry_error = batch_entry_error(err.as_ref());
                        indices
                            .into_iter()
                            .map(|request_index| UpdateMetadataBatchEntryResult {
                                request_index,
                                outcome: Some(update_metadata_batch_entry_result::Outcome::Error(entry_error.clone())),
                            })
                            .collect()
                    },
                }
            });
        }

        let grouped_results: Vec<Vec<UpdateMetadataBatchEntryResult>> = join_all(calls).await;
        Ok(UpdateMetadataBatchResponse {
            results: grouped_results.into_iter().flatten().collect(),
        })
    }

    #[instrument(name = "metadata_key_router::delete_metadata", skip_all)]
    async fn delete_metadata(&self, request: Request<DeleteMetadataRequest>) -> Result<DeleteMetadataResponse> {
        let target = self.route_for_key(&request.get_ref().user_metadata_key);
        match target {
            RouteTarget::Branch(i) => self.branches[i].service.delete_metadata(request).await,
            RouteTarget::Default => self.default.delete_metadata(request).await,
        }
    }
}

fn remap_batch_indices(results: &mut [UpdateMetadataBatchEntryResult], indices: &[u32]) {
    for result in results {
        if let Some(&original) = indices.get(result.request_index as usize) {
            result.request_index = original;
        }
    }
}

/// Merge per-branch responses with first-seen-wins on duplicate keys.
///
/// Call sites place branch responses before the default, so any key produced by
/// a routed branch beats the same key produced by the default. Duplicates
/// shouldn't happen in normal operation — the route table partitions keys among
/// branches — but a misconfiguration or an off-contract response from a branch
/// could produce one, and we'd rather succeed with a deterministic value than
/// fail the whole request.
fn merge_responses(uri: &str, responses: Vec<GetMetadataResponse>) -> GetMetadataResponse {
    let mut merged: HashMap<String, UserMetadataValue> = HashMap::new();
    for resp in responses {
        for (k, v) in resp.user_metadata {
            if merged.contains_key(&k) {
                info_limited!(
                    uri = %uri,
                    key = %k,
                    "duplicate metadata key returned by more than one branch; preserving first-seen value"
                );
                continue;
            }
            merged.insert(k, v);
        }
    }
    GetMetadataResponse { user_metadata: merged }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::metadata_key_router::router::Error as RouterError;
    use crate::metadata_key_router::MetadataKeyRouter;
    use storage_core_builder::traits::error::StorageError;
    use storage_core_builder::traits::metadata::v1alpha::{update_metadata_batch_entry_result, UserMetadataValue};
    use tonic::metadata::MetadataValue;

    /// Caller metadata key and extension that `get_metadata` must forward to every target.
    const CALLER_METADATA_KEY: &str = "x-caller-context";
    const CALLER_METADATA_VALUE: &str = "caller-token";

    #[derive(Clone, Debug, PartialEq)]
    struct CallerExtension(&'static str);

    /// The caller context one `get_metadata` call arrived with.
    #[derive(Debug, PartialEq)]
    struct SeenContext {
        metadata: Option<String>,
        extension: Option<CallerExtension>,
    }

    impl SeenContext {
        fn of<T>(request: &Request<T>) -> Self {
            Self {
                metadata: request
                    .metadata()
                    .get(CALLER_METADATA_KEY)
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_string),
                extension: request.extensions().get::<CallerExtension>().cloned(),
            }
        }

        fn caller() -> Self {
            Self {
                metadata: Some(CALLER_METADATA_VALUE.to_string()),
                extension: Some(CallerExtension("caller")),
            }
        }
    }

    /// Records every call and emits configurable responses. One per branch in tests.
    #[derive(Default)]
    struct StubInner {
        get_calls: Mutex<Vec<Vec<String>>>,
        get_contexts: Mutex<Vec<SeenContext>>,
        update_calls: Mutex<Vec<String>>,
        batch_calls: Mutex<Vec<Vec<String>>>,
        delete_calls: Mutex<Vec<String>>,
        /// Keys to include in the get_metadata response (each backed by a unique etag).
        get_response_keys: Mutex<Vec<String>>,
        /// If true, all methods return an error.
        fail: Mutex<bool>,
    }

    impl StubInner {
        fn new() -> Arc<Self> {
            Arc::new(Self::default())
        }

        fn with_get_response(keys: &[&str]) -> Arc<Self> {
            let s = Self::default();
            *s.get_response_keys.lock().unwrap() = keys.iter().map(|k| (*k).to_string()).collect();
            Arc::new(s)
        }

        fn set_fail(&self) {
            *self.fail.lock().unwrap() = true;
        }

        fn get_calls(&self) -> Vec<Vec<String>> {
            self.get_calls.lock().unwrap().clone()
        }

        fn take_get_contexts(&self) -> Vec<SeenContext> {
            std::mem::take(&mut *self.get_contexts.lock().unwrap())
        }

        fn update_calls(&self) -> Vec<String> {
            self.update_calls.lock().unwrap().clone()
        }

        fn batch_calls(&self) -> Vec<Vec<String>> {
            self.batch_calls.lock().unwrap().clone()
        }

        fn delete_calls(&self) -> Vec<String> {
            self.delete_calls.lock().unwrap().clone()
        }

        fn would_fail(&self) -> bool {
            *self.fail.lock().unwrap()
        }
    }

    #[async_trait]
    impl Metadata for StubInner {
        fn inner(&self) -> Option<&dyn Metadata> {
            None
        }

        async fn get_metadata(&self, request: Request<GetMetadataRequest>) -> Result<GetMetadataResponse> {
            self.get_contexts.lock().unwrap().push(SeenContext::of(&request));
            let req = request.into_inner();
            self.get_calls.lock().unwrap().push(req.user_metadata_keys.clone());
            if self.would_fail() {
                return Err(Box::new(RouterError::internal("stub failure")) as StorageError);
            }
            let keys = self.get_response_keys.lock().unwrap().clone();
            // If a non-empty list was requested, only return the intersection.
            let chosen: Vec<String> = if req.user_metadata_keys.is_empty() {
                keys
            } else {
                keys.into_iter()
                    .filter(|k| req.user_metadata_keys.contains(k))
                    .collect()
            };
            let user_metadata = chosen
                .into_iter()
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
            let req = request.into_inner();
            self.update_calls.lock().unwrap().push(req.user_metadata_key);
            if self.would_fail() {
                return Err(Box::new(RouterError::internal("stub failure")) as StorageError);
            }
            Ok(UpdateMetadataResponse { etag: "etag".into() })
        }

        async fn update_metadata_batch(
            &self,
            request: Request<UpdateMetadataBatchRequest>,
        ) -> Result<UpdateMetadataBatchResponse> {
            let req = request.into_inner();
            self.batch_calls.lock().unwrap().push(
                req.requests
                    .iter()
                    .map(|entry| entry.user_metadata_key.clone())
                    .collect(),
            );
            if self.would_fail() {
                return Err(Box::new(RouterError::internal("stub failure")) as StorageError);
            }
            let results = req
                .requests
                .iter()
                .enumerate()
                .map(|(request_index, _)| UpdateMetadataBatchEntryResult {
                    request_index: request_index as u32,
                    outcome: Some(update_metadata_batch_entry_result::Outcome::Success(
                        UpdateMetadataResponse {
                            etag: format!("etag-{request_index}"),
                        },
                    )),
                })
                .collect();
            Ok(UpdateMetadataBatchResponse { results })
        }

        async fn delete_metadata(&self, request: Request<DeleteMetadataRequest>) -> Result<DeleteMetadataResponse> {
            let req = request.into_inner();
            self.delete_calls.lock().unwrap().push(req.user_metadata_key);
            if self.would_fail() {
                return Err(Box::new(RouterError::internal("stub failure")) as StorageError);
            }
            Ok(DeleteMetadataResponse {})
        }
    }

    fn update_request(key: &str) -> Request<UpdateMetadataRequest> {
        Request::new(UpdateMetadataRequest {
            uri: "https://b.s3.amazonaws.com/k".into(),
            user_metadata_key: key.into(),
            user_metadata: None,
            expected_etag: None,
        })
    }

    fn delete_request(key: &str) -> Request<DeleteMetadataRequest> {
        Request::new(DeleteMetadataRequest {
            uri: "https://b.s3.amazonaws.com/k".into(),
            user_metadata_key: key.into(),
            expected_etag: None,
        })
    }

    fn get_request(keys: &[&str]) -> Request<GetMetadataRequest> {
        Request::new(GetMetadataRequest {
            uri: "https://b.s3.amazonaws.com/k".into(),
            user_metadata_keys: keys.iter().map(|k| (*k).to_string()).collect(),
        })
    }

    /// Build a router with branch A keys, branch B keys, plus a default. Returns the
    /// router and the three stubs.
    fn build_router(
        a_keys: &[&str],
        b_keys: &[&str],
    ) -> (MetadataKeyRouter, Arc<StubInner>, Arc<StubInner>, Arc<StubInner>) {
        let a = StubInner::new();
        let b = StubInner::new();
        let d = StubInner::new();
        let router = MetadataKeyRouter::builder()
            .route(a_keys.iter().map(|s| s.to_string()), a.clone())
            .route(b_keys.iter().map(|s| s.to_string()), b.clone())
            .default(d.clone())
            .build()
            .unwrap();
        (router, a, b, d)
    }

    // --- update_metadata / delete_metadata dispatch ----------------------------------

    #[tokio::test]
    async fn update_routes_to_branch_a() {
        let (router, a, b, d) = build_router(&["a-key"], &["b-key"]);

        router.update_metadata(update_request("a-key")).await.unwrap();

        assert_eq!(a.update_calls(), vec!["a-key".to_string()]);
        assert!(b.update_calls().is_empty());
        assert!(d.update_calls().is_empty());
    }

    #[tokio::test]
    async fn update_routes_to_branch_b() {
        let (router, a, b, d) = build_router(&["a-key"], &["b-key"]);

        router.update_metadata(update_request("b-key")).await.unwrap();

        assert!(a.update_calls().is_empty());
        assert_eq!(b.update_calls(), vec!["b-key".to_string()]);
        assert!(d.update_calls().is_empty());
    }

    #[tokio::test]
    async fn update_unrouted_key_falls_through_to_default() {
        let (router, a, b, d) = build_router(&["a-key"], &["b-key"]);

        router.update_metadata(update_request("not-routed")).await.unwrap();

        assert!(a.update_calls().is_empty());
        assert!(b.update_calls().is_empty());
        assert_eq!(d.update_calls(), vec!["not-routed".to_string()]);
    }

    #[tokio::test]
    async fn update_propagates_branch_error() {
        let (router, a, _b, _d) = build_router(&["a-key"], &["b-key"]);
        a.set_fail();

        let result = router.update_metadata(update_request("a-key")).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn update_batch_groups_by_branch_and_remaps_indices() {
        let (router, a, b, d) = build_router(&["a-key"], &["b-key"]);

        let response = router
            .update_metadata_batch(Request::new(UpdateMetadataBatchRequest {
                requests: vec![
                    UpdateMetadataRequest {
                        uri: "https://b.s3.amazonaws.com/k".into(),
                        user_metadata_key: "a-key".into(),
                        user_metadata: None,
                        expected_etag: None,
                    },
                    UpdateMetadataRequest {
                        uri: "https://b.s3.amazonaws.com/k".into(),
                        user_metadata_key: "default-key".into(),
                        user_metadata: None,
                        expected_etag: None,
                    },
                    UpdateMetadataRequest {
                        uri: "https://b.s3.amazonaws.com/k".into(),
                        user_metadata_key: "b-key".into(),
                        user_metadata: None,
                        expected_etag: None,
                    },
                    UpdateMetadataRequest {
                        uri: "https://b.s3.amazonaws.com/k".into(),
                        user_metadata_key: "a-key".into(),
                        user_metadata: None,
                        expected_etag: None,
                    },
                ],
            }))
            .await
            .unwrap();

        assert_eq!(a.batch_calls(), vec![vec!["a-key".to_string(), "a-key".to_string()]]);
        assert_eq!(b.batch_calls(), vec![vec!["b-key".to_string()]]);
        assert_eq!(d.batch_calls(), vec![vec!["default-key".to_string()]]);

        let mut indices: Vec<u32> = response
            .results
            .into_iter()
            .map(|result| result.request_index)
            .collect();
        indices.sort_unstable();
        assert_eq!(indices, vec![0, 1, 2, 3]);
    }

    #[tokio::test]
    async fn delete_routes_to_branch_a() {
        let (router, a, b, d) = build_router(&["a-key"], &["b-key"]);

        router.delete_metadata(delete_request("a-key")).await.unwrap();

        assert_eq!(a.delete_calls(), vec!["a-key".to_string()]);
        assert!(b.delete_calls().is_empty());
        assert!(d.delete_calls().is_empty());
    }

    #[tokio::test]
    async fn delete_routes_to_branch_b() {
        let (router, a, b, d) = build_router(&["a-key"], &["b-key"]);

        router.delete_metadata(delete_request("b-key")).await.unwrap();

        assert!(a.delete_calls().is_empty());
        assert_eq!(b.delete_calls(), vec!["b-key".to_string()]);
        assert!(d.delete_calls().is_empty());
    }

    #[tokio::test]
    async fn delete_unrouted_key_falls_through_to_default() {
        let (router, a, b, d) = build_router(&["a-key"], &["b-key"]);

        router.delete_metadata(delete_request("not-routed")).await.unwrap();

        assert!(a.delete_calls().is_empty());
        assert!(b.delete_calls().is_empty());
        assert_eq!(d.delete_calls(), vec!["not-routed".to_string()]);
    }

    #[tokio::test]
    async fn delete_propagates_branch_error() {
        let (router, _a, _b, d) = build_router(&["a-key"], &["b-key"]);
        d.set_fail();

        let result = router.delete_metadata(delete_request("not-routed")).await;
        assert!(result.is_err());
    }

    // --- inner ----------------------------------------------------------------------

    #[test]
    fn inner_is_none() {
        let (router, _a, _b, _d) = build_router(&["a"], &["b"]);
        assert!(router.inner().is_none());
    }

    // --- get_metadata, non-empty keys -----------------------------------------------

    /// Build a router with three configurable stubs and known response keys.
    fn build_router_with_responses(
        a_keys: &[&str],
        a_response: &[&str],
        b_keys: &[&str],
        b_response: &[&str],
        d_response: &[&str],
    ) -> (MetadataKeyRouter, Arc<StubInner>, Arc<StubInner>, Arc<StubInner>) {
        let a = StubInner::with_get_response(a_response);
        let b = StubInner::with_get_response(b_response);
        let d = StubInner::with_get_response(d_response);
        let router = MetadataKeyRouter::builder()
            .route(a_keys.iter().map(|s| s.to_string()), a.clone())
            .route(b_keys.iter().map(|s| s.to_string()), b.clone())
            .default(d.clone())
            .build()
            .unwrap();
        (router, a, b, d)
    }

    fn keys_set(calls: Vec<Vec<String>>) -> Vec<HashMap<String, ()>> {
        calls
            .into_iter()
            .map(|v| v.into_iter().map(|k| (k, ())).collect())
            .collect()
    }

    #[tokio::test]
    async fn get_all_keys_in_one_branch_only_calls_that_branch() {
        let (router, a, b, d) = build_router_with_responses(&["a1", "a2"], &["a1", "a2"], &["b1"], &["b1"], &["other"]);

        let response = router.get_metadata(get_request(&["a1", "a2"])).await.unwrap();

        let a_calls = keys_set(a.get_calls());
        assert_eq!(a_calls.len(), 1);
        assert!(a_calls[0].contains_key("a1") && a_calls[0].contains_key("a2"));
        assert!(b.get_calls().is_empty());
        assert!(d.get_calls().is_empty());
        assert!(response.user_metadata.contains_key("a1"));
        assert!(response.user_metadata.contains_key("a2"));
        assert_eq!(response.user_metadata.len(), 2);
    }

    #[tokio::test]
    async fn get_keys_split_across_a_and_b_calls_both_not_default() {
        let (router, a, b, d) = build_router_with_responses(&["a1"], &["a1"], &["b1"], &["b1"], &["other"]);

        let response = router.get_metadata(get_request(&["a1", "b1"])).await.unwrap();

        let a_calls = a.get_calls();
        let b_calls = b.get_calls();
        assert_eq!(a_calls, vec![vec!["a1".to_string()]]);
        assert_eq!(b_calls, vec![vec!["b1".to_string()]]);
        assert!(
            d.get_calls().is_empty(),
            "default must not be called when no keys land there"
        );

        assert!(response.user_metadata.contains_key("a1"));
        assert!(response.user_metadata.contains_key("b1"));
    }

    #[tokio::test]
    async fn get_routed_and_unrouted_calls_branches_and_default() {
        let (router, a, b, d) = build_router_with_responses(&["a1"], &["a1"], &["b1"], &["b1"], &["unrouted"]);

        let response = router
            .get_metadata(get_request(&["a1", "b1", "unrouted"]))
            .await
            .unwrap();

        assert_eq!(a.get_calls(), vec![vec!["a1".to_string()]]);
        assert_eq!(b.get_calls(), vec![vec!["b1".to_string()]]);
        assert_eq!(d.get_calls(), vec![vec!["unrouted".to_string()]]);
        assert!(response.user_metadata.contains_key("a1"));
        assert!(response.user_metadata.contains_key("b1"));
        assert!(response.user_metadata.contains_key("unrouted"));
    }

    #[tokio::test]
    async fn get_all_unrouted_only_calls_default() {
        let (router, a, b, d) = build_router_with_responses(&["a1"], &["a1"], &["b1"], &["b1"], &["x", "y"]);

        let response = router.get_metadata(get_request(&["x", "y"])).await.unwrap();

        assert!(a.get_calls().is_empty());
        assert!(b.get_calls().is_empty());
        let d_calls = keys_set(d.get_calls());
        assert_eq!(d_calls.len(), 1);
        assert!(d_calls[0].contains_key("x") && d_calls[0].contains_key("y"));
        assert!(response.user_metadata.contains_key("x"));
        assert!(response.user_metadata.contains_key("y"));
    }

    #[tokio::test]
    async fn get_propagates_branch_error() {
        let (router, a, _b, _d) = build_router_with_responses(&["a1"], &["a1"], &["b1"], &["b1"], &["unrouted"]);
        a.set_fail();

        let result = router.get_metadata(get_request(&["a1", "b1"])).await;
        assert!(result.is_err());
    }

    // --- get_metadata, empty keys (fan-out) -----------------------------------------

    #[tokio::test]
    async fn get_empty_keys_asks_each_branch_for_its_keys_and_default_for_everything() {
        let (router, a, b, d) =
            build_router_with_responses(&["a1", "a2"], &["a1", "a2"], &["b1"], &["b1"], &["d1", "d2"]);

        let response = router.get_metadata(get_request(&[])).await.unwrap();

        // Branch A was asked for its configured keys (set, not order).
        let a_calls = keys_set(a.get_calls());
        assert_eq!(a_calls.len(), 1);
        assert!(a_calls[0].contains_key("a1") && a_calls[0].contains_key("a2"));
        assert_eq!(a_calls[0].len(), 2);

        // Branch B was asked for its single key.
        assert_eq!(b.get_calls(), vec![vec!["b1".to_string()]]);

        // Default was asked with empty keys (give-me-all).
        assert_eq!(d.get_calls(), vec![Vec::<String>::new()]);

        // Response merges all four keys.
        assert!(response.user_metadata.contains_key("a1"));
        assert!(response.user_metadata.contains_key("a2"));
        assert!(response.user_metadata.contains_key("b1"));
        assert!(response.user_metadata.contains_key("d1"));
        assert!(response.user_metadata.contains_key("d2"));
        assert_eq!(response.user_metadata.len(), 5);
    }

    #[tokio::test]
    async fn get_empty_keys_propagates_branch_error() {
        let (router, _a, b, _d) = build_router_with_responses(&["a1"], &["a1"], &["b1"], &["b1"], &["d1"]);
        b.set_fail();

        let result = router.get_metadata(get_request(&[])).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn get_empty_keys_propagates_default_error() {
        let (router, _a, _b, d) = build_router_with_responses(&["a1"], &["a1"], &["b1"], &["b1"], &["d1"]);
        d.set_fail();

        let result = router.get_metadata(get_request(&[])).await;
        assert!(result.is_err());
    }

    // --- get_metadata, caller context -----------------------------------------------

    fn get_request_with_caller_context(keys: &[&str]) -> Request<GetMetadataRequest> {
        let mut request = get_request(keys);
        request
            .metadata_mut()
            .insert(CALLER_METADATA_KEY, MetadataValue::from_static(CALLER_METADATA_VALUE));
        request.extensions_mut().insert(CallerExtension("caller"));
        request
    }

    fn assert_each_saw_caller_context(stubs: [(&str, &StubInner); 3]) {
        for (name, stub) in stubs {
            assert_eq!(
                stub.take_get_contexts(),
                vec![SeenContext::caller()],
                "{name} must receive the caller's metadata and extensions"
            );
        }
    }

    #[tokio::test]
    async fn get_empty_keys_forwards_caller_context_to_every_branch_and_default() {
        let (router, a, b, d) = build_router_with_responses(&["a1"], &["a1"], &["b1"], &["b1"], &["d1"]);

        router.get_metadata(get_request_with_caller_context(&[])).await.unwrap();

        assert_each_saw_caller_context([("branch a", &a), ("branch b", &b), ("default", &d)]);
    }

    #[tokio::test]
    async fn get_bucketed_keys_forwards_caller_context_to_every_branch_and_default() {
        let (router, a, b, d) = build_router_with_responses(&["a1"], &["a1"], &["b1"], &["b1"], &["unrouted"]);

        router
            .get_metadata(get_request_with_caller_context(&["a1", "b1", "unrouted"]))
            .await
            .unwrap();

        assert_each_saw_caller_context([("branch a", &a), ("branch b", &b), ("default", &d)]);
    }

    #[tokio::test]
    async fn get_response_absent_key_is_treated_as_absent() {
        // Branch A configured for two keys but only returns one in its response.
        let (router, a, _b, d) = build_router_with_responses(&["a1", "a2"], &["a1"], &["b1"], &["b1"], &["unrouted"]);

        let response = router.get_metadata(get_request(&["a1", "a2"])).await.unwrap();

        let a_calls = keys_set(a.get_calls());
        assert_eq!(a_calls.len(), 1);
        assert!(a_calls[0].contains_key("a1") && a_calls[0].contains_key("a2"));
        assert!(d.get_calls().is_empty());
        assert!(response.user_metadata.contains_key("a1"));
        assert!(!response.user_metadata.contains_key("a2"));
    }

    // --- merge collision behavior --------------------------------------------------

    #[tokio::test]
    async fn get_empty_keys_with_duplicate_branch_default_keys_does_not_error() {
        // Defensive: if the default ever returns a key that's also in a branch's
        // set (misconfiguration, off-contract response), the merge should succeed
        // with the branch's value rather than fail the whole request.
        let (router, a, _b, d) = build_router_with_responses(
            &["routed"], // branch a's routed keys
            &["routed"], // branch a returns "routed"
            &[],
            &[],
            &["routed", "default-only"], // default also returns "routed"
        );

        let response = router.get_metadata(get_request(&[])).await.unwrap();

        assert_eq!(response.user_metadata.len(), 2);
        assert!(response.user_metadata.contains_key("routed"));
        assert!(response.user_metadata.contains_key("default-only"));
        assert!(!a.get_calls().is_empty(), "branch must have been called");
        assert!(!d.get_calls().is_empty(), "default must have been called");
    }

    #[test]
    fn merge_responses_first_seen_wins() {
        // Direct test of the merge function: when two responses carry the same key,
        // the first-listed (branch) wins and the second (default) is dropped.
        let branch = GetMetadataResponse {
            user_metadata: [(
                "routed".to_string(),
                UserMetadataValue {
                    value: None,
                    etag: "branch-etag".to_string(),
                },
            )]
            .into_iter()
            .collect(),
        };
        let default = GetMetadataResponse {
            user_metadata: [
                (
                    "routed".to_string(),
                    UserMetadataValue {
                        value: None,
                        etag: "default-etag".to_string(),
                    },
                ),
                (
                    "default-only".to_string(),
                    UserMetadataValue {
                        value: None,
                        etag: "default-only-etag".to_string(),
                    },
                ),
            ]
            .into_iter()
            .collect(),
        };

        let merged = super::merge_responses("https://example/k", vec![branch, default]);

        assert_eq!(merged.user_metadata.len(), 2);
        assert_eq!(
            merged.user_metadata.get("routed").unwrap().etag,
            "branch-etag",
            "first-listed (branch) value wins on duplicate"
        );
        assert_eq!(
            merged.user_metadata.get("default-only").unwrap().etag,
            "default-only-etag"
        );
    }
}
