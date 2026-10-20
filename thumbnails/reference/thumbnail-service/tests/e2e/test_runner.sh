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

# End-to-End Test Runner for Thumbnail Service
# Tests both HTTP and gRPC APIs with task queue integration

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Test configuration
TEST_RESOURCE_URL="https://i.pinimg.com/736x/b0/ef/ba/b0efba4b7e0ff7c606ac4ab13973105a.jpg"
HTTP_PORT=3001
GRPC_PORT=50052
TASK_QUEUE_PORT=8001

# Cleanup function
cleanup() {
    echo -e "${YELLOW}Cleaning up port forwards...${NC}"
    pkill -f "kubectl port-forward" || true
    sleep 2
}

# Setup port forwarding
setup_port_forwarding() {
    echo -e "${BLUE}Setting up port forwarding...${NC}"

    # Start port forwarding in background
    kubectl port-forward service/thumbnail-service ${HTTP_PORT}:3000 &
    HTTP_PF_PID=$!

    kubectl port-forward service/thumbnail-service ${GRPC_PORT}:50051 &
    GRPC_PF_PID=$!

    kubectl port-forward service/thumbnail-task-queue-dev-api ${TASK_QUEUE_PORT}:8000 &
    TASK_QUEUE_PF_PID=$!

    # Wait for port forwards to be ready
    echo -e "${BLUE}Waiting for port forwards to be ready...${NC}"
    sleep 5

    # Verify port forwards are working with retry
    echo -e "${BLUE}Checking port connectivity...${NC}"

    for i in {1..10}; do
        if timeout 3 nc -z localhost ${HTTP_PORT} && \
           timeout 3 nc -z localhost ${GRPC_PORT} && \
           timeout 3 nc -z localhost ${TASK_QUEUE_PORT}; then
            echo -e "${GREEN}All ports are accessible${NC}"
            break
        fi

        if [[ $i -eq 10 ]]; then
            echo -e "${RED}Port forwarding failed after 10 attempts${NC}"
            echo -e "${RED}HTTP port ${HTTP_PORT}: $(timeout 3 nc -z localhost ${HTTP_PORT} && echo "OK" || echo "FAIL")${NC}"
            echo -e "${RED}gRPC port ${GRPC_PORT}: $(timeout 3 nc -z localhost ${GRPC_PORT} && echo "OK" || echo "FAIL")${NC}"
            echo -e "${RED}Task queue port ${TASK_QUEUE_PORT}: $(timeout 3 nc -z localhost ${TASK_QUEUE_PORT} && echo "OK" || echo "FAIL")${NC}"
            exit 1
        fi

        echo -e "${YELLOW}Attempt $i/10: Waiting for ports to be ready...${NC}"
        sleep 2
    done

    echo -e "${GREEN}Port forwarding setup complete${NC}"
}

# Test HTTP API
test_http_api() {
    echo -e "${BLUE}Testing HTTP API...${NC}"

    # Test 1: Create thumbnails via HTTP
    echo -e "${YELLOW}Test 1: HTTP Create Thumbnails${NC}"

    HTTP_RESPONSE=$(curl -s -X POST "http://localhost:${HTTP_PORT}/thumbnails" \
        -H "Content-Type: application/json" \
        -H "Authorization: Bearer test-token" \
        -d '{
            "resource_identity": "'${TEST_RESOURCE_URL}'",
            "specification": []
        }')

    echo "HTTP Response: $HTTP_RESPONSE"

    # Check if response contains expected fields
    if echo "$HTTP_RESPONSE" | jq -e '.thumbnails[0].task_id' >/dev/null 2>&1; then
        TASK_ID=$(echo "$HTTP_RESPONSE" | jq -r '.thumbnails[0].task_id')
        STATUS=$(echo "$HTTP_RESPONSE" | jq -r '.thumbnails[0].status')
        echo -e "${GREEN}✓ HTTP API returned task_id: $TASK_ID, status: $STATUS${NC}"

        # Verify it's a pending status (not processing immediately)
        if [[ "$STATUS" == "Pending" ]] || [[ "$STATUS" == "pending" ]]; then
            echo -e "${GREEN}✓ Task correctly submitted to queue (status: $STATUS)${NC}"
        else
            echo -e "${YELLOW} Unexpected status: $STATUS (expected Pending)${NC}"
        fi
    else
        echo -e "${RED}✗ HTTP API response missing task_id${NC}"
        return 1
    fi

    # Test 2: Get task status (if implemented)
    echo -e "${YELLOW}Test 2: HTTP Get Task Status${NC}"
    if [[ "$TASK_ID" != "error-task-0" ]] && [[ "$TASK_ID" != "dummy-task-0" ]]; then
        TASK_STATUS_RESPONSE=$(curl -s "http://localhost:${HTTP_PORT}/tasks/${TASK_ID}" || echo '{"error": "endpoint not found"}')
        echo "Task Status Response: $TASK_STATUS_RESPONSE"
    else
        echo "Skipping task status check for error/dummy task ID: $TASK_ID"
    fi
}

