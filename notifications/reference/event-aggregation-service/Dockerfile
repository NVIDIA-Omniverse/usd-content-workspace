# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

FROM --platform=linux/amd64 python:3.14-slim as builder

WORKDIR /code

COPY --from=ghcr.io/astral-sh/uv:latest /uv /usr/local/bin/uv
COPY ./pyproject.toml ./uv.lock /code/
# notification-common is locked as the sibling ../common directory, which is
# outside this folder's build context. Pass it as a named build context:
#   docker build --build-context common=../common .
COPY --from=common . /common/

RUN uv export --no-dev --locked --no-emit-project > requirements.txt && \
    uv pip install --system --compile-bytecode --no-deps packaging -r requirements.txt

################################################################################
######################### Production Distroless Stage ##########################
################################################################################

FROM nvcr.io/nvidia/distroless/python:3.14-v4.1.4

WORKDIR /code

# Copy Python packages from builder stage
COPY --from=builder /usr/local/lib/python3.14/site-packages /usr/local/lib/python3.14/site-packages

# Copy application code
COPY . /code

# Set PYTHONPATH to ensure packages are found
ENV PYTHONPATH=/usr/local/lib/python3.14/site-packages

# Run as non-root user
USER nvs

# Distroless has no shell, so we directly invoke Python
ENTRYPOINT ["python", "-um", "app.main"]
