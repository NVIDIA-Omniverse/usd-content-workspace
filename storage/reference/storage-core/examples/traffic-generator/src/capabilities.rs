// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use storage_core::traits::capabilities::v1alpha::{
    ListRoutesRequest, ListServicesRequest, ListTopLevelAddressesRequest,
};
use storage_core_test::clients::traits::capabilities::Capabilities;

use crate::report::TrafficReport;
use crate::runner::ApiVersionArg;

const PHASE: &str = "Capabilities";

pub async fn run(client: &mut Box<dyn Capabilities>, api_version: ApiVersionArg, report: &TrafficReport) {
    // list_services
    report
        .track(PHASE, "list_services", async {
            client.list_services(ListServicesRequest {}).await
        })
        .await;

    // list_top_level_addresses
    report
        .track(PHASE, "list_top_level_addresses", async {
            client.list_top_level_addresses(ListTopLevelAddressesRequest {}).await
        })
        .await;

    // list_routes (v1alpha only)
    if api_version == ApiVersionArg::V1Alpha {
        report
            .track(PHASE, "list_routes", async {
                client.list_routes(ListRoutesRequest {}).await
            })
            .await;
    }
}
