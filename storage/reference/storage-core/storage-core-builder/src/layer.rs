// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

#[doc(inline)]
pub use tower::layer::util::Identity;
#[doc(inline)]
pub use tower::layer::util::Stack;
#[doc(inline)]
pub use tower::layer::Layer;

pub struct LayerArc<T>(pub Arc<T>);

impl<T> Clone for LayerArc<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T, S> Layer<S> for LayerArc<T>
where
    T: Layer<S>,
{
    type Service = T::Service;

    fn layer(&self, service: S) -> Self::Service {
        self.0.layer(service)
    }
}
