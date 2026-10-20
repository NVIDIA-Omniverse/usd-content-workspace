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

# Test script for create endpoint with multiple image_to_image specifications
# Using the provided resource identity; the auth token comes from BEARER_TOKEN

# Configuration
HTTP_ENDPOINT="http://localhost:3000"
STORAGE_ENDPOINT="http://localhost:50001"
RESOURCE_IDENTITY="https://checkpointblobs.blob.core.windows.net/forthumbnails/demo/1000%2B%20images/"
# Load the bearer token from the environment; never hardcode credentials in the repo.
# Export BEARER_TOKEN (e.g. from a CI secret) before running this script.
AUTH_TOKEN="${BEARER_TOKEN:?BEARER_TOKEN environment variable must be set (export it from a secret; do not hardcode)}"

echo "=== TESTING CREATE ENDPOINT WITH MULTIPLE SPECIFICATIONS ==="
echo "HTTP Endpoint: $HTTP_ENDPOINT"
echo "Storage Endpoint: $STORAGE_ENDPOINT"
echo "Resource Identity: $RESOURCE_IDENTITY"
echo ""

# Test 1: Multiple image_to_image specifications
echo "Test 1: Multiple image_to_image specifications"
JSON_PAYLOAD=$(cat <<EOF
{
  "resource_identity": "$RESOURCE_IDENTITY",
  "specification": [
    {
      "image_to_image": {
        "processor_identity": "PROCESSOR_IDENTITY_IMAGE_TO_IMAGE",
        "encoding_format": "IMAGE_ENCODING_FORMAT_PNG",
        "width": 256,
        "height": 256,
        "transform_type": "TRANSFORM_TYPE_SQUARE",
        "crop_direction": "CROP_DIRECTION_CENTER"
      }
    },
    {
      "image_to_image": {
        "processor_identity": "PROCESSOR_IDENTITY_IMAGE_TO_IMAGE",
        "encoding_format": "IMAGE_ENCODING_FORMAT_JPEG",
        "width": 512,
        "height": 512,
        "transform_type": "TRANSFORM_TYPE_FIT",
        "crop_direction": "CROP_DIRECTION_CENTER"
      }
    },
    {
      "image_to_image": {
        "processor_identity": "PROCESSOR_IDENTITY_IMAGE_TO_IMAGE",
        "encoding_format": "IMAGE_ENCODING_FORMAT_WEBP",
        "width": 128,
        "height": 128,
        "transform_type": "TRANSFORM_TYPE_SQUARE",
        "crop_direction": "CROP_DIRECTION_BOTTOM"
      }
    }
  ]
}
EOF
)

echo "Request payload:"
echo "$JSON_PAYLOAD" | jq .
echo ""

echo "Making request to create endpoint..."
echo "curl -X POST \"$HTTP_ENDPOINT/thumbnails/create\" \\"
echo "  -H \"Content-Type: application/json\" \\"
echo "  -H \"Authorization: Bearer <redacted>\" \\"
echo "  -d '$JSON_PAYLOAD'"
echo ""

# Make the actual request
RESPONSE=$(curl -s -X POST "$HTTP_ENDPOINT/thumbnails/create" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $AUTH_TOKEN" \
  -d "$JSON_PAYLOAD")

echo "Response:"
echo "$RESPONSE" | jq .
echo ""

# Check if the response contains multiple thumbnails
THUMBNAIL_COUNT=$(echo "$RESPONSE" | jq '.thumbnails | length')
echo "Number of thumbnails created: $THUMBNAIL_COUNT"

if [ "$THUMBNAIL_COUNT" -gt 1 ]; then
    echo " SUCCESS: Multiple specifications processed correctly!"
    echo "Created $THUMBNAIL_COUNT thumbnail tasks"
else
    echo " FAILURE: Expected multiple thumbnails, got $THUMBNAIL_COUNT"
fi

echo ""
echo "=== TEST COMPLETE ==="
