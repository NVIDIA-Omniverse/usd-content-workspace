// SPDX-FileCopyrightText: Copyright (c) 2024 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use parking_lot::Mutex;
use std::sync::Arc;

const CACHE_CONTROL: &str = "cache-control";

/// A list of headers to add to each request.
///
/// This is intentionally not a http::HeaderMap because HeaderName and HeaderValue cannot be directly
/// applied to Azure/S3 requests, so using those types only adds overhead without any benefit.
///
/// The Arc<Mutex<Option<>>> is used because the S3 SDK `mutate_request` method is an `Fn`, not `FnMut`,
/// and requires Send + Sync, so we need Arc<Mutex<>> to make the type Send + Sync, and Option<> to allow
/// allow the headers to be moved out in the non-mutable `Fn` closure.
type HeaderList = Arc<Mutex<Option<Vec<(String, String)>>>>;

/// Per request context information
#[derive(Default, Debug)]
pub struct AdditionalHeaders {
    // Headers to add to each request.
    request_headers: HeaderList,
    cache_control: Option<String>,
    cache_control_api: Option<String>,
    env_headers: Vec<(String, String)>,
}

impl AdditionalHeaders {
    pub fn new(
        request_headers: Vec<(String, String)>,
        cache_control: Option<String>,
        cache_control_api: Option<String>,
        env_headers: Vec<(String, String)>,
    ) -> Self {
        Self {
            request_headers: Arc::new(Mutex::new(Some(request_headers))),
            cache_control,
            cache_control_api,
            env_headers,
        }
    }

    pub fn request_headers(&self, request_type: RequestType) -> impl Iterator<Item = (String, String)> {
        let mut headers = self.request_headers.lock().take().unwrap_or_default();

        // Add env var headers (apply to all request types)
        headers.extend(self.env_headers.iter().cloned());

        // Add cache-control header if present. The clone here is again working around the
        // immutability of the `Fn` closure in the S3 SDK. We could do a similar Arc<Mutex<>>
        // dance here, but the cache_control_api string should be small, so the clone is probably
        // cheaper than the allocations/locks/atomic operations of an Arc<Mutex<>>.
        if let Some(cache_control) = match request_type {
            RequestType::Api => self.cache_control_api.as_deref(),
            RequestType::Data => self.cache_control.as_deref(),
        } {
            headers.push((CACHE_CONTROL.to_string(), cache_control.to_string()));
        }

        headers.into_iter()
    }
}

impl Clone for AdditionalHeaders {
    fn clone(&self) -> Self {
        Self {
            cache_control: self.cache_control.clone(),
            cache_control_api: self.cache_control_api.clone(),
            request_headers: Arc::new(Mutex::new(self.request_headers.lock().clone())),
            env_headers: self.env_headers.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum RequestType {
    /// Api requests, these will use the cache_control_api header
    Api,
    /// Requests to Get or Put data, these will use the cache_control header
    Data,
}
