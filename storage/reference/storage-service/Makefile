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

.DEFAULT_GOAL := build

BUILD_PROFILE := dev

TAPLO_VERSION := 0.10.0
CARGO_DENY_VERSION := 0.18.9
CARGO_MACHETE_VERSION := 0.9.1

TOOLS_DIR := $(shell pwd)/tools
TAPLO_DIR := $(TOOLS_DIR)/taplo/$(TAPLO_VERSION)
TAPLO := $(TAPLO_DIR)/taplo
CARGO_MACHETE_DIR := $(TOOLS_DIR)/cargo-machete/$(CARGO_DENY_VERSION)
CARGO_MACHETE := $(CARGO_MACHETE_DIR)/cargo-machete
CARGO_DENY_DIR := $(TOOLS_DIR)/cargo-deny/$(CARGO_DENY_VERSION)
CARGO_DENY := $(CARGO_DENY_DIR)/cargo-deny
CARGO_DENY_BASE := cargo-deny-$(CARGO_DENY_VERSION)-$(shell uname -m)-unknown-$(shell uname -s | tr '[:upper:]' '[:lower:]')-musl

DOCS_DIR := $(shell pwd)/docs
DOCS_DIST_DIR := $(DOCS_DIR)/_build

VIRTUALENV_DIR := $(shell pwd)/.venv
VIRTUALENV := $(VIRTUALENV_DIR)/virtualenv.o

$(TAPLO):
	@echo "=== Downloading taplo..."
	@mkdir -p $(TAPLO_DIR)
	@curl \
		-fsSL "https://github.com/tamasfe/taplo/releases/download/$(TAPLO_VERSION)/taplo-$(shell uname -s)-$(shell uname -m).gz" \
		-o - | gunzip > $(TAPLO)
	@chmod +x $(TAPLO)

$(CARGO_MACHETE):
	@echo "=== Downloading cargo-machete..."
	@mkdir -p $(CARGO_MACHETE_DIR)
	@curl \
		-fsSL "https://github.com/bnjbvr/cargo-machete/releases/download/v$(CARGO_MACHETE_VERSION)/cargo-machete-v$(CARGO_MACHETE_VERSION)-$(shell uname -m)-unknown-linux-musl.tar.gz" \
		-o - | tar -xzf - -C $(CARGO_MACHETE_DIR) "cargo-machete-v$(CARGO_MACHETE_VERSION)-$(shell uname -m)-unknown-linux-musl/cargo-machete" --strip-components 1
	@chmod +x $(CARGO_MACHETE)

$(CARGO_DENY):
	@echo "=== Downloading cargo-deny..."
	@mkdir -p $(CARGO_DENY_DIR)
	@curl \
		-fsSL "https://github.com/EmbarkStudios/cargo-deny/releases/download/$(CARGO_DENY_VERSION)/$(CARGO_DENY_BASE).tar.gz" \
		-o - | tar zx -C $(CARGO_DENY_DIR) --strip-components=1 $(CARGO_DENY_BASE)/cargo-deny
	@chmod +x $(CARGO_DENY)

.PHONY: .lint-dependencies
.lint-dependencies: $(TAPLO) $(CARGO_MACHETE) $(CARGO_DENY)
	@echo "=== Installing rustfmt..."
	@rustup component add rustfmt

	@echo "=== Installing clippy..."
	@rustup component add clippy

.PHONY: lint
lint: .lint-dependencies
	@echo "=== Running rustfmt checks..."
	@cargo fmt --all --message-format human -- --check

	@echo "=== Running clippy checks..."
	@cargo clippy --workspace --all-targets -- -Dwarnings

	@echo "=== Running taplo checks..."
	@$(TAPLO) fmt --check

	@echo "=== Running cargo-deny checks..."
	@$(CARGO_DENY) --locked check --show-stats

	@echo "=== Running cargo-machete checks..."
	@$(CARGO_MACHETE)

	@echo "=== Validating license headers..."
	@scripts/ci-check-license-headers.sh

	@echo "=== Validating version references..."
	@PATH="$(TAPLO_DIR):$(PATH)" scripts/ci-check-versions.sh CHANGELOG.md Cargo.toml

	@echo "=== Validating attributions..."
	@scripts/attributions.py validate -f attributions.json

.PHONY: fix-format
fix-format: .lint-dependencies
	@echo "=== Running clippy..."
	@cargo clippy --fix --allow-dirty

	@echo "=== Running rustfmt..."
	@cargo fmt

	@echo "=== Running taplo..."
	@$(TAPLO) fmt

.PHONY: build
build:
	@echo "=== Building the project..."
	@cargo build --profile $(BUILD_PROFILE) --workspace --all-targets

.PHONY: test
test:
	@echo "=== Running tests..."
	@cargo test --no-fail-fast --profile $(BUILD_PROFILE) --features test -- --nocapture

.PHONY: generate-attributions
generate-attributions:
	@echo "=== Generating attributions..."
	@scripts/attributions.py generate --temp-dir /tmp/storage-service-attributions > attributions.json

.PHONY: validate-attributions
validate-attributions:
	@echo "=== Validating attributions file..."
	@scripts/attributions.py validate -f attributions.json

.PHONY: cargo-deny
cargo-deny: $(CARGO_DENY)
	@echo "=== Running cargo-deny checks..."
	@$(CARGO_DENY) --locked check --show-stats

.PHONY: clean
clean:
	@echo "=== Cleaning up the workspace..."
	@rm -rf $(TOOLS_DIR)
	@rm -rf $(shell pwd)/target

.PHONY: docs
docs:
	@if [ -x "docs/repo.sh" ]; then \
		echo "=== Building docs..."; \
		cd "docs" && ./repo.sh docs; \
	else \
		echo "The documentation in docs/ is published as reStructuredText sources; NVIDIA builds the rendered site internally."; \
	fi
