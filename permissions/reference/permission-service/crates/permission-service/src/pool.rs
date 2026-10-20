// SPDX-FileCopyrightText: Copyright 2026 NVIDIA CORPORATION & AFFILIATES
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

use crate::metrics::Metrics;
use arc_swap::ArcSwap;
use async_trait::async_trait;
use deadpool_postgres::{Object, Pool, PoolError};
use opentelemetry::metrics::{Counter, Histogram, Meter, ObservableGauge, ObservableUpDownCounter};
use opentelemetry::KeyValue;
use permission_core::{Action, Group, PolicyId, Principal, Resource};
use permission_db::{
    ConnectionPool, Meta, MetadataConfig, MetadataStorage, PageResult, Policy, PolicyDbBatch,
    PolicyQueryBatch, PolicyStorage, PolicyUpsertResult, ResourceTypeMeta, ServiceMeta, StorageErr,
    StorageErrKind,
};
use std::sync::Arc;
use std::time::Instant;

/// `ConnectionPool` implementation that wraps a deadpool `Pool` and emits OTEL
/// metrics for connection usage, acquire wait time, wait-queue depth, and
/// acquisition timeouts. The storage layer sees only the `ConnectionPool`
/// trait and remains unaware of the instrumentation.
///
/// The pool is held behind an `ArcSwap` so a background task can atomically
/// replace it (for example after a rotated database token) without disrupting
/// in-flight callers.
pub struct InstrumentedPool {
    inner: Arc<ArcSwap<Pool>>,
    wait_time: Histogram<u64>,
    timeouts: Counter<u64>,
    _connection_count: ObservableUpDownCounter<i64>,
    _pending_requests: ObservableUpDownCounter<i64>,
    _max_connections: ObservableGauge<u64>,
}

impl InstrumentedPool {
    pub fn new(pool: Arc<ArcSwap<Pool>>, meter: &Meter) -> Self {
        let wait_time = meter
            .u64_histogram("db.client.connection.wait_time")
            .with_description("Time spent waiting to acquire a connection from the pool")
            .with_unit("ms")
            .build();
        let timeouts = meter
            .u64_counter("db.client.connection.timeouts")
            .with_description("Number of connection-pool acquisition timeouts")
            .with_unit("timeouts")
            .build();

        let count_pool = pool.clone();
        let connection_count = meter
            .i64_observable_up_down_counter("db.client.connection.count")
            .with_description("Number of connections broken down by state (used or idle)")
            .with_unit("connections")
            .with_callback(move |observer| {
                let status = count_pool.load().status();
                let used = status.size.saturating_sub(status.available) as i64;
                observer.observe(used, &[KeyValue::new("state", "used")]);
                observer.observe(status.available as i64, &[KeyValue::new("state", "idle")]);
            })
            .build();

        let waiting_pool = pool.clone();
        let pending_requests = meter
            .i64_observable_up_down_counter("db.client.connection.pending_requests")
            .with_description("Number of pending requests waiting for a connection")
            .with_unit("requests")
            .with_callback(move |observer| {
                observer.observe(waiting_pool.load().status().waiting as i64, &[]);
            })
            .build();

        let max_pool = pool.clone();
        let max_connections = meter
            .u64_observable_gauge("db.client.connection.max")
            .with_description("Maximum number of connections allowed in the pool")
            .with_unit("connections")
            .with_callback(move |observer| {
                observer.observe(max_pool.load().status().max_size as u64, &[]);
            })
            .build();

        Self {
            inner: pool,
            wait_time,
            timeouts,
            _connection_count: connection_count,
            _pending_requests: pending_requests,
            _max_connections: max_connections,
        }
    }
}

#[async_trait]
impl ConnectionPool for InstrumentedPool {
    async fn get(&self) -> Result<Object, PoolError> {
        let start = Instant::now();
        let pool = self.inner.load_full();
        let result = pool.get().await;
        self.wait_time
            .record(start.elapsed().as_millis() as u64, &[]);
        if let Err(PoolError::Timeout(_)) = &result {
            self.timeouts.add(1, &[]);
        }
        result
    }
}

