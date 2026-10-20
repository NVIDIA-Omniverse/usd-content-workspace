// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

pub mod v1beta {
    use crate::traits::{Request, Result};
    pub use storage_core_protos::nvidia::omniverse::storage::capabilities::v1beta::*;

    crate::generate_storage_trait! {
        Capabilities {
            async fn list_top_level_addresses(
                &self,
                request: Request<ListTopLevelAddressesRequest>,
            ) -> Result<ListTopLevelAddressesResponse>;
        }
    }
}

pub mod v1alpha {
    use crate::traits::{Request, Result};
    pub use storage_core_protos::nvidia::omniverse::storage::capabilities::v1alpha::*;

    crate::generate_storage_trait! {
        Capabilities {
            async fn list_top_level_addresses(
                &self,
                request: Request<ListTopLevelAddressesRequest>,
            ) -> Result<ListTopLevelAddressesResponse>;

            async fn list_routes(
                &self,
                request: Request<ListRoutesRequest>,
            ) -> Result<ListRoutesResponse>;
        }
    }
}
