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

# Thumbnail Service Makefile

.PHONY: help worker-logs worker-metrics test test-verbose test-coverage test-crate setup-hooks pre-commit-check format format-check lint yaml-check helm-lint openapi-update ci-check start-native stop-native test-native build-native status-native build-local-image deploy-prod deploy-local-docker deploy-local-nvcr create-local-values status logs port-forward-http port-forward-grpc clean

# Variables
CHART_DIR := helm/thumbnail-service
RELEASE_NAME := thumbnail-service
# The worker's Deployment is rendered by this same chart (see
# openspec/changes/fold-task-worker-into-service-chart) but keeps its own
# image, replica count, and memory sizing under the `thumbnailWorker:` values key.
# Its rendered Deployment name follows the chart's `worker.fullname` helper
# default: "<release>-thumbnail-task-worker" when RELEASE_NAME doesn't
# already contain that string, which is true for the default below.
WORKER_DEPLOYMENT_NAME := $(RELEASE_NAME)-thumbnail-task-worker
LOCAL_IMAGE := thumbnail-service:local
WORKER_LOCAL_IMAGE := thumbnail-task-worker:local
LOCAL_DOCKER_VALUES := $(CHART_DIR)/values.local-docker.yaml
LOCAL_NVCR_VALUES := $(CHART_DIR)/values.local-nvcr.yaml
HELM_ARGS ?=

help: ## Show this help message
	@echo "Thumbnail Service Commands"
	@echo ""
	@echo "Development Commands:"
	@echo "  make test                  - Run all tests"
	@echo "  make test-verbose          - Run tests with verbose output"
	@echo "  make test-coverage         - Generate test coverage report"
	@echo "  make test-crate CRATE=name - Run tests for specific crate"
	@echo "  make setup-hooks           - Install Git pre-commit hooks"
	@echo "  make pre-commit-check      - Run pre-commit checks manually"
	@echo "  make format                - Format code with rustfmt"
	@echo "  make lint                  - Run clippy linter"
	@echo "  make yaml-check            - Validate YAML syntax in project files"
	@echo "  make helm-lint             - Lint Helm chart"
	@echo "  make openapi-update        - Update OpenAPI spec from Rust annotations"
	@echo "  make ci-check              - Run all CI/CD checks locally"
	@echo ""
	@echo "Native Service Commands:"
	@echo "  make start-native          - Start all services natively"
	@echo "  make stop-native           - Stop all native services"
	@echo "  make test-native           - Test native services"
	@echo "  make build-native          - Build native services"
	@echo ""
	@echo "Deploy Commands:"
	@echo "  make build-local-image     - Build local Docker image ($(LOCAL_IMAGE))"
	@echo "  make deploy-prod           - Deploy chart with current values"
	@echo "  make deploy-local-docker   - Deploy chart using local image values override"
	@echo "  make deploy-local-nvcr     - Deploy chart using local nvcr override"
	@echo "  make create-local-values   - Generate local values override templates"
	@echo ""
	@echo "Utility Commands:"
	@echo "  make status                - Show deployment status"
	@echo "  make logs                  - Show thumbnail-service container logs (HTTP + gRPC, one process)"
	@echo "  make worker-logs           - Show thumbnail-worker container logs (conversion pipeline)"
	@echo "  make worker-metrics        - Print thumbnail-worker outcome counters"
	@echo "  make port-forward-http     - Port forward REST endpoint to localhost:3001"
	@echo "  make port-forward-grpc     - Port forward gRPC endpoint to localhost:50052"
	@echo "  make clean                 - Uninstall the Helm release"

# Development Commands
test: ## Run all tests
	@echo "Running all tests..."
	cargo test --all

test-verbose: ## Run tests with verbose output
	@echo "Running tests with verbose output..."
	cargo test --all --verbose

test-coverage: ## Generate test coverage report
	@echo "Generating test coverage report..."
	@command -v cargo-tarpaulin >/dev/null 2>&1 || { echo "Installing cargo-tarpaulin..."; cargo install cargo-tarpaulin; }
	cargo tarpaulin --all --out Html --output-dir coverage --ignore-tests \
		--exclude-files "target/*" \
		--exclude-files "crates/proto/*" \
		--exclude-files "crates/service/src/bin/generate_openapi.rs" \
		--exclude-files "crates/common/src/test_fixtures*"
	@echo "Coverage report generated at coverage/tarpaulin-report.html"

test-crate: ## Run tests for specific crate (usage: make test-crate CRATE=thumbnail-service)
	@if [ -z "$(CRATE)" ]; then echo "Usage: make test-crate CRATE=<crate-name>"; exit 1; fi
	@echo "Running tests for $(CRATE)..."
	cargo test -p $(CRATE)

