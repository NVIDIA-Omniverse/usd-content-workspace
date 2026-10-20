// SPDX-FileCopyrightText: Copyright (c) 2024 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
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

pub use prometheus_parse::{Sample, Scrape, Value};

use crate::common::clients::utils::rest::RequestBuilderExt;

#[derive(Debug, Clone)]
pub struct Client {
    endpoint: String,
}

impl Client {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
        }
    }

    async fn get_cache_metrics(&self, name: impl AsRef<str>) -> CacheMetrics {
        CacheMetrics::from_samples(
            self.scrape()
                .await
                .samples
                .iter()
                .filter(|sample| sample.labels.get("name") == Some(name.as_ref())),
        )
    }

    pub async fn get_stat_cache_metrics(&self) -> CacheMetrics {
        self.get_cache_metrics("stat_cache").await
    }

    pub async fn get_list_cache_metrics(&self) -> CacheMetrics {
        self.get_cache_metrics("list_cache").await
    }

    pub async fn get_dynamodb_metadata_backend_metrics(&self) -> DynamoDBMetadataBackendMetrics {
        DynamoDBMetadataBackendMetrics::from_samples(self.scrape().await.samples.iter())
    }

    pub async fn get_cosmosdb_metadata_backend_metrics(&self) -> CosmosDBMetadataBackendMetrics {
        CosmosDBMetadataBackendMetrics::from_samples(self.scrape().await.samples.iter())
    }

    pub async fn get_postgres_metadata_backend_metrics(&self) -> PostgresMetadataBackendMetrics {
        PostgresMetadataBackendMetrics::from_samples(self.scrape().await.samples.iter())
    }

    pub async fn scrape(&self) -> Scrape {
        let client = reqwest::Client::new();
        let response = client
            .get(&self.endpoint)
            .send_with_retry()
            .await
            .expect("failed to send a request");
        let response_body = response.text().await.expect("failed to read response body");

        Scrape::parse(response_body.lines().map(|line| Ok(line.to_owned()))).expect("failed to parse a response")
    }
}

pub trait UnwrapValueExt {
    fn unwrap_value(&self) -> u64;
}

impl UnwrapValueExt for Value {
    #[track_caller]
    fn unwrap_value(&self) -> u64 {
        match self {
            Value::Counter(value) => *value as u64,
            Value::Untyped(value) => *value as u64,
            Value::Gauge(value) => *value as u64,
            Value::Histogram(_) => panic!("counter/untyped/gauge value expected, got histogram"),
            Value::Summary(_) => panic!("counter/untyped/gauge value expected, got summary"),
        }
    }
}

impl UnwrapValueExt for Sample {
    #[track_caller]
    fn unwrap_value(&self) -> u64 {
        self.value.unwrap_value()
    }
}

#[derive(Debug, Clone)]
pub struct CacheMetrics {
    pub cache_access: u64,
    pub cache_miss: u64,
}

impl CacheMetrics {
    pub fn from_samples<'a>(samples: impl IntoIterator<Item = &'a Sample> + 'a) -> Self {
        let mut maybe_cache_access = None;
        let mut maybe_cache_miss = None;
        for sample in samples {
            if sample.metric == "storage_cache_operations_total" {
                match sample.labels.get("access_type") {
                    Some("access") => maybe_cache_access = Some(sample.unwrap_value()),
                    Some("miss") => maybe_cache_miss = Some(sample.unwrap_value()),
                    _ => continue,
                }
            }
        }

        Self {
            cache_access: maybe_cache_access.unwrap(),
            cache_miss: maybe_cache_miss.unwrap(),
        }
    }
}

pub struct DynamoDBMetadataBackendMetrics {
    pub put_item_count: u64,
    pub delete_item_count: u64,
    pub batch_get_item_count: u64,
    pub query_count: u64,
}

impl DynamoDBMetadataBackendMetrics {
    pub fn from_samples<'a>(samples: impl IntoIterator<Item = &'a Sample> + 'a) -> Self {
        let mut maybe_put_item_count = None;
        let mut maybe_delete_item_count = None;
        let mut maybe_batch_get_item_count = None;
        let mut maybe_query_count = None;
        for sample in samples {
            if sample.metric == "storage_aws_dynamodb_metadata_operation_latency_count" {
                match sample.labels.get("operation") {
                    Some("put_item") => maybe_put_item_count = Some(sample.unwrap_value()),
                    Some("delete_item") => maybe_delete_item_count = Some(sample.unwrap_value()),
                    Some("batch_get_item") => maybe_batch_get_item_count = Some(sample.unwrap_value()),
                    Some("query") => maybe_query_count = Some(sample.unwrap_value()),
                    _ => continue,
                }
            }
        }

        Self {
            put_item_count: maybe_put_item_count.unwrap(),
            delete_item_count: maybe_delete_item_count.unwrap(),
            batch_get_item_count: maybe_batch_get_item_count.unwrap(),
            query_count: maybe_query_count.unwrap(),
        }
    }
}

