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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
    let repo_root = manifest_dir
        .parent()
        .and_then(|crates| crates.parent())
        .ok_or("expected crate at crates/microsoft-graph-userinfo-smoke-test under repo root")?;
    let protos_root = repo_root.join("protos");
    let proto_file = protos_root.join("nvidia/omniverse/userinfo/v1alpha/user_info_service.proto");

    tonic_prost_build::configure()
        .build_server(false)
        .compile_protos(&[proto_file], &[protos_root])?;

    Ok(())
}
