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

FROM rust:1.94 AS builder
ARG DEBIAN_FRONTEND=noninteractive

# No protobuf compiler: the proto crates ship generated code, so nothing here runs
# `protoc`. Regeneration is `buf generate`, run deliberately outside the image build.
RUN apt-get update && apt-get install --no-install-recommends --yes pbzip2 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src/app
COPY crates crates
COPY vendor vendor
COPY proto proto
COPY Cargo.toml .
COPY Cargo.lock .
COPY LICENSE.txt .

# --locked, matching CI. Without it the image build is free to resolve versions
# the lockfile does not pin, so the artifact that ships can differ from the one
# every test ran against -- and Cargo.lock is copied in above precisely so it
# does not have to.
RUN cargo build --locked \
    --package thumbnail-service \
    --package thumbnail-task-worker \
    --package thumbnail-smoke-test \
    --release

# Vendor all third-party crates. This is a compliance artifact, not a build
# convenience: OSRB's low-risk auto-approval is conditional on third-party
# source-code distribution actually happening, and this archive -- copied into
# the published images below -- is how that obligation is discharged. Removing it
# to save image size would remove the basis on which every dependency is
# approved, and no test would fail. See docs/oss-compliance.md and
# openspec/specs/oss-license-compliance.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/root/.cache/target \
    --mount=type=bind,source=.,target=/usr/src/app \
    CARGO_TARGET_DIR=/root/.cache/target \
    cargo vendor /root/.cache/vendor --color never --locked --versioned-dirs && \
    mkdir /vendor && \
    tar -cf - /root/.cache/vendor | pbzip2 -c > /vendor/vendor.tar.bz2

# ---------------------------------------------------------------------------
# task-worker — the conversion executor.
#
# Its own image rather than a second process in the service image: image codecs
# and their memory profile have no business on the API deployment, and the two
# scale on completely different signals. Built with `--target task-worker`.
#
# Declared before the service stage on purpose: the last stage in the file is
# what `docker build` produces without an explicit `--target`, and that must stay
# the service so existing pipelines are unaffected.
#
# The NGC base permits anonymous pulls, including CI's docker-build component
# with NGC login disabled. Keep the service, worker, and standalone conformance
# runtime on the same version. The Debian 13 base retains nvs (UID/GID 1000),
# CA certificates, and libssl.so.3/libcrypto.so.3 for the dynamically linked
# binaries. The Rust builder remains unchanged.
# ---------------------------------------------------------------------------
FROM nvcr.io/nvidia/distroless/cc:v4.1.4 AS task-worker

LABEL maintainer="NVIDIA"
LABEL org.opencontainers.image.title="Thumbnail Task Worker"
LABEL org.opencontainers.image.description="Consumes omni.thumbnail.generate work and produces thumbnails"
LABEL org.opencontainers.image.vendor="NVIDIA"
LABEL org.opencontainers.image.source="https://github.com/NVIDIA-Omniverse/usd-content-workspace/tree/main/thumbnails/reference/thumbnail-service"

WORKDIR /app

COPY --from=builder /usr/src/app/target/release/task-worker /app/task-worker
COPY --from=builder /vendor/vendor.tar.bz2 /app/vendor.tar.bz2
COPY LICENSE.txt .
COPY ATTRIBUTIONS.md .

# Probes only. The worker serves no API: it consumes WorkerEventQueue and calls out.
EXPOSE 8080

USER nvs

CMD ["/app/task-worker"]

# ---------------------------------------------------------------------------
# server — REST and gRPC. Default build target.
# ---------------------------------------------------------------------------
FROM nvcr.io/nvidia/distroless/cc:v4.1.4 AS server

LABEL maintainer="NVIDIA"
LABEL org.opencontainers.image.title="Thumbnail Service"
LABEL org.opencontainers.image.description="HTTP and gRPC thumbnail service for processing and serving thumbnails"
LABEL org.opencontainers.image.vendor="NVIDIA"
LABEL org.opencontainers.image.source="https://github.com/NVIDIA-Omniverse/usd-content-workspace/tree/main/thumbnails/reference/thumbnail-service"

WORKDIR /app

COPY --from=builder /usr/src/app/target/release/server /app/server
COPY --from=builder /usr/src/app/target/release/thumbnail-smoke-test /app/thumbnail-smoke-test
COPY --from=builder /vendor/vendor.tar.bz2 /app/vendor.tar.bz2
COPY LICENSE.txt .
COPY ATTRIBUTIONS.md .

EXPOSE 3000 50051

USER nvs

# Unified server hosts both HTTP and gRPC in one process.
CMD ["/app/server"]
