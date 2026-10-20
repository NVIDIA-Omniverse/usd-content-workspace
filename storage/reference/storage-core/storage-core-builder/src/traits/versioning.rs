// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

pub mod v1beta {
    use crate::traits::{BoxStream, Request, Result, StreamingOptions};
    pub use storage_core_protos::nvidia::omniverse::storage::versioning::v1beta::*;
    crate::generate_storage_trait! {
        Versioning {
            async fn enumerate_versions(
                &self,
                request: Request<EnumerateVersionsRequest>,
                streaming: StreamingOptions,
                version_id_token: Option<String>,
            ) -> Result<BoxStream<EnumerateVersionsResponseWithContinueToken>>;
        }
    }

    pub struct EnumerateVersionsResponseWithContinueToken {
        pub response: EnumerateVersionsResponse,
        pub continue_token: Option<String>,
        pub version_id_token: Option<String>,
    }
}

pub mod v1alpha {
    use crate::traits::{BoxStream, Request, Result, StreamingOptions};
    pub use storage_core_protos::nvidia::omniverse::storage::versioning::v1alpha::*;
    crate::generate_storage_trait! {
        Versioning {
            async fn enumerate_versions(
                &self,
                request: Request<EnumerateVersionsRequest>,
                streaming: StreamingOptions,
                version_id_token: Option<String>,
            ) -> Result<BoxStream<EnumerateVersionsResponseWithContinueToken>>;

            async fn is_versioned(&self, request: Request<IsVersionedRequest>) -> Result<IsVersionedResponse>;
        }
    }

    pub struct EnumerateVersionsResponseWithContinueToken {
        pub response: EnumerateVersionsResponse,
        pub continue_token: Option<String>,
        pub version_id_token: Option<String>,
    }
}