pub struct CosmosDBMetadataBackendMetrics {
    pub get_document_count: u64,
    pub query_documents_count: u64,
    pub create_document_count: u64,
    pub delete_document_count: u64,
}

impl CosmosDBMetadataBackendMetrics {
    pub fn from_samples<'a>(samples: impl IntoIterator<Item = &'a Sample> + 'a) -> Self {
        let mut maybe_get_document_count = None;
        let mut maybe_query_documents_count = None;
        let mut maybe_create_documents_count = None;
        let mut maybe_delete_documents_count = None;
        for sample in samples {
            if sample.metric == "storage_azure_cosmosdb_metadata_operation_latency_count" {
                match sample.labels.get("operation") {
                    Some("query_documents") => maybe_query_documents_count = Some(sample.unwrap_value()),
                    Some("create_document") => maybe_create_documents_count = Some(sample.unwrap_value()),
                    Some("delete_document") => maybe_delete_documents_count = Some(sample.unwrap_value()),
                    Some("get_document") => maybe_get_document_count = Some(sample.unwrap_value()),
                    _ => continue,
                }
            }
        }

        Self {
            get_document_count: maybe_get_document_count.unwrap(),
            query_documents_count: maybe_query_documents_count.unwrap(),
            create_document_count: maybe_create_documents_count.unwrap(),
            delete_document_count: maybe_delete_documents_count.unwrap(),
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct PostgresMetadataBackendMetrics {
    pub query_all_metadata_for_resource_count: u64,
    pub query_metadata_batch_count: u64,
    pub upsert_metadata_batch_count: u64,
    pub update_metadata_batch_count: u64,
    pub delete_metadata_batch_count: u64,
}

impl PostgresMetadataBackendMetrics {
    pub fn from_samples<'a>(samples: impl IntoIterator<Item = &'a Sample> + 'a) -> Self {
        let mut maybe_query_all_metadata_for_resource_count = None;
        let mut maybe_query_metadata_batch_count = None;
        let mut maybe_upsert_metadata_batch_count = None;
        let mut maybe_update_metadata_batch_count = None;
        let mut maybe_delete_metadata_batch_count = None;
        for sample in samples.into_iter() {
            if sample.metric != "storage_postgres_metadata_query_latency_count" {
                continue;
            }

            let Some(label) = sample.labels.get("query") else {
                continue;
            };

            match label {
                "query_all_metadata_for_resource" => {
                    maybe_query_all_metadata_for_resource_count = Some(sample.unwrap_value())
                },
                "query_metadata_batch" => maybe_query_metadata_batch_count = Some(sample.unwrap_value()),
                "upsert_metadata_batch" => maybe_upsert_metadata_batch_count = Some(sample.unwrap_value()),
                "update_metadata_batch" => maybe_update_metadata_batch_count = Some(sample.unwrap_value()),
                "delete_metadata_batch" => maybe_delete_metadata_batch_count = Some(sample.unwrap_value()),
                _ => {},
            }
        }

        Self {
            query_all_metadata_for_resource_count: maybe_query_all_metadata_for_resource_count.unwrap(),
            query_metadata_batch_count: maybe_query_metadata_batch_count.unwrap(),
            upsert_metadata_batch_count: maybe_upsert_metadata_batch_count.unwrap(),
            update_metadata_batch_count: maybe_update_metadata_batch_count.unwrap(),
            delete_metadata_batch_count: maybe_delete_metadata_batch_count.unwrap(),
        }
    }
}

pub async fn get_metrics(port: u16) -> Scrape {
    let mut retries = 0;
    let response = loop {
        match reqwest::get(format!("http://localhost:{port}/metrics")).await {
            Ok(r) => break r,
            Err(e) => {
                tracing::debug!("Failed request {:?}", e);
                tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                retries += 1;
                if retries > 10 {
                    panic!("Failed to connect to metrics server");
                }
            },
        }
    };
    let body = response.text().await.unwrap();
    let lines: Vec<_> = body.lines().map(|s| Ok(s.to_owned())).collect();

    Scrape::parse(lines.into_iter()).unwrap()
}

pub fn get_metrics_with_name(metrics: Scrape, name: &str) -> Vec<Sample> {
    metrics
        .samples
        .iter()
        .filter(|m| m.metric == name)
        .cloned()
        .collect::<Vec<Sample>>()
}

pub fn find_value(samples: &[Sample], labels: &[(&str, &str)]) -> Option<Value> {
    samples
        .iter()
        .find(|s| {
            for l in labels {
                match s.labels.get(l.0) {
                    Some(v) => {
                        if v != l.1 {
                            return false;
                        }
                    },
                    None => return false,
                }
            }
            true
        })
        .map(|m| m.value.clone())
}

pub async fn get_metric_value(port: u16, name: &str, labels: &[(&str, &str)]) -> Option<Value> {
    find_value(&get_metrics_with_name(get_metrics(port).await, name), labels)
}
