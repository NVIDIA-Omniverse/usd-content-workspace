// SPDX-FileCopyrightText: Copyright (c) 2024-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
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

fn main() {
    let out_dir = std::env::var("OUT_DIR").expect("`OUT_DIR` not set");
    let out_dir = std::path::PathBuf::from(out_dir);

    // a bit hacky: run these builds first to generate the FDS. it still emits
    // `example.rs`, without a client, server, or transport (only types). the
    // last invocation then overwrites that file.
    tonic_prost_build::configure()
        .build_client(false)
        .build_server(false)
        .build_transport(false)
        .out_dir(&out_dir)
        .file_descriptor_set_path(out_dir.join("example_descriptor.bin"))
        .compile_protos(&["./proto/service.proto"], &["./proto/"])
        .unwrap();

    tonic_prost_build::configure()
        .build_client(false)
        .build_server(false)
        .build_transport(false)
        .out_dir(&out_dir)
        .file_descriptor_set_path(out_dir.join("foo_descriptor.bin"))
        .compile_protos(&["./proto/foo.proto"], &["./proto/"])
        .unwrap();

    tonic_prost_build::configure()
        .build_client(true)
        .build_server(true)
        .build_transport(true)
        .out_dir(&out_dir)
        .file_descriptor_set_path(out_dir.join("service_descriptor.bin"))
        .compile_protos(
            &["./proto/service.proto", "./proto/foo.proto"],
            &["./proto/"],
        )
        .unwrap();
}
