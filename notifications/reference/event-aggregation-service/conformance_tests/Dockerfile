# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

# ---------------------------------------------------------------------------
# Stage 1 — build the conformance tests wheel
# ---------------------------------------------------------------------------
FROM python:3.14-slim AS builder

WORKDIR /build

# Install uv for fast dependency resolution and wheel building
RUN pip install --no-cache-dir uv

COPY pyproject.toml uv.lock ./
COPY src/ ./src/

# Build the wheel
RUN uv build --wheel --out-dir /dist

# ---------------------------------------------------------------------------
# Stage 2 — minimal runtime image
# ---------------------------------------------------------------------------
FROM python:3.14-slim

WORKDIR /app

# curl is needed by wait_for_service.sh
RUN apt-get update && apt-get install -y --no-install-recommends curl \
    && rm -rf /var/lib/apt/lists/*

# Install the conformance tests wheel
COPY --from=builder /dist/*.whl /tmp/
RUN pip install --no-cache-dir /tmp/*.whl && rm /tmp/*.whl

COPY wait_for_service.sh run_tests.sh ./
RUN chmod +x wait_for_service.sh run_tests.sh

# Reports directory
RUN mkdir -p /app/reports

# Run as non-root (nobody = UID 65534, already exists in the base image)
USER nobody

CMD ["sh", "-c", "./wait_for_service.sh && ./run_tests.sh"]
