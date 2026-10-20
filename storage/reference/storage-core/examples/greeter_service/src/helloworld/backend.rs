// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use figment::Figment;
use serde::{Deserialize, Serialize};
use storage_core::traits::{
    async_trait,
    error::{status_with_default_error_info, StatusCode},
    Request, Result, StorageErrorExt,
};

use super::traits::{Greeter as GreeterTrait, HelloReply, HelloRequest};

/// The Config struct is used to configure the Greeter Service.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_greeter_name")]
    pub greeter_name: String,
    #[serde(default = "default_rejected_names")]
    pub rejected_names: Vec<String>,
}

fn default_greeter_name() -> String {
    "Your Best Friend".to_string()
}

fn default_rejected_names() -> Vec<String> {
    vec![]
}

impl Default for Config {
    fn default() -> Self {
        Self {
            greeter_name: default_greeter_name(),
            rejected_names: default_rejected_names(),
        }
    }
}

impl Config {
    pub const CONFIG_KEY: &str = "greeter";
}

impl TryFrom<&Figment> for Config {
    type Error = figment::Error;

    fn try_from(figment: &Figment) -> std::result::Result<Self, Self::Error> {
        figment.focus(Self::CONFIG_KEY).extract()
    }
}

/// The GreeterBackend is the concrete implementation of the Greeter Service.
pub struct GreeterBackend {
    config: Config,
}

impl GreeterBackend {
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    pub fn from_figment(figment: &Figment) -> std::result::Result<Self, figment::Error> {
        let config = Config::try_from(figment)?;
        Ok(Self::new(config))
    }
}

/// The GreeterBackend implements the GreeterTrait.
#[async_trait]
impl GreeterTrait for GreeterBackend {
    /// Backends do not have an inner service, so we return None. This will let the layer stack know that there's
    /// no inner service to pass through to. This prevents infinite recursion when the backend does not implement
    /// all of the functions in the trait.
    fn inner(&self) -> Option<&dyn GreeterTrait> {
        None
    }

    async fn say_hello(&self, request: Request<HelloRequest>) -> Result<HelloReply> {
        let request = request.into_inner();
        if self.config.rejected_names.contains(&request.name) {
            return Err(GreeterError("Name is rejected".to_string()).into());
        }
        Ok(HelloReply {
            message: format!("Hello, {}! I'm {}.", request.name, self.config.greeter_name),
        })
    }
}

struct GreeterError(String);

impl std::fmt::Display for GreeterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GreeterError: {}", self.0)
    }
}

impl std::fmt::Debug for GreeterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GreeterError: {:?}", self.0)
    }
}

impl std::error::Error for GreeterError {}

/// All traits return a StorageError, which is a `Box<dyn StorageErrorExt>`. Therefore any error that is returned
/// from a trait method will need to implement the StorageErrorExt trait.
impl StorageErrorExt for GreeterError {
    /// Convert the error to a tonic status.
    fn tonic_status(&self) -> tonic::Status {
        status_with_default_error_info(tonic::Code::InvalidArgument, self.0.clone(), "InvalidArgument")
    }

    /// Convert the error to an HTTP status code.
    fn http_status(&self) -> StatusCode {
        StatusCode::BAD_REQUEST
    }

    /// Get the size of the error. This is needed in case the error is cached.
    fn get_size(&self) -> usize {
        std::mem::size_of::<GreeterError>() + self.0.capacity()
    }
}
