// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

#[macro_use]
pub mod utils;

pub mod capabilities;
pub mod filefolder;
pub mod fileobject;
pub mod metadata;
pub mod versioning;

use std::{
    ops::Deref,
    sync::{Arc, OnceLock},
};

use storage_core_util::observability::metrics::{BoundUIntCounter, BoundUIntHistogram, MetricBinder};
use utils::{
    chunk_size_hist_base, data_size_hist_base, item_count_base, latency_base, redirects_base, results_counter_base,
    sdk_latency_base, sdk_results_counter_base, upload_method_base, MethodMetrics,
};

use crate::layer::Layer;

#[doc(inline)]
pub use crate::record_operation;

/// Layer that adds operation metrics to the service.
///
/// These metrics add Rate, Error, Duration (RED) metrics for each function, as well as per-operation metrics,
/// such as read/write size, etc.
///
/// This layer should be the top layer in the service to ensure that all metrics are recorded.
///
/// Supports the following traits:
/// - `v1alpha::FileObject`
/// - `v1alpha::FileFolder`
/// - `v1alpha::Capabilities`
/// - `v1alpha::Versioning`
/// - `v1beta::FileObject`
/// - `v1beta::FileFolder`
/// - `v1beta::Capabilities`
/// - `v1beta::Versioning`
/// - `v1alpha::Metadata`
pub struct OperationMetricsLayer {
    pub storage_backend: String,
    state: OnceLock<Arc<OperationMetricsState>>,
}

impl OperationMetricsLayer {
    pub fn new(storage_backend: impl Into<String>) -> Self {
        Self {
            storage_backend: storage_backend.into(),
            state: OnceLock::new(),
        }
    }
}

impl<S> Layer<S> for OperationMetricsLayer {
    type Service = OperationMetrics<S>;

    fn layer(&self, service: S) -> Self::Service {
        let state = self
            .state
            .get_or_init(|| Arc::new(OperationMetricsState::new(self.storage_backend.clone())))
            .clone();
        OperationMetrics::from_state(service, state)
    }
}

pub struct OperationMetrics<S> {
    pub inner: S,
    state: Arc<OperationMetricsState>,
}

impl<S> OperationMetrics<S> {
    pub fn new(inner: S, storage_backend: String) -> Self {
        Self {
            inner,
            state: Arc::new(OperationMetricsState::new(storage_backend)),
        }
    }

    pub fn from_state(inner: S, state: Arc<OperationMetricsState>) -> Self {
        Self { inner, state }
    }

    /// Returns the shared metrics state used by this wrapper.
    pub fn state(&self) -> &Arc<OperationMetricsState> {
        &self.state
    }
}

