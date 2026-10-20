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

use storage_core::traits::{async_trait, capabilities::v1beta::*, Request, Result};

use crate::backend::trace_error::{log_storage_error, TraceErrorService};

#[async_trait]
impl<S: Capabilities> Capabilities for TraceErrorService<S> {
    fn inner(&self) -> Option<&dyn Capabilities> {
        None
    }

    async fn list_top_level_addresses(
        &self,
        request: Request<ListTopLevelAddressesRequest>,
    ) -> Result<ListTopLevelAddressesResponse> {
        self.inner
            .list_top_level_addresses(request)
            .await
            .inspect_err(|error| log_storage_error(error, "list_top_level_addresses"))
    }
}
