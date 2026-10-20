#!/bin/bash
# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

# Script to generate OpenAPI specification from Rust code annotations
# This ensures the static YAML file stays in sync with the actual implementation

set -e

echo " Generating OpenAPI specification from Rust code..."

# Check if we're in the right directory
if [ ! -f "Cargo.toml" ] || [ ! -d "crates/service" ]; then
    echo " Error: This doesn't appear to be the thumbnail-service project root"
    echo "Please run this script from the project root directory."
    exit 1
fi

# Function to print colored output
print_status() {
    local status=$1
    local message=$2
    if [ "$status" = "success" ]; then
        echo " $message"
    elif [ "$status" = "error" ]; then
        echo " $message"
    elif [ "$status" = "info" ]; then
        echo "[INFO]  $message"
    fi
}

# Build and run the OpenAPI generator
print_status "info" "Building OpenAPI generator..."
if cargo build --bin generate_openapi >/dev/null 2>&1; then
    print_status "success" "OpenAPI generator built successfully"
else
    print_status "error" "Failed to build OpenAPI generator"
    exit 1
fi

# Extract the OpenAPI specification
print_status "info" "Extracting OpenAPI specification..."
if cargo run --bin generate_openapi > thumbnail.openapi.yaml.new 2>/dev/null; then
    print_status "success" "OpenAPI specification extracted successfully"

    # Compare with existing file
    if [ -f "thumbnail.openapi.yaml" ]; then
        if diff -q thumbnail.openapi.yaml thumbnail.openapi.yaml.new >/dev/null 2>&1; then
            print_status "success" "OpenAPI specification is up to date"
            rm thumbnail.openapi.yaml.new
        else
            print_status "info" "OpenAPI specification has been updated"
            mv thumbnail.openapi.yaml.new thumbnail.openapi.yaml
            echo " The thumbnail.openapi.yaml file has been updated to match the Rust implementation"
        fi
    else
        mv thumbnail.openapi.yaml.new thumbnail.openapi.yaml
        print_status "success" "Created new thumbnail.openapi.yaml file"
    fi
else
    print_status "error" "Failed to extract OpenAPI specification"
    rm -f thumbnail.openapi.yaml.new
    exit 1
fi

# No cleanup needed since we use a proper binary

print_status "success" "OpenAPI specification generation completed"
