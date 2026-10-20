#!/bin/bash
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

# Native Thumbnail Service Startup Script
# Starts all services natively without Docker or containers

set -e

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOG_DIR="$SCRIPT_DIR/logs"

cleanup() {
    echo -e "${YELLOW}Shutting down services...${NC}"

    # Kill unified service
    pkill -f 'target/debug/server' 2>/dev/null || true

    # Kill any legacy event consumers
    pkill -f consume_events 2>/dev/null || true
    pkill -f thumbnail_event_listener 2>/dev/null || true

    echo -e "${GREEN}All services stopped${NC}"
}

check_dependencies() {
    local missing_deps=()

    if ! command -v cargo &> /dev/null; then
        missing_deps+=("cargo (Rust toolchain)")
    fi

    if [[ ${#missing_deps[@]} -gt 0 ]]; then
        echo -e "${RED}Missing required dependencies: ${missing_deps[*]}${NC}"
        echo -e "${YELLOW}Install Rust: https://rustup.rs/${NC}"
        exit 1
    fi
}

setup_environment() {
    echo -e "${BLUE}Setting up environment...${NC}"

    # Create logs directory
    mkdir -p "$LOG_DIR"

    # Check for .env file
    if [ ! -f .env ]; then
        echo -e "${YELLOW}Creating .env file with development defaults...${NC}"
        cat > .env << 'EOF'
# Thumbnail Service Development Configuration

# Service Endpoints (localhost for development)
STORAGE_GRPC_ENDPOINT=localhost:50001
NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT=localhost:50002

# Storage Configuration
THUMBNAIL_ADDRESS=https://checkpointblobs.blob.core.windows.net/forthumbnails/thumbnails

# Development Mode (enables mock data)
DEVELOPMENT_MODE=true

# Service-identity / outbound auth (in-process client-credentials flow).
#
# Disabled by default so a freshly-generated .env lets the service boot
# without immediately failing OIDC discovery. To make outbound calls
# (storage-service, task-queue, event-consumer) actually authenticate,
# set ENABLE_CLIENT_CREDENTIALS=true and populate every CLIENT_* value
# below with real credentials and a reachable OIDC discovery URL.
#
# The sidecar-driven access-token-file flow is k8s-only and is
# configured via Helm values, not here.
ENABLE_CLIENT_CREDENTIALS=false
CLIENT_ID=
CLIENT_SECRET=
CLIENT_SCOPE=
CLIENT_CREDENTIALS_OPENID_CONFIGURATION=
NOTIFICATION_SERVICE_AUTH_TOKEN=dev_auth_token

# Service Ports
HTTP_PORT=3000
GRPC_PORT=50051

# Logging
RUST_LOG=info,thumbnail_common=debug,thumbnail_service=debug
EOF
        echo -e "${GREEN}Created .env file with development defaults${NC}"
        echo -e "${YELLOW}Edit .env file to customize configuration${NC}"
    else
        echo -e "${GREEN}Using existing .env file${NC}"
    fi

    # Load environment variables
    set -a
    source .env
    set +a

    echo -e "${GREEN}Environment configured${NC}"
}

build_services() {
    echo -e "${BLUE}Building services...${NC}"

    echo -e "${YELLOW}Building unified service...${NC}"
    cargo build --bin server

    echo -e "${GREEN}All services built successfully${NC}"
}

start_unified_service() {
    echo -e "${BLUE}Starting unified service (HTTP + gRPC)...${NC}"

    # HTTP_PORT/GRPC_PORT are this script's own documented, numeric-only
    # variables (see the generated .env above). The binary itself reads
    # `PORT` (plain number) and `GRPC_PORT` (a full bind address, e.g.
    # "0.0.0.0:50051") — GRPC_HOST supplies the bind address portion so the
    # numeric GRPC_PORT stays numeric everywhere else in this script (the
    # readiness checks and status output below).
    RUST_LOG=${RUST_LOG:-info} \
    PORT=${HTTP_PORT:-3000} \
    GRPC_PORT="${GRPC_HOST:-0.0.0.0}:${GRPC_PORT:-50051}" \
    cargo run --bin server > "$LOG_DIR/thumbnail-service.log" 2>&1 &
    SERVICE_PID=$!

    echo "$SERVICE_PID" > "$LOG_DIR/service.pid"
    echo -e "${GREEN}Unified service started (PID: $SERVICE_PID)${NC}"
    echo -e "${BLUE}  HTTP API available at: http://localhost:${HTTP_PORT:-3000}${NC}"
    echo -e "${BLUE}  gRPC available at: localhost:${GRPC_PORT:-50051}${NC}"
    echo -e "${BLUE}  Logs: $LOG_DIR/thumbnail-service.log${NC}"
}

start_event_consumers() {
    if [[ "${START_EVENT_CONSUMERS:-false}" == "true" ]]; then
        echo -e "${YELLOW}Standalone event-consumer binaries were removed.${NC}"
        echo -e "${YELLOW}Event consumption now runs inside the server binary.${NC}"
    else
        echo -e "${YELLOW}Event consumers disabled (set START_EVENT_CONSUMERS=true to enable)${NC}"
    fi
}

wait_for_services() {
    echo -e "${BLUE}Waiting for services to be ready...${NC}"

    # Wait for HTTP service
    for i in {1..30}; do
        if curl -s "http://localhost:${HTTP_PORT:-3000}/health" >/dev/null 2>&1; then
            echo -e "${GREEN}✓ HTTP service is ready${NC}"
            break
        fi
        if [[ $i -eq 30 ]]; then
            echo -e "${RED}✗ HTTP service failed to start${NC}"
            return 1
        fi
        sleep 1
    done

    # Wait for gRPC service (using nc to check port)
    for i in {1..30}; do
        if nc -z localhost ${GRPC_PORT:-50051} 2>/dev/null; then
            echo -e "${GREEN}✓ gRPC service is ready${NC}"
            break
        fi
        if [[ $i -eq 30 ]]; then
            echo -e "${RED}✗ gRPC service failed to start${NC}"
            return 1
        fi
        sleep 1
    done
}

show_status() {
    echo -e "${BLUE}=== Service Status ===${NC}"

    # Check HTTP service
    if curl -s "http://localhost:${HTTP_PORT:-3000}/health" >/dev/null 2>&1; then
        echo -e "${GREEN}✓ HTTP Service: Running on port ${HTTP_PORT:-3000}${NC}"
    else
        echo -e "${RED}✗ HTTP Service: Not responding${NC}"
    fi

    # Check gRPC service
    if nc -z localhost ${GRPC_PORT:-50051} 2>/dev/null; then
        echo -e "${GREEN}✓ gRPC Service: Running on port ${GRPC_PORT:-50051}${NC}"
    else
        echo -e "${RED}✗ gRPC Service: Not responding${NC}"
    fi

    echo ""
    echo -e "${BLUE}=== Quick Test Commands ===${NC}"
    echo -e "${YELLOW}Health Check:${NC}"
    echo "  curl http://localhost:${HTTP_PORT:-3000}/health"
    echo ""
    echo -e "${YELLOW}Create Thumbnail (HTTP):${NC}"
    echo '  curl -X POST http://localhost:'"${HTTP_PORT:-3000}"'/thumbnails \'
    echo '    -H "Content-Type: application/json" \'
    echo '    -H "Authorization: Bearer test-token" \'
    echo '    -d '"'"'{"source_resource_identities": ["test://resource"], "specifications": []}'"'"
    echo ""
    echo -e "${YELLOW}View Logs:${NC}"
    echo "  tail -f $LOG_DIR/thumbnail-service.log"
    echo ""
    echo -e "${YELLOW}Stop Services:${NC}"
    echo "  $0 stop"
}

run_tests() {
    echo -e "${BLUE}Running test suite...${NC}"

    echo -e "${YELLOW}Unit Tests:${NC}"
    cargo test --all

    echo -e "${YELLOW}Integration Tests:${NC}"
    if [[ -f "tests/e2e/test_runner.sh" ]]; then
        echo "Note: E2E tests require Kubernetes setup. Run manually if needed:"
        echo "  ./tests/e2e/test_runner.sh"
    fi

    echo -e "${GREEN}Tests completed${NC}"
}

main() {
    trap cleanup EXIT

    case "${1:-start}" in
        "start")
            echo -e "${BLUE}=== Starting Thumbnail Service (Native) ===${NC}"
            check_dependencies
            setup_environment
            build_services
            start_unified_service
            start_event_consumers
            wait_for_services
            show_status

            echo -e "${GREEN}=== All services started successfully ===${NC}"
            echo -e "${YELLOW}Press Ctrl+C to stop all services${NC}"

            # Keep script running
            while true; do
                sleep 5
                # Check if services are still running
                if ! kill -0 $SERVICE_PID 2>/dev/null; then
                    echo -e "${RED}Unified service stopped unexpectedly${NC}"
                    show_status
                    exit 1
                fi
            done
            ;;
        "stop")
            echo -e "${BLUE}Stopping services...${NC}"
            cleanup
            # Clean up PID files
            rm -f "$LOG_DIR"/*.pid
            ;;
        "status")
            show_status
            ;;
        "test")
            run_tests
            ;;
        "build")
            check_dependencies
            setup_environment
            build_services
            ;;
        "logs")
            echo -e "${BLUE}Service Logs:${NC}"
            echo -e "${YELLOW}Unified Service:${NC}"
            tail -n 20 "$LOG_DIR/thumbnail-service.log" 2>/dev/null || echo "No service logs yet"
            ;;
        "help"|"-h"|"--help")
            echo "Native Thumbnail Service Runner"
            echo ""
            echo "Usage: $0 [command]"
            echo ""
            echo "Commands:"
            echo "  start    Start all services (default)"
            echo "  stop     Stop all services"
            echo "  status   Show service status"
            echo "  test     Run test suite"
            echo "  build    Build services only"
            echo "  logs     Show recent logs"
            echo "  help     Show this help"
            echo ""
            echo "Environment Variables:"
            echo "  HTTP_PORT                 HTTP service port (default: 3000)"
            echo "  GRPC_PORT                gRPC service port, numeric only (default: 50051)"
            echo "  GRPC_HOST                gRPC bind address (default: 0.0.0.0)"
            echo "  START_EVENT_CONSUMERS    Start event consumers (default: false)"
            echo "  DEVELOPMENT_MODE         Enable development mode (default: true)"
            ;;
        *)
            echo -e "${RED}Unknown command: $1${NC}"
            echo "Use '$0 help' for available commands"
            exit 1
            ;;
    esac
}

if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
    main "$@"
fi