/// `PolicyStorage` decorator that records the duration and internal-error rate
/// of every operation through the shared `Metrics` instruments. The storage
/// layer (`permission-db`) stays unaware of OTEL.
pub struct InstrumentedPolicyStorage {
    inner: Arc<dyn PolicyStorage>,
    duration: Histogram<u64>,
    errors: Counter<u64>,
}

impl InstrumentedPolicyStorage {
    pub fn new(inner: Arc<dyn PolicyStorage>, metrics: &Metrics) -> Self {
        Self {
            inner,
            duration: metrics.db_operation_duration.clone(),
            errors: metrics.db_operation_errors.clone(),
        }
    }

    fn record<T>(&self, operation: &'static str, start: Instant, result: &Result<T, StorageErr>) {
        record_db_op(
            &self.duration,
            &self.errors,
            "policy",
            operation,
            start,
            result,
        );
    }
}

#[async_trait]
impl PolicyStorage for InstrumentedPolicyStorage {
    async fn initialize(&self) -> Result<(), StorageErr> {
        let start = Instant::now();
        let result = self.inner.initialize().await;
        self.record("initialize", start, &result);
        result
    }

    async fn destroy(&self) -> Result<(), StorageErr> {
        let start = Instant::now();
        let result = self.inner.destroy().await;
        self.record("destroy", start, &result);
        result
    }

    async fn set_system_policies(
        &self,
        policies: Vec<PolicyDbBatch>,
    ) -> Result<Vec<Policy>, StorageErr> {
        let start = Instant::now();
        let result = self.inner.set_system_policies(policies).await;
        self.record("set_system_policies", start, &result);
        result
    }

    async fn add_policy(
        &self,
        policy: &str,
        action: Option<&Action>,
        principal: Option<&Principal>,
        group: Option<&Group>,
        resource: Option<&Resource>,
        resource_prefix: Option<&Resource>,
        order: i32,
        created_by: &str,
        comment: Option<&str>,
    ) -> Result<PolicyUpsertResult, StorageErr> {
        let start = Instant::now();
        let result = self
            .inner
            .add_policy(
                policy,
                action,
                principal,
                group,
                resource,
                resource_prefix,
                order,
                created_by,
                comment,
            )
            .await;
        self.record("add_policy", start, &result);
        result
    }

    async fn add_policy_batch(
        &self,
        batches: Vec<PolicyDbBatch>,
    ) -> Result<Vec<PolicyUpsertResult>, StorageErr> {
        let start = Instant::now();
        let result = self.inner.add_policy_batch(batches).await;
        self.record("add_policy_batch", start, &result);
        result
    }

    async fn update_policy(
        &self,
        policy_id: PolicyId,
        policy: &str,
        action: Option<&Action>,
        principal: Option<&Principal>,
        group: Option<&Group>,
        resource: Option<&Resource>,
        resource_prefix: Option<&Resource>,
        order: i32,
        created_by: &str,
        comment: Option<&str>,
    ) -> Result<Policy, StorageErr> {
        let start = Instant::now();
        let result = self
            .inner
            .update_policy(
                policy_id,
                policy,
                action,
                principal,
                group,
                resource,
                resource_prefix,
                order,
                created_by,
                comment,
            )
            .await;
        self.record("update_policy", start, &result);
        result
    }

    async fn get_policies(
        &self,
        action: Option<&Action>,
        principal: Option<&Principal>,
        groups: &[String],
        resource: Option<&Resource>,
        resource_prefixes: &[Resource],
    ) -> Result<Vec<Policy>, StorageErr> {
        let start = Instant::now();
        let result = self
            .inner
            .get_policies(action, principal, groups, resource, resource_prefixes)
            .await;
        self.record("get_policies", start, &result);
        result
    }

    async fn get_policies_batched(
        &self,
        principal: Option<&Principal>,
        groups: &[String],
        batches: &[PolicyQueryBatch],
        resource_prefixes: &[Resource],
    ) -> Result<Vec<Policy>, StorageErr> {
        let start = Instant::now();
        let result = self
            .inner
            .get_policies_batched(principal, groups, batches, resource_prefixes)
            .await;
        self.record("get_policies_batched", start, &result);
        result
    }

