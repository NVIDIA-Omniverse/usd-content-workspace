#!/bin/bash
# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
#
# Wait for the Event Consumer Service to be ready before running tests.
# Uses the health endpoint: GET /api/v1beta/health  -> {"status": "healthy"}
#
# Usage:
#   CONFORMANCE_HEALTH_ENDPOINT=http://localhost:8000/api/v1beta/health ./wait_for_service.sh
#
# The health endpoint can be overridden by setting CONFORMANCE_HEALTH_ENDPOINT.
# If it is not set, falls back to CONFORMANCE_REST_ENDPOINT + /api/v1beta/health.

set -e

HEALTH_ENDPOINT="${CONFORMANCE_HEALTH_ENDPOINT:-}"

if [ -z "$HEALTH_ENDPOINT" ]; then
    REST_BASE="${CONFORMANCE_REST_ENDPOINT:-http://localhost:8000}"
    HEALTH_ENDPOINT="${REST_BASE}/api/v1beta/health"
fi

echo "Waiting for Event Consumer Service at ${HEALTH_ENDPOINT} ..."

MAX_ATTEMPTS=60
SLEEP_SECONDS=3

for i in $(seq 1 $MAX_ATTEMPTS); do
    if curl -s -f "${HEALTH_ENDPOINT}" > /dev/null 2>&1; then
        echo "Service is ready (attempt ${i}/${MAX_ATTEMPTS})."
        exit 0
    fi
    echo "  Attempt ${i}/${MAX_ATTEMPTS}: not ready yet. Retrying in ${SLEEP_SECONDS}s ..."
    sleep "${SLEEP_SECONDS}"
done

echo "ERROR: Service at ${HEALTH_ENDPOINT} did not become ready after ${MAX_ATTEMPTS} attempts." >&2
exit 1
