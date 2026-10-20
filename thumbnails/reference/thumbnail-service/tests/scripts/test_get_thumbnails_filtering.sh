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

# Test script for GET thumbnails endpoint with filtering
# This tests the height, width, and format filtering functionality

# Configuration
HTTP_ENDPOINT="http://localhost:3000"
RESOURCE_IDENTITY="https://checkpointblobs.blob.core.windows.net/forthumbnails/demo/1000%2B%20images/"
# Load the bearer token from the environment; never hardcode credentials in the repo.
# Export BEARER_TOKEN (e.g. from a CI secret) before running this script.
AUTH_TOKEN="${BEARER_TOKEN:?BEARER_TOKEN environment variable must be set (export it from a secret; do not hardcode)}"

echo "=== TESTING GET THUMBNAILS WITH FILTERING ==="
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

THUMBNAIL_COUNT1=$(echo "$RESPONSE1" | jq '.thumbnail_resource_identities | length')
echo "Total thumbnails found: $THUMBNAIL_COUNT1"
echo ""

# Test 2: Filter by height only
echo "Test 2: Filter by height=256"
echo "curl -X GET \"$HTTP_ENDPOINT/thumbnails?resource_identity=$RESOURCE_IDENTITY&height=256\" \\"
echo "  -H \"Authorization: Bearer <redacted>\""
echo ""

RESPONSE2=$(curl -s -X GET "$HTTP_ENDPOINT/thumbnails?resource_identity=$RESOURCE_IDENTITY&height=256" \
  -H "Authorization: Bearer $AUTH_TOKEN")

echo "Response:"
echo "$RESPONSE2" | jq .
echo ""

THUMBNAIL_COUNT2=$(echo "$RESPONSE2" | jq '.thumbnail_resource_identities | length')
echo "Thumbnails with height=256: $THUMBNAIL_COUNT2"
echo ""

# Test 3: Filter by width only
echo "Test 3: Filter by width=512"
echo "curl -X GET \"$HTTP_ENDPOINT/thumbnails?resource_identity=$RESOURCE_IDENTITY&width=512\" \\"
echo "  -H \"Authorization: Bearer <redacted>\""
echo ""

RESPONSE3=$(curl -s -X GET "$HTTP_ENDPOINT/thumbnails?resource_identity=$RESOURCE_IDENTITY&width=512" \
  -H "Authorization: Bearer $AUTH_TOKEN")

echo "Response:"
echo "$RESPONSE3" | jq .
echo ""

THUMBNAIL_COUNT3=$(echo "$RESPONSE3" | jq '.thumbnail_resource_identities | length')
echo "Thumbnails with width=512: $THUMBNAIL_COUNT3"
echo ""

# Test 4: Filter by format only
echo "Test 4: Filter by format=PNG"
echo "curl -X GET \"$HTTP_ENDPOINT/thumbnails?resource_identity=$RESOURCE_IDENTITY&format=PNG\" \\"
echo "  -H \"Authorization: Bearer <redacted>\""
echo ""

RESPONSE4=$(curl -s -X GET "$HTTP_ENDPOINT/thumbnails?resource_identity=$RESOURCE_IDENTITY&format=PNG" \
  -H "Authorization: Bearer $AUTH_TOKEN")

echo "Response:"
echo "$RESPONSE4" | jq .
echo ""

THUMBNAIL_COUNT4=$(echo "$RESPONSE4" | jq '.thumbnail_resource_identities | length')
echo "Thumbnails with format=PNG: $THUMBNAIL_COUNT4"
echo ""

# Test 5: Filter by multiple criteria
echo "Test 5: Filter by height=256 AND width=256 AND format=PNG"
echo "curl -X GET \"$HTTP_ENDPOINT/thumbnails?resource_identity=$RESOURCE_IDENTITY&height=256&width=256&format=PNG\" \\"
echo "  -H \"Authorization: Bearer <redacted>\""
echo ""

RESPONSE5=$(curl -s -X GET "$HTTP_ENDPOINT/thumbnails?resource_identity=$RESOURCE_IDENTITY&height=256&width=256&format=PNG" \
  -H "Authorization: Bearer $AUTH_TOKEN")

echo "Response:"
echo "$RESPONSE5" | jq .
echo ""

THUMBNAIL_COUNT5=$(echo "$RESPONSE5" | jq '.thumbnail_resource_identities | length')
echo "Thumbnails with height=256 AND width=256 AND format=PNG: $THUMBNAIL_COUNT5"
echo ""

# Test 6: Filter by non-existent criteria
echo "Test 6: Filter by non-existent criteria (height=999)"
echo "curl -X GET \"$HTTP_ENDPOINT/thumbnails?resource_identity=$RESOURCE_IDENTITY&height=999\" \\"
echo "  -H \"Authorization: Bearer <redacted>\""
echo ""

RESPONSE6=$(curl -s -X GET "$HTTP_ENDPOINT/thumbnails?resource_identity=$RESOURCE_IDENTITY&height=999" \
  -H "Authorization: Bearer $AUTH_TOKEN")

echo "Response:"
echo "$RESPONSE6" | jq .
echo ""

THUMBNAIL_COUNT6=$(echo "$RESPONSE6" | jq '.thumbnail_resource_identities | length')
echo "Thumbnails with height=999: $THUMBNAIL_COUNT6"
echo ""

echo "=== SUMMARY ==="
echo "Total thumbnails (no filter): $THUMBNAIL_COUNT1"
echo "Height=256: $THUMBNAIL_COUNT2"
echo "Width=512: $THUMBNAIL_COUNT3"
echo "Format=PNG: $THUMBNAIL_COUNT4"
echo "Height=256 AND Width=256 AND Format=PNG: $THUMBNAIL_COUNT5"
echo "Height=999 (non-existent): $THUMBNAIL_COUNT6"
echo ""

if [ "$THUMBNAIL_COUNT6" -eq 0 ]; then
    echo " SUCCESS: Filtering is working correctly - non-existent criteria returns 0 results"
else
    echo " WARNING: Non-existent criteria returned results - filtering may not be working"
fi

echo ""
echo "=== TEST COMPLETE ==="