    async fn get_policies_paginated(
        &self,
        action: Option<&Action>,
        principal: Option<&Principal>,
        group: Option<&Group>,
        resource: Option<&Resource>,
        resource_prefixes: &[Resource],
        order: Option<i32>,
        page: i64,
        limit: i64,
    ) -> Result<PageResult<Policy>, StorageErr> {
        let start = Instant::now();
        let result = self
            .inner
            .get_policies_paginated(
                action,
                principal,
                group,
                resource,
                resource_prefixes,
                order,
                page,
                limit,
            )
            .await;
        self.record("get_policies_paginated", start, &result);
        result
    }

    async fn get_policy_by_id(&self, policy_id: &PolicyId) -> Result<Policy, StorageErr> {
        let start = Instant::now();
        let result = self.inner.get_policy_by_id(policy_id).await;
        self.record("get_policy_by_id", start, &result);
        result
    }

    async fn remove_policy(&self, policy_id: &PolicyId) -> Result<(), StorageErr> {
        let start = Instant::now();
        let result = self.inner.remove_policy(policy_id).await;
        self.record("remove_policy", start, &result);
        result
    }
}

/// `MetadataStorage` decorator that records the duration and internal-error rate
/// of every operation through the shared `Metrics` instruments.
pub struct InstrumentedMetadataStorage {
    inner: Arc<dyn MetadataStorage>,
    duration: Histogram<u64>,
    errors: Counter<u64>,
}

impl InstrumentedMetadataStorage {
    pub fn new(inner: Arc<dyn MetadataStorage>, metrics: &Metrics) -> Self {
        Self {
            inner,
            duration: metrics.db_operation_duration.clone(),
            errors: metrics.db_operation_errors.clone(),
        }
    }

    fn record<T>(&self, operation: &'static str, start: Instant, result: &Result<T, StorageErr>) {
        record_db_op(
            &self.duration,
            &self.errors,
            "metadata",
            operation,
            start,
            result,
        );
    }
}

#[async_trait]
impl MetadataStorage for InstrumentedMetadataStorage {
    async fn initialize(&self) -> Result<(), StorageErr> {
        let start = Instant::now();
        let result = self.inner.initialize().await;
        self.record("initialize", start, &result);
        result
    }

    async fn destroy(&self) -> Result<(), StorageErr> {
        let start = Instant::now();
        let result = self.inner.destroy().await;
        self.record("destroy", start, &result);
        result
    }

    async fn set_system_metadata(&self, metadata: MetadataConfig) -> Result<(), StorageErr> {
        let start = Instant::now();
        let result = self.inner.set_system_metadata(metadata).await;
        self.record("set_system_metadata", start, &result);
        result
    }

    async fn get_meta(
        &self,
        action: &Action,
        resource: Option<&Resource>,
    ) -> Result<Meta, StorageErr> {
        let start = Instant::now();
        let result = self.inner.get_meta(action, resource).await;
        self.record("get_meta", start, &result);
        result
    }

    async fn get_services(&self) -> Result<Vec<ServiceMeta>, StorageErr> {
        let start = Instant::now();
        let result = self.inner.get_services().await;
        self.record("get_services", start, &result);
        result
    }

    async fn get_service_meta(&self, service: &str) -> Result<ServiceMeta, StorageErr> {
        let start = Instant::now();
        let result = self.inner.get_service_meta(service).await;
        self.record("get_service_meta", start, &result);
        result
    }

    async fn set_service_meta(&self, principal: &ServiceMeta) -> Result<(), StorageErr> {
        let start = Instant::now();
        let result = self.inner.set_service_meta(principal).await;
        self.record("set_service_meta", start, &result);
        result
    }

    async fn delete_service_meta(&self, service: &str) -> Result<(), StorageErr> {
        let start = Instant::now();
        let result = self.inner.delete_service_meta(service).await;
        self.record("delete_service_meta", start, &result);
        result
    }

