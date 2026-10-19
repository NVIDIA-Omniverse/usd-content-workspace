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

FROM rust:1.97-bookworm AS builder
ARG DEBIAN_FRONTEND="noninteractive"

RUN apt-get update && apt-get install --no-install-recommends --yes pbzip2 protobuf-compiler

WORKDIR /usr/src/app
COPY src src
COPY discovery-protos discovery-protos
COPY build.rs .
COPY Cargo.toml .
COPY Cargo.lock .
COPY LICENSE.txt .
RUN cargo build --package auto-discovery-service --release

# vendor all 3rd party crates for OSRB
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/root/.cache/target \
    --mount=type=bind,source=.,target=/usr/src/app \
    CARGO_TARGET_DIR=/root/.cache/target \
    cargo vendor /root/.cache/vendor --color never --locked --versioned-dirs && \
    mkdir /vendor && \
    tar -cf - /root/.cache/vendor | pbzip2 -c > /vendor/vendor.tar.bz2


FROM nvcr.io/nvidia/distroless/cc:v4.1.6

WORKDIR /app
COPY --from=builder /usr/src/app/target/release/auto-discovery-service /app/auto-discovery-service
COPY --from=builder /vendor/vendor.tar.bz2 /app/vendor.tar.bz2
COPY LICENSE.txt .
COPY ATTRIBUTIONS.md .

EXPOSE 8080
EXPOSE 8081

USER 1000:1000

CMD ["/app/auto-discovery-service"]
