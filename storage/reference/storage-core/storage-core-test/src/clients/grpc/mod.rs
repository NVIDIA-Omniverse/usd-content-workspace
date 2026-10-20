// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

pub mod capabilities;
pub mod filefolder;
pub mod fileobject;
pub mod metadata;
pub mod versioning;

use std::str::FromStr;

use crate::headers::HeadersConfig;
use tonic::{
    metadata::{Ascii, MetadataKey, MetadataValue},
    Request,
};
pub async fn connect_with_retries<F, Fut, T>(f: F) -> T
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T, tonic::transport::Error>>,
{
    let mut retry = 0;
    loop {
        match f().await {
            Ok(t) => break t,
            Err(e) => {
                retry += 1;
                if retry > 1000 {
                    panic!("Failed to connect to server: {e:?}");
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                continue;
            },
        }
    }
}

pub fn request_with_headers_config<T>(request: T, headers_config: &HeadersConfig) -> Request<T> {
    let mut request = Request::new(request);
    for (name, value) in headers_config.get_headers() {
        request.metadata_mut().insert(
            MetadataKey::<Ascii>::from_str(name.as_ref()).unwrap(),
            MetadataValue::<Ascii>::from_str(value.as_ref()).unwrap(),
        );
    }
    request
}
