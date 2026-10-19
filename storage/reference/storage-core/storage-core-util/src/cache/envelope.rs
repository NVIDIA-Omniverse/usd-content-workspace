// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use serde::{Deserialize, Serialize};

/// Wire envelope for L2 cache values. The `schema` field tags the serde
/// representation so rolling deploys with breaking type changes can detect
/// the mismatch and treat the entry as a miss rather than mis-deserialising.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Envelope<V> {
    pub schema: u32,
    pub value: V,
}
