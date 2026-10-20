// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use storage_core::traits::{generate_storage_trait, Request, Result};

pub use super::proto::helloworld::*;

// The Greeter trait is the main trait that defines the methods that the Greeter Service will implement.
// It is generated using the `generate_storage_trait` macro from the storage-core library.
// The macro generates the trait and the associated types for the Service.
generate_storage_trait! {
    Greeter {
        async fn say_hello(&self, request: Request<HelloRequest>) -> Result<HelloReply>;
    }
}
