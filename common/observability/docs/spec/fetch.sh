#!/usr/bin/env bash
# SPDX-FileCopyrightText: Copyright (c) 2024-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
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

# fetches remote docs/specs, so we can track any changes in the repo itself
set -euo pipefail

# specify this to fetch a specific version, e.g.
# OTEL_DOC_VERSION="v1.40.0" ./docs/fetch.sh
# (a specific version should usually be fetched as `main` changes often.)
OTEL_DOC_VERSION="${OTEL_DOC_VERSION:-"main"}"

cd "$(dirname "$0")"

# https://github.com/grpc/proposal/blob/master/A66-otel-stats.md
curl -fsSL --proto '=https' -o "A66-otel-stats.md" \
  'https://raw.githubusercontent.com/grpc/proposal/master/A66-otel-stats.md'

# https://github.com/open-telemetry/semantic-conventions/blob/main/docs/http/http-metrics.md
curl -fsSL --proto '=https' -o "otel-http-metrics.md" \
  "https://raw.githubusercontent.com/open-telemetry/semantic-conventions/${OTEL_DOC_VERSION}/docs/http/http-metrics.md"

# https://github.com/open-telemetry/semantic-conventions/blob/main/docs/http/http-spans.md
curl -fsSL --proto '=https' -o "otel-http-spans.md" \
  "https://raw.githubusercontent.com/open-telemetry/semantic-conventions/${OTEL_DOC_VERSION}/docs/http/http-spans.md"

# https://github.com/open-telemetry/semantic-conventions/blob/main/docs/http/http-exceptions.md
curl -fsSL --proto '=https' -o "otel-http-exceptions.md" \
  "https://raw.githubusercontent.com/open-telemetry/semantic-conventions/${OTEL_DOC_VERSION}/docs/http/http-exceptions.md"

# https://github.com/open-telemetry/semantic-conventions/blob/main/docs/rpc/rpc-metrics.md
curl -fsSL --proto '=https' -o "otel-rpc-metrics.md" \
  "https://raw.githubusercontent.com/open-telemetry/semantic-conventions/${OTEL_DOC_VERSION}/docs/rpc/rpc-metrics.md"

# https://github.com/open-telemetry/semantic-conventions/blob/main/docs/rpc/rpc-spans.md
curl -fsSL --proto '=https' -o "otel-rpc-spans.md" \
  "https://raw.githubusercontent.com/open-telemetry/semantic-conventions/${OTEL_DOC_VERSION}/docs/rpc/rpc-spans.md"

# https://github.com/open-telemetry/semantic-conventions/blob/main/docs/rpc/rpc-exceptions.md
curl -fsSL --proto '=https' -o "otel-rpc-exceptions.md" \
  "https://raw.githubusercontent.com/open-telemetry/semantic-conventions/${OTEL_DOC_VERSION}/docs/rpc/rpc-exceptions.md"

# https://github.com/open-telemetry/semantic-conventions/blob/main/docs/rpc/grpc.md
curl -fsSL --proto '=https' -o "otel-grpc.md" \
  "https://raw.githubusercontent.com/open-telemetry/semantic-conventions/${OTEL_DOC_VERSION}/docs/rpc/grpc.md"

# https://github.com/open-telemetry/semantic-conventions/blob/main/docs/general/recording-errors.md
curl -fsSL --proto '=https' -o "otel-recording-errors.md" \
  "https://raw.githubusercontent.com/open-telemetry/semantic-conventions/${OTEL_DOC_VERSION}/docs/general/recording-errors.md"
