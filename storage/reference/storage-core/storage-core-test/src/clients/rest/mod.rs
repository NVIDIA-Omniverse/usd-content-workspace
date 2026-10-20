// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

pub mod capabilities;
pub mod filefolder;
pub mod fileobject;
pub mod metadata;
pub mod versioning;

use http::{HeaderMap, HeaderName, HeaderValue};
use std::str::FromStr;

pub async fn request_with_retries<F, Fut, T>(f: F) -> Result<T, reqwest::Error>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T, reqwest::Error>>,
{
    let mut retry = 0;
    loop {
        match f().await {
            Ok(t) => break Ok(t),
            Err(e) => {
                if e.is_connect() || e.is_timeout() {
                    retry += 1;
                    if retry > 1000 {
                        panic!("Failed to connect to server: {e:?}");
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    continue;
                } else {
                    return Err(e);
                }
            },
        }
    }
}

fn check_status_code(status_code: reqwest::StatusCode) -> Result<(), tonic::Status> {
    match status_code {
        reqwest::StatusCode::OK => Ok(()),
        reqwest::StatusCode::CREATED => Ok(()),
        reqwest::StatusCode::ACCEPTED => Ok(()),
        reqwest::StatusCode::NO_CONTENT => Ok(()),
        reqwest::StatusCode::MULTIPLE_CHOICES => Ok(()),
        reqwest::StatusCode::NOT_FOUND => Err(tonic::Status::not_found("")),
        reqwest::StatusCode::PRECONDITION_FAILED => Err(tonic::Status::failed_precondition("")),
        reqwest::StatusCode::CONFLICT => Err(tonic::Status::already_exists("")),
        reqwest::StatusCode::RANGE_NOT_SATISFIABLE => Err(tonic::Status::out_of_range("")),
        reqwest::StatusCode::REQUEST_TIMEOUT => Err(tonic::Status::deadline_exceeded("")),
        reqwest::StatusCode::SERVICE_UNAVAILABLE => Err(tonic::Status::unavailable("")),
        reqwest::StatusCode::INTERNAL_SERVER_ERROR => Err(tonic::Status::internal("")),
        reqwest::StatusCode::BAD_REQUEST => Err(tonic::Status::invalid_argument("")),
        reqwest::StatusCode::FORBIDDEN => Err(tonic::Status::permission_denied("")),
        reqwest::StatusCode::TOO_MANY_REQUESTS => Err(tonic::Status::resource_exhausted("")),
        reqwest::StatusCode::NOT_IMPLEMENTED => Err(tonic::Status::unimplemented("")),
        reqwest::StatusCode::METHOD_NOT_ALLOWED => Err(tonic::Status::unimplemented("")),
        _ => Err(tonic::Status::unknown("")),
    }
}

pub fn headers_to_header_map<'a>(headers: impl Iterator<Item = (&'a String, &'a String)>) -> HeaderMap<HeaderValue> {
    let mut header_map = HeaderMap::new();
    for (name, value) in headers {
        if let (Ok(name), Ok(value)) = (
            HeaderName::from_str(name.as_ref()),
            HeaderValue::from_str(value.as_ref()),
        ) {
            header_map.insert(name, value);
        }
    }
    header_map
}
