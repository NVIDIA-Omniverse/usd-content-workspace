// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
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

use std::env;

fn main() {
    let out_dir = std::path::PathBuf::from(env::var("OUT_DIR").unwrap());
    tonic_prost_build::configure()
        .build_client(false)
        .build_server(true)
        .build_transport(true)
        .include_file("include.rs")
        .emit_rerun_if_changed(false)
        .file_descriptor_set_path(out_dir.join("envoy_auth_descriptor.bin"))
        .compile_protos(
            &[
                "google/rpc/status.proto",
                "envoy/service/auth/v3/external_auth.proto",
                "envoy/config/core/v3/base.proto",
            ],
            &["../../proto/vendor"],
        )
        .unwrap_or_else(|e| panic!("Failed to compile protos {:?}", e));
}
