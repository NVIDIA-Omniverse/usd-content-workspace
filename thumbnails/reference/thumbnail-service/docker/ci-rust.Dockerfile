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
#
# Project-owned CI image for Rust jobs (.rust:base).
#
# Pinning model:
# - Ubuntu base is digest-pinned (rebuilds don't silently float the rootfs).
# - apt packages stay soft (security updates on deliberate rebuilds); installed
#   versions are recorded under /etc/ci-rust/apt-manifest.txt for forensics.
# - rustup-init, buf, and protoc are checksum-verified downloads.
#
# To bump: update digests/checksums/versions below as needed, set a new
# DOCKER_CI_RUST_IMAGE_TAG in .gitlab-ci.yml, and rebuild in the same MR.

# Tag is kept alongside the digest: the digest is what actually pins the rootfs,
# the tag says which release it is (and lets tooling resolve it to 22.04 rather
# than assuming `latest`). Refreshed 2026-08-10.
FROM ubuntu:22.04@sha256:3b06811b2afd352be909dd088a004166d665dc76d38b13eada33522a9d915c6f

ARG DEBIAN_FRONTEND=noninteractive
# Must match BUF_VERSION in .gitlab-ci.yml when the image is built.
ARG BUF_VERSION=1.28.1
# https://github.com/protocolbuffers/protobuf/releases/tag/v28.3
ARG PROTOC_VERSION=28.3
# https://static.rust-lang.org/rustup/release-stable.toml — pin the installer,
# not the floating /rustup/dist/ path.
ARG RUSTUP_VERSION=1.29.0

ENV DEBIAN_FRONTEND=noninteractive

WORKDIR /app

