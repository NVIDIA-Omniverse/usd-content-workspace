# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

FROM python:3.14-slim AS builder

WORKDIR /build

# Copy only what we need to build the wheel
COPY pyproject.toml ./
COPY src/ ./src/

# Install uv (exact release, hash-checked: see the service Dockerfile), then
# build the wheel.
RUN printf '%s\n' 'uv==0.12.5 --hash=sha256:3e195ccf1ed60c8bb24a6447ce306441a4181d54b602407e09bc56e963911c15 --hash=sha256:58abfb0f658b39a834307a11223bc170294ea214263b4c99ecc7663720d43544' \
        > /tmp/uv-requirements.txt && \
    pip install --no-cache-dir --require-hashes -r /tmp/uv-requirements.txt && \
    uv build --wheel --out-dir /dist

# ---------------------------------------------------------------------------

FROM python:3.14-slim

WORKDIR /app

COPY --from=builder /dist /dist
COPY run_tests.sh ./
COPY wait_for_service.sh ./

RUN pip install --no-cache-dir /dist/*.whl && \
    rm -rf /dist && \
    chmod +x run_tests.sh wait_for_service.sh && \
    apt-get update && \
    apt-get install -y --no-install-recommends curl && \
    rm -rf /var/lib/apt/lists/*

# Default environment (override via docker run -e or Helm values)
ENV CONFORMANCE_GRPC_ENDPOINT=localhost:50052
ENV CONFORMANCE_REST_ENDPOINT=http://localhost:8000
ENV CONFORMANCE_AUTH_TOKEN=""
ENV CONFORMANCE_AUTH_REQUIRED=false
ENV CONFORMANCE_STREAM_TIMEOUT=10.0
ENV CONFORMANCE_REQUEST_TIMEOUT=30.0

# Run as non-root (matches helm values.yaml runAsUser: 65534)
USER 65534

CMD ["sh", "-c", "./wait_for_service.sh && ./run_tests.sh"]