# Test gRPC API (requires grpcurl)
test_grpc_api() {
    echo -e "${BLUE}Testing gRPC API...${NC}"

    # Check if grpcurl is available
    if ! command -v grpcurl &> /dev/null; then
        echo -e "${YELLOW} grpcurl not available, skipping gRPC tests${NC}"
        echo -e "${YELLOW}  Install with: go install github.com/fullstorydev/grpcurl/cmd/grpcurl@latest${NC}"
        return 0
    fi

    echo -e "${YELLOW}Test 3: gRPC Create Thumbnails${NC}"

    # List services to verify connection
    echo "Available gRPC services:"
    if grpcurl -plaintext localhost:${GRPC_PORT} list 2>/dev/null; then
        echo -e "${GREEN}✓ gRPC server is accessible${NC}"
    else
        echo -e "${RED}✗ gRPC server not accessible${NC}"
        return 1
    fi

    # Test gRPC CreateThumbnails
    GRPC_RESPONSE=$(grpcurl -plaintext \
        -d '{
            "resource_identity": "'${TEST_RESOURCE_URL}'",
            "specification": [{
                "image_to_image": {
                    "processor_identity": 0,
                    "encoding_format": 1,
                    "width": 512,
                    "height": 512,
                    "transform_type": 1,
                    "crop_direction": 1
                }
            }]
        }' \
        localhost:${GRPC_PORT} nvidia.omniverse.thumbnail.v1alpha.ThumbnailService/CreateThumbnails 2>/dev/null || echo '{"error": "grpc call failed"}')

    echo "gRPC Response: $GRPC_RESPONSE"

    if echo "$GRPC_RESPONSE" | jq -e '.thumbnails[0].taskId // .thumbnails[0].task_id' >/dev/null 2>&1; then
        GRPC_TASK_ID=$(echo "$GRPC_RESPONSE" | jq -r '.thumbnails[0].taskId // .thumbnails[0].task_id')
        GRPC_STATUS=$(echo "$GRPC_RESPONSE" | jq -r '.thumbnails[0].status')
        echo -e "${GREEN}✓ gRPC API returned task_id: $GRPC_TASK_ID, status: $GRPC_STATUS${NC}"
    else
        echo -e "${RED}✗ gRPC API response missing task_id${NC}"
        return 1
    fi
}

# Test task queue integration
test_task_queue() {
    echo -e "${BLUE}Testing Task Queue Integration...${NC}"

    echo -e "${YELLOW}Test 4: Task Queue API Health${NC}"

    # Test task queue health endpoint
    QUEUE_HEALTH=$(curl -s "http://localhost:${TASK_QUEUE_PORT}/health" || echo '{"status": "unreachable"}')
    echo "Task Queue Health: $QUEUE_HEALTH"

    if echo "$QUEUE_HEALTH" | grep -q "healthy\|ok\|running"; then
        echo -e "${GREEN}✓ Task queue is healthy${NC}"
    else
        echo -e "${YELLOW} Task queue health status unclear${NC}"
    fi

    # Test task queue list endpoint (if available)
    echo -e "${YELLOW}Test 5: Task Queue Tasks List${NC}"
    QUEUE_TASKS=$(curl -s "http://localhost:${TASK_QUEUE_PORT}/tasks" || curl -s "http://localhost:${TASK_QUEUE_PORT}/api/tasks" || echo '{"error": "endpoint not found"}')
    echo "Active Tasks: $QUEUE_TASKS"
}

