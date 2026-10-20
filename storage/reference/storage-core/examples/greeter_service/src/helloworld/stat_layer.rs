// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use figment::Figment;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};
use storage_core::layer::Layer;
use storage_core::traits::{
    async_trait,
    fileobject::v1alpha::{FileObject, StatRequest, StatResponse},
    Request, Result,
};

use super::traits::{Greeter, HelloReply, HelloRequest};

/// The Config struct is used to configure the StatLayer.
/// It is used to enable or disable the StatLayer.
/// The default is true.
/// Additional configuration can be added here if needed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Enable the stat layer.
    /// Default is true
    #[serde(default = "default_enabled")]
    enabled: bool,
}

fn default_enabled() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
        }
    }
}

impl Config {
    pub const CONFIG_KEY: &str = "stat_layer";
}

impl TryFrom<&Figment> for Config {
    type Error = figment::Error;

    fn try_from(figment: &Figment) -> std::result::Result<Self, Self::Error> {
        figment.focus(Self::CONFIG_KEY).extract()
    }
}

/// The StatLayer is a layer that stores the last stat operation and adds it to the response of the Greeter Service.
/// Any shared state needed for your layer should be stored in the layer struct.
pub struct StatLayer {
    last_stat: Arc<RwLock<Option<StatResponse>>>,
}

impl StatLayer {
    pub fn new(config: Config) -> Option<Self> {
        if config.enabled {
            Some(Self {
                last_stat: Arc::new(RwLock::new(None)),
            })
        } else {
            None
        }
    }

    pub fn from_figment(figment: &Figment) -> std::result::Result<Option<Self>, figment::Error> {
        let config = Config::try_from(figment)?;
        Ok(Self::new(config))
    }
}

/// The StatLayer implements the Layer trait from the storage-core library.
impl<S> Layer<S> for StatLayer {
    type Service = StatService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        StatService::new(inner, self.last_stat.clone())
    }
}

/// The StatService struct is what will actually implement the traits. Each trait implementation
/// will be it's own instance of the StatService struct, so any shared state must be stored in the Layer struct
/// itself and passed to the StatService constructor.
pub struct StatService<S> {
    inner: S,
    last_stat: Arc<RwLock<Option<StatResponse>>>,
}

impl<S> StatService<S> {
    pub fn new(inner: S, last_stat: Arc<RwLock<Option<StatResponse>>>) -> Self {
        Self { inner, last_stat }
    }
}

/// The StatService implements the FileObject trait from the storage-core library.
/// Layers only need to implement the functions they are interested in. The trait definitions
/// provide the default implementations for the other functions that pass through to the inner service.
#[async_trait]
impl<S: FileObject> FileObject for StatService<S> {
    /// You must implement the inner function to return a reference to the inner service.
    /// This is used by the storage-core library to pass through to the inner service in the default implementations.
    fn inner(&self) -> Option<&dyn FileObject> {
        Some(&self.inner)
    }

    async fn stat(&self, request: Request<StatRequest>) -> Result<StatResponse> {
        let stat = self.inner.stat(request).await?;
        *self.last_stat.write().unwrap() = Some(stat.clone());
        Ok(stat)
    }
}

/// The StatService implements the Greeter trait from our example service.
#[async_trait]
impl<S: Greeter> Greeter for StatService<S> {
    fn inner(&self) -> Option<&dyn Greeter> {
        Some(&self.inner)
    }

    async fn say_hello(&self, request: Request<HelloRequest>) -> Result<HelloReply> {
        let mut response = self.inner.say_hello(request).await?;
        let last_stat = self.last_stat.read().unwrap().clone();
        if let Some(last_stat) = last_stat {
            response.message = format!(
                "{}. By the way, the last stat result was {:?}",
                response.message, last_stat
            );
        }
        Ok(response)
    }
}
