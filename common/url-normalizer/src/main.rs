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

use clap::Parser;
use url_normalizer::normalize_url;

#[derive(Parser)]
#[command(name = "url_normalizer")]
#[command(about = "A URL normalization tool")]
#[command(version = "1.0")]
struct Args {
    /// The URL to parse and normalize
    #[arg(required = true)]
    url: String,
}

fn main() {
    let args = Args::parse();

    match normalize_url(&args.url) {
        Ok(normalized_url) => {
            println!("{}", normalized_url);
        }
        Err(e) => {
            eprintln!("Error parsing URL: {}", e);
            std::process::exit(1);
        }
    }
}
