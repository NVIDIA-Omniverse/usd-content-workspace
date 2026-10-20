// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

pub mod v1alpha;

use bytes::Bytes;
use reqwest::Client;
use serde::Serialize;
use tonic::Status;

use super::{check_status_code, headers_to_header_map, request_with_retries};
use crate::headers::HeadersConfig;

pub(crate) struct MetadataRestClient {
    client: Client,
    port: u16,
    headers_config: HeadersConfig,
}

impl MetadataRestClient {
    pub fn new(port: u16, headers_config: HeadersConfig) -> Self {
        Self {
            client: Client::new(),
            port,
            headers_config,
        }
    }

    pub async fn get_metadata(&self, path: &str, body: Vec<String>) -> Result<Bytes, Status> {
        let url = format!("http://localhost:{}/{}", self.port, path);

        let response = request_with_retries(|| {
            self.client
                .post(url.clone())
                .json(&body)
                .headers(headers_to_header_map(self.headers_config.get_headers()))
                .send()
        })
        .await
        .map_err(|e| check_status_code(e.status().unwrap()).unwrap_err())?;

        check_status_code(response.status())?;
        let body = response.bytes().await.unwrap();
        Ok(body)
    }

    pub async fn update_metadata(
        &self,
        path: &str,
        value: &impl Serialize,
        expected_etag: Option<&str>,
    ) -> Result<String, Status> {
        let url = format!("http://localhost:{}/{}", self.port, path);

        let response = request_with_retries(|| {
            let mut builder = self
                .client
                .put(url.clone())
                .json(value)
                .headers(headers_to_header_map(self.headers_config.get_headers()));
            if let Some(etag) = expected_etag {
                builder = builder.header("If-Match", etag);
            }
            builder.send()
        })
        .await
        .map_err(|e| check_status_code(e.status().unwrap()).unwrap_err())?;

        check_status_code(response.status())?;
        let etag = response
            .headers()
            .get("etag")
            .map(|v| v.to_str().unwrap().to_string())
            .unwrap_or_default();
        Ok(etag)
    }

    pub async fn update_metadata_batch(&self, path: &str, body: &impl Serialize) -> Result<Bytes, Status> {
        let url = format!("http://localhost:{}/{}", self.port, path);

        let response = request_with_retries(|| {
            self.client
                .post(url.clone())
                .json(body)
                .headers(headers_to_header_map(self.headers_config.get_headers()))
                .send()
        })
        .await
        .map_err(|e| check_status_code(e.status().unwrap()).unwrap_err())?;

        check_status_code(response.status())?;
        let body = response.bytes().await.unwrap();
        Ok(body)
    }

    pub async fn delete_metadata(&self, path: &str, expected_etag: Option<&str>) -> Result<(), Status> {
        let url = format!("http://localhost:{}/{}", self.port, path);

        let response = request_with_retries(|| {
            let mut builder = self
                .client
                .delete(url.clone())
                .headers(headers_to_header_map(self.headers_config.get_headers()));
            if let Some(etag) = expected_etag {
                builder = builder.header("If-Match", etag);
            }
            builder.send()
        })
        .await
        .map_err(|e| check_status_code(e.status().unwrap()).unwrap_err())?;

        check_status_code(response.status())?;
        Ok(())
    }
}
