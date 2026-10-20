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

FROM rust:1.97 AS builder

WORKDIR /root/source

RUN apt-get update && apt-get install --no-install-recommends --yes protobuf-compiler libprotobuf-dev pbzip2 python3

# build the auth extension
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/root/.cache/target \
    --mount=type=bind,source=..,target=/root/source \
    OUT_DIR=/root/.cache/target \
    CARGO_TARGET_DIR=/root/.cache/target \
    cargo build --package envoy-auth-ext --release \
        && mkdir /app \
        && cp /root/.cache/target/release/envoy-auth-ext /app/

# vendor all 3rd party crates for OSRB
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/root/.cache/target \
    --mount=type=bind,source=..,target=/root/source \
    CARGO_TARGET_DIR=/root/.cache/target \
    cargo vendor /root/.cache/vendor --color never --locked --versioned-dirs \
        && tar -cf - /root/.cache/vendor | pbzip2 -c > /app/vendor.tar.bz2

# Generate license attributions for release builds
# This step is only enabled when GENERATE_LICENSES=true (set by the release pipeline for tagged releases)
# Generates two files:
#   - attributions.md: Full license texts for all third-party dependencies
#   - license_links.md: Table with verified links to license files in repositories
# Note: This process takes 5-10 minutes due to URL verification with conservative rate limiting
ARG GENERATE_LICENSES="false"
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=bind,source=..,target=/root/source \
    if [ "$GENERATE_LICENSES" = "true" ]; then \
        echo "Generating license attributions (this may take 5-10 minutes)..."; \
        cd /root/source && \
        cargo metadata --format-version=1 2>/dev/null > /app/metadata && \
        cd /app && cat metadata | python3 /root/source/scripts/generate_licenses.py 2>&1 && \
        echo "✓ License files generated and copied to /app/"; \
    else \
        echo "Skipping license generation (GENERATE_LICENSES=$GENERATE_LICENSES)"; \
        echo "License generation only runs for tagged releases"; \
        touch /app/attributions.md /app/license_links.md; \
    fi


FROM nvcr.io/nvidia/distroless/cc:v4.1.6
WORKDIR /app

COPY --from=builder "/app/envoy-auth-ext" "/usr/local/bin/envoy-auth-ext"
COPY --from=builder "/app/vendor.tar.bz2" "/vendor.tar.bz2"
COPY --from=builder "/app/attributions.md" "/licenses/attributions.md"
COPY --from=builder "/app/license_links.md" "/licenses/license_links.md"

EXPOSE 50051

ENV LOG_LEVEL="debug"
ENV OTEL_SERVICE_NAME="envoy-auth-ext"

CMD ["/usr/local/bin/envoy-auth-ext","-c","/app/envoy-ext-config.toml"]
