// SPDX-FileCopyrightText: Copyright (c) 2024-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
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

use super::client::{Client, MakeClient};

pub(crate) trait HttpVersion {
    const VERSION: http::Version;

    fn client(make: &MakeClient) -> Client;

    fn otel() -> otel_asserts::types::common::Value {
        super::http::ver(Self::VERSION)
    }
}

#[derive(Debug)]
pub struct Http11;

impl HttpVersion for Http11 {
    const VERSION: http::Version = http::Version::HTTP_11;

    fn client(make: &MakeClient) -> Client {
        make.http1()
    }
}

#[derive(Debug)]
pub struct Http2;

impl HttpVersion for Http2 {
    const VERSION: http::Version = http::Version::HTTP_2;

    fn client(make: &MakeClient) -> Client {
        make.http2()
    }
}

macro_rules! http_test {
    () => {
        #[test]
        fn http11() {
            test::<Http11>();
        }

        #[test]
        fn http2() {
            test::<Http2>();
        }
    };
}
pub(crate) use http_test;