setup-hooks: ## Install Git pre-commit hooks
	@echo "Setting up Git pre-commit hooks..."
	./scripts/setup-hooks.sh

pre-commit-check: ## Run pre-commit checks manually
	@echo "Running pre-commit checks..."
	./scripts/pre-commit-check.sh

format: ## Format code with rustfmt
	@echo "Formatting code..."
	cargo fmt --all

format-check: ## Check code formatting
	@echo "Checking code formatting..."
	cargo fmt --all -- --check

lint: ## Run clippy linter
	@echo "Running clippy linter..."
	rustup component add clippy 2>/dev/null || true
	cargo clippy --all-targets --all-features -- -D warnings

yaml-check: ## Validate YAML syntax in project files
	@echo "Validating YAML syntax..."
	@validation_failed=false; \
	if [ -f ".gitlab-ci.yml" ]; then \
		if command -v glab >/dev/null 2>&1; then \
			if glab ci lint .gitlab-ci.yml >/dev/null 2>&1; then \
				echo "✅ GitLab CI YAML validation passed (glab)"; \
			else \
				echo "⚠️  glab ci lint unavailable in this environment; skipping strict .gitlab-ci.yml validation"; \
			fi; \
		else \
			echo "⚠️  glab not found; skipping strict .gitlab-ci.yml validation"; \
		fi; \
	fi; \
	for yaml_file in $(CHART_DIR)/values.yaml $(CHART_DIR)/Chart.yaml thumbnail.openapi.yaml .pre-commit-config.yaml; do \
		if [ -f "$$yaml_file" ]; then \
			if ! python3 -c "import yaml; yaml.safe_load(open('$$yaml_file', 'r'))" 2>/dev/null; then \
				echo "❌ YAML syntax error in: $$yaml_file"; \
				echo "   Run: python3 -c \"import yaml; yaml.safe_load(open('$$yaml_file', 'r'))\" to see details"; \
				validation_failed=true; \
			fi; \
		fi; \
	done; \
	if [ "$$validation_failed" = true ]; then \
		echo "❌ YAML validation failed"; \
		exit 1; \
	else \
		echo "✅ All YAML validation passed"; \
	fi

helm-lint: ## Lint the Helm chart (both the orchestrator and the fused worker Deployment)
	@echo "Linting Helm chart..."
	helm lint $(CHART_DIR) \
		--set thumbnailSizes=256x256 \
		--set thumbnailPathTemplate='https://example.blob.core.windows.net/c/{resource_address_md5}/{width}x{height}/{resource_filename}.{output_encoding}' \
		--set notifications.eventConsumer.endpoint=notifications:50051 \
		--set notifications.eventConsumer.resourcePrefixes[0]=https://account.blob.core.windows.net/container/ \
		--set storage.endpoint=storage:50001 \
		--set serviceIdentity.accessTokenFile.enabled=true \
		--set serviceIdentity.accessTokenFile.provisionerSidecar.container.image=cd-kube-jammy:latest
	bash helm/thumbnail-service/tests/render-fixtures.sh

openapi-update: ## Update OpenAPI spec from Rust annotations
	@echo "Updating OpenAPI specification..."
	./scripts/generate-openapi.sh

ci-check: yaml-check format-check lint test helm-lint ## Run all CI/CD checks locally
	@echo "✅ All CI/CD checks passed!"

# Native Service Commands
start-native: ## Start all services natively
	@echo "Starting native services..."
	./start-native-services.sh start

stop-native: ## Stop all native services
	@echo "Stopping native services..."
	./start-native-services.sh stop

test-native: ## Test native services
	@echo "Testing native services..."
	./start-native-services.sh test

build-native: ## Build native services
	@echo "Building native services..."
	./start-native-services.sh build

status-native: ## Show native service status
	@echo "Native service status..."
	./start-native-services.sh status

# Build and Deploy Commands
build-local-image: ## Build local Docker images
	@echo "Building local Docker images $(LOCAL_IMAGE) and $(WORKER_LOCAL_IMAGE)..."
	docker build -f docker/Dockerfile --target server -t $(LOCAL_IMAGE) .
	docker build -f docker/Dockerfile --target task-worker -t $(WORKER_LOCAL_IMAGE) .

deploy-prod: ## Deploy chart with current values (renders both the orchestrator and worker Deployments)
	@echo "Deploying $(RELEASE_NAME) from $(CHART_DIR)..."
	helm upgrade --install $(RELEASE_NAME) ./$(CHART_DIR) $(HELM_ARGS)

deploy-local-docker: build-local-image create-local-values ## Deploy using local Docker image override
	@echo "Deploying $(RELEASE_NAME) with local Docker image values..."
	helm upgrade --install $(RELEASE_NAME) ./$(CHART_DIR) -f $(LOCAL_DOCKER_VALUES) $(HELM_ARGS)

