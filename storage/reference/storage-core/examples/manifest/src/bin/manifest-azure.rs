// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use storage_core::azure::Storage;

use manifest_service::run;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(not(feature = "azure"))]
    compile_error!("This binary must be built with the 'azure' feature");

    run(env!("CARGO_BIN_NAME"), "azure", |figment, secrets| {
        Ok(Arc::new(Storage::from_figment(figment, secrets)?))
    })
    .await
}
