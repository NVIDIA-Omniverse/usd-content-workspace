#!/usr/bin/env sh
# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
#
# Wait for the Event Aggregation Service health endpoint to become ready.
# Polls up to MAX_ATTEMPTS times, sleeping SLEEP_SECONDS between each attempt.

set -e

HEALTH_URL="${CONFORMANCE_HEALTH_ENDPOINT:-${CONFORMANCE_REST_ENDPOINT:-http://localhost:8001}/api/v1beta/health}"
MAX_ATTEMPTS=60
SLEEP_SECONDS=3

echo "Waiting for service at ${HEALTH_URL} ..."

attempt=1
while [ "$attempt" -le "$MAX_ATTEMPTS" ]; do
    if curl -sf "${HEALTH_URL}" > /dev/null 2>&1; then
        echo "Service is ready (attempt ${attempt}/${MAX_ATTEMPTS})."
        exit 0
    fi
    echo "  Not ready yet (attempt ${attempt}/${MAX_ATTEMPTS}), retrying in ${SLEEP_SECONDS}s ..."
    sleep "${SLEEP_SECONDS}"
    attempt=$((attempt + 1))
done

echo "ERROR: Service did not become ready after $((MAX_ATTEMPTS * SLEEP_SECONDS))s." >&2
exit 1
