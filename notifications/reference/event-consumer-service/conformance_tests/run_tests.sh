#!/bin/bash -e
# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
#
# Run the Event Consumer Service conformance tests.
#
# Usage:
#   ./run_tests.sh [extra pytest args]
#
# Environment variables (all optional):
#   CONFORMANCE_GRPC_ENDPOINT   - gRPC host:port  (default: localhost:50052)
#   CONFORMANCE_REST_ENDPOINT   - REST base URL   (default: http://localhost:8000)
#   CONFORMANCE_AUTH_TOKEN      - Bearer token    (default: empty = no auth)
#   CONFORMANCE_AUTH_REQUIRED   - set to "true" to run auth-error tests
#   CONFORMANCE_STREAM_TIMEOUT  - SSE read timeout in seconds (default: 10.0)
#   CONFORMANCE_REQUEST_TIMEOUT - HTTP request timeout in seconds (default: 30.0)

mkdir -p reports

run-conformance-tests --log-level=INFO "$@"