impl<S> Deref for OperationMetrics<S> {
    type Target = OperationMetricsState;

    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

/// Shared operation metric handles used by [`OperationMetrics`].
///
/// Keeping these handles behind a shared `Arc` keeps each generic
/// `OperationMetrics<S>` wrapper small while preserving read-style field access
/// through `Deref`.
pub struct OperationMetricsState {
    pub storage_backend: String,
    // fileobject metrics
    pub stat_metrics_v1alpha: Arc<MethodMetrics>,
    pub stat_metrics_v1beta: Arc<MethodMetrics>,
    pub read_metrics_v1alpha: Arc<MethodMetrics>,
    pub read_metrics_v1beta: Arc<MethodMetrics>,
    pub read_from_address_metrics_v1alpha: Arc<MethodMetrics>,
    pub read_from_address_metrics_v1beta: Arc<MethodMetrics>,
    pub write_metrics_v1alpha: Arc<MethodMetrics>,
    pub write_metrics_v1beta: Arc<MethodMetrics>,
    pub delete_metrics_v1alpha: Arc<MethodMetrics>,
    pub delete_metrics_v1beta: Arc<MethodMetrics>,
    pub abort_multipart_upload_metrics_v1alpha: Arc<MethodMetrics>,
    pub abort_multipart_upload_metrics_v1beta: Arc<MethodMetrics>,
    pub complete_multipart_upload_metrics_v1alpha: Arc<MethodMetrics>,
    pub complete_multipart_upload_metrics_v1beta: Arc<MethodMetrics>,
    pub upload_part_metrics_v1alpha: Arc<MethodMetrics>,
    pub upload_part_metrics_v1beta: Arc<MethodMetrics>,
    pub complete_redirect_upload_metrics_v1alpha: Arc<MethodMetrics>,
    pub complete_redirect_upload_metrics_v1beta: Arc<MethodMetrics>,
    pub fetch_write_type_info_metrics_v1alpha: Arc<MethodMetrics>,
    pub fetch_write_type_info_metrics_v1beta: Arc<MethodMetrics>,
    pub enumerate_metrics_v1alpha: Arc<MethodMetrics>,
    pub enumerate_metrics_v1beta: Arc<MethodMetrics>,
    pub copy_metrics_v1alpha: Arc<MethodMetrics>,
    pub copy_metrics_v1beta: Arc<MethodMetrics>,
    pub move_object_metrics_v1alpha: Arc<MethodMetrics>,
    pub move_object_metrics_v1beta: Arc<MethodMetrics>,
    pub get_optimistic_locking_support_metrics_v1alpha: Arc<MethodMetrics>,
    pub is_read_only_metrics_v1alpha: Arc<MethodMetrics>,
    pub is_versioned_metrics_v1alpha: Arc<MethodMetrics>,
    // versioning metrics
    pub enumerate_versions_metrics_v1alpha: Arc<MethodMetrics>,
    pub enumerate_versions_metrics_v1beta: Arc<MethodMetrics>,
    // filefolder metrics
    pub list_metrics_v1alpha: Arc<MethodMetrics>,
    pub list_metrics_v1beta: Arc<MethodMetrics>,
    pub list_stat_metrics_v1alpha: Arc<MethodMetrics>,
    pub list_stat_metrics_v1beta: Arc<MethodMetrics>,
    pub delete_folder_metrics_v1alpha: Arc<MethodMetrics>,
    pub delete_folder_metrics_v1beta: Arc<MethodMetrics>,
    pub create_folder_metrics_v1alpha: Arc<MethodMetrics>,
    pub get_folder_mode_metrics_v1alpha: Arc<MethodMetrics>,
    // capabilities metrics
    pub list_routes_metrics_v1alpha: Arc<MethodMetrics>,
    pub list_top_level_addresses_metrics_v1alpha: Arc<MethodMetrics>,
    pub list_top_level_addresses_metrics_v1beta: Arc<MethodMetrics>,
    // metadata metrics
    pub get_metadata_metrics_v1alpha: Arc<MethodMetrics>,
    pub update_metadata_metrics_v1alpha: Arc<MethodMetrics>,
    pub update_metadata_batch_metrics_v1alpha: Arc<MethodMetrics>,
    pub delete_metadata_metrics_v1alpha: Arc<MethodMetrics>,
    // other metrics
    pub list_item_count_v1alpha: Arc<BoundUIntHistogram>,
    pub list_item_count_v1beta: Arc<BoundUIntHistogram>,
    pub list_stat_item_count_v1alpha: Arc<BoundUIntHistogram>,
    pub list_stat_item_count_v1beta: Arc<BoundUIntHistogram>,
    pub enumerate_item_count_v1alpha: Arc<BoundUIntHistogram>,
    pub enumerate_item_count_v1beta: Arc<BoundUIntHistogram>,
    pub enumerate_versions_item_count_v1alpha: Arc<BoundUIntHistogram>,
    pub enumerate_versions_item_count_v1beta: Arc<BoundUIntHistogram>,
    pub read_redirects_v1alpha: Arc<BoundUIntCounter>,
    pub read_redirects_v1beta: Arc<BoundUIntCounter>,
    pub read_chunk_size_hist_v1alpha: Arc<BoundUIntHistogram>,
    pub read_chunk_size_hist_v1beta: Arc<BoundUIntHistogram>,
    pub read_data_size_hist_v1alpha: Arc<BoundUIntHistogram>,
    pub read_data_size_hist_v1beta: Arc<BoundUIntHistogram>,
    pub read_from_address_redirects_v1alpha: Arc<BoundUIntCounter>,
    pub read_from_address_redirects_v1beta: Arc<BoundUIntCounter>,
    pub read_from_address_chunk_size_hist_v1alpha: Arc<BoundUIntHistogram>,
    pub read_from_address_chunk_size_hist_v1beta: Arc<BoundUIntHistogram>,
    pub read_from_address_data_size_hist_v1alpha: Arc<BoundUIntHistogram>,
    pub read_from_address_data_size_hist_v1beta: Arc<BoundUIntHistogram>,
    pub write_data_size_hist_v1alpha: Arc<BoundUIntHistogram>,
    pub write_data_size_hist_v1beta: Arc<BoundUIntHistogram>,
    pub write_upload_method_v1alpha_body: Arc<BoundUIntCounter>,
    pub write_upload_method_v1beta_body: Arc<BoundUIntCounter>,
    pub write_upload_method_v1alpha_redirect: Arc<BoundUIntCounter>,
    pub write_upload_method_v1beta_redirect: Arc<BoundUIntCounter>,
    pub write_upload_method_v1alpha_multipart: Arc<BoundUIntCounter>,
    pub write_upload_method_v1beta_multipart: Arc<BoundUIntCounter>,
}

impl OperationMetricsState {
    pub fn new(storage_backend: impl Into<String>) -> Self {
        let storage_backend = storage_backend.into();
        let pod_name = std::env::var("STORAGE_POD_NAME").unwrap_or("pod".to_string());
        let pkg_version = env!("CARGO_PKG_VERSION").to_string();
        Self {
            storage_backend: storage_backend.clone(),
            stat_metrics_v1alpha: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "stat",
                "v1alpha",
                storage_backend.as_str(),
            )),
            stat_metrics_v1beta: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "stat",
                "v1beta",
                storage_backend.as_str(),
            )),
            read_metrics_v1alpha: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "read",
                "v1alpha",
                storage_backend.as_str(),
            )),
            read_metrics_v1beta: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "read",
                "v1beta",
                storage_backend.as_str(),
            )),
            read_from_address_metrics_v1alpha: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "read_from_address",
                "v1alpha",
                storage_backend.as_str(),
            )),
            read_from_address_metrics_v1beta: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "read_from_address",
                "v1beta",
                storage_backend.as_str(),
            )),
            write_metrics_v1alpha: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "write",
                "v1alpha",
                storage_backend.as_str(),
            )),
            write_metrics_v1beta: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "write",
                "v1beta",
                storage_backend.as_str(),
            )),
            delete_metrics_v1alpha: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "delete",
                "v1alpha",
                storage_backend.as_str(),
            )),
            delete_metrics_v1beta: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "delete",
                "v1beta",
                storage_backend.as_str(),
            )),
            abort_multipart_upload_metrics_v1alpha: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "abort_multipart_upload",
                "v1alpha",
                storage_backend.as_str(),
            )),
            abort_multipart_upload_metrics_v1beta: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "abort_multipart_upload",
                "v1beta",
                storage_backend.as_str(),
            )),
            complete_multipart_upload_metrics_v1alpha: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "complete_multipart_upload",
                "v1alpha",
                storage_backend.as_str(),
            )),
            complete_multipart_upload_metrics_v1beta: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "complete_multipart_upload",
                "v1beta",
                storage_backend.as_str(),
            )),
            upload_part_metrics_v1alpha: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "upload_part",
                "v1alpha",
                storage_backend.as_str(),
            )),
            upload_part_metrics_v1beta: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "upload_part",
                "v1beta",
                storage_backend.as_str(),
            )),
            complete_redirect_upload_metrics_v1alpha: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "complete_redirect_upload",
                "v1alpha",
                storage_backend.as_str(),
            )),
            complete_redirect_upload_metrics_v1beta: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "complete_redirect_upload",
                "v1beta",
                storage_backend.as_str(),
            )),
            fetch_write_type_info_metrics_v1alpha: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "fetch_write_type_info",
                "v1alpha",
                storage_backend.as_str(),
            )),
            fetch_write_type_info_metrics_v1beta: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "fetch_write_type_info",
                "v1beta",
                storage_backend.as_str(),
            )),
            enumerate_metrics_v1alpha: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "enumerate",
                "v1alpha",
                storage_backend.as_str(),
            )),
            enumerate_metrics_v1beta: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "enumerate",
                "v1beta",
                storage_backend.as_str(),
            )),
            copy_metrics_v1alpha: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "copy",
                "v1alpha",
                storage_backend.as_str(),
            )),
            copy_metrics_v1beta: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "copy",
                "v1beta",
                storage_backend.as_str(),
            )),
            move_object_metrics_v1alpha: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "move",
                "v1alpha",
                storage_backend.as_str(),
            )),
            move_object_metrics_v1beta: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "move",
                "v1beta",
                storage_backend.as_str(),
            )),
            get_optimistic_locking_support_metrics_v1alpha: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "get_optimistic_locking_support",
                "v1alpha",
                storage_backend.as_str(),
            )),
            is_read_only_metrics_v1alpha: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "is_read_only",
                "v1alpha",
                storage_backend.as_str(),
            )),
            is_versioned_metrics_v1alpha: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "is_versioned",
                "v1alpha",
                storage_backend.as_str(),
            )),
            enumerate_versions_metrics_v1alpha: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "enumerate_versions",
                "v1alpha",
                storage_backend.as_str(),
            )),
            enumerate_versions_metrics_v1beta: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "enumerate_versions",
                "v1beta",
                storage_backend.as_str(),
            )),
            list_metrics_v1alpha: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "list",
                "v1alpha",
                storage_backend.as_str(),
            )),
            list_metrics_v1beta: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "list",
                "v1beta",
                storage_backend.as_str(),
            )),
            list_stat_metrics_v1alpha: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "list_stat",
                "v1alpha",
                storage_backend.as_str(),
            )),
            list_stat_metrics_v1beta: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "list_stat",
                "v1beta",
                storage_backend.as_str(),
            )),
            delete_folder_metrics_v1alpha: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "delete_folder",
                "v1alpha",
                storage_backend.as_str(),
            )),
            delete_folder_metrics_v1beta: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "delete_folder",
                "v1beta",
                storage_backend.as_str(),
            )),
            create_folder_metrics_v1alpha: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "create_folder",
                "v1alpha",
                storage_backend.as_str(),
            )),
            get_folder_mode_metrics_v1alpha: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "get_folder_mode",
                "v1alpha",
                storage_backend.as_str(),
            )),
            list_routes_metrics_v1alpha: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "list_routes",
                "v1alpha",
                storage_backend.as_str(),
            )),
            list_top_level_addresses_metrics_v1alpha: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "list_top_level_addresses",
                "v1alpha",
                storage_backend.as_str(),
            )),
            list_top_level_addresses_metrics_v1beta: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "list_top_level_addresses",
                "v1beta",
                storage_backend.as_str(),
            )),
            get_metadata_metrics_v1alpha: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "get_metadata",
                "v1alpha",
                storage_backend.as_str(),
            )),
            update_metadata_metrics_v1alpha: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "update_metadata",
                "v1alpha",
                storage_backend.as_str(),
            )),
            update_metadata_batch_metrics_v1alpha: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "update_metadata_batch",
                "v1alpha",
                storage_backend.as_str(),
            )),
            delete_metadata_metrics_v1alpha: Arc::new(MethodMetrics::new(
                results_counter_base(),
                latency_base(),
                "delete_metadata",
                "v1alpha",
                storage_backend.as_str(),
            )),
            list_item_count_v1alpha: Arc::new(item_count_base().bind([
                ("method", "list".to_string()),
                ("api_version", "v1alpha".to_string()),
                ("storage_backend", storage_backend.clone()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ])),
            list_item_count_v1beta: Arc::new(item_count_base().bind([
                ("method", "list".to_string()),
                ("api_version", "v1beta".to_string()),
                ("storage_backend", storage_backend.clone()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ])),
            list_stat_item_count_v1alpha: Arc::new(item_count_base().bind([
                ("method", "list_stat".to_string()),
                ("api_version", "v1alpha".to_string()),
                ("storage_backend", storage_backend.clone()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ])),
            list_stat_item_count_v1beta: Arc::new(item_count_base().bind([
                ("method", "list_stat".to_string()),
                ("api_version", "v1beta".to_string()),
                ("storage_backend", storage_backend.clone()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ])),
            enumerate_item_count_v1alpha: Arc::new(item_count_base().bind([
                ("method", "enumerate".to_string()),
                ("api_version", "v1alpha".to_string()),
                ("storage_backend", storage_backend.clone()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ])),
            enumerate_item_count_v1beta: Arc::new(item_count_base().bind([
                ("method", "enumerate".to_string()),
                ("api_version", "v1beta".to_string()),
                ("storage_backend", storage_backend.clone()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ])),
            enumerate_versions_item_count_v1alpha: Arc::new(item_count_base().bind([
                ("method", "enumerate_versions".to_string()),
                ("api_version", "v1alpha".to_string()),
                ("storage_backend", storage_backend.clone()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ])),
            enumerate_versions_item_count_v1beta: Arc::new(item_count_base().bind([
                ("method", "enumerate_versions".to_string()),
                ("api_version", "v1beta".to_string()),
                ("storage_backend", storage_backend.clone()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ])),
            read_redirects_v1alpha: Arc::new(redirects_base().bind([
                ("method", "read".to_string()),
                ("api_version", "v1alpha".to_string()),
                ("storage_backend", storage_backend.clone()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ])),
            read_redirects_v1beta: Arc::new(redirects_base().bind([
                ("method", "read".to_string()),
                ("api_version", "v1beta".to_string()),
                ("storage_backend", storage_backend.clone()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ])),
            read_chunk_size_hist_v1alpha: Arc::new(chunk_size_hist_base().bind([
                ("method", "read".to_string()),
                ("api_version", "v1alpha".to_string()),
                ("storage_backend", storage_backend.clone()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ])),
            read_chunk_size_hist_v1beta: Arc::new(chunk_size_hist_base().bind([
                ("method", "read".to_string()),
                ("api_version", "v1beta".to_string()),
                ("storage_backend", storage_backend.clone()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ])),
            read_data_size_hist_v1alpha: Arc::new(data_size_hist_base().bind([
                ("method", "read".to_string()),
                ("api_version", "v1alpha".to_string()),
                ("storage_backend", storage_backend.clone()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ])),
            read_data_size_hist_v1beta: Arc::new(data_size_hist_base().bind([
                ("method", "read".to_string()),
                ("api_version", "v1beta".to_string()),
                ("storage_backend", storage_backend.clone()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ])),
            read_from_address_redirects_v1alpha: Arc::new(redirects_base().bind([
                ("method", "read_from_address".to_string()),
                ("api_version", "v1alpha".to_string()),
                ("storage_backend", storage_backend.clone()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ])),
            read_from_address_redirects_v1beta: Arc::new(redirects_base().bind([
                ("method", "read_from_address".to_string()),
                ("api_version", "v1beta".to_string()),
                ("storage_backend", storage_backend.clone()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ])),
            read_from_address_chunk_size_hist_v1alpha: Arc::new(chunk_size_hist_base().bind([
                ("method", "read_from_address".to_string()),
                ("api_version", "v1alpha".to_string()),
                ("storage_backend", storage_backend.clone()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ])),
            read_from_address_chunk_size_hist_v1beta: Arc::new(chunk_size_hist_base().bind([
                ("method", "read_from_address".to_string()),
                ("api_version", "v1beta".to_string()),
                ("storage_backend", storage_backend.clone()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ])),
            read_from_address_data_size_hist_v1alpha: Arc::new(data_size_hist_base().bind([
                ("method", "read_from_address".to_string()),
                ("api_version", "v1alpha".to_string()),
                ("storage_backend", storage_backend.clone()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ])),
            read_from_address_data_size_hist_v1beta: Arc::new(data_size_hist_base().bind([
                ("method", "read_from_address".to_string()),
                ("api_version", "v1beta".to_string()),
                ("storage_backend", storage_backend.clone()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ])),
            write_data_size_hist_v1alpha: Arc::new(data_size_hist_base().bind([
                ("method", "write".to_string()),
                ("api_version", "v1alpha".to_string()),
                ("storage_backend", storage_backend.clone()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ])),
            write_data_size_hist_v1beta: Arc::new(data_size_hist_base().bind([
                ("method", "write".to_string()),
                ("api_version", "v1beta".to_string()),
                ("storage_backend", storage_backend.clone()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ])),
            write_upload_method_v1alpha_body: Arc::new(upload_method_base().bind([
                ("method", "write".to_string()),
                ("api_version", "v1alpha".to_string()),
                ("storage_backend", storage_backend.clone()),
                ("upload_preference", "body".to_string()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ])),
            write_upload_method_v1beta_body: Arc::new(upload_method_base().bind([
                ("method", "write".to_string()),
                ("api_version", "v1beta".to_string()),
                ("storage_backend", storage_backend.clone()),
                ("upload_preference", "body".to_string()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ])),
            write_upload_method_v1alpha_redirect: Arc::new(upload_method_base().bind([
                ("method", "write".to_string()),
                ("api_version", "v1alpha".to_string()),
                ("storage_backend", storage_backend.clone()),
                ("upload_preference", "redirect".to_string()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ])),
            write_upload_method_v1beta_redirect: Arc::new(upload_method_base().bind([
                ("method", "write".to_string()),
                ("api_version", "v1beta".to_string()),
                ("storage_backend", storage_backend.clone()),
                ("upload_preference", "redirect".to_string()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ])),
            write_upload_method_v1alpha_multipart: Arc::new(upload_method_base().bind([
                ("method", "write".to_string()),
                ("api_version", "v1alpha".to_string()),
                ("storage_backend", storage_backend.clone()),
                ("upload_preference", "multipart".to_string()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ])),
            write_upload_method_v1beta_multipart: Arc::new(upload_method_base().bind([
                ("method", "write".to_string()),
                ("api_version", "v1beta".to_string()),
                ("storage_backend", storage_backend.clone()),
                ("upload_preference", "multipart".to_string()),
                ("pod_name", pod_name.clone()),
                ("pkg_version", pkg_version.clone()),
            ])),
        }
    }
}

/// Layer that adds SDK metrics to the service.
///
/// These metrics add Rate, Error, Duration (RED) metrics for each function, which serve to track requests
/// to the backend.
///
/// This layer should be the last layer in the service, right before the backend, so that they only capture
/// operations that are actually sent to the backend.
///
/// Supports the following traits:
/// - `v1alpha::FileObject`
/// - `v1alpha::FileFolder`
/// - `v1alpha::Capabilities`
/// - `v1alpha::Versioning`
/// - `v1beta::FileObject`
/// - `v1beta::FileFolder`
/// - `v1beta::Capabilities`
/// - `v1beta::Versioning`
/// - `v1alpha::Metadata`
pub struct SdkMetricsLayer {
    pub storage_backend: String,
    state: OnceLock<Arc<SdkMetricsState>>,
}

impl SdkMetricsLayer {
    pub fn new(storage_backend: impl Into<String>) -> Self {
        Self {
            storage_backend: storage_backend.into(),
            state: OnceLock::new(),
        }
    }
}

impl<S> Layer<S> for SdkMetricsLayer {
    type Service = SdkMetrics<S>;

    fn layer(&self, service: S) -> Self::Service {
        let state = self
            .state
            .get_or_init(|| Arc::new(SdkMetricsState::new(self.storage_backend.clone())))
            .clone();
        SdkMetrics::from_state(service, state)
    }
}

pub struct SdkMetrics<S> {
    pub inner: S,
    state: Arc<SdkMetricsState>,
}

impl<S> SdkMetrics<S> {
    pub fn new(inner: S, storage_backend: String) -> Self {
        Self {
            inner,
            state: Arc::new(SdkMetricsState::new(storage_backend)),
        }
    }

    pub fn from_state(inner: S, state: Arc<SdkMetricsState>) -> Self {
        Self { inner, state }
    }

    /// Returns the shared metrics state used by this wrapper.
    pub fn state(&self) -> &Arc<SdkMetricsState> {
        &self.state
    }
}

impl<S> Deref for SdkMetrics<S> {
    type Target = SdkMetricsState;

    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

/// Shared SDK metric handles used by [`SdkMetrics`].
///
/// Keeping these handles behind a shared `Arc` keeps each generic
/// `SdkMetrics<S>` wrapper small while preserving read-style field access
/// through `Deref`.
pub struct SdkMetricsState {
    pub storage_backend: String,
    // fileobject metrics
    pub stat_metrics_v1alpha: Arc<MethodMetrics>,
    pub stat_metrics_v1beta: Arc<MethodMetrics>,
    pub read_metrics_v1alpha: Arc<MethodMetrics>,
    pub read_metrics_v1beta: Arc<MethodMetrics>,
    pub read_from_address_metrics_v1alpha: Arc<MethodMetrics>,
    pub read_from_address_metrics_v1beta: Arc<MethodMetrics>,
    pub write_metrics_v1alpha: Arc<MethodMetrics>,
    pub write_metrics_v1beta: Arc<MethodMetrics>,
    pub delete_metrics_v1alpha: Arc<MethodMetrics>,
    pub delete_metrics_v1beta: Arc<MethodMetrics>,
    pub abort_multipart_upload_metrics_v1alpha: Arc<MethodMetrics>,
    pub abort_multipart_upload_metrics_v1beta: Arc<MethodMetrics>,
    pub complete_multipart_upload_metrics_v1alpha: Arc<MethodMetrics>,
    pub complete_multipart_upload_metrics_v1beta: Arc<MethodMetrics>,
    pub upload_part_metrics_v1alpha: Arc<MethodMetrics>,
    pub upload_part_metrics_v1beta: Arc<MethodMetrics>,
    pub complete_redirect_upload_metrics_v1alpha: Arc<MethodMetrics>,
    pub complete_redirect_upload_metrics_v1beta: Arc<MethodMetrics>,
    pub fetch_write_type_info_metrics_v1alpha: Arc<MethodMetrics>,
    pub fetch_write_type_info_metrics_v1beta: Arc<MethodMetrics>,
    pub enumerate_metrics_v1alpha: Arc<MethodMetrics>,
    pub enumerate_metrics_v1beta: Arc<MethodMetrics>,
    pub copy_metrics_v1alpha: Arc<MethodMetrics>,
    pub copy_metrics_v1beta: Arc<MethodMetrics>,
    pub move_object_metrics_v1alpha: Arc<MethodMetrics>,
    pub move_object_metrics_v1beta: Arc<MethodMetrics>,
    pub get_optimistic_locking_support_metrics_v1alpha: Arc<MethodMetrics>,
    pub is_read_only_metrics_v1alpha: Arc<MethodMetrics>,
    pub is_versioned_metrics_v1alpha: Arc<MethodMetrics>,
    // versioning metrics
    pub enumerate_versions_metrics_v1alpha: Arc<MethodMetrics>,
    pub enumerate_versions_metrics_v1beta: Arc<MethodMetrics>,
    // filefolder metrics
    pub list_metrics_v1alpha: Arc<MethodMetrics>,
    pub list_metrics_v1beta: Arc<MethodMetrics>,
    pub list_stat_metrics_v1alpha: Arc<MethodMetrics>,
    pub list_stat_metrics_v1beta: Arc<MethodMetrics>,
    pub delete_folder_metrics_v1alpha: Arc<MethodMetrics>,
    pub delete_folder_metrics_v1beta: Arc<MethodMetrics>,
    pub create_folder_metrics_v1alpha: Arc<MethodMetrics>,
    pub get_folder_mode_metrics_v1alpha: Arc<MethodMetrics>,
    // capabilities metrics
    pub list_routes_metrics_v1alpha: Arc<MethodMetrics>,
    pub list_top_level_addresses_metrics_v1alpha: Arc<MethodMetrics>,
    pub list_top_level_addresses_metrics_v1beta: Arc<MethodMetrics>,
    // metadata metrics
    pub get_metadata_metrics_v1alpha: Arc<MethodMetrics>,
    pub update_metadata_metrics_v1alpha: Arc<MethodMetrics>,
    pub update_metadata_batch_metrics_v1alpha: Arc<MethodMetrics>,
    pub delete_metadata_metrics_v1alpha: Arc<MethodMetrics>,
}

impl SdkMetricsState {
    pub fn new(storage_backend: impl Into<String>) -> Self {
        let storage_backend = storage_backend.into();
        Self {
            storage_backend: storage_backend.clone(),
            stat_metrics_v1alpha: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "stat",
                "v1alpha",
                storage_backend.as_str(),
            )),
            stat_metrics_v1beta: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "stat",
                "v1beta",
                storage_backend.as_str(),
            )),
            read_metrics_v1alpha: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "read",
                "v1alpha",
                storage_backend.as_str(),
            )),
            read_metrics_v1beta: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "read",
                "v1beta",
                storage_backend.as_str(),
            )),
            read_from_address_metrics_v1alpha: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "read_from_address",
                "v1alpha",
                storage_backend.as_str(),
            )),
            read_from_address_metrics_v1beta: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "read_from_address",
                "v1beta",
                storage_backend.as_str(),
            )),
            write_metrics_v1alpha: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "write",
                "v1alpha",
                storage_backend.as_str(),
            )),
            write_metrics_v1beta: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "write",
                "v1beta",
                storage_backend.as_str(),
            )),
            delete_metrics_v1alpha: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "delete",
                "v1alpha",
                storage_backend.as_str(),
            )),
            delete_metrics_v1beta: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "delete",
                "v1beta",
                storage_backend.as_str(),
            )),
            abort_multipart_upload_metrics_v1alpha: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "abort_multipart_upload",
                "v1alpha",
                storage_backend.as_str(),
            )),
            abort_multipart_upload_metrics_v1beta: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "abort_multipart_upload",
                "v1beta",
                storage_backend.as_str(),
            )),
            complete_multipart_upload_metrics_v1alpha: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "complete_multipart_upload",
                "v1alpha",
                storage_backend.as_str(),
            )),
            complete_multipart_upload_metrics_v1beta: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "complete_multipart_upload",
                "v1beta",
                storage_backend.as_str(),
            )),
            upload_part_metrics_v1alpha: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "upload_part",
                "v1alpha",
                storage_backend.as_str(),
            )),
            upload_part_metrics_v1beta: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "upload_part",
                "v1beta",
                storage_backend.as_str(),
            )),
            complete_redirect_upload_metrics_v1alpha: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "complete_redirect_upload",
                "v1alpha",
                storage_backend.as_str(),
            )),
            complete_redirect_upload_metrics_v1beta: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "complete_redirect_upload",
                "v1beta",
                storage_backend.as_str(),
            )),
            fetch_write_type_info_metrics_v1alpha: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "fetch_write_type_info",
                "v1alpha",
                storage_backend.as_str(),
            )),
            fetch_write_type_info_metrics_v1beta: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "fetch_write_type_info",
                "v1beta",
                storage_backend.as_str(),
            )),
            enumerate_metrics_v1alpha: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "enumerate",
                "v1alpha",
                storage_backend.as_str(),
            )),
            enumerate_metrics_v1beta: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "enumerate",
                "v1beta",
                storage_backend.as_str(),
            )),
            copy_metrics_v1alpha: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "copy",
                "v1alpha",
                storage_backend.as_str(),
            )),
            copy_metrics_v1beta: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "copy",
                "v1beta",
                storage_backend.as_str(),
            )),
            move_object_metrics_v1alpha: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "move",
                "v1alpha",
                storage_backend.as_str(),
            )),
            move_object_metrics_v1beta: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "move",
                "v1beta",
                storage_backend.as_str(),
            )),
            get_optimistic_locking_support_metrics_v1alpha: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "get_optimistic_locking_support",
                "v1alpha",
                storage_backend.as_str(),
            )),
            is_read_only_metrics_v1alpha: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "is_read_only",
                "v1alpha",
                storage_backend.as_str(),
            )),
            is_versioned_metrics_v1alpha: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "is_versioned",
                "v1alpha",
                storage_backend.as_str(),
            )),
            enumerate_versions_metrics_v1alpha: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "enumerate_versions",
                "v1alpha",
                storage_backend.as_str(),
            )),
            enumerate_versions_metrics_v1beta: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "enumerate_versions",
                "v1beta",
                storage_backend.as_str(),
            )),
            list_metrics_v1alpha: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "list",
                "v1alpha",
                storage_backend.as_str(),
            )),
            list_metrics_v1beta: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "list",
                "v1beta",
                storage_backend.as_str(),
            )),
            list_stat_metrics_v1alpha: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "list_stat",
                "v1alpha",
                storage_backend.as_str(),
            )),
            list_stat_metrics_v1beta: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "list_stat",
                "v1beta",
                storage_backend.as_str(),
            )),
            delete_folder_metrics_v1alpha: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "delete_folder",
                "v1alpha",
                storage_backend.as_str(),
            )),
            delete_folder_metrics_v1beta: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "delete_folder",
                "v1beta",
                storage_backend.as_str(),
            )),
            create_folder_metrics_v1alpha: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "create_folder",
                "v1alpha",
                storage_backend.as_str(),
            )),
            get_folder_mode_metrics_v1alpha: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "get_folder_mode",
                "v1alpha",
                storage_backend.as_str(),
            )),
            list_routes_metrics_v1alpha: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "list_routes",
                "v1alpha",
                storage_backend.as_str(),
            )),
            list_top_level_addresses_metrics_v1alpha: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "list_top_level_addresses",
                "v1alpha",
                storage_backend.as_str(),
            )),
            list_top_level_addresses_metrics_v1beta: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "list_top_level_addresses",
                "v1beta",
                storage_backend.as_str(),
            )),
            get_metadata_metrics_v1alpha: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "get_metadata",
                "v1alpha",
                storage_backend.as_str(),
            )),
            update_metadata_metrics_v1alpha: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "update_metadata",
                "v1alpha",
                storage_backend.as_str(),
            )),
            update_metadata_batch_metrics_v1alpha: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "update_metadata_batch",
                "v1alpha",
                storage_backend.as_str(),
            )),
            delete_metadata_metrics_v1alpha: Arc::new(MethodMetrics::new(
                sdk_results_counter_base(),
                sdk_latency_base(),
                "delete_metadata",
                "v1alpha",
                storage_backend.as_str(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_metrics_layer_reuses_state() {
        let layer = OperationMetricsLayer::new("test-backend");

        assert!(layer.state.get().is_none());
        let first = layer.layer(());
        let second = layer.layer(());

        assert!(layer.state.get().is_some());
        assert!(Arc::ptr_eq(first.state(), second.state()));
        assert_eq!(first.storage_backend.as_str(), "test-backend");
        let _ = first.stat_metrics_v1alpha.clone();
    }

    #[test]
    fn sdk_metrics_layer_reuses_state() {
        let layer = SdkMetricsLayer::new("test-backend");

        assert!(layer.state.get().is_none());
        let first = layer.layer(());
        let second = layer.layer(());

        assert!(layer.state.get().is_some());
        assert!(Arc::ptr_eq(first.state(), second.state()));
        assert_eq!(first.storage_backend.as_str(), "test-backend");
        let _ = first.stat_metrics_v1alpha.clone();
    }
}
