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

# Polls the discovery service health endpoint until it responds or the timeout
# is reached.  Called by run_tests.sh and the Docker CMD before running tests.
set -e

HEALTH_URL="${CONFORMANCE_HEALTH_ENDPOINT:-http://localhost:8080/health}"
MAX_ATTEMPTS=60
SLEEP_SECONDS=3

echo "Waiting for discovery service at ${HEALTH_URL}..."
for i in $(seq 1 $MAX_ATTEMPTS); do
    if curl -sf "${HEALTH_URL}" > /dev/null 2>&1; then
        echo "Service is ready (attempt ${i})."
        exit 0
    fi
    echo "Attempt ${i}/${MAX_ATTEMPTS}: not ready yet, retrying in ${SLEEP_SECONDS}s..."
    sleep $SLEEP_SECONDS
done

echo "ERROR: service did not become ready within $((MAX_ATTEMPTS * SLEEP_SECONDS))s."
exit 1
