# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
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

# ---------------------------------------------------------------------------
# Stage 1 — build the conformance tests wheel
# ---------------------------------------------------------------------------
FROM python:3.13-slim AS builder

WORKDIR /build

# Install uv for fast dependency resolution and wheel building
RUN pip install --no-cache-dir uv

COPY pyproject.toml uv.lock* ./
COPY src/ ./src/

# Build the wheel
RUN uv build --wheel --out-dir /dist

# ---------------------------------------------------------------------------
# Stage 2 — minimal runtime image
# ---------------------------------------------------------------------------
FROM python:3.13-slim

WORKDIR /app

# curl is needed by wait_for_service.sh
RUN apt-get update && apt-get install -y --no-install-recommends curl \
    && rm -rf /var/lib/apt/lists/*

# Install the conformance tests wheel
COPY --from=builder /dist/*.whl /tmp/
RUN pip install --no-cache-dir /tmp/*.whl && rm /tmp/*.whl

COPY wait_for_service.sh run_tests.sh ./
RUN chmod +x wait_for_service.sh run_tests.sh

# Test output directories must be writable by the non-root runtime user.
RUN mkdir -p /app/reports /app/.pytest_cache \
    && chown -R 65534:65534 /app/reports /app/.pytest_cache

# Run as non-root (nobody = UID 65534, already exists in the base image)
USER nobody

CMD ["sh", "-c", "./wait_for_service.sh && ./run_tests.sh"]
