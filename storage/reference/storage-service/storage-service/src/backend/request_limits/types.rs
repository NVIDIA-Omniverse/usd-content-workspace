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

use storage_core::traits::StreamingOptions;

use crate::backend::request_limits::MAX_PAGE_SIZE;

pub trait StreamingOptionsExt {
    fn with_page_size_cap(self) -> Self;
}

impl StreamingOptionsExt for StreamingOptions {
    fn with_page_size_cap(mut self) -> Self {
        if let StreamingOptions::Paginated(ref mut options) = self {
            options.max_page_size = options.max_page_size.map(|v| v.min(MAX_PAGE_SIZE))
        }

        self
    }
}