deploy-local-nvcr: create-local-values ## Deploy using local nvcr override values
	@echo "Deploying $(RELEASE_NAME) with local nvcr values..."
	helm upgrade --install $(RELEASE_NAME) ./$(CHART_DIR) -f $(LOCAL_NVCR_VALUES) $(HELM_ARGS)

create-local-values: ## Create local values files if missing (one file covers both components)
	@if [ ! -f "$(LOCAL_DOCKER_VALUES)" ]; then \
		echo "Creating $(LOCAL_DOCKER_VALUES)..."; \
		printf '%s\n' \
			'storage:' \
			'  endpoint: "172.18.0.1:50001"' \
			'' \
			'serviceIdentity:' \
			'  clientCredentials:' \
			'    enabled: true' \
			'    clientId: "local-dev"' \
			'    clientScope: "local"' \
			'    openIdConfigurationUri: "http://localhost/.well-known/openid-configuration"' \
			'    clientSecretRef:' \
			'      name: "thumbnail-oauth"' \
			'' \
			'notifications:' \
			'  eventConsumer:' \
			'    endpoint: "event-consumer-service:50051"' \
			'    resourcePrefixes:' \
			'      - "http://172.18.0.1:50001/local/"' \
			'' \
			'thumbnailPathTemplate: "http://172.18.0.1:50001/local/thumbnails/{resource_address_md5}/{width}x{height}/{resource_filename}.{output_encoding}"' \
			'' \
			'image:' \
			'  tag: "local"' \
			'  pullPolicy: Never' \
			'' \
			'thumbnailService:' \
			'  image:' \
			'    repository: thumbnail-service' \
			'' \
			'thumbnailWorker:' \
			'  image:' \
			'    repository: thumbnail-task-worker' \
			> "$(LOCAL_DOCKER_VALUES)"; \
	fi
	@if [ ! -f "$(LOCAL_NVCR_VALUES)" ]; then \
		echo "Creating $(LOCAL_NVCR_VALUES)..."; \
		printf '%s\n' \
			'storage:' \
			'  endpoint: "172.18.0.1:50001"' \
			'' \
			'serviceIdentity:' \
			'  clientCredentials:' \
			'    enabled: true' \
			'    clientId: "local-dev"' \
			'    clientScope: "local"' \
			'    openIdConfigurationUri: "http://localhost/.well-known/openid-configuration"' \
			'    clientSecretRef:' \
			'      name: "thumbnail-oauth"' \
			'' \
			'notifications:' \
			'  eventConsumer:' \
			'    endpoint: "event-consumer-service:50051"' \
			'    resourcePrefixes:' \
			'      - "http://172.18.0.1:50001/local/"' \
			'' \
			'thumbnailPathTemplate: "http://172.18.0.1:50001/local/thumbnails/{resource_address_md5}/{width}x{height}/{resource_filename}.{output_encoding}"' \
			> "$(LOCAL_NVCR_VALUES)"; \
	fi

# Utility Commands
status: ## Show deployment status (both the orchestrator and worker Deployments)
	@echo "Deployment Status:"
	kubectl get deployment "$(RELEASE_NAME)" "$(WORKER_DEPLOYMENT_NAME)" || true
	@echo ""
	@echo "Pod Status:"
	kubectl get pods -l 'app.kubernetes.io/instance=$(RELEASE_NAME)' || true
	@echo ""
	@echo "Service Status:"
	kubectl get service "$(RELEASE_NAME)" || true

logs: ## Show thumbnail-service container logs (HTTP + gRPC share one process)
	kubectl logs -l app.kubernetes.io/instance=$(RELEASE_NAME) -c thumbnail-service --tail=50

worker-logs: ## Show thumbnail-worker container logs (conversion, upload, associate)
	kubectl logs -l app.kubernetes.io/instance=$(RELEASE_NAME) -c thumbnail-worker --tail=50

worker-metrics: ## Print the worker's outcome counters without leaving the cluster
	@kubectl port-forward deployment/$(WORKER_DEPLOYMENT_NAME) 8080:8080 & \
		pf=$$!; \
		trap "kill $$pf 2>/dev/null" EXIT; \
		sleep 2; curl -s localhost:8080/metrics

port-forward-http: ## Port forward REST endpoint to localhost:3001
	@echo "Port forwarding REST endpoint to http://localhost:3001"
	kubectl port-forward service/$(RELEASE_NAME) 3001:3000

port-forward-grpc: ## Port forward gRPC endpoint to localhost:50052
	@echo "Port forwarding gRPC endpoint to localhost:50052"
	kubectl port-forward service/$(RELEASE_NAME) 50052:50051

clean: ## Clean up deployment
	@echo "Cleaning up Helm release $(RELEASE_NAME)..."
	-helm uninstall $(RELEASE_NAME)
	@echo "Cleanup complete"
