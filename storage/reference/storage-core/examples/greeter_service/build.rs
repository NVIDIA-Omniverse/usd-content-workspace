// SPDX-FileCopyrightText: Copyright (c) 2024 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::env;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = PathBuf::from(env::var("OUT_DIR")?);

    // set the protoc env variable to the path of the vendored protoc installed
    std::env::set_var(
        "PROTOC",
        protoc_bin_vendored::protoc_bin_path().expect("protoc not found"),
    );
    std::env::set_var(
        "PROTOC_INCLUDE",
        protoc_bin_vendored::include_path().expect("protoc not found"),
    );

    // Compile the proto file and create a file descriptor set
    tonic_prost_build::configure()
        .build_client(true)
        .build_server(true)
        .build_transport(true)
        .compile_well_known_types(true)
        .bytes(".")
        .file_descriptor_set_path(out_dir.join("helloworld.bin"))
        .compile_protos(&["proto/helloworld.proto"], &["proto"])?;

    Ok(())
}
