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

use std::str::FromStr;

use crate::common::headers::get_headers;
use storage_core_protos::google::protobuf::Timestamp;
use time::OffsetDateTime;
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

pub fn request_with_headers<T>(request: T) -> Request<T> {
    let mut request = Request::new(request);
    for (name, value) in get_headers() {
        request.metadata_mut().insert(
            MetadataKey::<Ascii>::from_str(name.as_ref()).unwrap(),
            MetadataValue::<Ascii>::from_str(value.as_ref()).unwrap(),
        );
    }
    request
}

pub fn create_timestamp_from_datetime(value: impl Into<OffsetDateTime>) -> Timestamp {
    let value = value.into();
    Timestamp {
        seconds: value.unix_timestamp(),
        nanos: value.nanosecond() as i32,
    }
}