# Performance test
# test_load() {
#     echo -e "${BLUE}Testing Load Performance...${NC}"
#
#     echo -e "${YELLOW}Test 6: Concurrent HTTP Requests${NC}"
#
#     # Send 5 concurrent requests
#     for i in {1..5}; do
#         (curl -s -X POST "http://localhost:${HTTP_PORT}/thumbnails" \
#             -H "Content-Type: application/json" \
#             -H "Authorization: Bearer test-token-$i" \
#             -d '{
#                 "resource_identity": "'${TEST_RESOURCE_URL}'",
#                 "specification": []
#             }' > /tmp/response_$i.json) &
#     done
#
#     # Wait for all requests to complete
#     wait
#
#     # Check results
#     SUCCESS_COUNT=0
#     for i in {1..5}; do
#         if [[ -f "/tmp/response_$i.json" ]] && jq -e '.thumbnails[0].task_id' /tmp/response_$i.json >/dev/null 2>&1; then
#             SUCCESS_COUNT=$((SUCCESS_COUNT + 1))
#             TASK_ID=$(jq -r '.thumbnails[0].task_id' /tmp/response_$i.json)
#             echo "Request $i: Success (task_id: $TASK_ID)"
#         else
#             echo "Request $i: Failed"
#         fi
#         rm -f /tmp/response_$i.json
#     done
#
#     echo -e "${GREEN}✓ $SUCCESS_COUNT/5 concurrent requests successful${NC}"
#
#     if [[ $SUCCESS_COUNT -ge 4 ]]; then
#         echo -e "${GREEN}✓ Load test passed${NC}"
#     else
#         echo -e "${RED}✗ Load test failed (less than 4/5 requests successful)${NC}"
#         return 1
#     fi
# }

# Main test execution
main() {
    trap cleanup EXIT

    echo -e "${BLUE}=== Thumbnail Service E2E Tests ===${NC}"
    echo -e "${BLUE}Testing with resource: ${TEST_RESOURCE_URL}${NC}"
    echo ""

    # Check if we're connected to the right cluster
    CURRENT_CONTEXT=$(kubectl config current-context)
    echo -e "${BLUE}Current kubectl context: ${CURRENT_CONTEXT}${NC}"

    EXPECTED_CONTEXT="${KUBE_CONTEXT:-}"
    if [[ -n "$EXPECTED_CONTEXT" && "$CURRENT_CONTEXT" != "$EXPECTED_CONTEXT" ]]; then
        echo -e "${YELLOW} Not using ${EXPECTED_CONTEXT} context. Switching...${NC}"
        kubectl config use-context "$EXPECTED_CONTEXT"
    elif [[ -z "$EXPECTED_CONTEXT" ]]; then
        echo -e "${BLUE}KUBE_CONTEXT not set; using current kubectl context${NC}"
    fi

    # Verify pods are running
    echo -e "${BLUE}Checking pod status...${NC}"
    kubectl get pods | grep thumbnail
    echo ""

    setup_port_forwarding

    # Run tests
    echo -e "${BLUE}=== Starting Tests ===${NC}"

    test_http_api
    echo ""

    test_grpc_api
    echo ""

    test_task_queue
    echo ""

    # test_load
    echo ""

    echo -e "${GREEN}=== All E2E Tests Complete ===${NC}"
}

# Check dependencies
check_dependencies() {
    local missing_deps=()

    if ! command -v kubectl &> /dev/null; then
        missing_deps+=("kubectl")
    fi

    if ! command -v curl &> /dev/null; then
        missing_deps+=("curl")
    fi

    if ! command -v jq &> /dev/null; then
        missing_deps+=("jq")
    fi

    if ! command -v nc &> /dev/null; then
        missing_deps+=("nc (netcat)")
    fi

    if [[ ${#missing_deps[@]} -gt 0 ]]; then
        echo -e "${RED}Missing required dependencies: ${missing_deps[*]}${NC}"
        exit 1
    fi
}

# Script entry point
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
    check_dependencies
    main "$@"
fi