    async fn get_service_actions(&self, service: &str) -> Result<Vec<Action>, StorageErr> {
        let start = Instant::now();
        let result = self.inner.get_service_actions(service).await;
        self.record("get_service_actions", start, &result);
        result
    }

    async fn replace_service_actions(
        &self,
        service: &str,
        actions: Vec<Action>,
    ) -> Result<(), StorageErr> {
        let start = Instant::now();
        let result = self.inner.replace_service_actions(service, actions).await;
        self.record("replace_service_actions", start, &result);
        result
    }

    async fn set_service_action(&self, service: &str, action: &str) -> Result<Action, StorageErr> {
        let start = Instant::now();
        let result = self.inner.set_service_action(service, action).await;
        self.record("set_service_action", start, &result);
        result
    }

    async fn delete_service_action(&self, service: &str, action: &str) -> Result<(), StorageErr> {
        let start = Instant::now();
        let result = self.inner.delete_service_action(service, action).await;
        self.record("delete_service_action", start, &result);
        result
    }

    async fn get_service_resource_types(
        &self,
        service: &str,
    ) -> Result<Vec<ResourceTypeMeta>, StorageErr> {
        let start = Instant::now();
        let result = self.inner.get_service_resource_types(service).await;
        self.record("get_service_resource_types", start, &result);
        result
    }

    async fn get_resource_type(
        &self,
        service: &str,
        kind: &str,
    ) -> Result<ResourceTypeMeta, StorageErr> {
        let start = Instant::now();
        let result = self.inner.get_resource_type(service, kind).await;
        self.record("get_resource_type", start, &result);
        result
    }

    async fn get_resource_type_by_kind(
        &self,
        kind: &str,
    ) -> Result<Option<ResourceTypeMeta>, StorageErr> {
        let start = Instant::now();
        let result = self.inner.get_resource_type_by_kind(kind).await;
        self.record("get_resource_type_by_kind", start, &result);
        result
    }

    async fn replace_resource_types(
        &self,
        service: &str,
        resource_types: Vec<ResourceTypeMeta>,
    ) -> Result<(), StorageErr> {
        let start = Instant::now();
        let result = self
            .inner
            .replace_resource_types(service, resource_types)
            .await;
        self.record("replace_resource_types", start, &result);
        result
    }

    async fn set_resource_type_meta(&self, resource: &ResourceTypeMeta) -> Result<(), StorageErr> {
        let start = Instant::now();
        let result = self.inner.set_resource_type_meta(resource).await;
        self.record("set_resource_type_meta", start, &result);
        result
    }

    async fn delete_resource_type_meta(&self, service: &str, kind: &str) -> Result<(), StorageErr> {
        let start = Instant::now();
        let result = self.inner.delete_resource_type_meta(service, kind).await;
        self.record("delete_resource_type_meta", start, &result);
        result
    }
}

fn record_db_op<T>(
    duration: &Histogram<u64>,
    errors: &Counter<u64>,
    storage: &'static str,
    operation: &'static str,
    start: Instant,
    result: &Result<T, StorageErr>,
) {
    let attrs = [
        KeyValue::new("db.storage", storage),
        KeyValue::new("db.operation.name", operation),
    ];
    duration.record(start.elapsed().as_millis() as u64, &attrs);

    if let Err(err) = result {
        if err.kind == StorageErrKind::Internal {
            errors.add(
                1,
                &[
                    KeyValue::new("db.storage", storage),
                    KeyValue::new("db.operation.name", operation),
                    KeyValue::new("error.kind", kind_str(&err.kind)),
                ],
            );
        }
    }
}

fn kind_str(kind: &StorageErrKind) -> &'static str {
    match kind {
        StorageErrKind::Internal => "internal",
        StorageErrKind::Constraint => "constraint",
        StorageErrKind::DoesNotExist => "does_not_exist",
        StorageErrKind::InvalidPagination => "invalid_pagination",
        StorageErrKind::NotSupported => "not_supported",
        StorageErrKind::Forbidden => "forbidden",
        StorageErrKind::Conflict => "conflict",
    }
}
