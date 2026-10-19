// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

pub mod v1alpha;
pub mod v1beta;

use bytes::Bytes;
use reqwest::{Client, Response};
use serde::Serialize;
use tonic::Status;
use url::Url;
use urlencoding::encode;

use super::{check_status_code, headers_to_header_map, request_with_retries};
use crate::headers::HeadersConfig;

#[derive(Clone)]
pub struct FileObjectRestClient {
    pub client: Client,
    pub port: u16,
    pub headers_config: HeadersConfig,
}

impl FileObjectRestClient {
    pub fn new(port: u16, headers_config: HeadersConfig) -> Self {
        Self {
            client: Client::new(),
            port,
            headers_config,
        }
    }

    pub async fn stat(&mut self, path: &str) -> Result<Response, Status> {
        let url = format!("http://localhost:{}/{}", self.port, path);

        let response = request_with_retries(|| {
            self.client
                .head(url.clone())
                .headers(headers_to_header_map(self.headers_config.get_headers()))
                .send()
        })
        .await
        .map_err(|e| check_status_code(e.status().unwrap()).unwrap_err())?;

        let status = response.status();
        check_status_code(status)?;

        Ok(response)
    }

    pub async fn read_from_address(
        &mut self,
        path: &str,
        download_preference: Option<&str>,
    ) -> Result<Response, Status> {
        let mut url = format!("http://localhost:{}/{}", self.port, path);

        if let Some(pref) = download_preference {
            url.push_str(&format!("?download_preference={}", pref));
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

        Ok(response)
    }

    pub async fn read(&mut self, path: &str, download_preference: Option<&str>) -> Result<Response, Status> {
        let mut url = format!("http://localhost:{}/{}", self.port, path);

        if let Some(pref) = download_preference {
            url.push_str(&format!("?download_preference={}", pref));
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

        Ok(response)
    }

    pub async fn enumerate(&mut self, path: &str, continuation_handle: Option<&str>) -> Result<Bytes, Status> {
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

    pub async fn write(
        &mut self,
        path: &str,
        data_object_size: u64,
        upload_preference: Option<&str>,
        previous_version: Option<&str>,
        fail_if_exists: Option<bool>,
        data: Option<Bytes>,
    ) -> Result<Response, Status> {
        let mut url = Url::parse(&format!("http://localhost:{}/{}", self.port, path)).unwrap();

        url.query_pairs_mut()
            .append_pair("data_object_size", &data_object_size.to_string());

        if let Some(pref) = upload_preference {
            url.query_pairs_mut().append_pair("upload_preference", pref);
        }

        if let Some(prev) = previous_version {
            url.query_pairs_mut().append_pair("previous_version", prev);
        }

        if let Some(fail_if_exists) = fail_if_exists {
            url.query_pairs_mut()
                .append_pair("fail_if_exists", &fail_if_exists.to_string());
        }

        let response = request_with_retries(|| {
            let mut builder = self
                .client
                .put(url.clone())
                .headers(headers_to_header_map(self.headers_config.get_headers()));
            if let Some(data) = &data {
                builder = builder.body(data.clone());
            }
            builder.send()
        })
        .await
        .map_err(|e| check_status_code(e.status().unwrap()).unwrap_err())?;

        let status = response.status();
        check_status_code(status)?;

        Ok(response)
    }

    pub async fn delete(&mut self, path: &str, previous_version: Option<&str>) -> Result<(), Status> {
        let mut url = format!("http://localhost:{}/{}", self.port, path);

        if let Some(prev) = previous_version {
            url.push_str(&format!("?previous_version={}", encode(prev)));
        }

        let response = request_with_retries(|| {
            self.client
                .delete(url.clone())
                .headers(headers_to_header_map(self.headers_config.get_headers()))
                .send()
        })
        .await
        .map_err(|e| check_status_code(e.status().unwrap()).unwrap_err())?;

        let status = response.status();
        check_status_code(status)?;

        Ok(())
    }

    pub async fn abort_multipart_upload<T>(&mut self, path: &str, body: T) -> Result<(), Status>
    where
        T: Serialize,
    {
        let url = format!("http://localhost:{}/{}", self.port, path);

        let response = request_with_retries(|| {
            self.client
                .post(url.clone())
                .headers(headers_to_header_map(self.headers_config.get_headers()))
                .json(&body)
                .send()
        })
        .await
        .map_err(|e| check_status_code(e.status().unwrap()).unwrap_err())?;

        let status = response.status();
        check_status_code(status)?;

        Ok(())
    }

    pub async fn complete_multipart_upload<T>(&mut self, path: &str, body: T) -> Result<Bytes, Status>
    where
        T: Serialize,
    {
        let url = format!("http://localhost:{}/{}", self.port, path);

        let response = request_with_retries(|| {
            self.client
                .post(url.clone())
                .headers(headers_to_header_map(self.headers_config.get_headers()))
                .json(&body)
                .send()
        })
        .await
        .map_err(|e| check_status_code(e.status().unwrap()).unwrap_err())?;

        let status = response.status();
        check_status_code(status)?;

        let body = response.bytes().await.unwrap();
        Ok(body)
    }

    pub async fn upload_part<T>(&mut self, path: &str, body: T) -> Result<Bytes, Status>
    where
        T: Serialize,
    {
        let url = format!("http://localhost:{}/{}", self.port, path);

        let response = request_with_retries(|| {
            self.client
                .post(url.clone())
                .headers(headers_to_header_map(self.headers_config.get_headers()))
                .json(&body)
                .send()
        })
        .await
        .map_err(|e| check_status_code(e.status().unwrap()).unwrap_err())?;
        let status = response.status();
        check_status_code(status)?;

        let body = response.bytes().await.unwrap();
        Ok(body)
    }

    pub async fn complete_redirect_upload<T>(&mut self, path: &str, body: T) -> Result<Bytes, Status>
    where
        T: Serialize,
    {
        let url = format!("http://localhost:{}/{}", self.port, path);

        let response = request_with_retries(|| {
            self.client
                .post(url.clone())
                .headers(headers_to_header_map(self.headers_config.get_headers()))
                .json(&body)
                .send()
        })
        .await
        .map_err(|e| check_status_code(e.status().unwrap()).unwrap_err())?;

        let status = response.status();
        check_status_code(status)?;

        let body = response.bytes().await.unwrap();
        Ok(body)
    }

    pub async fn fetch_write_type_info(&mut self, path: &str) -> Result<Bytes, Status> {
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

    pub async fn get_optimistic_locking_support(&mut self, path: &str) -> Result<Bytes, Status> {
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

    pub async fn get_native_metadata_support(&mut self, path: &str) -> Result<Bytes, Status> {
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

    pub async fn is_read_only(&mut self, path: &str) -> Result<Bytes, Status> {
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

    pub async fn copy<T>(&mut self, path: &str, body: T) -> Result<Bytes, Status>
    where
        T: Serialize,
    {
        let url = format!("http://localhost:{}/{}", self.port, path);

        let response = request_with_retries(|| {
            self.client
                .post(url.clone())
                .headers(headers_to_header_map(self.headers_config.get_headers()))
                .json(&body)
                .send()
        })
        .await
        .map_err(|e| check_status_code(e.status().unwrap()).unwrap_err())?;

        let status = response.status();
        check_status_code(status)?;

        let body = response.bytes().await.unwrap();
        Ok(body)
    }

    pub async fn move_object<T>(&mut self, path: &str, body: T) -> Result<Bytes, Status>
    where
        T: Serialize,
    {
        let url = format!("http://localhost:{}/{}", self.port, path);

        let response = request_with_retries(|| {
            self.client
                .post(url.clone())
                .headers(headers_to_header_map(self.headers_config.get_headers()))
                .json(&body)
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