# Soft-pinned apt set: floating within the digest-pinned base's archive view.
# openssl/libssl-dev are intentionally not pkg=version locked so a deliberate
# rebuild can pick up Jammy security updates; record exact versions instead.
# protobuf-compiler comes from a checksummed release below, not apt.
RUN apt-get update -y && apt-get install -y --no-install-recommends \
    ca-certificates \
    build-essential \
    curl \
    git \
    pkg-config \
    openssl \
    libssl-dev \
    unzip \
    && mkdir -p /etc/ci-rust \
    && dpkg-query -W -f='${Package}\t${Version}\n' \
      ca-certificates build-essential curl git pkg-config openssl libssl-dev unzip \
      > /etc/ci-rust/apt-manifest.txt \
    && cat /etc/ci-rust/apt-manifest.txt \
    && rm -rf /var/lib/apt/lists/*

# Install rustup (no default toolchain yet), checksum-verified from a versioned
# archive path — never execute the floating /rustup/dist/ binary without a pin.
# https://rust-lang.github.io/rustup/installation/other.html#manual-installation
RUN arch="$(uname -m)" \
    && case "${arch}" in \
         x86_64) rustup_sha256="4acc9acc76d5079515b46346a485974457b5a79893cfb01112423c89aeb5aa10" ;; \
         aarch64) rustup_sha256="9732d6c5e2a098d3521fca8145d826ae0aaa067ef2385ead08e6feac88fa5792" ;; \
         *) echo "Unsupported architecture for rustup: ${arch}" >&2; exit 1 ;; \
       esac \
    && curl --proto '=https' --tlsv1.2 --silent --show-error --fail --location \
         --output rustup-init \
         "https://static.rust-lang.org/rustup/archive/${RUSTUP_VERSION}/${arch}-unknown-linux-gnu/rustup-init" \
    && echo "${rustup_sha256}  rustup-init" | sha256sum -c - \
    && chmod +x rustup-init \
    && ./rustup-init --version \
    && ./rustup-init -y --default-toolchain 'none' --no-update-default-toolchain --no-modify-path \
    && rm rustup-init

ENV PATH="/root/.cargo/bin:${PATH}"

# Install the channel from rust-toolchain.toml with rustfmt + clippy only.
COPY rust-toolchain.toml .
RUN RUST_VERSION="$(perl -lne 'print $1 if /^channel = \"(.+)\"$/' rust-toolchain.toml)" \
    && printf '[toolchain]\nchannel = "%s"\nprofile = "minimal"\ncomponents = ["rustc", "cargo", "rust-std", "rustfmt", "clippy"]\n' \
      "${RUST_VERSION}" > rust-toolchain.toml \
    && cat rust-toolchain.toml \
    && rustup set auto-self-update disable \
    && rustup show \
    && rustup default "${RUST_VERSION}" \
    && rm rust-toolchain.toml

# Fail fast on toolchain mismatch instead of silently installing.
# https://blog.rust-lang.org/2025/03/04/Rustup-1.28.1.html#whats-new-in-rustup-1281
ENV RUSTUP_AUTO_INSTALL=0

# Bake checksum-verified buf at the pinned version onto PATH.
# Checksums from https://github.com/bufbuild/buf/releases/download/v${BUF_VERSION}/sha256.txt
RUN arch="$(uname -m)" \
    && case "${arch}" in \
         x86_64) buf_sha256="855a055c8615a03ee93219f287bd7f652586c6b6b8d2b01079782cba54ee6033" ;; \
         aarch64) buf_sha256="1c3bc4a1aad5be3a30d20d999cb123eca8ab4861ce0db9aa3ee2c9a4dfe2d78c" ;; \
         *) echo "Unsupported architecture for buf: ${arch}" >&2; exit 1 ;; \
       esac \
    && curl --proto '=https' --tlsv1.2 --silent --show-error --fail --location \
         -o /usr/local/bin/buf \
         "https://github.com/bufbuild/buf/releases/download/v${BUF_VERSION}/buf-Linux-${arch}" \
    && echo "${buf_sha256}  /usr/local/bin/buf" | sha256sum -c - \
    && chmod +x /usr/local/bin/buf \
    && buf --version

# Bake checksum-verified protoc (not apt protobuf-compiler).
# https://github.com/protocolbuffers/protobuf/releases/tag/v28.3
# GitHub asset arch names differ from uname: aarch64 -> aarch_64.
RUN arch="$(uname -m)" \
    && case "${arch}" in \
         x86_64) \
           protoc_arch="x86_64" \
           protoc_sha256="0ad949f04a6a174da83cdcbdb36dee0a4925272a5b6d83f79a6bf9852076d53f" \
           ;; \
         aarch64) \
           protoc_arch="aarch_64" \
           protoc_sha256="1de522032a8b194002fe35cab86d747848238b5e4de4f99648372079f5b46f9a" \
           ;; \
         *) echo "Unsupported architecture for protoc: ${arch}" >&2; exit 1 ;; \
       esac \
    && curl --proto '=https' --tlsv1.2 --silent --show-error --fail --location \
         -o /tmp/protoc.zip \
         "https://github.com/protocolbuffers/protobuf/releases/download/v${PROTOC_VERSION}/protoc-${PROTOC_VERSION}-linux-${protoc_arch}.zip" \
    && echo "${protoc_sha256}  /tmp/protoc.zip" | sha256sum -c - \
    && unzip -q /tmp/protoc.zip -d /tmp/protoc \
    && mv /tmp/protoc/bin/protoc /usr/local/bin/protoc \
    && chmod +x /usr/local/bin/protoc \
    && rm -rf /tmp/protoc /tmp/protoc.zip \
    && [ "$(protoc --version)" = "libprotoc ${PROTOC_VERSION}" ]

# Bake cargo-audit at a pinned version so the audit job does not build it from
# the network on every pipeline.
#
# Pinned by exact version and `--locked` rather than by download checksum: this
# is a crates.io source build, not a release binary, so the lockfile shipped in
# the crate is the reproducibility control. 0.22 or newer is required — 0.21
# aborts parsing the CVSS 4.0 fields now present in the RustSec database, which
# fails as an unreadable "error loading advisory database" rather than as a
# version complaint.
ARG CARGO_AUDIT_VERSION=0.22.2
RUN cargo install cargo-audit --version "${CARGO_AUDIT_VERSION}" --locked \
    && cargo audit --version

# Sanity checks — fail the image build if anything required is missing.
RUN rustup component list --installed \
    && rustup toolchain list \
    && rustc --version \
    && cargo --version \
    && cargo fmt --version \
    && cargo clippy --version \
    && cargo audit --version \
    && buf --version \
    && protoc --version \
    && test -s /etc/ci-rust/apt-manifest.txt
