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

use url_normalizer::normalize_url;

fn main() {
    // Example URLs to normalize
    let urls = vec![
        "https://example.com//path//to//file?b=2&a=1#fragment",
        "http://example.com:8080/path/to/file?param=value",
        "https://example.com/path with spaces/file.txt",
    ];

    println!("URL Normalization Examples:\n");

    for url in urls {
        match normalize_url(url) {
            Ok(normalized) => {
                println!("Original: {}", url);
                println!("Normalized: {}", normalized);
                println!();
            }
            Err(e) => {
                println!("Error normalizing {}: {}", url, e);
                println!();
            }
        }
    }
}
