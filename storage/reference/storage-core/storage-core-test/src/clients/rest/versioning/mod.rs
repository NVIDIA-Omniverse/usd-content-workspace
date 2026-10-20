// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

pub mod v1alpha;
pub mod v1beta;

use bytes::Bytes;
use reqwest::Client;
use tonic::Status;
use urlencoding::encode;

use super::{check_status_code, headers_to_header_map, request_with_retries};
use crate::headers::HeadersConfig;

#[derive(Clone)]
pub struct VersioningRestClient {
    pub client: Client,
    pub port: u16,
    pub headers_config: HeadersConfig,
}

impl VersioningRestClient {
    pub fn new(port: u16, headers_config: HeadersConfig) -> Self {
        Self {
            client: Client::new(),
            port,
            headers_config,
        }
    }

    pub async fn enumerate_versions(&mut self, path: &str, continuation_handle: Option<&str>) -> Result<Bytes, Status> {
        let mut url = format!("http://localhost:{}/{}", self.port, path);

        if let Some(continuation_handle) = continuation_handle {
            url.push_str(&format!("?continuation_handle={}", encode(continuation_handle)));
        }

        let response = request_with_retries(|| {
            self.client
                .get(url.clone())
                .headers(headers_to_header_map(self.headers_config.get_headers()))
                .send()
        })
        .await
        .map_err(|e| check_status_code(e.status().unwrap()).unwrap_err())?;

        let status = response.status();
        check_status_code(status)?;

        let body = response.bytes().await.unwrap();
        Ok(body)
    }

    pub async fn is_versioned(&mut self, path: &str) -> Result<Bytes, Status> {
        let url = format!("http://localhost:{}/{}", self.port, path);

        let response = request_with_retries(|| {
            self.client
                .get(url.clone())
                .headers(headers_to_header_map(self.headers_config.get_headers()))
                .send()
        })
        .await
        .map_err(|e| check_status_code(e.status().unwrap()).unwrap_err())?;

        let status = response.status();
        check_status_code(status)?;

        let body = response.bytes().await.unwrap();
        Ok(body)
    }
}
