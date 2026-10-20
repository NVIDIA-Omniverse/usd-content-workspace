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

# Start unified thumbnail service for LOCAL DEVELOPMENT
# Runs both HTTP and gRPC from a single process.

echo " Starting unified thumbnail service for LOCAL DEVELOPMENT"
echo "========================================================="
echo ""

# Kill any existing thumbnail service processes
pkill -f 'target/debug/server' 2>/dev/null
sleep 2

echo " Loading environment variables from .env file..."

# Check if .env file exists
if [ ! -f .env ]; then
    echo " ERROR: .env file not found!"
    echo " Please copy .env.example to .env and configure your values:"
    echo "   cp .env.example .env"
    echo "   # Edit .env with your configuration"
    exit 1
fi

echo " Environment variables will be loaded from .env file by the application"
echo ""

echo " Building and starting unified service with LOCAL configuration..."
echo " Branch: $(git branch --show-current)"
echo " Last commit: $(git log --oneline -1)"
echo ""
cargo run --bin server
