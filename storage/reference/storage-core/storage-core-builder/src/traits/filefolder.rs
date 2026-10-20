// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

pub mod v1beta {
    use crate::traits::{BoxStream, Request, Result, StreamingOptions};
    pub use storage_core_protos::nvidia::omniverse::storage::filefolder::v1beta::*;

    crate::generate_storage_trait! {
        FileFolder {
            async fn list(
                &self,
                request: Request<ListRequest>,
                streaming: StreamingOptions,
            ) -> Result<BoxStream<ListResponseWithContinueToken>>;

            async fn list_stat(
                &self,
                request: Request<ListStatRequest>,
                streaming: StreamingOptions,
            ) -> Result<BoxStream<ListStatResponseWithContinueToken>>;

            async fn delete_folder(
                &self,
                request: Request<DeleteFolderRequest>,
            ) -> Result<DeleteFolderResponse>;
        }
    }

    /// A response with a continue token.
    pub struct ListResponseWithContinueToken {
        pub response: ListResponse,
        pub continue_token: Option<String>,
    }

    pub struct ListStatResponseWithContinueToken {
        pub response: ListStatResponse,
        pub continue_token: Option<String>,
    }
}

pub mod v1alpha {
    use crate::traits::{BoxStream, Request, Result, StreamingOptions};
    pub use storage_core_protos::nvidia::omniverse::storage::filefolder::v1alpha::*;

    crate::generate_storage_trait! {
        FileFolder {
            async fn list(
                &self,
                request: Request<ListRequest>,
                streaming: StreamingOptions,
            ) -> Result<BoxStream<ListResponseWithContinueToken>>;

            async fn list_stat(
                &self,
                request: Request<ListStatRequest>,
                streaming: StreamingOptions,
            ) -> Result<BoxStream<ListStatResponseWithContinueToken>>;

            async fn delete_folder(
                &self,
                request: Request<DeleteFolderRequest>,
            ) -> Result<DeleteFolderResponse>;

            async fn create_folder(
                &self,
                request: Request<CreateFolderRequest>,
            ) -> Result<CreateFolderResponse>;

            async fn get_folder_mode(
                &self,
                request: Request<GetFolderModeRequest>,
            ) -> Result<GetFolderModeResponse>;
        }
    }

    /// A response with a continue token.
    #[derive(Debug, Clone)]
    pub struct ListResponseWithContinueToken {
        pub response: ListResponse,
        pub continue_token: Option<String>,
    }

    #[derive(Debug, Clone)]
    pub struct ListStatResponseWithContinueToken {
        pub response: ListStatResponse,
        pub continue_token: Option<String>,
    }
}
