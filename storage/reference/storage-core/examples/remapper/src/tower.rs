// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use figment::Figment;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use storage_core_util::info_limited;
use tower::{Layer, Service};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Prefix to prepend to all log messages.
    /// Default is "Log Layer"
    #[serde(default = "default_prefix")]
    prefix: String,
}

fn default_prefix() -> String {
    "Log Layer".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            prefix: default_prefix(),
        }
    }
}

impl Config {
    pub const CONFIG_KEY: &str = "log";
}

impl TryFrom<&Figment> for Config {
    type Error = figment::Error;

    fn try_from(figment: &Figment) -> std::result::Result<Self, Self::Error> {
        figment.focus(Self::CONFIG_KEY).extract()
    }
}

/// Log layer that logs a configurable message for all requests
#[derive(Clone)]
pub struct LogLayer {
    config: Config,
}

impl LogLayer {
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    pub fn from_figment(figment: &Figment) -> std::result::Result<Self, figment::Error> {
        let config = Config::try_from(figment)?;
        Ok(Self::new(config))
    }
}

impl<S> Layer<S> for LogLayer {
    type Service = LogLayerService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        LogLayerService {
            inner,
            prefix: self.config.prefix.clone(),
        }
    }
}

#[derive(Clone)]
pub struct LogLayerService<S> {
    inner: S,
    prefix: String,
}

impl<S, Request> Service<Request> for LogLayerService<S>
where
    S: Service<Request> + Send + 'static,
    S::Future: Send + 'static,
    Request: std::fmt::Debug,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request) -> Self::Future {
        info_limited!("{}: {:?}", self.prefix, req);
        let fut = self.inner.call(req);
        Box::pin(async move {
            let response = fut.await?;
            Ok(response)
        })
    }
}
