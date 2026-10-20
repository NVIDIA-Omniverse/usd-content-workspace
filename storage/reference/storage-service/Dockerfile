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

ARG FROM_IMAGE="nvcr.io/nvidia/distroless/cc:v4.1.5"
ARG BUILD_IMAGE="rust:1.96.1-bookworm"

# Path of this component inside the build context: "." when building from this
# directory. In NVIDIA-Omniverse/usd-content-workspace the workspace has
# path dependencies outside its own directory (storage-core, the Azure SDK fork and
# url-normalizer), so there the image is built from the repository root:
#
#   docker build -f storage/reference/storage-service/Dockerfile \
#     --build-arg COMPONENT_DIR=storage/reference/storage-service .
ARG COMPONENT_DIR="."

FROM ${BUILD_IMAGE} AS build
ARG COMPONENT_DIR

WORKDIR /root/source/${COMPONENT_DIR}

RUN apt-get update && apt-get install --no-install-recommends --yes pbzip2

# build the storage service
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/root/.cache/target \
    --mount=type=bind,source=.,target=/root/source \
    CARGO_TARGET_DIR=/root/.cache/target \
    cargo build --workspace --release \
        && mkdir /app \
        && cp /root/.cache/target/release/storage-service /app/

# vendor all 3rd party crates for the open-source license review
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/root/.cache/target \
    --mount=type=bind,source=.,target=/root/source \
    CARGO_TARGET_DIR=/root/.cache/target \
    cargo vendor /root/.cache/vendor --color never --locked --versioned-dirs \
        && for f in /root/.cache/vendor/storage-core-*/src/s3/service.rs; do \
               if [ -e "$f" ]; then sed -i '/^#\[cfg(test)\]/,$d' "$f"; fi; \
           done \
        && tar -cf - /root/.cache/vendor | pbzip2 -c > /app/vendor.tar.bz2

FROM python:3.13.12-slim AS build-attributions
ARG COMPONENT_DIR

WORKDIR /root/source/${COMPONENT_DIR}

# Generate license attributions file.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=bind,source=.,target=/root/source \
    mkdir -p /app && ./scripts/attributions.py -v render -f ./attributions.json > /app/attributions.md

FROM ${FROM_IMAGE} AS final

ARG COMPONENT_DIR
ARG GIT_REVISION="unknown"

LABEL \
    application="Storage Service" \
    description="NVIDIA Omniverse Storage Service" \
    version="${GIT_REVISION}"

LABEL \
    org.opencontainers.image.documentation="https://github.com/NVIDIA-Omniverse/usd-content-workspace/tree/main/storage/reference/storage-service" \
    org.opencontainers.image.licenses="Apache-2.0" \
    org.opencontainers.image.revision="${GIT_REVISION}" \
    org.opencontainers.image.source="https://github.com/NVIDIA-Omniverse/usd-content-workspace/tree/main/storage/reference/storage-service" \
    org.opencontainers.image.title="NVIDIA Omniverse Sevan Storage Service" \
    org.opencontainers.image.url="https://github.com/NVIDIA-Omniverse/usd-content-workspace/tree/main/storage/reference/storage-service" \
    org.opencontainers.image.vendor="NVIDIA CORPORATION"

COPY --from=build "/app/storage-service" "/usr/local/bin/storage-service"
COPY --from=build "/app/vendor.tar.bz2" "/vendor.tar.bz2"
COPY --from=build-attributions "/app/attributions.md" "/licenses/attributions.md"
COPY "${COMPONENT_DIR}/LICENSE" "/licenses/LICENSE"

USER nvs:nvs

ENTRYPOINT ["/usr/local/bin/storage-service"]
