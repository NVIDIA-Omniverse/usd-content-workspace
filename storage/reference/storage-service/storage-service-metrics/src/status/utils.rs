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

use storage_core_util::observability::metrics::{BoundUIntCounter, UIntCounter};

use crate::utils::BindExt;

pub trait BindResultExt {
    type Bound;

    fn bind_result(&self, result: &'static str) -> Self::Bound;
}

impl BindResultExt for UIntCounter {
    type Bound = BoundUIntCounter;

    fn bind_result(&self, result: &'static str) -> Self::Bound {
        self.bind_with_default_attributes([("result", result)])
    }
}
