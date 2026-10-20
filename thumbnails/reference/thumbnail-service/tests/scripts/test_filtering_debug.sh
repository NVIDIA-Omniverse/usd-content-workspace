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

# Debug script for the specific filtering issues
# This will help us understand why the filtering is not working correctly

# Configuration
HTTP_ENDPOINT="http://localhost:3000"
RESOURCE_IDENTITY="https://checkpointblobs.blob.core.windows.net/forthumbnails/demo/1000%2B%20images/"
# Load the bearer token from the environment; never hardcode credentials in the repo.
# Export BEARER_TOKEN (e.g. from a CI secret) before running this script.
AUTH_TOKEN="${BEARER_TOKEN:?BEARER_TOKEN environment variable must be set (export it from a secret; do not hardcode)}"

echo "=== DEBUGGING FILTERING ISSUES ==="
echo "HTTP Endpoint: $HTTP_ENDPOINT"
echo "Resource Identity: $RESOURCE_IDENTITY"
echo ""

# Test 1: Get all thumbnails (no filters)
echo "Test 1: Get all thumbnails (no filters)"
echo "curl -X GET \"$HTTP_ENDPOINT/thumbnails?resource_identity=$RESOURCE_IDENTITY\" \\"
echo "  -H \"Authorization: Bearer <redacted>\""
echo ""

RESPONSE1=$(curl -s -X GET "$HTTP_ENDPOINT/thumbnails?resource_identity=$RESOURCE_IDENTITY" \
  -H "Authorization: Bearer $AUTH_TOKEN")

echo "Response:"
echo "$RESPONSE1" | jq .
echo ""

# Test 2: Filter by height=256, width=256 (should only return 256x256)
echo "Test 2: Filter by height=256, width=256"
echo "curl -X GET \"$HTTP_ENDPOINT/thumbnails?resource_identity=$RESOURCE_IDENTITY&height=256&width=256\" \\"
echo "  -H \"Authorization: Bearer <redacted>\""
echo ""

RESPONSE2=$(curl -s -X GET "$HTTP_ENDPOINT/thumbnails?resource_identity=$RESOURCE_IDENTITY&height=256&width=256" \
  -H "Authorization: Bearer $AUTH_TOKEN")

echo "Response:"
echo "$RESPONSE2" | jq .
echo ""

# Test 3: Filter by height=512, width=512 (should return 512x256)
echo "Test 3: Filter by height=512, width=512"
echo "curl -X GET \"$HTTP_ENDPOINT/thumbnails?resource_identity=$RESOURCE_IDENTITY&height=512&width=512\" \\"
echo "  -H \"Authorization: Bearer <redacted>\""
echo ""

RESPONSE3=$(curl -s -X GET "$HTTP_ENDPOINT/thumbnails?resource_identity=$RESOURCE_IDENTITY&height=512&width=512" \
  -H "Authorization: Bearer $AUTH_TOKEN")

echo "Response:"
echo "$RESPONSE3" | jq .
echo ""

# Test 4: Filter by height=512, width=256 (should return 512x256)
echo "Test 4: Filter by height=512, width=256"
echo "curl -X GET \"$HTTP_ENDPOINT/thumbnails?resource_identity=$RESOURCE_IDENTITY&height=512&width=256\" \\"
echo "  -H \"Authorization: Bearer <redacted>\""
echo ""

RESPONSE4=$(curl -s -X GET "$HTTP_ENDPOINT/thumbnails?resource_identity=$RESOURCE_IDENTITY&height=512&width=256" \
  -H "Authorization: Bearer $AUTH_TOKEN")

echo "Response:"
echo "$RESPONSE4" | jq .
echo ""

# Test 5: Filter by height=200, width=200 (should return 200x200)
echo "Test 5: Filter by height=200, width=200"
echo "curl -X GET \"$HTTP_ENDPOINT/thumbnails?resource_identity=$RESOURCE_IDENTITY&height=200&width=200\" \\"
echo "  -H \"Authorization: Bearer <redacted>\""
echo ""

RESPONSE5=$(curl -s -X GET "$HTTP_ENDPOINT/thumbnails?resource_identity=$RESOURCE_IDENTITY&height=200&width=200" \
  -H "Authorization: Bearer $AUTH_TOKEN")

echo "Response:"
echo "$RESPONSE5" | jq .
echo ""

# Test 6: Filter by height=64, width=64 (should return 64x64)
echo "Test 6: Filter by height=64, width=64"
echo "curl -X GET \"$HTTP_ENDPOINT/thumbnails?resource_identity=$RESOURCE_IDENTITY&height=64&width=64\" \\"
echo "  -H \"Authorization: Bearer <redacted>\""
echo ""

RESPONSE6=$(curl -s -X GET "$HTTP_ENDPOINT/thumbnails?resource_identity=$RESOURCE_IDENTITY&height=64&width=64" \
  -H "Authorization: Bearer $AUTH_TOKEN")

echo "Response:"
echo "$RESPONSE6" | jq .
echo ""

echo "=== ANALYSIS ==="
echo "Based on your examples, we should see:"
echo "1. No filters: All thumbnails (5 total)"
echo "2. height=256, width=256: Only 256x256 thumbnails (2 total)"
echo "3. height=512, width=512: Empty array (should be 512x256)"
echo "4. height=512, width=256: Should return 512x256 thumbnail"
echo "5. height=200, width=200: Should return 200x200 thumbnail"
echo "6. height=64, width=64: Should return 64x64 thumbnail"
echo ""
echo "The issue seems to be that the filtering logic is not working correctly."
echo "Let's check the server logs to see what's happening during filtering."
